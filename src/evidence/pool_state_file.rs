//! The boundary between a node's `getblock` response and the pool-state evidence file.
//!
//! # Why the response is projected rather than stored verbatim
//!
//! A `getblock` response mixes two kinds of field. Some describe the block: its hash, its
//! height, the value each pool held after it. Others describe the moment it was asked for,
//! `confirmations` being the clearest, it is the distance to the chain tip, so the same
//! block yields a different response every few minutes.
//!
//! Storing the response verbatim would make an evidence file a record of when a capture ran
//! rather than of what a block contains, and two operators capturing the same interval would
//! produce different bytes for the same block. Independent reproduction is the property this
//! project rests on, so [`project`] keeps the fields that belong to the block and discards
//! the rest.
//!
//! This applies to the reported-pools file only. A block's consensus bytes are stored
//! exactly as the node served them, because there the encoding is the evidence.
//!
//! # Encoding differs from the canonical report
//!
//! Node responses encode monetary values as JSON **numbers**; this crate's own artifacts
//! encode them as strings, because RFC 8785 canonicalization routes numbers through
//! IEEE-754 doubles. Both are correct in their own context, and the conversion happens
//! exactly here.
//!
//! # Absence is preserved
//!
//! A pool entry may carry no `chainValueZat`, and a node undergoing a database upgrade may
//! omit them entirely while otherwise appearing healthy. An absent value stays absent; it
//! never becomes zero.
//!
//! The node's `monitored` flag is preserved because it is part of the response, not because
//! anything infers from it. Zebra builds every pool entry through one constructor that sets
//! `monitored: amount.zatoshis() != 0`, so the flag restates whether the balance is non-zero
//! and is not a statement about which pools the node tracks. Nothing in this crate may treat
//! a `false` there as marking a balance unmeasured.

use serde::Deserialize;

use crate::domain::height::BlockHeight;
use crate::domain::pool::Pool;
use crate::domain::pool_state::ReportedPoolState;
use crate::domain::zatoshi::Zatoshi;
use crate::error::ReconcileError;

/// One entry of a node's `valuePools` array.
#[derive(Debug, Clone, Deserialize)]
struct ValuePoolEntry {
    id: String,
    #[serde(default)]
    #[serde(rename = "chainValueZat")]
    chain_value_zat: Option<i64>,
    #[serde(default)]
    #[serde(rename = "valueDeltaZat")]
    value_delta_zat: Option<i64>,
    /// Whether the node is tracking this pool at this height. Absent on nodes that do not
    /// publish the flag, which is distinct from a reported `false`.
    #[serde(default)]
    monitored: Option<bool>,
}

/// The subset of a `getblock` response this crate reads.
#[derive(Debug, Clone, Deserialize)]
struct BlockStateResponse {
    hash: String,
    height: u32,
    #[serde(default)]
    #[serde(rename = "valuePools")]
    value_pools: Vec<ValuePoolEntry>,
}

/// What a node reported about one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedBlockState {
    pub height: BlockHeight,
    pub block_hash: String,
    pub pools: ReportedPoolState,
}

/// Parses a captured node response.
///
/// Pools are resolved by identifier, never by position in the array: the set has grown
/// across upgrades and one member was renamed, so positional access would silently read the
/// wrong pool against a node of a different version.
pub fn parse(bytes: &[u8]) -> Result<CapturedBlockState, ReconcileError> {
    let response: BlockStateResponse =
        serde_json::from_slice(bytes).map_err(|source| ReconcileError::CaptureIncomplete {
            reason: format!("captured pool state is not a recognisable response: {source}"),
        })?;

    let height = BlockHeight::new(response.height);
    let mut pools = ReportedPoolState::new(height);

    for entry in &response.value_pools {
        let Some(pool) = Pool::from_rpc_id(&entry.id) else {
            // A pool introduced by a future upgrade is ignored rather than misattributed.
            // It cannot affect a comparison, because only recognised pools are compared.
            continue;
        };

        if let Some(balance) = entry.chain_value_zat {
            pools = pools.with_balance(pool, Zatoshi::new_checked(balance)?);
        }
        if let Some(delta) = entry.value_delta_zat {
            pools = pools.with_delta(pool, Zatoshi::new_checked(delta)?);
        }
        if let Some(monitored) = entry.monitored {
            pools = pools.with_monitored(pool, monitored);
        }
    }

    Ok(CapturedBlockState {
        height,
        block_hash: response.hash,
        pools,
    })
}

