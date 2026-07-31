//! Evidence file hashing.
//!
//! Files are hashed incrementally rather than read into memory whole, so that bundle size
//! does not bound memory use.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::ReconcileError;

/// Buffer size for incremental hashing.
const CHUNK_SIZE: usize = 64 * 1024;

pub use crate::canonical::sha256_hex;

/// SHA-256 digest of a file's contents, as lowercase hexadecimal.
pub fn sha256_file(path: &Path) -> Result<String, ReconcileError> {
    let file = File::open(path).map_err(|source| ReconcileError::Filesystem {
        path: path.display().to_string(),
        source,
    })?;

    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; CHUNK_SIZE];

    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| ReconcileError::Filesystem {
                path: path.display().to_string(),
                source,
            })?;
        if read == 0 {
            break;
        }
        match buffer.get(..read) {
            Some(chunk) => hasher.update(chunk),
            None => {
                return Err(ReconcileError::Internal {
                    reason: "read count exceeded buffer length".to_owned(),
                });
            }
        }
    }

    Ok(hex::encode(hasher.finalize()))
}

/// Byte length of a file, used to populate and verify manifest size entries.
pub fn file_size(path: &Path) -> Result<u64, ReconcileError> {
    let metadata = std::fs::metadata(path).map_err(|source| ReconcileError::Filesystem {
        path: path.display().to_string(),
        source,
    })?;
    Ok(metadata.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(contents: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("evidence.bin");
        let mut file = File::create(&path).unwrap();
        file.write_all(contents).unwrap();
        file.sync_all().unwrap();
        (dir, path)
    }

    #[test]
    fn known_digest_of_the_empty_input() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn known_digest_of_a_short_input() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn file_digest_matches_in_memory_digest() {
        let contents = b"raw consensus bytes";
        let (_dir, path) = temp_file(contents);
        assert_eq!(sha256_file(&path).unwrap(), sha256_hex(contents));
    }

    #[test]
    fn file_digest_is_correct_across_multiple_buffer_chunks() {
        let contents = vec![0xAB_u8; CHUNK_SIZE * 3 + 17];
        let (_dir, path) = temp_file(&contents);
        assert_eq!(sha256_file(&path).unwrap(), sha256_hex(&contents));
    }

    #[test]
    fn a_single_altered_byte_changes_the_digest() {
        let (_dir, original) = temp_file(b"raw consensus bytes");
        let (_dir2, altered) = temp_file(b"raw consensus byteS");
        assert_ne!(
            sha256_file(&original).unwrap(),
            sha256_file(&altered).unwrap()
        );
    }

    #[test]
    fn digests_are_lowercase_hexadecimal() {
        let digest = sha256_hex(b"anything");
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!digest.chars().any(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn missing_file_reports_a_filesystem_error() {
        let result = sha256_file(Path::new("/nonexistent/evidence.bin"));
        assert!(matches!(result, Err(ReconcileError::Filesystem { .. })));
    }

    #[test]
    fn file_size_matches_written_length() {
        let contents = vec![7_u8; 4096];
        let (_dir, path) = temp_file(&contents);
        assert_eq!(file_size(&path).unwrap(), 4096);
    }
}
