//! Checks that the evidence is intact and describes what it claims to.
//!
//! These run before any accounting verdict. A reconciliation over altered or incomplete
//! evidence produces a number, but not a meaningful one.

use crate::checks::{Check, CheckRegistry, ids};
use crate::domain::network::Network;
use crate::error::ReconcileError;
use crate::evidence::manifest::Manifest;
use crate::evidence::validation::{ValidationReport, ValidationWarning};

/// Records the structural verdicts for a bundle.
///
/// `continuity` carries the outcome of chain verification, which is performed separately
/// because it needs the parsed blocks rather than only the manifest.
pub fn evaluate(
    manifest: &Manifest,
    validation: &ValidationReport,
    requested_network: Network,
    anchor_block_present: bool,
    continuity: Result<(), &ReconcileError>,
    registry: &mut CheckRegistry,
) {
    // Reaching this point means the manifest parsed and its structure validated; a failure
    // would have aborted before any check could be recorded.
    registry.record(Check::pass(ids::MANIFEST_SCHEMA_RECOGNIZED));

    check_evidence_hashes(validation, registry);
    check_network(manifest, requested_network, registry);

    registry.record_condition(
        ids::ANCHOR_BLOCK_PRESENT,
        anchor_block_present,
        "the bundle contains no anchor block",
    );

    check_continuity(continuity, registry);
    record_unlisted_file_warnings(validation, registry);
}

fn check_evidence_hashes(validation: &ValidationReport, registry: &mut CheckRegistry) {
    match validation.failures.first() {
        None => registry.record(Check::pass(ids::EVIDENCE_HASHES_VALID)),
        Some(first) => registry.record(Check::fail(
            ids::EVIDENCE_HASHES_VALID,
            format!(
                "{} evidence failure(s); first: {first}",
                validation.failures.len()
            ),
        )),
    }
}

fn check_network(manifest: &Manifest, requested: Network, registry: &mut CheckRegistry) {
    registry.record_condition(
        ids::NETWORK_MATCHES,
        manifest.network == requested,
        format!(
            "bundle declares network {} but {requested} was requested",
            manifest.network
        ),
    );
}

/// Chain continuity, split across the two identifiers a report distinguishes.
///
/// A coverage problem and a linkage problem are different findings: the first says a block
/// is missing, the second says the blocks present do not form a chain.
fn check_continuity(continuity: Result<(), &ReconcileError>, registry: &mut CheckRegistry) {
    match continuity {
        Ok(()) => {
            registry.record(Check::pass(ids::BLOCK_SEQUENCE_COMPLETE));
            registry.record(Check::pass(ids::PREVIOUS_BLOCK_LINKS_VALID));
        }
        Err(error @ ReconcileError::MissingBlock(_)) => {
            registry.record(Check::fail(ids::BLOCK_SEQUENCE_COMPLETE, error.to_string()));
            registry.record(Check::not_applicable(
                ids::PREVIOUS_BLOCK_LINKS_VALID,
                "linkage cannot be assessed while the sequence is incomplete",
            ));
        }
        Err(error) => {
            registry.record(Check::pass(ids::BLOCK_SEQUENCE_COMPLETE));
            registry.record(Check::fail(
                ids::PREVIOUS_BLOCK_LINKS_VALID,
                error.to_string(),
            ));
        }
    }
}

