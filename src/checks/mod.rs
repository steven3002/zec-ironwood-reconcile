//! The check registry.
//!
//! Every verdict a report carries is a [`Check`] with a **stable identifier**. The
//! identifier is part of the report schema contract: it never changes when human-readable
//! wording changes, so improving a message cannot alter a published report's hash.
//!
//! The registry is the single source of truth for both the documented check table and the
//! `checks[]` array in a report. They cannot drift apart because they are the same data.
//!
//! Warnings are recorded through the same mechanism as failures but carry a distinct
//! status. A warning must never be mistakable for a pass, and a failure is never downgraded
//! into one.

pub mod accounting;
pub mod activation;
pub mod structural;

use serde::{Deserialize, Serialize};

/// Stable check identifiers.
///
/// Changing any of these strings is a breaking change to the report schema and requires a
/// schema version increment.
pub mod ids {
    // Structural: the evidence is intact and describes what it claims to.
    pub const MANIFEST_SCHEMA_RECOGNIZED: &str = "manifest_schema_recognized";
    pub const EVIDENCE_HASHES_VALID: &str = "evidence_hashes_valid";
    pub const NETWORK_MATCHES: &str = "network_matches";
    pub const ANCHOR_BLOCK_PRESENT: &str = "anchor_block_present";
    pub const BLOCK_SEQUENCE_COMPLETE: &str = "block_sequence_complete";
    pub const PREVIOUS_BLOCK_LINKS_VALID: &str = "previous_block_links_valid";

    // Activation: properties specific to the NU6.3 upgrade boundary.
    pub const ACTIVATION_CONTEXT_VALID: &str = "activation_context_valid";
    pub const CONSENSUS_BRANCH_ID_VALID: &str = "consensus_branch_id_valid";
    pub const TRANSACTION_VERSIONS_RECOGNIZED: &str = "transaction_versions_recognized";
    pub const IRONWOOD_ANCHOR_ZERO: &str = "ironwood_anchor_zero";
    pub const NO_IRONWOOD_BEFORE_ACTIVATION: &str = "no_ironwood_before_activation";
    pub const ORCHARD_WITHDRAWAL_ONLY: &str = "orchard_withdrawal_only";

    // Accounting: the reconstruction itself.
    pub const ORCHARD_DELTAS_RECONSTRUCTED: &str = "orchard_deltas_reconstructed";
    pub const IRONWOOD_DELTAS_RECONSTRUCTED: &str = "ironwood_deltas_reconstructed";
    pub const NO_ARITHMETIC_OVERFLOW: &str = "no_arithmetic_overflow";
    pub const NO_NEGATIVE_POOL_BALANCE: &str = "no_negative_pool_balance";
    pub const PER_HEIGHT_BALANCES_MATCH: &str = "per_height_balances_match";
    pub const PER_HEIGHT_DELTAS_MATCH: &str = "per_height_deltas_match";
    pub const ORCHARD_END_BALANCE_MATCHES: &str = "orchard_end_balance_matches";
    pub const IRONWOOD_END_BALANCE_MATCHES: &str = "ironwood_end_balance_matches";

    // Reporting: recorded once a report has been produced and re-verified.
    pub const CANONICAL_REPORT_GENERATED: &str = "canonical_report_generated";
    pub const OFFLINE_REPORT_HASH_REPRODUCIBLE: &str = "offline_report_hash_reproducible";