/// Reduces a node's `getblock` response to the fields that describe the block.
///
/// The result is what a bundle stores. Keeping the projection explicit, an allow-list
/// rather than a list of known-variable fields to remove, means a field introduced by a
/// future node release cannot silently make evidence unreproducible.
///
/// An absent balance stays absent. Omission is meaningful, and the guard that refuses an
/// unusable capture depends on it surviving.
pub fn project(response: &serde_json::Value) -> serde_json::Value {
    let pools: Vec<serde_json::Value> = response
        .get("valuePools")
        .and_then(serde_json::Value::as_array)
        .map(|entries| entries.iter().map(project_pool).collect())
        .unwrap_or_default();

    let mut projected = serde_json::Map::new();
    for field in ["hash", "height"] {
        if let Some(value) = response.get(field) {
            projected.insert(field.to_owned(), value.clone());
        }
    }
    projected.insert("valuePools".to_owned(), serde_json::Value::Array(pools));

    serde_json::Value::Object(projected)
}

/// Keeps one pool entry's identity, integral amounts, and tracking flag.
///
/// The node also reports each amount as a floating-point `chainValue`. Those are dropped:
/// they are a lossy restatement of the zatoshi figures, and nothing here reads them.
fn project_pool(entry: &serde_json::Value) -> serde_json::Value {
    let mut projected = serde_json::Map::new();
    for field in ["id", "chainValueZat", "valueDeltaZat", "monitored"] {
        if let Some(value) = entry.get(field) {
            projected.insert(field.to_owned(), value.clone());
        }
    }
    serde_json::Value::Object(projected)
}

