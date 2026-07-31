//! Interval accumulation and per-height comparison.
//!
//! The anchor balances are a declared starting point, not a derived one: this crate
//! reconstructs *changes* over a bounded interval and does not compute supply from genesis.
//!
//! Comparison happens at every height rather than only at the interval endpoints, because
//! nodes report both a running balance and a per-block delta. Two axes at every height turn
//! "the totals disagree" into "the totals diverge at this block, in this transaction".

use std::collections::BTreeMap;

use crate::domain::height::{BlockHeight, HeightInterval};
use crate::domain::pool::Pool;
use crate::domain::pool_state::ReportedPoolState;
use crate::domain::zatoshi::Zatoshi;
use crate::error::ReconcileError;
use crate::reconcile::ledger::BlockLedger;

/// Declared pool balances at the anchor block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchorBalances {
    pub orchard: Zatoshi,
    pub ironwood: Zatoshi,
}

/// Whether a comparison could be made, and if so whether it agreed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agreement {
    Agrees,
    Differs,
    /// The node reported no value to compare against.
    NotReported,
}

impl Agreement {
    fn of(reconstructed: Zatoshi, reported: Option<Zatoshi>) -> Self {
        match reported {
            Some(value) if value == reconstructed => Self::Agrees,
            Some(_) => Self::Differs,
            None => Self::NotReported,
        }
    }

    pub const fn differs(self) -> bool {
        matches!(self, Self::Differs)
    }
}

/// Reconstruction and comparison at a single height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeightOutcome {
    pub height: BlockHeight,
    pub reconstructed_orchard_delta: Zatoshi,
    pub reconstructed_ironwood_delta: Zatoshi,
    pub expected_orchard_balance: Zatoshi,
    pub expected_ironwood_balance: Zatoshi,
    pub reported_orchard_balance: Option<Zatoshi>,
    pub reported_ironwood_balance: Option<Zatoshi>,
    pub reported_orchard_delta: Option<Zatoshi>,
    pub reported_ironwood_delta: Option<Zatoshi>,
}

impl HeightOutcome {
    pub fn balance_agreement(&self, pool: Pool) -> Agreement {
        match pool {
            Pool::Orchard => {
                Agreement::of(self.expected_orchard_balance, self.reported_orchard_balance)
            }
            Pool::Ironwood => Agreement::of(
                self.expected_ironwood_balance,
                self.reported_ironwood_balance,
            ),
            _ => Agreement::NotReported,
        }
    }

    pub fn delta_agreement(&self, pool: Pool) -> Agreement {
        match pool {
            Pool::Orchard => Agreement::of(
                self.reconstructed_orchard_delta,
                self.reported_orchard_delta,
            ),
            Pool::Ironwood => Agreement::of(
                self.reconstructed_ironwood_delta,
                self.reported_ironwood_delta,
            ),
            _ => Agreement::NotReported,
        }
    }

    pub fn expected_balance(&self, pool: Pool) -> Option<Zatoshi> {
        match pool {
            Pool::Orchard => Some(self.expected_orchard_balance),
            Pool::Ironwood => Some(self.expected_ironwood_balance),
            _ => None,
        }
    }

    pub fn reconstructed_delta(&self, pool: Pool) -> Option<Zatoshi> {
        match pool {
            Pool::Orchard => Some(self.reconstructed_orchard_delta),
            Pool::Ironwood => Some(self.reconstructed_ironwood_delta),
            _ => None,
        }
    }
}

/// Cumulative flow across the turnstile boundary.
///
/// These are **observations**, not assertions. An intuitive invariant — that Ironwood
/// cannot receive more than Orchard released — would hold only if the Orchard turnstile
/// were the sole route into Ironwood. Whether value may also enter Ironwood directly from
/// the transparent pool is not established, and asserting an unsourced inequality could
/// emit a false failure against a perfectly valid chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TurnstileObservation {
    /// Total value that left the Orchard pool, as a positive magnitude.
    pub orchard_outflow: Zatoshi,
    /// Total value that entered the Ironwood pool, as a positive magnitude.
    pub ironwood_inflow: Zatoshi,
}

