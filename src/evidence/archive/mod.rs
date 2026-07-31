//! Evidence bundle packaging.
//!
//! Packing is deterministic; extraction is hardened. The asymmetry is deliberate: this tool
//! produces archives for its own operator and consumes archives from anyone.

pub mod extract;
pub mod limits;
pub mod pack;

pub use extract::{ExtractionSummary, extract};
pub use limits::ExtractionLimits;
pub use pack::{pack, pack_with_digest};
