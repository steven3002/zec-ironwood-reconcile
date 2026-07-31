//! Networks and the NU6.3 activation context.
//!
//! Activation heights and the consensus branch identifier are specified by ZIP 258
//! (Deployment of the NU6.3 Network Upgrade). They are encoded here rather than read from
//! a node so that an evidence bundle can be validated against the protocol constants
//! offline, without trusting the node that produced it.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::domain::height::BlockHeight;
use crate::error::ReconcileError;

/// Consensus branch identifier for NU6.3 ("Ironwood"), per ZIP 258.
pub const NU6_3_BRANCH_ID: u32 = 0x37A5_165B;

/// Mainnet activation height for NU6.3, per ZIP 258.
pub const NU6_3_MAINNET_ACTIVATION: u32 = 3_428_143;

/// Testnet activation height for NU6.3, per ZIP 258.
pub const NU6_3_TESTNET_ACTIVATION: u32 = 4_134_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet => "testnet",
        }
    }

    /// Height at which NU6.3 activates and the Ironwood pool begins to exist.
    pub const fn ironwood_activation_height(self) -> BlockHeight {
        match self {
            Self::Mainnet => BlockHeight::new(NU6_3_MAINNET_ACTIVATION),
            Self::Testnet => BlockHeight::new(NU6_3_TESTNET_ACTIVATION),
        }
    }

    /// Consensus branch identifier expected for transactions at or after activation.
    pub const fn nu6_3_branch_id(self) -> u32 {
        NU6_3_BRANCH_ID
    }

    pub const fn is_post_activation(self, height: BlockHeight) -> bool {
        height.get() >= self.ironwood_activation_height().get()
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Network {
    type Err = ReconcileError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "mainnet" => Ok(Self::Mainnet),
            "testnet" => Ok(Self::Testnet),
            other => Err(ReconcileError::UnknownNetwork {
                value: other.to_owned(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_heights_match_zip_258() {
        assert_eq!(
            Network::Mainnet.ironwood_activation_height().get(),
            3_428_143
        );
        assert_eq!(
            Network::Testnet.ironwood_activation_height().get(),
            4_134_000
        );
    }

    #[test]
    fn branch_id_matches_zip_258() {
        assert_eq!(NU6_3_BRANCH_ID, 0x37A5_165B);
        assert_eq!(NU6_3_BRANCH_ID, 933_566_043);
    }

    #[test]
    fn activation_boundary_is_inclusive() {
        let network = Network::Mainnet;
        let activation = network.ironwood_activation_height();
        assert!(network.is_post_activation(activation));
        assert!(!network.is_post_activation(activation.checked_previous().unwrap()));
    }

    #[test]
    fn parses_known_networks() {
        assert_eq!("mainnet".parse::<Network>().unwrap(), Network::Mainnet);
        assert_eq!("testnet".parse::<Network>().unwrap(), Network::Testnet);
    }

    #[test]
    fn rejects_unknown_networks() {
        assert!(matches!(
            "regtest".parse::<Network>(),
            Err(ReconcileError::UnknownNetwork { .. })
        ));
    }

    #[test]
    fn round_trips_through_display() {
        for network in [Network::Mainnet, Network::Testnet] {
            assert_eq!(network.name().parse::<Network>().unwrap(), network);
        }
    }
}
