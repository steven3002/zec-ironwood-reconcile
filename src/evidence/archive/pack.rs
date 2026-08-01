//! Archive creation.
//!
//! Packing is deterministic: entries are emitted in sorted path order and every piece of
//! filesystem metadata that varies between machines — modification time, ownership,
//! permission bits beyond a fixed mode, user and group names — is normalised away.
//!
//! Two operators who pack the same bundle therefore produce byte-identical archives with
//! identical digests. That is not required for verification, which works from the manifest,
//! but it removes an otherwise confusing source of divergence when two people publish what
//! should be the same artifact.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use crate::error::ReconcileError;
use crate::evidence::layout;

/// Compression level. Chosen for a useful ratio at a modest cost; the value is part of what
/// makes packing reproducible and should not be varied per run.
const ZSTD_LEVEL: i32 = 9;

/// Fixed mode applied to every archived file.
const FILE_MODE: u32 = 0o644;

/// Packs a bundle and writes the detached digest a third party checks it against.
///
/// The digest file uses the two-space layout `sha256sum -c` expects, so verifying the
/// download requires no tool from this project.
pub fn pack_with_digest(bundle_root: &Path, archive_path: &Path) -> Result<String, ReconcileError> {
    pack(bundle_root, archive_path)?;

    let digest = crate::evidence::hashing::sha256_file(archive_path)?;
    let name = archive_path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| ReconcileError::InvalidInput {
            reason: format!("{} has no usable file name", archive_path.display()),
        })?;

    // Appended rather than set as an extension: `with_extension` replaces the last one, so
    // an archive named without an extension would become `bundle..sha256`.
    let mut digest_path = archive_path.as_os_str().to_owned();
    digest_path.push(".sha256");
    let digest_path = PathBuf::from(digest_path);

    std::fs::write(&digest_path, format!("{digest}  {name}\n")).map_err(|source| {
        ReconcileError::Filesystem {
            path: digest_path.display().to_string(),
            source,
        }
    })?;

    Ok(digest)
}

/// Packs a bundle directory into a `.tar.zst` archive.
pub fn pack(bundle_root: &Path, archive_path: &Path) -> Result<(), ReconcileError> {
    let paths = collect_sorted_paths(bundle_root)?;

    let file = File::create(archive_path).map_err(|source| ReconcileError::Filesystem {
        path: archive_path.display().to_string(),
        source,
    })?;

    let encoder = zstd::Encoder::new(BufWriter::new(file), ZSTD_LEVEL).map_err(|source| {
        ReconcileError::Filesystem {
            path: archive_path.display().to_string(),
            source,
        }
    })?;

    let mut builder = tar::Builder::new(encoder);

    for relative in &paths {
        let absolute = layout::resolve(bundle_root, relative)?;
        append(&mut builder, &absolute, relative)?;
    }

    let encoder = builder
        .into_inner()
        .map_err(|source| ReconcileError::Filesystem {
            path: archive_path.display().to_string(),
            source,
        })?;

    encoder
        .finish()
        .map_err(|source| ReconcileError::Filesystem {
            path: archive_path.display().to_string(),
            source,
        })?;

    Ok(())
}

/// Appends one file with normalised metadata.
fn append<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    absolute: &Path,
    relative: &str,
) -> Result<(), ReconcileError> {
    let contents = std::fs::read(absolute).map_err(|source| ReconcileError::Filesystem {
        path: absolute.display().to_string(),
        source,
    })?;

    let mut header = tar::Header::new_gnu();
    header.set_size(contents.len() as u64);
    header.set_mode(FILE_MODE);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_entry_type(tar::EntryType::Regular);
    header.set_cksum();

    builder
        .append_data(&mut header, relative, contents.as_slice())
        .map_err(|source| ReconcileError::Filesystem {
            path: relative.to_owned(),
            source,
        })
}

/// Lists bundle-relative file paths in sorted order.
///
/// Symbolic links are not followed and are not archived: an evidence bundle is a set of
/// regular files, and a link would either duplicate content or point outside the bundle.
fn collect_sorted_paths(root: &Path) -> Result<Vec<String>, ReconcileError> {
    let mut found = Vec::new();
    walk(root, root, &mut found)?;
    found.sort();
    Ok(found)
}

