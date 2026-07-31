//! Checks specific to the NU6.3 activation boundary.
//!
//! These properties are checkable because of what ZIP 258 specifies about the upgrade, and
//! do not generalise to arbitrary historical intervals. Where a property does not apply to
//! the interval under examination, it is recorded as not applicable with a stated reason
//! rather than silently passed.

use crate::checks::{Check, CheckRegistry, ids};
use crate::domain::height::HeightInterval;
use crate::domain::network::Network;
use crate::domain::zatoshi::Zatoshi;
use crate::reconcile::interval::{AnchorBalances, IntervalOutcome};

/// Records the activation-context verdicts for an interval.
pub fn evaluate(
    outcome: &IntervalOutcome,
    network: Network,
    declared_activation_height: Option<u32>,
    registry: &mut CheckRegistry,
) {
    check_activation_context(network, declared_activation_height, registry);
    check_ironwood_anchor_zero(outcome, network, registry);
    check_no_ironwood_before_activation(outcome, network, registry);
    check_orchard_withdrawal_only(outcome, network, registry);

    // Branch identifiers and transaction versions are validated during parsing; any
    // violation aborts before reconciliation. Recording them states what was verified.
    registry.record(Check::pass(ids::CONSENSUS_BRANCH_ID_VALID));
    registry.record(Check::pass(ids::TRANSACTION_VERSIONS_RECOGNIZED));
}

/// The bundle's declared activation height matches the protocol constant for its network.
fn check_activation_context(network: Network, declared: Option<u32>, registry: &mut CheckRegistry) {
    let expected = network.ironwood_activation_height().get();
    match declared {
        None => registry.record(Check::pass(ids::ACTIVATION_CONTEXT_VALID)),
        Some(height) if height == expected => {
            registry.record(Check::pass(ids::ACTIVATION_CONTEXT_VALID));
        }
        Some(height) => registry.record(Check::fail(
            ids::ACTIVATION_CONTEXT_VALID,
            format!(
                "bundle declares activation height {height} for {network}, but ZIP 258 specifies {expected}"
            ),
        )),
    }
}

/// The Ironwood pool holds nothing at an anchor immediately preceding activation.
///
/// ZIP 258 introduces the pool empty. An interval anchored elsewhere cannot assert this.
fn check_ironwood_anchor_zero(
    outcome: &IntervalOutcome,
    network: Network,
    registry: &mut CheckRegistry,
) {
    let activation = network.ironwood_activation_height();
    let anchor_height = outcome.interval.anchor_height();

    let Ok(pre_activation) = activation.checked_previous() else {
        registry.record(Check::not_applicable(
            ids::IRONWOOD_ANCHOR_ZERO,
            "activation height has no preceding block",
        ));
        return;
    };

    if anchor_height != pre_activation {
        registry.record(Check::not_applicable(
            ids::IRONWOOD_ANCHOR_ZERO,
            format!(
                "anchor height {anchor_height} does not immediately precede activation at {activation}"
            ),
        ));
        return;
    }

    if outcome.anchor.ironwood == Zatoshi::ZERO {
        registry.record(Check::pass(ids::IRONWOOD_ANCHOR_ZERO));
    } else {
        registry.record(Check::fail(
            ids::IRONWOOD_ANCHOR_ZERO,
            format!(
                "Ironwood pool is declared as {} zatoshi at the block before activation, expected zero",
                outcome.anchor.ironwood
            ),
        ));
    }
}

