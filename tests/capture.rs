//! Capture against a scripted node.
//!
//! The node is a real HTTP server rather than a substituted transport, so these tests
//! exercise the shipped client: the request encoding, the authentication header, the
//! response limits, and the error paths. What varies between tests is what the node says,
//! which is exactly what the guards exist to react to.
//!
//! Every scripted response is shaped after a response recorded from Zebra 6.2.3, including
//! the details that are easy to get wrong: `chain` reads `main` rather than `mainnet`, a
//! block height is a string parameter, an application error arrives with HTTP 200, and a
//! pool carries a `monitored` flag alongside its balance.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zec_ironwood_reconcile::capture::plan::CaptureRequest;
use zec_ironwood_reconcile::capture::run::{self, CaptureOptions, CaptureSummary};
use zec_ironwood_reconcile::capture::writer::OutputMode;
use zec_ironwood_reconcile::cli::exit::ExitCode;
use zec_ironwood_reconcile::domain::network::Network;
use zec_ironwood_reconcile::error::ReconcileError;
use zec_ironwood_reconcile::evidence::{layout, validation};
use zec_ironwood_reconcile::rpc::auth::{Authentication, Secret};
use zec_ironwood_reconcile::rpc::client::HttpTransport;
use zec_ironwood_reconcile::rpc::method::NodeClient;

const ANCHOR: u32 = 3_428_142;
const START: u32 = 3_428_143;
const END: u32 = 3_428_147;
const TIP: u32 = 3_428_300;
const TIP_DISTANCE: u32 = 100;

const PASSWORD: &str = "correct-horse-battery-staple";

/// How the scripted node should misbehave, if at all.
#[derive(Debug, Clone, Default)]
struct Script {
    chain: Option<&'static str>,
    testnet: Option<bool>,
    tip: Option<u32>,
    activation_height: Option<u32>,
    /// Heights whose pool response omits `chainValueZat` entirely.
    heights_without_pool_values: Vec<u32>,
    /// Serve a different hash for the end block from this call onward.
    reorg_end_block_after_calls: Option<u32>,
    /// Refuse block requests once this many have been served.
    fail_block_requests_after: Option<u32>,
    /// Reverse and extend the `valuePools` array.
    scramble_pool_order: bool,
    /// Report the Ironwood pool as one the node is not tracking.
    ironwood_untracked: bool,
    /// Close the connection instead of answering, as Zebra does when credentials are wrong.
    require_password: bool,
}

#[derive(Debug, Default)]
struct Counters {
    end_block_object_calls: u32,
    block_requests: u32,
    methods: Vec<String>,
}

struct StubNode {
    address: SocketAddr,
    shutdown: Arc<AtomicBool>,
    counters: Arc<Mutex<Counters>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl StubNode {
    fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();

        let shutdown = Arc::new(AtomicBool::new(false));
        let counters = Arc::new(Mutex::new(Counters::default()));

        let worker_shutdown = Arc::clone(&shutdown);
        let worker_counters = Arc::clone(&counters);
        let handle = std::thread::spawn(move || {
            while !worker_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        serve(stream, &script, &worker_counters);
                    }
                    Err(ref error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            address,
            shutdown,
            counters,
            handle: Some(handle),
        }
    }

    fn url(&self) -> String {
        format!("http://{}/", self.address)
    }

    fn methods_called(&self) -> Vec<String> {
        self.counters.lock().unwrap().methods.clone()
    }
}

impl Drop for StubNode {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Reads one request, answers it, and closes the connection.
fn serve(mut stream: TcpStream, script: &Script, counters: &Arc<Mutex<Counters>>) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());

    let mut content_length = 0_usize;
    let mut authorized = !script.require_password;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(value) = header_value(trimmed, "content-length") {
            content_length = value.parse().unwrap_or(0);
        }
        if let Some(value) = header_value(trimmed, "authorization") {
            authorized = decodes_to_password(value);
        }
    }

    if !authorized {
        // Zebra answers a bad credential by dropping the connection, not with a 401.
        return;
    }

    let mut body = vec![0_u8; content_length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }

    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let method = request["method"].as_str().unwrap_or_default().to_owned();
    let params = request["params"].as_array().cloned().unwrap_or_default();

    let response = dispatch(&method, &params, script, counters);
    let payload = serde_json::to_vec(&response).unwrap();

    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let _ = stream.write_all(&payload);
    let _ = stream.flush();
}

