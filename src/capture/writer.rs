//! Writing an evidence bundle to disk.
//!
//! # A file that exists is a file that is complete
//!
//! Every write goes to a temporary name in the destination directory, is flushed, and is
//! then renamed into place. Rename within a directory is atomic, so a run interrupted at
//! any point leaves either the previous state or the finished file, never a half-written
//! one. That property is what makes `--resume` sound: a file already present can be trusted
//! to be whole, so the capture can skip retrieving it again.
//!
//! The manifest is written last, so a bundle carrying a manifest is a bundle whose capture
//! finished.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::domain::height::{BlockHeight, HeightInterval};
use crate::error::ReconcileError;
use crate::evidence::hashing;
use crate::evidence::layout;
use crate::evidence::manifest::{Encoding, FileEntry, Manifest};

/// Suffix of a file that is being written.
///
/// Deliberately not a bundle-relative path: it never survives a successful write, and is
/// removed on the next attempt if a process died mid-write.
const PARTIAL_SUFFIX: &str = ".partial";

/// How an existing output directory is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMode {
    /// Refuse to write into a directory that already holds anything.
    Create,
    /// Replace an existing bundle.
    Overwrite,
    /// Keep files already captured and retrieve only what is missing.
    Resume,
}

/// Accumulates a bundle's files and their digests.
#[derive(Debug)]
pub struct BundleWriter {
    root: PathBuf,
    files: Vec<FileEntry>,
    reused: u32,
    written: u32,
}

impl BundleWriter {
    /// Prepares an output directory.
    pub fn open(root: &Path, mode: OutputMode) -> Result<Self, ReconcileError> {
        match mode {
            OutputMode::Overwrite => clear_existing_bundle(root)?,
            OutputMode::Create => refuse_non_empty(root)?,
            OutputMode::Resume => {}
        }

        fs::create_dir_all(root).map_err(|source| ReconcileError::Filesystem {
            path: root.display().to_string(),
            source,
        })?;

        Ok(Self {
            root: root.to_path_buf(),
            files: Vec::new(),
            reused: 0,
            written: 0,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn reused_count(&self) -> u32 {
        self.reused
    }

    pub const fn written_count(&self) -> u32 {
        self.written
    }

    /// Whether a bundle file is already present.
    pub fn contains(&self, relative: &str) -> Result<bool, ReconcileError> {
        Ok(layout::resolve(&self.root, relative)?.is_file())
    }

    /// Refuses to continue into a bundle already holding blocks from another interval.
    ///
    /// `--resume` means "finish this capture", not "start a different one in the same
    /// directory". Without this, resuming with different bounds leaves the earlier
    /// interval's blocks on disk: the manifest does not list them, so verification only
    /// warns, and a published archive would carry blocks outside the interval it declares.
    pub fn ensure_no_blocks_outside(&self, interval: HeightInterval) -> Result<(), ReconcileError> {
        let blocks = self.root.join("blocks");
        let Ok(entries) = fs::read_dir(&blocks) else {
            return Ok(());
        };

        let mut foreign: Vec<u32> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.to_owned();
                let height: u32 = name.split('.').next()?.parse().ok()?;
                (!interval.contains(BlockHeight::new(height))).then_some(height)
            })
            .collect();

        if foreign.is_empty() {
            return Ok(());
        }

        foreign.sort_unstable();
        foreign.dedup();

        Err(ReconcileError::InvalidInput {
            reason: format!(
                "{} already holds blocks outside {}..={} (first: {}); it was captured for a \
                 different interval, so resuming would mix two captures. Use a new --output \
                 directory, or --overwrite to replace it",
                self.root.display(),
                interval.start_height(),
                interval.end_height(),
                foreign.first().map_or(0, |height| *height),
            ),
        })
    }

    /// Writes a bundle file and records its digest.
    pub fn write(
        &mut self,
        relative: &str,
        contents: &[u8],
        encoding: Encoding,
    ) -> Result<(), ReconcileError> {
        let path = layout::resolve(&self.root, relative)?;
        write_atomically(&path, contents)?;

        self.record(relative, contents, encoding);
        self.written = self.written.saturating_add(1);
        Ok(())
    }

    /// Adopts a file already on disk, returning its contents for revalidation.
    ///
    /// Used by `--resume`. The digest recorded in the manifest is computed from the bytes
    /// found on disk, never carried over from a previous run, so a manifest can only ever
    /// describe what is actually present.
    pub fn adopt(&mut self, relative: &str, encoding: Encoding) -> Result<Vec<u8>, ReconcileError> {
        let path = layout::resolve(&self.root, relative)?;
        let contents = fs::read(&path).map_err(|source| ReconcileError::Filesystem {
            path: path.display().to_string(),
            source,
        })?;

        self.record(relative, &contents, encoding);
        self.reused = self.reused.saturating_add(1);
        Ok(contents)
    }

    fn record(&mut self, relative: &str, contents: &[u8], encoding: Encoding) {
        self.files.retain(|entry| entry.path != relative);
        self.files.push(FileEntry {
            path: relative.to_owned(),
            sha256: hashing::sha256_hex(contents),
            size_bytes: contents.len() as u64,
            encoding,
        });
    }

    /// Writes the manifest and its detached digest, completing the bundle.
    ///
    /// The file list is taken from what this writer actually wrote or adopted, so a manifest
    /// cannot claim a file the capture did not produce.
    pub fn finish(self, mut manifest: Manifest) -> Result<Manifest, ReconcileError> {
        manifest.files = Vec::new();
        for entry in self.files {
            manifest.add_file(entry);
        }
        manifest.validate_structure()?;

        let (_, digest) = manifest.to_canonical_bytes_and_hash()?;

        let json =
            serde_json::to_vec_pretty(&manifest).map_err(|source| ReconcileError::Internal {
                reason: format!("could not serialize the manifest: {source}"),
            })?;

        write_atomically(&layout::resolve(&self.root, layout::MANIFEST)?, &json)?;
        write_atomically(
            &layout::resolve(&self.root, layout::MANIFEST_HASH)?,
            format!("{digest}\n").as_bytes(),
        )?;

        Ok(manifest)
    }
}

/// Writes bytes to a temporary name and renames them into place.
fn write_atomically(path: &Path, contents: &[u8]) -> Result<(), ReconcileError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ReconcileError::Filesystem {
            path: parent.display().to_string(),
            source,
        })?;
    }

    let mut partial = path.as_os_str().to_owned();
    partial.push(PARTIAL_SUFFIX);
    let partial = PathBuf::from(partial);

    let filesystem_error = |path: &Path| {
        let path = path.display().to_string();
        move |source| ReconcileError::Filesystem {
            path: path.clone(),
            source,
        }
    };

    let mut file = File::create(&partial).map_err(filesystem_error(&partial))?;
    file.write_all(contents)
        .map_err(filesystem_error(&partial))?;
    // Flushed before the rename so the rename cannot publish a name whose contents are still
    // only in the page cache.
    file.sync_all().map_err(filesystem_error(&partial))?;
    drop(file);

    fs::rename(&partial, path).map_err(filesystem_error(path))
}

