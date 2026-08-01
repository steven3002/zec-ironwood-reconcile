//! The NU6.3 activation boundary, over a bundle that spans it.
//!
//! The three activation checks are the ones that speak directly to what ZIP 258 specifies
//! about the upgrade, and each is only meaningful on an interval positioned across the
//! boundary. Every interval captured before this fixture existed sat wholly on one side of
//! it, so all three had only ever been observed reporting *not applicable* — a state that
//! looks like coverage in a summary and is none.
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
use zec_ironwood_reconcile::commands::reconcile;

/// NU6.3 activation height on testnet, as ZIP 258 specifies and the node's upgrade table
/// independently reports.
const ACTIVATION: u32 = 4_134_000;

fn boundary_bundle() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/bundles/testnet-activation-boundary");
    assert!(
        path.is_dir(),
        "the captured boundary bundle is missing from {}",
        path.display()
    );
    path
}

fn check(reconciliation: &reconcile::Reconciliation, id: &str) -> (Status, String) {
    let check = reconciliation
        .report
        .checks
        .iter()
        .find(|check| check.id == id)
        .unwrap_or_else(|| panic!("check {id} should have been recorded"));
    (check.status, check.details.clone().unwrap_or_default())
}

#[test]
fn the_bundle_spans_the_activation_height() {
    let reconciliation = reconcile::reconcile(&boundary_bundle()).unwrap();
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
fn no_ironwood_value_moves_below_activation() {
    // Affirmative, not merely applicable: the interval contains height 4,133,999, and the
    // reconstruction from that block's own bytes yields no Ironwood movement.
    let reconciliation = reconcile::reconcile(&boundary_bundle()).unwrap();
    let (status, details) = check(&reconciliation, ids::NO_IRONWOOD_BEFORE_ACTIVATION);

    assert_eq!(
        status,
        Status::Pass,
        "expected an affirmative verdict, got {status:?}: {details}"
    );
}

#[test]
fn orchard_takes_in_no_value_at_or_after_activation() {
    let reconciliation = reconcile::reconcile(&boundary_bundle()).unwrap();
    let (status, details) = check(&reconciliation, ids::ORCHARD_WITHDRAWAL_ONLY);

    assert_eq!(
        status,
        Status::Pass,
        "expected an affirmative verdict, got {status:?}: {details}"
    );
}

#[test]
fn the_boundary_ironwood_balance_is_read_from_inside_the_interval() {
    // The block before activation is a height *within* this interval rather than its
    // anchor. Reading the boundary balance only from the anchor made this check and
    // `no_ironwood_before_activation` mutually exclusive: one needs the interval to start
    // at activation, the other needs it to start below. No bundle could then affirm both
    // halves of the boundary claim, which is the claim the tool exists to make.
    let reconciliation = reconcile::reconcile(&boundary_bundle()).unwrap();
    let (status, details) = check(&reconciliation, ids::IRONWOOD_ANCHOR_ZERO);

    assert_eq!(
        status,
        Status::Pass,
        "the bundle covers height {}, so the check must reach an affirmative verdict: {details}",
        ACTIVATION - 1
    );
}

#[test]
fn the_nodes_monitored_flag_does_not_downgrade_an_empty_ironwood_pool() {
    // Zebra reports `chainValueZat: 0, monitored: false` for Ironwood at every height
    // before the pool first receives value. That flag was read as "the node is not tracking
    // this pool, so its zero is a placeholder", which turned the boundary check into a
    // warning it could never escape. Zebra computes the flag as `chainValueZat != 0`, so it
    // restates the balance and says nothing about tracking.
    let reconciliation = reconcile::reconcile(&boundary_bundle()).unwrap();

    for id in [ids::IRONWOOD_ANCHOR_ZERO, ids::END_BALANCES_CORROBORATED] {
        let (status, details) = check(&reconciliation, id);
        assert_eq!(status, Status::Pass, "{id} was downgraded: {details}");
        assert!(
            !details.contains("placeholder"),
            "{id} still claims the node reported a placeholder: {details}"
        );
    }
}

#[test]
fn all_three_activation_checks_reach_a_verdict_on_one_bundle() {
    let reconciliation = reconcile::reconcile(&boundary_bundle()).unwrap();

    for id in [
        ids::IRONWOOD_ANCHOR_ZERO,
        ids::NO_IRONWOOD_BEFORE_ACTIVATION,
        ids::ORCHARD_WITHDRAWAL_ONLY,
    ] {
        let (status, details) = check(&reconciliation, id);
        assert_ne!(
            status,
            Status::NotApplicable,
            "{id} did not apply to an interval spanning the boundary: {details}"
        );
    }
}

#[test]
fn a_boundary_spanning_interval_reconciles_without_failure() {
    let reconciliation = reconcile::reconcile(&boundary_bundle()).unwrap();

    let failures: Vec<&str> = reconciliation
        .report
        .checks
        .iter()
        .filter(|check| check.status == Status::Fail)
        .map(|check| check.id.as_str())
        .collect();

    assert!(failures.is_empty(), "unexpected failures: {failures:?}");
    assert_eq!(
        reconciliation.report.per_height_summary.heights_diverging,
        0
    );
}
