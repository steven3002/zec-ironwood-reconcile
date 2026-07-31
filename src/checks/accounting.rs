//! Checks over the reconstruction itself.

use crate::checks::{Check, CheckRegistry, ids};
use crate::domain::pool::Pool;
use crate::domain::zatoshi::Zatoshi;
use crate::reconcile::interval::IntervalOutcome;

/// Records every accounting verdict for a completed interval reconciliation.
///
/// Reaching this point already implies no arithmetic overflow occurred, because
/// accumulation returns an error rather than a wrapped value; the check is recorded
/// explicitly so the report states it rather than leaving it implied.
pub fn evaluate(
    outcome: &IntervalOutcome,
    reported_end_orchard: Option<Zatoshi>,
    reported_end_ironwood: Option<Zatoshi>,
    registry: &mut CheckRegistry,
) {
    registry.record(Check::pass(ids::ORCHARD_DELTAS_RECONSTRUCTED));
    registry.record(Check::pass(ids::IRONWOOD_DELTAS_RECONSTRUCTED));
    registry.record(Check::pass(ids::NO_ARITHMETIC_OVERFLOW));

    check_no_negative_balance(outcome, registry);
    check_per_height_agreement(outcome, registry);
    check_end_balances(
        outcome,
        reported_end_orchard,
        reported_end_ironwood,
        registry,
    );
}

/// No reconstructed running balance may be negative at any height.
///
/// This re-derives the ZIP 209 non-negativity rule, as extended to Ironwood by ZIP 258,
/// from transaction data rather than trusting the node that enforced it.
fn check_no_negative_balance(outcome: &IntervalOutcome, registry: &mut CheckRegistry) {
    let offender = outcome.heights.iter().find_map(|height| {
        Pool::RECONSTRUCTED.into_iter().find_map(|pool| {
            height
                .expected_balance(pool)
                .filter(|balance| balance.is_negative())
                .map(|balance| (height.height, pool, balance))
        })
    });

    match offender {
        Some((height, pool, balance)) => registry.record(Check::fail(
            ids::NO_NEGATIVE_POOL_BALANCE,
            format!(
                "reconstructed {pool} balance is negative at height {height}: {balance} zatoshi"
            ),
        )),
        None => registry.record(Check::pass(ids::NO_NEGATIVE_POOL_BALANCE)),
    }
}

/// Both per-height comparison axes, against the values the node reported.
fn check_per_height_agreement(outcome: &IntervalOutcome, registry: &mut CheckRegistry) {
    let mut balance_failures = Vec::new();
    let mut delta_failures = Vec::new();

    for height in &outcome.heights {
        for pool in Pool::RECONSTRUCTED {
            if height.balance_agreement(pool).differs() {
                balance_failures.push((height.height, pool));
            }
            if height.delta_agreement(pool).differs() {
                delta_failures.push((height.height, pool));
            }
        }
    }

    record_divergences(
        ids::PER_HEIGHT_BALANCES_MATCH,
        "balance",
        &balance_failures,
        registry,
    );
    record_divergences(
        ids::PER_HEIGHT_DELTAS_MATCH,
        "delta",
        &delta_failures,
        registry,
    );
}

/// Reports divergences, naming the first one.
///
/// The first divergence is where a discrepancy originates. Reporting it is the diagnostic
/// an endpoint-only comparison cannot provide.
fn record_divergences(
    id: &str,
    axis: &str,
    divergences: &[(crate::domain::height::BlockHeight, Pool)],
    registry: &mut CheckRegistry,
) {
    match divergences.first() {
        None => registry.record(Check::pass(id)),
        Some((height, pool)) => registry.record(Check::fail(
            id,
            format!(
                "{} {axis} divergence(s); first at height {height} in the {pool} pool",
                divergences.len()
            ),
        )),
    }
}

