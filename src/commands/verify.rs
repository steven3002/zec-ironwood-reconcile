//! `verify` — offline reproduction of a published result.
//!
//! This is the command a third party actually runs, and the one the project's credibility
//! rests on. It requires no node, no internet access, no database, no hosted service, no
//! wallet, and nothing belonging to whoever produced the evidence.
//!
//! That property is structural rather than maintained: verification runs the same pure
//! pipeline as `reconcile`, and the modules that pipeline depends on cannot reach the
//! network because they do not import anything that can.
//!
//! Verification proceeds in stages, each of which must pass before the next is attempted.
//! Extraction comes first because the archive is untrusted; manifest structure comes before
//! any file is opened, because the file list is part of that untrusted input.

use std::path::Path;

use crate::error::ReconcileError;
use crate::evidence::archive::{self, ExtractionLimits};
use crate::evidence::manifest::Manifest;
use crate::evidence::validation::{self, ValidationWarning};

/// How far verification progressed, and what it found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationOutcome {
    pub bundle_id: String,
    pub entries_extracted: u32,
    pub bytes_extracted: u64,
    pub files_verified: usize,
    pub warnings: Vec<ValidationWarning>,
    /// Hash of the canonical report, once reconciliation has been performed.
    pub observed_report_hash: Option<String>,
    pub expected_report_hash: Option<String>,
}

impl VerificationOutcome {
    /// Whether the observed report hash matched the expected one.
    ///
    /// `None` when no expectation was supplied or no report was produced. A caller must not
    /// treat `None` as success.
    pub fn hash_matches(&self) -> Option<bool> {
        match (&self.observed_report_hash, &self.expected_report_hash) {
            (Some(observed), Some(expected)) => Some(observed == expected),
            _ => None,
        }
    }
}

/// Extracts an archive and verifies its evidence against its manifest.
///
/// Returns the extracted bundle root alongside the outcome so a caller can proceed to
/// reconciliation over the same files.
pub fn verify_archive(
    archive_path: &Path,
    destination: &Path,
    expected_report_hash: Option<&str>,
    limits: &ExtractionLimits,
) -> Result<(Manifest, VerificationOutcome), ReconcileError> {
    let extraction = archive::extract(archive_path, destination, limits)?;

    let manifest = validation::load_manifest(destination)?;

    let report = validation::validate_bundle(destination, &manifest);
    let files_verified = manifest.files.len();
    let warnings = report.into_result()?;

    Ok((
        manifest.clone(),
        VerificationOutcome {
            bundle_id: manifest.bundle_id,
            entries_extracted: extraction.entries,
            bytes_extracted: extraction.total_bytes,
            files_verified,
            warnings,
            observed_report_hash: None,
            expected_report_hash: expected_report_hash.map(str::to_owned),
        },
    ))
}

