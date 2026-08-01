//! Conditions under which a capture must not proceed.
//!
//! Each guard exists because a node can serve a response that looks healthy while being
//! unusable as evidence. Without them the tool would produce a confident report from data
//! that cannot support one, which is the single worst outcome available to it: a wrong
//! answer is more damaging than no answer, because a wrong answer gets published.
//!
//! Every function here is pure. They take what a node said and decide, so they are tested
//! without a node and cannot behave differently in production than in a test.

use crate::domain::height::{BlockHeight, HeightInterval};
use crate::domain::network::Network;
use crate::error::ReconcileError;
use crate::evidence::pool_state_file::CapturedBlockState;
use crate::rpc::dto::{ChainInfo, NodeInfo};

/// Name of the upgrade this tool reconciles, as it appears in a node's upgrade table.
pub const IRONWOOD_UPGRADE_NAME: &str = "NU6.3";

/// The `chain` value a node reports for a network.
///
/// These are not the strings `--network` accepts. A node says `main` and `test`; this tool
/// says `mainnet` and `testnet`. Comparing the two directly would fail on every capture.
pub const fn node_chain_name(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "main",
        Network::Testnet => "test",
    }
}

/// Something worth telling the operator that does not invalidate the capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Advisory {
    pub id: &'static str,
    pub detail: String,
}

impl Advisory {
    fn new(id: &'static str, detail: String) -> Self {
        Self { id, detail }
    }
}

/// Confirms the node is serving the network the caller asked for.
///
/// Checked against two independent fields, because a capture pointed at the wrong network
/// produces a report that is internally consistent and entirely meaningless.
pub fn check_network(
    expected: Network,
    chain_info: &ChainInfo,
    node_info: &NodeInfo,
) -> Result<(), ReconcileError> {
    let expected_chain = node_chain_name(expected);
    if chain_info.chain != expected_chain {
        return Err(ReconcileError::NetworkMismatch {
            expected: format!("{expected} ({expected_chain})"),
            actual: chain_info.chain.clone(),
        });
    }

    let expects_testnet = matches!(expected, Network::Testnet);
    if node_info.testnet != expects_testnet {
        return Err(ReconcileError::NetworkMismatch {
            expected: format!("{expected} (getinfo testnet={expects_testnet})"),
            actual: format!("getinfo testnet={}", node_info.testnet),
        });
    }

    Ok(())
}

/// Confirms the node's activation schedule matches the constants compiled into this build.
///
/// The tool encodes NU6.3's branch identifier and activation heights so that a bundle can
/// be validated offline without trusting the node that produced it. That only holds if the
/// two agree at capture time, so the disagreement is caught here rather than becoming a
/// silent difference of opinion inside a published report.
pub fn check_activation(
    expected: Network,
    chain_info: &ChainInfo,
    asserted_height: Option<u32>,
) -> Result<(), ReconcileError> {
    let branch_id = expected.nu6_3_branch_id();
    let compiled_height = expected.ironwood_activation_height().get();

    if let Some(asserted) = asserted_height
        && asserted != compiled_height
    {
        return Err(ReconcileError::ActivationMismatch {
            reason: format!(
                "--expected-activation-height {asserted} does not match the {IRONWOOD_UPGRADE_NAME} \
                 activation height this build implements for {expected}, {compiled_height}"
            ),
        });
    }

    // A node that publishes no upgrade table at all cannot be cross-checked. That is a
    // limitation of the response, not a disagreement, so it is reported as unusable
    // evidence rather than as a mismatch.
    if chain_info.upgrades.is_empty() {
        return Err(ReconcileError::CaptureIncomplete {
            reason: "the node published no network upgrade table, so its activation schedule \
                     could not be cross-checked"
                .to_owned(),
        });
    }

    let upgrade = chain_info.upgrade_by_branch_id(branch_id).ok_or_else(|| {
        ReconcileError::ActivationMismatch {
            reason: format!(
                "the node does not know consensus branch {branch_id:#010x} \
                 ({IRONWOOD_UPGRADE_NAME}); it predates the upgrade and cannot report the \
                 Ironwood pool"
            ),
        }
    })?;

    if upgrade.activationheight != compiled_height {
        return Err(ReconcileError::ActivationMismatch {
            reason: format!(
                "the node activates {IRONWOOD_UPGRADE_NAME} on {expected} at height {}, but this \
                 build implements {compiled_height}",
                upgrade.activationheight
            ),
        });
    }

    Ok(())
}

