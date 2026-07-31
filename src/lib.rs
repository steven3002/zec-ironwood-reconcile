//! Reconstructs Orchard and Ironwood chain value-pool changes over a bounded interval of
//! Zcash blocks, from transaction-level public data, and compares the result against the
//! balances reported by a Zebra node.
//!
//! # Architecture
//!
//! The crate is a pure core with I/O confined to its edges. Deserialization, accounting,
//! verdicts and reporting touch no network, no clock and no filesystem; only capture and
//! evidence handling do. Offline verification and byte-identical report hashes follow from
//! that separation rather than from being maintained as features.
//!
//! # Claim boundary
//!
//! Given a declared starting anchor and a continuous sequence of public blocks, this crate
//! reconstructs value-pool changes over the interval and compares the calculated ending
//! balances with those a node reports. It does not prove total supply from genesis, verify
//! zero-knowledge proofs, validate consensus rules generally, establish circuit soundness,
//! determine whether historical counterfeiting occurred, or replace full-node validation.

// Panicking accessors are denied in production code so that every failure path is a typed
// error with a documented exit code. Tests assert on known-good values, where unwrapping is
// the clearer expression of intent.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        clippy::cast_possible_truncation
    )
)]

pub mod canonical;
pub mod checks;
pub mod cli;
pub mod commands;
pub mod domain;
pub mod error;
pub mod evidence;
pub mod parse;
pub mod reconcile;
pub mod report;
