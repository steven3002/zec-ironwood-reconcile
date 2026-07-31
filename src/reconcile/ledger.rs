//! Per-block aggregation of transaction deltas.

use crate::domain::height::BlockHeight;
use crate::domain::zatoshi::Zatoshi;
use crate::error::ReconcileError;
use crate::parse::block::ParsedBlock;
use crate::parse::transaction::TransactionPoolDelta;

/// The reconstructed pool movement of a single block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLedger {
    pub height: BlockHeight,
    pub block_hash: String,
    pub previous_block_hash: String,
    pub orchard_delta: Zatoshi,
    pub ironwood_delta: Zatoshi,
    pub transactions: Vec<TransactionPoolDelta>,
}

impl BlockLedger {
    /// Aggregates a parsed block's transactions.
    ///
    /// Summation is checked at every step, so an overflow is an error rather than a
    /// wrapped value that would silently corrupt the interval total.
    pub fn from_parsed(block: &ParsedBlock) -> Result<Self, ReconcileError> {
        let orchard_delta = Zatoshi::try_sum(block.transactions.iter().map(|tx| tx.orchard_delta))?;
        let ironwood_delta =
            Zatoshi::try_sum(block.transactions.iter().map(|tx| tx.ironwood_delta))?;

        Ok(Self {
            height: block.claimed_height,
            block_hash: block.block_hash.clone(),
            previous_block_hash: block.previous_block_hash.clone(),
            orchard_delta,
            ironwood_delta,
            transactions: block.transactions.clone(),
        })
    }

    pub fn transaction_count(&self) -> usize {
        self.transactions.len()
    }

    /// Transactions that moved value in at least one reconstructed pool.
    pub fn active_transactions(&self) -> impl Iterator<Item = &TransactionPoolDelta> {
        self.transactions.iter().filter(|tx| !tx.is_inert())
    }

    /// Whether any transaction in this block carries an Ironwood contribution.
    pub fn touches_ironwood(&self) -> bool {
        self.transactions
            .iter()
            .any(|tx| tx.ironwood_delta != Zatoshi::ZERO)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delta(index: u32, orchard: i64, ironwood: i64) -> TransactionPoolDelta {
        TransactionPoolDelta {
            txid: format!("{index:064}"),
            block_height: BlockHeight::new(3_428_143),
            tx_index: index,
            transaction_version: 6,
            orchard_delta: Zatoshi::from_raw(orchard),
            ironwood_delta: Zatoshi::from_raw(ironwood),
        }
    }

    fn block(transactions: Vec<TransactionPoolDelta>) -> ParsedBlock {
        ParsedBlock {
            claimed_height: BlockHeight::new(3_428_143),
            block_hash: "a".repeat(64),
            previous_block_hash: "b".repeat(64),
            transactions,
        }
    }

    #[test]
    fn a_block_with_no_shielded_activity_aggregates_to_zero() {
        let ledger = BlockLedger::from_parsed(&block(vec![delta(0, 0, 0)])).unwrap();
        assert_eq!(ledger.orchard_delta, Zatoshi::ZERO);
        assert_eq!(ledger.ironwood_delta, Zatoshi::ZERO);
        assert_eq!(ledger.transaction_count(), 1);
        assert_eq!(ledger.active_transactions().count(), 0);
    }

    #[test]
    fn orchard_only_activity_aggregates_into_orchard_alone() {
        let ledger =
            BlockLedger::from_parsed(&block(vec![delta(0, 0, 0), delta(1, -5_000, 0)])).unwrap();
        assert_eq!(ledger.orchard_delta, Zatoshi::from_raw(-5_000));
        assert_eq!(ledger.ironwood_delta, Zatoshi::ZERO);
        assert!(!ledger.touches_ironwood());
    }

    #[test]
    fn ironwood_only_activity_aggregates_into_ironwood_alone() {
        let ledger = BlockLedger::from_parsed(&block(vec![delta(0, 0, 3_000)])).unwrap();
        assert_eq!(ledger.orchard_delta, Zatoshi::ZERO);
        assert_eq!(ledger.ironwood_delta, Zatoshi::from_raw(3_000));
        assert!(ledger.touches_ironwood());
    }

    #[test]
    fn a_migration_within_one_block_nets_to_zero_across_pools() {
        let ledger =
            BlockLedger::from_parsed(&block(vec![delta(0, 0, 0), delta(1, -8_000, 8_000)]))
                .unwrap();
        assert_eq!(ledger.orchard_delta, Zatoshi::from_raw(-8_000));
        assert_eq!(ledger.ironwood_delta, Zatoshi::from_raw(8_000));
        assert_eq!(
            ledger
                .orchard_delta
                .checked_add(ledger.ironwood_delta)
                .unwrap(),
            Zatoshi::ZERO
        );
    }

    #[test]
    fn many_transactions_in_one_block_are_all_summed() {
        let transactions = (0..10).map(|i| delta(i, -100, 100)).collect();
        let ledger = BlockLedger::from_parsed(&block(transactions)).unwrap();
        assert_eq!(ledger.orchard_delta, Zatoshi::from_raw(-1_000));
        assert_eq!(ledger.ironwood_delta, Zatoshi::from_raw(1_000));
        assert_eq!(ledger.transaction_count(), 10);
        assert_eq!(ledger.active_transactions().count(), 10);
    }

    #[test]
    fn aggregation_overflow_is_an_error_not_a_wrap() {
        let transactions = vec![delta(0, i64::MAX, 0), delta(1, 1, 0)];
        assert!(matches!(
            BlockLedger::from_parsed(&block(transactions)),
            Err(ReconcileError::ArithmeticOverflow)
        ));
    }

    #[test]
    fn chain_linkage_fields_are_carried_through() {
        let ledger = BlockLedger::from_parsed(&block(vec![delta(0, 0, 0)])).unwrap();
        assert_eq!(ledger.block_hash, "a".repeat(64));
        assert_eq!(ledger.previous_block_hash, "b".repeat(64));
        assert_eq!(ledger.height, BlockHeight::new(3_428_143));
    }
}