/// No Ironwood value movement appears at a height below activation.
fn check_no_ironwood_before_activation(
    outcome: &IntervalOutcome,
    network: Network,
    registry: &mut CheckRegistry,
) {
    let pre_activation: Vec<_> = outcome
        .heights
        .iter()
        .filter(|height| !network.is_post_activation(height.height))
        .collect();

    if pre_activation.is_empty() {
        registry.record(Check::not_applicable(
            ids::NO_IRONWOOD_BEFORE_ACTIVATION,
            "the interval contains no pre-activation heights",
        ));
        return;
    }

    match pre_activation
        .iter()
        .find(|height| height.reconstructed_ironwood_delta != Zatoshi::ZERO)
    {
        None => registry.record(Check::pass(ids::NO_IRONWOOD_BEFORE_ACTIVATION)),
        Some(height) => registry.record(Check::fail(
            ids::NO_IRONWOOD_BEFORE_ACTIVATION,
            format!(
                "Ironwood movement of {} zatoshi at height {}, below activation",
                height.reconstructed_ironwood_delta, height.height
            ),
        )),
    }
}

/// No new value may enter the Orchard pool at or after activation (ZIP 258).
///
/// A positive per-block Orchard delta post-activation would be a consensus violation, so
/// this is a substantive finding rather than a formality.
fn check_orchard_withdrawal_only(
    outcome: &IntervalOutcome,
    network: Network,
    registry: &mut CheckRegistry,
) {
    let offender = outcome
        .heights
        .iter()
        .filter(|height| network.is_post_activation(height.height))
        .find(|height| {
            !height.reconstructed_orchard_delta.is_negative()
                && height.reconstructed_orchard_delta != Zatoshi::ZERO
        });

    match offender {
        None => registry.record(Check::pass(ids::ORCHARD_WITHDRAWAL_ONLY)),
        Some(height) => registry.record(Check::fail(
            ids::ORCHARD_WITHDRAWAL_ONLY,
            format!(
                "value of {} zatoshi entered the Orchard pool at height {}, which is at or after activation",
                height.reconstructed_orchard_delta, height.height
            ),
        )),
    }
}

/// Convenience for constructing anchor balances in the shape this module expects.
pub const fn anchor(orchard: Zatoshi, ironwood: Zatoshi) -> AnchorBalances {
    AnchorBalances { orchard, ironwood }
}

