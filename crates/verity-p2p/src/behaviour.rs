//! The composed libp2p behaviour: identify, gossipsub, and the three req/resp protocols.

use libp2p::swarm::NetworkBehaviour;
use libp2p::{gossipsub, identify, identity, request_response};

use crate::gossip;
use crate::reqresp;
use crate::reqresp::codec::Codec;
use crate::reqresp::messages::Protocol;

/// Identify protocol-version string. leanSpec defines no identify protocol; this is
/// interop plumbing — go-libp2p peers gate gossipsub GRAFT on an identify exchange, so
/// registering the behaviour keeps Verity meshable with them.
const IDENTIFY_PROTOCOL_VERSION: &str = "leanconsensus";

/// Everything the swarm speaks. One req/resp behaviour per protocol — see
/// [`reqresp::build_behaviour`] for why that split is what keeps upstream libp2p usable
/// without a fork.
#[derive(NetworkBehaviour)]
pub struct Behaviour {
    /// Peer identification, for gossipsub interop; its events are deliberately unhandled.
    pub identify: identify::Behaviour,
    /// Gossip, at leanSpec's parameters.
    pub gossipsub: gossipsub::Behaviour,
    /// The status handshake.
    pub status: request_response::Behaviour<Codec>,
    /// Blocks by root.
    pub blocks_by_root: request_response::Behaviour<Codec>,
    /// Blocks by slot range.
    pub blocks_by_range: request_response::Behaviour<Codec>,
}

impl Behaviour {
    /// Assembles the behaviour for a node identified by `local_public_key`.
    pub fn new(local_public_key: identity::PublicKey) -> Result<Self, &'static str> {
        Ok(Self {
            identify: identify::Behaviour::new(
                identify::Config::new(IDENTIFY_PROTOCOL_VERSION.to_string(), local_public_key)
                    .with_agent_version(format!("verity/{}", env!("CARGO_PKG_VERSION"))),
            ),
            gossipsub: gossip::build_behaviour()?,
            status: reqresp::build_behaviour(Protocol::Status),
            blocks_by_root: reqresp::build_behaviour(Protocol::BlocksByRoot),
            blocks_by_range: reqresp::build_behaviour(Protocol::BlocksByRange),
        })
    }
}
