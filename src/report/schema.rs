//! The canonical report schema.
//!
//! A report is a pure function of the evidence bundle and the tool version. Nothing that
//! varies between machines or runs may appear in it: no wall-clock timestamps, no
//! filesystem paths, no hostnames, no durations, no locale-dependent formatting, and no
//! unordered collections. That exclusion is what makes the report hash reproducible by a
//! third party who was not present when the evidence was captured.
//!
//! Measurements that legitimately vary are carried by
//! [`crate::report::performance::PerformanceMetadata`], which is emitted separately and
//! never enters the hashed artifact.

use serde::{Deserialize, Serialize};

use crate::checks::{Check, Status};
use crate::domain::height::BlockHeight;
use crate::domain::network::Network;
use crate::domain::zatoshi::Zatoshi;
use crate::reconcile::interval::Agreement;

/// Report schema version understood by this build.
pub const REPORT_SCHEMA_VERSION: &str = "1.2.0";

/// The boundary of what this tool's output may be cited for.
///
/// These strings are part of the schema. They appear verbatim in every report so that a
/// report cannot circulate separated from its own limitations.
pub const LIMITATIONS: [&str; 9] = [
    "Does not verify zero-knowledge proofs.",
    "Does not independently validate all Zcash consensus rules.",
    "Does not prove whether historical counterfeiting occurred.",
    "Does not prove total Zcash supply from genesis.",
    "Does not prove the Ironwood circuit is sound.",
    "Does not prove that the Orchard pool was never exploited.",
    "Does not replace full-node consensus validation.",
    "Does not constitute a formal security audit.",
    "The Orchard outflow and Ironwood inflow figures are separate observations, not a \
balance. Ironwood also receives newly issued value directly from coinbase transactions, so \
Ironwood inflow exceeding Orchard outflow is expected and is not unexplained supply.",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportInterval {
    pub anchor_height: BlockHeight,
    pub start_height: BlockHeight,
    pub end_height: BlockHeight,
    pub block_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportAnchor {
    pub orchard_balance_zatoshis: Zatoshi,
    pub ironwood_balance_zatoshis: Zatoshi,
}

/// What this crate reconstructed from transaction data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reconstructed {
    pub orchard_delta_zatoshis: Zatoshi,
    pub ironwood_delta_zatoshis: Zatoshi,
    pub orchard_expected_end_zatoshis: Zatoshi,
    pub ironwood_expected_end_zatoshis: Zatoshi,
}

/// What the capturing node reported, which the reconstruction is compared against.
///
/// A value is absent when the node did not report one. Absence is preserved rather than
/// defaulted, so a reader can tell a comparison that failed from one that never happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reported {
    pub orchard_end_zatoshis: Option<Zatoshi>,
    pub ironwood_end_zatoshis: Option<Zatoshi>,
}

/// Cumulative value out of Orchard and into Ironwood, over the interval.
///
/// Two separate observations, deliberately not presented as a balance: Ironwood also
/// receives newly issued value directly from coinbase transactions, so an inflow larger
/// than Orchard's outflow is ordinary rather than unexplained.
///
/// No inequality between these figures is asserted. See `ACCOUNTING_MODEL.md`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolFlowsObserved {
    pub orchard_outflow_zatoshis: Zatoshi,
    pub ironwood_inflow_zatoshis: Zatoshi,
}

/// One height's reconstruction and comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeightRow {
    pub height: BlockHeight,
    pub orchard_delta_zatoshis: Zatoshi,
    pub ironwood_delta_zatoshis: Zatoshi,
    pub orchard_expected_balance_zatoshis: Zatoshi,
    pub ironwood_expected_balance_zatoshis: Zatoshi,
    pub orchard_reported_balance_zatoshis: Option<Zatoshi>,
    pub ironwood_reported_balance_zatoshis: Option<Zatoshi>,
    pub orchard_balance_agreement: Agreement,
    pub ironwood_balance_agreement: Agreement,
    pub orchard_delta_agreement: Agreement,
    pub ironwood_delta_agreement: Agreement,
}

impl HeightRow {
    /// Whether either comparison axis disagreed at this height.
    pub fn diverges(&self) -> bool {
        self.orchard_balance_agreement.differs()
            || self.ironwood_balance_agreement.differs()
            || self.orchard_delta_agreement.differs()
            || self.ironwood_delta_agreement.differs()
    }
}

