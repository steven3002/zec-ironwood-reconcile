//! Reconciliation and offline verification over real chain data.
//!
//! Every other test in this suite works on bytes this project constructed. These work on a
//! bundle captured from a live Zebra node covering the heights at which value first entered
//! the Ironwood pool on testnet — so they exercise the one thing synthetic fixtures cannot:
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
    // evidence, so a misstatement changes no figure — but it is still surfaced rather than
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
