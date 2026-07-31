//! The node methods this tool is permitted to call.
//!
//! The set is closed and every member is a read. There is no method here that creates,
//! signs, spends, broadcasts, imports, or touches a wallet, and none that modifies node
//! state, so pointing this tool at a node cannot change it.
//!
//! # Block access pattern
//!
//! Blocks are requested **by height**, with verbosity `0` for consensus bytes and
//! verbosity `1` for the node's reported value pools. Verbosity `2` is never requested:
//! it expands every transaction, costs far more to serve, and yields nothing this tool
//! reads, since transaction data is taken from the consensus bytes instead.
//!
//! The height is sent as a **string**. Zebra rejects a numeric height parameter with
//! `Invalid params`, which is not obvious from the method's documentation.

use crate::domain::height::BlockHeight;
use crate::error::ReconcileError;
use crate::rpc::client::RpcTransport;
use crate::rpc::dto;

/// Verbosity that returns a block as hex-encoded consensus bytes.
pub const VERBOSITY_RAW: u32 = 0;

/// Verbosity that returns a block as an object including `valuePools`.
pub const VERBOSITY_OBJECT: u32 = 1;

/// A decoded response together with the JSON it came from.
///
/// Evidence preserves the node's response rather than this crate's interpretation of it,
/// so both travel together from the point of retrieval.
#[derive(Debug, Clone)]
pub struct Captured<T> {
    pub value: T,
    pub json: serde_json::Value,
}

/// Typed access to a node.
pub struct NodeClient<'a> {
    transport: &'a dyn RpcTransport,
}

impl<'a> NodeClient<'a> {
    pub const fn new(transport: &'a dyn RpcTransport) -> Self {
        Self { transport }
    }

    pub fn get_info(&self) -> Result<Captured<dto::NodeInfo>, ReconcileError> {
        let json = self.transport.call("getinfo", Vec::new())?;
        let value = decode("getinfo", json.clone())?;
        Ok(Captured { value, json })
    }

    pub fn get_blockchain_info(&self) -> Result<Captured<dto::ChainInfo>, ReconcileError> {
        let json = self.transport.call("getblockchaininfo", Vec::new())?;
        let value = decode("getblockchaininfo", json.clone())?;
        Ok(Captured { value, json })
    }

    /// Retrieves a block's consensus bytes, hex encoded.
    pub fn get_block_raw_hex(&self, height: BlockHeight) -> Result<String, ReconcileError> {
        let json = self
            .transport
            .call("getblock", block_params(height, VERBOSITY_RAW))?;

        let hex = json.as_str().ok_or_else(|| {
            ReconcileError::Rpc(format!(
                "getblock at height {height} with verbosity {VERBOSITY_RAW} did not return hex text"
            ))
        })?;

        if hex.is_empty() || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ReconcileError::Rpc(format!(
                "getblock at height {height} returned text that is not hexadecimal"
            )));
        }

        Ok(hex.to_owned())
    }

    /// Retrieves a block as an object, which is where `valuePools` is reported.
    ///
    /// The response is returned undecoded. Capture stores it and then reads it back with
    /// the same parser offline verification uses, so a response this tool could not
    /// interpret later is refused at the moment it is captured rather than after
    /// publication.
    pub fn get_block_object(
        &self,
        height: BlockHeight,
    ) -> Result<serde_json::Value, ReconcileError> {
        self.transport
            .call("getblock", block_params(height, VERBOSITY_OBJECT))
    }
}

/// Parameters for a `getblock` call.
fn block_params(height: BlockHeight, verbosity: u32) -> Vec<serde_json::Value> {
    vec![
        serde_json::Value::String(height.to_string()),
        serde_json::Value::from(verbosity),
    ]
}