/// The complete result of reconciling an interval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntervalOutcome {
    pub interval: HeightInterval,
    pub anchor: AnchorBalances,
    pub heights: Vec<HeightOutcome>,
    pub cumulative_orchard_delta: Zatoshi,
    pub cumulative_ironwood_delta: Zatoshi,
    pub expected_end_orchard: Zatoshi,
    pub expected_end_ironwood: Zatoshi,
    pub turnstile: TurnstileObservation,
}

impl IntervalOutcome {
    pub fn expected_end_balance(&self, pool: Pool) -> Option<Zatoshi> {
        match pool {
            Pool::Orchard => Some(self.expected_end_orchard),
            Pool::Ironwood => Some(self.expected_end_ironwood),
            _ => None,
        }
    }

    /// Heights where either comparison axis disagreed, in ascending order.
    ///
    /// The first entry is where a divergence originates, which is the diagnostic an
    /// endpoint-only comparison cannot provide.
    pub fn divergent_heights(&self) -> Vec<BlockHeight> {
        self.heights
            .iter()
            .filter(|outcome| {
                Pool::RECONSTRUCTED.iter().any(|&pool| {
                    outcome.balance_agreement(pool).differs()
                        || outcome.delta_agreement(pool).differs()
                })
            })
            .map(|outcome| outcome.height)
            .collect()
    }
}

/// Accumulates block ledgers into an interval outcome.
///
/// Running totals are held as `i128` so that a transient excursion beyond the 64-bit range
/// is representable, and are narrowed back with a checked conversion at each height. This
/// makes an overflow a reported error rather than a wrapped value.
pub fn reconcile_interval(
    ledgers: &[BlockLedger],
    interval: HeightInterval,
    anchor: AnchorBalances,
    reported: &BTreeMap<BlockHeight, ReportedPoolState>,
) -> Result<IntervalOutcome, ReconcileError> {
    let mut orchard_running = i128::from(anchor.orchard.get());
    let mut ironwood_running = i128::from(anchor.ironwood.get());
    let mut orchard_cumulative = 0_i128;
    let mut ironwood_cumulative = 0_i128;
    let mut orchard_outflow = 0_i128;
    let mut ironwood_inflow = 0_i128;

    let mut heights = Vec::with_capacity(ledgers.len());

    for ledger in ledgers {
        let orchard_delta = i128::from(ledger.orchard_delta.get());
        let ironwood_delta = i128::from(ledger.ironwood_delta.get());

        orchard_running = add(orchard_running, orchard_delta)?;
        ironwood_running = add(ironwood_running, ironwood_delta)?;
        orchard_cumulative = add(orchard_cumulative, orchard_delta)?;
        ironwood_cumulative = add(ironwood_cumulative, ironwood_delta)?;

        if orchard_delta < 0 {
            let magnitude = orchard_delta
                .checked_neg()
                .ok_or(ReconcileError::ArithmeticOverflow)?;
            orchard_outflow = add(orchard_outflow, magnitude)?;
        }
        if ironwood_delta > 0 {
            ironwood_inflow = add(ironwood_inflow, ironwood_delta)?;
        }

        let state = reported.get(&ledger.height);
        heights.push(HeightOutcome {
            height: ledger.height,
            reconstructed_orchard_delta: ledger.orchard_delta,
            reconstructed_ironwood_delta: ledger.ironwood_delta,
            expected_orchard_balance: narrow(orchard_running)?,
            expected_ironwood_balance: narrow(ironwood_running)?,
            reported_orchard_balance: state.and_then(|s| s.balance(Pool::Orchard)),
            reported_ironwood_balance: state.and_then(|s| s.balance(Pool::Ironwood)),
            reported_orchard_delta: state.and_then(|s| s.delta(Pool::Orchard)),
            reported_ironwood_delta: state.and_then(|s| s.delta(Pool::Ironwood)),
        });
    }

    Ok(IntervalOutcome {
        interval,
        anchor,
        heights,
        cumulative_orchard_delta: narrow(orchard_cumulative)?,
        cumulative_ironwood_delta: narrow(ironwood_cumulative)?,
        expected_end_orchard: narrow(orchard_running)?,
        expected_end_ironwood: narrow(ironwood_running)?,
        turnstile: TurnstileObservation {
            orchard_outflow: narrow(orchard_outflow)?,
            ironwood_inflow: narrow(ironwood_inflow)?,
        },
    })
}

