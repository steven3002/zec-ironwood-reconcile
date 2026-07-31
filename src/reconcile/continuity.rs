//! Block sequence and chain linkage validation.
//!
//! A continuity failure invalidates the entire reconciliation. Summing deltas over blocks
//! that do not form an unbroken chain from the declared anchor produces a number, but not a
//! meaningful one: a missing or substituted block would silently omit or invent value
//! movement.
//!
//! Linkage is verified against block hashes this crate computed from block headers, not
//! against hashes a node asserted, so the check is independent of the node being examined.

use std::collections::BTreeSet;

use crate::domain::height::HeightInterval;
use crate::error::ReconcileError;
use crate::reconcile::ledger::BlockLedger;

/// The chain endpoints a bundle declares, against which the captured sequence is anchored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainEndpoints {
    /// Hash of the anchor block, the block immediately preceding the interval.
    pub anchor_block_hash: String,
    /// Hash the capture recorded for the final block of the interval.
    pub end_block_hash: String,
}

/// Verifies that the ledgers form an unbroken chain covering exactly the declared interval.
///
/// The ledgers are expected in ascending height order, as captured.
pub fn verify(
    ledgers: &[BlockLedger],
    interval: HeightInterval,
    endpoints: &ChainEndpoints,
) -> Result<(), ReconcileError> {
    verify_coverage(ledgers, interval)?;
    verify_linkage(ledgers, endpoints)?;
    Ok(())
}

/// Every requested height present exactly once, in ascending order, with no gaps.
fn verify_coverage(
    ledgers: &[BlockLedger],
    interval: HeightInterval,
) -> Result<(), ReconcileError> {
    let mut seen = BTreeSet::new();
    for ledger in ledgers {
        if !seen.insert(ledger.height) {
            return Err(ReconcileError::BlockContinuity {
                height: ledger.height.get(),
                reason: "duplicate height in the captured sequence".to_owned(),
            });
        }
        if !interval.contains(ledger.height) {
            return Err(ReconcileError::BlockContinuity {
                height: ledger.height.get(),
                reason: format!(
                    "height lies outside the declared interval {}..={}",
                    interval.start_height(),
                    interval.end_height()
                ),
            });
        }
    }

    for expected in interval.heights() {
        if !seen.contains(&expected) {
            return Err(ReconcileError::MissingBlock(expected.get()));
        }
    }

    // Ascending order is required so that cumulative balances are applied in chain order.
    for pair in ledgers.windows(2) {
        if let [previous, current] = pair {
            let next = previous.height.checked_next()?;
            if current.height != next {
                return Err(ReconcileError::BlockContinuity {
                    height: current.height.get(),
                    reason: format!(
                        "heights must increase by exactly one; {} follows {}",
                        current.height, previous.height
                    ),
                });
            }
        }
    }

    Ok(())
}