/// Refuses to write into a directory that already holds something.
fn refuse_non_empty(root: &Path) -> Result<(), ReconcileError> {
    if !root.exists() {
        return Ok(());
    }

    let mut entries = fs::read_dir(root).map_err(|source| ReconcileError::Filesystem {
        path: root.display().to_string(),
        source,
    })?;

    if entries.next().is_some() {
        return Err(ReconcileError::InvalidInput {
            reason: format!(
                "{} already contains files; pass --overwrite to replace the bundle or --resume \
                 to continue it",
                root.display()
            ),
        });
    }

    Ok(())
}

/// Removes an existing bundle so a fresh capture can replace it.
///
/// Only a directory this tool recognises as a bundle is removed. `--overwrite` is a
/// convenience for recapturing, not a licence to delete whatever the path happens to name,
/// and a mistyped `--output` must not destroy an unrelated directory.
fn clear_existing_bundle(root: &Path) -> Result<(), ReconcileError> {
    if !root.exists() {
        return Ok(());
    }

    if !root.is_dir() {
        return Err(ReconcileError::InvalidInput {
            reason: format!("{} is not a directory", root.display()),
        });
    }

    let mut entries = fs::read_dir(root).map_err(|source| ReconcileError::Filesystem {
        path: root.display().to_string(),
        source,
    })?;
    if entries.next().is_none() {
        return Ok(());
    }

    if !root.join(layout::MANIFEST).is_file() {
        return Err(ReconcileError::InvalidInput {
            reason: format!(
                "{} is not empty and does not contain {}, so it is not a bundle this tool wrote; \
                 refusing to delete it",
                root.display(),
                layout::MANIFEST
            ),
        });
    }

    fs::remove_dir_all(root).map_err(|source| ReconcileError::Filesystem {
        path: root.display().to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn writer(mode: OutputMode) -> (tempfile::TempDir, BundleWriter) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("bundle");
        let writer = BundleWriter::open(&root, mode).unwrap();
        (dir, writer)
    }

    #[test]
    fn a_written_file_lands_at_its_final_name_with_its_digest_recorded() {
        let (_dir, mut writer) = writer(OutputMode::Create);
        writer
            .write("blocks/100.hex", b"00aabb", Encoding::RawBlockHex)
            .unwrap();

        let path = writer.root().join("blocks/100.hex");
        assert!(path.is_file());
        assert_eq!(fs::read(&path).unwrap(), b"00aabb");
        assert_eq!(writer.files.len(), 1);
        assert_eq!(writer.files[0].sha256, hashing::sha256_hex(b"00aabb"));
        assert_eq!(writer.files[0].size_bytes, 6);
    }

    #[test]
    fn no_partial_file_survives_a_successful_write() {
        let (_dir, mut writer) = writer(OutputMode::Create);
        writer
            .write("blocks/100.hex", b"00aabb", Encoding::RawBlockHex)
            .unwrap();

        let leftover = writer.root().join("blocks/100.hex.partial");
        assert!(!leftover.exists(), "a partial file was left behind");
    }

    #[test]
    fn rewriting_a_path_replaces_its_entry_rather_than_duplicating_it() {
        let (_dir, mut writer) = writer(OutputMode::Create);
        writer
            .write("blocks/100.hex", b"0000", Encoding::RawBlockHex)
            .unwrap();
        writer
            .write("blocks/100.hex", b"ffff", Encoding::RawBlockHex)
            .unwrap();

        assert_eq!(writer.files.len(), 1);
        assert_eq!(writer.files[0].sha256, hashing::sha256_hex(b"ffff"));
    }

    #[test]
    fn an_adopted_file_is_hashed_from_disk() {
        let (_dir, mut writer) = writer(OutputMode::Create);
        writer
            .write("blocks/100.hex", b"00aabb", Encoding::RawBlockHex)
            .unwrap();

        let mut resumed = BundleWriter::open(writer.root(), OutputMode::Resume).unwrap();
        assert!(resumed.contains("blocks/100.hex").unwrap());

        let contents = resumed
            .adopt("blocks/100.hex", Encoding::RawBlockHex)
            .unwrap();
        assert_eq!(contents, b"00aabb");
        assert_eq!(resumed.files[0].sha256, hashing::sha256_hex(b"00aabb"));
        assert_eq!(resumed.reused_count(), 1);
        assert_eq!(resumed.written_count(), 0);
    }

    #[test]
    fn an_adopted_file_that_was_altered_is_recorded_as_it_now_is() {
        // The manifest must describe the bytes present, never the bytes a previous run
        // intended. A caller revalidates the returned contents; the digest cannot be stale.
        let (_dir, mut writer) = writer(OutputMode::Create);
        writer
            .write("blocks/100.hex", b"0000", Encoding::RawBlockHex)
            .unwrap();
        fs::write(writer.root().join("blocks/100.hex"), b"ffff").unwrap();

        let mut resumed = BundleWriter::open(writer.root(), OutputMode::Resume).unwrap();
        let contents = resumed
            .adopt("blocks/100.hex", Encoding::RawBlockHex)
            .unwrap();

        assert_eq!(contents, b"ffff");
        assert_eq!(resumed.files[0].sha256, hashing::sha256_hex(b"ffff"));
    }

    #[test]
    fn creating_into_a_non_empty_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("bundle");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("stray.txt"), b"x").unwrap();

        let error = BundleWriter::open(&root, OutputMode::Create).unwrap_err();
        assert!(matches!(error, ReconcileError::InvalidInput { .. }));
        assert!(error.to_string().contains("--overwrite"), "{error}");
    }

    #[test]
    fn creating_into_an_empty_directory_is_permitted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("bundle");
        fs::create_dir_all(&root).unwrap();
        assert!(BundleWriter::open(&root, OutputMode::Create).is_ok());
    }

    #[test]
    fn overwriting_replaces_a_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("bundle");
        fs::create_dir_all(root.join("blocks")).unwrap();
        fs::write(root.join(layout::MANIFEST), b"{}").unwrap();
        fs::write(root.join("blocks/1.hex"), b"old").unwrap();

        BundleWriter::open(&root, OutputMode::Overwrite).unwrap();
        assert!(!root.join("blocks/1.hex").exists());
        assert!(root.is_dir());
    }

    #[test]
    fn overwriting_refuses_a_directory_that_is_not_a_bundle() {
        // A mistyped --output must not delete an unrelated directory.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("important");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("thesis.txt"), b"years of work").unwrap();

        let error = BundleWriter::open(&root, OutputMode::Overwrite).unwrap_err();
        assert!(matches!(error, ReconcileError::InvalidInput { .. }));
        assert!(
            root.join("thesis.txt").is_file(),
            "unrelated data was deleted"
        );
    }

    #[test]
    fn overwriting_an_empty_directory_is_permitted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("bundle");
        fs::create_dir_all(&root).unwrap();
        assert!(BundleWriter::open(&root, OutputMode::Overwrite).is_ok());
    }

    #[test]
    fn resuming_leaves_existing_files_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("bundle");
        fs::create_dir_all(root.join("blocks")).unwrap();
        fs::write(root.join("blocks/1.hex"), b"kept").unwrap();

        BundleWriter::open(&root, OutputMode::Resume).unwrap();
        assert_eq!(fs::read(root.join("blocks/1.hex")).unwrap(), b"kept");
    }

    #[test]
    fn a_traversing_path_is_refused() {
        let (_dir, mut writer) = writer(OutputMode::Create);
        assert!(
            writer
                .write("../escape.hex", b"x", Encoding::RawBlockHex)
                .is_err()
        );
    }
}
