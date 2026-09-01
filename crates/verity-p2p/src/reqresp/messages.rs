//! The req/resp envelope types: protocol IDs, request containers, and response shapes.
//!
//! Container shapes transcribe leanSpec `node/networking/reqresp/message.py` field for
//! field, in leanSpec's order. Blocks inside responses are NOT decoded here — a chunk
//! crosses this crate as raw SSZ bytes, and the sync service decodes and structurally
//! validates it (`docs/design/sync.md`, Decision 2).

use libssz_derive::{HashTreeRoot, SszDecode, SszEncode};
use libssz_types::SszList;
use verity_types::{Bytes32, Checkpoint, Slot};

use crate::config::MAX_REQUEST_BLOCKS;

/// Protocol ID for the status handshake.
pub const STATUS_PROTOCOL_V1: &str = "/leanconsensus/req/status/1/ssz_snappy";
/// Protocol ID for requesting blocks by root.
pub const BLOCKS_BY_ROOT_PROTOCOL_V1: &str = "/leanconsensus/req/blocks_by_root/1/ssz_snappy";
/// Protocol ID for requesting a slot range of blocks.
pub const BLOCKS_BY_RANGE_PROTOCOL_V1: &str = "/leanconsensus/req/blocks_by_range/1/ssz_snappy";

/// The three req/resp protocols, each negotiated by its own behaviour instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    /// The status handshake.
    Status,
    /// Blocks by root.
    BlocksByRoot,
    /// Blocks by slot range.
    BlocksByRange,
}

impl AsRef<str> for Protocol {
    fn as_ref(&self) -> &str {
        match self {
            Self::Status => STATUS_PROTOCOL_V1,
            Self::BlocksByRoot => BLOCKS_BY_ROOT_PROTOCOL_V1,
            Self::BlocksByRange => BLOCKS_BY_RANGE_PROTOCOL_V1,
        }
    }
}

/// The status handshake payload: the sender's finalized and head checkpoints. 80 bytes,
/// deliberately validator-set-free.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct Status {
    /// The sender's latest finalized checkpoint.
    pub finalized: Checkpoint,
    /// The sender's current head.
    pub head: Checkpoint,
}

/// The roots list inside a [`BlocksByRootRequest`].
pub type RequestedBlockRoots = SszList<Bytes32, MAX_REQUEST_BLOCKS>;

/// Request for specific blocks by their roots. Missing roots are skipped silently —
/// a partial response is legal.
#[derive(Debug, Clone, Default, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlocksByRootRequest {
    /// The roots requested.
    pub roots: RequestedBlockRoots,
}

/// Request for a contiguous slot range of blocks. leanSpec omits the legacy `step` field:
/// the stride is always one slot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, SszEncode, SszDecode, HashTreeRoot)]
pub struct BlocksByRangeRequest {
    /// First slot of the range.
    pub start_slot: Slot,
    /// Number of slots covered, at most [`MAX_REQUEST_BLOCKS`].
    pub count: u64,
}

/// An outbound or inbound request, tagged by protocol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// A status handshake.
    Status(Status),
    /// A blocks-by-root request.
    BlocksByRoot(BlocksByRootRequest),
    /// A blocks-by-range request.
    BlocksByRange(BlocksByRangeRequest),
}

impl Request {
    /// The protocol this request travels on, hence the behaviour that must send it.
    #[must_use]
    pub fn protocol(&self) -> Protocol {
        match self {
            Self::Status(_) => Protocol::Status,
            Self::BlocksByRoot(_) => Protocol::BlocksByRoot,
            Self::BlocksByRange(_) => Protocol::BlocksByRange,
        }
    }
}

/// A response, either the protocol's success payload or the peer's error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// The responder's status, answering a status handshake.
    Status(Status),
    /// Block chunks answering a blocks request, one raw SSZ `SignedBlock` each, in the
    /// order the responder sent them. Possibly empty — an empty response is how a
    /// responder says "none of those roots" or "no blocks in that range".
    Blocks(Vec<Vec<u8>>),
    /// The peer refused or failed the request.
    Error {
        /// The response code, never `SUCCESS`.
        code: ErrorCode,
        /// The peer's human-readable explanation. Bounded, possibly empty, and
        /// untrusted — for logs only.
        message: String,
    },
}

/// Non-success response codes.
///
/// `RESOURCE_UNAVAILABLE` deserves its asymmetry: it is the *legal* answer for history
/// below the serving window, and the sync service's scoring table treats it as neutral
/// (`docs/design/sync.md`, Decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The request was malformed or out of bounds.
    InvalidRequest,
    /// The responder failed internally.
    ServerError,
    /// The responder does not have the requested data.
    ResourceUnavailable,
}

/// The wire byte of the success response code.
pub const RESPONSE_CODE_SUCCESS: u8 = 0;

impl ErrorCode {
    /// The wire byte of this code.
    #[must_use]
    pub fn as_byte(self) -> u8 {
        match self {
            Self::InvalidRequest => 1,
            Self::ServerError => 2,
            Self::ResourceUnavailable => 3,
        }
    }

    /// Classifies a non-success wire byte, degrading gracefully exactly as leanSpec's
    /// codec does: unknown codes 4–127 read as a server error, 128–255 as an invalid
    /// request.
    ///
    /// Returns `None` for [`RESPONSE_CODE_SUCCESS`], which is not an error.
    #[must_use]
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            RESPONSE_CODE_SUCCESS => None,
            1 => Some(Self::InvalidRequest),
            2 => Some(Self::ServerError),
            3 => Some(Self::ResourceUnavailable),
            4..=127 => Some(Self::ServerError),
            128..=255 => Some(Self::InvalidRequest),
        }
    }
}

#[cfg(test)]
mod tests {
    use libssz::{SszDecode, SszEncode};

    use super::*;

    #[test]
    fn should_encode_status_as_eighty_bytes() {
        let status = Status {
            finalized: Checkpoint {
                root: [1u8; 32],
                slot: Slot(9),
            },
            head: Checkpoint {
                root: [2u8; 32],
                slot: Slot(12),
            },
        };
        let bytes = status.to_ssz();
        assert_eq!(bytes.len(), 80);
        assert_eq!(Status::from_ssz_bytes(&bytes).expect("round trip"), status);
    }

    #[test]
    fn should_round_trip_blocks_by_range_request() {
        let request = BlocksByRangeRequest {
            start_slot: Slot(3600),
            count: 64,
        };
        let bytes = request.to_ssz();
        assert_eq!(bytes.len(), 16);
        assert_eq!(
            BlocksByRangeRequest::from_ssz_bytes(&bytes).expect("round trip"),
            request
        );
    }

    #[test]
    fn should_degrade_unknown_response_codes_as_leanspec_does() {
        assert_eq!(ErrorCode::from_byte(0), None);
        assert_eq!(ErrorCode::from_byte(1), Some(ErrorCode::InvalidRequest));
        assert_eq!(ErrorCode::from_byte(2), Some(ErrorCode::ServerError));
        assert_eq!(
            ErrorCode::from_byte(3),
            Some(ErrorCode::ResourceUnavailable)
        );
        assert_eq!(ErrorCode::from_byte(64), Some(ErrorCode::ServerError));
        assert_eq!(ErrorCode::from_byte(200), Some(ErrorCode::InvalidRequest));
    }
}
