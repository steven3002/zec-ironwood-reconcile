//! The module dependency rules, enforced mechanically.
//!
//! The architecture's central rule is that reconciliation and everything downstream of it
//! cannot reach the network layer. That is what makes offline verification structural
//! rather than a feature somebody has to remember to preserve, but only while the rule
//! actually holds, and a rule enforced by review holds until the review that misses it.
//!
//! These tests read the source and fail on a violation. They are deliberately blunt: a
//! textual scan cannot be defeated by an unusual import form, and the crate is small enough
//! that the cost of bluntness is a rare explicit exception rather than constant friction.
//!
//! Test modules are excluded. A test may reach for whatever it needs to build a fixture;
//! the rule constrains what the shipped code paths can touch.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

/// Layers, and the layers each one may never import.
///
/// Mirrors the dependency table in the architecture document. A change here is an
/// architectural decision, not a test fix.
const FORBIDDEN: &[(&str, &[&str])] = &[
    // The vocabulary of the problem. Depends on nothing but the error type.
    (
        "domain",
        &[
            "parse",
            "reconcile",
            "checks",
            "report",
            "rpc",
            "capture",
            "evidence",
            "commands",
            "cli",
        ],
    ),
    (
        "parse",
        &["rpc", "capture", "evidence", "report", "commands"],
    ),
    ("reconcile", &["rpc", "capture", "evidence", "commands"]),
    ("checks", &["rpc", "capture", "commands"]),
    ("report", &["rpc", "capture", "commands"]),
    // Transport knows nothing about accounting.
    (
        "rpc",
        &["parse", "reconcile", "checks", "report", "commands"],
    ),
    ("evidence", &["rpc", "parse", "reconcile", "commands"]),
    ("capture", &["reconcile", "checks", "report"]),
];

/// Reads a source file with line endings normalised to `\n`.
///
/// The scans in this file are anchored at the start of each line, so a trailing carriage
/// return does not currently change a verdict. Normalising anyway keeps that from being a
/// property anyone has to re-establish: a CRLF checkout is the default on Windows, and a
/// scan added later that searches for a literal containing `\n` would fail there while
/// passing everywhere else.
fn read_source(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap().replace("\r\n", "\n")
}

/// Source of a file, with test modules and comments removed.
///
/// Everything from the first `#[cfg(test)]` onward is dropped. Every file in this crate
/// places its test module last, and the test below fails loudly if one does not.
fn shipped_source(path: &Path) -> String {
    let text = read_source(path);
    let body = match text.find("#[cfg(test)]") {
        Some(index) => &text[..index],
        None => &text[..],
    };

    body.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn rust_files_under(directory: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![directory.to_path_buf()];

    while let Some(current) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
    }

    found.sort();
    found
}

fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