/// Refuses a capture whose end height is too close to the node's tip.
///
/// Zcash offers no protocol finality. A block near the tip can be reorganised out after it
/// was read, which would leave a published bundle describing a chain that no longer exists.
/// The distance is measured against the node's validated tip, never against its estimate of
/// the network tip, which on a syncing node is derived from the local tip's timestamp and
/// so tracks the node rather than the chain.
pub fn check_tip_distance(
    tip: BlockHeight,
    end_height: BlockHeight,
    minimum_distance: u32,
) -> Result<(), ReconcileError> {
    let required = end_height
        .get()
        .checked_add(minimum_distance)
        .ok_or(ReconcileError::ArithmeticOverflow)?;

    if tip.get() < required {
        let behind = required.saturating_sub(tip.get());
        return Err(ReconcileError::CaptureIncomplete {
            reason: format!(
                "the node's tip is height {tip}, which is not {minimum_distance} blocks beyond the \
                 requested end height {end_height}; wait for {behind} more block(s) or lower \
                 --tip-distance"
            ),
        });
    }

    Ok(())
}

/// Confirms the end block was not reorganised out while the interval was being read.
///
/// Run after the interval completes, against a fresh query of the same height. A capture
/// spans many requests and the chain does not hold still for it.
pub fn check_end_block_unchanged(
    height: BlockHeight,
    hash_before: &str,
    hash_after: &str,
) -> Result<(), ReconcileError> {
    if hash_before != hash_after {
        return Err(ReconcileError::CaptureIncomplete {
            reason: format!(
                "height {height} held block {hash_before} when the capture began and block \
                 {hash_after} when it finished; the chain was reorganised and the evidence is \
                 not a single consistent chain"
            ),
        });
    }
    Ok(())
}

/// Confirms a node's response can support a comparison at this height.
///
/// Rejects the case Zebra documents during a database upgrade, in which pool values are
/// absent or empty at arbitrary heights while the node otherwise appears healthy.
pub fn check_pool_state_usable(state: &CapturedBlockState) -> Result<(), ReconcileError> {
    let missing = state.pools.missing_reconstructed_balances();
    if missing.is_empty() {
        return Ok(());
    }

    let names: Vec<&str> = missing.iter().map(|pool| pool.rpc_id()).collect();
    Err(ReconcileError::CaptureIncomplete {
        reason: format!(
            "the node reported no balance for {} at height {}; a node serves empty pool values \
             while a database upgrade is in progress, so this capture cannot support a comparison",
            names.join(", "),
            state.height
        ),
    })
}

/// Confirms a response describes the height it was requested for.
pub fn check_height_matches(
    requested: BlockHeight,
    state: &CapturedBlockState,
) -> Result<(), ReconcileError> {
    if state.height != requested {
        return Err(ReconcileError::CaptureIncomplete {
            reason: format!(
                "height {requested} was requested but the node answered for height {}",
                state.height
            ),
        });
    }
    Ok(())
}

