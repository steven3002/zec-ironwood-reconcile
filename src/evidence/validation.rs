//! Evidence bundle validation.
//!
//! Validation answers one question: do the bytes on disk still match what the manifest
//! claims? It runs before any accounting, because a reconciliation over altered evidence
//! would be meaningless regardless of whether the arithmetic is correct.
//!
//! Failures and warnings are kept in separate collections. A warning must never be
//! mistakable for a pass, and a failure must never be downgraded into one.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::canonical;
use crate::error::ReconcileError;
use crate::evidence::hashing;
use crate::evidence::layout;
use crate::evidence::manifest::Manifest;

/// A condition worth reporting that does not invalidate the evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationWarning {
    /// A file exists in the bundle but is absent from the manifest. It contributed nothing
    /// to any hash and is ignored, but its presence is unexplained.
    UnlistedFile { path: String },
    /// The bundle carries no detached manifest digest, so the manifest itself was not
    /// independently pinned.
    MissingManifestDigest,
}

impl std::fmt::Display for ValidationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnlistedFile { path } => {
                write!(f, "file present in bundle but absent from manifest: {path}")
            }
            Self::MissingManifestDigest => {
                write!(f, "bundle contains no {} file", layout::MANIFEST_HASH)
            }
        }
    }
}

/// Outcome of validating a bundle against its manifest.
#[derive(Debug, Default)]
pub struct ValidationReport {
    pub failures: Vec<ReconcileError>,
    pub warnings: Vec<ValidationWarning>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.failures.is_empty()
    }

    /// Reduces the report to a result, surfacing the first failure.
    ///
    /// All failures are collected during validation so that a caller can report every
    /// damaged file at once; this is for callers that only need to proceed or stop.
    pub fn into_result(mut self) -> Result<Vec<ValidationWarning>, ReconcileError> {
        if self.failures.is_empty() {
            Ok(self.warnings)
        } else {
            Err(self.failures.swap_remove(0))
        }
    }
}

/// Reads and structurally validates the manifest at a bundle root.
pub fn load_manifest(root: &Path) -> Result<Manifest, ReconcileError> {
    let path = layout::resolve(root, layout::MANIFEST)?;
    let bytes = fs::read(&path).map_err(|source| ReconcileError::Filesystem {
        path: path.display().to_string(),
        source,
    })?;
    let manifest = Manifest::from_json_bytes(&bytes)?;
    manifest.validate_structure()?;
    Ok(manifest)
}

/// Verifies the detached manifest digest, when one is present.
///
/// The digest covers the manifest's canonical serialization, not the bytes of
/// `manifest.json` as written, so that reformatting the file does not invalidate it.
pub fn verify_manifest_digest(
    root: &Path,
    manifest: &Manifest,
) -> Result<Option<ValidationWarning>, ReconcileError> {
    let path = layout::resolve(root, layout::MANIFEST_HASH)?;
    if !path.exists() {
        return Ok(Some(ValidationWarning::MissingManifestDigest));
    }

    let recorded = fs::read_to_string(&path).map_err(|source| ReconcileError::Filesystem {
        path: path.display().to_string(),
        source,
    })?;
    let recorded = recorded.split_whitespace().next().unwrap_or_default();

    let (_, computed) = manifest.to_canonical_bytes_and_hash()?;
    if recorded != computed {
        return Err(ReconcileError::HashMismatch {
            path: layout::MANIFEST.to_owned(),
        });
    }
    Ok(None)
}

/// Verifies every file listed in the manifest against the bundle on disk.
pub fn validate_bundle(root: &Path, manifest: &Manifest) -> ValidationReport {
    let mut report = ValidationReport::default();

    match verify_manifest_digest(root, manifest) {
        Ok(Some(warning)) => report.warnings.push(warning),
        Ok(None) => {}
        Err(error) => report.failures.push(error),
    }

    for entry in &manifest.files {
        let path = match layout::resolve(root, &entry.path) {
            Ok(path) => path,
            Err(error) => {
                report.failures.push(error);
                continue;
            }
        };

        if !path.is_file() {
            report.failures.push(ReconcileError::MissingFile {
                path: entry.path.clone(),
            });
            continue;
        }

        match hashing::file_size(&path) {
            Ok(size) if size != entry.size_bytes => {
                report.failures.push(ReconcileError::HashMismatch {
                    path: entry.path.clone(),
                });
                continue;
            }
            Ok(_) => {}
            Err(error) => {
                report.failures.push(error);
                continue;
            }
        }

        match hashing::sha256_file(&path) {
            Ok(digest) if digest == entry.sha256 => {}
            Ok(_) => report.failures.push(ReconcileError::HashMismatch {
                path: entry.path.clone(),
            }),
            Err(error) => report.failures.push(error),
        }
    }

    match collect_unlisted(root, manifest) {
        Ok(unlisted) => report.warnings.extend(
            unlisted
                .into_iter()
                .map(|path| ValidationWarning::UnlistedFile { path }),
        ),
        Err(error) => report.failures.push(error),
    }

    report
}

