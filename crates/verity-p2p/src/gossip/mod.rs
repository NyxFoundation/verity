//! Gossip: the topic grammar, the message-ID function, and the gossipsub behaviour
//! configured to leanSpec's parameters.

pub mod message_id;
pub mod topic;

use std::time::Duration;

use libp2p::gossipsub;
use verity_types::config::{JUSTIFICATION_LOOKBACK_SLOTS, SECONDS_PER_SLOT};

use crate::config::{MAX_PAYLOAD_SIZE, max_compressed_len};
use crate::gossip::message_id::compute_message_id;

/// Builds the gossipsub behaviour with leanSpec's mesh and cache parameters
/// (`gossipsub/parameters.py` at the pinned revision).
///
/// Messages are unsigned and anonymous — the lean protocol authenticates content by XMSS
/// signature inside the payload, never by libp2p envelope. Message validation gating
/// (`validate_messages`) is deliberately NOT enabled: the current spec forwards before any
/// application-level verification, and `docs/design/concurrency.md` Decision 2 keeps
/// Verity on that behavior.
pub fn build_behaviour() -> Result<gossipsub::Behaviour, &'static str> {
    let config = gossipsub::ConfigBuilder::default()
        .mesh_n(8)
        .mesh_n_low(6)
        .mesh_n_high(12)
        .gossip_lazy(6)
        .heartbeat_interval(Duration::from_millis(700))
        .fanout_ttl(Duration::from_secs(60))
        .history_length(6)
        .history_gossip(3)
        // seen_ttl = SECONDS_PER_SLOT * JUSTIFICATION_LOOKBACK_SLOTS * 2.
        .duplicate_cache_time(Duration::from_secs(
            SECONDS_PER_SLOT * JUSTIFICATION_LOOKBACK_SLOTS * 2,
        ))
        .validation_mode(gossipsub::ValidationMode::Anonymous)
        .message_id_fn(|message| compute_message_id(message.topic.as_str(), &message.data))
        .max_transmit_size(max_compressed_len(MAX_PAYLOAD_SIZE))
        // Not a leanSpec parameter: a defensive bound on how many messages one RPC frame
        // may carry, so a single peer cannot make one frame arbitrarily expensive.
        .max_messages_per_rpc(Some(500))
        .idontwant_message_size_threshold(1024)
        .build()
        .map_err(|_| "invalid gossipsub configuration")?;
    gossipsub::Behaviour::new(gossipsub::MessageAuthenticity::Anonymous, config)
}
