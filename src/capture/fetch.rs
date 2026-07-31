//! Per-height retrieval.
//!
//! Two files are stored for every height: the block's consensus bytes, which are what the
//! reconstruction is computed from, and the node's reported value pools, which are the
//! figures the reconstruction is checked against. Storing raw bytes rather than a node's
//! decoded view is what keeps a bundle meaningful across node versions.
//!
//! Whatever is retrieved is validated before it is written, and whatever is reused is
//! validated after it is read, so the same conditions hold whether a capture ran once or
//! was resumed.

use crate::domain::height::BlockHeight;
use crate::error::ReconcileError;
use crate::evidence::manifest::Encoding;
use crate::evidence::pool_state_file::CapturedBlockState;
use crate::rpc::method::NodeClient;

use crate::capture::plan;
use crate::capture::writer::BundleWriter;

/// One height's evidence, however it was obtained.
#[derive(Debug, Clone)]
pub struct FetchedHeight {
    pub height: BlockHeight,
    pub state: CapturedBlockState,
    /// Whether the files were already present and reused rather than retrieved.
    pub reused: bool,
}

/// Retrieves, or reuses, the evidence for one height.
pub fn height(
    client: &NodeClient<'_>,
    writer: &mut BundleWriter,
    height: BlockHeight,
    block_path: &str,
    pools_path: &str,
) -> Result<FetchedHeight, ReconcileError> {
    let already_present = writer.contains(block_path)? && writer.contains(pools_path)?;

    if already_present {
        let hex = writer.adopt(block_path, Encoding::RawBlockHex)?;
        let pools = writer.adopt(pools_path, Encoding::Json)?;

        validate_block_hex(height, &hex)?;
        let state = parse_stored_pools(height, &pools)?;

        return Ok(FetchedHeight {
            height,
            state,
            reused: true,
        });
    }

    let hex = client.get_block_raw_hex(height)?;
    let response = client.get_block_object(height)?;

    // Validated before anything is written, so a bundle never gains a file the tool would
    // refuse to read back.
    let state = plan::parse_pool_state(height, &response)?;
    let pools = plan::pool_state_bytes(&response)?;
    validate_block_hex(height, hex.as_bytes())?;

    writer.write(block_path, hex.as_bytes(), Encoding::RawBlockHex)?;
    writer.write(pools_path, &pools, Encoding::Json)?;

    Ok(FetchedHeight {
        height,
        state,
        reused: false,
    })
}

/// Reads a stored pool response back through the parser verification will use.
fn parse_stored_pools(
    height: BlockHeight,
    bytes: &[u8],
) -> Result<CapturedBlockState, ReconcileError> {
    let response: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|source| ReconcileError::CaptureIncomplete {
            reason: format!(
                "the stored pool response for height {height} is not readable JSON: {source}"
            ),
        })?;

    plan::parse_pool_state(height, &response)
}

/// Confirms stored block text is a plausible hex encoding of consensus bytes.
///
/// A truncated file cannot reach this point through an interrupted write, which is atomic,
/// but it can arrive in a bundle assembled by hand. The check is cheap and the alternative
/// is a confusing parse failure much later.
fn validate_block_hex(height: BlockHeight, hex: &[u8]) -> Result<(), ReconcileError> {
    if hex.is_empty() {
        return Err(ReconcileError::CaptureIncomplete {
            reason: format!("the stored block for height {height} is empty"),
        });
    }

    if !hex.len().is_multiple_of(2) {
        return Err(ReconcileError::CaptureIncomplete {
            reason: format!(
                "the stored block for height {height} has an odd number of hex digits, so it does \
                 not encode whole bytes"
            ),
        });
    }

    if let Some(position) = hex.iter().position(|byte| !byte.is_ascii_hexdigit()) {
        return Err(ReconcileError::CaptureIncomplete {
            reason: format!(
                "the stored block for height {height} contains a non-hexadecimal character at \
                 offset {position}"
            ),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_block_hex_is_accepted() {
        assert!(validate_block_hex(BlockHeight::new(1), b"0400000f").is_ok());
        assert!(validate_block_hex(BlockHeight::new(1), b"ABCDEF01").is_ok());
    }

    #[test]
    fn empty_block_hex_is_refused() {
        assert!(matches!(
            validate_block_hex(BlockHeight::new(1), b""),
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }

    #[test]
    fn odd_length_block_hex_is_refused() {
        let error = validate_block_hex(BlockHeight::new(7), b"0400000").unwrap_err();
        assert!(error.to_string().contains("odd number"), "{error}");
    }

    #[test]
    fn non_hexadecimal_block_text_is_refused() {
        let error = validate_block_hex(BlockHeight::new(7), b"04zz0000").unwrap_err();
        assert!(error.to_string().contains("offset 2"), "{error}");
    }

    #[test]
    fn a_stored_pool_response_is_parsed_and_checked_against_its_height() {
        let bytes = br#"{
            "hash": "aa",
            "height": 100,
            "valuePools": [
                {"id": "orchard", "chainValueZat": 1, "monitored": true},
                {"id": "ironwood", "chainValueZat": 2, "monitored": true}
            ]
        }"#;

        let state = parse_stored_pools(BlockHeight::new(100), bytes).unwrap();
        assert_eq!(state.block_hash, "aa");

        assert!(matches!(
            parse_stored_pools(BlockHeight::new(101), bytes),
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }

    #[test]
    fn an_unreadable_stored_pool_response_is_refused() {
        assert!(matches!(
            parse_stored_pools(BlockHeight::new(1), b"{ truncated"),
            Err(ReconcileError::CaptureIncomplete { .. })
        ));
    }
}
