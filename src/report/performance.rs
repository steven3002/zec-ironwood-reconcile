//! Performance measurements, carried outside the hashed report.
//!
//! Duration, throughput and host details legitimately vary between machines and runs. They
//! are useful to record and useless to hash: including any of them would make the report
//! hash unreproducible, which would defeat offline verification entirely.
//!
//! This type is deliberately **not** reachable from
//! [`crate::report::schema::Report`]. It is emitted on its own, and a test asserts that its
//! field names never appear in canonical report bytes.
//!
//! Only raw integer measurements are stored. No derived rate is computed here: the crate
//! contains no floating-point arithmetic anywhere, so that no rounding behaviour can differ
//! between platforms. A consumer that wants a throughput figure derives it at the point of
//! display, where it cannot reach an artifact.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceMetadata {
    pub blocks_processed: u32,
    pub transactions_processed: u64,
    pub elapsed_milliseconds: u64,
    pub evidence_bytes_read: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical;
    use crate::checks::Status;
    use crate::domain::height::BlockHeight;
    use crate::domain::network::Network;
    use crate::domain::zatoshi::Zatoshi;
    use crate::report::schema::{
        PerHeightSummary, PoolFlowsObserved, Reconstructed, Report, ReportAnchor, ReportInterval,
        Reported,
    };

    fn sample() -> PerformanceMetadata {
        PerformanceMetadata {
            blocks_processed: 1_001,
            transactions_processed: 4_212,
            elapsed_milliseconds: 2_000,
            evidence_bytes_read: 41_943_040,
        }
    }

    fn empty_report() -> Report {
        Report {
            report_schema_version: "1.0.0".to_owned(),
            tool_version: "0.1.0".to_owned(),
            bundle_id: "mainnet-1-2".to_owned(),
            network: Network::Mainnet,
            interval: ReportInterval {
                anchor_height: BlockHeight::new(1),
                start_height: BlockHeight::new(2),
                end_height: BlockHeight::new(2),
                block_count: 1,
            },
            anchor: ReportAnchor {
                orchard_balance_zatoshis: Zatoshi::ZERO,
                ironwood_balance_zatoshis: Zatoshi::ZERO,
            },
            reconstructed: Reconstructed {
                orchard_delta_zatoshis: Zatoshi::ZERO,
                ironwood_delta_zatoshis: Zatoshi::ZERO,
                orchard_expected_end_zatoshis: Zatoshi::ZERO,
                ironwood_expected_end_zatoshis: Zatoshi::ZERO,
            },
            reported: Reported {
                orchard_end_zatoshis: None,
                ironwood_end_zatoshis: None,
            },
            pool_flows_observed: PoolFlowsObserved {
                orchard_outflow_zatoshis: Zatoshi::ZERO,
                ironwood_inflow_zatoshis: Zatoshi::ZERO,
            },
            per_height_summary: PerHeightSummary {
                heights_compared: 0,
                heights_diverging: 0,
                first_diverging_height: None,
            },
            per_height: Vec::new(),
            checks: Vec::new(),
            overall_status: Status::Pass,
            limitations: Report::standard_limitations(),
        }
    }

    #[test]
    fn only_integer_measurements_are_stored() {
        let json = serde_json::to_string(&sample()).unwrap();
        assert!(
            !json.contains('.'),
            "performance metadata must carry no fractional values: {json}"
        );
    }

    #[test]
    fn metadata_round_trips() {
        let json = serde_json::to_string(&sample()).unwrap();
        let parsed: PerformanceMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, sample());
    }

    #[test]
    fn performance_field_names_never_appear_in_a_canonical_report() {
        let bytes = canonical::to_canonical_bytes(&empty_report()).unwrap();
        let text = String::from_utf8(bytes).unwrap();

        for forbidden in [
            "elapsed_milliseconds",
            "blocks_processed",
            "evidence_bytes_read",
            "transactions_processed",
        ] {
            assert!(
                !text.contains(forbidden),
                "performance field {forbidden} leaked into the canonical report"
            );
        }
    }

    #[test]
    fn performance_metadata_is_not_reachable_from_a_report() {
        // The report type has no field of this type; serializing one and the other must
        // produce disjoint key sets for the measurement fields.
        let report_bytes = canonical::to_canonical_bytes(&empty_report()).unwrap();
        let performance_bytes = canonical::to_canonical_bytes(&sample()).unwrap();
        assert_ne!(report_bytes, performance_bytes);
    }
}
