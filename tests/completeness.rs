//! Hand-maintained lists that must stay complete.
//!
//! Several places in this crate declare a set twice: once as the definition, and once as an
//! array or a test helper enumerating it. Rust checks a `match` for exhaustiveness, so
//! anything expressed as a match is safe. An array is not, adding a variant or a constant
//! and forgetting the array compiles cleanly and passes every test, and the test that was
//! supposed to cover the new member quietly covers nothing.
//!
//! That is not hypothetical. `ids::ALL` drives the canonical presentation order of a report,
//! so an identifier missing from it would sort by whatever order the checks happened to be
//! evaluated in, and a report hash that must be reproducible would stop being so.
//!
//! These tests read the source and compare the definition against the enumeration. They are
//! textual, which is the point: they see what the compiler is not asked to.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Reads a repository file with line endings normalised to `\n`.
///
/// Normalisation is not cosmetic. The scanners below locate the end of a block by searching
/// for a literal `"\n}\n"`, which does not occur in a file checked out with CRLF endings: the
/// bytes there are `"\n}\r\n"`. Git for Windows enables `core.autocrlf` by default and this
/// repository marks only `tests/fixtures/**` and `*.sh` as exempt, so every source file this
/// function reads arrives with CRLF on a Windows clone and every scan panics.
fn source(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    text.replace("\r\n", "\n")
}

/// Body between `opening` and `closing`, excluding both.
///
/// The opening marker is excluded deliberately: it carries the brace that opens the block,
/// and including it would leave every subsequent line one level deep, so a brace-depth scan
/// would never see a top-level declaration.
fn block_after(text: &str, opening: &str, closing: &str) -> String {
    let start = text
        .find(opening)
        .unwrap_or_else(|| panic!("could not find {opening:?} in the source"))
        + opening.len();
    let rest = &text[start..];
    let end = rest
        .find(closing)
        .unwrap_or_else(|| panic!("could not find the end of {opening:?}"));
    rest[..end].to_owned()
}

/// Names declared as `pub const NAME: &str = "value";`, returned as their values.
fn string_constants(block: &str) -> BTreeSet<String> {
    block
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("pub const ")?;
            let (_, value) = rest.split_once("&str = ")?;
            let value = value.trim().strip_prefix('"')?;
            let value = value.split('"').next()?;
            Some(value.to_owned())
        })
        .collect()
}

/// Variant names declared at the top level of an enum body.
///
/// Skips attributes, doc comments, and the field lines of struct-like variants, which are
/// indented further than the variant that introduces them.
fn enum_variants(block: &str, indent: &str) -> BTreeSet<String> {
    let mut variants = BTreeSet::new();
    let mut depth = 0_i32;

    for line in block.lines() {
        let trimmed = line.trim();

        // Track brace depth so a struct variant's fields are not read as variants.
        let entering = depth == 0
            && line.starts_with(indent)
            && !line.starts_with(&format!("{indent} "))
            && trimmed.chars().next().is_some_and(char::is_uppercase);

        if entering
            && let Some(name) = trimmed
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .next()
            && !name.is_empty()
        {
            variants.insert(name.to_owned());
        }

        depth += i32::try_from(trimmed.matches('{').count()).unwrap();
        depth -= i32::try_from(trimmed.matches('}').count()).unwrap();
        depth = depth.max(0);
    }

    variants
}

#[test]
fn every_check_identifier_is_listed_in_the_presentation_order() {
    let text = source("src/checks/mod.rs");
    let ids_module = block_after(&text, "pub mod ids {", "\n}\n");

    let defined = string_constants(&ids_module);
    assert!(
        defined.len() > 10,
        "the scan found only {} identifiers, so it is not reading the source correctly",
        defined.len()
    );

    let listed: BTreeSet<String> = zec_ironwood_reconcile::checks::ids::ALL
        .iter()
        .map(|id| (*id).to_owned())
        .collect();

    let missing: Vec<&String> = defined.difference(&listed).collect();
    assert!(
        missing.is_empty(),
        "check identifiers are defined but absent from ids::ALL, so a report containing them \
         would not sort deterministically: {missing:?}"
    );

    let unknown: Vec<&String> = listed.difference(&defined).collect();
    assert!(
        unknown.is_empty(),
        "ids::ALL lists identifiers that are not defined: {unknown:?}"
    );
}

