//! Wire constants and the network service configuration.
//!
//! Every constant here transcribes leanSpec `src/lean_spec/node/networking/config.py` (and
//! the gossip parameters in `gossipsub/parameters.py`), read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`. Values the spec leaves free — channel
//! capacities above all — are fields on [`NetworkConfig`] instead, per
//! `docs/design/concurrency.md` "Deliberately deferred to implementation".

use std::time::Duration;

use libp2p::{Multiaddr, identity::Keypair};
use verity_types::SubnetId;

/// Maximum number of blocks in one `BlocksByRoot` or `BlocksByRange` request, and the
/// maximum number of response chunks a requester accepts.
pub const MAX_REQUEST_BLOCKS: usize = 1024;

/// Maximum uncompressed payload size, in bytes, for both gossip messages and req/resp
/// chunks (10 MiB).
pub const MAX_PAYLOAD_SIZE: usize = 10 * 1024 * 1024;

/// Maximum byte length of the UTF-8 message carried by an error response.
pub const MAX_ERROR_MESSAGE_SIZE: usize = 256;

/// The sliding history window, in slots, a `BlocksByRange` responder MUST serve. A request
/// whose `start_slot` falls below `current_slot - MIN_SLOTS_FOR_BLOCK_REQUESTS` is answered
/// with `RESOURCE_UNAVAILABLE`. Enforcing this is the responder's policy, not this crate's;
/// the constant lives here because it is part of the wire contract.
pub const MIN_SLOTS_FOR_BLOCK_REQUESTS: u64 = 3600;

/// Per-request timeout for req/resp, both as requester and as responder.
pub const RESP_TIMEOUT: Duration = Duration::from_secs(10);

/// Message-ID domain prefix for a gossip payload whose snappy decompression succeeded.
pub const MESSAGE_DOMAIN_VALID_SNAPPY: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

/// Message-ID domain prefix for a gossip payload whose snappy decompression failed.
pub const MESSAGE_DOMAIN_INVALID_SNAPPY: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

/// Worst-case compressed size for a payload of `uncompressed` bytes in either snappy
/// format: the framing overhead plus snappy's maximum expansion of one sixth. A chunk
/// whose compressed byte count exceeds this bound for its declared uncompressed length is
/// malformed, whatever its content.
#[must_use]
pub const fn max_compressed_len(uncompressed: usize) -> usize {
    32 + uncompressed + uncompressed / 6
}

/// Configuration for [`crate::service::spawn`].
///
/// The buffer capacities are the tunables `docs/design/concurrency.md` defers to
/// implementation: every buffer is bounded, gossip is dropped (never awaited) when the
/// event buffer is full, and the defaults are sized against per-slot gossip volume
/// (order hundreds).
pub struct NetworkConfig {
    /// The node's libp2p identity. The lean network convention is secp256k1.
    pub keypair: Keypair,
    /// Address to listen on, e.g. `/ip4/0.0.0.0/udp/9000/quic-v1`.
    pub listen: Multiaddr,
    /// The fork-digest segment of topic names, e.g. `12345678`. Opaque to this crate:
    /// leanSpec treats it as a caller-supplied string, and so does Verity.
    pub network_name: String,
    /// Peers to dial at startup, e.g. `/ip4/10.0.0.1/udp/9000/quic-v1/p2p/16Uiu2HAm...`.
    pub bootnodes: Vec<Multiaddr>,
    /// Attestation subnets to subscribe to, in addition to the block and aggregation
    /// topics which are always subscribed.
    pub attestation_subnets: Vec<SubnetId>,
    /// Capacity of the command channel into the network task.
    pub command_buffer: usize,
    /// Capacity of the event channel out of the network task — the single drop point of
    /// the inbound pipeline.
    pub event_buffer: usize,
}

impl NetworkConfig {
    /// A configuration with the deferred tunables at their defaults; the caller supplies
    /// everything the spec or the operator fixes.
    #[must_use]
    pub fn new(keypair: Keypair, listen: Multiaddr, network_name: String) -> Self {
        Self {
            keypair,
            listen,
            network_name,
            bootnodes: Vec::new(),
            attestation_subnets: vec![SubnetId(0)],
            command_buffer: 64,
            event_buffer: 512,
        }
    }
}
