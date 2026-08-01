//! Interval planning and the preflight probe.
//!
//! Preflight exists so that a capture which cannot succeed fails in seconds rather than
//! after hours of block retrieval. Every condition checked here is one that would otherwise
//! be discovered at the end, once the bulk of the work had already been done and a large
//! bundle written.
//!
//! The order is deliberate: identity before schedule, schedule before position, position
//! before data. Each step is cheaper than the one after it and rules out a broader class of
//! mistake.

use crate::domain::height::{BlockHeight, HeightInterval};
use crate::domain::network::Network;
use crate::error::ReconcileError;
use crate::evidence::pool_state_file::{self, CapturedBlockState};
use crate::rpc::dto;
use crate::rpc::method::NodeClient;

use crate::capture::guard::{self, Advisory};

/// Largest interval a single capture may cover.
///
/// Derived from the archive entry limit rather than chosen: a bundle stores two files per
/// height, and [`crate::evidence::archive::ExtractionLimits`] admits 100,000 entries, so an
/// interval beyond this could produce a bundle the tool would later refuse to extract.
pub const MAX_INTERVAL_BLOCKS: u32 = 49_000;

/// What the caller asked to capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureRequest {
    pub interval: HeightInterval,
    pub network: Network,
    pub tip_distance: u32,
    pub expected_activation_height: Option<u32>,
}

impl CaptureRequest {
    /// Builds a request from command-line bounds, rejecting an interval that cannot be
    /// captured before any node is contacted.
    pub fn new(
        network: Network,
        from_height: u32,
        to_height: u32,
        tip_distance: u32,
        expected_activation_height: Option<u32>,
    ) -> Result<Self, ReconcileError> {
        let interval =
            HeightInterval::new(BlockHeight::new(from_height), BlockHeight::new(to_height))?;

        if interval.block_count() > MAX_INTERVAL_BLOCKS {
            return Err(ReconcileError::InvalidInterval {
                reason: format!(
                    "an interval of {} blocks exceeds the {MAX_INTERVAL_BLOCKS} block maximum; \
                     capture it as several bundles",
                    interval.block_count()
                ),
            });
        }

        Ok(Self {
            interval,
            network,
            tip_distance,
            expected_activation_height,
        })
    }
}

/// What preflight established about the node and the requested interval.
#[derive(Debug, Clone)]
pub struct Preflight {
    pub node: dto::NodeInfo,
    pub node_json: serde_json::Value,
    pub chain: dto::ChainInfo,
    pub chain_json: serde_json::Value,
    /// The node's validated tip at the moment the capture began.
    pub tip: BlockHeight,
    /// Reported state at the anchor, probed early to catch a node serving unusable values.
    pub anchor: CapturedBlockState,
    pub anchor_json: serde_json::Value,
    pub advisories: Vec<Advisory>,
}

/// Establishes that a capture can proceed, before any block is retrieved.
pub fn preflight(
    client: &NodeClient<'_>,
    request: &CaptureRequest,
) -> Result<Preflight, ReconcileError> {
    let node = client.get_info()?;
    let chain = client.get_blockchain_info()?;

    guard::check_network(request.network, &chain.value, &node.value)?;
    guard::check_activation(
        request.network,
        &chain.value,
        request.expected_activation_height,
    )?;

    let tip = BlockHeight::new(chain.value.blocks);
    guard::check_tip_distance(tip, request.interval.end_height(), request.tip_distance)?;

    // The anchor is probed rather than assumed. A node serving empty pool values does so at
    // arbitrary heights, so the only way to know this capture is usable is to read one.
    let anchor_height = request.interval.anchor_height();
    let anchor_json = client.get_block_object(anchor_height)?;
    let anchor = parse_pool_state(anchor_height, &anchor_json)?;

    let mut advisories = guard::interval_advisories(request.network, request.interval);
    advisories.extend(guard::advisories(&anchor));

    Ok(Preflight {
        node: node.value,
        node_json: node.json,
        chain: chain.value,
        chain_json: chain.json,
        tip,
        anchor,
        anchor_json,
        advisories,
    })
}

/// Reads a node response through the same parser offline verification will use.
///
/// Capture deliberately does not decode `valuePools` itself. It validates the exact bytes
/// it is about to store, using the code that will later read them back, so a response this
/// build could not interpret is refused at capture time rather than after publication.
pub fn parse_pool_state(
    height: BlockHeight,
    response: &serde_json::Value,
) -> Result<CapturedBlockState, ReconcileError> {
    let state = pool_state_file::parse(&pool_state_bytes(response)?)?;

    guard::check_height_matches(height, &state)?;
    guard::check_pool_state_usable(&state)?;

    Ok(state)
}

