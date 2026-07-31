//! Assembly of a canonical report from a completed reconciliation.
//!
//! The builder takes only values that are themselves derived from the evidence bundle. It
//! reads no clock, no environment and no filesystem, so a report cannot acquire a
//! machine-dependent field by accident.

use crate::checks::CheckRegistry;
use crate::domain::network::Network;
use crate::domain::pool::Pool;
use crate::domain::zatoshi::Zatoshi;
use crate::error::ReconcileError;
use crate::reconcile::interval::IntervalOutcome;
use crate::report::schema::{
    HeightRow, PerHeightSummary, REPORT_SCHEMA_VERSION, Reconstructed, Report, ReportAnchor,
    ReportInterval, Reported, TurnstileObserved,
};

/// Identity of the bundle a report describes, taken from its manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportContext {
    pub bundle_id: String,
    pub tool_version: String,
    pub network: Network,
    pub reported_end_orchard: Option<Zatoshi>,
    pub reported_end_ironwood: Option<Zatoshi>,
}

/// Builds the canonical report.
///
/// The check registry is sorted into canonical order here rather than by the caller, so
/// that report ordering cannot depend on the sequence in which checks were evaluated.
pub fn build(
    outcome: &IntervalOutcome,
    registry: &CheckRegistry,
    context: &ReportContext,
) -> Result<Report, ReconcileError> {
    let mut ordered = registry.clone();
    ordered.sort_canonically();

    let per_height: Vec<HeightRow> = outcome
        .heights
        .iter()
        .map(|height| HeightRow {
            height: height.height,
            orchard_delta_zatoshis: height.reconstructed_orchard_delta,
            ironwood_delta_zatoshis: height.reconstructed_ironwood_delta,
            orchard_expected_balance_zatoshis: height.expected_orchard_balance,
            ironwood_expected_balance_zatoshis: height.expected_ironwood_balance,
            orchard_reported_balance_zatoshis: height.reported_orchard_balance,
            ironwood_reported_balance_zatoshis: height.reported_ironwood_balance,
            orchard_balance_agreement: height.balance_agreement(Pool::Orchard),
            ironwood_balance_agreement: height.balance_agreement(Pool::Ironwood),
            orchard_delta_agreement: height.delta_agreement(Pool::Orchard),
            ironwood_delta_agreement: height.delta_agreement(Pool::Ironwood),
        })
        .collect();

    let diverging: Vec<&HeightRow> = per_height.iter().filter(|row| row.diverges()).collect();

    let summary = PerHeightSummary {
        heights_compared: u32::try_from(per_height.len()).map_err(|_| {
            ReconcileError::Internal {
                reason: "height count exceeds the representable range".to_owned(),
            }
        })?,
        heights_diverging: u32::try_from(diverging.len()).map_err(|_| {
            ReconcileError::Internal {
                reason: "divergence count exceeds the representable range".to_owned(),
            }
        })?,
        first_diverging_height: diverging.first().map(|row| row.height),
    };

    Ok(Report {
        report_schema_version: REPORT_SCHEMA_VERSION.to_owned(),
        tool_version: context.tool_version.clone(),
        bundle_id: context.bundle_id.clone(),
        network: context.network,
        interval: ReportInterval {
            anchor_height: outcome.interval.anchor_height(),
            start_height: outcome.interval.start_height(),
            end_height: outcome.interval.end_height(),
            block_count: outcome.interval.block_count(),
        },
        anchor: ReportAnchor {
            orchard_balance_zatoshis: outcome.anchor.orchard,
            ironwood_balance_zatoshis: outcome.anchor.ironwood,
        },
        reconstructed: Reconstructed {
            orchard_delta_zatoshis: outcome.cumulative_orchard_delta,
            ironwood_delta_zatoshis: outcome.cumulative_ironwood_delta,
            orchard_expected_end_zatoshis: outcome.expected_end_orchard,
            ironwood_expected_end_zatoshis: outcome.expected_end_ironwood,
        },
        reported: Reported {
            orchard_end_zatoshis: context.reported_end_orchard,
            ironwood_end_zatoshis: context.reported_end_ironwood,
        },
        turnstile_observed: TurnstileObserved {
            orchard_outflow_zatoshis: outcome.turnstile.orchard_outflow,
            ironwood_inflow_zatoshis: outcome.turnstile.ironwood_inflow,
        },
        per_height_summary: summary,
        per_height,
        checks: ordered.checks().to_vec(),
        overall_status: ordered.overall_status(),
        limitations: Report::standard_limitations(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::{Check, ids};
    use crate::domain::height::{BlockHeight, HeightInterval};
    use crate::domain::pool_state::ReportedPoolState;
    use crate::reconcile::interval::{AnchorBalances, reconcile_interval};
    use crate::reconcile::ledger::BlockLedger;
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

    fn context() -> ReportContext {
        ReportContext {
            bundle_id: "mainnet-3428142-3428144".to_owned(),
            tool_version: "0.1.0".to_owned(),
            network: Network::Mainnet,
            reported_end_orchard: Some(Zatoshi::from_raw(500)),
            reported_end_ironwood: Some(Zatoshi::from_raw(500)),
        }
    }

    fn outcome(reported: BTreeMap<BlockHeight, ReportedPoolState>) -> IntervalOutcome {
        let ledgers = vec![ledger(3_428_143, -300, 300), ledger(3_428_144, -200, 200)];
        let interval =
            HeightInterval::new(BlockHeight::new(3_428_143), BlockHeight::new(3_428_144)).unwrap();
        reconcile_interval(
            &ledgers,
            interval,
            AnchorBalances {
                orchard: Zatoshi::from_raw(1_000),
                ironwood: Zatoshi::ZERO,
            },
            &reported,
        )
        .unwrap()
    }

    fn registry() -> CheckRegistry {
        let mut registry = CheckRegistry::new();
        registry.record(Check::pass(ids::NETWORK_MATCHES));
        registry.record(Check::pass(ids::EVIDENCE_HASHES_VALID));
        registry
    }

    #[test]
    fn a_report_carries_the_reconstruction_and_the_reported_figures() {
        let report = build(&outcome(BTreeMap::new()), &registry(), &context()).unwrap();

        assert_eq!(
            report.reconstructed.orchard_delta_zatoshis,
            Zatoshi::from_raw(-500)
        );
        assert_eq!(
            report.reconstructed.orchard_expected_end_zatoshis,
            Zatoshi::from_raw(500)
        );
        assert_eq!(
            report.reported.orchard_end_zatoshis,
            Some(Zatoshi::from_raw(500))
        );
    }

    #[test]
    fn every_height_appears_as_a_row() {
        let report = build(&outcome(BTreeMap::new()), &registry(), &context()).unwrap();
        assert_eq!(report.per_height.len(), 2);
        assert_eq!(report.per_height_summary.heights_compared, 2);
        assert_eq!(report.per_height_summary.heights_diverging, 0);
        assert_eq!(report.per_height_summary.first_diverging_height, None);
    }

    #[test]
    fn the_summary_names_the_first_diverging_height() {
        let mut reported = BTreeMap::new();
        reported.insert(
            BlockHeight::new(3_428_144),
            ReportedPoolState::new(BlockHeight::new(3_428_144))
                .with_delta(Pool::Orchard, Zatoshi::from_raw(-999)),
        );

        let report = build(&outcome(reported), &registry(), &context()).unwrap();
        assert_eq!(report.per_height_summary.heights_diverging, 1);
        assert_eq!(
            report.per_height_summary.first_diverging_height,
            Some(BlockHeight::new(3_428_144))
        );
    }

    #[test]
    fn checks_appear_in_canonical_order_regardless_of_evaluation_order() {
        let mut forward = CheckRegistry::new();
        forward.record(Check::pass(ids::NETWORK_MATCHES));
        forward.record(Check::pass(ids::EVIDENCE_HASHES_VALID));

        let mut reverse = CheckRegistry::new();
        reverse.record(Check::pass(ids::EVIDENCE_HASHES_VALID));
        reverse.record(Check::pass(ids::NETWORK_MATCHES));

        let first = build(&outcome(BTreeMap::new()), &forward, &context()).unwrap();
        let second = build(&outcome(BTreeMap::new()), &reverse, &context()).unwrap();
        assert_eq!(first.checks, second.checks);
        assert_eq!(first, second);
    }

    #[test]
    fn every_report_carries_the_standard_limitations() {
        let report = build(&outcome(BTreeMap::new()), &registry(), &context()).unwrap();
        assert_eq!(
            report.limitations,
            crate::report::schema::LIMITATIONS
                .iter()
                .map(|text| (*text).to_owned())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_absent_reported_balance_stays_absent_rather_than_becoming_zero() {
        let mut context = context();
        context.reported_end_ironwood = None;
        let report = build(&outcome(BTreeMap::new()), &registry(), &context).unwrap();
        assert_eq!(report.reported.ironwood_end_zatoshis, None);
    }

    #[test]
    fn turnstile_flows_are_carried_as_observations() {
        let report = build(&outcome(BTreeMap::new()), &registry(), &context()).unwrap();
        assert_eq!(
            report.turnstile_observed.orchard_outflow_zatoshis,
            Zatoshi::from_raw(500)
        );
        assert_eq!(
            report.turnstile_observed.ironwood_inflow_zatoshis,
            Zatoshi::from_raw(500)
        );
    }

    #[test]
    fn building_the_same_reconciliation_twice_yields_an_identical_report() {
        let first = build(&outcome(BTreeMap::new()), &registry(), &context()).unwrap();
        let second = build(&outcome(BTreeMap::new()), &registry(), &context()).unwrap();
        assert_eq!(first, second);
    }
}