#[test]
fn no_layer_imports_a_layer_it_is_forbidden_to_reach() {
    let mut violations = Vec::new();

    for (layer, forbidden) in FORBIDDEN {
        let directory = source_root().join(layer);
        let files = rust_files_under(&directory);
        assert!(
            !files.is_empty(),
            "layer {layer} has no source files; the rule table is out of date"
        );

        for file in files {
            let source = shipped_source(&file);
            for other in *forbidden {
                let needle = format!("crate::{other}");
                if source.contains(&needle) {
                    violations.push(format!(
                        "{} imports {needle}, which {layer}/ may never reach",
                        file.strip_prefix(source_root()).unwrap().display()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "the module dependency rules were broken:\n  {}",
        violations.join("\n  ")
    );
}

#[test]
fn the_offline_verification_path_cannot_reach_the_network_layer() {
    // The specific consequence the architecture exists to guarantee, asserted directly
    // rather than left to follow from the table above.
    let mut offenders = Vec::new();

    // Every command except `capture` runs offline, and `verify` reaches the accounting path
    // by calling straight into `commands/reconcile.rs`. Naming the one exception rather than
    // listing the members means a command added later is covered by default instead of
    // escaping the rule, which is how `commands/reconcile.rs` came to sit on the
    // verification path unscanned while only `commands/verify.rs` was listed.
    let mut files: Vec<PathBuf> = rust_files_under(&source_root().join("commands"))
        .into_iter()
        .filter(|path| !path.ends_with("capture.rs"))
        .collect();
    assert!(
        source_root().join("commands/capture.rs").is_file(),
        "the exempted command no longer exists, so the exemption is stale"
    );

    for layer in [
        "parse",
        "reconcile",
        "checks",
        "report",
        "evidence",
        "domain",
    ] {
        files.extend(rust_files_under(&source_root().join(layer)));
    }

    // The top-level modules are on the verification path too. `canonical.rs` in particular
    // is reached by every hashed artifact, and belongs to no layer directory, so the rule
    // table above does not cover it.
    files.push(source_root().join("canonical.rs"));
    files.push(source_root().join("error.rs"));

    for file in files {
        let source = shipped_source(&file);
        if source.contains("crate::rpc")
            || source.contains("crate::capture")
            || source.contains("ureq")
        {
            offenders.push(
                file.strip_prefix(source_root())
                    .unwrap()
                    .display()
                    .to_string(),
            );
        }
    }

    assert!(
        offenders.is_empty(),
        "verification code reached the network layer: {offenders:?}"
    );
}

#[test]
fn only_the_transport_module_names_the_http_client() {
    // `ureq` is linked into the binary for `capture`. Confining it to one module is what
    // keeps "verification opens no socket" a property of the code rather than a claim.
    let mut offenders = Vec::new();

    for file in rust_files_under(&source_root()) {
        if file.ends_with("rpc/client.rs") {
            continue;
        }
        if shipped_source(&file).contains("ureq") {
            offenders.push(
                file.strip_prefix(source_root())
                    .unwrap()
                    .display()
                    .to_string(),
            );
        }
    }

    assert_eq!(
        offenders,
        Vec::<String>::new(),
        "the HTTP client is referenced outside rpc/client.rs"
    );
}

#[test]
fn test_modules_are_last_in_every_file() {
    // `shipped_source` drops everything from the first `#[cfg(test)]` onward. If a file put
    // shipped code after its tests, that code would escape every rule above.
    let mut offenders = Vec::new();

    for file in rust_files_under(&source_root()) {
        let text = read_source(&file);
        let Some(index) = text.find("#[cfg(test)]") else {
            continue;
        };

        let after = &text[index..];
        // Everything following the marker must belong to the test module: its attribute,
        // its `mod tests` line, and the indented body.
        let escaped = after.lines().skip(1).any(|line| {
            let trimmed = line.trim_start();
            !line.starts_with(' ')
                && !trimmed.is_empty()
                && !trimmed.starts_with("mod tests")
                && !trimmed.starts_with('}')
                && !trimmed.starts_with("//")
        });

        if escaped {
            offenders.push(
                file.strip_prefix(source_root())
                    .unwrap()
                    .display()
                    .to_string(),
            );
        }
    }

    assert_eq!(
        offenders,
        Vec::<String>::new(),
        "shipped code appears after a test module, where the layering scan cannot see it"
    );
}

#[test]
fn no_top_level_module_outside_the_layers_reaches_the_network() {
    // `canonical.rs` and `error.rs` sit outside every layer directory, so nothing in the
    // rule table constrains them. They are on the verification path, so they are named here
    // explicitly and a new top-level module must be classified rather than slip through.
    let unclassified: Vec<String> = std::fs::read_dir(source_root())
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|e| e == "rs"))
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            !matches!(
                name.as_str(),
                "lib.rs" | "main.rs" | "canonical.rs" | "error.rs"
            )
        })
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        unclassified,
        Vec::<String>::new(),
        "a top-level module exists that no layering rule covers"
    );
}

#[test]
fn the_rule_table_covers_every_layer_that_exists() {
    // A new layer must be classified deliberately, not default to unconstrained.
    let unlisted: Vec<String> = std::fs::read_dir(source_root())
        .unwrap()
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            // `commands` orchestrates every layer by design; `cli` is argument definitions.
            !matches!(name.as_str(), "commands" | "cli")
                && !FORBIDDEN.iter().any(|(layer, _)| layer == name)
        })
        .collect();

    assert_eq!(
        unlisted,
        Vec::<String>::new(),
        "a source layer has no entry in the dependency rule table"
    );
}
