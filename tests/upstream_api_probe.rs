//! Pins the upstream API contract this crate depends on.
//!
//! The value-pool reconstruction rests on a small number of guarantees from the Zcash
//! crates: that both shielded bundles relevant to NU6.3 are reachable from a parsed
//! transaction, that a bundle exposes its value balance, and that the NU6.3 consensus
//! branch identifier matches the value specified by ZIP 258.
//!
//! These tests fail at compile time or assertion time if an upstream release changes any
//! of those guarantees, which is preferable to discovering it through a silently altered
//! accounting result.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use orchard::bundle::Bundle;
use orchard::value::ValueSum;
use zcash_primitives::transaction::Transaction;
use zcash_protocol::consensus::BranchId;

/// The consensus branch identifier for NU6.3 ("Ironwood"), per ZIP 258.
const ZIP_258_NU6_3_BRANCH_ID: u32 = 0x37A5_165B;

#[test]
fn nu6_3_branch_id_matches_zip_258() {
    assert_eq!(u32::from(BranchId::Nu6_3), ZIP_258_NU6_3_BRANCH_ID);
}

#[test]
fn branch_id_round_trips_through_its_numeric_encoding() {
    let branch_id = BranchId::try_from(ZIP_258_NU6_3_BRANCH_ID)
        .expect("NU6.3 branch id should be recognised by zcash_protocol");
    assert_eq!(branch_id, BranchId::Nu6_3);
}

/// Pins the exact expression the parse layer uses to obtain a pool's value balance.
///
/// Both pools are reached through the same accessor shape, because the Ironwood bundle
/// reuses the Orchard wire layout. The functions are never called; requiring them to
/// type-check is the point.
#[allow(dead_code)]
fn orchard_value_balance_chain(tx: &Transaction) -> Option<i64> {
    tx.orchard_bundle()
        .map(|bundle| (*bundle.value_balance()).into())
}

#[allow(dead_code)]
fn ironwood_value_balance_chain(tx: &Transaction) -> Option<i64> {
    tx.ironwood_bundle()
        .map(|bundle| (*bundle.value_balance()).into())
}

/// Confirms a bundle exposes a value balance independently of how it was obtained.
#[allow(dead_code)]
fn value_balance_is_reachable<A: orchard::bundle::Authorization>(
    bundle: &Bundle<A, ValueSum>,
) -> &ValueSum {
    bundle.value_balance()
}

#[test]
fn transaction_exposes_a_consensus_branch_id_accessor() {
    // Reading the branch identifier back from a parsed transaction is how the parse layer
    // detects a bundle labelled with the wrong upgrade. Version 5 and 6 transactions carry
    // it in their own bytes rather than taking it from the caller.
    fn _accessor(tx: &Transaction) -> BranchId {
        tx.consensus_branch_id()
    }
}
