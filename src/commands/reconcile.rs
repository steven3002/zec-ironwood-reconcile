//! `reconcile` — reconstruct pool changes from an evidence bundle.
//!
//! This is the accounting path, and `verify` runs exactly the same function over an
//! extracted archive. There is deliberately no second implementation: if reconciliation and
//! verification could disagree, the report hash a third party reproduces would mean nothing.
//!
//! # The bundle is read, never written
//!
//! Reconciliation opens the evidence read-only and returns the report to its caller. A
//! bundle's digests therefore cannot be disturbed by reconciling it, and reconciling the
//! same bundle twice — or a hundred times, in the course of verifying it — leaves it
//! byte-identical.
//!
//! # Order
//!
//! Evidence integrity is established before any block is parsed, because a reconciliation
//! over altered evidence produces a number rather than a finding. Continuity is established
//! before accumulation, because summing deltas over blocks that do not form an unbroken
//! chain from the declared anchor is arithmetic without meaning.

use std::collections::BTreeMap;
use std::path::Path;

use crate::canonical;
use crate::checks::{Check, CheckRegistry, accounting, activation, ids, structural};
use crate::domain::height::{BlockHeight, HeightInterval};
use crate::domain::pool::Pool;
use crate::domain::pool_state::ReportedPoolState;
use crate::domain::zatoshi::Zatoshi;
use crate::error::ReconcileError;
use crate::evidence::layout;
use crate::evidence::manifest::Manifest;
use crate::evidence::pool_state_file;
use crate::evidence::validation::{self, ValidationWarning};
use crate::parse::block;
use crate::reconcile::continuity::{self, ChainEndpoints};
use crate::reconcile::interval::{AnchorBalances, IntervalOutcome, reconcile_interval};
use crate::reconcile::ledger::BlockLedger;
use crate::report::builder::{self, ReportContext};
use crate::report::markdown;
use crate::report::schema::Report;

/// A completed reconciliation.
#[derive(Debug, Clone)]
pub struct Reconciliation {
    pub report: Report,
    /// RFC 8785 serialization of the report; the bytes the hash is taken over.
    pub canonical_bytes: Vec<u8>,
    pub report_hash: String,
    pub outcome: IntervalOutcome,
    pub warnings: Vec<ValidationWarning>,
}

impl Reconciliation {
    pub fn markdown(&self) -> String {
        markdown::render(&self.report)
    }
}

/// Reconciles a bundle directory.
pub fn reconcile(bundle_root: &Path) -> Result<Reconciliation, ReconcileError> {
    let manifest = validation::load_manifest(bundle_root)?;
    reconcile_with_manifest(bundle_root, &manifest)
}