/// Whether an interval is anchored exactly at the activation boundary.
pub fn is_activation_anchored(interval: HeightInterval, network: Network) -> bool {
    network
        .ironwood_activation_height()
        .checked_previous()
        .is_ok_and(|expected| interval.anchor_height() == expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::Status;
    use crate::domain::height::BlockHeight;
    use crate::reconcile::interval::reconcile_interval;
    use crate::reconcile::ledger::BlockLedger;
    use std::collections::BTreeMap;

    const ACTIVATION: u32 = 3_428_143;

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

    fn outcome(ledgers: &[BlockLedger], anchor_ironwood: i64) -> IntervalOutcome {
        let start = ledgers.first().unwrap().height;
        let end = ledgers.last().unwrap().height;
        let interval = HeightInterval::new(start, end).unwrap();
        reconcile_interval(
            ledgers,
            interval,
            AnchorBalances {
                orchard: Zatoshi::from_raw(366_000_000_000_000),
                ironwood: Zatoshi::from_raw(anchor_ironwood),
            },
            &BTreeMap::new(),
        )
        .unwrap()
    }

    fn status_of(registry: &CheckRegistry, id: &str) -> Status {
        registry
            .checks()
            .iter()
            .find(|check| check.id == id)
            .map(|check| check.status)
            .expect("check should have been recorded")
    }

    #[test]
    fn an_activation_anchored_interval_passes_every_activation_check() {
        let ledgers = vec![ledger(ACTIVATION, -8_000, 8_000)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            Network::Mainnet,
            Some(ACTIVATION),
            &mut registry,
        );
        assert!(!registry.has_failures(), "{:?}", registry.checks());
        assert_eq!(
            status_of(&registry, ids::IRONWOOD_ANCHOR_ZERO),
            Status::Pass
        );
    }

    #[test]
    fn a_nonzero_ironwood_anchor_at_activation_fails() {
        let ledgers = vec![ledger(ACTIVATION, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(&outcome(&ledgers, 1), Network::Mainnet, None, &mut registry);
        assert_eq!(
            status_of(&registry, ids::IRONWOOD_ANCHOR_ZERO),
            Status::Fail
        );
    }

    #[test]
    fn an_interval_anchored_elsewhere_marks_the_anchor_check_not_applicable() {
        let ledgers = vec![ledger(ACTIVATION + 500, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 42),
            Network::Mainnet,
            None,
            &mut registry,
        );
        assert_eq!(
            status_of(&registry, ids::IRONWOOD_ANCHOR_ZERO),
            Status::NotApplicable
        );
        assert!(!registry.has_failures());
    }

    #[test]
    fn value_entering_orchard_after_activation_fails() {
        let ledgers = vec![ledger(ACTIVATION, 5_000, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(&outcome(&ledgers, 0), Network::Mainnet, None, &mut registry);

        let check = registry
            .checks()
            .iter()
            .find(|check| check.id == ids::ORCHARD_WITHDRAWAL_ONLY)
            .unwrap();
        assert_eq!(check.status, Status::Fail);
        assert!(
            check
                .details
                .as_ref()
                .unwrap()
                .contains(&ACTIVATION.to_string())
        );
    }

    #[test]
    fn value_leaving_orchard_after_activation_passes() {
        let ledgers = vec![ledger(ACTIVATION, -5_000, 5_000)];
        let mut registry = CheckRegistry::new();
        evaluate(&outcome(&ledgers, 0), Network::Mainnet, None, &mut registry);
        assert_eq!(
            status_of(&registry, ids::ORCHARD_WITHDRAWAL_ONLY),
            Status::Pass
        );
    }

    #[test]
    fn an_all_post_activation_interval_marks_the_pre_activation_check_not_applicable() {
        let ledgers = vec![ledger(ACTIVATION, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(&outcome(&ledgers, 0), Network::Mainnet, None, &mut registry);
        assert_eq!(
            status_of(&registry, ids::NO_IRONWOOD_BEFORE_ACTIVATION),
            Status::NotApplicable
        );
    }

    #[test]
    fn ironwood_movement_below_activation_fails() {
        let ledgers = vec![ledger(ACTIVATION - 2, 0, 100), ledger(ACTIVATION - 1, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(&outcome(&ledgers, 0), Network::Mainnet, None, &mut registry);
        assert_eq!(
            status_of(&registry, ids::NO_IRONWOOD_BEFORE_ACTIVATION),
            Status::Fail
        );
    }

    #[test]
    fn a_wrong_declared_activation_height_fails() {
        let ledgers = vec![ledger(ACTIVATION, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            Network::Mainnet,
            Some(3_000_000),
            &mut registry,
        );
        assert_eq!(
            status_of(&registry, ids::ACTIVATION_CONTEXT_VALID),
            Status::Fail
        );
    }

    #[test]
    fn testnet_uses_its_own_activation_height() {
        let ledgers = vec![ledger(4_134_000, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            Network::Testnet,
            Some(4_134_000),
            &mut registry,
        );
        assert_eq!(
            status_of(&registry, ids::ACTIVATION_CONTEXT_VALID),
            Status::Pass
        );
    }

    #[test]
    fn activation_anchoring_is_recognised_per_network() {
        let mainnet = HeightInterval::new(
            BlockHeight::new(ACTIVATION),
            BlockHeight::new(ACTIVATION + 100),
        )
        .unwrap();
        assert!(is_activation_anchored(mainnet, Network::Mainnet));
        assert!(!is_activation_anchored(mainnet, Network::Testnet));
    }

    #[test]
    fn the_anchor_helper_builds_the_expected_shape() {
        let balances = anchor(Zatoshi::from_raw(5), Zatoshi::ZERO);
        assert_eq!(balances.orchard, Zatoshi::from_raw(5));
        assert_eq!(balances.ironwood, Zatoshi::ZERO);
    }
}
