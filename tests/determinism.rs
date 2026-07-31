//! Determinism of the canonical report.
//!
//! Offline verification rests entirely on one property: the same evidence and the same tool
//! version must produce byte-identical canonical JSON, and therefore an identical hash, on
//! any machine at any time. These tests exercise that property against the environmental
//! factors most likely to break it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use std::collections::BTreeMap;

use zec_ironwood_reconcile::checks::{Check, CheckRegistry, ids};
use zec_ironwood_reconcile::domain::height::{BlockHeight, HeightInterval};
use zec_ironwood_reconcile::domain::network::Network;
use zec_ironwood_reconcile::domain::pool::Pool;
use zec_ironwood_reconcile::domain::pool_state::ReportedPoolState;
use zec_ironwood_reconcile::domain::zatoshi::Zatoshi;
use zec_ironwood_reconcile::reconcile::interval::{AnchorBalances, reconcile_interval};
use zec_ironwood_reconcile::reconcile::ledger::BlockLedger;
use zec_ironwood_reconcile::report::schema::Report;
use zec_ironwood_reconcile::report::{self, ReportContext};

const ACTIVATION: u32 = 3_428_143;

fn ledger(height: u32, orchard: i64, ironwood: i64) -> BlockLedger {
    BlockLedger {
        height: BlockHeight::new(height),
        block_hash: format!("{height:064x}"),
        previous_block_hash: format!("{:064x}", height.saturating_sub(1)),
        orchard_delta: Zatoshi::from_raw(orchard),
        ironwood_delta: Zatoshi::from_raw(ironwood),
        transactions: Vec::new(),
    }
}

fn reported_state(height: u32, orchard_balance: i64, ironwood_balance: i64) -> ReportedPoolState {
    ReportedPoolState::new(BlockHeight::new(height))
        .with_balance(Pool::Orchard, Zatoshi::from_raw(orchard_balance))
        .with_balance(Pool::Ironwood, Zatoshi::from_raw(ironwood_balance))
}

/// Builds a report over a small but representative interval.
fn build_report() -> Report {
    let ledgers: Vec<BlockLedger> = (0..5)
        .map(|i| ledger(ACTIVATION + i, -1_000, 1_000))
        .collect();

    let mut reported = BTreeMap::new();
    for (index, _) in ledgers.iter().enumerate() {
        let step = i64::try_from(index).unwrap() + 1;
        reported.insert(
            BlockHeight::new(ACTIVATION + u32::try_from(index).unwrap()),
            reported_state(
                ACTIVATION + u32::try_from(index).unwrap(),
                366_000_000_000_000 - step * 1_000,
                step * 1_000,
            ),
        );
    }

    let interval = HeightInterval::new(
        BlockHeight::new(ACTIVATION),
        BlockHeight::new(ACTIVATION + 4),
    )
    .unwrap();

    let outcome = reconcile_interval(
        &ledgers,
        interval,
        AnchorBalances {
            orchard: Zatoshi::from_raw(366_000_000_000_000),
            ironwood: Zatoshi::ZERO,
        },
        &reported,
    )
    .unwrap();

    let mut registry = CheckRegistry::new();
    registry.record(Check::pass(ids::NETWORK_MATCHES));
    registry.record(Check::pass(ids::IRONWOOD_ANCHOR_ZERO));
    registry.record(Check::pass(ids::ORCHARD_WITHDRAWAL_ONLY));
    registry.record(Check::pass(ids::PER_HEIGHT_DELTAS_MATCH));

    report::build(
        &outcome,
        &registry,
        &ReportContext {
            bundle_id: "mainnet-3428142-3428147".to_owned(),
            tool_version: "0.1.0".to_owned(),
            network: Network::Mainnet,
            reported_end_orchard: Some(Zatoshi::from_raw(365_999_999_995_000)),
            reported_end_ironwood: Some(Zatoshi::from_raw(5_000)),
        },
    )
    .unwrap()
}

#[test]
fn the_same_reconciliation_yields_an_identical_hash() {
    let (first_bytes, first_hash) = report::canonical_bytes_and_hash(&build_report()).unwrap();
    let (second_bytes, second_hash) = report::canonical_bytes_and_hash(&build_report()).unwrap();

    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first_hash, second_hash);
}

#[test]
fn the_hash_survives_a_change_of_working_directory() {
    let (_, before) = report::canonical_bytes_and_hash(&build_report()).unwrap();

    let original = std::env::current_dir().unwrap();
    std::env::set_current_dir(std::env::temp_dir()).unwrap();
    let (_, inside_temp) = report::canonical_bytes_and_hash(&build_report()).unwrap();
    std::env::set_current_dir(original).unwrap();

    assert_eq!(before, inside_temp);
}

