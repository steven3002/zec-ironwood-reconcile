//! The evidence bundle manifest.
//!
//! The manifest is the versioned index of a bundle: what interval it covers, which node
//! produced it, what the declared anchor and reported ending state were, and the digest of
//! every file it contains. Verifying a bundle means verifying the manifest and then
//! verifying that every listed file still hashes to its recorded digest.
//!
//! The manifest is hashed through [`crate::canonical`], so its digest is independent of
//! field declaration order and of any map iteration order.

use serde::{Deserialize, Deserializer, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::canonical;
use crate::domain::height::{BlockHeight, HeightInterval};
use crate::domain::network::Network;
use crate::domain::zatoshi::Zatoshi;
use crate::error::ReconcileError;
use crate::evidence::layout;

/// Manifest schema version understood by this build.
///
/// A bundle declaring a different major version is rejected rather than interpreted, so
/// that a future format cannot be silently misread as the current one. A minor increment
/// signals an additive change that an older reader can ignore safely.
///
/// `1.1.0` added `end.tracking`, recording whether the capturing node said it was tracking
/// each reconstructed pool.
pub const SCHEMA_VERSION: &str = "1.1.0";

/// An RFC 3339 timestamp in UTC.
///
/// Timestamps appear in manifests only. They are deliberately absent from canonical
/// reports, whose hashes must be a pure function of the evidence and the tool version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Rfc3339Timestamp(String);

impl Rfc3339Timestamp {
    pub fn parse(value: &str) -> Result<Self, ReconcileError> {
        OffsetDateTime::parse(value, &Rfc3339).map_err(|_| ReconcileError::ManifestInvalid {
            reason: format!("timestamp is not valid RFC 3339: {value:?}"),
        })?;
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Rfc3339Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// How a bundle file is encoded.
///
/// Recorded per file so a reader never has to infer an encoding from a file extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Encoding {
    /// Raw consensus bytes, hex encoded.
    RawBlockHex,
    /// A node RPC response or structured metadata, as JSON.
    Json,
    /// Plain UTF-8 text.
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub encoding: Encoding,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    pub name: String,
    pub version: String,
    pub git_commit: Option<String>,
}

/// Identity of the node the evidence was captured from.
///
/// The RPC endpoint is never recorded, only the fact that it was redacted. An endpoint may
/// embed credentials or reveal a private host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub implementation: String,
    pub version: String,
    pub rpc_url_redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activation {
    pub upgrade: String,
    pub expected_height: BlockHeight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Interval {
    pub anchor_height: BlockHeight,
    pub start_height: BlockHeight,
    pub end_height: BlockHeight,
    pub block_count: u32,
}

impl From<HeightInterval> for Interval {
    fn from(interval: HeightInterval) -> Self {
        Self {
            anchor_height: interval.anchor_height(),
            start_height: interval.start_height(),
            end_height: interval.end_height(),
            block_count: interval.block_count(),
        }
    }
}

/// Declared balances at the anchor block, the starting point of all interval arithmetic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorState {
    pub block_hash: String,
    pub orchard_balance_zatoshis: Zatoshi,
    pub ironwood_balance_zatoshis: Zatoshi,
}

/// The node's `monitored` flag for each reconstructed pool at the end height.
///
/// **The field names are misnomers, kept only for schema compatibility.** They were chosen
/// on the reading that `monitored` states whether a node is tracking a pool. It does not:
/// Zebra builds every pool entry through one constructor that sets
/// `monitored: amount.zatoshis() != 0`, so the flag restates whether the balance is non-zero
/// and says nothing about tracking.
///
/// The value is recorded because it is part of the response, not because anything depends on
/// it. Nothing in the reconciliation reads it, and no check may be derived from it — treating
/// it as a tracking signal downgrades every correct comparison against a legitimately empty
/// pool, which is every capture of the activation boundary.
///
/// `None` means the node published no such field at all.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndStateTracking {
    pub orchard_tracked_by_node: Option<bool>,
    pub ironwood_tracked_by_node: Option<bool>,
}

/// Balances the capturing node reported at the end height, which reconciliation compares
/// its own reconstruction against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndState {
    pub block_hash: String,
    pub reported_orchard_balance_zatoshis: Zatoshi,
    pub reported_ironwood_balance_zatoshis: Zatoshi,
    #[serde(default)]
    pub tracking: EndStateTracking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: String,
    pub bundle_id: String,
    pub created_at: Rfc3339Timestamp,
    pub tool: Tool,
    pub source: Source,
    pub network: Network,
    pub activation: Activation,
    pub interval: Interval,
    pub anchor: AnchorState,
    pub end: EndState,
    pub files: Vec<FileEntry>,
}

