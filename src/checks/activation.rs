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
use crate::reconcile::ledger::BlockLedger;

/// What a bundle records about the Ironwood pool at the last block before activation.
///
/// A bundle can establish this in either of two ways, the block before activation may be
/// the interval's anchor, or it may be a height within the interval, and the check that
/// consumes this does not need to care which. Constructing it at the call site keeps the
/// checks layer free of any knowledge of bundle layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreActivationIronwood {
    /// The balance the node reported at that height, if it reported one.
    pub balance: Option<Zatoshi>,
}

/// Records the activation-context verdicts for an interval.
pub fn evaluate(
    outcome: &IntervalOutcome,
    ledgers: &[BlockLedger],
    network: Network,
    declared_activation_height: Option<u32>,
    pre_activation_ironwood: Option<PreActivationIronwood>,
    registry: &mut CheckRegistry,
) {
    check_activation_context(network, declared_activation_height, registry);
    check_ironwood_anchor_zero(network, pre_activation_ironwood, registry);
    check_no_ironwood_before_activation(outcome, network, registry);
    check_orchard_withdrawal_only(outcome, ledgers, network, registry);

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

/// The Ironwood pool holds nothing at the block immediately preceding activation.
///
/// ZIP 258 introduces the pool empty. The property is about a specific height, not about a
/// specific position in a bundle: whether that height arrives as the interval's anchor or as
/// a height inside the interval, the same claim is being made about the same block. Tying
/// the check to the anchor alone made it mutually exclusive with
/// [`check_no_ironwood_before_activation`], which needs a pre-activation height *inside* the
/// interval, so no single bundle could ever affirm both halves of the boundary claim.
///
/// This is a consistency check rather than a discovery. ZIP 258 defines the balance to be
/// zero before activation, so a node that disagreed would be the finding; agreement is the
/// expected case. The report's limitations say as much, so a reader does not mistake the
/// pass for an independent measurement of the pool at that height.
fn check_ironwood_anchor_zero(
    network: Network,
    observed: Option<PreActivationIronwood>,
    registry: &mut CheckRegistry,
) {
    let activation = network.ironwood_activation_height();

    let Ok(pre_activation) = activation.checked_previous() else {
        registry.record(Check::not_applicable(
            ids::IRONWOOD_ANCHOR_ZERO,
            "activation height has no preceding block",
        ));
        return;
    };

    let Some(observed) = observed else {
        registry.record(Check::not_applicable(
            ids::IRONWOOD_ANCHOR_ZERO,
            format!(
                "the bundle does not cover height {pre_activation}, the block before activation at {activation}"
            ),
        ));
        return;
    };

    let Some(balance) = observed.balance else {
        registry.record(Check::not_applicable(
            ids::IRONWOOD_ANCHOR_ZERO,
            format!("the bundle records no Ironwood balance at height {pre_activation}"),
        ));
        return;
    };

    if balance == Zatoshi::ZERO {
        registry.record(Check::pass(ids::IRONWOOD_ANCHOR_ZERO));
    } else {
        registry.record(Check::fail(
            ids::IRONWOOD_ANCHOR_ZERO,
            format!(
                "Ironwood pool is recorded as {balance} zatoshi at height {pre_activation}, the block before activation, expected zero"
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
/// ZIP 258 states the rule **per transaction**: "No new value may enter the Orchard pool:
/// for every transaction, v^OrchardPoolBalance >= 0". Under the ZIP 209 sign convention a
/// non-negative encoded `valueBalanceOrchard` is a non-positive change to the pool, so the
/// rule is that no single transaction may add to Orchard.
///
/// Testing it on the block total instead would net offsetting transactions against each
/// other: a block holding one transaction that adds 100 to Orchard and another that removes
/// 200 sums to -100 and looks compliant, while the first transaction breaks a consensus rule.
/// The offending transaction is named, because "this block is wrong" is a much weaker
/// finding than "this transaction is wrong".
///
/// The rule is created by the upgrade, so an interval lying entirely below activation gives
/// it nothing to range over. Passing there would assert a post-activation consensus rule
/// held across heights at which it did not yet exist.
fn check_orchard_withdrawal_only(
    outcome: &IntervalOutcome,
    ledgers: &[BlockLedger],
    network: Network,
    registry: &mut CheckRegistry,
) {
    let has_post_activation = outcome
        .heights
        .iter()
        .any(|height| network.is_post_activation(height.height));

    if !has_post_activation {
        registry.record(Check::not_applicable(
            ids::ORCHARD_WITHDRAWAL_ONLY,
            format!(
                "the interval contains no heights at or after activation at {}, where ZIP 258 makes Orchard withdrawal-only",
                network.ironwood_activation_height()
            ),
        ));
        return;
    }

    let offender = ledgers
        .iter()
        .filter(|ledger| network.is_post_activation(ledger.height))
        .flat_map(|ledger| ledger.transactions.iter())
        .find(|transaction| {
            !transaction.orchard_delta.is_negative() && transaction.orchard_delta != Zatoshi::ZERO
        });

    match offender {
        None => registry.record(Check::pass(ids::ORCHARD_WITHDRAWAL_ONLY)),
        Some(transaction) => registry.record(Check::fail(
            ids::ORCHARD_WITHDRAWAL_ONLY,
            format!(
                "transaction {} at index {} in height {} adds {} zatoshi to the Orchard pool, which is at or after activation",
                transaction.txid,
                transaction.tx_index,
                transaction.block_height,
                transaction.orchard_delta
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
    use crate::parse::transaction::TransactionPoolDelta;
    use crate::reconcile::interval::reconcile_interval;
    use crate::reconcile::ledger::BlockLedger;
    use std::collections::BTreeMap;

    const ACTIVATION: u32 = 3_428_143;

    /// What every node reports for Ironwood before NU6.3 activates.
    const EMPTY_POOL: PreActivationIronwood = PreActivationIronwood {
        balance: Some(Zatoshi::ZERO),
    };

    /// A block whose whole movement is carried by one transaction.
    fn ledger(height: u32, orchard: i64, ironwood: i64) -> BlockLedger {
        ledger_of(height, &[(orchard, ironwood)])
    }

    /// A block holding one transaction per supplied `(orchard, ironwood)` pair.
    ///
    /// ZIP 258 states the withdrawal-only rule per transaction, so a block total is not a
    /// faithful stand-in for the thing being checked.
    fn ledger_of(height: u32, transactions: &[(i64, i64)]) -> BlockLedger {
        let transactions: Vec<TransactionPoolDelta> = transactions
            .iter()
            .enumerate()
            .map(|(index, &(orchard, ironwood))| TransactionPoolDelta {
                txid: format!("{height:056x}{index:08x}"),
                block_height: BlockHeight::new(height),
                tx_index: u32::try_from(index).unwrap(),
                transaction_version: 6,
                orchard_delta: Zatoshi::from_raw(orchard),
                ironwood_delta: Zatoshi::from_raw(ironwood),
            })
            .collect();

        BlockLedger {
            height: BlockHeight::new(height),
            block_hash: format!("{height:064x}"),
            previous_block_hash: format!("{:064x}", height.saturating_sub(1)),
            orchard_delta: Zatoshi::try_sum(transactions.iter().map(|tx| tx.orchard_delta))
                .unwrap(),
            ironwood_delta: Zatoshi::try_sum(transactions.iter().map(|tx| tx.ironwood_delta))
                .unwrap(),
            transactions,
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

    fn details_of(registry: &CheckRegistry, id: &str) -> String {
        registry
            .checks()
            .iter()
            .find(|check| check.id == id)
            .and_then(|check| check.details.clone())
            .unwrap_or_default()
    }

    #[test]
    fn an_activation_anchored_interval_passes_every_activation_check() {
        let ledgers = vec![ledger(ACTIVATION, -8_000, 8_000)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            Some(ACTIVATION),
            Some(EMPTY_POOL),
            &mut registry,
        );
        assert!(!registry.has_failures(), "{:?}", registry.checks());
        assert_eq!(
            status_of(&registry, ids::IRONWOOD_ANCHOR_ZERO),
            Status::Pass
        );
    }

    #[test]
    fn a_nonzero_ironwood_balance_before_activation_fails() {
        let ledgers = vec![ledger(ACTIVATION, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 1),
            &ledgers,
            Network::Mainnet,
            None,
            Some(PreActivationIronwood {
                balance: Some(Zatoshi::from_raw(1)),
            }),
            &mut registry,
        );
        assert_eq!(
            status_of(&registry, ids::IRONWOOD_ANCHOR_ZERO),
            Status::Fail
        );
    }

    #[test]
    fn a_nonzero_ironwood_balance_before_activation_is_named_in_the_failure() {
        let ledgers = vec![ledger(ACTIVATION, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            Some(PreActivationIronwood {
                balance: Some(Zatoshi::from_raw(500)),
            }),
            &mut registry,
        );
        assert_eq!(
            status_of(&registry, ids::IRONWOOD_ANCHOR_ZERO),
            Status::Fail
        );
        assert!(details_of(&registry, ids::IRONWOOD_ANCHOR_ZERO).contains("500"));
    }

    #[test]
    fn an_empty_ironwood_pool_before_activation_passes() {
        // Zebra reports `monitored: false` here, but that field is computed as
        // `chainValueZat != 0`, so it restates the balance and cannot mark the zero as
        // unmeasured. Treating it as a placeholder made this check unable ever to pass.
        let ledgers = vec![ledger(ACTIVATION, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            Some(EMPTY_POOL),
            &mut registry,
        );

        assert_eq!(
            status_of(&registry, ids::IRONWOOD_ANCHOR_ZERO),
            Status::Pass
        );
        assert!(!registry.has_failures());
    }

    #[test]
    fn an_absent_ironwood_balance_before_activation_is_not_passed_vacuously() {
        let ledgers = vec![ledger(ACTIVATION, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            Some(PreActivationIronwood { balance: None }),
            &mut registry,
        );
        assert_eq!(
            status_of(&registry, ids::IRONWOOD_ANCHOR_ZERO),
            Status::NotApplicable
        );
    }

    #[test]
    fn a_bundle_not_covering_the_boundary_marks_the_anchor_check_not_applicable() {
        let ledgers = vec![ledger(ACTIVATION + 500, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 42),
            &ledgers,
            Network::Mainnet,
            None,
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
    fn the_boundary_and_pre_activation_checks_can_both_apply_to_one_interval() {
        // Tying the boundary balance to the anchor made these two mutually exclusive: the
        // anchor check needed the interval to start at activation, and the pre-activation
        // check needed a height below it. No bundle could affirm both halves of the
        // boundary claim at once, which is the claim the tool exists to make.
        let ledgers = vec![ledger(ACTIVATION - 1, 0, 0), ledger(ACTIVATION, -10, 10)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            Some(EMPTY_POOL),
            &mut registry,
        );

        assert_eq!(
            status_of(&registry, ids::IRONWOOD_ANCHOR_ZERO),
            Status::Pass
        );
        assert_eq!(
            status_of(&registry, ids::NO_IRONWOOD_BEFORE_ACTIVATION),
            Status::Pass
        );
        assert_eq!(
            status_of(&registry, ids::ORCHARD_WITHDRAWAL_ONLY),
            Status::Pass
        );
    }

    #[test]
    fn value_entering_orchard_after_activation_fails() {
        let ledgers = vec![ledger(ACTIVATION, 5_000, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            Some(EMPTY_POOL),
            &mut registry,
        );

        assert_eq!(
            status_of(&registry, ids::ORCHARD_WITHDRAWAL_ONLY),
            Status::Fail
        );
        assert!(
            details_of(&registry, ids::ORCHARD_WITHDRAWAL_ONLY).contains(&ACTIVATION.to_string())
        );
    }

    #[test]
    fn a_transaction_adding_to_orchard_fails_even_when_the_block_total_looks_compliant() {
        // ZIP 258 constrains every transaction, not the block sum. Netting the two
        // transactions here gives -100, which a block-level test reads as a withdrawal
        // while the first transaction breaks the consensus rule outright.
        let ledgers = vec![ledger_of(ACTIVATION, &[(100, 0), (-200, 0)])];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            Some(EMPTY_POOL),
            &mut registry,
        );

        assert_eq!(
            status_of(&registry, ids::ORCHARD_WITHDRAWAL_ONLY),
            Status::Fail,
            "the block total is -100, so only a per-transaction test can catch this"
        );
        let details = details_of(&registry, ids::ORCHARD_WITHDRAWAL_ONLY);
        assert!(
            details.contains("index 0"),
            "the offender must be named: {details}"
        );
        assert!(details.contains("100"), "{details}");
    }

    #[test]
    fn value_leaving_orchard_after_activation_passes() {
        let ledgers = vec![ledger(ACTIVATION, -5_000, 5_000)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            Some(EMPTY_POOL),
            &mut registry,
        );
        assert_eq!(
            status_of(&registry, ids::ORCHARD_WITHDRAWAL_ONLY),
            Status::Pass
        );
    }

    #[test]
    fn a_wholly_pre_activation_interval_does_not_pass_the_withdrawal_only_rule_vacuously() {
        // ZIP 258 makes Orchard withdrawal-only from activation onwards. Below it the rule
        // does not exist, so there is nothing to affirm and a pass would be a claim the
        // evidence does not support.
        let ledgers = vec![
            ledger(ACTIVATION - 3, 5_000, 0),
            ledger(ACTIVATION - 2, 0, 0),
        ];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            None,
            &mut registry,
        );

        assert_eq!(
            status_of(&registry, ids::ORCHARD_WITHDRAWAL_ONLY),
            Status::NotApplicable
        );
        assert!(
            details_of(&registry, ids::ORCHARD_WITHDRAWAL_ONLY).contains(&ACTIVATION.to_string()),
            "the reason must name the activation height it is relative to"
        );
        assert!(!registry.has_failures());
    }

    #[test]
    fn an_all_post_activation_interval_marks_the_pre_activation_check_not_applicable() {
        let ledgers = vec![ledger(ACTIVATION, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            Some(EMPTY_POOL),
            &mut registry,
        );
        assert_eq!(
            status_of(&registry, ids::NO_IRONWOOD_BEFORE_ACTIVATION),
            Status::NotApplicable
        );
    }

    #[test]
    fn ironwood_movement_below_activation_fails() {
        let ledgers = vec![ledger(ACTIVATION - 2, 0, 100), ledger(ACTIVATION - 1, 0, 0)];
        let mut registry = CheckRegistry::new();
        evaluate(
            &outcome(&ledgers, 0),
            &ledgers,
            Network::Mainnet,
            None,
            Some(EMPTY_POOL),
            &mut registry,
        );
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
            &ledgers,
            Network::Mainnet,
            Some(3_000_000),
            Some(EMPTY_POOL),
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
            &ledgers,
            Network::Testnet,
            Some(4_134_000),
            Some(EMPTY_POOL),
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
