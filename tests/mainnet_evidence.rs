//! The committed mainnet bundle, over the NU6.3 activation boundary.
//!
//! Every other bundle in this suite was captured from testnet, where value entered the
//! Ironwood pool 683 blocks after activation and arrived as coinbase issuance with the
//! Orchard pool unmoved. Mainnet funded Ironwood differently, in the first block after
//! activation, by moving value out of Orchard. A bundle that shows only one of the two
//! mechanisms cannot demonstrate that the reconstruction handles both.
//!
//! The fixture's provenance is recorded in `tests/fixtures/PROVENANCE.md`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use zec_ironwood_reconcile::checks::Status;
use zec_ironwood_reconcile::commands::reconcile;
use zec_ironwood_reconcile::domain::network::Network;
use zec_ironwood_reconcile::reconcile::interval::Agreement;

/// NU6.3 activation height on mainnet, as ZIP 258 specifies and the node's upgrade table
/// independently reports.
const ACTIVATION: u32 = 3_428_143;

/// The first mainnet height at which the Ironwood pool held value.
const FIRST_INFLOW_HEIGHT: u32 = 3_428_144;

/// Value that entered Ironwood at [`FIRST_INFLOW_HEIGHT`], in zatoshi.
const FIRST_INFLOW: &str = "1000000";

/// Value that left Orchard at the same height, in zatoshi.
///
/// Larger than the inflow: the difference is a fee, and the node reports it arriving in the
/// transparent pool. The two figures are separate observations rather than a balance, so
/// this records what mainnet did at one height and nothing more general.
const FIRST_OUTFLOW: &str = "-1020000";

fn mainnet_bundle() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/bundles/mainnet-activation-boundary");
    assert!(
        path.is_dir(),
        "the captured mainnet bundle is missing from {}",
        path.display()
    );
    path
}

#[test]
fn the_committed_bundle_reconciles_to_its_published_hash_on_every_platform() {
    // The counterpart of the same assertion over the testnet bundle in `end_to_end.rs`, and
    // the reason for repeating it: a pinned literal is the only check here that compares the
    // tool against something other than itself. Determinism assertions run one build against
    // another run of the same build on the same machine, which cannot tell "reproducible"
    // from "consistently wrong", and a Windows build once passed the whole suite while
    // computing a different hash.
    //
    // Pinning the testnet hash does not cover this bundle. The report carries the network,
    // the interval and every check verdict, all of which differ here, so a defect reachable
    // only from mainnet evidence would leave the testnet literal intact.
    //
    // If this fails, find out why before changing it; the constant is only wrong if the
    // report schema or the fixture changed deliberately.
    const PUBLISHED_REPORT_HASH: &str =
        "0a2ca229afb716ca77e3857c5f0a0700a8d36ee2a99b9235fec58cdb1fdc78db";

    let reconciliation = reconcile::reconcile(&mainnet_bundle()).unwrap();

    assert_eq!(
        reconciliation.report_hash, PUBLISHED_REPORT_HASH,
        "the committed mainnet bundle no longer reconciles to its published hash"
    );
}

#[test]
fn the_report_describes_mainnet() {
    let report = reconcile::reconcile(&mainnet_bundle()).unwrap().report;

    assert_eq!(report.network, Network::Mainnet);
    assert_eq!(report.bundle_id, "mainnet-3428141-3428146");
}

#[test]
fn the_bundle_spans_the_mainnet_activation_height() {
    let reconciliation = reconcile::reconcile(&mainnet_bundle()).unwrap();
    let interval = &reconciliation.report.interval;

    assert!(
        interval.start_height.get() < ACTIVATION,
        "the interval must begin below activation for the pre-activation check to apply"
    );
    assert!(
        interval.end_height.get() >= ACTIVATION,
        "the interval must reach activation for the withdrawal-only rule to apply"
    );
}

#[test]
fn the_first_mainnet_ironwood_inflow_is_reconstructed_from_the_blocks_own_bytes() {
    // The assertion this fixture exists to support. Testnet establishes that the tool reads
    // an Ironwood bundle funded by issuance; only mainnet data establishes that it reads one
    // funded by a movement out of Orchard, and the two produce different transaction shapes.
    let reconciliation = reconcile::reconcile(&mainnet_bundle()).unwrap();

    let row = reconciliation
        .report
        .per_height
        .iter()
        .find(|row| row.height.get() == FIRST_INFLOW_HEIGHT)
        .expect("the fixture should cover the height value first entered the pool");

    assert_eq!(row.ironwood_delta_zatoshis.to_string(), FIRST_INFLOW);
    assert_eq!(row.orchard_delta_zatoshis.to_string(), FIRST_OUTFLOW);

    // Both figures agree with what the node reported for the same height, on both axes.
    assert_eq!(row.ironwood_delta_agreement, Agreement::Agrees);
    assert_eq!(row.ironwood_balance_agreement, Agreement::Agrees);
    assert_eq!(row.orchard_delta_agreement, Agreement::Agrees);
    assert_eq!(row.orchard_balance_agreement, Agreement::Agrees);
}

#[test]
fn ironwood_holds_no_value_until_the_block_after_activation() {
    // Recorded as an observation of this interval, not as a rule. Testnet's first inflow was
    // 683 blocks past activation, so the height at which a network funds Ironwood is not
    // fixed by the upgrade and must not be asserted as though it were.
    let reconciliation = reconcile::reconcile(&mainnet_bundle()).unwrap();

    let below_first_inflow = reconciliation
        .report
        .per_height
        .iter()
        .filter(|row| row.height.get() < FIRST_INFLOW_HEIGHT);

    for row in below_first_inflow {
        assert_eq!(
            row.ironwood_expected_balance_zatoshis.to_string(),
            "0",
            "Ironwood held value at height {}, below the first inflow",
            row.height
        );
    }
}

#[test]
fn every_height_agrees_on_both_axes() {
    let reconciliation = reconcile::reconcile(&mainnet_bundle()).unwrap();

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

    assert_eq!(
        reconciliation.report.per_height_summary.heights_diverging,
        0
    );
}

#[test]
fn every_check_reaches_an_affirmative_verdict() {
    // An interval positioned wholly on one side of the activation boundary leaves the three
    // activation checks reporting *not applicable*, which reads like coverage in a summary
    // and is none. The testnet boundary bundle escapes that but has both pools motionless at
    // every height, so it affirms the boundary rules against a ledger that never moves. This
    // interval is the one that does both, which makes a verdict other than Pass anywhere in
    // the report a regression rather than a property of the evidence.
    let reconciliation = reconcile::reconcile(&mainnet_bundle()).unwrap();

    let unaffirmed: Vec<(&str, Status)> = reconciliation
        .report
        .checks
        .iter()
        .filter(|check| check.status != Status::Pass)
        .map(|check| (check.id.as_str(), check.status))
        .collect();

    assert!(
        unaffirmed.is_empty(),
        "not all checks passed: {unaffirmed:?}"
    );
    assert_eq!(reconciliation.report.overall_status, Status::Pass);
}