#[test]
fn every_error_variant_is_covered_by_the_stable_identifier_test() {
    // `stable_id` is a match, so the compiler forces an identifier for every variant. What
    // is not enforced is that the uniqueness test actually exercises them all, a variant
    // absent from `every_variant()` could share an identifier with another and nothing
    // would notice.
    let text = source("src/error.rs");
    let enum_body = block_after(&text, "pub enum ReconcileError {", "\n}\n");
    let declared = enum_variants(&enum_body, "    ");

    assert!(
        declared.len() > 15,
        "the scan found only {} variants, so it is not reading the source correctly",
        declared.len()
    );

    let test_helper = block_after(&text, "fn every_variant()", "\n    }\n");
    let exercised: BTreeSet<String> = declared
        .iter()
        .filter(|variant| test_helper.contains(&format!("ReconcileError::{variant}")))
        .cloned()
        .collect();

    let missing: Vec<&String> = declared.difference(&exercised).collect();
    assert!(
        missing.is_empty(),
        "error variants are not exercised by every_variant(), so their stable identifiers \
         are untested: {missing:?}"
    );
}

#[test]
fn every_pool_variant_is_listed_in_all() {
    use zec_ironwood_reconcile::domain::pool::Pool;

    let text = source("src/domain/pool.rs");
    let enum_body = block_after(&text, "pub enum Pool {", "\n}\n");
    let declared = enum_variants(&enum_body, "    ");

    assert_eq!(
        declared.len(),
        Pool::ALL.len(),
        "Pool declares {} variants but Pool::ALL lists {}; the round-trip tests that iterate \
         ALL would silently skip the difference. Declared: {declared:?}",
        declared.len(),
        Pool::ALL.len()
    );

    let listed: BTreeSet<String> = Pool::ALL.iter().map(|pool| format!("{pool:?}")).collect();
    assert_eq!(listed, declared);
}

#[test]
fn the_generated_path_list_names_only_real_layout_constants() {
    // Under-listing here produces a spurious "unlisted file" warning rather than a false
    // pass, so it is the mildest member of this family, but a path named here that no
    // longer exists would silently stop suppressing anything.
    use zec_ironwood_reconcile::evidence::layout;

    let text = source("src/evidence/layout.rs");
    let declared = string_constants(&text);

    for path in layout::GENERATED_PATHS {
        assert!(
            declared.contains(path),
            "GENERATED_PATHS names {path:?}, which is not declared as a layout constant"
        );
    }
}

/// Text with every run of whitespace collapsed to a single space.
///
/// The published limitations are compared after this, so a document may wrap them across
/// lines to stay readable while still being required to carry the same words.
fn unwrapped(text: &str) -> String {
    text.split_whitespace().collect::<Vec<&str>>().join(" ")
}

#[test]
fn the_published_limitations_match_the_ones_every_report_carries() {
    // `LIMITATIONS.md` states that a report and the document cannot disagree. The strings
    // are compiled into the binary, so the document is the copy that can drift.
    use zec_ironwood_reconcile::report::schema::LIMITATIONS;

    let published = unwrapped(&source("LIMITATIONS.md"));

    let missing: Vec<&str> = LIMITATIONS
        .into_iter()
        .filter(|limitation| !published.contains(&unwrapped(limitation)))
        .collect();

    assert!(
        missing.is_empty(),
        "LIMITATIONS.md does not carry every limitation a report states, so the two disagree \
         about what the output may be cited for: {missing:?}"
    );
}

#[test]
fn the_limitation_comparison_would_notice_a_changed_word() {
    // Without this, a comparison that silently matched anything would make the test above
    // vacuous, the failure mode the rest of this file exists to catch.
    let published = unwrapped(&source("LIMITATIONS.md"));

    assert!(published.contains(&unwrapped("Does not verify zero-knowledge proofs.")));
    assert!(!published.contains(&unwrapped("Does not verify zero-knowledge proofs at all.")));
}

#[test]
fn the_scanner_recognises_a_variant_it_should_reject() {
    // A scan that silently matches nothing would make every test above vacuous. This pins
    // the parser's behaviour on input whose answer is known.
    let block = "
    Simple,
    Tuple(String),
    Struct {
        field: String,
        Nested: u32,
    },
    Last,
";
    let variants = enum_variants(block, "    ");
    assert_eq!(
        variants,
        ["Simple", "Tuple", "Struct", "Last"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<String>>(),
        "the enum scanner misreads variants; the completeness tests cannot be trusted"
    );
}

#[test]
fn the_constant_scanner_reads_only_string_constants() {
    let block = r#"
    pub const NAME: &str = "value";
    pub const OTHER: &str = "other_value";
    pub const NUMERIC: u32 = 7;
    const PRIVATE: &str = "private";
"#;
    let found = string_constants(block);
    assert!(found.contains("value"));
    assert!(found.contains("other_value"));
    assert!(!found.contains("7"));
    assert_eq!(found.len(), 2, "found {found:?}");
}

#[test]
fn every_scanned_source_file_exists_where_the_tests_expect_it() {
    for relative in [
        "src/checks/mod.rs",
        "src/error.rs",
        "src/domain/pool.rs",
        "src/evidence/layout.rs",
    ] {
        assert!(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(relative)
                .is_file(),
            "{relative} has moved; a completeness test is silently scanning nothing"
        );
    }
}
