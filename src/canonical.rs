//! Canonical JSON serialization.
//!
//! Report and manifest hashes are only reproducible if serialization is byte-stable across
//! machines, library versions and map iteration orders. This module is the sole producer of
//! bytes that get hashed, and applies RFC 8785 (JSON Canonicalization Scheme).
//!
//! # Monetary values
//!
//! RFC 8785 canonicalizes JSON numbers through IEEE-754 doubles, which cannot represent
//! integers above 2^53 exactly. Zatoshi values reach that magnitude, so every monetary
//! quantity is serialized as a string. [`crate::domain::zatoshi::Zatoshi`] enforces this at
//! the type level and rejects JSON numbers on deserialization.
//!
//! # Determinism
//!
//! Canonical output must be a pure function of the value being serialized. Callers are
//! responsible for keeping wall-clock timestamps, filesystem paths, hostnames and durations
//! out of any structure passed to this module when the result is used as a published hash.

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::ReconcileError;

/// Serializes a value to RFC 8785 canonical JSON bytes.
pub fn to_canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ReconcileError> {
    serde_jcs::to_vec(value).map_err(|source| ReconcileError::Internal {
        reason: format!("canonical serialization failed: {source}"),
    })
}

/// Serializes a value canonically and returns both the bytes and their SHA-256 digest.
///
/// The digest is returned alongside the bytes so a caller can persist exactly the bytes
/// that were hashed, rather than re-serializing and risking a divergent result.
pub fn to_canonical_bytes_and_hash<T: Serialize>(
    value: &T,
) -> Result<(Vec<u8>, String), ReconcileError> {
    let bytes = to_canonical_bytes(value)?;
    let digest = sha256_hex(&bytes);
    Ok((bytes, digest))
}

/// SHA-256 digest of `bytes`, as lowercase hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::HashMap;

    #[derive(Serialize)]
    struct Unordered {
        zulu: u32,
        alpha: u32,
        mike: u32,
    }

    #[derive(Serialize, Deserialize)]
    struct Nested {
        outer: Vec<Inner>,
    }

    #[derive(Serialize, Deserialize)]
    struct Inner {
        b: String,
        a: String,
    }

    #[test]
    fn object_keys_are_sorted_regardless_of_declaration_order() {
        let bytes = to_canonical_bytes(&Unordered {
            zulu: 1,
            alpha: 2,
            mike: 3,
        })
        .unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            r#"{"alpha":2,"mike":3,"zulu":1}"#
        );
    }

    #[test]
    fn output_contains_no_insignificant_whitespace() {
        let bytes = to_canonical_bytes(&Nested {
            outer: vec![Inner {
                b: "second".to_owned(),
                a: "first".to_owned(),
            }],
        })
        .unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert_eq!(text, r#"{"outer":[{"a":"first","b":"second"}]}"#);
        assert!(!text.contains(' '));
        assert!(!text.contains('\n'));
    }

    #[test]
    fn hash_map_iteration_order_does_not_affect_output() {
        // Canonical output must not inherit the map's unspecified iteration order.
        let mut first = HashMap::new();
        let mut second = HashMap::new();
        for key in ["delta", "alpha", "charlie", "bravo", "echo"] {
            first.insert(key.to_owned(), key.len());
        }
        for key in ["echo", "bravo", "charlie", "alpha", "delta"] {
            second.insert(key.to_owned(), key.len());
        }
        assert_eq!(
            to_canonical_bytes(&first).unwrap(),
            to_canonical_bytes(&second).unwrap()
        );
    }

    #[test]
    fn serialization_is_stable_across_repeated_calls() {
        let value = Nested {
            outer: vec![
                Inner {
                    b: "one".to_owned(),
                    a: "two".to_owned(),
                },
                Inner {
                    b: "three".to_owned(),
                    a: "four".to_owned(),
                },
            ],
        };
        let first = to_canonical_bytes_and_hash(&value).unwrap();
        let second = to_canonical_bytes_and_hash(&value).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn digest_is_lowercase_hex_of_the_canonical_bytes() {
        let value = Unordered {
            zulu: 1,
            alpha: 2,
            mike: 3,
        };
        let (bytes, digest) = to_canonical_bytes_and_hash(&value).unwrap();
        assert_eq!(digest, sha256_hex(&bytes));
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!digest.chars().any(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn known_digest_of_the_empty_object() {
        // SHA-256 of the two bytes "{}".
        let value: HashMap<String, String> = HashMap::new();
        let (bytes, digest) = to_canonical_bytes_and_hash(&value).unwrap();
        assert_eq!(bytes, b"{}");
        assert_eq!(
            digest,
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
        );
    }
}
