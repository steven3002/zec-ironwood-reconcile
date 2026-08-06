//! Reconciliation and offline verification over real chain data.
//!
//! Every other test in this suite works on bytes this project constructed. These work on a
//! bundle captured from a live Zebra node covering the heights at which value first entered
//! the Ironwood pool on testnet, so they exercise the one thing synthetic fixtures cannot:
//! that extraction from a real version 6 transaction's Ironwood bundle produces the value
//! the network itself reports.
//!
//! The fixture's provenance is recorded in `tests/fixtures/PROVENANCE.md`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use zec_ironwood_reconcile::checks::{Status, ids};
use zec_ironwood_reconcile::cli::exit::ExitCode;
use zec_ironwood_reconcile::commands::{reconcile, verify};
use zec_ironwood_reconcile::domain::network::Network;
use zec_ironwood_reconcile::error::ReconcileError;
use zec_ironwood_reconcile::evidence::archive::{self, ExtractionLimits};

/// Ironwood value that first entered the pool at testnet height 4,134,683, in zatoshi.
///
/// Independently reported by Zebra as `valueDeltaZat` for that height. The whole point of
/// this file is that the tool arrives at the same figure from the block's own bytes.
const FIRST_IRONWOOD_INFLOW: &str = "125000000";

const FIXTURE_HEIGHT: u32 = 4_134_683;

fn fixture_bundle() -> PathBuf {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bundles/testnet-ironwood");
    assert!(
        path.is_dir(),
        "the captured Ironwood bundle is missing from {}",
        path.display()
    );
    path
}

/// Copies the fixture so a test may alter it without touching the committed bytes.
fn scratch_copy() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("bundle");

    let source = fixture_bundle();
    let mut stack = vec![source.clone()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(&source).unwrap();
            let destination = root.join(relative);
            if path.is_dir() {
                std::fs::create_dir_all(&destination).unwrap();
                stack.push(path);
            } else {
                std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
                std::fs::copy(&path, &destination).unwrap();
            }
        }
    }

    (dir, root)
}

#[test]
fn a_real_ironwood_bundle_reconstructs_the_value_the_network_reports() {
    // This is the assertion the project exists to be able to make.
    let reconciliation = reconcile::reconcile(&fixture_bundle()).unwrap();
    let report = &reconciliation.report;

    assert_eq!(report.network, Network::Testnet);
    assert_eq!(
        report.reconstructed.ironwood_delta_zatoshis.to_string(),
        FIRST_IRONWOOD_INFLOW,
        "the reconstructed Ironwood inflow does not match the network's own figure"
    );
    assert_eq!(
        report
            .reconstructed
            .ironwood_expected_end_zatoshis
            .to_string(),
        FIRST_IRONWOOD_INFLOW
    );
    assert_eq!(report.per_height_summary.heights_diverging, 0);
    assert_eq!(report.overall_status, Status::Pass);
}

#[test]
fn the_divergence_is_located_at_the_height_the_value_moved() {
    let reconciliation = reconcile::reconcile(&fixture_bundle()).unwrap();

    let row = reconciliation
        .report
        .per_height
        .iter()
        .find(|row| row.height.get() == FIXTURE_HEIGHT)
        .expect("the fixture should cover the height value entered the pool");

    assert_eq!(
        row.ironwood_delta_zatoshis.to_string(),
        FIRST_IRONWOOD_INFLOW
    );

    // Both comparison axes, against figures the node produced independently.
    use zec_ironwood_reconcile::reconcile::interval::Agreement;
    assert_eq!(row.ironwood_delta_agreement, Agreement::Agrees);
    assert_eq!(row.ironwood_balance_agreement, Agreement::Agrees);
    assert_eq!(row.orchard_balance_agreement, Agreement::Agrees);
}

