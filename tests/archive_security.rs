//! Rejection of malicious archives.
//!
//! An evidence archive is the one input this tool accepts from an arbitrary third party,
//! and verification necessarily begins before its contents are known to be honest. Each
//! test here constructs a real archive exercising one attack and asserts it is refused.
//!
//! Archives are built rather than committed as binary fixtures so that what each one
//! contains is visible in the test that uses it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use std::io::Write;
use std::path::{Path, PathBuf};

use zec_ironwood_reconcile::error::ReconcileError;
use zec_ironwood_reconcile::evidence::archive::{self, ExtractionLimits};

/// Deliberately tight limits so bomb tests stay fast.
fn strict_limits() -> ExtractionLimits {
    ExtractionLimits {
        max_total_bytes: 64 * 1024,
        max_entries: 8,
        max_entry_bytes: 16 * 1024,
        max_path_depth: 4,
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    archive: PathBuf,
    destination: PathBuf,
}

impl Fixture {
    fn extract_with(
        &self,
        limits: &ExtractionLimits,
    ) -> Result<archive::ExtractionSummary, ReconcileError> {
        archive::extract(&self.archive, &self.destination, limits)
    }

    fn extract(&self) -> Result<archive::ExtractionSummary, ReconcileError> {
        self.extract_with(&strict_limits())
    }
}

/// Builds a `.tar.zst` archive from a closure that populates the tar builder.
fn build_archive<F>(populate: F) -> Fixture
where
    F: FnOnce(&mut tar::Builder<zstd::Encoder<'_, std::fs::File>>),
{
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("evidence.tar.zst");
    let destination = dir.path().join("extracted");
    std::fs::create_dir_all(&destination).unwrap();

    let file = std::fs::File::create(&archive).unwrap();
    let encoder = zstd::Encoder::new(file, 1).unwrap();
    let mut builder = tar::Builder::new(encoder);

    populate(&mut builder);

    let encoder = builder.into_inner().unwrap();
    encoder.finish().unwrap().sync_all().unwrap();

    Fixture {
        _dir: dir,
        archive,
        destination,
    }
}

fn regular_header(size: u64) -> tar::Header {
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(0o644);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_entry_type(tar::EntryType::Regular);
    header
}

/// Appends an entry whose path bypasses the tar crate's own path handling.
fn append_raw_path<W: Write>(builder: &mut tar::Builder<W>, path: &str, contents: &[u8]) {
    let mut header = regular_header(contents.len() as u64);
    // `set_path` normalises and may reject; writing the bytes directly is what a hostile
    // archive would do.
    let bytes = header.as_old_mut();
    let name = path.as_bytes();
    bytes.name[..name.len()].copy_from_slice(name);
    header.set_cksum();
    builder.append(&header, contents).unwrap();
}

fn assert_rejected(result: Result<archive::ExtractionSummary, ReconcileError>, attack: &str) {
    match result {
        Err(ReconcileError::ArchiveRejected { .. }) => {}
        Err(other) => panic!("{attack}: rejected, but not as an archive violation: {other:?}"),
        Ok(summary) => panic!("{attack}: ACCEPTED a malicious archive ({summary:?})"),
    }
}

/// Confirms nothing was written outside the extraction directory.
fn assert_nothing_escaped(destination: &Path, canary: &Path) {
    assert!(
        !canary.exists(),
        "a file was written outside the extraction directory: {}",
        canary.display()
    );
    if let Ok(entries) = std::fs::read_dir(destination) {
        for entry in entries.flatten() {
            let path = entry.path();
            assert!(
                path.starts_with(destination),
                "extracted path escaped: {}",
                path.display()
            );
        }
    }
}

#[test]
fn a_well_formed_archive_extracts() {
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "manifest.json", b"{}");
        append_raw_path(builder, "blocks/3428143.hex", b"0011");
    });

    let summary = fixture.extract().unwrap();
    assert_eq!(summary.entries, 2);
    assert_eq!(summary.total_bytes, 6);
    assert!(fixture.destination.join("blocks/3428143.hex").is_file());
}

#[test]
fn path_traversal_is_rejected() {
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "manifest.json", b"{}");
        append_raw_path(builder, "../escaped.txt", b"owned");
    });

    let canary = fixture.destination.parent().unwrap().join("escaped.txt");
    assert_rejected(fixture.extract(), "path traversal");
    assert_nothing_escaped(&fixture.destination, &canary);
}

#[test]
fn deep_path_traversal_is_rejected() {
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "blocks/../../../../etc/cron.d/payload", b"owned");
    });

    assert_rejected(fixture.extract(), "nested path traversal");
}

#[test]
fn an_absolute_path_is_rejected() {
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "/tmp/zec-ironwood-absolute-canary", b"owned");
    });

    let canary = PathBuf::from("/tmp/zec-ironwood-absolute-canary");
    assert_rejected(fixture.extract(), "absolute path");
    assert!(!canary.exists(), "an absolute path was written");
}

#[test]
fn a_symbolic_link_entry_is_rejected() {
    let fixture = build_archive(|builder| {
        let mut header = regular_header(0);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("/etc/passwd").unwrap();
        header.set_path("evidence-link").unwrap();
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
    });

    assert_rejected(fixture.extract(), "symbolic link");
    assert!(!fixture.destination.join("evidence-link").exists());
}

