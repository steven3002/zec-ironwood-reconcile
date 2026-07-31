//! Executing a capture, from preflight to finished manifest.
//!
//! The order of operations is the guarantee this module provides. Preflight rules out a
//! doomed run before any block is retrieved; the interval is read in ascending order; the
//! end block is re-queried afterwards to detect a reorganisation that happened while the
//! interval was being read; and the manifest is written last, so a bundle carrying one is a
//! bundle whose capture completed.

use std::path::{Path, PathBuf};

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::domain::height::BlockHeight;
use crate::domain::network::Network;
use crate::domain::pool::Pool;
use crate::error::ReconcileError;
use crate::evidence::layout;
use crate::evidence::manifest::{
    Activation, AnchorState, Encoding, EndState, EndStateTracking, Manifest, Rfc3339Timestamp,
    SCHEMA_VERSION, Source, Tool,
};
use crate::evidence::pool_state_file::CapturedBlockState;
use crate::rpc::method::NodeClient;

use crate::capture::fetch;
use crate::capture::guard::{self, Advisory, IRONWOOD_UPGRADE_NAME};
use crate::capture::plan::{self, CaptureRequest, Preflight};
use crate::capture::writer::{BundleWriter, OutputMode};

/// Receives human-readable progress. Progress never reaches a hashed artifact.
pub type Progress<'a> = &'a mut dyn FnMut(&str);

/// Everything a capture needs beyond the node itself.
#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub request: CaptureRequest,
    pub output_mode: OutputMode,
    /// How often progress is reported, in blocks.
    pub progress_interval: u32,
}

/// A packed archive of a bundle and the digest published alongside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archive {
    pub path: PathBuf,
    pub sha256: String,
}

/// What a completed capture produced.
#[derive(Debug, Clone)]
pub struct CaptureSummary {
    pub bundle_root: PathBuf,
    /// Present when the caller asked for the bundle to be packed.
    pub archive: Option<Archive>,
    pub manifest: Manifest,
    pub block_count: u32,
    /// Files retrieved during this run.
    pub files_written: u32,
    /// Files already present and reused, which is non-zero only on a resumed capture.
    pub files_reused: u32,
    pub tip_at_capture: BlockHeight,
    pub advisories: Vec<Advisory>,
}

/// Environment recorded alongside the evidence.
///
/// Deliberately excludes the endpoint, any credential, the operator's identity, and any
/// path outside the bundle. None of it affects a report hash; it exists so a reader knows
/// what produced the bundle.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureEnvironment {
    pub tool_name: &'static str,
    pub tool_version: &'static str,
    pub node_implementation: String,
    pub node_version: String,
    pub node_protocol_version: u64,
    pub network: Network,
    pub tip_height_at_capture: u32,
    pub anchor_height: u32,
    pub start_height: u32,
    pub end_height: u32,
    pub block_count: u32,
    pub tip_distance_required: u32,
    pub captured_at: String,
    pub build_target: String,
    /// Recorded as a statement of policy: an endpoint can embed credentials, so it is never
    /// written to a bundle.
    pub rpc_url_recorded: bool,
}

/// Runs a capture to completion.
pub fn run(
    client: &NodeClient<'_>,
    options: &CaptureOptions,
    bundle_root: &Path,
    progress: Progress<'_>,
) -> Result<CaptureSummary, ReconcileError> {
    let request = &options.request;

    progress("checking the node before retrieving anything");
    let preflight = plan::preflight(client, request)?;
    progress(&format!(
        "node {} {} at height {}",
        preflight.node.implementation(),
        preflight.node.version(),
        preflight.tip
    ));

    let mut writer = BundleWriter::open(bundle_root, options.output_mode)?;

    write_node_responses(&mut writer, &preflight)?;
    let anchor = write_anchor(client, &mut writer, &preflight, request)?;

    let end_state = capture_interval(client, &mut writer, request, options, progress)?;

    progress("re-reading the end block to detect a reorganisation");
    let recheck = client.get_block_object(request.interval.end_height())?;
    let recheck = plan::parse_pool_state(request.interval.end_height(), &recheck)?;
    guard::check_end_block_unchanged(
        request.interval.end_height(),
        &end_state.block_hash,
        &recheck.block_hash,
    )?;

    let closing = client.get_blockchain_info()?;
    writer.write(
        layout::RPC_CHAIN_INFO_END,
        &plan::serialize_response(&closing.json)?,
        Encoding::Json,
    )?;

    let advisories = collect_advisories(&preflight, request.network, &end_state);
    write_metadata(&mut writer, &preflight, request)?;

    let files_written = writer.written_count();
    let files_reused = writer.reused_count();
    let manifest = writer.finish(build_manifest(&preflight, request, &anchor, &end_state)?)?;

    Ok(CaptureSummary {
        bundle_root: bundle_root.to_path_buf(),
        archive: None,
        manifest,
        block_count: request.interval.block_count(),
        files_written,
        files_reused,
        tip_at_capture: preflight.tip,
        advisories,
    })
}

