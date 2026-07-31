//! Collecting an evidence bundle from a node.
//!
//! This layer is where the network meets the filesystem, and the only place in the crate
//! where both are in scope at once. It orchestrates [`crate::rpc`] and [`crate::evidence`]
//! and holds no accounting logic of its own: it decides what to retrieve and whether the
//! result is fit to publish, never what the numbers mean.

pub mod fetch;
pub mod guard;
pub mod plan;
pub mod run;
pub mod writer;
