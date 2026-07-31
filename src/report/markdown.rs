//! Human-readable rendering of a canonical report.
//!
//! Markdown is rendered **from** the [`Report`] struct and never recomputes anything. There
//! is exactly one accounting path in this crate; a second one that happened to agree most
//! of the time would be worse than none, because a disagreement between the two documents
//! would surface as a contradiction rather than as an error.

use std::fmt::Write as _;

use crate::checks::Status;
use crate::domain::zatoshi::Zatoshi;
use crate::report::schema::{HeightRow, Report};

/// Number of per-height rows rendered in full before the table is elided.
///
/// The complete series is always present in the JSON report; Markdown is a reading aid, and
/// a thousand-row table serves no reader. Diverging heights are always shown.
const MAX_RENDERED_ROWS: usize = 25;

/// Renders a report as Markdown.
pub fn render(report: &Report) -> String {
    let mut out = String::new();

    render_header(&mut out, report);
    render_summary(&mut out, report);
    render_reconciliation(&mut out, report);
    render_turnstile(&mut out, report);
    render_per_height(&mut out, report);
    render_checks(&mut out, report);
    render_limitations(&mut out, report);

    out
}

fn render_header(out: &mut String, report: &Report) {
    let _ = writeln!(out, "# Value-pool reconciliation: {}", report.bundle_id);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Reconstructed from transaction-level public data and compared against the balances \
         reported by the capturing node."
    );
    let _ = writeln!(out);
}

fn render_summary(out: &mut String, report: &Report) {
    let _ = writeln!(out, "## Summary");
    let _ = writeln!(out);
    let _ = writeln!(out, "| Field | Value |");
    let _ = writeln!(out, "| --- | --- |");
    let _ = writeln!(
        out,
        "| Overall status | **{}** |",
        status_text(report.overall_status)
    );
    let _ = writeln!(out, "| Network | {} |", report.network);
    let _ = writeln!(out, "| Anchor height | {} |", report.interval.anchor_height);
    let _ = writeln!(out, "| Start height | {} |", report.interval.start_height);
    let _ = writeln!(out, "| End height | {} |", report.interval.end_height);
    let _ = writeln!(out, "| Block count | {} |", report.interval.block_count);
    let _ = writeln!(out, "| Tool version | {} |", report.tool_version);
    let _ = writeln!(out, "| Report schema | {} |", report.report_schema_version);
    let _ = writeln!(out);
}

fn render_reconciliation(out: &mut String, report: &Report) {
    let _ = writeln!(out, "## Reconciliation");
    let _ = writeln!(out);
    let _ = writeln!(out, "All values are integer zatoshi.");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| Pool | Anchor | Reconstructed change | Expected end | Reported end |"
    );
    let _ = writeln!(out, "| --- | ---: | ---: | ---: | ---: |");
    let _ = writeln!(
        out,
        "| Orchard | {} | {} | {} | {} |",
        report.anchor.orchard_balance_zatoshis,
        report.reconstructed.orchard_delta_zatoshis,
        report.reconstructed.orchard_expected_end_zatoshis,
        optional(report.reported.orchard_end_zatoshis),
    );
    let _ = writeln!(
        out,
        "| Ironwood | {} | {} | {} | {} |",
        report.anchor.ironwood_balance_zatoshis,
        report.reconstructed.ironwood_delta_zatoshis,
        report.reconstructed.ironwood_expected_end_zatoshis,
        optional(report.reported.ironwood_end_zatoshis),
    );
    let _ = writeln!(out);
}

fn render_turnstile(out: &mut String, report: &Report) {
    let _ = writeln!(out, "## Turnstile flow (observed)");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Reported as observations. No inequality between these figures is asserted."
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Value that left the Orchard pool: {} zatoshi",
        report.turnstile_observed.orchard_outflow_zatoshis
    );
    let _ = writeln!(
        out,
        "- Value that entered the Ironwood pool: {} zatoshi",
        report.turnstile_observed.ironwood_inflow_zatoshis
    );
    let _ = writeln!(out);
}