/// The interval endpoints, which are what the core claim is stated in terms of.
fn check_end_balances(
    outcome: &IntervalOutcome,
    reported_orchard: Option<Zatoshi>,
    reported_ironwood: Option<Zatoshi>,
    registry: &mut CheckRegistry,
) {
    for (pool, id, expected, reported) in [
        (
            Pool::Orchard,
            ids::ORCHARD_END_BALANCE_MATCHES,
            outcome.expected_end_orchard,
            reported_orchard,
        ),
        (
            Pool::Ironwood,
            ids::IRONWOOD_END_BALANCE_MATCHES,
            outcome.expected_end_ironwood,
            reported_ironwood,
        ),
    ] {
        match reported {
            Some(value) if value == expected => registry.record(Check::pass(id)),
            Some(value) => registry.record(Check::fail(
                id,
                format!(
                    "reconstructed {pool} ending balance {expected} does not match the reported {value}"
                ),
            )),
            None => registry.record(Check::fail(
                id,
                format!("the capture recorded no reported {pool} ending balance to compare against"),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::height::{BlockHeight, HeightInterval};
    use crate::domain::pool_state::ReportedPoolState;
    use crate::reconcile::interval::{AnchorBalances, reconcile_interval};
    use crate::reconcile::ledger::BlockLedger;
    use std::collections::BTreeMap;

    fn ledger(height: u32, orchard: i64, ironwood: i64) -> BlockLedger {
        BlockLedger {
            height: BlockHeight::new(height),
            block_hash: format!("{height:064x}"),
            previous_block_hash: format!("{:064x}", height.saturating_sub(1)),
            orchard_delta: Zatoshi::from_raw(orchard),
            ironwood_delta: Zatoshi::from_raw(ironwood),
            transactions: Vec::new(),
        }
    }

    fn outcome_with(
        ledgers: &[BlockLedger],
        anchor_orchard: i64,
        reported: BTreeMap<BlockHeight, ReportedPoolState>,
    ) -> IntervalOutcome {
        let interval = HeightInterval::new(
            BlockHeight::new(101),
            BlockHeight::new(100 + ledgers.len() as u32),
        )
        .unwrap();
        reconcile_interval(
            ledgers,
            interval,
            AnchorBalances {
                orchard: Zatoshi::from_raw(anchor_orchard),
                ironwood: Zatoshi::ZERO,
            },
            &reported,
        )
        .unwrap()
    }

    fn status_of(registry: &CheckRegistry, id: &str) -> crate::checks::Status {
        registry
            .checks()
            .iter()
            .find(|check| check.id == id)
            .map(|check| check.status)
            .expect("check should have been recorded")
    }

    #[test]
    fn a_clean_reconciliation_passes_every_accounting_check() {
        let ledgers = vec![ledger(101, -300, 300)];
        let outcome = outcome_with(&ledgers, 1_000, BTreeMap::new());

        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome,
            Some(Zatoshi::from_raw(700)),
            Some(Zatoshi::from_raw(300)),
            &mut registry,
        );

        assert!(!registry.has_failures(), "{:?}", registry.checks());
    }

    #[test]
    fn a_mismatched_ending_balance_fails_and_never_passes() {
        let ledgers = vec![ledger(101, -300, 300)];
        let outcome = outcome_with(&ledgers, 1_000, BTreeMap::new());

        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome,
            Some(Zatoshi::from_raw(999)),
            Some(Zatoshi::from_raw(300)),
            &mut registry,
        );

        assert_eq!(
            status_of(&registry, ids::ORCHARD_END_BALANCE_MATCHES),
            crate::checks::Status::Fail
        );
        assert!(registry.has_failures());
    }

    #[test]
    fn an_absent_reported_ending_balance_fails_rather_than_passing_vacuously() {
        let ledgers = vec![ledger(101, 0, 0)];
        let outcome = outcome_with(&ledgers, 1_000, BTreeMap::new());

        let mut registry = CheckRegistry::new();
        evaluate(&outcome, None, None, &mut registry);

        assert_eq!(
            status_of(&registry, ids::ORCHARD_END_BALANCE_MATCHES),
            crate::checks::Status::Fail
        );
        assert_eq!(
            status_of(&registry, ids::IRONWOOD_END_BALANCE_MATCHES),
            crate::checks::Status::Fail
        );
    }

    #[test]
    fn a_negative_reconstructed_balance_is_detected_with_its_height() {
        let ledgers = vec![ledger(101, -5_000, 0)];
        let outcome = outcome_with(&ledgers, 1_000, BTreeMap::new());

        let mut registry = CheckRegistry::new();
        evaluate(&outcome, None, None, &mut registry);

        let check = registry
            .checks()
            .iter()
            .find(|check| check.id == ids::NO_NEGATIVE_POOL_BALANCE)
            .unwrap();
        assert_eq!(check.status, crate::checks::Status::Fail);
        assert!(check.details.as_ref().unwrap().contains("101"));
        assert!(check.details.as_ref().unwrap().contains("orchard"));
    }

    #[test]
    fn a_per_height_delta_divergence_names_the_first_offending_height() {
        let ledgers = vec![ledger(101, -300, 0), ledger(102, -200, 0)];
        let mut reported = BTreeMap::new();
        reported.insert(
            BlockHeight::new(102),
            ReportedPoolState::new(BlockHeight::new(102))
                .with_delta(Pool::Orchard, Zatoshi::from_raw(-999)),
        );
        let outcome = outcome_with(&ledgers, 10_000, reported);

        let mut registry = CheckRegistry::new();
        evaluate(&outcome, None, None, &mut registry);

        let check = registry
            .checks()
            .iter()
            .find(|check| check.id == ids::PER_HEIGHT_DELTAS_MATCH)
            .unwrap();
        assert_eq!(check.status, crate::checks::Status::Fail);
        assert!(check.details.as_ref().unwrap().contains("102"));
    }

    #[test]
    fn unreported_per_height_values_do_not_produce_a_divergence() {
        let ledgers = vec![ledger(101, -300, 300)];
        let outcome = outcome_with(&ledgers, 1_000, BTreeMap::new());

        let mut registry = CheckRegistry::new();
        evaluate(&outcome, None, None, &mut registry);

        assert_eq!(
            status_of(&registry, ids::PER_HEIGHT_BALANCES_MATCH),
            crate::checks::Status::Pass
        );
    }
}