#[test]
fn every_height_agrees_on_both_axes() {
    use zec_ironwood_reconcile::reconcile::interval::Agreement;
    let reconciliation = reconcile::reconcile(&fixture_bundle()).unwrap();

    for row in &reconciliation.report.per_height {
        for (label, agreement) in [
            ("orchard balance", row.orchard_balance_agreement),
            ("ironwood balance", row.ironwood_balance_agreement),
            ("orchard delta", row.orchard_delta_agreement),
            ("ironwood delta", row.ironwood_delta_agreement),
        ] {
            assert_eq!(
                agreement,
                Agreement::Agrees,
                "{label} disagreed at height {}",
                row.height
            );
        }
    }
}

#[test]
fn the_end_balances_are_corroborated_by_a_node_that_was_tracking_the_pools() {
    // The counterpart to the pre-activation case, where the node reports a placeholder zero
    // and the check warns instead. Here the node was tracking both pools, so agreement
    // means something and the check says so.
    let reconciliation = reconcile::reconcile(&fixture_bundle()).unwrap();

    let check = reconciliation
        .report
        .checks
        .iter()
        .find(|check| check.id == ids::END_BALANCES_CORROBORATED)
        .expect("the corroboration check should be recorded");

    assert_eq!(check.status, Status::Pass, "{:?}", check.details);
}

#[test]
fn reconciliation_is_deterministic_across_runs() {
    let first = reconcile::reconcile(&fixture_bundle()).unwrap();
    let second = reconcile::reconcile(&fixture_bundle()).unwrap();

    assert_eq!(first.report_hash, second.report_hash);
    assert_eq!(first.canonical_bytes, second.canonical_bytes);
}

#[test]
fn the_committed_bundle_reconciles_to_its_published_hash_on_every_platform() {
    // Every other determinism assertion here compares the tool against itself on one machine,
    // which cannot distinguish "reproducible" from "consistently wrong". A platform that
    // computes a different hash from identical evidence passes all of them.
    //
    // That is not hypothetical. Before the fix in `evidence::layout::to_bundle_path`, Windows
    // rendered bundle-relative paths with backslashes, matched nothing in the manifest, warned
    // that all 16 files were unlisted, and hashed that warning into the report — yielding
    // fd43e96c… against the value below, with the whole suite green.
    //
    // Pinning the literal is what makes a cross-platform CI run meaningful. If this fails,
    // find out why before changing it; the constant is only wrong if the report schema or the
    // fixture changed deliberately.
    const PUBLISHED_REPORT_HASH: &str =
        "4a5d4d7603618a80a8de29c84fe8e6fb601365f06502be374d1e43338902039e";

    let reconciliation = reconcile::reconcile(&fixture_bundle()).unwrap();

    assert_eq!(
        reconciliation.report_hash, PUBLISHED_REPORT_HASH,
        "the committed bundle no longer reconciles to its published hash"
    );
}

#[test]
fn reconciliation_does_not_modify_the_bundle() {
    // A bundle's digests must not be disturbed by reading it, or verifying a bundle would
    // change the thing being verified.
    let (_dir, root) = scratch_copy();

    let before = digest_of_every_file(&root);
    reconcile::reconcile(&root).unwrap();
    let after = digest_of_every_file(&root);

    assert_eq!(before, after, "reconciliation wrote into the bundle");
}

#[test]
fn an_archive_verifies_offline_and_reproduces_the_published_hash() {
    let (_dir, root) = scratch_copy();
    let published = reconcile::reconcile(&root).unwrap().report_hash;

    let workspace = tempfile::tempdir().unwrap();
    let archive_path = workspace.path().join("evidence.tar.zst");
    archive::pack_with_digest(&root, &archive_path).unwrap();

    let extraction = tempfile::tempdir().unwrap();
    let (outcome, reconciliation) = verify::verify_and_reconcile(
        &archive_path,
        extraction.path(),
        Some(&published),
        &ExtractionLimits::default(),
    )
    .unwrap();

    assert_eq!(outcome.hash_matches(), Some(true));
    assert_eq!(reconciliation.report_hash, published);
    assert_eq!(outcome.overall_status, Some(Status::Pass));
}

