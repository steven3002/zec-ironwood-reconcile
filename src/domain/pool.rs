//! Chain value pools.
//!
//! Pools are always resolved from RPC responses by their identifier string, never by
//! position in the `valuePools` array. The set has grown over successive network upgrades
//! and one member was renamed (`deferred` became `lockbox`), so positional access would
//! silently read the wrong pool against a newer or older node.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A chain value pool as reported by a Zcash node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pool {
    Transparent,
    Sprout,
    Sapling,
    Orchard,
    Lockbox,
    Ironwood,
}

impl Pool {
    pub const ALL: [Self; 6] = [
        Self::Transparent,
        Self::Sprout,
        Self::Sapling,
        Self::Orchard,
        Self::Lockbox,
        Self::Ironwood,
    ];

    /// Pools this crate reconstructs from transaction data.
    ///
    /// The remaining pools are read from the node where reported, but are not
    /// independently reconstructed and are never the subject of an accounting comparison.
    pub const RECONSTRUCTED: [Self; 2] = [Self::Orchard, Self::Ironwood];

    /// The identifier used by the node's `valuePools` entries.
    pub const fn rpc_id(self) -> &'static str {
        match self {
            Self::Transparent => "transparent",
            Self::Sprout => "sprout",
            Self::Sapling => "sapling",
            Self::Orchard => "orchard",
            Self::Lockbox => "lockbox",
            Self::Ironwood => "ironwood",
        }
    }

    /// Resolves an identifier reported by a node.
    ///
    /// Returns `None` for unrecognized identifiers rather than defaulting, so that a pool
    /// introduced by a future upgrade is surfaced instead of being absorbed into an
    /// existing variant.
    pub fn from_rpc_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pool| pool.rpc_id() == id)
    }
}

impl fmt::Display for Pool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.rpc_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_variant_round_trips_through_its_identifier() {
        for pool in Pool::ALL {
            assert_eq!(Pool::from_rpc_id(pool.rpc_id()), Some(pool));
        }
    }

    #[test]
    fn identifiers_are_unique() {
        let mut ids: Vec<&str> = Pool::ALL.iter().map(|pool| pool.rpc_id()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }

    #[test]
    fn resolves_the_renamed_deferred_pool_by_its_current_identifier() {
        assert_eq!(Pool::from_rpc_id("lockbox"), Some(Pool::Lockbox));
        assert_eq!(Pool::from_rpc_id("deferred"), None);
    }

    #[test]
    fn resolves_ironwood() {
        assert_eq!(Pool::from_rpc_id("ironwood"), Some(Pool::Ironwood));
    }

    #[test]
    fn unknown_identifier_does_not_default() {
        assert_eq!(Pool::from_rpc_id("orchard_v2"), None);
        assert_eq!(Pool::from_rpc_id(""), None);
    }

    #[test]
    fn reconstructed_pools_are_the_accounting_subjects() {
        assert_eq!(Pool::RECONSTRUCTED, [Pool::Orchard, Pool::Ironwood]);
    }
}