#[test]
fn a_hard_link_entry_is_rejected() {
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "manifest.json", b"{}");

        let mut header = regular_header(0);
        header.set_entry_type(tar::EntryType::Link);
        header.set_link_name("manifest.json").unwrap();
        header.set_path("alias.json").unwrap();
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
    });

    assert_rejected(fixture.extract(), "hard link");
}

#[test]
fn a_device_entry_is_rejected() {
    let fixture = build_archive(|builder| {
        let mut header = regular_header(0);
        header.set_entry_type(tar::EntryType::Char);
        header.set_path("console").unwrap();
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
    });

    assert_rejected(fixture.extract(), "character device");
}

#[test]
fn a_fifo_entry_is_rejected() {
    let fixture = build_archive(|builder| {
        let mut header = regular_header(0);
        header.set_entry_type(tar::EntryType::Fifo);
        header.set_path("pipe").unwrap();
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
    });

    assert_rejected(fixture.extract(), "FIFO");
}

#[test]
fn a_decompression_bomb_is_rejected_by_the_total_limit() {
    // Highly compressible content: 256 KiB of zeros occupies a few bytes compressed but
    // exceeds the 64 KiB total limit on extraction.
    let fixture = build_archive(|builder| {
        let payload = vec![0_u8; 256 * 1024];
        append_raw_path(builder, "bomb.bin", &payload);
    });

    assert_rejected(fixture.extract(), "decompression bomb");
}

#[test]
fn an_oversized_single_entry_is_rejected() {
    let fixture = build_archive(|builder| {
        let payload = vec![0_u8; 32 * 1024];
        append_raw_path(builder, "large.bin", &payload);
    });

    assert_rejected(fixture.extract(), "oversized entry");
}

#[test]
fn too_many_entries_are_rejected() {
    let fixture = build_archive(|builder| {
        for index in 0..20 {
            append_raw_path(builder, &format!("blocks/{index}.hex"), b"00");
        }
    });

    assert_rejected(fixture.extract(), "entry flood");
}

#[test]
fn excessive_path_depth_is_rejected() {
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "a/b/c/d/e/f/deep.hex", b"00");
    });

    assert_rejected(fixture.extract(), "excessive nesting");
}

#[test]
fn a_declared_size_larger_than_the_delivered_content_is_rejected() {
    // The header claims more bytes than the entry supplies. Trusting the header would leave
    // a truncated file that later reads as merely corrupt rather than as a hostile archive.
    let fixture = build_archive(|builder| {
        let mut header = regular_header(4_096);
        header.set_path("blocks/short.hex").unwrap();
        header.set_cksum();
        builder.append(&header, &b"0011"[..]).unwrap();
    });

    let result = fixture.extract();
    assert!(
        result.is_err(),
        "an archive whose header lies about its size was accepted"
    );
}

#[test]
fn a_backslash_path_is_rejected() {
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "blocks\\escape.hex", b"00");
    });

    assert_rejected(fixture.extract(), "backslash path");
}

#[test]
fn a_current_directory_path_component_is_rejected() {
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "./manifest.json", b"{}");
    });

    assert_rejected(fixture.extract(), "current-directory component");
}

#[test]
fn a_directory_entry_is_skipped_rather_than_refused() {
    // Ordinary archiving tools emit directory entries. Refusing them would mean this tool
    // could only read archives it produced itself.
    let fixture = build_archive(|builder| {
        let mut header = regular_header(0);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_path("blocks/").unwrap();
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();

        append_raw_path(builder, "blocks/3428143.hex", b"0011");
    });

    let summary = fixture.extract().unwrap();
    assert_eq!(
        summary.entries, 1,
        "the directory entry must not be counted"
    );
    assert!(fixture.destination.join("blocks/3428143.hex").is_file());
}

#[test]
fn a_directory_entry_with_a_traversing_path_is_still_rejected() {
    // Skipping directory entries must not become a way to smuggle a hostile path past
    // validation.
    let fixture = build_archive(|builder| {
        let mut header = regular_header(0);
        header.set_entry_type(tar::EntryType::Directory);
        header.set_cksum();
        let bytes = header.as_old_mut();
        let name = b"../escaped/";
        bytes.name[..name.len()].copy_from_slice(name);
        header.set_cksum();
        builder.append(&header, std::io::empty()).unwrap();
    });

    assert_rejected(fixture.extract(), "directory entry with traversal");
}

#[test]
fn a_generous_limit_still_rejects_a_traversal() {
    // Bounds and path safety are independent defences: relaxing the former must not weaken
    // the latter.
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "../escaped.txt", b"owned");
    });

    assert_rejected(
        fixture.extract_with(&ExtractionLimits::default()),
        "traversal under default limits",
    );
}

#[test]
fn rejection_happens_before_any_content_is_written() {
    // The hostile entry is first, so nothing legitimate should have been written either.
    let fixture = build_archive(|builder| {
        append_raw_path(builder, "../escaped.txt", b"owned");
        append_raw_path(builder, "manifest.json", b"{}");
    });

    assert_rejected(fixture.extract(), "traversal before valid entry");
    assert!(!fixture.destination.join("manifest.json").exists());
}
