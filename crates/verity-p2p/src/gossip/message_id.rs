//! The gossip message-ID function.
//!
//! Transcribed from leanSpec `node/networking/gossipsub/message.py`:
//!
//! ```text
//! message_id = SHA256(domain ‖ uint64_le(len(topic)) ‖ topic ‖ data)[:20]
//! ```
//!
//! where `data` is the decompressed payload when snappy decompression succeeds (domain
//! `MESSAGE_DOMAIN_VALID_SNAPPY`) and the raw transmitted bytes when it fails (domain
//! `MESSAGE_DOMAIN_INVALID_SNAPPY`). A payload that decompresses but declares a length
//! above the payload cap is treated as invalid: caching an ID for content the node would
//! refuse to allocate is the same decompression-bomb it refuses elsewhere.
//!
//! This is a transport identifier — deduplication and IWANT bookkeeping — not a consensus
//! root, which is why SHA-256 appears here directly rather than through `libssz-merkle`.

use libp2p::gossipsub::MessageId;
use sha2::{Digest, Sha256};

use crate::config::{MAX_PAYLOAD_SIZE, MESSAGE_DOMAIN_INVALID_SNAPPY, MESSAGE_DOMAIN_VALID_SNAPPY};
use crate::wire::snappy::decompress_block;

/// Bytes of the truncated SHA-256 that form a message ID.
pub const MESSAGE_ID_LEN: usize = 20;

/// The bare hash: `SHA256(domain ‖ uint64_le(len(topic)) ‖ topic ‖ data)[:20]`, with the
/// domain chosen by the caller. [`compute_message_id`] is the wrapper that chooses it;
/// this form exists because leanSpec's conformance vectors supply the domain explicitly.
#[must_use]
pub fn message_id_with_domain(domain: &[u8], topic: &[u8], data: &[u8]) -> [u8; MESSAGE_ID_LEN] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update((topic.len() as u64).to_le_bytes());
    hasher.update(topic);
    hasher.update(data);
    let mut id = [0u8; MESSAGE_ID_LEN];
    id.copy_from_slice(&hasher.finalize()[..MESSAGE_ID_LEN]);
    id
}

/// Computes the message ID for a message received or published on `topic` with
/// transmitted (compressed) payload `data`.
#[must_use]
pub fn compute_message_id(topic: &str, data: &[u8]) -> MessageId {
    let (domain, payload) = match decompress_block(data, MAX_PAYLOAD_SIZE) {
        Ok(decompressed) => (MESSAGE_DOMAIN_VALID_SNAPPY, decompressed),
        Err(_) => (MESSAGE_DOMAIN_INVALID_SNAPPY, data.to_vec()),
    };
    MessageId::new(&message_id_with_domain(&domain, topic.as_bytes(), &payload))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::snappy::compress_block;

    #[test]
    fn should_use_valid_domain_when_payload_decompresses() {
        let topic = "/leanconsensus/12345678/block/ssz_snappy";
        let payload = b"ssz bytes";
        let id = compute_message_id(topic, &compress_block(payload));

        let mut hasher = Sha256::new();
        hasher.update(MESSAGE_DOMAIN_VALID_SNAPPY);
        hasher.update((topic.len() as u64).to_le_bytes());
        hasher.update(topic.as_bytes());
        hasher.update(payload);
        assert_eq!(id.0, &hasher.finalize()[..MESSAGE_ID_LEN]);
    }

    #[test]
    fn should_use_invalid_domain_when_payload_is_not_snappy() {
        let topic = "/leanconsensus/12345678/block/ssz_snappy";
        let raw = [0xffu8; 16];
        let id = compute_message_id(topic, &raw);

        let mut hasher = Sha256::new();
        hasher.update(MESSAGE_DOMAIN_INVALID_SNAPPY);
        hasher.update((topic.len() as u64).to_le_bytes());
        hasher.update(topic.as_bytes());
        hasher.update(raw);
        assert_eq!(id.0, &hasher.finalize()[..MESSAGE_ID_LEN]);
    }

    #[test]
    fn should_differ_when_topic_differs_for_same_payload() {
        let payload = compress_block(b"same bytes");
        let a = compute_message_id("/leanconsensus/net/block/ssz_snappy", &payload);
        let b = compute_message_id("/leanconsensus/net/aggregation/ssz_snappy", &payload);
        assert_ne!(a, b);
    }
}