/// Reconciles a bundle whose manifest has already been loaded and validated.
///
/// `verify` uses this so the manifest is read once rather than twice, and so the same
/// manifest instance drives both integrity verification and accounting.
pub fn reconcile_with_manifest(
    bundle_root: &Path,
    manifest: &Manifest,
) -> Result<Reconciliation, ReconcileError> {
    // Hashed once. Structural checks read the same report the gate below acts on, so the
    // verdict a report carries and the decision to produce one cannot disagree.
    let validation_report = validation::validate_bundle(bundle_root, manifest);
    if !validation_report.is_valid() {
        return Err(validation_report
            .into_result()
            .err()
            .unwrap_or(ReconcileError::Internal {
                reason: "evidence validation failed without recording a failure".to_owned(),
            }));
    }
    let warnings = validation_report.warnings.clone();

    let interval =
        HeightInterval::new(manifest.interval.start_height, manifest.interval.end_height)?;
    let network = manifest.network;

    let anchor_block = read_anchor_block(bundle_root, manifest)?;
    let anchor_state = read_pool_state(bundle_root, layout::ANCHOR_VALUE_POOLS)?;

    let mut ledgers = Vec::with_capacity(interval.block_count() as usize);
    let mut reported: BTreeMap<BlockHeight, ReportedPoolState> = BTreeMap::new();

    for height in interval.heights() {
        let hex = read_text(bundle_root, &layout::block(height))?;
        let parsed = block::parse_block_hex(&hex, network, height)?;
        ledgers.push(BlockLedger::from_parsed(&parsed)?);

        let state = read_pool_state(bundle_root, &layout::block_value_pools(height))?;
        reported.insert(height, state.pools);
    }

    // The anchor hash is the one this crate computed from the anchor block's own bytes, not
    // the one the manifest asserts, so linkage rests on evidence rather than on a claim. The
    // end hash is deliberately the manifest's claim, so that a manifest disagreeing with the
    // blocks it indexes is caught rather than believed.
    let endpoints = ChainEndpoints {
        anchor_block_hash: anchor_block
            .clone()
            .unwrap_or_else(|| manifest.anchor.block_hash.clone()),
        end_block_hash: manifest.end.block_hash.clone(),
    };
    let continuity_result = continuity::verify(&ledgers, interval, &endpoints);

    // Anchor balances come from the evidence, not from the manifest. The manifest is written
    // by whoever produced the bundle and its figures are a summary; the anchor is where every
    // later balance comes from, so it is taken from what the node actually said and the
    // summary is checked against it.
    let anchor = AnchorBalances {
        orchard: anchor_state.pools.require_balance(Pool::Orchard)?,
        ironwood: anchor_state.pools.require_balance(Pool::Ironwood)?,
    };
    let outcome = reconcile_interval(&ledgers, interval, anchor, &reported)?;

    // Likewise the figures the reconstruction is compared against at the end height: taking
    // them from the manifest would compare a manifest-derived number with another
    // manifest-derived number and call the agreement a result.
    let end_state = reported.get(&interval.end_height());
    let reported_end_orchard = end_state.and_then(|state| state.balance(Pool::Orchard));
    let reported_end_ironwood = end_state.and_then(|state| state.balance(Pool::Ironwood));

    let mut registry = CheckRegistry::new();
    structural::evaluate(
        manifest,
        &validation_report,
        network,
        anchor_block.is_some(),
        continuity_result.as_ref().map(|_| ()),
        &mut registry,
    );
    record_manifest_agreement(
        manifest,
        anchor,
        reported_end_orchard,
        reported_end_ironwood,
        &mut registry,
    );
    activation::evaluate(
        &outcome,
        network,
        Some(manifest.activation.expected_height.get()),
        &mut registry,
    );
    accounting::evaluate(
        &outcome,
        reported_end_orchard,
        reported_end_ironwood,
        &manifest.end.tracking,
        &mut registry,
    );
    registry.record(Check::pass(ids::CANONICAL_REPORT_GENERATED));

    let context = ReportContext {
        bundle_id: manifest.bundle_id.clone(),
        tool_version: manifest.tool.version.clone(),
        network,
        reported_end_orchard,
        reported_end_ironwood,
    };

    let report = builder::build(&outcome, &registry, &context)?;
    let (canonical_bytes, report_hash) = canonical::to_canonical_bytes_and_hash(&report)?;

    Ok(Reconciliation {
        report,
        canonical_bytes,
        report_hash,
        outcome,
        warnings,
    })
}

/// Confirms the manifest's summary figures are the ones the evidence records.
///
/// Nothing downstream depends on the manifest's balances — the arithmetic uses the evidence
/// directly — so this check exists to surface a disagreement rather than to guard against
/// one. A manifest that misstates its own bundle is worth knowing about even when it changes
/// no number, because it is the part of a bundle a reader is most likely to skim.
fn record_manifest_agreement(
    manifest: &Manifest,
    anchor: AnchorBalances,
    reported_end_orchard: Option<Zatoshi>,
    reported_end_ironwood: Option<Zatoshi>,
    registry: &mut CheckRegistry,
) {
    let mut mismatches = Vec::new();

    let mut compare = |field: &str, declared: Zatoshi, recorded: Option<Zatoshi>| match recorded {
        Some(value) if value == declared => {}
        Some(value) => mismatches.push(format!(
            "manifest states {field} {declared}, evidence records {value}"
        )),
        None => mismatches.push(format!(
            "manifest states {field} {declared}, evidence records no value"
        )),
    };

    compare(
        "anchor orchard balance",
        manifest.anchor.orchard_balance_zatoshis,
        Some(anchor.orchard),
    );
    compare(
        "anchor ironwood balance",
        manifest.anchor.ironwood_balance_zatoshis,
        Some(anchor.ironwood),
    );
    compare(
        "reported orchard end balance",
        manifest.end.reported_orchard_balance_zatoshis,
        reported_end_orchard,
    );
    compare(
        "reported ironwood end balance",
        manifest.end.reported_ironwood_balance_zatoshis,
        reported_end_ironwood,
    );

    registry.record_condition(
        ids::MANIFEST_MATCHES_EVIDENCE,
        mismatches.is_empty(),
        mismatches.join("; "),
    );
}