/// Observations about one captured block that are worth recording but do not invalidate it.
///
/// Everything here is a statement about the single height `state` describes. A property of
/// the interval as a whole belongs in [`interval_advisories`], which is given the interval;
/// deriving one from a single block was how the tool came to state that no Ironwood value
/// could exist in an interval that in fact ran well past activation.
pub fn advisories(state: &CapturedBlockState) -> Vec<Advisory> {
    let mut advisories = Vec::new();

    let empty = state.pools.empty_reconstructed_pools();
    if !empty.is_empty() {
        let names: Vec<&str> = empty.iter().map(|pool| pool.rpc_id()).collect();
        advisories.push(Advisory::new(
            "pool_balance_is_zero",
            format!(
                "the node reports a balance of zero for {} at height {}, so any comparison \
                 against that pool at this height is a comparison against zero",
                names.join(", "),
                state.height
            ),
        ));
    }

    advisories
}

/// Observations about the interval being captured, as opposed to any one block in it.
///
/// The Ironwood pool cannot hold value below activation, so an interval lying entirely
/// below it can contain none. That is a claim about the interval's **end**: an interval
/// that starts below activation and runs past it contains Ironwood-capable heights, and
/// saying otherwise would put a false statement in front of whoever captured the most
/// interesting interval the tool supports.
pub fn interval_advisories(network: Network, interval: HeightInterval) -> Vec<Advisory> {
    let mut advisories = Vec::new();

    if !network.is_post_activation(interval.end_height()) {
        advisories.push(Advisory::new(
            "interval_precedes_activation",
            format!(
                "the interval ends at height {}, below the {IRONWOOD_UPGRADE_NAME} activation \
                 height {} on {network}, so no Ironwood value can exist in it",
                interval.end_height(),
                network.ironwood_activation_height()
            ),
        ));
    }

    advisories
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pool::Pool;
    use crate::domain::zatoshi::Zatoshi;
    use crate::rpc::dto::Upgrade;
    use std::collections::BTreeMap;

    fn node_info(testnet: bool) -> NodeInfo {
        NodeInfo {
            build: "v6.2.3".to_owned(),
            subversion: "/Zebra:6.2.3/".to_owned(),
            protocolversion: 170_160,
            blocks: 4_200_000,
            testnet,
        }
    }

    fn chain_info(chain: &str, activation_height: u32) -> ChainInfo {
        let mut upgrades = BTreeMap::new();
        upgrades.insert(
            "37a5165b".to_owned(),
            Upgrade {
                name: IRONWOOD_UPGRADE_NAME.to_owned(),
                activationheight: activation_height,
                status: "active".to_owned(),
            },
        );

        ChainInfo {
            chain: chain.to_owned(),
            blocks: 4_200_000,
            bestblockhash: "aa".repeat(32),
            estimatedheight: Some(4_200_000),
            upgrades,
            consensus: None,
        }
    }

    fn pool_state(height: u32) -> CapturedBlockState {
        use crate::domain::pool_state::ReportedPoolState;
        CapturedBlockState {
            height: BlockHeight::new(height),
            block_hash: "bb".repeat(32),
            pools: ReportedPoolState::new(BlockHeight::new(height))
                .with_balance(Pool::Orchard, Zatoshi::from_raw(1))
                .with_balance(Pool::Ironwood, Zatoshi::from_raw(2))
                .with_monitored(Pool::Orchard, true)
                .with_monitored(Pool::Ironwood, true),
        }
    }

    #[test]
    fn the_node_chain_names_are_not_the_command_line_names() {
        // Recorded from a live node: the field reads `test`, not `testnet`.
        assert_eq!(node_chain_name(Network::Mainnet), "main");
        assert_eq!(node_chain_name(Network::Testnet), "test");
        assert_ne!(node_chain_name(Network::Testnet), Network::Testnet.name());
    }

    #[test]
    fn a_matching_network_is_accepted() {
        assert!(
            check_network(
                Network::Testnet,
                &chain_info("test", 4_134_000),
                &node_info(true),
            )
            .is_ok()
        );
        assert!(
            check_network(
                Network::Mainnet,
                &chain_info("main", 3_428_143),
                &node_info(false),
            )
            .is_ok()
        );
    }

    #[test]
    fn a_mismatched_chain_is_refused() {
        let error = check_network(
            Network::Mainnet,
            &chain_info("test", 3_428_143),
            &node_info(true),
        )
        .unwrap_err();
        assert!(matches!(error, ReconcileError::NetworkMismatch { .. }));
    }

    #[test]
    fn a_chain_field_that_contradicts_getinfo_is_refused() {
        // Both fields must agree; either one alone could be misreported.
        let error = check_network(
            Network::Testnet,
            &chain_info("test", 4_134_000),
            &node_info(false),
        )
        .unwrap_err();
        assert!(matches!(error, ReconcileError::NetworkMismatch { .. }));
    }

    #[test]
    fn an_unknown_chain_name_is_refused() {
        let error = check_network(
            Network::Mainnet,
            &chain_info("regtest", 3_428_143),
            &node_info(false),
        )
        .unwrap_err();
        assert!(matches!(error, ReconcileError::NetworkMismatch { .. }));
    }

    #[test]
    fn an_agreeing_activation_schedule_is_accepted() {
        assert!(check_activation(Network::Testnet, &chain_info("test", 4_134_000), None).is_ok());
        assert!(check_activation(Network::Mainnet, &chain_info("main", 3_428_143), None).is_ok());
    }

    #[test]
    fn a_node_activating_ironwood_elsewhere_is_refused() {
        let error =
            check_activation(Network::Mainnet, &chain_info("main", 9_999_999), None).unwrap_err();
        assert!(matches!(error, ReconcileError::ActivationMismatch { .. }));
        assert!(error.to_string().contains("3428143"), "{error}");
    }

    #[test]
    fn a_node_without_the_ironwood_branch_is_refused() {
        let mut info = chain_info("main", 3_428_143);
        info.upgrades.remove("37a5165b");
        info.upgrades.insert(
            "c8e71055".to_owned(),
            Upgrade {
                name: "NU6".to_owned(),
                activationheight: 2_976_000,
                status: "active".to_owned(),
            },
        );

        let error = check_activation(Network::Mainnet, &info, None).unwrap_err();
        assert!(matches!(error, ReconcileError::ActivationMismatch { .. }));
        assert!(error.to_string().contains("0x37a5165b"), "{error}");
    }

    #[test]
    fn a_node_publishing_no_upgrade_table_cannot_be_cross_checked() {
        let mut info = chain_info("main", 3_428_143);
        info.upgrades.clear();

        let error = check_activation(Network::Mainnet, &info, None).unwrap_err();
        assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
    }

    #[test]
    fn an_asserted_activation_height_must_match_the_build() {
        assert!(
            check_activation(
                Network::Mainnet,
                &chain_info("main", 3_428_143),
                Some(3_428_143)
            )
            .is_ok()
        );
        assert!(matches!(
            check_activation(
                Network::Mainnet,
                &chain_info("main", 3_428_143),
                Some(3_000_000)
            ),
            Err(ReconcileError::ActivationMismatch { .. })
        ));
    }

    #[test]
    fn a_tip_far_enough_beyond_the_interval_is_accepted() {
        assert!(check_tip_distance(BlockHeight::new(1_100), BlockHeight::new(1_000), 100).is_ok());
        assert!(check_tip_distance(BlockHeight::new(5_000), BlockHeight::new(1_000), 100).is_ok());
    }

    #[test]
    fn a_tip_too_close_to_the_interval_is_refused() {
        let error =
            check_tip_distance(BlockHeight::new(1_099), BlockHeight::new(1_000), 100).unwrap_err();
        assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
        // The operator is told how much longer to wait rather than only that it failed.
        assert!(error.to_string().contains("1 more block"), "{error}");
    }

    #[test]
    fn an_end_height_beyond_the_tip_is_refused() {
        assert!(check_tip_distance(BlockHeight::new(500), BlockHeight::new(1_000), 0).is_err());
    }

    #[test]
    fn a_zero_tip_distance_still_requires_the_block_to_exist() {
        assert!(check_tip_distance(BlockHeight::new(1_000), BlockHeight::new(1_000), 0).is_ok());
    }

    #[test]
    fn a_tip_distance_that_would_overflow_is_reported_rather_than_wrapping() {
        assert!(matches!(
            check_tip_distance(BlockHeight::new(1), BlockHeight::new(u32::MAX), 100),
            Err(ReconcileError::ArithmeticOverflow)
        ));
    }

    #[test]
    fn an_unchanged_end_block_is_accepted() {
        assert!(check_end_block_unchanged(BlockHeight::new(1), "aa", "aa").is_ok());
    }

    #[test]
    fn a_changed_end_block_is_refused_as_a_reorganisation() {
        let error = check_end_block_unchanged(BlockHeight::new(3_428_143), "aa", "bb").unwrap_err();
        assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
        assert!(error.to_string().contains("reorganised"), "{error}");
    }

    #[test]
    fn a_complete_pool_response_is_accepted() {
        assert!(check_pool_state_usable(&pool_state(3_428_143)).is_ok());
    }

    #[test]
    fn a_response_missing_a_reconstructed_pool_is_refused() {
        use crate::domain::pool_state::ReportedPoolState;
        let mut state = pool_state(3_428_143);
        state.pools = ReportedPoolState::new(BlockHeight::new(3_428_143))
            .with_balance(Pool::Orchard, Zatoshi::ZERO);

        let error = check_pool_state_usable(&state).unwrap_err();
        assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
        assert!(error.to_string().contains("ironwood"), "{error}");
    }

    #[test]
    fn a_response_for_the_wrong_height_is_refused() {
        let error = check_height_matches(BlockHeight::new(10), &pool_state(11)).unwrap_err();
        assert!(matches!(error, ReconcileError::CaptureIncomplete { .. }));
    }

    #[test]
    fn an_empty_pool_raises_an_advisory_rather_than_a_failure() {
        // An empty pool is usable evidence — the comparison is simply against zero. It is
        // worth saying so, but it is not a reason to refuse the capture.
        let mut state = pool_state(3_428_143);
        state.pools = state.pools.with_balance(Pool::Ironwood, Zatoshi::ZERO);

        assert!(check_pool_state_usable(&state).is_ok());
        let ids: Vec<&str> = advisories(&state)
            .iter()
            .map(|advisory| advisory.id)
            .collect();
        assert!(ids.contains(&"pool_balance_is_zero"), "{ids:?}");
    }

    fn advisory_ids(network: Network, start: u32, end: u32) -> Vec<&'static str> {
        let interval = HeightInterval::new(BlockHeight::new(start), BlockHeight::new(end)).unwrap();
        interval_advisories(network, interval)
            .iter()
            .map(|advisory| advisory.id)
            .collect()
    }

    #[test]
    fn an_interval_lying_entirely_before_activation_raises_an_advisory() {
        let ids = advisory_ids(Network::Mainnet, 3_000_000, 3_000_100);
        assert!(ids.contains(&"interval_precedes_activation"), "{ids:?}");
    }

    #[test]
    fn an_interval_reaching_past_activation_is_not_called_pre_activation() {
        // The interval starts below activation and ends above it, so it does contain
        // Ironwood-capable heights. Deciding this from the anchor alone told the operator
        // that no Ironwood value could exist in exactly the interval where it first can.
        let ids = advisory_ids(Network::Mainnet, 3_428_142, 3_428_200);
        assert!(!ids.contains(&"interval_precedes_activation"), "{ids:?}");
    }

    #[test]
    fn an_interval_starting_exactly_at_activation_is_not_called_pre_activation() {
        let ids = advisory_ids(Network::Mainnet, 3_428_143, 3_428_150);
        assert!(!ids.contains(&"interval_precedes_activation"), "{ids:?}");
    }

    #[test]
    fn a_healthy_post_activation_capture_raises_no_advisories() {
        assert!(advisories(&pool_state(3_428_200)).is_empty());
    }
}
