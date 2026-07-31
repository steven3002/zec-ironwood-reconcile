//! The evidence bundle: the artifact this tool produces and verifies.
//!
//! A bundle is a directory containing raw captured chain data, the values the capturing
//! node reported, and a versioned manifest listing every file with its digest. Everything
//! downstream is derived from it, and offline verification is verification of it.

pub mod hashing;
pub mod layout;
pub mod manifest;
pub mod validation;