fn header_value<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let (key, value) = line.split_once(':')?;
    key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
}

fn decodes_to_password(header: &str) -> bool {
    let Some(encoded) = header.strip_prefix("Basic ") else {
        return false;
    };
    let expected = Authentication::Basic {
        user: "__cookie__".to_owned(),
        secret: Secret::new(PASSWORD),
    };
    expected.header_value().as_deref() == Some(&format!("Basic {encoded}"))
}

fn dispatch(
    method: &str,
    params: &[serde_json::Value],
    script: &Script,
    counters: &Arc<Mutex<Counters>>,
) -> serde_json::Value {
    counters.lock().unwrap().methods.push(method.to_owned());

    match method {
        "getinfo" => result(serde_json::json!({
            "version": 6_020_300_u64,
            "build": "v6.2.3",
            "subversion": "/Zebra:6.2.3/",
            "protocolversion": 170_160,
            "blocks": script.tip.unwrap_or(TIP),
            "connections": 30,
            "testnet": script.testnet.unwrap_or(false),
        })),

        "getblockchaininfo" => result(serde_json::json!({
            "chain": script.chain.unwrap_or("main"),
            "blocks": script.tip.unwrap_or(TIP),
            "headers": script.tip.unwrap_or(TIP),
            "bestblockhash": block_hash(script.tip.unwrap_or(TIP)),
            "estimatedheight": script.tip.unwrap_or(TIP),
            "upgrades": {
                "c8e71055": {"name": "NU6", "activationheight": 2_976_000, "status": "active"},
                "37a5165b": {
                    "name": "NU6.3",
                    "activationheight": script.activation_height.unwrap_or(3_428_143),
                    "status": "active"
                }
            },
            "consensus": {"chaintip": "37a5165b", "nextblock": "37a5165b"},
            "valuePools": pools(script.tip.unwrap_or(TIP), script, false),
        })),

        "getblock" => get_block(params, script, counters),

        other => error_response(-32_601, &format!("Method not found: {other}")),
    }
}

fn get_block(
    params: &[serde_json::Value],
    script: &Script,
    counters: &Arc<Mutex<Counters>>,
) -> serde_json::Value {
    // Zebra rejects a numeric height; the tool must send a string.
    let Some(height) = params
        .first()
        .and_then(serde_json::Value::as_str)
        .and_then(|text| text.parse::<u32>().ok())
    else {
        return error_response(-1, "Invalid params");
    };

    let verbosity = params
        .get(1)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1);
    if verbosity == 2 {
        return error_response(-1, "verbosity 2 must never be requested by this tool");
    }

    {
        let mut counters = counters.lock().unwrap();
        counters.block_requests += 1;
        if let Some(limit) = script.fail_block_requests_after
            && counters.block_requests > limit
        {
            return error_response(-99, "scripted interruption");
        }
    }

    if verbosity == 0 {
        return result(serde_json::Value::String(block_hex(height)));
    }

    let reorged = if height == END {
        let mut counters = counters.lock().unwrap();
        counters.end_block_object_calls += 1;
        script
            .reorg_end_block_after_calls
            .is_some_and(|limit| counters.end_block_object_calls > limit)
    } else {
        false
    };

    let omit_values = script.heights_without_pool_values.contains(&height);

    let tip = script.tip.unwrap_or(TIP);

    result(serde_json::json!({
        "hash": if reorged { reorg_hash(height) } else { block_hash(height) },
        "height": height,
        // Distance to the tip, exactly as a node computes it. It describes when the block
        // was asked for rather than the block, so it must not reach the evidence.
        "confirmations": tip.saturating_sub(height) + 1,
        "size": 1630,
        "previousblockhash": block_hash(height.saturating_sub(1)),
        "nextblockhash": block_hash(height + 1),
        "tx": [],
        "valuePools": pools(height, script, omit_values),
    }))
}

