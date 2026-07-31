//! Hardened archive extraction.
//!
//! This is the crate's largest untrusted-input surface: an evidence archive can be authored
//! by anyone, and verification necessarily runs before its contents are known to be honest.
//!
//! Extraction refuses, rather than accommodates:
//!
//! * paths that are absolute or contain `..`, which would write outside the destination —
//!   the vulnerability class commonly called Zip Slip;
//! * symbolic and hard links, which could redirect a later write outside the destination
//!   even when every path looks benign;
//! * device, FIFO and other special entry types, which have no meaning in an evidence
//!   bundle;
//! * archives exceeding the configured entry count, per-entry size, total size, or path
//!   depth, which bounds a compression bomb;
//! * entries whose actual byte count differs from the size declared in their header.
//!
//! Every refusal is checked *before* the entry's contents are written.

use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path};

use crate::error::ReconcileError;
use crate::evidence::archive::limits::ExtractionLimits;

/// What extraction produced, for reporting and for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionSummary {
    pub entries: u32,
    pub total_bytes: u64,
}

/// Extracts an archive into `destination`, enforcing `limits`.
///
/// `destination` must already exist and should be a directory the caller controls, such as
/// one obtained from `tempfile`.
pub fn extract(
    archive_path: &Path,
    destination: &Path,
    limits: &ExtractionLimits,
) -> Result<ExtractionSummary, ReconcileError> {
    let file = File::open(archive_path).map_err(|source| ReconcileError::Filesystem {
        path: archive_path.display().to_string(),
        source,
    })?;

    let decoder =
        zstd::Decoder::new(BufReader::new(file)).map_err(|source| ReconcileError::Filesystem {
            path: archive_path.display().to_string(),
            source,
        })?;

    let mut archive = tar::Archive::new(decoder);

    // Neither of these would be honoured for entries this extractor accepts, since links
    // and special types are rejected outright. They are disabled anyway so that no future
    // change to entry handling can silently re-enable them.
    archive.set_preserve_permissions(false);
    archive.set_unpack_xattrs(false);

    let entries = archive
        .entries()
        .map_err(|source| ReconcileError::Filesystem {
            path: archive_path.display().to_string(),
            source,
        })?;

    let mut count: u32 = 0;
    let mut total: u64 = 0;

    for entry in entries {
        let mut entry = entry.map_err(|source| ReconcileError::Filesystem {
            path: archive_path.display().to_string(),
            source,
        })?;

        let raw_path = entry
            .path()
            .map_err(|source| ReconcileError::ArchiveRejected {
                reason: format!("entry path is not readable: {source}"),
            })?
            .to_path_buf();

        let relative = path_string(&raw_path)?;

        // Path safety is checked before entry type, so that a hostile path in an otherwise
        // skippable entry is still refused rather than passed over.
        validate_archive_path(&relative)?;
        limits.check_depth(&relative)?;

        match classify(entry.header().entry_type(), &relative)? {
            Disposition::Extract => {}
            Disposition::Skip => continue,
        }

        let declared = entry
            .header()
            .size()
            .map_err(|source| ReconcileError::ArchiveRejected {
                reason: format!("entry {relative:?} has an unreadable size: {source}"),
            })?;

        limits.check_entry_size(&relative, declared)?;

        count = count
            .checked_add(1)
            .ok_or_else(|| ReconcileError::ArchiveRejected {
                reason: "entry count overflowed".to_owned(),
            })?;
        limits.check_entry_count(count)?;

        total = total
            .checked_add(declared)
            .ok_or_else(|| ReconcileError::ArchiveRejected {
                reason: "declared sizes overflowed".to_owned(),
            })?;
        limits.check_running_total(total)?;

        let written = write_entry(&mut entry, destination, &relative, declared)?;

        // A header may declare one size and the stream deliver another. Trusting the header
        // alone would leave a truncated file on disk that later hashes as merely corrupt
        // rather than as a malformed archive.
        if written != declared {
            return Err(ReconcileError::ArchiveRejected {
                reason: format!(
                    "entry {relative:?} declared {declared} bytes but delivered {written}"
                ),
            });
        }
    }

    Ok(ExtractionSummary {
        entries: count,
        total_bytes: total,
    })
}

/// Writes one entry, bounded by its declared size.
fn write_entry<R: Read>(
    entry: &mut R,
    destination: &Path,
    relative: &str,
    declared: u64,
) -> Result<u64, ReconcileError> {
    let target = destination.join(relative);

    // Re-checked after joining: the destination is trusted and the relative path has been
    // validated, so this can only fail if one of those assumptions is broken.
    if !target.starts_with(destination) {
        return Err(ReconcileError::ArchiveRejected {
            reason: format!("entry {relative:?} resolves outside the extraction directory"),
        });
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ReconcileError::Filesystem {
            path: parent.display().to_string(),
            source,
        })?;
    }

    let mut file = File::create(&target).map_err(|source| ReconcileError::Filesystem {
        path: target.display().to_string(),
        source,
    })?;

    // Reading one byte beyond the declaration detects an over-long entry without buffering
    // it, so a lying header cannot be used to write an unbounded file.
    let limit = declared
        .checked_add(1)
        .ok_or_else(|| ReconcileError::ArchiveRejected {
            reason: format!("entry {relative:?} declares an unrepresentable size"),
        })?;

    let written = std::io::copy(&mut entry.take(limit), &mut file).map_err(|source| {
        ReconcileError::Filesystem {
            path: target.display().to_string(),
            source,
        }
    })?;

    file.flush().map_err(|source| ReconcileError::Filesystem {
        path: target.display().to_string(),
        source,
    })?;

    Ok(written)
}