/// Stores the node identity responses the manifest's provenance claims rest on.
fn write_node_responses(
    writer: &mut BundleWriter,
    preflight: &Preflight,
) -> Result<(), ReconcileError> {
    writer.write(
        layout::RPC_NODE_INFO,
        &plan::serialize_response(&preflight.node_json)?,
        Encoding::Json,
    )?;
    writer.write(
        layout::RPC_CHAIN_INFO_START,
        &plan::serialize_response(&preflight.chain_json)?,
        Encoding::Json,
    )
}

/// Stores the anchor block and the balances declared at it.
///
/// The anchor's consensus bytes are kept as well as its reported balances so that a
/// verifier can confirm the anchor hash from the bytes rather than accepting the node's
/// word for which block the interval hangs from.
fn write_anchor(
    client: &NodeClient<'_>,
    writer: &mut BundleWriter,
    preflight: &Preflight,
    request: &CaptureRequest,
) -> Result<CapturedBlockState, ReconcileError> {
    let height = request.interval.anchor_height();

    if !writer.contains(layout::ANCHOR_BLOCK)? {
        let hex = client.get_block_raw_hex(height)?;
        writer.write(layout::ANCHOR_BLOCK, hex.as_bytes(), Encoding::RawBlockHex)?;
    } else {
        writer.adopt(layout::ANCHOR_BLOCK, Encoding::RawBlockHex)?;
    }

    writer.write(
        layout::ANCHOR_VALUE_POOLS,
        &plan::pool_state_bytes(&preflight.anchor_json)?,
        Encoding::Json,
    )?;

    Ok(preflight.anchor.clone())
}

/// Reads every height in the interval, in ascending order.
fn capture_interval(
    client: &NodeClient<'_>,
    writer: &mut BundleWriter,
    request: &CaptureRequest,
    options: &CaptureOptions,
    progress: Progress<'_>,
) -> Result<CapturedBlockState, ReconcileError> {
    let total = request.interval.block_count();
    let mut last: Option<CapturedBlockState> = None;
    let mut done = 0_u32;

    for height in request.interval.heights() {
        let fetched = fetch::height(
            client,
            writer,
            height,
            &layout::block(height),
            &layout::block_value_pools(height),
        )?;

        done = done.saturating_add(1);
        if options.progress_interval != 0 && done.is_multiple_of(options.progress_interval) {
            progress(&format!("captured {done} of {total} blocks"));
        }

        last = Some(fetched.state);
    }

    last.ok_or_else(|| ReconcileError::InvalidInterval {
        reason: "the interval contained no blocks".to_owned(),
    })
}

fn collect_advisories(
    preflight: &Preflight,
    network: Network,
    end_state: &CapturedBlockState,
) -> Vec<Advisory> {
    let mut advisories = preflight.advisories.clone();
    for advisory in guard::advisories(network, end_state) {
        if !advisories.iter().any(|existing| existing.id == advisory.id) {
            advisories.push(advisory);
        }
    }
    advisories
}

