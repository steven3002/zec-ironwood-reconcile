//! Reading captured node responses about value pools.
//!
//! Evidence preserves the node's response as it was served, rather than a form this crate
//! finds convenient. This module is the boundary that turns that response into the domain
//! type used for comparison.
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
    }

    Ok(CapturedBlockState {
        height,
        block_hash: response.hash,
        pools,
    })
}

/// Parses a captured response and requires it to carry every reconstructed pool balance.
///
/// This is the guard against reconciling against absent values. A node serving empty pool
/// data at arbitrary heights — documented Zebra behaviour during a database upgrade — would
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

    #[test]
    fn the_block_hash_is_carried_through() {
        let state = parse(COMPLETE.as_bytes()).unwrap();
        assert_eq!(
            state.block_hash,
            "0000000000aaaabbbbccccddddeeeeffff00001111222233334444555566667777"
        );
    }
}
