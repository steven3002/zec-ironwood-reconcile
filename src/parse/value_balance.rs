//! Conversion of shielded bundle value balances into chain value pool deltas.
//!
//! This is the highest-consequence file in the crate. The sign convention appears here
//! exactly once; every downstream consumer receives an already-signed delta and must never
//! re-derive direction. A second negation anywhere else is a defect.
//!
//! # The convention
//!
//! ZIP 209 defines a shielded pool's chain value balance as the **negation** of the sum of
//! the `valueBalance` fields of that pool's bundles across the chain. ZIP 258 extends the
//! same treatment to the Ironwood pool for NU6.3. Therefore, for a pool `P` and a
//! transaction `t`:
//!
//! ```text
//! delta_P(t) = -valueBalance_P(t)
//! ```
//!
//! A negative `valueBalance` denotes value **entering** the pool; a positive one denotes
//! value **leaving** it.
//!
//! # Corroboration
//!
//! The convention is derived from specification, not inferred from observed transactions.
//! It is independently corroborated by the reference implementation: `orchard` documents
//! `Bundle::value_balance` as the net value moved into or out of the pool, computed as the
//! sum of spends minus outputs. A transaction creating more output value than it spends is
//! moving value into the pool and yields a negative `valueBalance`, which under the rule
//! above produces a positive pool delta. Specification and implementation agree.

use zcash_primitives::transaction::Transaction;

use crate::domain::zatoshi::Zatoshi;
use crate::error::ReconcileError;

/// Converts a bundle's `valueBalance` field into a chain value pool delta.
///
/// The value is bounds-checked before negation so that a field outside the representable
/// money range is reported as malformed data rather than propagated into accounting.
pub fn pool_delta(value_balance: i64) -> Result<Zatoshi, ReconcileError> {
    Zatoshi::new_checked(value_balance)?.checked_neg()
}

/// Orchard pool delta contributed by a transaction.
///
/// A transaction with no Orchard bundle contributes exactly zero. This is a correct
/// accounting statement rather than a skip: such a transaction moves no value into or out
/// of the Orchard pool.
pub fn orchard_delta(transaction: &Transaction) -> Result<Zatoshi, ReconcileError> {
    match transaction.orchard_bundle() {
        Some(bundle) => pool_delta((*bundle.value_balance()).into()),
        None => Ok(Zatoshi::ZERO),
    }
}

/// Ironwood pool delta contributed by a transaction.
///
/// Only version 6 transactions can carry an Ironwood bundle; every earlier version
/// contributes zero.
pub fn ironwood_delta(transaction: &Transaction) -> Result<Zatoshi, ReconcileError> {
    match transaction.ironwood_bundle() {
        Some(bundle) => pool_delta((*bundle.value_balance()).into()),
        None => Ok(Zatoshi::ZERO),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::zatoshi::MAX_MONEY;

    #[test]
    fn negative_value_balance_increases_the_pool() {
        // Value entering a pool is encoded as a negative valueBalance.
        let delta = pool_delta(-17_600_000_000_000).unwrap();
        assert_eq!(delta.get(), 17_600_000_000_000);
        assert!(!delta.is_negative());
    }

    #[test]
    fn positive_value_balance_decreases_the_pool() {
        // Value leaving a pool is encoded as a positive valueBalance.
        let delta = pool_delta(17_600_000_000_000).unwrap();
        assert_eq!(delta.get(), -17_600_000_000_000);
        assert!(delta.is_negative());
    }

    #[test]
    fn zero_value_balance_leaves_the_pool_unchanged() {
        assert_eq!(pool_delta(0).unwrap(), Zatoshi::ZERO);
    }

    #[test]
    fn the_convention_is_an_involution() {
        // Applying the convention twice must return the original magnitude, which catches
        // an accidental second negation introduced downstream.
        for value in [-1_i64, 1, -500_000, 500_000, MAX_MONEY, -MAX_MONEY] {
            let once = pool_delta(value).unwrap();
            let twice = pool_delta(once.get()).unwrap();
            assert_eq!(twice.get(), value);
        }
    }

    #[test]
    fn a_migration_pair_nets_to_the_amount_moved() {
        // A transaction that spends from Orchard and creates into Ironwood encodes a
        // positive Orchard valueBalance (value leaving) and a negative Ironwood one (value
        // entering), so the deltas have opposite signs and equal magnitude. This exercises
        // the sign convention on a constructed pair; it is not a claim that migration is
        // how Ironwood is funded, which real blocks disprove, see `PoolFlows`.
        let moved = 8_100_000_000_000_i64;
        let orchard = pool_delta(moved).unwrap();
        let ironwood = pool_delta(-moved).unwrap();

        assert!(orchard.is_negative(), "Orchard must shrink");
        assert!(!ironwood.is_negative(), "Ironwood must grow");
        assert_eq!(orchard.checked_add(ironwood).unwrap(), Zatoshi::ZERO);
    }

    #[test]
    fn a_value_balance_beyond_the_money_bound_is_rejected() {
        assert!(matches!(
            pool_delta(MAX_MONEY + 1),
            Err(ReconcileError::ValueOutOfBounds { .. })
        ));
        assert!(matches!(
            pool_delta(-MAX_MONEY - 1),
            Err(ReconcileError::ValueOutOfBounds { .. })
        ));
    }

    #[test]
    fn the_money_bounds_themselves_are_accepted() {
        assert_eq!(pool_delta(MAX_MONEY).unwrap().get(), -MAX_MONEY);
        assert_eq!(pool_delta(-MAX_MONEY).unwrap().get(), MAX_MONEY);
    }

    #[test]
    fn extreme_integer_inputs_are_rejected_before_negation() {
        // i64::MIN has no positive counterpart; it must be rejected by the bounds check
        // rather than reaching the negation and overflowing.
        assert!(matches!(
            pool_delta(i64::MIN),
            Err(ReconcileError::ValueOutOfBounds { .. })
        ));
        assert!(matches!(
            pool_delta(i64::MAX),
            Err(ReconcileError::ValueOutOfBounds { .. })
        ));
    }
}