fn write_metadata(
    writer: &mut BundleWriter,
    preflight: &Preflight,
    request: &CaptureRequest,
) -> Result<(), ReconcileError> {
    let environment = CaptureEnvironment {
        tool_name: env!("CARGO_PKG_NAME"),
        tool_version: env!("CARGO_PKG_VERSION"),
        node_implementation: preflight.node.implementation(),
        node_version: preflight.node.version(),
        node_protocol_version: preflight.node.protocolversion,
        network: request.network,
        tip_height_at_capture: preflight.tip.get(),
        anchor_height: request.interval.anchor_height().get(),
        start_height: request.interval.start_height().get(),
        end_height: request.interval.end_height().get(),
        block_count: request.interval.block_count(),
        tip_distance_required: request.tip_distance,
        captured_at: now_rfc3339()?,
        build_target: build_target(),
        rpc_url_recorded: false,
    };

    let json =
        serde_json::to_vec_pretty(&environment).map_err(|source| ReconcileError::Internal {
            reason: format!("could not serialize the capture environment: {source}"),
        })?;
    writer.write(layout::METADATA_ENVIRONMENT, &json, Encoding::Json)?;

    writer.write(
        layout::METADATA_COMMAND,
        reconstructed_command(request).as_bytes(),
        Encoding::Text,
    )?;
    writer.write(
        layout::METADATA_TOOL_VERSION,
        format!("{} {}\n", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")).as_bytes(),
        Encoding::Text,
    )
}

/// Rebuilds the command from the parsed request rather than copying the process arguments.
///
/// Reproducing what the operator typed would risk carrying a password that was passed on
/// the command line. Only the arguments that affect the evidence are recorded, and the
/// endpoint is not one of them.
fn reconstructed_command(request: &CaptureRequest) -> String {
    format!(
        "{} capture --network {} --from-height {} --to-height {} --tip-distance {} \
         --rpc-url <not recorded>\n",
        env!("CARGO_PKG_NAME"),
        request.network,
        request.interval.start_height(),
        request.interval.end_height(),
        request.tip_distance,
    )
}

fn build_manifest(
    preflight: &Preflight,
    request: &CaptureRequest,
    anchor: &CapturedBlockState,
    end_state: &CapturedBlockState,
) -> Result<Manifest, ReconcileError> {
    Ok(Manifest {
        schema_version: SCHEMA_VERSION.to_owned(),
        bundle_id: Manifest::derive_bundle_id(request.network, request.interval),
        created_at: Rfc3339Timestamp::parse(&now_rfc3339()?)?,
        tool: Tool {
            name: env!("CARGO_PKG_NAME").to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            git_commit: option_env!("ZEC_IRONWOOD_GIT_COMMIT").map(str::to_owned),
        },
        source: Source {
            implementation: preflight.node.implementation(),
            version: preflight.node.version(),
            rpc_url_redacted: true,
        },
        network: request.network,
        activation: Activation {
            upgrade: IRONWOOD_UPGRADE_NAME.to_owned(),
            expected_height: request.network.ironwood_activation_height(),
        },
        interval: request.interval.into(),
        anchor: AnchorState {
            block_hash: anchor.block_hash.clone(),
            orchard_balance_zatoshis: anchor.pools.require_balance(Pool::Orchard)?,
            ironwood_balance_zatoshis: anchor.pools.require_balance(Pool::Ironwood)?,
        },
        end: EndState {
            block_hash: end_state.block_hash.clone(),
            reported_orchard_balance_zatoshis: end_state.pools.require_balance(Pool::Orchard)?,
            reported_ironwood_balance_zatoshis: end_state.pools.require_balance(Pool::Ironwood)?,
            tracking: EndStateTracking {
                orchard_tracked_by_node: end_state.pools.monitored(Pool::Orchard),
                ironwood_tracked_by_node: end_state.pools.monitored(Pool::Ironwood),
            },
        },
        files: Vec::new(),
    })
}

/// Platform the capture ran on, recorded so a reader knows what produced a bundle.
///
/// Taken from the compiler's own constants rather than a build script, because it is
/// descriptive metadata and no hashed artifact depends on it.
fn build_target() -> String {
    format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS)
}

fn now_rfc3339() -> Result<String, ReconcileError> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|source| ReconcileError::Internal {
            reason: format!("could not format the current time: {source}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> CaptureRequest {
        CaptureRequest::new(Network::Mainnet, 3_428_143, 3_428_243, 100, None).unwrap()
    }

    #[test]
    fn the_recorded_command_omits_the_endpoint() {
        let command = reconstructed_command(&request());
        assert!(command.contains("--network mainnet"), "{command}");
        assert!(command.contains("--from-height 3428143"), "{command}");
        assert!(command.contains("<not recorded>"), "{command}");
        assert!(!command.contains("http"), "{command}");
    }

    #[test]
    fn the_current_time_formats_as_rfc3339() {
        let now = now_rfc3339().unwrap();
        assert!(Rfc3339Timestamp::parse(&now).is_ok(), "{now}");
    }

    #[test]
    fn the_build_target_is_recorded() {
        let target = build_target();
        assert!(target.contains('-'), "{target}");
    }
}