fn decode<T: serde::de::DeserializeOwned>(
    method: &str,
    json: serde_json::Value,
) -> Result<T, ReconcileError> {
    serde_json::from_value(json).map_err(|source| {
        ReconcileError::Rpc(format!(
            "{method} returned a response this build cannot interpret: {source}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Records what was asked and replies with prepared responses.
    struct Recorder {
        calls: RefCell<Vec<(String, Vec<serde_json::Value>)>>,
        reply: serde_json::Value,
    }

    impl Recorder {
        fn new(reply: serde_json::Value) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                reply,
            }
        }

        fn last_params(&self) -> Vec<serde_json::Value> {
            self.calls.borrow().last().cloned().unwrap().1
        }
    }

    impl RpcTransport for Recorder {
        fn call(
            &self,
            method: &str,
            params: Vec<serde_json::Value>,
        ) -> Result<serde_json::Value, ReconcileError> {
            self.calls
                .borrow_mut()
                .push((method.to_owned(), params.clone()));
            Ok(self.reply.clone())
        }
    }

    #[test]
    fn a_block_height_is_sent_as_a_string() {
        // Zebra rejects a numeric height with `Invalid params`.
        let recorder = Recorder::new(serde_json::json!("00ff"));
        NodeClient::new(&recorder)
            .get_block_raw_hex(BlockHeight::new(280_769))
            .unwrap();

        assert_eq!(
            recorder.last_params(),
            vec![serde_json::json!("280769"), serde_json::json!(0)]
        );
    }

    #[test]
    fn verbosity_two_is_never_requested() {
        // Verbosity 2 expands every transaction and is not needed by anything this tool
        // reads. No method may request it.
        let recorder = Recorder::new(serde_json::json!({"hash": "00", "height": 1}));
        let client = NodeClient::new(&recorder);

        client.get_block_object(BlockHeight::new(1)).unwrap();
        let recorder = Recorder::new(serde_json::json!("00ff"));
        NodeClient::new(&recorder)
            .get_block_raw_hex(BlockHeight::new(1))
            .unwrap();

        for (_, params) in recorder.calls.borrow().iter() {
            assert_ne!(
                params.get(1),
                Some(&serde_json::json!(2)),
                "a method requested verbosity 2"
            );
        }
    }

    #[test]
    fn the_object_form_requests_verbosity_one() {
        let recorder = Recorder::new(serde_json::json!({"hash": "00", "height": 1}));
        NodeClient::new(&recorder)
            .get_block_object(BlockHeight::new(1))
            .unwrap();

        assert_eq!(
            recorder.last_params(),
            vec![serde_json::json!("1"), serde_json::json!(1)]
        );
    }

    #[test]
    fn only_read_methods_are_reachable() {
        // The closed method set is asserted here so that adding a mutating call cannot pass
        // review unnoticed.
        let recorder = Recorder::new(serde_json::json!({
            "build": "v6.2.3",
            "subversion": "/Zebra:6.2.3/",
            "protocolversion": 170160,
            "blocks": 1,
            "testnet": true
        }));
        let client = NodeClient::new(&recorder);
        client.get_info().unwrap();

        let methods: Vec<String> = recorder
            .calls
            .borrow()
            .iter()
            .map(|(method, _)| method.clone())
            .collect();
        assert_eq!(methods, vec!["getinfo".to_owned()]);
    }

    #[test]
    fn non_hexadecimal_block_text_is_refused() {
        for reply in [
            serde_json::json!("not hex"),
            serde_json::json!(""),
            serde_json::json!(1234),
            serde_json::json!({"hash": "00"}),
        ] {
            let recorder = Recorder::new(reply.clone());
            assert!(
                NodeClient::new(&recorder)
                    .get_block_raw_hex(BlockHeight::new(1))
                    .is_err(),
                "accepted {reply:?} as block hex"
            );
        }
    }

    #[test]
    fn a_response_missing_a_required_field_is_refused() {
        let recorder = Recorder::new(serde_json::json!({"build": "v6.2.3"}));
        assert!(matches!(
            NodeClient::new(&recorder).get_info(),
            Err(ReconcileError::Rpc(_))
        ));
    }

    #[test]
    fn a_captured_response_keeps_the_json_it_was_decoded_from() {
        let reply = serde_json::json!({
            "build": "v6.2.3",
            "subversion": "/Zebra:6.2.3/",
            "protocolversion": 170160,
            "blocks": 1358400,
            "testnet": true
        });
        let recorder = Recorder::new(reply.clone());
        let captured = NodeClient::new(&recorder).get_info().unwrap();

        assert_eq!(captured.json, reply);
        assert_eq!(captured.value.implementation(), "zebra");
    }
}