/// Surfaces validation warnings without letting them affect a pass.
fn record_unlisted_file_warnings(validation: &ValidationReport, registry: &mut CheckRegistry) {
    let unlisted = validation
        .warnings
        .iter()
        .filter(|warning| matches!(warning, ValidationWarning::UnlistedFile { .. }))
        .count();

    if unlisted > 0 {
        registry.record(Check::warn(
            ids::EVIDENCE_HASHES_VALID,
            format!("{unlisted} file(s) present in the bundle but absent from the manifest"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checks::Status;
    use crate::domain::height::{BlockHeight, HeightInterval};
    use crate::domain::zatoshi::Zatoshi;
    use crate::evidence::manifest::{
        Activation, AnchorState, EndState, Rfc3339Timestamp, SCHEMA_VERSION, Source, Tool,
    };

    fn manifest(network: Network) -> Manifest {
        let interval =
            HeightInterval::new(BlockHeight::new(3_428_143), BlockHeight::new(3_428_243)).unwrap();
        Manifest {
            schema_version: SCHEMA_VERSION.to_owned(),
            bundle_id: Manifest::derive_bundle_id(network, interval),
            created_at: Rfc3339Timestamp::parse("2026-07-29T14:30:00Z").unwrap(),
            tool: Tool {
                name: "zec-ironwood-reconcile".to_owned(),
                version: "0.1.0".to_owned(),
                git_commit: None,
            },
            source: Source {
                implementation: "zebra".to_owned(),
                version: "6.2.3".to_owned(),
                rpc_url_redacted: true,
            },
            network,
            activation: Activation {
                upgrade: "NU6.3".to_owned(),
                expected_height: network.ironwood_activation_height(),
            },
            interval: interval.into(),
            anchor: AnchorState {
                block_hash: "0".repeat(64),
                orchard_balance_zatoshis: Zatoshi::ZERO,
                ironwood_balance_zatoshis: Zatoshi::ZERO,
            },
            end: EndState {
                block_hash: "1".repeat(64),
                reported_orchard_balance_zatoshis: Zatoshi::ZERO,
                reported_ironwood_balance_zatoshis: Zatoshi::ZERO,
            },
            files: Vec::new(),
        }
    }

    fn status_of(registry: &CheckRegistry, id: &str) -> Status {
        registry
            .checks()
            .iter()
            .find(|check| check.id == id)
            .map(|check| check.status)
            .expect("check should have been recorded")
    }

    #[test]
    fn intact_evidence_on_the_requested_network_passes() {
        let mut registry = CheckRegistry::new();
        evaluate(
            &manifest(Network::Mainnet),
            &ValidationReport::default(),
            Network::Mainnet,
            true,
            Ok(()),
            &mut registry,
        );
        assert!(!registry.has_failures(), "{:?}", registry.checks());
    }

    #[test]
    fn a_network_mismatch_fails() {
        let mut registry = CheckRegistry::new();
        evaluate(
            &manifest(Network::Testnet),
            &ValidationReport::default(),
            Network::Mainnet,
            true,
            Ok(()),
            &mut registry,
        );
        assert_eq!(status_of(&registry, ids::NETWORK_MATCHES), Status::Fail);
    }

    #[test]
    fn an_evidence_hash_failure_is_reported_with_a_count() {
        let validation = ValidationReport {
            failures: vec![
                ReconcileError::HashMismatch {
                    path: "blocks/1.hex".to_owned(),
                },
                ReconcileError::MissingFile {
                    path: "blocks/2.hex".to_owned(),
                },
            ],
            warnings: Vec::new(),
        };

        let mut registry = CheckRegistry::new();
        evaluate(
            &manifest(Network::Mainnet),
            &validation,
            Network::Mainnet,
            true,
            Ok(()),
            &mut registry,
        );

        let check = registry
            .checks()
            .iter()
            .find(|check| check.id == ids::EVIDENCE_HASHES_VALID && check.status == Status::Fail)
            .unwrap();
        assert!(check.details.as_ref().unwrap().contains('2'));
    }

    #[test]
    fn a_missing_anchor_fails() {
        let mut registry = CheckRegistry::new();
        evaluate(
            &manifest(Network::Mainnet),
            &ValidationReport::default(),
            Network::Mainnet,
            false,
            Ok(()),
            &mut registry,
        );
        assert_eq!(
            status_of(&registry, ids::ANCHOR_BLOCK_PRESENT),
            Status::Fail
        );
    }

    #[test]
    fn a_missing_block_fails_coverage_and_suspends_linkage() {
        let error = ReconcileError::MissingBlock(3_428_150);
        let mut registry = CheckRegistry::new();
        evaluate(
            &manifest(Network::Mainnet),
            &ValidationReport::default(),
            Network::Mainnet,
            true,
            Err(&error),
            &mut registry,
        );

        assert_eq!(
            status_of(&registry, ids::BLOCK_SEQUENCE_COMPLETE),
            Status::Fail
        );
        assert_eq!(
            status_of(&registry, ids::PREVIOUS_BLOCK_LINKS_VALID),
            Status::NotApplicable
        );
    }

    #[test]
    fn a_broken_link_fails_linkage_while_coverage_stands() {
        let error = ReconcileError::BlockContinuity {
            height: 3_428_150,
            reason: "previous hash mismatch".to_owned(),
        };
        let mut registry = CheckRegistry::new();
        evaluate(
            &manifest(Network::Mainnet),
            &ValidationReport::default(),
            Network::Mainnet,
            true,
            Err(&error),
            &mut registry,
        );

        assert_eq!(
            status_of(&registry, ids::BLOCK_SEQUENCE_COMPLETE),
            Status::Pass
        );
        assert_eq!(
            status_of(&registry, ids::PREVIOUS_BLOCK_LINKS_VALID),
            Status::Fail
        );
    }

    #[test]
    fn an_unlisted_file_warns_without_failing() {
        let validation = ValidationReport {
            failures: Vec::new(),
            warnings: vec![ValidationWarning::UnlistedFile {
                path: "blocks/999.hex".to_owned(),
            }],
        };

        let mut registry = CheckRegistry::new();
        evaluate(
            &manifest(Network::Mainnet),
            &validation,
            Network::Mainnet,
            true,
            Ok(()),
            &mut registry,
        );

        assert!(!registry.has_failures());
        assert_eq!(registry.warnings().count(), 1);
        assert_eq!(registry.overall_status(), Status::Warn);
    }
}