fn render_per_height(out: &mut String, report: &Report) {
    let summary = &report.per_height_summary;
    let _ = writeln!(out, "## Per-height comparison");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "{} height(s) compared on two axes each: the running balance and the per-block delta.",
        summary.heights_compared
    );
    let _ = writeln!(out);

    match summary.first_diverging_height {
        None => {
            let _ = writeln!(out, "No divergence was found at any height.");
        }
        Some(height) => {
            let _ = writeln!(
                out,
                "**{} height(s) diverge. The first is {height}.**",
                summary.heights_diverging
            );
        }
    }
    let _ = writeln!(out);

    let diverging: Vec<&HeightRow> = report
        .per_height
        .iter()
        .filter(|row| row.diverges())
        .collect();
    let rendered: Vec<&HeightRow> = if diverging.is_empty() {
        report.per_height.iter().take(MAX_RENDERED_ROWS).collect()
    } else {
        diverging.into_iter().take(MAX_RENDERED_ROWS).collect()
    };

    if rendered.is_empty() {
        return;
    }

    let _ = writeln!(
        out,
        "| Height | Orchard delta | Ironwood delta | Orchard balance | Ironwood balance |"
    );
    let _ = writeln!(out, "| ---: | ---: | ---: | ---: | ---: |");
    for row in &rendered {
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} |",
            row.height,
            row.orchard_delta_zatoshis,
            row.ironwood_delta_zatoshis,
            row.orchard_expected_balance_zatoshis,
            row.ironwood_expected_balance_zatoshis,
        );
    }

    if report.per_height.len() > rendered.len() {
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Showing {} of {} heights. The complete series is in the JSON report.",
            rendered.len(),
            report.per_height.len()
        );
    }
    let _ = writeln!(out);
}

fn render_checks(out: &mut String, report: &Report) {
    let _ = writeln!(out, "## Checks");
    let _ = writeln!(out);
    let _ = writeln!(out, "| Check | Result | Details |");
    let _ = writeln!(out, "| --- | --- | --- |");
    for check in &report.checks {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} |",
            check.id,
            status_text(check.status),
            check.details.as_deref().unwrap_or("")
        );
    }
    let _ = writeln!(out);
}

fn render_limitations(out: &mut String, report: &Report) {
    let _ = writeln!(out, "## Limitations");
    let _ = writeln!(out);
    for limitation in &report.limitations {
        let _ = writeln!(out, "- {limitation}");
    }
    let _ = writeln!(out);
}

const fn status_text(status: Status) -> &'static str {
    match status {
        Status::Pass => "PASS",
        Status::Fail => "FAIL",
        Status::Warn => "WARN",
        Status::NotApplicable => "N/A",
    }
}