impl Manifest {
    /// Derives the canonical bundle identifier, `<network>-<anchor>-<end>`.
    pub fn derive_bundle_id(network: Network, interval: HeightInterval) -> String {
        format!(
            "{}-{}-{}",
            network.name(),
            interval.anchor_height(),
            interval.end_height()
        )
    }

    /// Adds a file entry, keeping the list sorted by path.
    ///
    /// Sortedness is maintained on insertion rather than at serialization time so that a
    /// manifest is deterministic regardless of the order in which capture wrote files.
    pub fn add_file(&mut self, entry: FileEntry) {
        let position = self
            .files
            .binary_search_by(|existing| existing.path.as_str().cmp(entry.path.as_str()));
        match position {
            Ok(index) | Err(index) => self.files.insert(index, entry),
        }
    }

    pub fn find_file(&self, path: &str) -> Option<&FileEntry> {
        self.files
            .binary_search_by(|existing| existing.path.as_str().cmp(path))
            .ok()
            .and_then(|index| self.files.get(index))
    }

    /// Total size of all listed files.
    pub fn total_size_bytes(&self) -> Result<u64, ReconcileError> {
        self.files
            .iter()
            .try_fold(0_u64, |total, entry| total.checked_add(entry.size_bytes))
            .ok_or(ReconcileError::ArithmeticOverflow)
    }

    pub fn to_canonical_bytes_and_hash(&self) -> Result<(Vec<u8>, String), ReconcileError> {
        canonical::to_canonical_bytes_and_hash(self)
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, ReconcileError> {
        serde_json::from_slice(bytes).map_err(|source| ReconcileError::ManifestInvalid {
            reason: format!("could not parse manifest: {source}"),
        })
    }

    /// Checks the manifest is internally consistent and safe to act on.
    ///
    /// This runs before any file is opened, because the file list is untrusted input.
    pub fn validate_structure(&self) -> Result<(), ReconcileError> {
        self.validate_schema_version()?;

        let expected_id = format!(
            "{}-{}-{}",
            self.network.name(),
            self.interval.anchor_height,
            self.interval.end_height
        );
        if self.bundle_id != expected_id {
            return Err(ReconcileError::ManifestInvalid {
                reason: format!(
                    "bundle id {:?} does not match interval, expected {expected_id:?}",
                    self.bundle_id
                ),
            });
        }

        self.validate_interval()?;
        self.validate_files()?;
        Ok(())
    }

    /// Rejects any schema version whose major component this build does not implement.
    fn validate_schema_version(&self) -> Result<(), ReconcileError> {
        let major = |version: &str| {
            version
                .split('.')
                .next()
                .and_then(|part| part.parse::<u32>().ok())
        };

        let declared =
            major(&self.schema_version).ok_or_else(|| ReconcileError::ManifestInvalid {
                reason: format!("unparseable schema version {:?}", self.schema_version),
            })?;
        let supported = major(SCHEMA_VERSION).unwrap_or(0);

        if declared != supported {
            return Err(ReconcileError::ManifestInvalid {
                reason: format!(
                    "unsupported schema version {:?}; this build implements {SCHEMA_VERSION}",
                    self.schema_version
                ),
            });
        }
        Ok(())
    }

    fn validate_interval(&self) -> Result<(), ReconcileError> {
        let interval = HeightInterval::new(self.interval.start_height, self.interval.end_height)
            .map_err(|error| ReconcileError::ManifestInvalid {
                reason: error.to_string(),
            })?;

        if interval.anchor_height() != self.interval.anchor_height {
            return Err(ReconcileError::ManifestInvalid {
                reason: format!(
                    "anchor height {} is not one below start height {}",
                    self.interval.anchor_height, self.interval.start_height
                ),
            });
        }

        if interval.block_count() != self.interval.block_count {
            return Err(ReconcileError::ManifestInvalid {
                reason: format!(
                    "block count {} does not match interval, expected {}",
                    self.interval.block_count,
                    interval.block_count()
                ),
            });
        }

        Ok(())
    }

    fn validate_files(&self) -> Result<(), ReconcileError> {
        let mut previous: Option<&str> = None;

        for entry in &self.files {
            layout::validate_relative_path(&entry.path)?;

            if entry.sha256.len() != 64 || !entry.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(ReconcileError::ManifestInvalid {
                    reason: format!(
                        "file {:?} has a malformed digest {:?}",
                        entry.path, entry.sha256
                    ),
                });
            }
            if entry.sha256.chars().any(|c| c.is_ascii_uppercase()) {
                return Err(ReconcileError::ManifestInvalid {
                    reason: format!("file {:?} digest is not lowercase", entry.path),
                });
            }

            match previous {
                Some(previous_path) if previous_path == entry.path => {
                    return Err(ReconcileError::ManifestInvalid {
                        reason: format!("duplicate file entry {:?}", entry.path),
                    });
                }
                Some(previous_path) if previous_path > entry.path.as_str() => {
                    return Err(ReconcileError::ManifestInvalid {
                        reason: "file entries are not sorted by path".to_owned(),
                    });
                }
                _ => {}
            }
            previous = Some(&entry.path);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interval() -> HeightInterval {
        HeightInterval::new(BlockHeight::new(3_428_143), BlockHeight::new(3_429_143)).unwrap()
    }

    fn entry(path: &str) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned(),
            size_bytes: 0,
            encoding: Encoding::Json,
        }
    }