    /// Every identifier, in the order checks are presented in a report.
    pub const ALL: [&str; 22] = [
        MANIFEST_SCHEMA_RECOGNIZED,
        EVIDENCE_HASHES_VALID,
        NETWORK_MATCHES,
        ANCHOR_BLOCK_PRESENT,
        BLOCK_SEQUENCE_COMPLETE,
        PREVIOUS_BLOCK_LINKS_VALID,
        ACTIVATION_CONTEXT_VALID,
        CONSENSUS_BRANCH_ID_VALID,
        TRANSACTION_VERSIONS_RECOGNIZED,
        IRONWOOD_ANCHOR_ZERO,
        NO_IRONWOOD_BEFORE_ACTIVATION,
        ORCHARD_WITHDRAWAL_ONLY,
        ORCHARD_DELTAS_RECONSTRUCTED,
        IRONWOOD_DELTAS_RECONSTRUCTED,
        NO_ARITHMETIC_OVERFLOW,
        NO_NEGATIVE_POOL_BALANCE,
        PER_HEIGHT_BALANCES_MATCH,
        PER_HEIGHT_DELTAS_MATCH,
        ORCHARD_END_BALANCE_MATCHES,
        IRONWOOD_END_BALANCE_MATCHES,
        CANONICAL_REPORT_GENERATED,
        OFFLINE_REPORT_HASH_REPRODUCIBLE,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Pass,
    Fail,
    Warn,
    /// The check does not apply to this bundle, for a stated reason.
    NotApplicable,
}

impl Status {
    pub const fn is_failure(self) -> bool {
        matches!(self, Self::Fail)
    }

    pub const fn is_warning(self) -> bool {
        matches!(self, Self::Warn)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub status: Status,
    pub details: Option<String>,
}

impl Check {
    pub fn pass(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            status: Status::Pass,
            details: None,
        }
    }

    pub fn fail(id: &str, details: impl Into<String>) -> Self {
        Self {
            id: id.to_owned(),
            status: Status::Fail,
            details: Some(details.into()),
        }
    }

    pub fn warn(id: &str, details: impl Into<String>) -> Self {
        Self {
            id: id.to_owned(),
            status: Status::Warn,
            details: Some(details.into()),
        }
    }

    pub fn not_applicable(id: &str, details: impl Into<String>) -> Self {
        Self {
            id: id.to_owned(),
            status: Status::NotApplicable,
            details: Some(details.into()),
        }
    }
}

/// An ordered collection of check results.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CheckRegistry {
    checks: Vec<Check>,
}