#[test]
fn a_wrong_expected_hash_is_a_mismatch_rather_than_a_pass() {
    let (_dir, root) = scratch_copy();

    let workspace = tempfile::tempdir().unwrap();
    let archive_path = workspace.path().join("evidence.tar.zst");
    archive::pack(&root, &archive_path).unwrap();

    let extraction = tempfile::tempdir().unwrap();
    let (outcome, _) = verify::verify_and_reconcile(
        &archive_path,
        extraction.path(),
        Some(&"0".repeat(64)),
        &ExtractionLimits::default(),
    )
    .unwrap();

    assert_eq!(outcome.hash_matches(), Some(false));
}

#[test]
fn altering_one_byte_of_a_block_is_refused_before_any_accounting() {
    let (_dir, root) = scratch_copy();

    let block = root.join(format!("blocks/{FIXTURE_HEIGHT}.hex"));
    let mut hex = std::fs::read_to_string(&block).unwrap();
    // Flip a hex digit deep inside the block, leaving the file well formed.
    let position = hex.len() / 2;
    let replacement = if hex.as_bytes()[position] == b'a' {
        'b'
    } else {
        'a'
    };
    hex.replace_range(position..position + 1, &replacement.to_string());
    std::fs::write(&block, hex).unwrap();

    let error = reconcile::reconcile(&root).unwrap_err();
    assert!(
        matches!(error, ReconcileError::HashMismatch { .. }),
        "expected an evidence hash failure, got {error:?}"
    );
    assert_eq!(ExitCode::from(&error), ExitCode::EvidenceUnavailable);
}

#[test]
fn a_manifest_that_misstates_its_own_anchor_is_reported() {
    // The manifest is authored by whoever produced the bundle. The arithmetic uses the
    // evidence, so a misstatement changes no figure, but it is still surfaced rather than
    // passed over.
    let (_dir, root) = scratch_copy();

    let manifest_path = root.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["anchor"]["orchard_balance_zatoshis"] = serde_json::json!("999999999");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    // The detached digest covers the manifest, so it must be removed for the bundle to load.
    std::fs::remove_file(root.join("manifest.sha256")).unwrap();

    let reconciliation = reconcile::reconcile(&root).unwrap();
    let check = reconciliation
        .report
        .checks
        .iter()
        .find(|check| check.id == ids::MANIFEST_MATCHES_EVIDENCE)
        .expect("the manifest agreement check should be recorded");

    assert_eq!(check.status, Status::Fail);
    assert!(
        check
            .details
            .as_deref()
            .unwrap_or_default()
            .contains("999999999"),
        "{:?}",
        check.details
    );

    // The reconstruction itself is unaffected, because it never read the manifest's figure.
    assert_eq!(
        reconciliation
            .report
            .reconstructed
            .ironwood_delta_zatoshis
            .to_string(),
        FIRST_IRONWOOD_INFLOW
    );
}

#[test]
fn the_report_markdown_renders_from_the_same_result() {
    let reconciliation = reconcile::reconcile(&fixture_bundle()).unwrap();
    let markdown = reconciliation.markdown();

    assert!(markdown.contains(&reconciliation.report.bundle_id));
    assert!(markdown.contains(FIRST_IRONWOOD_INFLOW));
}

fn digest_of_every_file(root: &Path) -> std::collections::BTreeMap<String, String> {
    use zec_ironwood_reconcile::evidence::hashing;

    let mut digests = std::collections::BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root).unwrap().display().to_string();
                digests.insert(relative, hashing::sha256_file(&path).unwrap());
            }
        }
    }
    digests
}