/// Renders the pool-state file a bundle stores for one height.
///
/// The node's response is projected onto the fields that describe the block before it is
/// written, so that capturing the same height twice — on resume, or by a second operator —
/// produces the same bytes.
pub fn pool_state_bytes(response: &serde_json::Value) -> Result<Vec<u8>, ReconcileError> {
    serialize_response(&pool_state_file::project(response))
}

/// Renders a node response into the bytes a bundle stores.
///
/// Serialization is stable for a given value, so the same response always produces the same
/// file and a resumed capture cannot disagree with an uninterrupted one.
pub fn serialize_response(response: &serde_json::Value) -> Result<Vec<u8>, ReconcileError> {
    serde_json::to_vec_pretty(response).map_err(|source| ReconcileError::Internal {
        reason: format!("could not serialize a node response: {source}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reasonable_interval_is_accepted() {
        let request =
            CaptureRequest::new(Network::Mainnet, 3_428_143, 3_429_143, 100, None).unwrap();
        assert_eq!(request.interval.block_count(), 1_001);
        assert_eq!(request.interval.anchor_height().get(), 3_428_142);
    }

    #[test]
    fn a_reversed_interval_is_refused() {
        assert!(matches!(
            CaptureRequest::new(Network::Mainnet, 200, 100, 100, None),
            Err(ReconcileError::InvalidInterval { .. })
        ));
    }

    #[test]
    fn an_interval_starting_at_genesis_has_no_anchor() {
        assert!(CaptureRequest::new(Network::Mainnet, 0, 100, 100, None).is_err());
    }

    #[test]
    fn an_interval_beyond_the_maximum_is_refused_before_contacting_a_node() {
        let error = CaptureRequest::new(Network::Mainnet, 1, 1 + MAX_INTERVAL_BLOCKS, 100, None)
            .unwrap_err();
        assert!(matches!(error, ReconcileError::InvalidInterval { .. }));
        assert!(error.to_string().contains("several bundles"), "{error}");
    }

    #[test]
    fn an_interval_at_exactly_the_maximum_is_accepted() {
        let last = MAX_INTERVAL_BLOCKS;
        assert!(CaptureRequest::new(Network::Mainnet, 1, last, 100, None).is_ok());
    }

    #[test]
    fn the_maximum_interval_stays_within_the_archive_entry_limit() {
        use crate::evidence::archive::ExtractionLimits;
        // Two files per height, plus the anchor, manifest, metadata, and reports.
        let entries = u64::from(MAX_INTERVAL_BLOCKS)
            .checked_mul(2)
            .and_then(|files| files.checked_add(32))
            .unwrap();
        assert!(entries < u64::from(ExtractionLimits::default().max_entries));
    }

    #[test]
    fn a_pool_response_for_the_wrong_height_is_refused() {
        let response = serde_json::json!({
            "hash": "00",
            "height": 99,
            "valuePools": [
                {"id": "orchard", "chainValueZat": 1},
                {"id": "ironwood", "chainValueZat": 2}
            ]
        });
        assert!(matches!(
            parse_pool_state(BlockHeight::new(100), &response),
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }

    #[test]
    fn a_pool_response_without_reconstructed_balances_is_refused() {
        let response = serde_json::json!({"hash": "00", "height": 100, "valuePools": []});
        assert!(matches!(
            parse_pool_state(BlockHeight::new(100), &response),
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }

    #[test]
    fn a_usable_pool_response_is_accepted() {
        let response = serde_json::json!({
            "hash": "aa",
            "height": 100,
            "valuePools": [
                {"id": "orchard", "chainValueZat": 1, "monitored": true},
                {"id": "ironwood", "chainValueZat": 2, "monitored": true}
            ]
        });
        let state = parse_pool_state(BlockHeight::new(100), &response).unwrap();
        assert_eq!(state.block_hash, "aa");
    }

    #[test]
    fn serialization_of_a_response_is_stable() {
        let response = serde_json::json!({"b": 2, "a": 1, "nested": {"z": 26, "y": 25}});
        assert_eq!(
            serialize_response(&response).unwrap(),
            serialize_response(&response).unwrap()
        );
    }
}