/// Value pools shaped as Zebra reports them.
///
/// Orchard drains and Ironwood fills by the same amount each block, which is the shape of a
/// post-activation interval.
fn pools(height: u32, script: &Script, omit_values: bool) -> serde_json::Value {
    let moved = i64::from(height.saturating_sub(ANCHOR)) * 1_000;
    let orchard = 366_000_000_000_000_i64 - moved;

    let entry = |id: &str, balance: i64, delta: i64, monitored: bool| {
        if omit_values {
            serde_json::json!({"id": id, "monitored": monitored})
        } else {
            serde_json::json!({
                "id": id,
                "chainValueZat": balance,
                "valueDeltaZat": delta,
                "monitored": monitored,
            })
        }
    };

    let mut entries = vec![
        entry("transparent", 1_000_000, 0, true),
        entry("sprout", 0, 0, true),
        entry("sapling", 2_000_000, 0, true),
        entry("orchard", orchard, -1_000, true),
        entry("lockbox", 0, 0, true),
        entry("ironwood", moved, 1_000, !script.ironwood_untracked),
    ];

    if script.scramble_pool_order {
        entries.reverse();
        // A pool introduced by a later upgrade must be ignored, not misattributed.
        entries.insert(3, entry("sequoia", 42, 42, true));
    }

    serde_json::Value::Array(entries)
}

fn block_hash(height: u32) -> String {
    format!("{:0>64}", format!("{height:x}"))
}

fn reorg_hash(height: u32) -> String {
    format!("{:f>64}", format!("{height:x}"))
}

fn block_hex(height: u32) -> String {
    format!("04000000{height:08x}")
}

fn result(value: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": value})
}

fn error_response(code: i64, message: &str) -> serde_json::Value {
    // Zebra serves application errors with HTTP 200 and no `result` member.
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "error": {"code": code, "message": message}
    })
}

struct Capture {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

fn authentication() -> Authentication {
    Authentication::Basic {
        user: "__cookie__".to_owned(),
        secret: Secret::new(PASSWORD),
    }
}

fn run_capture(
    node: &StubNode,
    network: Network,
    mode: OutputMode,
    root: &Path,
) -> Result<CaptureSummary, ReconcileError> {
    let transport = HttpTransport::new(
        &node.url(),
        authentication(),
        Duration::from_secs(10),
        1_000,
    )?;
    let client = NodeClient::new(&transport);

    let options = CaptureOptions {
        request: CaptureRequest::new(network, START, END, TIP_DISTANCE, None)?,
        output_mode: mode,
        progress_interval: 0,
    };

    run::run(&client, &options, root, &mut |_| {})
}

fn capture_with(script: Script) -> (StubNode, Capture, Result<CaptureSummary, ReconcileError>) {
    let node = StubNode::start(script);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");
    let outcome = run_capture(&node, Network::Mainnet, OutputMode::Create, &root);
    (node, Capture { _dir: dir, root }, outcome)
}

/// Every file in a bundle, as bytes.
fn bundle_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut found = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_owned();
                found.insert(relative, std::fs::read(&path).unwrap());
            }
        }
    }
    found
}

#[test]
fn a_healthy_node_yields_a_complete_bundle() {
    let (_node, capture, outcome) = capture_with(Script::default());
    let summary = outcome.expect("a healthy capture should succeed");

    assert_eq!(summary.manifest.bundle_id, "mainnet-3428142-3428147");
    assert_eq!(summary.block_count, 5);
    assert_eq!(summary.files_reused, 0);
    assert_eq!(summary.tip_at_capture.get(), TIP);

    let manifest = validation::load_manifest(&capture.root).unwrap();
    let report = validation::validate_bundle(&capture.root, &manifest);
    assert!(report.is_valid(), "failures: {:?}", report.failures);
    assert!(
        report.warnings.is_empty(),
        "warnings: {:?}",
        report.warnings
    );
}

#[test]
fn the_bundle_contains_every_expected_artifact() {
    let (_node, capture, outcome) = capture_with(Script::default());
    outcome.unwrap();

    let files = bundle_files(&capture.root);
    for expected in [
        layout::MANIFEST,
        layout::MANIFEST_HASH,
        layout::ANCHOR_BLOCK,
        layout::ANCHOR_VALUE_POOLS,
        layout::RPC_NODE_INFO,
        layout::RPC_CHAIN_INFO_START,
        layout::RPC_CHAIN_INFO_END,
        layout::METADATA_ENVIRONMENT,
        layout::METADATA_COMMAND,
        layout::METADATA_TOOL_VERSION,
        "blocks/3428143.hex",
        "blocks/3428143.pools.json",
        "blocks/3428147.hex",
        "blocks/3428147.pools.json",
    ] {
        assert!(files.contains_key(expected), "bundle is missing {expected}");
    }
}

