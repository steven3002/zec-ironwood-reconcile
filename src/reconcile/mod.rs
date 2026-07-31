//! Deltas to interval result.
//!
//! Aggregation, accumulation and chain-linkage validation. Nothing here performs I/O or
//! reaches the network, which is what allows `reconcile` and offline `verify` to execute
//! the identical code path over the identical bytes.

pub mod continuity;
pub mod interval;
pub mod ledger;

pub use continuity::ChainEndpoints;
pub use interval::{AnchorBalances, IntervalOutcome};
pub use ledger::BlockLedger;