/// Reads the anchor block and returns the hash computed from its bytes.
///
/// A bundle without an anchor block is not refused outright: continuity then falls back to
/// the manifest's declared hash, and the weaker basis is recorded as a check rather than
/// passed over.
fn read_anchor_block(
    bundle_root: &Path,
    manifest: &Manifest,
) -> Result<Option<String>, ReconcileError> {
    let path = layout::resolve(bundle_root, layout::ANCHOR_BLOCK)?;
    if !path.is_file() {
        return Ok(None);
    }

    let hex = read_text(bundle_root, layout::ANCHOR_BLOCK)?;
    let parsed = block::parse_block_hex(&hex, manifest.network, manifest.interval.anchor_height)?;
    Ok(Some(parsed.block_hash))
}

fn read_text(bundle_root: &Path, relative: &str) -> Result<String, ReconcileError> {
    let path = layout::resolve(bundle_root, relative)?;
    std::fs::read_to_string(&path).map_err(|source| ReconcileError::Filesystem {
        path: path.display().to_string(),
        source,
    })
}

fn read_pool_state(
    bundle_root: &Path,
    relative: &str,
) -> Result<pool_state_file::CapturedBlockState, ReconcileError> {
    let path = layout::resolve(bundle_root, relative)?;
    let bytes = std::fs::read(&path).map_err(|source| ReconcileError::Filesystem {
        path: path.display().to_string(),
        source,
    })?;
    pool_state_file::parse(&bytes)
}

/// Writes the three report artifacts into a directory.
///
/// `report.json` holds the canonical bytes exactly as hashed, so a reader can recompute the
/// digest from the file without re-serializing anything.
pub fn write_reports(reconciliation: &Reconciliation, output: &Path) -> Result<(), ReconcileError> {
    std::fs::create_dir_all(output).map_err(|source| ReconcileError::Filesystem {
        path: output.display().to_string(),
        source,
    })?;

    let write = |name: &str, contents: &[u8]| -> Result<(), ReconcileError> {
        let path = output.join(name);
        std::fs::write(&path, contents).map_err(|source| ReconcileError::Filesystem {
            path: path.display().to_string(),
            source,
        })
    };

    write("report.json", &reconciliation.canonical_bytes)?;
    write("report.md", reconciliation.markdown().as_bytes())?;
    write(
        "report.sha256",
        format!("{}  report.json\n", reconciliation.report_hash).as_bytes(),
    )
}

/// Renders a concise terminal summary.
pub fn render(reconciliation: &Reconciliation) -> String {
    use std::fmt::Write as _;

    let report = &reconciliation.report;
    let mut out = String::new();
    let mut line = |label: &str, value: &str| {
        let _ = writeln!(out, "{label:<26} {value}");
    };

    line("Bundle id:", &report.bundle_id);
    line("Network:", report.network.name());
    line(
        "Interval:",
        &format!(
            "{}..={} ({} blocks)",
            report.interval.start_height, report.interval.end_height, report.interval.block_count
        ),
    );
    line(
        "Orchard reconstructed:",
        &format!(
            "{} (end {})",
            report.reconstructed.orchard_delta_zatoshis,
            report.reconstructed.orchard_expected_end_zatoshis
        ),
    );
    line(
        "Ironwood reconstructed:",
        &format!(
            "{} (end {})",
            report.reconstructed.ironwood_delta_zatoshis,
            report.reconstructed.ironwood_expected_end_zatoshis
        ),
    );
    line(
        "Heights compared:",
        &format!(
            "{} ({} diverging)",
            report.per_height_summary.heights_compared, report.per_height_summary.heights_diverging
        ),
    );
    line("Report hash:", &reconciliation.report_hash);
    line("Overall status:", &format!("{:?}", report.overall_status));

    let notable: Vec<&Check> = report
        .checks
        .iter()
        .filter(|check| check.status != crate::checks::Status::Pass)
        .collect();

    if !notable.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "Checks needing attention:");
        for check in notable {
            let _ = writeln!(
                out,
                "  [{:?}] {} — {}",
                check.status,
                check.id,
                check.details.as_deref().unwrap_or("no detail recorded")
            );
        }
    }

    out
}
