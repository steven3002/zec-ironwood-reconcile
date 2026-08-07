//! Evidence bundle path conventions.
//!
//! This module is the only place bundle-relative paths are constructed or validated.
//! Centralising it means a path appearing anywhere else in the crate is a defect, and it
//! gives a single location to enforce that paths read from an untrusted manifest cannot
//! escape the bundle root.

use std::path::{Component, Path, PathBuf};

use crate::domain::height::BlockHeight;
use crate::error::ReconcileError;

pub const MANIFEST: &str = "manifest.json";
pub const MANIFEST_HASH: &str = "manifest.sha256";

pub const ANCHOR_BLOCK: &str = "anchor/block.hex";
pub const ANCHOR_VALUE_POOLS: &str = "anchor/value-pools.json";

pub const RPC_CHAIN_INFO_START: &str = "rpc/blockchain-info-start.json";
pub const RPC_CHAIN_INFO_END: &str = "rpc/blockchain-info-end.json";
pub const RPC_NODE_INFO: &str = "rpc/node-info.json";

pub const REPORT_JSON: &str = "reports/report.json";
pub const REPORT_MARKDOWN: &str = "reports/report.md";
pub const REPORT_HASH: &str = "reports/report.sha256";

pub const METADATA_ENVIRONMENT: &str = "metadata/capture-environment.json";
pub const METADATA_COMMAND: &str = "metadata/command.txt";
pub const METADATA_TOOL_VERSION: &str = "metadata/tool-version.txt";

/// Raw consensus bytes for a block, hex encoded.
///
/// Raw bytes are stored rather than a node's decoded JSON so that the evidence is stable
/// across node versions and independent of any node's presentation choices.
pub fn block(height: BlockHeight) -> String {
    format!("blocks/{height}.hex")
}

/// Per-height value pool balances as reported by the capturing node.
pub fn block_value_pools(height: BlockHeight) -> String {
    format!("blocks/{height}.pools.json")
}

/// Files that are produced by reconciliation rather than capture.
///
/// These are excluded from the "unlisted file" warning, because a bundle may legitimately
/// gain reports after it was captured and hashed.
pub const GENERATED_PATHS: [&str; 4] = [MANIFEST_HASH, REPORT_JSON, REPORT_MARKDOWN, REPORT_HASH];

/// Rejects any path that is not a plain relative path inside the bundle.
///
/// Manifest entries are untrusted input: a bundle can be authored by anyone. A path
/// containing `..`, a root component, or a Windows prefix could cause a reader to open or
/// overwrite a file outside the bundle directory.
pub fn validate_relative_path(path: &str) -> Result<(), ReconcileError> {
    let reject = |reason: &str| {
        Err(ReconcileError::ManifestInvalid {
            reason: format!("unsafe file path {path:?}: {reason}"),
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

/// Renders a path inside a bundle as the bundle-relative form the manifest records.
///
/// The separator is always `/`, on every platform. A `Path` on Windows renders with
/// backslashes, so producing this string with `to_str` yields `blocks\1.hex` there against
/// the `blocks/1.hex` a manifest stores. Nothing then matches: every file in the bundle is
/// reported as unlisted, that becomes a warning, and the warning enters the hashed check
/// array, so the same evidence yields a different report hash on Windows than on Linux,
/// which is precisely the property this project exists to guarantee.
///
/// Built from [`Path::components`] rather than by replacing characters, so the result cannot
/// depend on how the caller spelled the path.
pub fn to_bundle_path(root: &Path, path: &Path) -> Result<String, ReconcileError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| ReconcileError::Internal {
            reason: format!("path {} escaped the bundle root", path.display()),
        })?;

    let mut rendered = String::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Err(ReconcileError::ManifestInvalid {
                reason: format!("unexpected path component in {}", relative.display()),
            });
        };
        let part = part
            .to_str()
            .ok_or_else(|| ReconcileError::ManifestInvalid {
                reason: format!("file name is not valid UTF-8: {}", relative.display()),
            })?;

        if !rendered.is_empty() {
            rendered.push('/');
        }
        rendered.push_str(part);
    }

    Ok(rendered)
}