#[test]
fn the_manifest_records_the_anchor_and_end_balances_the_node_reported() {
    let (_node, _capture, outcome) = capture_with(Script::default());
    let manifest = outcome.unwrap().manifest;

    assert_eq!(
        manifest.anchor.orchard_balance_zatoshis.get(),
        366_000_000_000_000
    );
    assert_eq!(manifest.anchor.ironwood_balance_zatoshis.get(), 0);
    assert_eq!(
        manifest.end.reported_ironwood_balance_zatoshis.get(),
        i64::from(END - ANCHOR) * 1_000
    );
    assert_eq!(manifest.end.tracking.ironwood_tracked_by_node, Some(true));
}

#[test]
fn no_credential_reaches_any_artifact() {
    let (_node, capture, outcome) = capture_with(Script {
        require_password: true,
        ..Script::default()
    });
    outcome.expect("the scripted node should have accepted the credentials");

    for (path, bytes) in bundle_files(&capture.root) {
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains(PASSWORD), "{path} contains the password");
        assert!(
            !text.contains("__cookie__"),
            "{path} contains the cookie user"
        );
        // The endpoint is withheld too: a URL can carry credentials in its userinfo.
        assert!(!text.contains("http://"), "{path} contains the endpoint");
    }
}

#[test]
fn the_manifest_states_that_the_endpoint_was_withheld() {
    let (_node, _capture, outcome) = capture_with(Script::default());
    assert!(outcome.unwrap().manifest.source.rpc_url_redacted);
}

#[test]
fn wrong_credentials_fail_with_a_message_that_names_the_likely_cause() {
    // Zebra drops the connection rather than answering 401, so the surface symptom is a
    // transport failure with no status to explain it.
    let node = StubNode::start(Script {
        require_password: true,
        ..Script::default()
    });
    let dir = tempfile::tempdir().unwrap();

    let transport = HttpTransport::new(
        &node.url(),
        Authentication::Basic {
            user: "__cookie__".to_owned(),
            secret: Secret::new("the-wrong-password"),
        },
        Duration::from_secs(5),
        1_000,
    )
    .unwrap();
    let client = NodeClient::new(&transport);

    let options = CaptureOptions {
        request: CaptureRequest::new(Network::Mainnet, START, END, TIP_DISTANCE, None).unwrap(),
        output_mode: OutputMode::Create,
        progress_interval: 0,
    };

    let error = run::run(&client, &options, &dir.path().join("bundle"), &mut |_| {}).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("--rpc-cookie-file"), "{message}");
    assert!(
        !message.contains("the-wrong-password"),
        "the failure quoted the password: {message}"
    );
    assert_eq!(ExitCode::from(&error), ExitCode::CaptureIncomplete);
}

#[test]
fn a_refused_connection_is_not_blamed_on_authentication() {
    // Nothing is listening, so the request never reaches a node that could reject it.
    // Naming credentials here sends the reader after a problem that does not exist.
    let address = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };

    let dir = tempfile::tempdir().unwrap();
    let transport = HttpTransport::new(
        &format!("http://{address}/"),
        authentication(),
        Duration::from_secs(5),
        1_000,
    )
    .unwrap();
    let client = NodeClient::new(&transport);

    let options = CaptureOptions {
        request: CaptureRequest::new(Network::Mainnet, START, END, TIP_DISTANCE, None).unwrap(),
        output_mode: OutputMode::Create,
        progress_interval: 0,
    };

    let error = run::run(&client, &options, &dir.path().join("bundle"), &mut |_| {}).unwrap_err();
    let message = error.to_string();
    assert!(
        !message.contains("--rpc-cookie-file"),
        "a refused connection was blamed on authentication: {message}"
    );
    assert!(
        message.to_lowercase().contains("refused"),
        "the failure did not name the real cause: {message}"
    );
}

#[test]
fn absent_pool_values_abort_during_preflight_before_any_block_is_fetched() {
    let (node, capture, outcome) = capture_with(Script {
        heights_without_pool_values: vec![ANCHOR],
        ..Script::default()
    });

    let error = outcome.unwrap_err();
    assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
    assert_eq!(ExitCode::from(&error).code(), 7);

    // Preflight probes the anchor only. No interval block was requested, and nothing was
    // written, so a doomed capture costs one round trip rather than an interval.
    let methods = node.methods_called();
    assert_eq!(methods.iter().filter(|m| *m == "getblock").count(), 1);
    assert!(
        !capture.root.join("blocks").exists(),
        "blocks were fetched despite an unusable anchor"
    );
}