    fn manifest() -> Manifest {
        let interval = interval();
        Manifest {
            schema_version: SCHEMA_VERSION.to_owned(),
            bundle_id: Manifest::derive_bundle_id(Network::Mainnet, interval),
            created_at: Rfc3339Timestamp::parse("2026-07-29T14:30:00Z").unwrap(),
            tool: Tool {
                name: "zec-ironwood-reconcile".to_owned(),
                version: "0.1.0".to_owned(),
                git_commit: None,
            },
            source: Source {
                implementation: "zebra".to_owned(),
                version: "6.2.3".to_owned(),
                rpc_url_redacted: true,
            },
            network: Network::Mainnet,
            activation: Activation {
                upgrade: "NU6.3".to_owned(),
                expected_height: Network::Mainnet.ironwood_activation_height(),
            },
            interval: interval.into(),
            anchor: AnchorState {
                block_hash: "0".repeat(64),
                orchard_balance_zatoshis: Zatoshi::from_raw(366_000_000_000_000),
                ironwood_balance_zatoshis: Zatoshi::ZERO,
            },
            end: EndState {
                block_hash: "1".repeat(64),
                reported_orchard_balance_zatoshis: Zatoshi::from_raw(348_400_000_000_000),
                reported_ironwood_balance_zatoshis: Zatoshi::from_raw(17_600_000_000_000),
                tracking: EndStateTracking::default(),
            },
            files: Vec::new(),
        }
    }

    #[test]
    fn bundle_id_is_derived_from_network_and_interval() {
        assert_eq!(
            Manifest::derive_bundle_id(Network::Mainnet, interval()),
            "mainnet-3428142-3429143"
        );
    }

    #[test]
    fn a_well_formed_manifest_validates() {
        let mut manifest = manifest();
        manifest.add_file(entry(layout::ANCHOR_BLOCK));
        manifest.add_file(entry("blocks/3428143.hex"));
        assert!(manifest.validate_structure().is_ok());
    }