#[test]
fn the_canonical_report_contains_no_machine_dependent_fields() {
    let (bytes, _) = report::canonical_bytes_and_hash(&build_report()).unwrap();
    let text = String::from_utf8(bytes).unwrap();

    for forbidden in [
        "created_at",
        "timestamp",
        "generated_at",
        "duration",
        "elapsed",
        "hostname",
        "path",
        "/home/",
        "/tmp/",
    ] {
        assert!(
            !text.contains(forbidden),
            "canonical report contains machine-dependent content: {forbidden}"
        );
    }
}

#[test]
fn every_monetary_value_is_a_string() {
    let (bytes, _) = report::canonical_bytes_and_hash(&build_report()).unwrap();
    let text = String::from_utf8(bytes).unwrap();

    // A zatoshi field followed by a bare number would be a precision hazard under RFC 8785,
    // which canonicalizes JSON numbers through IEEE-754 doubles.
    for fragment in [
        r#"_zatoshis":3"#,
        r#"_zatoshis":-"#,
        r#"_zatoshis":0,"#,
        r#"_zatoshis":1"#,
    ] {
        assert!(
            !text.contains(fragment),
            "a monetary value was serialized as a JSON number: {fragment}"
        );
    }

    assert!(
        text.contains(r#""ironwood_balance_zatoshis":"0""#),
        "{text}"
    );
}

#[test]
fn a_report_round_trips_through_its_canonical_form() {
    let original = build_report();
    let (bytes, hash) = report::canonical_bytes_and_hash(&original).unwrap();

    let parsed = report::from_json_bytes(&bytes).unwrap();
    assert_eq!(parsed, original);

    let (reserialized, rehash) = report::canonical_bytes_and_hash(&parsed).unwrap();
    assert_eq!(reserialized, bytes);
    assert_eq!(rehash, hash);
}

#[test]
fn any_change_to_the_result_changes_the_hash() {
    let (_, original) = report::canonical_bytes_and_hash(&build_report()).unwrap();

    let mut altered = build_report();
    altered.reconstructed.ironwood_delta_zatoshis = Zatoshi::from_raw(4_999);
    let (_, changed) = report::canonical_bytes_and_hash(&altered).unwrap();

    assert_ne!(original, changed);
}

#[test]
fn markdown_and_json_agree_on_every_headline_figure() {
    let report = build_report();
    let markdown = report::markdown::render(&report);
    let (bytes, _) = report::canonical_bytes_and_hash(&report).unwrap();
    let parsed = report::from_json_bytes(&bytes).unwrap();

    for figure in [
        parsed.reconstructed.orchard_delta_zatoshis.to_string(),
        parsed.reconstructed.ironwood_delta_zatoshis.to_string(),
        parsed
            .reconstructed
            .orchard_expected_end_zatoshis
            .to_string(),
        parsed
            .reconstructed
            .ironwood_expected_end_zatoshis
            .to_string(),
        parsed.anchor.orchard_balance_zatoshis.to_string(),
        parsed.interval.end_height.to_string(),
        parsed.bundle_id.clone(),
    ] {
        assert!(
            markdown.contains(&figure),
            "markdown omits a figure present in the JSON report: {figure}"
        );
    }
}

#[test]
fn every_limitation_appears_in_both_artifacts() {
    let report = build_report();
    let markdown = report::markdown::render(&report);
    let (bytes, _) = report::canonical_bytes_and_hash(&report).unwrap();
    let text = String::from_utf8(bytes).unwrap();

    for limitation in &report.limitations {
        assert!(
            markdown.contains(limitation),
            "markdown omits: {limitation}"
        );
        assert!(
            text.contains(limitation.trim_end_matches('.')),
            "json omits: {limitation}"
        );
    }
}

#[test]
fn the_hash_is_lowercase_hexadecimal_of_the_expected_width() {
    let (_, hash) = report::canonical_bytes_and_hash(&build_report()).unwrap();
    assert_eq!(hash.len(), 64);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!hash.chars().any(|c| c.is_ascii_uppercase()));
}

/// Pins the canonical serialization against a committed golden file.
///
/// Any change to the report schema, to field naming, or to the canonicalization procedure
/// alters these bytes. That is not necessarily wrong, but it is always significant: a
/// published report hash would no longer be reproducible by this build. The golden file
/// forces such a change to be deliberate and to be accompanied by a schema version
/// increment, rather than shifting silently.
///
/// Paths are resolved from `CARGO_MANIFEST_DIR` rather than the working directory, so this
/// test is unaffected by any other test changing it.
#[test]
fn the_canonical_serialization_matches_the_committed_golden_file() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let expected_bytes = std::fs::read(root.join("tests/fixtures/golden-report.json"))
        .expect("golden report fixture should be committed");
    let expected_hash = std::fs::read_to_string(root.join("tests/fixtures/golden-report.sha256"))
        .expect("golden report hash should be committed");

    let (bytes, hash) = report::canonical_bytes_and_hash(&build_report()).unwrap();

    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        String::from_utf8(expected_bytes).unwrap(),
        "canonical serialization changed; if deliberate, regenerate the golden file and \
         increment the report schema version"
    );
    assert_eq!(hash, expected_hash.trim());
}
