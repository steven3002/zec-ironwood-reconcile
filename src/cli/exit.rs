//! Process exit codes.
//!
//! This module is the sole authority mapping outcomes to exit codes. The codes are a
//! documented part of the tool's interface and are relied upon by scripted verification,
//! so they are stable across releases.
//!
//! Exit code `0` is returned only when every required check passed. A completed run whose
//! accounting comparison failed exits `1`.

use crate::error::ReconcileError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    /// Reconciliation completed and all required checks passed.
    Success = 0,
    /// Reconciliation completed but one or more checks failed.
    ChecksFailed = 1,
    InvalidInput = 2,
    ManifestInvalid = 3,
    EvidenceUnavailable = 4,
    UnsupportedTransaction = 5,
    BlockContinuity = 6,
    CaptureIncomplete = 7,
    ContextMismatch = 8,
    InternalError = 9,
    FilesystemError = 10,
}

impl ExitCode {
    pub const fn code(self) -> i32 {
        self as i32
    }
}

impl From<&ReconcileError> for ExitCode {
    fn from(error: &ReconcileError) -> Self {
        match error {
            ReconcileError::InvalidInput { .. }
            | ReconcileError::UnknownNetwork { .. }
            | ReconcileError::InvalidInterval { .. } => Self::InvalidInput,

            ReconcileError::ManifestInvalid { .. } => Self::ManifestInvalid,

            ReconcileError::HashMismatch { .. }
            | ReconcileError::MissingFile { .. }
            | ReconcileError::ArchiveRejected { .. } => Self::EvidenceUnavailable,

            ReconcileError::UnsupportedTransaction { .. }
            | ReconcileError::TransactionParse { .. } => Self::UnsupportedTransaction,

            ReconcileError::BlockContinuity { .. } | ReconcileError::MissingBlock(_) => {
                Self::BlockContinuity
            }

            // Absent historical pool values and mid-capture reorganisations both mean the
            // capture did not yield usable evidence.
            ReconcileError::Rpc(_) | ReconcileError::CaptureIncomplete { .. } => {
                Self::CaptureIncomplete
            }

            ReconcileError::NetworkMismatch { .. } | ReconcileError::BranchIdMismatch { .. } => {
                Self::ContextMismatch
            }

            ReconcileError::ArithmeticOverflow
            | ReconcileError::ValueOutOfBounds { .. }
            | ReconcileError::PoolMismatch { .. }
            | ReconcileError::Internal { .. } => Self::InternalError,

            ReconcileError::Filesystem { .. } => Self::FilesystemError,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_numeric_values_are_stable() {
        assert_eq!(ExitCode::Success.code(), 0);
        assert_eq!(ExitCode::ChecksFailed.code(), 1);
        assert_eq!(ExitCode::InvalidInput.code(), 2);
        assert_eq!(ExitCode::ManifestInvalid.code(), 3);
        assert_eq!(ExitCode::EvidenceUnavailable.code(), 4);
        assert_eq!(ExitCode::UnsupportedTransaction.code(), 5);
        assert_eq!(ExitCode::BlockContinuity.code(), 6);
        assert_eq!(ExitCode::CaptureIncomplete.code(), 7);
        assert_eq!(ExitCode::ContextMismatch.code(), 8);
        assert_eq!(ExitCode::InternalError.code(), 9);
        assert_eq!(ExitCode::FilesystemError.code(), 10);
    }

    #[test]
    fn hash_mismatch_reports_evidence_unavailable() {
        let error = ReconcileError::HashMismatch {
            path: "blocks/1.hex".to_owned(),
        };
        assert_eq!(ExitCode::from(&error), ExitCode::EvidenceUnavailable);
    }

    #[test]
    fn null_pool_values_report_capture_incomplete() {
        let error = ReconcileError::CaptureIncomplete {
            reason: "chainValueZat absent at height 3428142".to_owned(),
        };
        assert_eq!(ExitCode::from(&error), ExitCode::CaptureIncomplete);
    }

    #[test]
    fn branch_id_mismatch_reports_context_mismatch() {
        let error = ReconcileError::BranchIdMismatch {
            expected: 0x37A5_165B,
            actual: 0,
        };
        assert_eq!(ExitCode::from(&error), ExitCode::ContextMismatch);
    }

    #[test]
    fn continuity_failures_report_block_continuity() {
        assert_eq!(
            ExitCode::from(&ReconcileError::MissingBlock(10)),
            ExitCode::BlockContinuity
        );
        assert_eq!(
            ExitCode::from(&ReconcileError::BlockContinuity {
                height: 10,
                reason: "previous hash mismatch".to_owned(),
            }),
            ExitCode::BlockContinuity
        );
    }

    #[test]
    fn no_error_maps_to_success() {
        let errors = [
            ReconcileError::ArithmeticOverflow,
            ReconcileError::Rpc("x".to_owned()),
            ReconcileError::ManifestInvalid {
                reason: "x".to_owned(),
            },
        ];
        for error in errors {
            assert_ne!(ExitCode::from(&error), ExitCode::Success);
        }
    }
}