/// Each block's recorded previous hash matches its predecessor, and the endpoints match.
fn verify_linkage(
    ledgers: &[BlockLedger],
    endpoints: &ChainEndpoints,
) -> Result<(), ReconcileError> {
    let Some(first) = ledgers.first() else {
        return Err(ReconcileError::BlockContinuity {
            height: 0,
            reason: "the captured sequence contains no blocks".to_owned(),
        });
    };

    if first.previous_block_hash != endpoints.anchor_block_hash {
        return Err(ReconcileError::BlockContinuity {
            height: first.height.get(),
            reason: format!(
                "first block does not link to the declared anchor; it points at {}, anchor is {}",
                first.previous_block_hash, endpoints.anchor_block_hash
            ),
        });
    }

    for pair in ledgers.windows(2) {
        if let [previous, current] = pair
            && current.previous_block_hash != previous.block_hash
        {
            return Err(ReconcileError::BlockContinuity {
                height: current.height.get(),
                reason: format!(
                    "block does not link to its predecessor; it points at {}, predecessor hashes to {}",
                    current.previous_block_hash, previous.block_hash
                ),
            });
        }
    }

    let Some(last) = ledgers.last() else {
        return Err(ReconcileError::BlockContinuity {
            height: 0,
            reason: "the captured sequence contains no blocks".to_owned(),
        });
    };

    if last.block_hash != endpoints.end_block_hash {
        return Err(ReconcileError::BlockContinuity {
            height: last.height.get(),
            reason: format!(
                "final block hashes to {}, but the capture recorded {}",
                last.block_hash, endpoints.end_block_hash
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::height::BlockHeight;
    use crate::domain::zatoshi::Zatoshi;

    fn hash(label: u32) -> String {
        format!("{label:064x}")
    }

    fn ledger(height: u32) -> BlockLedger {
        BlockLedger {
            height: BlockHeight::new(height),
            block_hash: hash(height),
            previous_block_hash: hash(height.saturating_sub(1)),
            orchard_delta: Zatoshi::ZERO,
            ironwood_delta: Zatoshi::ZERO,
            transactions: Vec::new(),
        }
    }

    fn chain(start: u32, end: u32) -> Vec<BlockLedger> {
        (start..=end).map(ledger).collect()
    }

    fn interval(start: u32, end: u32) -> HeightInterval {
        HeightInterval::new(BlockHeight::new(start), BlockHeight::new(end)).unwrap()
    }

    fn endpoints(anchor: u32, end: u32) -> ChainEndpoints {
        ChainEndpoints {
            anchor_block_hash: hash(anchor),
            end_block_hash: hash(end),
        }
    }

    #[test]
    fn an_unbroken_chain_passes() {
        let ledgers = chain(101, 105);
        assert!(verify(&ledgers, interval(101, 105), &endpoints(100, 105)).is_ok());
    }

    #[test]
    fn a_single_block_interval_passes() {
        let ledgers = chain(101, 101);
        assert!(verify(&ledgers, interval(101, 101), &endpoints(100, 101)).is_ok());
    }

    #[test]
    fn a_missing_block_is_detected() {
        let mut ledgers = chain(101, 105);
        ledgers.remove(2);
        assert!(matches!(
            verify(&ledgers, interval(101, 105), &endpoints(100, 105)),
            Err(ReconcileError::MissingBlock(103))
        ));
    }

    #[test]
    fn a_duplicate_height_is_detected() {
        let mut ledgers = chain(101, 105);
        ledgers.push(ledger(103));
        assert!(matches!(
            verify(&ledgers, interval(101, 105), &endpoints(100, 105)),
            Err(ReconcileError::BlockContinuity { .. })
        ));
    }

    #[test]
    fn a_wrong_previous_hash_breaks_linkage() {
        let mut ledgers = chain(101, 105);
        ledgers[3].previous_block_hash = hash(9_999);

        match verify(&ledgers, interval(101, 105), &endpoints(100, 105)) {
            Err(ReconcileError::BlockContinuity { height, reason }) => {
                assert_eq!(height, 104);
                assert!(reason.contains("predecessor"), "{reason}");
            }
            other => panic!("expected a linkage failure, got {other:?}"),
        }
    }

    #[test]
    fn the_first_block_must_link_to_the_anchor() {
        let ledgers = chain(101, 105);
        match verify(&ledgers, interval(101, 105), &endpoints(9_999, 105)) {
            Err(ReconcileError::BlockContinuity { height, reason }) => {
                assert_eq!(height, 101);
                assert!(reason.contains("anchor"), "{reason}");
            }
            other => panic!("expected an anchor linkage failure, got {other:?}"),
        }
    }

    #[test]
    fn the_final_block_must_match_the_recorded_end_hash() {
        let ledgers = chain(101, 105);
        match verify(&ledgers, interval(101, 105), &endpoints(100, 9_999)) {
            Err(ReconcileError::BlockContinuity { height, reason }) => {
                assert_eq!(height, 105);
                assert!(reason.contains("final block"), "{reason}");
            }
            other => panic!("expected an end hash failure, got {other:?}"),
        }
    }

    #[test]
    fn a_height_outside_the_interval_is_rejected() {
        let mut ledgers = chain(101, 105);
        ledgers.push(ledger(106));
        assert!(matches!(
            verify(&ledgers, interval(101, 105), &endpoints(100, 105)),
            Err(ReconcileError::BlockContinuity { .. })
        ));
    }

    #[test]
    fn an_empty_sequence_is_rejected() {
        assert!(verify(&[], interval(101, 105), &endpoints(100, 105)).is_err());
    }

    #[test]
    fn a_substituted_block_at_the_right_height_is_still_detected() {
        // A block of the correct height whose contents differ hashes differently, so the
        // next block no longer links to it.
        let mut ledgers = chain(101, 105);
        ledgers[2].block_hash = hash(7_777);
        assert!(verify(&ledgers, interval(101, 105), &endpoints(100, 105)).is_err());
    }
}