fn add(accumulator: i128, delta: i128) -> Result<i128, ReconcileError> {
    accumulator
        .checked_add(delta)
        .ok_or(ReconcileError::ArithmeticOverflow)
}

/// Narrows a running total back to a monetary value, rejecting anything unrepresentable.
fn narrow(accumulator: i128) -> Result<Zatoshi, ReconcileError> {
    let value = i64::try_from(accumulator).map_err(|_| ReconcileError::ArithmeticOverflow)?;
    Zatoshi::new_checked(value)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn interval(start: u32, end: u32) -> HeightInterval {
        HeightInterval::new(BlockHeight::new(start), BlockHeight::new(end)).unwrap()
    }

    fn anchor(orchard: i64, ironwood: i64) -> AnchorBalances {
        AnchorBalances {
            orchard: Zatoshi::from_raw(orchard),
            ironwood: Zatoshi::from_raw(ironwood),
        }
    }

    fn no_reports() -> BTreeMap<BlockHeight, ReportedPoolState> {
        BTreeMap::new()
    }

    #[test]
    fn an_interval_with_no_activity_preserves_the_anchor() {
        let ledgers = vec![ledger(101, 0, 0), ledger(102, 0, 0)];
        let outcome = reconcile_interval(
            &ledgers,
            interval(101, 102),
            anchor(1_000, 0),
            &no_reports(),
        )
        .unwrap();

        assert_eq!(outcome.expected_end_orchard, Zatoshi::from_raw(1_000));
        assert_eq!(outcome.expected_end_ironwood, Zatoshi::ZERO);
        assert_eq!(outcome.cumulative_orchard_delta, Zatoshi::ZERO);
    }

    #[test]
    fn deltas_accumulate_across_blocks() {
        let ledgers = vec![ledger(101, -300, 300), ledger(102, -200, 200)];
        let outcome = reconcile_interval(
            &ledgers,
            interval(101, 102),
            anchor(1_000, 0),
            &no_reports(),
        )
        .unwrap();

        assert_eq!(outcome.cumulative_orchard_delta, Zatoshi::from_raw(-500));
        assert_eq!(outcome.cumulative_ironwood_delta, Zatoshi::from_raw(500));
        assert_eq!(outcome.expected_end_orchard, Zatoshi::from_raw(500));
        assert_eq!(outcome.expected_end_ironwood, Zatoshi::from_raw(500));
    }

    #[test]
    fn running_balances_are_recorded_at_every_height() {
        let ledgers = vec![ledger(101, -300, 300), ledger(102, -200, 200)];
        let outcome = reconcile_interval(
            &ledgers,
            interval(101, 102),
            anchor(1_000, 0),
            &no_reports(),
        )
        .unwrap();

        assert_eq!(outcome.heights.len(), 2);
        assert_eq!(
            outcome.heights[0].expected_orchard_balance,
            Zatoshi::from_raw(700)
        );
        assert_eq!(
            outcome.heights[0].expected_ironwood_balance,
            Zatoshi::from_raw(300)
        );
        assert_eq!(
            outcome.heights[1].expected_orchard_balance,
            Zatoshi::from_raw(500)
        );
    }

    #[test]
    fn a_matching_report_agrees_on_both_axes() {
        let ledgers = vec![ledger(101, -300, 300)];
        let mut reported = BTreeMap::new();
        reported.insert(
            BlockHeight::new(101),
            ReportedPoolState::new(BlockHeight::new(101))
                .with_balance(Pool::Orchard, Zatoshi::from_raw(700))
                .with_balance(Pool::Ironwood, Zatoshi::from_raw(300))
                .with_delta(Pool::Orchard, Zatoshi::from_raw(-300))
                .with_delta(Pool::Ironwood, Zatoshi::from_raw(300)),
        );

        let outcome =
            reconcile_interval(&ledgers, interval(101, 101), anchor(1_000, 0), &reported).unwrap();

        let height = &outcome.heights[0];
        assert_eq!(height.balance_agreement(Pool::Orchard), Agreement::Agrees);
        assert_eq!(height.delta_agreement(Pool::Ironwood), Agreement::Agrees);
        assert!(outcome.divergent_heights().is_empty());
    }

    #[test]
    fn a_divergence_is_localised_to_the_block_where_it_originates() {
        let ledgers = vec![ledger(101, -300, 300), ledger(102, -200, 200)];
        let mut reported = BTreeMap::new();
        // Height 101 agrees; height 102 reports a delta the reconstruction does not produce.
        reported.insert(
            BlockHeight::new(101),
            ReportedPoolState::new(BlockHeight::new(101))
                .with_delta(Pool::Orchard, Zatoshi::from_raw(-300)),
        );
        reported.insert(
            BlockHeight::new(102),
            ReportedPoolState::new(BlockHeight::new(102))
                .with_delta(Pool::Orchard, Zatoshi::from_raw(-999)),
        );

        let outcome =
            reconcile_interval(&ledgers, interval(101, 102), anchor(1_000, 0), &reported).unwrap();

        assert_eq!(
            outcome.divergent_heights(),
            vec![BlockHeight::new(102)],
            "divergence should be attributed to the block that caused it"
        );
    }

    #[test]
    fn an_unreported_pool_is_not_counted_as_disagreement() {
        let ledgers = vec![ledger(101, -300, 300)];
        let mut reported = BTreeMap::new();
        reported.insert(
            BlockHeight::new(101),
            ReportedPoolState::new(BlockHeight::new(101)),
        );

        let outcome =
            reconcile_interval(&ledgers, interval(101, 101), anchor(1_000, 0), &reported).unwrap();

        assert_eq!(
            outcome.heights[0].balance_agreement(Pool::Orchard),
            Agreement::NotReported
        );
        assert!(outcome.divergent_heights().is_empty());
    }

    #[test]
    fn turnstile_flows_are_observed_as_positive_magnitudes() {
        let ledgers = vec![ledger(101, -300, 300), ledger(102, -200, 150)];
        let outcome = reconcile_interval(
            &ledgers,
            interval(101, 102),
            anchor(1_000, 0),
            &no_reports(),
        )
        .unwrap();

        assert_eq!(outcome.turnstile.orchard_outflow, Zatoshi::from_raw(500));
        assert_eq!(outcome.turnstile.ironwood_inflow, Zatoshi::from_raw(450));
    }

    #[test]
    fn turnstile_observation_ignores_movement_in_the_opposite_direction() {
        // Inflow to Orchard would be a consensus violation post-activation, but the
        // observation must still report only genuine outflow rather than netting.
        let ledgers = vec![ledger(101, -300, 0), ledger(102, 100, 0)];
        let outcome = reconcile_interval(
            &ledgers,
            interval(101, 102),
            anchor(1_000, 0),
            &no_reports(),
        )
        .unwrap();

        assert_eq!(outcome.turnstile.orchard_outflow, Zatoshi::from_raw(300));
    }

    #[test]
    fn accumulation_overflow_is_an_error_not_a_wrap() {
        let ledgers = vec![ledger(101, i64::MAX, 0), ledger(102, i64::MAX, 0)];
        assert!(matches!(
            reconcile_interval(
                &ledgers,
                interval(101, 102),
                anchor(i64::MAX, 0),
                &no_reports()
            ),
            Err(ReconcileError::ArithmeticOverflow)
        ));
    }

    #[test]
    fn a_reconstructed_balance_may_go_negative_for_a_dedicated_check_to_catch() {
        // Accumulation must not itself reject a negative balance; detecting one is the
        // job of the check registry, which can report it with context.
        let ledgers = vec![ledger(101, -5_000, 0)];
        let outcome = reconcile_interval(
            &ledgers,
            interval(101, 101),
            anchor(1_000, 0),
            &no_reports(),
        )
        .unwrap();
        assert!(outcome.expected_end_orchard.is_negative());
    }

    #[test]
    fn the_end_balance_equals_the_anchor_plus_the_cumulative_delta() {
        let ledgers = vec![ledger(101, -300, 300), ledger(102, -200, 200)];
        let anchor = anchor(1_000, 50);
        let outcome =
            reconcile_interval(&ledgers, interval(101, 102), anchor, &no_reports()).unwrap();

        assert_eq!(
            outcome.expected_end_orchard,
            anchor
                .orchard
                .checked_add(outcome.cumulative_orchard_delta)
                .unwrap()
        );
        assert_eq!(
            outcome.expected_end_ironwood,
            anchor
                .ironwood
                .checked_add(outcome.cumulative_ironwood_delta)
                .unwrap()
        );
    }
}