#[test]
fn absent_pool_values_partway_through_an_interval_abort_the_capture() {
    let (_node, _capture, outcome) = capture_with(Script {
        heights_without_pool_values: vec![START + 2],
        ..Script::default()
    });

    let error = outcome.unwrap_err();
    assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
    assert!(error.to_string().contains("3428145"), "{error}");
}

#[test]
fn a_reorganised_end_block_aborts_the_capture() {
    // The end block is read once during the interval and once afterwards. Changing the hash
    // between those reads is what a reorganisation looks like from here.
    let (_node, _capture, outcome) = capture_with(Script {
        reorg_end_block_after_calls: Some(1),
        ..Script::default()
    });

    let error = outcome.unwrap_err();
    assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
    assert!(error.to_string().contains("reorganised"), "{error}");
    assert_eq!(ExitCode::from(&error).code(), 7);
}

#[test]
fn a_tip_too_close_to_the_interval_is_refused() {
    let (node, _capture, outcome) = capture_with(Script {
        tip: Some(END + TIP_DISTANCE - 1),
        ..Script::default()
    });

    let error = outcome.unwrap_err();
    assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
    assert_eq!(ExitCode::from(&error).code(), 7);
    assert!(
        !node.methods_called().iter().any(|m| m == "getblock"),
        "a block was fetched despite a tip that is too close"
    );
}

#[test]
fn a_network_mismatch_is_refused_with_a_context_mismatch() {
    let (_node, _capture, outcome) = capture_with(Script {
        chain: Some("test"),
        testnet: Some(true),
        ..Script::default()
    });

    let error = outcome.unwrap_err();
    assert!(matches!(error, ReconcileError::NetworkMismatch { .. }));
    assert_eq!(ExitCode::from(&error).code(), 8);
}

#[test]
fn a_node_whose_getinfo_contradicts_its_chain_field_is_refused() {
    let (_node, _capture, outcome) = capture_with(Script {
        testnet: Some(true),
        ..Script::default()
    });
    assert!(matches!(
        outcome.unwrap_err(),
        ReconcileError::NetworkMismatch { .. }
    ));
}

#[test]
fn a_node_activating_ironwood_elsewhere_is_refused() {
    let (_node, _capture, outcome) = capture_with(Script {
        activation_height: Some(3_000_000),
        ..Script::default()
    });

    let error = outcome.unwrap_err();
    assert!(matches!(error, ReconcileError::ActivationMismatch { .. }));
    assert_eq!(ExitCode::from(&error).code(), 8);
}

#[test]
fn pools_are_read_by_identifier_regardless_of_order_or_additions() {
    // A reordered array carrying a pool from a future upgrade must parse to the same result.
    let (_node, _capture, ordered) = capture_with(Script::default());
    let (_node2, _capture2, scrambled) = capture_with(Script {
        scramble_pool_order: true,
        ..Script::default()
    });

    let ordered = ordered.unwrap().manifest;
    let scrambled = scrambled.unwrap().manifest;

    assert_eq!(ordered.anchor, scrambled.anchor);
    assert_eq!(
        ordered.end.reported_orchard_balance_zatoshis,
        scrambled.end.reported_orchard_balance_zatoshis
    );
    assert_eq!(
        ordered.end.reported_ironwood_balance_zatoshis,
        scrambled.end.reported_ironwood_balance_zatoshis
    );
}

#[test]
fn an_empty_pool_is_recorded_and_raises_an_advisory_without_failing() {
    // The node's `monitored` flag is stored because it is part of the response, but nothing
    // infers from it: Zebra computes it as `chainValueZat != 0`. The advisory is raised by
    // the balance being zero, which is the fact a reader needs.
    let (_node, _capture, outcome) = capture_with(Script {
        ironwood_untracked: true,
        ..Script::default()
    });
    let summary = outcome.expect("an empty pool is a caveat, not a capture failure");

    assert_eq!(
        summary.manifest.end.tracking.ironwood_tracked_by_node,
        Some(false)
    );
    assert!(
        summary
            .advisories
            .iter()
            .any(|advisory| advisory.id == "pool_balance_is_zero"),
        "advisories: {:?}",
        summary.advisories
    );
}