fn walk(root: &Path, directory: &Path, found: &mut Vec<String>) -> Result<(), ReconcileError> {
    let entries = std::fs::read_dir(directory).map_err(|source| ReconcileError::Filesystem {
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
            std::fs::symlink_metadata(&path).map_err(|source| ReconcileError::Filesystem {
                path: path.display().to_string(),
                source,
            })?;

        if metadata.is_symlink() {
            continue;
        }

        if metadata.is_dir() {
            walk(root, &path, found)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| ReconcileError::Internal {
                    reason: format!("path {} escaped the bundle root", path.display()),
                })?;
            let relative = relative
                .to_str()
                .ok_or_else(|| ReconcileError::ManifestInvalid {
                    reason: format!("file name is not valid UTF-8: {}", relative.display()),
                })?;
            layout::validate_relative_path(relative)?;
            found.push(relative.to_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evidence::hashing;
    use std::io::Write;

    fn write_file(root: &Path, relative: &str, contents: &[u8]) {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut file = File::create(&path).unwrap();
        file.write_all(contents).unwrap();
        file.sync_all().unwrap();
    }

    fn sample_bundle(root: &Path) {
        write_file(root, layout::MANIFEST, b"{\"schema_version\":\"1.0.0\"}");
        write_file(root, layout::ANCHOR_BLOCK, b"00aabb");
        write_file(root, "blocks/3428143.hex", b"0011");
        write_file(root, "blocks/3428144.hex", b"2233");
        write_file(root, "metadata/command.txt", b"capture");
    }

    #[test]
    fn packing_produces_an_archive() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        sample_bundle(&bundle);

        let archive = dir.path().join("bundle.tar.zst");
        pack(&bundle, &archive).unwrap();

        assert!(archive.is_file());
        assert!(hashing::file_size(&archive).unwrap() > 0);
    }

    #[test]
    fn packing_the_same_bundle_twice_yields_identical_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        sample_bundle(&bundle);

        let first = dir.path().join("first.tar.zst");
        let second = dir.path().join("second.tar.zst");
        pack(&bundle, &first).unwrap();
        pack(&bundle, &second).unwrap();

        assert_eq!(
            hashing::sha256_file(&first).unwrap(),
            hashing::sha256_file(&second).unwrap(),
            "packing must be reproducible"
        );
    }

    #[test]
    fn two_bundles_with_identical_content_pack_identically() {
        let dir = tempfile::tempdir().unwrap();
        let first_bundle = dir.path().join("a");
        let second_bundle = dir.path().join("b");
        std::fs::create_dir_all(&first_bundle).unwrap();
        std::fs::create_dir_all(&second_bundle).unwrap();
        sample_bundle(&first_bundle);
        sample_bundle(&second_bundle);

        let first = dir.path().join("a.tar.zst");
        let second = dir.path().join("b.tar.zst");
        pack(&first_bundle, &first).unwrap();
        pack(&second_bundle, &second).unwrap();

        assert_eq!(
            hashing::sha256_file(&first).unwrap(),
            hashing::sha256_file(&second).unwrap(),
            "bundles differing only in location must pack identically"
        );
    }

    #[test]
    fn changing_one_byte_changes_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        sample_bundle(&bundle);

        let before_path = dir.path().join("before.tar.zst");
        pack(&bundle, &before_path).unwrap();
        let before = hashing::sha256_file(&before_path).unwrap();

        write_file(&bundle, "blocks/3428143.hex", b"00ff");
        let after_path = dir.path().join("after.tar.zst");
        pack(&bundle, &after_path).unwrap();

        assert_ne!(before, hashing::sha256_file(&after_path).unwrap());
    }

    #[test]
    fn paths_are_collected_in_sorted_order() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        sample_bundle(&bundle);

        let paths = collect_sorted_paths(&bundle).unwrap();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
        assert!(paths.contains(&"blocks/3428143.hex".to_owned()));
    }

    #[test]
    fn packing_with_a_digest_writes_a_checkable_sidecar() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        sample_bundle(&bundle);

        let archive = dir.path().join("evidence.tar.zst");
        let digest = pack_with_digest(&bundle, &archive).unwrap();

        let sidecar = dir.path().join("evidence.tar.zst.sha256");
        let contents = std::fs::read_to_string(&sidecar).unwrap();

        assert_eq!(contents, format!("{digest}  evidence.tar.zst\n"));
        assert_eq!(digest, hashing::sha256_file(&archive).unwrap());
    }

    #[test]
    fn the_digest_sidecar_is_named_by_appending_rather_than_replacing() {
        // `with_extension` replaces the last extension, so an archive named without one
        // would have produced `bundle..sha256`.
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        sample_bundle(&bundle);

        for (name, expected) in [
            ("evidence.tar.zst", "evidence.tar.zst.sha256"),
            ("evidence.tar", "evidence.tar.sha256"),
            ("evidence", "evidence.sha256"),
        ] {
            let archive = dir.path().join(name);
            pack_with_digest(&bundle, &archive).unwrap();
            assert!(
                dir.path().join(expected).is_file(),
                "expected a sidecar named {expected} for an archive named {name}"
            );
        }
    }

    #[test]
    fn symbolic_links_are_not_archived() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("bundle");
        std::fs::create_dir_all(&bundle).unwrap();
        sample_bundle(&bundle);

        std::os::unix::fs::symlink("/etc/passwd", bundle.join("link.txt")).unwrap();

        let paths = collect_sorted_paths(&bundle).unwrap();
        assert!(
            !paths.iter().any(|path| path == "link.txt"),
            "a symbolic link must not be archived"
        );
    }
}