fn optional(value: Option<Zatoshi>) -> String {
    value.map_or_else(|| "not reported".to_owned(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{Check, CheckRegistry, ids};
    use crate::domain::height::{BlockHeight, HeightInterval};
    use crate::domain::network::Network;
    use crate::domain::pool::Pool;
    use crate::domain::pool_state::ReportedPoolState;
    use crate::reconcile::interval::{AnchorBalances, reconcile_interval};
    use crate::reconcile::ledger::BlockLedger;
    use crate::report::builder::{self, ReportContext};
    use std::collections::BTreeMap;

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

    fn report_over(
        ledgers: Vec<BlockLedger>,
        reported: BTreeMap<BlockHeight, ReportedPoolState>,
    ) -> Report {
        let start = ledgers.first().unwrap().height;
        let end = ledgers.last().unwrap().height;
        let outcome = reconcile_interval(
            &ledgers,
            HeightInterval::new(start, end).unwrap(),
            AnchorBalances {
                orchard: Zatoshi::from_raw(1_000),
                ironwood: Zatoshi::ZERO,
            },
            &reported,
        )
        .unwrap();

        let mut registry = CheckRegistry::new();
        registry.record(Check::pass(ids::NETWORK_MATCHES));
        registry.record(Check::fail(ids::EVIDENCE_HASHES_VALID, "digest mismatch"));

        builder::build(
            &outcome,
            &registry,
            &ReportContext {
                bundle_id: "mainnet-3428142-3428144".to_owned(),
                tool_version: "0.1.0".to_owned(),
                network: Network::Mainnet,
                reported_end_orchard: Some(Zatoshi::from_raw(500)),
                reported_end_ironwood: Some(Zatoshi::from_raw(500)),
            },
        )
        .unwrap()
    }

    fn sample() -> Report {
        report_over(
            vec![ledger(3_428_143, -300, 300), ledger(3_428_144, -200, 200)],
            BTreeMap::new(),
        )
    }

    #[test]
    fn rendering_is_deterministic() {
        let report = sample();
        assert_eq!(render(&report), render(&report));
    }

    #[test]
    fn every_figure_matches_the_report_struct() {
        let report = sample();
        let markdown = render(&report);

        assert!(markdown.contains(&report.reconstructed.orchard_delta_zatoshis.to_string()));
        assert!(
            markdown.contains(
                &report
                    .reconstructed
                    .orchard_expected_end_zatoshis
                    .to_string()
            )
        );
        assert!(markdown.contains(&report.bundle_id));
        assert!(markdown.contains(&report.interval.end_height.to_string()));
    }

    #[test]
    fn every_check_appears_with_its_stable_identifier() {
        let report = sample();
        let markdown = render(&report);
        for check in &report.checks {
            assert!(
                markdown.contains(&check.id),
                "check {} missing from markdown",
                check.id
            );
        }
    }

    #[test]
    fn every_limitation_appears_verbatim() {
        let report = sample();
        let markdown = render(&report);
        for limitation in &report.limitations {
            assert!(markdown.contains(limitation), "missing: {limitation}");
        }
    }

    #[test]
    fn a_failing_report_renders_a_fail_status() {
        let report = sample();
        let markdown = render(&report);
        assert_eq!(report.overall_status, Status::Fail);
        assert!(markdown.contains("**FAIL**"));
    }

    #[test]
    fn an_unreported_balance_renders_as_not_reported_rather_than_zero() {
        let mut report = sample();
        report.reported.ironwood_end_zatoshis = None;
        let markdown = render(&report);
        assert!(markdown.contains("not reported"));
    }

    #[test]
    fn diverging_heights_are_named_in_the_rendering() {
        let mut reported = BTreeMap::new();
        reported.insert(
            BlockHeight::new(3_428_144),
            ReportedPoolState::new(BlockHeight::new(3_428_144))
                .with_delta(Pool::Orchard, Zatoshi::from_raw(-999)),
        );
        let report = report_over(
            vec![ledger(3_428_143, -300, 300), ledger(3_428_144, -200, 200)],
            reported,
        );

        let markdown = render(&report);
        assert!(markdown.contains("3428144"));
        assert!(markdown.contains("diverge"));
    }

    #[test]
    fn a_long_interval_elides_rows_but_states_that_it_did() {
        let ledgers: Vec<_> = (0..60).map(|i| ledger(3_428_143 + i, -1, 1)).collect();
        let report = report_over(ledgers, BTreeMap::new());
        let markdown = render(&report);

        assert!(markdown.contains("Showing 25 of 60 heights"));
        assert!(markdown.contains("complete series is in the JSON report"));
    }

    #[test]
    fn turnstile_figures_are_labelled_as_observations() {
        let markdown = render(&sample());
        assert!(markdown.contains("observed") || markdown.contains("observations"));
        assert!(markdown.contains("No inequality between these figures is asserted."));
    }
}