/// Parses a captured response and requires it to carry every reconstructed pool balance.
///
/// This is the guard against reconciling against absent values. A node serving empty pool
/// data at arbitrary heights, documented Zebra behaviour during a database upgrade, would
/// otherwise produce a confident and meaningless result.
pub fn parse_requiring_balances(bytes: &[u8]) -> Result<CapturedBlockState, ReconcileError> {
    let state = parse(bytes)?;

    let missing = state.pools.missing_reconstructed_balances();
    if !missing.is_empty() {
        let names: Vec<&str> = missing.iter().map(|pool| pool.rpc_id()).collect();
        return Err(ReconcileError::CaptureIncomplete {
            reason: format!(
                "node reported no balance for {} at height {}; the capture cannot support a comparison",
                names.join(", "),
                state.height
            ),
        });
    }

    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMPLETE: &str = r#"{
        "hash": "0000000000aaaabbbbccccddddeeeeffff00001111222233334444555566667777",
        "height": 3428143,
        "valuePools": [
            {"id": "transparent", "chainValueZat": 100, "valueDeltaZat": 1},
            {"id": "sprout", "chainValueZat": 200},
            {"id": "sapling", "chainValueZat": 300},
            {"id": "orchard", "chainValueZat": 366000000000000, "valueDeltaZat": -1000},
            {"id": "lockbox", "chainValueZat": 400},
            {"id": "ironwood", "chainValueZat": 1000, "valueDeltaZat": 1000}
        ]
    }"#;

    #[test]
    fn a_complete_response_parses() {
        let state = parse_requiring_balances(COMPLETE.as_bytes()).unwrap();
        assert_eq!(state.height, BlockHeight::new(3_428_143));
        assert_eq!(
            state.pools.balance(Pool::Orchard),
            Some(Zatoshi::from_raw(366_000_000_000_000))
        );
        assert_eq!(
            state.pools.delta(Pool::Ironwood),
            Some(Zatoshi::from_raw(1_000))
        );
    }

    #[test]
    fn pools_are_resolved_by_identifier_not_position() {
        // The same pools in a different order must parse identically.
        let reordered = r#"{
            "hash": "00",
            "height": 1,
            "valuePools": [
                {"id": "ironwood", "chainValueZat": 7},
                {"id": "orchard", "chainValueZat": 9}
            ]
        }"#;
        let state = parse(reordered.as_bytes()).unwrap();
        assert_eq!(
            state.pools.balance(Pool::Orchard),
            Some(Zatoshi::from_raw(9))
        );
        assert_eq!(
            state.pools.balance(Pool::Ironwood),
            Some(Zatoshi::from_raw(7))
        );
    }

    #[test]
    fn an_absent_balance_stays_absent_rather_than_becoming_zero() {
        let partial = r#"{
            "hash": "00",
            "height": 1,
            "valuePools": [
                {"id": "orchard", "valueDeltaZat": -5},
                {"id": "ironwood", "chainValueZat": 3}
            ]
        }"#;
        let state = parse(partial.as_bytes()).unwrap();
        assert_eq!(state.pools.balance(Pool::Orchard), None);
        assert_eq!(
            state.pools.delta(Pool::Orchard),
            Some(Zatoshi::from_raw(-5))
        );
    }

    #[test]
    fn a_null_balance_is_treated_as_absent() {
        let with_null = r#"{
            "hash": "00",
            "height": 1,
            "valuePools": [
                {"id": "orchard", "chainValueZat": null},
                {"id": "ironwood", "chainValueZat": 3}
            ]
        }"#;
        let state = parse(with_null.as_bytes()).unwrap();
        assert_eq!(state.pools.balance(Pool::Orchard), None);
    }

    #[test]
    fn a_missing_reconstructed_balance_is_refused_by_the_guard() {
        let partial = r#"{
            "hash": "00",
            "height": 3428143,
            "valuePools": [{"id": "ironwood", "chainValueZat": 3}]
        }"#;
        match parse_requiring_balances(partial.as_bytes()) {
            Err(ReconcileError::CaptureIncomplete { reason }) => {
                assert!(reason.contains("orchard"), "{reason}");
                assert!(reason.contains("3428143"), "{reason}");
            }
            other => panic!("expected a capture failure, got {other:?}"),
        }
    }

    #[test]
    fn an_empty_value_pools_array_is_refused_by_the_guard() {
        // Zebra can serve this while a database upgrade is in progress.
        let empty = r#"{"hash": "00", "height": 3428143, "valuePools": []}"#;
        assert!(matches!(
            parse_requiring_balances(empty.as_bytes()),
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }

    #[test]
    fn an_absent_value_pools_field_is_refused_by_the_guard() {
        let absent = r#"{"hash": "00", "height": 3428143}"#;
        assert!(matches!(
            parse_requiring_balances(absent.as_bytes()),
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }

    #[test]
    fn an_unrecognised_pool_is_ignored_rather_than_misattributed() {
        let future = r#"{
            "hash": "00",
            "height": 1,
            "valuePools": [
                {"id": "orchard", "chainValueZat": 1},
                {"id": "ironwood", "chainValueZat": 2},
                {"id": "sequoia", "chainValueZat": 999}
            ]
        }"#;
        let state = parse(future.as_bytes()).unwrap();
        assert_eq!(
            state.pools.balance(Pool::Orchard),
            Some(Zatoshi::from_raw(1))
        );
        assert_eq!(state.pools.balances.len(), 2);
    }

    #[test]
    fn malformed_json_is_reported_as_an_incomplete_capture() {
        assert!(matches!(
            parse(b"not json"),
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }

    #[test]
    fn a_balance_beyond_the_money_bound_is_rejected() {
        let absurd = r#"{
            "hash": "00",
            "height": 1,
            "valuePools": [{"id": "orchard", "chainValueZat": 9000000000000000}]
        }"#;
        assert!(matches!(
            parse(absurd.as_bytes()),
            Err(ReconcileError::ValueOutOfBounds { .. })
        ));
    }

    /// Reads a response recorded from a live node.
    fn recorded(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/rpc")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("missing fixture {}: {e}", path.display()))
    }

    #[test]
    fn a_response_recorded_from_a_live_node_parses() {
        let state = parse(&recorded("getblock-verbose-280769.json")).unwrap();

        assert_eq!(state.height, BlockHeight::new(280_769));
        assert_eq!(state.pools.balance(Pool::Orchard), Some(Zatoshi::ZERO));
        assert_eq!(state.pools.balance(Pool::Ironwood), Some(Zatoshi::ZERO));
        assert_eq!(
            state.pools.balance(Pool::Sapling),
            Some(Zatoshi::from_raw(1_000_000_000))
        );
    }

    #[test]
    fn an_untracked_pool_is_distinguished_from_a_measured_zero() {
        // Recorded either side of the height at which the sapling pool first held value.
        // Both responses state a sapling balance; only the later one is a measurement.
        let before = parse(&recorded("getblock-verbose-280768-unmonitored.json")).unwrap();
        let after = parse(&recorded("getblock-verbose-280769.json")).unwrap();

        assert_eq!(before.pools.balance(Pool::Sapling), Some(Zatoshi::ZERO));
        assert_eq!(before.pools.monitored(Pool::Sapling), Some(false));
        assert_eq!(after.pools.monitored(Pool::Sapling), Some(true));
    }

    #[test]
    fn empty_reconstructed_pools_are_listed() {
        // At this height neither Orchard nor Ironwood has been activated, so both hold
        // nothing.
        let state = parse(&recorded("getblock-verbose-280769.json")).unwrap();
        assert_eq!(
            state.pools.empty_reconstructed_pools(),
            vec![Pool::Orchard, Pool::Ironwood]
        );
    }

    #[test]
    fn projection_drops_fields_that_depend_on_when_the_block_was_asked_for() {
        // `confirmations` is the distance to the chain tip, so the same block yields a
        // different response minutes later. Keeping it would make evidence unreproducible.
        let response: serde_json::Value =
            serde_json::from_slice(&recorded("getblock-verbose-280769.json")).unwrap();
        assert!(
            response.get("confirmations").is_some(),
            "the recorded response should contain the field this test is about"
        );

        let projected = project(&response);
        assert!(projected.get("confirmations").is_none());
        assert!(projected.get("nextblockhash").is_none());
        assert_eq!(projected.get("height"), response.get("height"));
        assert_eq!(projected.get("hash"), response.get("hash"));
    }

    #[test]
    fn projection_survives_a_round_trip_through_the_parser() {
        let response: serde_json::Value =
            serde_json::from_slice(&recorded("getblock-verbose-280769.json")).unwrap();

        let direct = parse(&recorded("getblock-verbose-280769.json")).unwrap();
        let projected = parse(&serde_json::to_vec(&project(&response)).unwrap()).unwrap();

        assert_eq!(direct, projected);
    }

    #[test]
    fn projection_keeps_the_integral_amounts_and_drops_the_floating_point_ones() {
        let response = serde_json::json!({
            "hash": "aa",
            "height": 5,
            "confirmations": 900,
            "valuePools": [{
                "id": "orchard",
                "chainValue": 3.66,
                "chainValueZat": 366_000_000_i64,
                "valueDelta": -0.01,
                "valueDeltaZat": -1_000_000_i64,
                "monitored": true
            }]
        });

        let projected = project(&response);
        let pool = &projected["valuePools"][0];
        assert_eq!(pool["chainValueZat"], serde_json::json!(366_000_000_i64));
        assert_eq!(pool["valueDeltaZat"], serde_json::json!(-1_000_000_i64));
        assert_eq!(pool["monitored"], serde_json::json!(true));
        assert!(pool.get("chainValue").is_none());
        assert!(pool.get("valueDelta").is_none());
    }

    #[test]
    fn projection_preserves_an_absent_balance_rather_than_inventing_one() {
        let response = serde_json::json!({
            "hash": "aa",
            "height": 5,
            "valuePools": [{"id": "orchard", "monitored": false}]
        });

        let projected = project(&response);
        assert!(projected["valuePools"][0].get("chainValueZat").is_none());

        // The guard must still fire on the projected form.
        let bytes = serde_json::to_vec(&projected).unwrap();
        assert!(matches!(
            parse_requiring_balances(&bytes),
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }

    #[test]
    fn projection_of_an_unrecognisable_response_yields_no_pools_rather_than_failing() {
        // Refusal belongs to the parser, which reports why. Projection only selects fields.
        let projected = project(&serde_json::json!({"unexpected": true}));
        assert_eq!(projected["valuePools"], serde_json::json!([]));
    }

    #[test]
    fn a_node_that_omits_the_flag_reports_no_opinion() {
        // The flag is absent from older responses. Nothing reads it as a tracking signal,
        // but its absence must still be distinguishable from a reported `false`.
        let without = r#"{
            "hash": "00",
            "height": 1,
            "valuePools": [{"id": "orchard", "chainValueZat": 5}]
        }"#;
        let state = parse(without.as_bytes()).unwrap();
        assert_eq!(state.pools.monitored(Pool::Orchard), None);
    }

    #[test]
    fn the_block_hash_is_carried_through() {
        let state = parse(COMPLETE.as_bytes()).unwrap();
        assert_eq!(
            state.block_hash,
            "0000000000aaaabbbbccccddddeeeeffff00001111222233334444555566667777"
        );
    }
}
