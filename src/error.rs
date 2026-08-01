//! Typed errors.
//!
//! Every variant carries a stable identifier via [`ReconcileError::stable_id`]. Reports
//! embed the identifier, never the rendered message, so that wording can be improved
//! without altering a published canonical report or its hash.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("RPC request failed: {0}")]
    Rpc(String),

    #[error("network mismatch: expected {expected}, received {actual}")]
    NetworkMismatch { expected: String, actual: String },

    #[error("unknown network: {value}")]
    UnknownNetwork { value: String },

    #[error("invalid interval: {reason}")]
    InvalidInterval { reason: String },

    #[error("missing block at height {0}")]
    MissingBlock(u32),

    #[error("block continuity failure at height {height}: {reason}")]
    BlockContinuity { height: u32, reason: String },

    #[error("unsupported transaction version {version} at height {height}")]
    UnsupportedTransaction { version: u32, height: u32 },

    #[error("transaction parse failure at height {height}, index {tx_index}: {reason}")]
    TransactionParse {
        height: u32,
        tx_index: u32,
        reason: String,
    },

    #[error("consensus branch id mismatch: expected {expected:#010x}, received {actual:#010x}")]
    BranchIdMismatch { expected: u32, actual: u32 },

    /// Raised when a node's declared upgrade schedule disagrees with the protocol constants
    /// compiled into this build, which would make every activation-dependent conclusion in
    /// a report unsound.
    #[error("activation context mismatch: {reason}")]
    ActivationMismatch { reason: String },

    #[error("evidence hash mismatch for {path}")]
    HashMismatch { path: String },

    #[error("evidence file missing: {path}")]
    MissingFile { path: String },

    #[error("evidence manifest invalid: {reason}")]
    ManifestInvalid { reason: String },

    #[error("evidence file {path} is inconsistent with the bundle: {reason}")]
    EvidenceInconsistent { path: String, reason: String },

    #[error("value-pool mismatch for {pool}: calculated {calculated}, reported {reported}")]
    PoolMismatch {
        pool: String,
        calculated: i128,
        reported: i128,
    },

    #[error("arithmetic overflow")]
    ArithmeticOverflow,

    #[error("value out of permitted bounds: {value}")]
    ValueOutOfBounds { value: i64 },

    /// Raised when the node served a response that cannot support a sound reconciliation,
    /// notably absent or non-numeric historical pool values, or a chain reorganisation
    /// detected between the start and end of a capture.
    #[error("capture incomplete: {reason}")]
    CaptureIncomplete { reason: String },

    #[error("invalid input: {reason}")]
    InvalidInput { reason: String },

    #[error("archive rejected: {reason}")]
    ArchiveRejected { reason: String },

    #[error("filesystem error at {path}: {source}")]
    Filesystem {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("internal error: {reason}")]
    Internal { reason: String },
}

impl ReconcileError {
    /// Stable machine-readable identifier, safe to embed in canonical reports.
    ///
    /// These strings are part of the report schema contract. Changing one is a breaking
    /// change to the report format and requires a schema version increment.
    pub const fn stable_id(&self) -> &'static str {
        match self {
            Self::Rpc(_) => "rpc_request_failed",
            Self::NetworkMismatch { .. } => "network_mismatch",
            Self::UnknownNetwork { .. } => "unknown_network",
            Self::InvalidInterval { .. } => "invalid_interval",
            Self::MissingBlock(_) => "missing_block",
            Self::BlockContinuity { .. } => "block_continuity_failure",
            Self::UnsupportedTransaction { .. } => "unsupported_transaction_version",
            Self::TransactionParse { .. } => "transaction_parse_failure",
            Self::BranchIdMismatch { .. } => "branch_id_mismatch",
            Self::ActivationMismatch { .. } => "activation_mismatch",
            Self::HashMismatch { .. } => "evidence_hash_mismatch",
            Self::MissingFile { .. } => "evidence_file_missing",
            Self::ManifestInvalid { .. } => "manifest_invalid",
            Self::EvidenceInconsistent { .. } => "evidence_inconsistent",
            Self::PoolMismatch { .. } => "pool_balance_mismatch",
            Self::ArithmeticOverflow => "arithmetic_overflow",
            Self::ValueOutOfBounds { .. } => "value_out_of_bounds",
            Self::CaptureIncomplete { .. } => "capture_incomplete",
            Self::InvalidInput { .. } => "invalid_input",
            Self::ArchiveRejected { .. } => "archive_rejected",
            Self::Filesystem { .. } => "filesystem_error",
            Self::Internal { .. } => "internal_error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_variant() -> Vec<ReconcileError> {
        vec![
            ReconcileError::Rpc("timeout".to_owned()),
            ReconcileError::NetworkMismatch {
                expected: "mainnet".to_owned(),
                actual: "testnet".to_owned(),
            },
            ReconcileError::UnknownNetwork {
                value: "regtest".to_owned(),
            },
            ReconcileError::InvalidInterval {
                reason: "reversed".to_owned(),
            },
            ReconcileError::MissingBlock(1),
            ReconcileError::BlockContinuity {
                height: 1,
                reason: "gap".to_owned(),
            },
            ReconcileError::UnsupportedTransaction {
                version: 7,
                height: 1,
            },
            ReconcileError::TransactionParse {
                height: 1,
                tx_index: 0,
                reason: "truncated".to_owned(),
            },
            ReconcileError::BranchIdMismatch {
                expected: 1,
                actual: 2,
            },
            ReconcileError::ActivationMismatch {
                reason: "height disagreement".to_owned(),
            },
            ReconcileError::HashMismatch {
                path: "blocks/1.hex".to_owned(),
            },
            ReconcileError::MissingFile {
                path: "blocks/1.hex".to_owned(),
            },
            ReconcileError::ManifestInvalid {
                reason: "schema".to_owned(),
            },
            ReconcileError::EvidenceInconsistent {
                path: "blocks/1.pools.json".to_owned(),
                reason: "declares another height".to_owned(),
            },
            ReconcileError::PoolMismatch {
                pool: "orchard".to_owned(),
                calculated: 1,
                reported: 2,
            },
            ReconcileError::ArithmeticOverflow,
            ReconcileError::ValueOutOfBounds { value: 1 },
            ReconcileError::CaptureIncomplete {
                reason: "null pool value".to_owned(),
            },
            ReconcileError::InvalidInput {
                reason: "bad flag".to_owned(),
            },
            ReconcileError::ArchiveRejected {
                reason: "traversal".to_owned(),
            },
            ReconcileError::Filesystem {
                path: "/tmp/x".to_owned(),
                source: std::io::Error::other("denied"),
            },
            ReconcileError::Internal {
                reason: "invariant".to_owned(),
            },
        ]
    }

    #[test]
    fn stable_ids_are_unique_across_variants() {
        let mut ids: Vec<&str> = every_variant().iter().map(|e| e.stable_id()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate stable error identifier");
    }

    #[test]
    fn stable_ids_are_snake_case_and_non_empty() {
        for error in every_variant() {
            let id = error.stable_id();
            assert!(!id.is_empty());
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "identifier is not snake_case: {id}"
            );
        }
    }

    #[test]
    fn branch_id_mismatch_renders_hexadecimal() {
        let error = ReconcileError::BranchIdMismatch {
            expected: 0x37A5_165B,
            actual: 0,
        };
        assert!(error.to_string().contains("0x37a5165b"));
    }
}