#[test]
fn an_archive_is_published_for_the_offline_syscall_check() {
    // `scripts/check-offline-verify.sh` traces `verify` against this archive and asserts the
    // process issues no network syscall. That check needs a real archive and this suite is
    // the only place one can be produced without a node, so the archive is written where the
    // script can find it rather than into a temporary directory.
    let (_node, capture, outcome) = capture_with(Script::default());
    outcome.unwrap();

    let destination = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/offline-check");
    std::fs::create_dir_all(&destination).unwrap();
    let archive = destination.join("evidence.tar.zst");

    let digest =
        zec_ironwood_reconcile::evidence::archive::pack_with_digest(&capture.root, &archive)
            .unwrap();

    assert!(archive.is_file());
    assert_eq!(digest.len(), 64);
}

#[test]
fn two_captures_at_different_tips_produce_identical_evidence() {
    // Independent reproduction is the property the project rests on: a second operator
    // capturing the same interval later must obtain the same evidence bytes. A node reports
    // each block's distance to the tip, so a response stored verbatim would fail this.
    let evidence_of = |tip: u32| -> BTreeMap<String, Vec<u8>> {
        let node = StubNode::start(Script {
            tip: Some(tip),
            ..Script::default()
        });
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("bundle");
        run_capture(&node, Network::Mainnet, OutputMode::Create, &root).unwrap();

        bundle_files(&root)
            .into_iter()
            .filter(|(path, _)| path.starts_with("blocks/") || path.starts_with("anchor/"))
            .collect()
    };

    let earlier = evidence_of(TIP);
    let later = evidence_of(TIP + 10_000);

    assert!(!earlier.is_empty());
    assert_eq!(
        earlier, later,
        "evidence changed with the chain tip, so two operators would not agree"
    );
}