/// Finds files present in the bundle that the manifest does not list.
fn collect_unlisted(root: &Path, manifest: &Manifest) -> Result<Vec<String>, ReconcileError> {
    let listed: BTreeSet<&str> = manifest
        .files
        .iter()
        .map(|entry| entry.path.as_str())
        .collect();

    let mut present = Vec::new();
    walk(root, root, &mut present)?;
    present.sort();

    Ok(present
        .into_iter()
        .filter(|path| {
            path != layout::MANIFEST
                && !listed.contains(path.as_str())
                && !layout::GENERATED_PATHS.contains(&path.as_str())
        })
        .collect())
}

/// Recursively lists bundle-relative file paths.
///
/// Symbolic links are reported as ordinary entries and never traversed, so a link cannot
/// cause the walk to leave the bundle or loop.
fn walk(root: &Path, directory: &Path, found: &mut Vec<String>) -> Result<(), ReconcileError> {
    let entries = fs::read_dir(directory).map_err(|source| ReconcileError::Filesystem {
        path: directory.display().to_string(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| ReconcileError::Filesystem {
            path: directory.display().to_string(),
            source,
        })?;
        let path = entry.path();

        let metadata =
            fs::symlink_metadata(&path).map_err(|source| ReconcileError::Filesystem {
                path: path.display().to_string(),
                source,
            })?;

        if metadata.is_dir() {
            walk(root, &path, found)?;
        } else {
            found.push(layout::to_bundle_path(root, &path)?);
        }
    }
    Ok(())
}

/// Convenience digest of arbitrary bytes, used when writing bundles.
pub fn digest_bytes(bytes: &[u8]) -> String {
    canonical::sha256_hex(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::height::{BlockHeight, HeightInterval};
    use crate::domain::network::Network;
    use crate::domain::zatoshi::Zatoshi;
    use crate::evidence::manifest::{
        Activation, AnchorState, Encoding, EndState, EndStateTracking, FileEntry, Rfc3339Timestamp,
        SCHEMA_VERSION, Source, Tool,
    };
    use std::io::Write;
    use std::path::PathBuf;

    struct Bundle {
        _dir: tempfile::TempDir,
        root: PathBuf,
        manifest: Manifest,
    }

    fn write_file(root: &Path, relative: &str, contents: &[u8]) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(contents).unwrap();
        file.sync_all().unwrap();
    }

    /// Builds a small but structurally complete bundle on disk.
    fn bundle() -> Bundle {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let interval =
            HeightInterval::new(BlockHeight::new(3_428_143), BlockHeight::new(3_428_144)).unwrap();

        let mut manifest = Manifest {
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
                reported_orchard_balance_zatoshis: Zatoshi::from_raw(366_000_000_000_000),
                reported_ironwood_balance_zatoshis: Zatoshi::ZERO,
                tracking: EndStateTracking::default(),
            },
            files: Vec::new(),
        };

        for (relative, contents, encoding) in [
            (
                layout::ANCHOR_BLOCK,
                b"00aabb".as_slice(),
                Encoding::RawBlockHex,
            ),
            (layout::ANCHOR_VALUE_POOLS, b"{}".as_slice(), Encoding::Json),
            (
                "blocks/3428143.hex",
                b"0011".as_slice(),
                Encoding::RawBlockHex,
            ),
            (
                "blocks/3428144.hex",
                b"2233".as_slice(),
                Encoding::RawBlockHex,
            ),
        ] {
            write_file(&root, relative, contents);
            manifest.add_file(FileEntry {
                path: relative.to_owned(),
                sha256: hashing::sha256_hex(contents),
                size_bytes: contents.len() as u64,
                encoding,
            });
        }

        let manifest_json = serde_json::to_vec(&manifest).unwrap();
        write_file(&root, layout::MANIFEST, &manifest_json);
        let (_, digest) = manifest.to_canonical_bytes_and_hash().unwrap();
        write_file(&root, layout::MANIFEST_HASH, digest.as_bytes());

        Bundle {
            _dir: dir,
            root,
            manifest,
        }
    }

    #[test]
    fn a_well_formed_bundle_validates_clean() {
        let bundle = bundle();
        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert!(report.is_valid(), "failures: {:?}", report.failures);
        assert!(
            report.warnings.is_empty(),
            "warnings: {:?}",
            report.warnings
        );
    }

    #[test]
    fn the_manifest_can_be_loaded_from_disk() {
        let bundle = bundle();
        let loaded = load_manifest(&bundle.root).unwrap();
        assert_eq!(loaded, bundle.manifest);
    }

    #[test]
    fn mutating_one_evidence_byte_fails_and_names_the_path() {
        let bundle = bundle();
        write_file(&bundle.root, "blocks/3428143.hex", b"00ff");

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert!(!report.is_valid());
        assert!(
            report.failures.iter().any(|failure| matches!(
                failure,
                ReconcileError::HashMismatch { path } if path == "blocks/3428143.hex"
            )),
            "expected a hash mismatch naming the file, got {:?}",
            report.failures
        );
    }

    #[test]
    fn removing_a_listed_file_fails_as_missing_rather_than_passing() {
        let bundle = bundle();
        fs::remove_file(bundle.root.join("blocks/3428144.hex")).unwrap();

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert!(!report.is_valid());
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            ReconcileError::MissingFile { path } if path == "blocks/3428144.hex"
        )));
    }

    #[test]
    fn an_unlisted_file_warns_but_does_not_fail() {
        let bundle = bundle();
        write_file(&bundle.root, "blocks/9999999.hex", b"dead");

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert!(report.is_valid(), "failures: {:?}", report.failures);
        assert_eq!(
            report.warnings,
            vec![ValidationWarning::UnlistedFile {
                path: "blocks/9999999.hex".to_owned()
            }]
        );
    }

    #[test]
    fn a_truncated_file_of_the_same_name_fails_on_size() {
        let bundle = bundle();
        write_file(&bundle.root, "blocks/3428143.hex", b"0");

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert!(!report.is_valid());
    }

    #[test]
    fn a_tampered_manifest_digest_fails() {
        let bundle = bundle();
        write_file(&bundle.root, layout::MANIFEST_HASH, &[b'0'; 64]);

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert!(!report.is_valid());
        assert!(report.failures.iter().any(|failure| matches!(
            failure,
            ReconcileError::HashMismatch { path } if path == layout::MANIFEST
        )));
    }

    #[test]
    fn an_absent_manifest_digest_warns_but_does_not_fail() {
        let bundle = bundle();
        fs::remove_file(bundle.root.join(layout::MANIFEST_HASH)).unwrap();

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert!(report.is_valid());
        assert!(
            report
                .warnings
                .contains(&ValidationWarning::MissingManifestDigest)
        );
    }

    #[test]
    fn generated_report_files_do_not_warn_as_unlisted() {
        let bundle = bundle();
        write_file(&bundle.root, layout::REPORT_JSON, b"{}");
        write_file(&bundle.root, layout::REPORT_MARKDOWN, b"# report");

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert!(report.is_valid());
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn every_damaged_file_is_reported_not_only_the_first() {
        let bundle = bundle();
        write_file(&bundle.root, "blocks/3428143.hex", b"ffff");
        write_file(&bundle.root, "blocks/3428144.hex", b"eeee");

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert_eq!(report.failures.len(), 2, "{:?}", report.failures);
    }

    #[test]
    fn into_result_surfaces_a_failure() {
        let bundle = bundle();
        fs::remove_file(bundle.root.join("blocks/3428143.hex")).unwrap();

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        assert!(report.into_result().is_err());
    }

    #[test]
    fn into_result_yields_warnings_when_valid() {
        let bundle = bundle();
        write_file(&bundle.root, "stray.txt", b"x");

        let report = validate_bundle(&bundle.root, &bundle.manifest);
        let warnings = report.into_result().unwrap();
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn a_manifest_whose_structure_is_invalid_is_rejected_before_reading_files() {
        let bundle = bundle();
        let mut broken = bundle.manifest.clone();
        broken.schema_version = "99.0.0".to_owned();
        let bytes = serde_json::to_vec(&broken).unwrap();
        write_file(&bundle.root, layout::MANIFEST, &bytes);

        assert!(matches!(
            load_manifest(&bundle.root),
            Err(ReconcileError::ManifestInvalid { .. })
        ));
    }
}
