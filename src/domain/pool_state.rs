//! Pool balances and deltas as reported by a node.
//!
//! These are the figures the reconstruction is compared against. They are never inputs to
//! the calculation.
//!
//! # Absence is not zero
//!
//! A node can serve a response in which a pool's balance is absent or non-numeric. Zebra
//! documents doing exactly this for arbitrary heights while a database upgrade is in
//! progress, all the while appearing healthy. Treating an absent balance as zero would
//! produce a confident and completely wrong reconciliation, so absence is modelled
//! explicitly and [`ReportedPoolState::require_balance`] refuses to guess.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::domain::height::BlockHeight;
use crate::domain::pool::Pool;
use crate::domain::zatoshi::Zatoshi;
use crate::error::ReconcileError;

/// What a node reported about the value pools after a given block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedPoolState {
    pub height: BlockHeight,
    /// Balance after this block, per pool. A pool absent from the map was not reported.
    pub balances: BTreeMap<Pool, Zatoshi>,
    /// Change contributed by this block, per pool, where the node reported one.
    pub deltas: BTreeMap<Pool, Zatoshi>,
}

impl ReportedPoolState {
    pub fn new(height: BlockHeight) -> Self {
        Self {
            height,
            balances: BTreeMap::new(),
            deltas: BTreeMap::new(),
        }
    }

    pub fn with_balance(mut self, pool: Pool, balance: Zatoshi) -> Self {
        self.balances.insert(pool, balance);
        self
    }

    pub fn with_delta(mut self, pool: Pool, delta: Zatoshi) -> Self {
        self.deltas.insert(pool, delta);
        self
    }

    pub fn balance(&self, pool: Pool) -> Option<Zatoshi> {
        self.balances.get(&pool).copied()
    }

    pub fn delta(&self, pool: Pool) -> Option<Zatoshi> {
        self.deltas.get(&pool).copied()
    }

    /// Returns a balance, or fails if the node did not report one.
    ///
    /// The failure is a capture problem rather than an accounting one: the evidence cannot
    /// support a comparison at this height.
    pub fn require_balance(&self, pool: Pool) -> Result<Zatoshi, ReconcileError> {
        self.balance(pool)
            .ok_or_else(|| ReconcileError::CaptureIncomplete {
                reason: format!(
                    "node reported no {pool} balance at height {}; the capture cannot support a comparison here",
                    self.height
                ),
            })
    }

    /// Whether every pool this crate reconstructs was reported at this height.
    pub fn has_all_reconstructed_balances(&self) -> bool {
        Pool::RECONSTRUCTED
            .iter()
            .all(|pool| self.balances.contains_key(pool))
    }

    /// Pools that this crate reconstructs but the node did not report a balance for.
    pub fn missing_reconstructed_balances(&self) -> Vec<Pool> {
        Pool::RECONSTRUCTED
            .into_iter()
            .filter(|pool| !self.balances.contains_key(pool))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ReportedPoolState {
        ReportedPoolState::new(BlockHeight::new(3_428_143))
            .with_balance(Pool::Orchard, Zatoshi::from_raw(366_000_000_000_000))
            .with_balance(Pool::Ironwood, Zatoshi::ZERO)
            .with_delta(Pool::Orchard, Zatoshi::from_raw(-1_000))
            .with_delta(Pool::Ironwood, Zatoshi::from_raw(1_000))
    }

    #[test]
    fn reported_balances_are_returned() {
        let state = state();
        assert_eq!(state.balance(Pool::Ironwood), Some(Zatoshi::ZERO));
        assert_eq!(
            state.balance(Pool::Orchard),
            Some(Zatoshi::from_raw(366_000_000_000_000))
        );
    }

    #[test]
    fn an_unreported_pool_is_absent_not_zero() {
        let state = state();
        assert_eq!(state.balance(Pool::Sapling), None);
    }

    #[test]
    fn requiring_an_absent_balance_reports_an_incomplete_capture() {
        let state = state();
        let result = state.require_balance(Pool::Sapling);
        assert!(matches!(
            result,
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }

    #[test]
    fn a_zero_balance_is_distinguishable_from_an_absent_one() {
        // Ironwood legitimately holds zero at activation. That must not be conflated with
        // a node that failed to report the pool at all.
        let state = state();
        assert_eq!(
            state.require_balance(Pool::Ironwood).unwrap(),
            Zatoshi::ZERO
        );
        assert!(state.require_balance(Pool::Sprout).is_err());
    }

    #[test]
    fn the_failure_message_names_the_pool_and_height() {
        let state = state();
        let message = state.require_balance(Pool::Sprout).unwrap_err().to_string();
        assert!(message.contains("sprout"), "{message}");
        assert!(message.contains("3428143"), "{message}");
    }

    #[test]
    fn completeness_covers_only_the_reconstructed_pools() {
        assert!(state().has_all_reconstructed_balances());

        let partial =
            ReportedPoolState::new(BlockHeight::new(1)).with_balance(Pool::Orchard, Zatoshi::ZERO);
        assert!(!partial.has_all_reconstructed_balances());
        assert_eq!(
            partial.missing_reconstructed_balances(),
            vec![Pool::Ironwood]
        );
    }

    #[test]
    fn a_state_with_nothing_reported_lists_every_reconstructed_pool_as_missing() {
        let empty = ReportedPoolState::new(BlockHeight::new(1));
        assert_eq!(
            empty.missing_reconstructed_balances(),
            vec![Pool::Orchard, Pool::Ironwood]
        );
    }

    #[test]
    fn deltas_are_reported_independently_of_balances() {
        let state = state();
        assert_eq!(state.delta(Pool::Orchard), Some(Zatoshi::from_raw(-1_000)));
        assert_eq!(state.delta(Pool::Sapling), None);
    }
}
