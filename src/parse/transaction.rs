//! Per-transaction pool delta extraction.
//!
//! Transactions are deserialized by `zcash_primitives`; the accounting interpretation of
//! what they contain is implemented here. An interval contains a mixture of transaction
//! versions, because version 6 is not mandatory after NU6.3 activation.

use zcash_primitives::transaction::{Transaction, TxVersion};
use zcash_protocol::consensus::BranchId;

use crate::domain::height::BlockHeight;
use crate::domain::zatoshi::Zatoshi;
use crate::error::ReconcileError;
use crate::parse::value_balance;

/// The pool contribution of a single transaction.
///
/// Both deltas are already sign-corrected. Every transaction in an interval produces a
/// record, including those contributing zero to both pools, so the ledger accounts for the
/// whole block rather than only its shielded subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionPoolDelta {
    pub txid: String,
    pub block_height: BlockHeight,
    pub tx_index: u32,
    pub transaction_version: u32,
    pub orchard_delta: Zatoshi,
    pub ironwood_delta: Zatoshi,
}

impl TransactionPoolDelta {
    /// Whether this transaction moved no value in either reconstructed pool.
    pub fn is_inert(&self) -> bool {
        self.orchard_delta == Zatoshi::ZERO && self.ironwood_delta == Zatoshi::ZERO
    }
}

/// Numeric form of a transaction version, for reporting.
///
/// Versions are reported rather than filtered: an unrecognized version is rejected at the
/// point of deserialization, so anything reaching here is a version this build understands.
pub fn version_number(version: TxVersion) -> u32 {
    match version {
        TxVersion::Sprout(number) => number,
        TxVersion::V3 => 3,
        TxVersion::V4 => 4,
        TxVersion::V5 => 5,
        TxVersion::V6 => 6,
    }
}

/// Whether a version is capable of carrying an Ironwood bundle.
///
/// Used to distinguish "this transaction had no Ironwood bundle" from "this transaction
/// could not have had one", which matters when explaining a zero delta.
pub const fn can_carry_ironwood(version: TxVersion) -> bool {
    matches!(version, TxVersion::V6)
}

/// Extracts the pool contribution of one transaction.
///
/// The transaction's own consensus branch identifier is checked against the one expected
/// for the interval. Version 5 and 6 transactions carry the branch identifier in their own
/// bytes, so this detects a transaction belonging to a different network or upgrade rather
/// than silently attributing its value to the wrong chain.
pub fn extract_delta(
    transaction: &Transaction,
    block_height: BlockHeight,
    tx_index: u32,
    expected_branch_id: BranchId,
) -> Result<TransactionPoolDelta, ReconcileError> {
    let actual_branch_id = transaction.consensus_branch_id();
    if actual_branch_id != expected_branch_id {
        return Err(ReconcileError::BranchIdMismatch {
            expected: u32::from(expected_branch_id),
            actual: u32::from(actual_branch_id),
        });
    }

    let describe = |error: ReconcileError| ReconcileError::TransactionParse {
        height: block_height.get(),
        tx_index,
        reason: error.to_string(),
    };

    let orchard_delta = value_balance::orchard_delta(transaction).map_err(describe)?;
    let ironwood_delta = value_balance::ironwood_delta(transaction).map_err(describe)?;

    Ok(TransactionPoolDelta {
        txid: transaction.txid().to_string(),
        block_height,
        tx_index,
        transaction_version: version_number(transaction.version()),
        orchard_delta,
        ironwood_delta,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_numbers_match_the_wire_versions() {
        assert_eq!(version_number(TxVersion::V4), 4);
        assert_eq!(version_number(TxVersion::V5), 5);
        assert_eq!(version_number(TxVersion::V6), 6);
    }

    #[test]
    fn only_version_six_can_carry_an_ironwood_bundle() {
        assert!(can_carry_ironwood(TxVersion::V6));
        assert!(!can_carry_ironwood(TxVersion::V5));
        assert!(!can_carry_ironwood(TxVersion::V4));
        assert!(!can_carry_ironwood(TxVersion::V3));
    }

    #[test]
    fn a_delta_with_no_movement_is_inert() {
        let delta = TransactionPoolDelta {
            txid: "0".repeat(64),
            block_height: BlockHeight::new(3_428_143),
            tx_index: 0,
            transaction_version: 4,
            orchard_delta: Zatoshi::ZERO,
            ironwood_delta: Zatoshi::ZERO,
        };
        assert!(delta.is_inert());
    }

    #[test]
    fn a_delta_with_movement_in_either_pool_is_not_inert() {
        let base = TransactionPoolDelta {
            txid: "0".repeat(64),
            block_height: BlockHeight::new(3_428_143),
            tx_index: 1,
            transaction_version: 6,
            orchard_delta: Zatoshi::ZERO,
            ironwood_delta: Zatoshi::ZERO,
        };

        let orchard_only = TransactionPoolDelta {
            orchard_delta: Zatoshi::from_raw(-1),
            ..base.clone()
        };
        let ironwood_only = TransactionPoolDelta {
            ironwood_delta: Zatoshi::from_raw(1),
            ..base
        };

        assert!(!orchard_only.is_inert());
        assert!(!ironwood_only.is_inert());
    }
}
