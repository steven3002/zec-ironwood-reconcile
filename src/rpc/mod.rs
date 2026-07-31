//! Node transport.
//!
//! This is the only part of the crate that opens a socket, and nothing downstream of
//! reconciliation may import it. That rule is what makes offline verification structural:
//! the code that reproduces a published result has no way to reach a network, whether or
//! not anyone remembers to check.
//!
//! The layer knows nothing about value-pool accounting. It retrieves what a node says and
//! hands it on unmodified.

pub mod auth;
pub mod client;
pub mod dto;
pub mod method;