/// Rewrites the manifest's per-file digests to match what is on disk.
///
/// A bundle's author can always reseal it, so a tampering test that only trips the digest
/// check proves nothing about whether the contents are examined. The detached digest is
/// removed because it covers the manifest this rewrites.
fn reseal(root: &Path) {
    let manifest_path = root.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();

    for entry in manifest["files"].as_array_mut().unwrap() {
        let relative = entry["path"].as_str().unwrap().to_owned();
        let path = root.join(&relative);
        if path.is_file() {
            let bytes = std::fs::read(&path).unwrap();
            entry["sha256"] =
                serde_json::json!(zec_ironwood_reconcile::canonical::sha256_hex(&bytes));
            entry["size_bytes"] = serde_json::json!(bytes.len());
        }
    }

    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let _ = std::fs::remove_file(root.join("manifest.sha256"));
}

#[test]
fn a_height_absent_from_the_bundle_is_unusable_evidence_not_a_filesystem_fault() {
    // A bundle whose manifest indexes an interval but omits one of its heights describes
    // the bundle, not this machine. Reporting it as a filesystem error hands a caller an
    // exit code about the disk and a message naming an absolute path rather than the
    // missing height.
    let (_dir, root) = scratch_copy();

    let missing = format!("blocks/{}.hex", FIXTURE_HEIGHT + 1);
    std::fs::remove_file(root.join(&missing)).unwrap();

    let manifest_path = root.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["files"]
        .as_array_mut()
        .unwrap()
        .retain(|entry| entry["path"].as_str() != Some(missing.as_str()));
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    reseal(&root);

    let error = reconcile::reconcile(&root).unwrap_err();
    assert!(
        matches!(&error, ReconcileError::MissingFile { path } if path == &missing),
        "expected the missing height to be named, got {error:?}"
    );
    assert_eq!(ExitCode::from(&error), ExitCode::EvidenceUnavailable);
}

#[test]
fn a_pool_state_file_belonging_to_another_height_is_refused() {
    // Nothing in a bundle's file names ties a pool record to the block it describes, and
    // resealing the manifest is always available to whoever produced the bundle. The record
    // states its own height and block hash, so the binding is checked rather than assumed.
    let (_dir, root) = scratch_copy();

    let donor = root.join(format!("blocks/{}.pools.json", FIXTURE_HEIGHT + 1));
    let target = root.join(format!("blocks/{FIXTURE_HEIGHT}.pools.json"));
    std::fs::copy(&donor, &target).unwrap();
    reseal(&root);

    let error = reconcile::reconcile(&root).unwrap_err();
    match &error {
        ReconcileError::EvidenceInconsistent { path, reason } => {
            assert!(path.contains(&FIXTURE_HEIGHT.to_string()), "{path}");
            assert!(
                reason.contains(&(FIXTURE_HEIGHT + 1).to_string()),
                "{reason}"
            );
        }
        other => panic!("expected an evidence inconsistency, got {other:?}"),
    }
    assert_eq!(ExitCode::from(&error), ExitCode::EvidenceUnavailable);
}

#[test]
fn a_pool_state_file_describing_a_different_block_is_refused() {
    // The height agrees here, so only the block-hash binding can catch it. The hash used is
    // the one this crate computes from the block's own header, never one the bundle asserts.
    let (_dir, root) = scratch_copy();

    let target = root.join(format!("blocks/{FIXTURE_HEIGHT}.pools.json"));
    let mut state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&target).unwrap()).unwrap();
    state["hash"] =
        serde_json::json!("0000000000000000000000000000000000000000000000000000000000000001");
    std::fs::write(&target, serde_json::to_vec(&state).unwrap()).unwrap();
    reseal(&root);

    let error = reconcile::reconcile(&root).unwrap_err();
    assert!(
        matches!(&error, ReconcileError::EvidenceInconsistent { reason, .. }
            if reason.contains("0000000000000000000000000000000000000000000000000000000000000001")),
        "expected the declared hash to be named, got {error:?}"
    );
    assert_eq!(ExitCode::from(&error), ExitCode::EvidenceUnavailable);
}

/// Consensus header word of the fixture's first transaction: the overwintered bit, then
/// version 6.
const VERSION_6_HEADER_WORD: u32 = 0x8000_0006;