impl CheckRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, check: Check) {
        self.checks.push(check);
    }

    /// Records a pass or a failure from a boolean condition.
    pub fn record_condition(&mut self, id: &str, passed: bool, failure_details: impl Into<String>) {
        if passed {
            self.record(Check::pass(id));
        } else {
            self.record(Check::fail(id, failure_details));
        }
    }

    pub fn checks(&self) -> &[Check] {
        &self.checks
    }

    pub fn failures(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|check| check.status.is_failure())
    }

    pub fn warnings(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|check| check.status.is_warning())
    }

    pub fn has_failures(&self) -> bool {
        self.failures().next().is_some()
    }

    /// The report's overall verdict.
    ///
    /// A warning never produces an overall pass on its own, but neither does it produce a
    /// failure: it is reported as a warning so a reader can weigh it.
    pub fn overall_status(&self) -> Status {
        if self.has_failures() {
            Status::Fail
        } else if self.warnings().next().is_some() {
            Status::Warn
        } else {
            Status::Pass
        }
    }

    /// Sorts checks into the canonical presentation order.
    ///
    /// Ordering is by the position of an identifier in [`ids::ALL`], so report output does
    /// not depend on the order in which checks happened to be evaluated.
    pub fn sort_canonically(&mut self) {
        self.checks.sort_by_key(|check| {
            ids::ALL
                .iter()
                .position(|id| *id == check.id)
                .unwrap_or(usize::MAX)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn identifiers_are_unique() {
        let unique: BTreeSet<&str> = ids::ALL.into_iter().collect();
        assert_eq!(unique.len(), ids::ALL.len(), "duplicate check identifier");
    }

    #[test]
    fn identifiers_are_snake_case_and_non_empty() {
        for id in ids::ALL {
            assert!(!id.is_empty());
            assert!(
                id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "identifier is not snake_case: {id}"
            );
        }
    }

    #[test]
    fn an_empty_registry_passes() {
        assert_eq!(CheckRegistry::new().overall_status(), Status::Pass);
    }

    #[test]
    fn any_failure_makes_the_overall_status_fail() {
        let mut registry = CheckRegistry::new();
        registry.record(Check::pass(ids::NETWORK_MATCHES));
        registry.record(Check::fail(ids::EVIDENCE_HASHES_VALID, "digest mismatch"));
        assert_eq!(registry.overall_status(), Status::Fail);
        assert!(registry.has_failures());
    }

    #[test]
    fn a_warning_does_not_produce_a_pass_or_a_failure() {
        let mut registry = CheckRegistry::new();
        registry.record(Check::pass(ids::NETWORK_MATCHES));
        registry.record(Check::warn(ids::BLOCK_SEQUENCE_COMPLETE, "unlisted file"));
        assert_eq!(registry.overall_status(), Status::Warn);
        assert!(!registry.has_failures());
    }

    #[test]
    fn a_failure_outranks_a_warning() {
        let mut registry = CheckRegistry::new();
        registry.record(Check::warn(ids::BLOCK_SEQUENCE_COMPLETE, "unlisted file"));
        registry.record(Check::fail(ids::EVIDENCE_HASHES_VALID, "digest mismatch"));
        assert_eq!(registry.overall_status(), Status::Fail);
    }

    #[test]
    fn warnings_and_failures_are_separately_enumerable() {
        let mut registry = CheckRegistry::new();
        registry.record(Check::pass(ids::NETWORK_MATCHES));
        registry.record(Check::warn(ids::BLOCK_SEQUENCE_COMPLETE, "note"));
        registry.record(Check::fail(ids::EVIDENCE_HASHES_VALID, "bad"));

        assert_eq!(registry.warnings().count(), 1);
        assert_eq!(registry.failures().count(), 1);
        assert_eq!(registry.checks().len(), 3);
    }

    #[test]
    fn recording_a_condition_produces_the_matching_status() {
        let mut registry = CheckRegistry::new();
        registry.record_condition(ids::NETWORK_MATCHES, true, "unused");
        registry.record_condition(ids::ANCHOR_BLOCK_PRESENT, false, "anchor absent");

        assert_eq!(registry.checks()[0].status, Status::Pass);
        assert_eq!(registry.checks()[0].details, None);
        assert_eq!(registry.checks()[1].status, Status::Fail);
        assert_eq!(
            registry.checks()[1].details.as_deref(),
            Some("anchor absent")
        );
    }

    #[test]
    fn canonical_sorting_is_independent_of_evaluation_order() {
        let mut first = CheckRegistry::new();
        first.record(Check::pass(ids::ORCHARD_END_BALANCE_MATCHES));
        first.record(Check::pass(ids::NETWORK_MATCHES));
        first.record(Check::pass(ids::IRONWOOD_ANCHOR_ZERO));
        first.sort_canonically();

        let mut second = CheckRegistry::new();
        second.record(Check::pass(ids::IRONWOOD_ANCHOR_ZERO));
        second.record(Check::pass(ids::ORCHARD_END_BALANCE_MATCHES));
        second.record(Check::pass(ids::NETWORK_MATCHES));
        second.sort_canonically();

        assert_eq!(first, second);
        assert_eq!(first.checks()[0].id, ids::NETWORK_MATCHES);
    }

    #[test]
    fn a_check_serializes_with_an_uppercase_status() {
        let json = serde_json::to_string(&Check::pass(ids::NETWORK_MATCHES)).unwrap();
        assert!(json.contains(r#""status":"PASS""#), "{json}");
        assert!(json.contains(r#""id":"network_matches""#), "{json}");
    }

    #[test]
    fn a_not_applicable_check_is_neither_pass_nor_failure() {
        let mut registry = CheckRegistry::new();
        registry.record(Check::not_applicable(
            ids::NO_IRONWOOD_BEFORE_ACTIVATION,
            "interval begins at activation",
        ));
        assert!(!registry.has_failures());
        assert_eq!(registry.overall_status(), Status::Pass);
    }
}
