//! Reconciliation result to published artifacts.
//!
//! Two artifacts are produced from one result: canonical JSON, which is what gets hashed
//! and verified, and Markdown, which is rendered from the same struct for a human reader.
//! There is exactly one accounting path.
//!
//! Canonicalization and hashing live in [`crate::canonical`], shared with the evidence
//! manifest so that both artifacts are hashed by identical code.

pub mod builder;
pub mod markdown;
pub mod performance;
pub mod schema;

pub use builder::{ReportContext, build};
pub use performance::PerformanceMetadata;
pub use schema::Report;

use crate::canonical;
use crate::error::ReconcileError;

/// Serializes a report to canonical bytes and returns them with their digest.
///
/// The bytes are returned alongside the digest so a caller writes exactly what was hashed,
/// rather than re-serializing and risking a divergent result.
pub fn canonical_bytes_and_hash(report: &Report) -> Result<(Vec<u8>, String), ReconcileError> {
    canonical::to_canonical_bytes_and_hash(report)
}

/// Parses a canonical report.
pub fn from_json_bytes(bytes: &[u8]) -> Result<Report, ReconcileError> {
    serde_json::from_slice(bytes).map_err(|source| ReconcileError::Internal {
        reason: format!("could not parse report: {source}"),
    })
}