/// Counts over the per-height comparison, so a reader need not aggregate the rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerHeightSummary {
    pub heights_compared: u32,
    pub heights_diverging: u32,
    pub first_diverging_height: Option<BlockHeight>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub report_schema_version: String,
    /// Version of the tool that **captured** the bundle, as its manifest states.
    ///
    /// This is a claim by whoever produced the bundle, and it describes the capture rather
    /// than the reconciliation. It is not the build that decided any verdict below.
    pub tool_version: String,
    /// Version of the build that **reconciled** the bundle and produced this report.
    ///
    /// Check semantics decide every verdict and therefore the report hash, so two builds can
    /// reconcile one bundle to two different hashes. Without this field a verifier comparing
    /// hashes cannot tell whether a mismatch means the evidence differs or merely that the
    /// builds do, and the only version the report carried was one the bundle's own author
    /// supplied. Compiled in, so it cannot be set by input.
    pub reconciled_by_version: String,
    pub bundle_id: String,
    pub network: Network,
    pub interval: ReportInterval,
    pub anchor: ReportAnchor,
    pub reconstructed: Reconstructed,
    pub reported: Reported,
    pub pool_flows_observed: PoolFlowsObserved,
    pub per_height_summary: PerHeightSummary,
    pub per_height: Vec<HeightRow>,
    pub checks: Vec<Check>,
    pub overall_status: Status,
    pub limitations: Vec<String>,
}

impl Report {
    /// The limitations every report carries.
    pub fn standard_limitations() -> Vec<String> {
        LIMITATIONS.iter().map(|text| (*text).to_owned()).collect()
    }

    pub fn has_failures(&self) -> bool {
        self.checks.iter().any(|check| check.status == Status::Fail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limitations_are_distinct_and_non_empty() {
        let mut sorted = LIMITATIONS.to_vec();
        sorted.sort_unstable();
        let count = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), count, "duplicate limitation");
        assert!(LIMITATIONS.iter().all(|text| !text.is_empty()));
    }

    #[test]
    fn the_required_disclaimers_are_present_verbatim() {
        for required in [
            "Does not verify zero-knowledge proofs.",
            "Does not independently validate all Zcash consensus rules.",
            "Does not prove whether historical counterfeiting occurred.",
        ] {
            assert!(
                LIMITATIONS.contains(&required),
                "missing required disclaimer: {required}"
            );
        }
    }

    #[test]
    fn limitations_cover_every_non_goal() {
        let joined = LIMITATIONS.join(" ").to_lowercase();
        for topic in [
            "zero-knowledge",
            "consensus rules",
            "counterfeiting",
            "genesis",
            "circuit",
            "orchard",
            "full-node",
            "audit",
        ] {
            assert!(joined.contains(topic), "no limitation mentions {topic}");
        }
    }

    #[test]
    fn standard_limitations_match_the_constant() {
        assert_eq!(Report::standard_limitations().len(), LIMITATIONS.len());
    }

    #[test]
    fn a_height_row_reports_divergence_on_any_axis() {
        let base = HeightRow {
            height: BlockHeight::new(1),
            orchard_delta_zatoshis: Zatoshi::ZERO,
            ironwood_delta_zatoshis: Zatoshi::ZERO,
            orchard_expected_balance_zatoshis: Zatoshi::ZERO,
            ironwood_expected_balance_zatoshis: Zatoshi::ZERO,
            orchard_reported_balance_zatoshis: None,
            ironwood_reported_balance_zatoshis: None,
            orchard_balance_agreement: Agreement::Agrees,
            ironwood_balance_agreement: Agreement::Agrees,
            orchard_delta_agreement: Agreement::Agrees,
            ironwood_delta_agreement: Agreement::Agrees,
        };
        assert!(!base.diverges());

        let diverging = HeightRow {
            ironwood_delta_agreement: Agreement::Differs,
            ..base
        };
        assert!(diverging.diverges());
    }

    #[test]
    fn agreement_serializes_in_screaming_snake_case() {
        assert_eq!(
            serde_json::to_string(&Agreement::NotReported).unwrap(),
            r#""NOT_REPORTED""#
        );
    }
}