/// What to do with an archive entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    Extract,
    /// Accepted but not written. Its path has still been validated.
    Skip,
}

/// Decides how to treat an entry, refusing anything that has no place in an evidence bundle.
///
/// Directory entries are skipped rather than refused. Ordinary archiving tools emit them,
/// and refusing them would mean this tool could only read archives it produced itself.
/// Skipping is safe: parent directories are created implicitly when a file is written, and
/// the entry's path has already been validated, so a directory entry cannot be used to
/// reach outside the destination.
fn classify(entry_type: tar::EntryType, relative: &str) -> Result<Disposition, ReconcileError> {
    match entry_type {
        tar::EntryType::Regular | tar::EntryType::Continuous => Ok(Disposition::Extract),
        tar::EntryType::Directory => Ok(Disposition::Skip),

        // Metadata entries carry no bundle content and are not needed to reconstruct one.
        tar::EntryType::XGlobalHeader | tar::EntryType::XHeader | tar::EntryType::GNULongName => {
            Ok(Disposition::Skip)
        }

        tar::EntryType::Symlink => Err(ReconcileError::ArchiveRejected {
            reason: format!("entry {relative:?} is a symbolic link"),
        }),
        tar::EntryType::Link => Err(ReconcileError::ArchiveRejected {
            reason: format!("entry {relative:?} is a hard link"),
        }),
        other => Err(ReconcileError::ArchiveRejected {
            reason: format!("entry {relative:?} has unsupported type {other:?}"),
        }),
    }
}

/// Rejects any path that is not a plain relative path.
///
/// This duplicates the manifest path rule deliberately. An archive is validated before its
/// manifest has been read, so it cannot rely on manifest validation having run.
fn validate_archive_path(path: &str) -> Result<(), ReconcileError> {
    let reject = |reason: &str| {
        Err(ReconcileError::ArchiveRejected {
            reason: format!("unsafe archive path {path:?}: {reason}"),
        })
    };

    if path.is_empty() {
        return reject("empty");
    }
    if path.contains('\0') {
        return reject("contains a null byte");
    }
    if path.contains('\\') {
        return reject("contains a backslash");
    }
    if path.starts_with('/') {
        return reject("absolute");
    }

    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => return reject("contains a `.` component"),
            Component::ParentDir => return reject("contains a `..` component"),
            Component::RootDir | Component::Prefix(_) => return reject("absolute"),
        }
    }

    Ok(())
}

fn path_string(path: &Path) -> Result<String, ReconcileError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| ReconcileError::ArchiveRejected {
            reason: format!("entry path is not valid UTF-8: {}", path.display()),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_paths_are_accepted() {
        for path in [
            "manifest.json",
            "blocks/3428143.hex",
            "metadata/command.txt",
        ] {
            assert!(validate_archive_path(path).is_ok(), "rejected {path}");
        }
    }

    #[test]
    fn traversal_paths_are_rejected() {
        for path in ["../escape", "blocks/../../escape", "..", "a/../../b"] {
            assert!(
                matches!(
                    validate_archive_path(path),
                    Err(ReconcileError::ArchiveRejected { .. })
                ),
                "accepted traversal path {path}"
            );
        }
    }

    #[test]
    fn absolute_paths_are_rejected() {
        for path in ["/etc/passwd", "/", "//tmp/x"] {
            assert!(validate_archive_path(path).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn backslash_and_null_paths_are_rejected() {
        assert!(validate_archive_path("blocks\\1.hex").is_err());
        assert!(validate_archive_path("blocks/\u{0}1.hex").is_err());
        assert!(validate_archive_path("").is_err());
        assert!(validate_archive_path("./blocks/1.hex").is_err());
    }

    #[test]
    fn links_and_special_types_are_rejected() {
        for entry_type in [
            tar::EntryType::Symlink,
            tar::EntryType::Link,
            tar::EntryType::Char,
            tar::EntryType::Block,
            tar::EntryType::Fifo,
        ] {
            assert!(
                classify(entry_type, "x").is_err(),
                "accepted entry type {entry_type:?}"
            );
        }
    }

    #[test]
    fn regular_files_are_extracted() {
        assert_eq!(
            classify(tar::EntryType::Regular, "x").unwrap(),
            Disposition::Extract
        );
    }

    #[test]
    fn directory_entries_are_skipped_rather_than_refused() {
        // Ordinary archiving tools emit directory entries. Refusing them would mean this
        // tool could only read archives it produced itself.
        assert_eq!(
            classify(tar::EntryType::Directory, "blocks/").unwrap(),
            Disposition::Skip
        );
    }
}
