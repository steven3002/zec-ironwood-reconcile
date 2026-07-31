//! Block deserialization and per-block delta extraction.
//!
//! Blocks are read from raw consensus bytes rather than from any node's decoded JSON, so
//! that the reconstruction depends on the chain's own encoding and not on another
//! implementation's presentation of it.

use zcash_primitives::block::Block;
use zcash_protocol::consensus::{BranchId, Network as ConsensusNetwork};

use crate::domain::height::BlockHeight;
use crate::domain::network::Network;
use crate::error::ReconcileError;
use crate::parse::transaction::{self, TransactionPoolDelta};

/// A block's contribution to the reconstructed pools, with the fields needed to verify it
/// belongs where the manifest claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBlock {
    /// Height claimed by the block itself, taken from its coinbase transaction.
    pub claimed_height: BlockHeight,
    /// Hash of the preceding block, in the display byte order used by node RPC responses.
    pub previous_block_hash: String,
    pub transactions: Vec<TransactionPoolDelta>,
}

impl ParsedBlock {
    pub fn transaction_count(&self) -> usize {
        self.transactions.len()
    }
}

/// Decodes a hex-encoded raw block.
pub fn parse_block_hex(
    hex_text: &str,
    network: Network,
    expected_height: BlockHeight,
) -> Result<ParsedBlock, ReconcileError> {
    let bytes =
        hex::decode(hex_text.trim()).map_err(|source| ReconcileError::TransactionParse {
            height: expected_height.get(),
            tx_index: 0,
            reason: format!("block is not valid hexadecimal: {source}"),
        })?;
    parse_block(&bytes, network, expected_height)
}

/// Decodes raw consensus block bytes and extracts every transaction's pool contribution.
///
/// The block's claimed height is checked against the height it was captured as. A block
/// filed under the wrong height would otherwise be attributed to the wrong point in the
/// interval, and the resulting per-height comparison would be meaningless.
pub fn parse_block(
    bytes: &[u8],
    network: Network,
    expected_height: BlockHeight,
) -> Result<ParsedBlock, ReconcileError> {
    let block = Block::read(bytes, &consensus_network(network)).map_err(|source| {
        ReconcileError::TransactionParse {
            height: expected_height.get(),
            tx_index: 0,
            reason: format!("could not decode block: {source}"),
        }
    })?;

    let claimed_height = BlockHeight::new(u32::from(block.claimed_height()));
    if claimed_height != expected_height {
        return Err(ReconcileError::BlockContinuity {
            height: expected_height.get(),
            reason: format!("block claims height {claimed_height}, captured as {expected_height}"),
        });
    }

    let expected_branch_id = branch_id_for(network, claimed_height);

    let mut transactions = Vec::with_capacity(block.vtx().len());
    for (index, tx) in block.vtx().iter().enumerate() {
        let tx_index = u32::try_from(index).map_err(|_| ReconcileError::TransactionParse {
            height: claimed_height.get(),
            tx_index: 0,
            reason: "transaction index exceeds the representable range".to_owned(),
        })?;
        transactions.push(transaction::extract_delta(
            tx,
            claimed_height,
            tx_index,
            expected_branch_id,
        )?);
    }

    Ok(ParsedBlock {
        claimed_height,
        previous_block_hash: display_hash(&block.header().prev_block.0),
        transactions,
    })
}

/// Consensus branch identifier expected for transactions at a height.
///
/// Only the NU6.3 boundary is modelled, because the tool reconciles intervals anchored at
/// or after that activation. A height below activation is reported with the branch
/// identifier of the preceding upgrade so that the mismatch is surfaced rather than masked.
fn branch_id_for(network: Network, height: BlockHeight) -> BranchId {
    if network.is_post_activation(height) {
        BranchId::Nu6_3
    } else {
        BranchId::Nu6_2
    }
}

const fn consensus_network(network: Network) -> ConsensusNetwork {
    match network {
        Network::Mainnet => ConsensusNetwork::MainNetwork,
        Network::Testnet => ConsensusNetwork::TestNetwork,
    }
}

/// Renders a 32-byte internal hash in the reversed order used by node RPC output.
///
/// Zcash inherits Bitcoin's convention of displaying block and transaction hashes in the
/// reverse of their internal byte order. Captured manifests record hashes as the node
/// reports them, so comparison requires the reversed form.
fn display_hash(internal: &[u8; 32]) -> String {
    let mut reversed = *internal;
    reversed.reverse();
    hex::encode(reversed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_rendered_in_reversed_display_order() {
        let mut internal = [0_u8; 32];
        internal[0] = 0xAA;
        internal[31] = 0xBB;

        let rendered = display_hash(&internal);
        assert!(rendered.starts_with("bb"));
        assert!(rendered.ends_with("aa"));
        assert_eq!(rendered.len(), 64);
    }

    #[test]
    fn display_order_is_lowercase_hexadecimal() {
        let rendered = display_hash(&[0xFF_u8; 32]);
        assert_eq!(rendered, "f".repeat(64));
    }

    #[test]
    fn branch_id_follows_the_activation_boundary() {
        let network = Network::Mainnet;
        let activation = network.ironwood_activation_height();

        assert_eq!(branch_id_for(network, activation), BranchId::Nu6_3);
        assert_eq!(
            branch_id_for(network, activation.checked_previous().unwrap()),
            BranchId::Nu6_2
        );
    }

    #[test]
    fn testnet_uses_its_own_activation_height() {
        let network = Network::Testnet;
        assert_eq!(
            branch_id_for(network, BlockHeight::new(4_134_000)),
            BranchId::Nu6_3
        );
        assert_eq!(
            branch_id_for(network, BlockHeight::new(4_133_999)),
            BranchId::Nu6_2
        );
    }

    #[test]
    fn empty_input_fails_rather_than_yielding_an_empty_block() {
        let result = parse_block(&[], Network::Mainnet, BlockHeight::new(3_428_143));
        assert!(matches!(
            result,
            Err(ReconcileError::TransactionParse { .. })
        ));
    }

    #[test]
    fn truncated_input_fails_rather_than_yielding_a_partial_block() {
        // A plausible header prefix that stops partway through.
        let truncated = vec![0_u8; 80];
        let result = parse_block(&truncated, Network::Mainnet, BlockHeight::new(3_428_143));
        assert!(matches!(
            result,
            Err(ReconcileError::TransactionParse { .. })
        ));
    }

    #[test]
    fn non_hexadecimal_input_is_reported_as_such() {
        let result = parse_block_hex("not hex at all", Network::Mainnet, BlockHeight::new(1));
        match result {
            Err(ReconcileError::TransactionParse { reason, .. }) => {
                assert!(
                    reason.contains("hexadecimal"),
                    "unexpected reason: {reason}"
                );
            }
            other => panic!("expected a parse failure, got {other:?}"),
        }
    }

    #[test]
    fn odd_length_hex_is_rejected() {
        let result = parse_block_hex("abc", Network::Mainnet, BlockHeight::new(1));
        assert!(result.is_err());
    }

    #[test]
    fn parse_failures_carry_the_captured_height() {
        let result = parse_block(&[0_u8; 10], Network::Mainnet, BlockHeight::new(3_428_200));
        match result {
            Err(ReconcileError::TransactionParse { height, .. }) => {
                assert_eq!(height, 3_428_200);
            }
            other => panic!("expected a parse failure carrying the height, got {other:?}"),
        }
    }
}