/// Renders a concise verdict.
pub fn render(outcome: &VerificationOutcome) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let _ = writeln!(out, "Bundle:            {}", outcome.bundle_id);
    let _ = writeln!(out, "Entries extracted: {}", outcome.entries_extracted);
    let _ = writeln!(out, "Bytes extracted:   {}", outcome.bytes_extracted);
    let _ = writeln!(out, "Files verified:    {}", outcome.files_verified);

    match &outcome.observed_report_hash {
        Some(hash) => {
            let _ = writeln!(out, "Report hash:       {hash}");
        }
        None => {
            let _ = writeln!(out, "Report hash:       not computed");
        }
    }

    match outcome.hash_matches() {
        Some(true) => {
            let _ = writeln!(out, "Result:            MATCH");
        }
        Some(false) => {
            let _ = writeln!(out, "Result:            MISMATCH");
            if let Some(expected) = &outcome.expected_report_hash {
                let _ = writeln!(out, "Expected:          {expected}");
            }
        }
        None => {
            let _ = writeln!(
                out,
                "Result:            evidence verified; no report hash comparison was made"
            );
        }
    }

    if !outcome.warnings.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Warnings:");
        for warning in &outcome.warnings {
            let _ = writeln!(out, "  - {warning}");
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::height::{BlockHeight, HeightInterval};
    use crate::domain::network::Network;
    use crate::domain::zatoshi::Zatoshi;
    use crate::evidence::hashing;
    use crate::evidence::layout;
    use crate::evidence::manifest::{
        Activation, AnchorState, Encoding, EndState, EndStateTracking, FileEntry, Rfc3339Timestamp,
        SCHEMA_VERSION, Source, Tool,
    };
    use std::io::Write;
    use std::path::PathBuf;

    fn write_file(root: &Path, relative: &str, contents: &[u8]) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents).unwrap();
        file.sync_all().unwrap();
    }

    /// Builds a bundle directory and packs it, returning the archive path.
    fn packed_bundle() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();

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
                orchard_balance_zatoshis: Zatoshi::ZERO,
                ironwood_balance_zatoshis: Zatoshi::ZERO,
            },
            end: EndState {
                block_hash: "1".repeat(64),
                reported_orchard_balance_zatoshis: Zatoshi::ZERO,
                reported_ironwood_balance_zatoshis: Zatoshi::ZERO,
                tracking: EndStateTracking::default(),
            },
            files: Vec::new(),
        };

        for (relative, contents) in [
            ("blocks/3428143.hex", b"0011".as_slice()),
            ("blocks/3428144.hex", b"2233".as_slice()),
            ("metadata/command.txt", b"capture".as_slice()),
        ] {
            write_file(&bundle, relative, contents);
            manifest.add_file(FileEntry {
                path: relative.to_owned(),
                sha256: hashing::sha256_hex(contents),
                size_bytes: contents.len() as u64,
                encoding: Encoding::RawBlockHex,
            });
        }

        write_file(
            &bundle,
            layout::MANIFEST,
            &serde_json::to_vec(&manifest).unwrap(),
        );
        let (_, digest) = manifest.to_canonical_bytes_and_hash().unwrap();
        write_file(&bundle, layout::MANIFEST_HASH, digest.as_bytes());

        let archive = dir.path().join("bundle.tar.zst");
        archive::pack(&bundle, &archive).unwrap();
        (dir, archive)
    }

    #[test]
    fn a_valid_archive_verifies() {
        let (dir, archive) = packed_bundle();
        let destination = dir.path().join("extracted");
        std::fs::create_dir_all(&destination).unwrap();

        let (_, outcome) =
            verify_archive(&archive, &destination, None, &ExtractionLimits::default()).unwrap();

        assert_eq!(outcome.bundle_id, "mainnet-3428142-3428144");
        assert_eq!(outcome.files_verified, 3);
        assert_eq!(outcome.entries_extracted, 5);
        assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
    }

    #[test]
    fn a_modified_archive_fails_verification() {
        let (dir, archive) = packed_bundle();

        // Rebuild the bundle with one byte changed and repack it.
        let tampered_bundle = dir.path().join("tampered");
        std::fs::create_dir_all(&tampered_bundle).unwrap();
        let extracted = dir.path().join("staging");
        std::fs::create_dir_all(&extracted).unwrap();
        archive::extract(&archive, &extracted, &ExtractionLimits::default()).unwrap();

        for relative in [
            layout::MANIFEST,
            layout::MANIFEST_HASH,
            "blocks/3428143.hex",
            "blocks/3428144.hex",
            "metadata/command.txt",
        ] {
            let contents = std::fs::read(extracted.join(relative)).unwrap();
            write_file(&tampered_bundle, relative, &contents);
        }
        write_file(&tampered_bundle, "blocks/3428143.hex", b"ffff");

        let tampered_archive = dir.path().join("tampered.tar.zst");
        archive::pack(&tampered_bundle, &tampered_archive).unwrap();

        let destination = dir.path().join("out");
        std::fs::create_dir_all(&destination).unwrap();

        assert!(matches!(
            verify_archive(
                &tampered_archive,
                &destination,
                None,
                &ExtractionLimits::default()
            ),
            Err(ReconcileError::HashMismatch { .. })
        ));
    }

    #[test]
    fn hash_comparison_reports_none_when_no_expectation_was_supplied() {
        let outcome = VerificationOutcome {
            bundle_id: "x".to_owned(),
            entries_extracted: 1,
            bytes_extracted: 1,
            files_verified: 1,
            warnings: Vec::new(),
            observed_report_hash: Some("abc".to_owned()),
            expected_report_hash: None,
        };
        assert_eq!(outcome.hash_matches(), None);
    }

    #[test]
    fn a_mismatched_expectation_is_reported_as_a_mismatch() {
        let outcome = VerificationOutcome {
            bundle_id: "x".to_owned(),
            entries_extracted: 1,
            bytes_extracted: 1,
            files_verified: 1,
            warnings: Vec::new(),
            observed_report_hash: Some("abc".to_owned()),
            expected_report_hash: Some("def".to_owned()),
        };
        assert_eq!(outcome.hash_matches(), Some(false));
        assert!(render(&outcome).contains("MISMATCH"));
    }

    #[test]
    fn a_matching_expectation_is_reported_as_a_match() {
        let outcome = VerificationOutcome {
            bundle_id: "x".to_owned(),
            entries_extracted: 1,
            bytes_extracted: 1,
            files_verified: 1,
            warnings: Vec::new(),
            observed_report_hash: Some("abc".to_owned()),
            expected_report_hash: Some("abc".to_owned()),
        };
        assert_eq!(outcome.hash_matches(), Some(true));
        assert!(render(&outcome).contains("MATCH"));
    }

    #[test]
    fn a_run_without_a_report_hash_does_not_render_as_a_match() {
        let (dir, archive) = packed_bundle();
        let destination = dir.path().join("extracted");
        std::fs::create_dir_all(&destination).unwrap();

        let (_, outcome) =
            verify_archive(&archive, &destination, None, &ExtractionLimits::default()).unwrap();

        let text = render(&outcome);
        assert!(!text.contains("MATCH"), "{text}");
        assert!(text.contains("no report hash comparison"), "{text}");
    }
}
