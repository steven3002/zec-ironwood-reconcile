//! Wire shapes of the node responses this tool reads.
//!
//! These types describe what a node sends, not what the tool believes. They are kept
//! deliberately permissive about fields the tool does not use, so that a node release
//! adding a field does not break capture, and deliberately strict about the fields it does
//! use, so that a missing one is an error rather than a default.
//!
//! Monetary fields are read as `i64` here and converted at the boundary. Node responses
//! also carry the same amounts as floating-point `chainValue` / `valueDelta`; those fields
//! are never read, because a double cannot represent the whole zatoshi range exactly.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A JSON-RPC 2.0 request.
///
/// The identifier is fixed rather than incremented: requests are issued one at a time over
/// a blocking transport, so there is no pipelining for an identifier to disambiguate.
#[derive(Debug, Clone, Serialize)]
pub struct Request<'a> {
    pub jsonrpc: &'static str,
    pub id: u32,
    pub method: &'a str,
    pub params: Vec<serde_json::Value>,
}

impl<'a> Request<'a> {
    pub fn new(method: &'a str, params: Vec<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id: 1,
            method,
            params,
        }
    }
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Deserialize)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

/// A JSON-RPC 2.0 response.
///
/// Zebra returns application errors with HTTP 200 and an `error` member, so the presence of
/// `error`, not the HTTP status, is what distinguishes a failed call.
#[derive(Debug, Clone, Deserialize)]
pub struct Response {
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<ResponseError>,
}

/// `getinfo`: node identity.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeInfo {
    /// Human-readable build string, for example `v6.2.3`.
    pub build: String,
    /// Peer-to-peer subversion string, for example `/Zebra:6.2.3/`.
    pub subversion: String,
    pub protocolversion: u64,
    pub blocks: u32,
    /// Whether the node is running on a test network. Cross-checked against `chain`.
    pub testnet: bool,
}

impl NodeInfo {
    /// Node implementation name, derived from the subversion string.
    ///
    /// The subversion is formatted `/Name:version/`. Anything that does not fit that shape
    /// is recorded as unknown rather than guessed at, because the value ends up in the
    /// manifest as a claim about the evidence's provenance.
    pub fn implementation(&self) -> String {
        self.subversion
            .trim_matches('/')
            .split(':')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("unknown")
            .to_ascii_lowercase()
    }

    /// Version string as the node reports it, without a leading `v`.
    pub fn version(&self) -> String {
        self.build.trim_start_matches('v').to_owned()
    }
}

/// One entry of the `upgrades` map in `getblockchaininfo`, keyed by branch identifier.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Upgrade {
    pub name: String,
    pub activationheight: u32,
    /// `active` or `pending`, evaluated against the node's own tip rather than the network
    /// tip. A syncing node reports `pending` for upgrades the network activated long ago,
    /// so this field is recorded but never used as a decision input.
    pub status: String,
}

/// Consensus branch identifiers in effect at the node's tip.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Consensus {
    /// Branch identifier of the node's current tip, as lowercase hexadecimal.
    pub chaintip: String,
    /// Branch identifier the next block must use.
    pub nextblock: String,
}

/// `getblockchaininfo`: chain identity, tip, and the activation table.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChainInfo {
    /// `main` or `test`. Note these are not the same strings as the `--network` values.
    pub chain: String,
    /// Height of the node's validated tip.
    pub blocks: u32,
    pub bestblockhash: String,
    /// The node's estimate of the network tip.
    ///
    /// Derived from the tip's timestamp and the target spacing, not from peer reports, so
    /// on a syncing node it tracks the local tip rather than the real chain. Recorded for
    /// diagnostics; never used as a guard input.
    #[serde(default)]
    pub estimatedheight: Option<u32>,
    #[serde(default)]
    pub upgrades: BTreeMap<String, Upgrade>,
    #[serde(default)]
    pub consensus: Option<Consensus>,
}

impl ChainInfo {
    /// Looks up an upgrade by consensus branch identifier.
    ///
    /// The map is keyed by lowercase hexadecimal without a `0x` prefix.
    pub fn upgrade_by_branch_id(&self, branch_id: u32) -> Option<&Upgrade> {
        self.upgrades.get(&format!("{branch_id:08x}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_response_parses() {
        let response: Response = serde_json::from_str(r#"{"jsonrpc":"2.0","id":1,"result":7}"#)
            .expect("a result response should parse");
        assert!(response.error.is_none());
        assert_eq!(response.result, Some(serde_json::json!(7)));
    }

    #[test]
    fn an_error_response_parses_without_a_result_member() {
        // Recorded from Zebra 6.2.3: application errors omit `result` entirely.
        let response: Response = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-8,"message":"block height not in best chain"}}"#,
        )
        .expect("an error response should parse");
        assert!(response.result.is_none());
        let error = response.error.expect("the error member should be present");
        assert_eq!(error.code, -8);
        assert_eq!(error.message, "block height not in best chain");
    }

    #[test]
    fn a_request_serializes_with_the_protocol_envelope() {
        let request = Request::new(
            "getblock",
            vec![serde_json::json!("280769"), serde_json::json!(1)],
        );
        let text = serde_json::to_string(&request).unwrap();
        assert!(text.contains(r#""jsonrpc":"2.0""#));
        assert!(text.contains(r#""method":"getblock""#));
        assert!(text.contains(r#""params":["280769",1]"#));
    }

    #[test]
    fn the_implementation_name_is_derived_from_the_subversion_string() {
        let info = NodeInfo {
            build: "v6.2.3".to_owned(),
            subversion: "/Zebra:6.2.3/".to_owned(),
            protocolversion: 170_160,
            blocks: 1,
            testnet: true,
        };
        assert_eq!(info.implementation(), "zebra");
        assert_eq!(info.version(), "6.2.3");
    }

    #[test]
    fn an_unrecognisable_subversion_yields_unknown_rather_than_a_guess() {
        let info = NodeInfo {
            build: "custom".to_owned(),
            subversion: String::new(),
            protocolversion: 0,
            blocks: 1,
            testnet: false,
        };
        assert_eq!(info.implementation(), "unknown");
    }

    #[test]
    fn unknown_response_fields_are_tolerated() {
        // A node release adding a field must not break capture.
        let info: NodeInfo = serde_json::from_str(
            r#"{"build":"v6.2.3","subversion":"/Zebra:6.2.3/","protocolversion":170160,
                "blocks":1,"testnet":true,"somethingnew":{"a":1}}"#,
        )
        .expect("an unknown field should be ignored");
        assert_eq!(info.blocks, 1);
    }

    #[test]
    fn upgrades_are_addressed_by_branch_identifier() {
        let info: ChainInfo = serde_json::from_str(
            r#"{"chain":"test","blocks":1,"bestblockhash":"00",
                "upgrades":{"37a5165b":{"name":"NU6.3","activationheight":4134000,"status":"pending"}}}"#,
        )
        .unwrap();

        let upgrade = info
            .upgrade_by_branch_id(0x37A5_165B)
            .expect("NU6.3 should be found by its branch identifier");
        assert_eq!(upgrade.name, "NU6.3");
        assert_eq!(upgrade.activationheight, 4_134_000);
        assert!(info.upgrade_by_branch_id(0).is_none());
    }
}
