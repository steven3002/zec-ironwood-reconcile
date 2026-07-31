//! Raw consensus bytes to signed pool deltas.
//!
//! Deserialization is delegated to `zcash_primitives`, a codebase independent of the node
//! whose figures this tool cross-checks. The accounting interpretation of what those bytes
//! mean — the sign convention, the per-transaction deltas, the treatment of absent bundles
//! — is implemented here.
//!
//! Nothing in this module performs I/O or reaches the network. It is given bytes and
//! returns deltas, which is what allows the same code path to serve both `reconcile` and
//! offline `verify`.
//!
//! Malformed input is never silently skipped. Unknown versions, truncated transactions,
//! unparseable blocks and out-of-range values each produce a deterministic failure carrying
//! the height, the transaction index, and a stable error identifier.

pub mod block;
pub mod transaction;
pub mod value_balance;

pub use block::ParsedBlock;
pub use transaction::TransactionPoolDelta;
