//! Binds each captured pool-state record to the block it claims to describe.
//!
//! A bundle stores two files per height: the block's consensus bytes and the pool figures
//! the node reported for it. Nothing in the file names ties one to the other, a bundle in
//! which the pool files have been shuffled, or in which one belongs to a different block
//! entirely, is indistinguishable from an honest one by digest alone, because resealing a
//! manifest is something whoever produced the bundle can always do.
//!
//! Both files already carry the evidence needed to tie them together. The pool-state record
//! states the height and block hash it was read from, and this crate computes a block's hash
//! from the block's own header rather than accepting a node's word for it. Comparing the two
//! turns "these files sit next to each other" into "these files describe the same block".
//!
//! The capture path performs the equivalent check as it reads each height. This is the same
//! property established again on the reconciliation path, which is the one a third party
//! runs offline against a bundle they did not produce and whose capture they did not observe.

use crate::domain::height::BlockHeight;
use crate::error::ReconcileError;

/// Confirms a pool-state record describes the block captured at `height`.
///
/// The record's own claims are taken as plain values rather than as an evidence type, so
/// that this stays a statement about two heights and two hashes and the reconciliation layer
/// acquires no knowledge of how a bundle is laid out.
///
/// `computed_block_hash` must be the hash this crate derived from the block's own bytes, not
/// one read from the manifest or from a node response; otherwise the comparison is between
/// two assertions by the same author.
pub fn check_pool_state_describes_block(
    path: &str,
    height: BlockHeight,
    declared_height: BlockHeight,
    declared_block_hash: &str,
    computed_block_hash: &str,
) -> Result<(), ReconcileError> {
    if declared_height != height {
        return Err(ReconcileError::EvidenceInconsistent {
            path: path.to_owned(),
            reason: format!(
                "the record declares height {declared_height} but is stored as the pool state for height {height}"
            ),
        });
    }

    if !declared_block_hash.eq_ignore_ascii_case(computed_block_hash) {
        return Err(ReconcileError::EvidenceInconsistent {
            path: path.to_owned(),
            reason: format!(
                "the record describes block {declared_block_hash} but the block captured at \
                 height {height} hashes to {computed_block_hash}"
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0000003a8d8f580b0ca6dfc96ae95832e6d2732aef706bb17046b062eaae055a";
    const OTHER: &str = "000014ae5260f2c6eacf46fdaa04765e7e720bc9f99d3f7cdf0fa79263004128";

    fn check(
        stored_at: u32,
        declared_height: u32,
        declared_hash: &str,
        computed: &str,
    ) -> Result<(), ReconcileError> {
        check_pool_state_describes_block(
            "blocks/100.pools.json",
            BlockHeight::new(stored_at),
            BlockHeight::new(declared_height),
            declared_hash,
            computed,
        )
    }

    #[test]
    fn a_record_describing_its_own_block_is_accepted() {
        assert!(check(100, 100, HASH, HASH).is_ok());
    }

    #[test]
    fn a_pool_state_stored_under_another_height_is_refused() {
        match check(100, 105, HASH, HASH).unwrap_err() {
            ReconcileError::EvidenceInconsistent { path, reason } => {
                assert_eq!(path, "blocks/100.pools.json");
                assert!(reason.contains("105"), "{reason}");
                assert!(reason.contains("100"), "{reason}");
            }
            other => panic!("expected an inconsistency, got {other:?}"),
        }
    }

    #[test]
    fn a_pool_state_describing_a_different_block_at_the_right_height_is_refused() {
        // The height agrees, so only the hash binding can catch this one.
        match check(100, 100, OTHER, HASH).unwrap_err() {
            ReconcileError::EvidenceInconsistent { reason, .. } => {
                assert!(reason.contains(OTHER), "{reason}");
                assert!(reason.contains(HASH), "{reason}");
            }
            other => panic!("expected an inconsistency, got {other:?}"),
        }
    }

    #[test]
    fn hash_comparison_ignores_hexadecimal_case() {
        assert!(check(100, 100, &HASH.to_uppercase(), HASH).is_ok());
    }

    #[test]
    fn the_inconsistency_is_reported_as_unusable_evidence_not_a_filesystem_fault() {
        use crate::cli::exit::ExitCode;

        let error = check(100, 105, HASH, HASH).unwrap_err();
        assert_eq!(ExitCode::from(&error), ExitCode::EvidenceUnavailable);
    }
}