    #[test]
    fn files_are_kept_sorted_regardless_of_insertion_order() {
        let mut manifest = manifest();
        for path in ["metadata/command.txt", "anchor/block.hex", "blocks/1.hex"] {
            manifest.add_file(entry(path));
        }
        let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["anchor/block.hex", "blocks/1.hex", "metadata/command.txt"]
        );
        assert!(manifest.validate_structure().is_ok());
    }

    #[test]
    fn insertion_order_does_not_change_the_manifest_hash() {
        let mut first = manifest();
        let mut second = manifest();
        for path in ["metadata/command.txt", "anchor/block.hex", "blocks/1.hex"] {
            first.add_file(entry(path));
        }
        for path in ["blocks/1.hex", "metadata/command.txt", "anchor/block.hex"] {
            second.add_file(entry(path));
        }
        assert_eq!(
            first.to_canonical_bytes_and_hash().unwrap(),
            second.to_canonical_bytes_and_hash().unwrap()
        );
    }

    #[test]
    fn an_unsupported_major_schema_version_is_rejected() {
        let mut manifest = manifest();
        manifest.schema_version = "2.0.0".to_owned();
        assert!(matches!(
            manifest.validate_structure(),
            Err(ReconcileError::ManifestInvalid { .. })
        ));
    }

    #[test]
    fn a_later_minor_schema_version_is_accepted() {
        let mut manifest = manifest();
        manifest.schema_version = "1.4.2".to_owned();
        assert!(manifest.validate_structure().is_ok());
    }

    #[test]
    fn a_traversing_file_path_is_rejected() {
        let mut manifest = manifest();
        manifest.add_file(entry("../../etc/passwd"));
        assert!(matches!(
            manifest.validate_structure(),
            Err(ReconcileError::ManifestInvalid { .. })
        ));
    }

    #[test]
    fn a_malformed_digest_is_rejected() {
        let mut manifest = manifest();
        let mut bad = entry("blocks/1.hex");
        bad.sha256 = "not-a-digest".to_owned();
        manifest.add_file(bad);
        assert!(manifest.validate_structure().is_err());
    }

    #[test]
    fn an_uppercase_digest_is_rejected() {
        let mut manifest = manifest();
        let mut bad = entry("blocks/1.hex");
        bad.sha256 = bad.sha256.to_uppercase();
        manifest.add_file(bad);
        assert!(manifest.validate_structure().is_err());
    }

    #[test]
    fn duplicate_file_entries_are_rejected() {
        let mut manifest = manifest();
        manifest.add_file(entry("blocks/1.hex"));
        manifest.add_file(entry("blocks/1.hex"));
        assert!(manifest.validate_structure().is_err());
    }

    #[test]
    fn a_bundle_id_inconsistent_with_the_interval_is_rejected() {
        let mut manifest = manifest();
        manifest.bundle_id = "mainnet-1-2".to_owned();
        assert!(manifest.validate_structure().is_err());
    }

    #[test]
    fn an_anchor_not_immediately_preceding_start_is_rejected() {
        let mut manifest = manifest();
        manifest.interval.anchor_height = BlockHeight::new(3_428_000);
        manifest.bundle_id = "mainnet-3428000-3429143".to_owned();
        assert!(manifest.validate_structure().is_err());
    }

    #[test]
    fn an_incorrect_block_count_is_rejected() {
        let mut manifest = manifest();
        manifest.interval.block_count = 999;
        assert!(manifest.validate_structure().is_err());
    }

    #[test]
    fn a_non_rfc3339_timestamp_is_rejected() {
        assert!(Rfc3339Timestamp::parse("29 July 2026").is_err());
        assert!(Rfc3339Timestamp::parse("2026-07-29 14:30:00").is_err());
        assert!(Rfc3339Timestamp::parse("2026-07-29T14:30:00Z").is_ok());
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let mut original = manifest();
        original.add_file(entry("blocks/3428143.hex"));
        let bytes = serde_json::to_vec(&original).unwrap();
        let parsed = Manifest::from_json_bytes(&bytes).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn monetary_fields_serialize_as_strings() {
        let text = serde_json::to_string(&manifest()).unwrap();
        assert!(text.contains(r#""ironwood_balance_zatoshis":"0""#));
        assert!(text.contains(r#""reported_ironwood_balance_zatoshis":"17600000000000""#));
    }

    #[test]
    fn total_size_detects_overflow() {
        let mut manifest = manifest();
        for path in ["a.json", "b.json"] {
            let mut large = entry(path);
            large.size_bytes = u64::MAX;
            manifest.add_file(large);
        }
        assert!(matches!(
            manifest.total_size_bytes(),
            Err(ReconcileError::ArithmeticOverflow)
        ));
    }

    #[test]
    fn find_file_locates_entries_by_path() {
        let mut manifest = manifest();
        manifest.add_file(entry("blocks/3428143.hex"));
        assert!(manifest.find_file("blocks/3428143.hex").is_some());
        assert!(manifest.find_file("blocks/9999999.hex").is_none());
    }
}