/// Resolves a bundle-relative path against a bundle root, after validating it.
pub fn resolve(root: &Path, relative: &str) -> Result<PathBuf, ReconcileError> {
    validate_relative_path(relative)?;
    Ok(root.join(relative))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_paths_are_derived_from_height() {
        assert_eq!(block(BlockHeight::new(3_428_143)), "blocks/3428143.hex");
        assert_eq!(
            block_value_pools(BlockHeight::new(3_428_143)),
            "blocks/3428143.pools.json"
        );
    }

    #[test]
    fn well_formed_relative_paths_are_accepted() {
        for path in [
            MANIFEST,
            ANCHOR_BLOCK,
            ANCHOR_VALUE_POOLS,
            REPORT_JSON,
            "blocks/3428143.hex",
            "metadata/command.txt",
        ] {
            assert!(validate_relative_path(path).is_ok(), "rejected {path}");
        }
    }

    #[test]
    fn parent_directory_traversal_is_rejected() {
        for path in [
            "../escape",
            "blocks/../../escape",
            "..",
            "a/b/../../../etc/passwd",
        ] {
            assert!(
                matches!(
                    validate_relative_path(path),
                    Err(ReconcileError::ManifestInvalid { .. })
                ),
                "accepted traversal path {path}"
            );
        }
    }

    #[test]
    fn absolute_paths_are_rejected() {
        for path in ["/etc/passwd", "/", "//tmp/x"] {
            assert!(
                validate_relative_path(path).is_err(),
                "accepted absolute path {path}"
            );
        }
    }

    #[test]
    fn backslashes_and_null_bytes_are_rejected() {
        assert!(validate_relative_path("blocks\\1.hex").is_err());
        assert!(validate_relative_path("..\\..\\escape").is_err());
        assert!(validate_relative_path("blocks/\u{0}1.hex").is_err());
    }

    #[test]
    fn empty_and_current_directory_paths_are_rejected() {
        assert!(validate_relative_path("").is_err());
        assert!(validate_relative_path("./blocks/1.hex").is_err());
    }

    #[test]
    fn a_bundle_path_always_uses_forward_slashes() {
        // Built from components, so the rendered form cannot inherit the platform
        // separator. On Windows `to_str` would yield `blocks\\1.hex`, which matches nothing
        // in a manifest and made every file look unlisted, a warning that then entered the
        // hashed check array and changed the report hash.
        let root = Path::new("/bundles/example");
        let nested = root.join("blocks").join("3428143.hex");

        let rendered = to_bundle_path(root, &nested).unwrap();
        assert_eq!(rendered, "blocks/3428143.hex");
        assert!(!rendered.contains('\\'));
        assert!(validate_relative_path(&rendered).is_ok());
    }

    #[test]
    fn a_bundle_path_at_the_root_has_no_separator() {
        let root = Path::new("/bundles/example");
        assert_eq!(
            to_bundle_path(root, &root.join(MANIFEST)).unwrap(),
            MANIFEST
        );
    }

    #[test]
    fn a_path_outside_the_root_is_refused() {
        assert!(to_bundle_path(Path::new("/bundles/example"), Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn every_rendered_bundle_path_round_trips_through_resolve() {
        let root = Path::new("/bundles/example");
        for relative in [MANIFEST, ANCHOR_BLOCK, "blocks/1.hex", METADATA_COMMAND] {
            let resolved = resolve(root, relative).unwrap();
            assert_eq!(to_bundle_path(root, &resolved).unwrap(), relative);
        }
    }

    #[test]
    fn resolve_keeps_the_result_inside_the_root() {
        let root = Path::new("/bundles/example");
        let resolved = resolve(root, "blocks/3428143.hex").unwrap();
        assert_eq!(
            resolved,
            PathBuf::from("/bundles/example/blocks/3428143.hex")
        );
        assert!(resolved.starts_with(root));
    }

    #[test]
    fn resolve_refuses_to_escape_the_root() {
        assert!(resolve(Path::new("/bundles/example"), "../../etc/passwd").is_err());
    }
}