#[test]
fn an_interrupted_capture_resumes_into_an_identical_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");

    // The first attempt dies partway through the interval.
    {
        let node = StubNode::start(Script {
            fail_block_requests_after: Some(5),
            ..Script::default()
        });
        let error = run_capture(&node, Network::Mainnet, OutputMode::Create, &root).unwrap_err();
        assert!(matches!(error, ReconcileError::Rpc(_)));
    }

    assert!(
        !root.join(layout::MANIFEST).exists(),
        "an interrupted capture must not leave a manifest"
    );

    let partial_files = bundle_files(&root);
    assert!(
        !partial_files.is_empty(),
        "the interrupted run should have written something to resume from"
    );
    assert!(
        !partial_files.keys().any(|path| path.ends_with(".partial")),
        "a partially written file survived: {:?}",
        partial_files.keys().collect::<Vec<_>>()
    );

    // The second attempt resumes and completes.
    let resumed = {
        let node = StubNode::start(Script::default());
        run_capture(&node, Network::Mainnet, OutputMode::Resume, &root).unwrap()
    };
    assert!(resumed.files_reused > 0, "nothing was reused on resume");

    // An uninterrupted capture of the same interval, for comparison.
    let fresh_dir = tempfile::tempdir().unwrap();
    let fresh_root = fresh_dir.path().join("bundle");
    {
        let node = StubNode::start(Script::default());
        run_capture(&node, Network::Mainnet, OutputMode::Create, &fresh_root).unwrap();
    }

    let resumed_files = bundle_files(&root);
    let fresh_files = bundle_files(&fresh_root);

    // Evidence must be byte-identical. The manifest and the capture environment are not
    // compared, because both record when the capture ran.
    let evidence = |files: &BTreeMap<String, Vec<u8>>| -> BTreeMap<String, Vec<u8>> {
        files
            .iter()
            .filter(|(path, _)| {
                path.as_str() != layout::MANIFEST
                    && path.as_str() != layout::MANIFEST_HASH
                    && path.as_str() != layout::METADATA_ENVIRONMENT
            })
            .map(|(path, bytes)| (path.clone(), bytes.clone()))
            .collect()
    };

    assert_eq!(evidence(&resumed_files), evidence(&fresh_files));

    // The manifests must agree on every file digest, which is the claim a reader checks.
    let resumed_manifest = validation::load_manifest(&root).unwrap();
    let fresh_manifest = validation::load_manifest(&fresh_root).unwrap();

    let digests = |manifest: &zec_ironwood_reconcile::evidence::manifest::Manifest| {
        manifest
            .files
            .iter()
            .filter(|entry| entry.path != layout::METADATA_ENVIRONMENT)
            .map(|entry| (entry.path.clone(), entry.sha256.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(digests(&resumed_manifest), digests(&fresh_manifest));
    assert_eq!(resumed_manifest.anchor, fresh_manifest.anchor);
    assert_eq!(resumed_manifest.end, fresh_manifest.end);
}

#[test]
fn resuming_into_a_bundle_captured_for_another_interval_is_refused() {
    // Without this, the earlier interval's blocks stay on disk: the manifest does not list
    // them, so verification only warns, and a published archive carries blocks outside the
    // interval it declares.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");

    {
        let node = StubNode::start(Script::default());
        run_capture(&node, Network::Mainnet, OutputMode::Create, &root).unwrap();
    }

    let node = StubNode::start(Script::default());
    let transport = HttpTransport::new(
        &node.url(),
        authentication(),
        Duration::from_secs(10),
        1_000,
    )
    .unwrap();
    let client = NodeClient::new(&transport);

    // Same start, an earlier end: heights END-1 and END are now outside the interval.
    let options = CaptureOptions {
        request: CaptureRequest::new(Network::Mainnet, START, END - 2, TIP_DISTANCE, None).unwrap(),
        output_mode: OutputMode::Resume,
        progress_interval: 0,
    };

    let error = run::run(&client, &options, &root, &mut |_| {}).unwrap_err();
    assert!(matches!(error, ReconcileError::InvalidInput { .. }));

    let message = error.to_string();
    assert!(message.contains("different interval"), "{message}");
    assert!(message.contains("--overwrite"), "{message}");
}

#[test]
fn resuming_the_same_interval_is_still_permitted() {
    // The guard must not make ordinary resumption impossible.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");

    {
        let node = StubNode::start(Script {
            fail_block_requests_after: Some(5),
            ..Script::default()
        });
        run_capture(&node, Network::Mainnet, OutputMode::Create, &root).unwrap_err();
    }

    let node = StubNode::start(Script::default());
    let summary = run_capture(&node, Network::Mainnet, OutputMode::Resume, &root).unwrap();
    assert!(summary.files_reused > 0);
}

#[test]
fn a_corrupted_anchor_block_is_refused_on_resume() {
    // The anchor is evidence like any other block. A resumed capture must not adopt bytes
    // it would have refused to write.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");

    {
        let node = StubNode::start(Script::default());
        run_capture(&node, Network::Mainnet, OutputMode::Create, &root).unwrap();
    }

    std::fs::write(root.join(layout::ANCHOR_BLOCK), b"not hex at all").unwrap();
    std::fs::remove_file(root.join(layout::MANIFEST)).unwrap();

    let node = StubNode::start(Script::default());
    let error = run_capture(&node, Network::Mainnet, OutputMode::Resume, &root).unwrap_err();

    assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
    assert!(error.to_string().contains("hexadecimal"), "{error}");
}

#[test]
fn a_resumed_bundle_still_validates_against_its_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");

    {
        let node = StubNode::start(Script {
            fail_block_requests_after: Some(5),
            ..Script::default()
        });
        run_capture(&node, Network::Mainnet, OutputMode::Create, &root).unwrap_err();
    }
    {
        let node = StubNode::start(Script::default());
        run_capture(&node, Network::Mainnet, OutputMode::Resume, &root).unwrap();
    }

    let manifest = validation::load_manifest(&root).unwrap();
    let report = validation::validate_bundle(&root, &manifest);
    assert!(report.is_valid(), "failures: {:?}", report.failures);
}

#[test]
fn capturing_into_an_existing_bundle_is_refused_without_a_flag() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");

    {
        let node = StubNode::start(Script::default());
        run_capture(&node, Network::Mainnet, OutputMode::Create, &root).unwrap();
    }

    let node = StubNode::start(Script::default());
    let error = run_capture(&node, Network::Mainnet, OutputMode::Create, &root).unwrap_err();
    assert!(matches!(error, ReconcileError::InvalidInput { .. }));
    assert_eq!(ExitCode::from(&error).code(), 2);
}

#[test]
fn only_read_methods_are_ever_called() {
    let (node, _capture, outcome) = capture_with(Script::default());
    outcome.unwrap();

    let permitted = ["getinfo", "getblockchaininfo", "getblock"];
    for method in node.methods_called() {
        assert!(
            permitted.contains(&method.as_str()),
            "capture called an unexpected method: {method}"
        );
    }
}

#[test]
fn the_node_is_never_asked_for_verbosity_two() {
    // The scripted node refuses verbosity 2, so a capture that requested it would fail.
    let (_node, _capture, outcome) = capture_with(Script::default());
    outcome.expect("capture must never request verbosity 2");
}