/// The same word with the version raised to 7, which no transaction format defines.
///
/// The version group identifier that follows is left untouched, so the pair the reader
/// matches on describes a version it does not implement rather than a malformed field.
const UNRECOGNISED_VERSION_HEADER_WORD: u32 = 0x8000_0007;

/// Offset of the first transaction's version header within a captured block.
///
/// A block is a fixed 140-byte header, version, three 32-byte hashes, `nTime`, `nBits` and
/// a 32-byte nonce, followed by the length-prefixed Equihash solution, then a compact-size
/// transaction count. The `(200, 9)` solution these networks use is 1,344 bytes and carries
/// a three-byte length prefix; the fixture holds one transaction, so its count occupies a
/// single byte.
///
/// The caller asserts that the word found here is the version it expects before altering it,
/// so an offset that ever stopped being right fails the test rather than mutating an
/// arbitrary part of the block.
const FIRST_TRANSACTION_OFFSET: usize = (4 + 32 + 32 + 32 + 4 + 4 + 32) + 3 + 1344 + 1;

#[test]
fn a_transaction_declaring_an_unrecognised_version_is_refused() {
    // Closes the last of the fixture scenarios. An unknown version is rejected during
    // deserialization, but reaching that path needs a well-formed block around it, a valid
    // header, Equihash solution and coinbase, which cannot be constructed without real
    // chain data. Mutating a captured block's version field is what makes it reachable.
    let (_dir, root) = scratch_copy();
    let block_path = root.join(format!("blocks/{FIXTURE_HEIGHT}.hex"));

    let mut block = hex::decode(std::fs::read_to_string(&block_path).unwrap().trim()).unwrap();
    let version_field = FIRST_TRANSACTION_OFFSET..FIRST_TRANSACTION_OFFSET + 4;
    assert_eq!(
        u32::from_le_bytes(block[version_field.clone()].try_into().unwrap()),
        VERSION_6_HEADER_WORD,
        "the first transaction is not where this test expects it, so the mutation below \
         would alter unrelated bytes"
    );

    // The control. Resealing rewrites the manifest, so without this a refusal after the
    // mutation could as easily be an artifact of copying and resealing the bundle.
    reseal(&root);
    reconcile::reconcile(&root).expect("the unmutated bundle must still reconcile");

    block[version_field].copy_from_slice(&UNRECOGNISED_VERSION_HEADER_WORD.to_le_bytes());
    std::fs::write(&block_path, hex::encode(&block)).unwrap();
    reseal(&root);

    let error = reconcile::reconcile(&root).unwrap_err();
    assert!(
        matches!(&error, ReconcileError::TransactionParse { height, .. } if *height == FIXTURE_HEIGHT),
        "expected the offending height to be named, got {error:?}"
    );
    assert_eq!(ExitCode::from(&error), ExitCode::UnsupportedTransaction);
    assert_eq!(ExitCode::from(&error).code(), 5);
}

#[test]
fn the_report_records_the_build_that_reconciled_it_not_the_one_named_by_the_bundle() {
    // Check semantics decide every verdict and therefore the report hash, so two builds can
    // reconcile one bundle to two different hashes. The only version the report used to
    // carry came from the manifest, a field the bundle's author writes, so a verifier
    // comparing hashes could not tell a difference in evidence from a difference in builds.
    let (_dir, root) = scratch_copy();

    let manifest_path = root.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["tool"]["version"] = serde_json::json!("0.0.1-supplied-by-the-bundle");
    std::fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::remove_file(root.join("manifest.sha256")).unwrap();

    let report = reconcile::reconcile(&root).unwrap().report;

    assert_eq!(report.tool_version, "0.0.1-supplied-by-the-bundle");
    assert_eq!(report.reconciled_by_version, env!("CARGO_PKG_VERSION"));
    assert_ne!(
        report.reconciled_by_version, report.tool_version,
        "the reconciling build's version must not be taken from the bundle"
    );
}
