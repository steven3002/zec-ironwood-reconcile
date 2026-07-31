//! `inspect` — bundle metadata without reconciliation.
//!
//! Deliberately cheap. It reads the manifest and reports what the bundle claims to be. It
//! does not hash evidence files, parse blocks, or reconcile, so it stays fast on a large
//! bundle and can be used to triage an archive before committing to verifying it.
//!
//! Because it performs no verification, its output describes claims rather than findings.

use std::path::Path;

use crate::error::ReconcileError;
use crate::evidence::layout;
use crate::evidence::manifest::Manifest;
use crate::evidence::validation;

/// Everything `inspect` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleSummary {
    pub schema_version: String,
    pub bundle_id: String,
    pub network: String,
    pub anchor_height: u32,
    pub start_height: u32,
    pub end_height: u32,
    pub block_count: u32,
    pub node_implementation: String,
    pub node_version: String,
    pub tool_version: String,
    pub manifest_hash: String,
    pub file_count: usize,
    pub total_size_bytes: u64,
    pub report_included: bool,
    /// Whether every file the manifest lists is present. File contents are not hashed.
    pub evidence_complete: bool,
}

/// Reads a bundle's manifest and summarises it.
pub fn inspect(bundle_root: &Path) -> Result<BundleSummary, ReconcileError> {
    let manifest = validation::load_manifest(bundle_root)?;
    summarise(bundle_root, &manifest)
}

fn summarise(bundle_root: &Path, manifest: &Manifest) -> Result<BundleSummary, ReconcileError> {
    let (_, manifest_hash) = manifest.to_canonical_bytes_and_hash()?;

    let report_included = layout::resolve(bundle_root, layout::REPORT_JSON)?.is_file();

    let evidence_complete = manifest.files.iter().try_fold(true, |complete, entry| {
        let path = layout::resolve(bundle_root, &entry.path)?;
        Ok::<bool, ReconcileError>(complete && path.is_file())
    })?;

    Ok(BundleSummary {
        schema_version: manifest.schema_version.clone(),
        bundle_id: manifest.bundle_id.clone(),
        network: manifest.network.name().to_owned(),
        anchor_height: manifest.interval.anchor_height.get(),
        start_height: manifest.interval.start_height.get(),
        end_height: manifest.interval.end_height.get(),
        block_count: manifest.interval.block_count,
        node_implementation: manifest.source.implementation.clone(),
        node_version: manifest.source.version.clone(),
        tool_version: manifest.tool.version.clone(),
        manifest_hash,
        file_count: manifest.files.len(),
        total_size_bytes: manifest.total_size_bytes()?,
        report_included,
        evidence_complete,
    })
}

/// Renders a summary for a terminal.
pub fn render(summary: &BundleSummary) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    let mut line = |label: &str, value: &str| {
        let _ = writeln!(out, "{label:<22} {value}");
    };

    line("Bundle version:", &summary.schema_version);
    line("Bundle id:", &summary.bundle_id);
    line("Network:", &summary.network);
    line("Anchor height:", &summary.anchor_height.to_string());
    line("Start height:", &summary.start_height.to_string());
    line("End height:", &summary.end_height.to_string());
    line("Block count:", &summary.block_count.to_string());
    line("Node implementation:", &summary.node_implementation);
    line("Node version:", &summary.node_version);
    line("Tool version:", &summary.tool_version);
    line("Manifest hash:", &summary.manifest_hash);
    line("Files listed:", &summary.file_count.to_string());
    line("Total size (bytes):", &summary.total_size_bytes.to_string());
    line(
        "Report included:",
        if summary.report_included { "yes" } else { "no" },
    );
    line(
        "Evidence complete:",
        if summary.evidence_complete {
            "yes"
        } else {
            "no (files missing)"
        },
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::height::{BlockHeight, HeightInterval};
    use crate::domain::network::Network;
    use crate::domain::zatoshi::Zatoshi;
    use crate::evidence::hashing;
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

    fn bundle() -> (tempfile::TempDir, PathBuf) {
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
        ] {
            write_file(&root, relative, contents);
            manifest.add_file(FileEntry {
                path: relative.to_owned(),
                sha256: hashing::sha256_hex(contents),
                size_bytes: contents.len() as u64,
                encoding: Encoding::RawBlockHex,
            });
        }

        write_file(
            &root,
            layout::MANIFEST,
            &serde_json::to_vec(&manifest).unwrap(),
        );
        (dir, root)
    }

    #[test]
    fn a_bundle_is_summarised_from_its_manifest() {
        let (_dir, root) = bundle();
        let summary = inspect(&root).unwrap();

        assert_eq!(summary.bundle_id, "mainnet-3428142-3428144");
        assert_eq!(summary.network, "mainnet");
        assert_eq!(summary.anchor_height, 3_428_142);
        assert_eq!(summary.start_height, 3_428_143);
        assert_eq!(summary.end_height, 3_428_144);
        assert_eq!(summary.block_count, 2);
        assert_eq!(summary.node_implementation, "zebra");
        assert_eq!(summary.file_count, 2);
        assert_eq!(summary.total_size_bytes, 8);
        assert!(summary.evidence_complete);
        assert!(!summary.report_included);
    }

    #[test]
    fn a_missing_evidence_file_is_reported_as_incomplete() {
        let (_dir, root) = bundle();
        std::fs::remove_file(root.join("blocks/3428144.hex")).unwrap();

        let summary = inspect(&root).unwrap();
        assert!(!summary.evidence_complete);
    }

    #[test]
    fn a_present_report_is_noticed() {
        let (_dir, root) = bundle();
        write_file(&root, layout::REPORT_JSON, b"{}");

        assert!(inspect(&root).unwrap().report_included);
    }

    #[test]
    fn inspection_does_not_detect_altered_contents() {
        // Documents the boundary: inspect reports claims, not findings. Altering a file
        // without touching the manifest is invisible here and is caught by `verify`.
        let (_dir, root) = bundle();
        write_file(&root, "blocks/3428143.hex", b"ffff");

        let summary = inspect(&root).unwrap();
        assert!(summary.evidence_complete);
    }

    #[test]
    fn an_invalid_manifest_is_rejected() {
        let (_dir, root) = bundle();
        write_file(&root, layout::MANIFEST, b"{ not json");
        assert!(matches!(
            inspect(&root),
            Err(ReconcileError::ManifestInvalid { .. })
        ));
    }

    #[test]
    fn the_rendering_contains_every_summarised_field() {
        let (_dir, root) = bundle();
        let summary = inspect(&root).unwrap();
        let text = render(&summary);

        for expected in [
            &summary.bundle_id,
            &summary.network,
            &summary.manifest_hash,
            &summary.node_version,
        ] {
            assert!(text.contains(expected), "rendering omits {expected}");
        }
        assert!(text.contains("3428142"));
        assert!(text.contains("Evidence complete:"));
    }

    #[test]
    fn the_manifest_hash_is_the_canonical_digest() {
        let (_dir, root) = bundle();
        let manifest = validation::load_manifest(&root).unwrap();
        let (_, expected) = manifest.to_canonical_bytes_and_hash().unwrap();

        assert_eq!(inspect(&root).unwrap().manifest_hash, expected);
    }
}
