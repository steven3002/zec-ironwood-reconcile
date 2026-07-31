//! Bounds applied to untrusted archives.
//!
//! An evidence archive is the one input this tool accepts from an arbitrary third party. A
//! crafted archive can attempt to write outside the extraction directory, to exhaust disk
//! through a compression bomb, or to exhaust memory through a declared size that is never
//! honoured. Every bound here exists to make one of those a refusal rather than a resource
//! exhaustion.

use crate::error::ReconcileError;

/// Caps applied while extracting an archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionLimits {
    /// Total decompressed bytes across all entries.
    pub max_total_bytes: u64,
    /// Number of entries.
    pub max_entries: u32,
    /// Decompressed bytes in any single entry.
    pub max_entry_bytes: u64,
    /// Path components in any single entry.
    pub max_path_depth: usize,
}

impl Default for ExtractionLimits {
    /// Bounds sized for the largest bundle this tool is intended to produce.
    ///
    /// A one-thousand-block mainnet interval stored as hex-encoded consensus bytes, with a
    /// reported-pools file per height, sits far below these. They are deliberately generous
    /// enough not to reject honest evidence and far too small to permit an unbounded
    /// expansion.
    fn default() -> Self {
        Self {
            max_total_bytes: 8 * 1024 * 1024 * 1024,
            max_entries: 100_000,
            max_entry_bytes: 512 * 1024 * 1024,
            max_path_depth: 8,
        }
    }
}

impl ExtractionLimits {
    pub fn check_entry_size(&self, path: &str, size: u64) -> Result<(), ReconcileError> {
        if size > self.max_entry_bytes {
            return Err(ReconcileError::ArchiveRejected {
                reason: format!(
                    "entry {path:?} declares {size} bytes, above the per-entry limit of {}",
                    self.max_entry_bytes
                ),
            });
        }
        Ok(())
    }

    pub fn check_running_total(&self, total: u64) -> Result<(), ReconcileError> {
        if total > self.max_total_bytes {
            return Err(ReconcileError::ArchiveRejected {
                reason: format!(
                    "archive exceeds the total extraction limit of {} bytes",
                    self.max_total_bytes
                ),
            });
        }
        Ok(())
    }

    pub fn check_entry_count(&self, count: u32) -> Result<(), ReconcileError> {
        if count > self.max_entries {
            return Err(ReconcileError::ArchiveRejected {
                reason: format!("archive exceeds the entry limit of {}", self.max_entries),
            });
        }
        Ok(())
    }

    pub fn check_depth(&self, path: &str) -> Result<(), ReconcileError> {
        let depth = path.split('/').filter(|part| !part.is_empty()).count();
        if depth > self.max_path_depth {
            return Err(ReconcileError::ArchiveRejected {
                reason: format!(
                    "entry {path:?} is nested {depth} deep, above the limit of {}",
                    self.max_path_depth
                ),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_accommodate_a_realistic_bundle() {
        let limits = ExtractionLimits::default();
        // Roughly a 1000-block interval: two files per height plus anchor and metadata.
        assert!(limits.check_entry_count(2_010).is_ok());
        assert!(limits.check_running_total(4 * 1024 * 1024 * 1024).is_ok());
        assert!(limits.check_depth("blocks/3428143.hex").is_ok());
    }

    #[test]
    fn an_oversized_entry_is_rejected() {
        let limits = ExtractionLimits::default();
        assert!(matches!(
            limits.check_entry_size("blocks/1.hex", limits.max_entry_bytes + 1),
            Err(ReconcileError::ArchiveRejected { .. })
        ));
    }

    #[test]
    fn an_oversized_total_is_rejected() {
        let limits = ExtractionLimits::default();
        assert!(limits.check_running_total(limits.max_total_bytes).is_ok());
        assert!(
            limits
                .check_running_total(limits.max_total_bytes + 1)
                .is_err()
        );
    }

    #[test]
    fn too_many_entries_are_rejected() {
        let limits = ExtractionLimits::default();
        assert!(limits.check_entry_count(limits.max_entries).is_ok());
        assert!(limits.check_entry_count(limits.max_entries + 1).is_err());
    }

    #[test]
    fn excessive_nesting_is_rejected() {
        let limits = ExtractionLimits::default();
        let deep = "a/b/c/d/e/f/g/h/i/j/file.txt";
        assert!(matches!(
            limits.check_depth(deep),
            Err(ReconcileError::ArchiveRejected { .. })
        ));
    }

    #[test]
    fn limits_can_be_tightened_for_tests_and_callers() {
        let strict = ExtractionLimits {
            max_total_bytes: 1_024,
            max_entries: 4,
            max_entry_bytes: 512,
            max_path_depth: 2,
        };
        assert!(strict.check_entry_count(5).is_err());
        assert!(strict.check_entry_size("x", 513).is_err());
        assert!(strict.check_depth("a/b/c").is_err());
    }
}
