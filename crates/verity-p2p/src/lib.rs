//! The I/O Edge network service: leanSpec's wire protocol over upstream libp2p.
//!
//! # What belongs here
//!
//! The wire protocol and nothing above it. This crate owns the gossip topic grammar, the
//! gossip message-ID function, both snappy formats, the req/resp framing with its response
//! codes, and the one long-lived task that drives the libp2p swarm. Payload *meaning* is
//! deliberately absent: gossip leaves this crate as raw uncompressed SSZ bytes, and block
//! chunks in req/resp responses cross the API as raw SSZ bytes in both directions. The
//! network task performs topic checks and deduplication only — no SSZ decode of consensus
//! containers, no cryptography — so network liveness never waits on a proof
//! (`docs/design/concurrency.md`, Decision 2).
//!
//! The req/resp *envelope* types are the exception, because they are wire protocol, not
//! consensus payload: [`Status`], [`BlocksByRootRequest`], and [`BlocksByRangeRequest`] are
//! decoded here — a responder cannot serve a request it cannot read.
//!
//! What this crate deliberately does not own, and who does:
//!
//! - **Verification and decode of gossip payloads** — the verification stage
//!   (`docs/design/concurrency.md`, Decision 2). Raw bytes out, `try_send`, drops counted.
//! - **Peer scoring, sync state machine, request orchestration** — the sync service
//!   (`docs/design/sync.md`). This crate reports request outcomes; it never judges peers.
//! - **Answering requests** — whoever owns the data. Inbound requests surface as events
//!   carrying a response channel; the responder policy (serving window, refusal codes)
//!   lives with the storage owner, not here.
//!
//! # Source
//!
//! Wire constants, topic strings, protocol IDs, framing, and the message-ID function are
//! transcribed from leanSpec `src/lean_spec/node/networking/`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`. leanSpec is the only authority for the wire
//! format; where a Verity design document disagrees, leanSpec wins.
//!
//! # Example — two nodes on localhost
//!
//! ```no_run
//! use verity_p2p::{
//!     GossipKind, NetworkConfig, NetworkEvent, Request, Response, Status, identity,
//! };
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Node A listens on an ephemeral port.
//! let config_a = NetworkConfig::new(
//!     identity::Keypair::generate_secp256k1(),
//!     "/ip4/127.0.0.1/udp/0/quic-v1".parse()?,
//!     "00000000".to_string(),
//! );
//! let (node_a, mut events_a) = verity_p2p::spawn(config_a)?;
//! let peer_a = node_a.local_peer_id();
//!
//! // The bound address (with the real port) arrives as an event; appending `/p2p/<id>`
//! // makes it dialable.
//! let listen_a = loop {
//!     if let Some(NetworkEvent::NewListenAddr(addr)) = events_a.recv().await {
//!         break addr;
//!     }
//! };
//! let addr_a = listen_a.with_p2p(peer_a).expect("address with peer id");
//!
//! // The event stream must be drained continuously, typically in its own task: gossip,
//! // inbound requests, and connection changes all arrive here, and an inbound request
//! // that is never answered times out on the peer's side.
//! tokio::spawn(async move {
//!     while let Some(event) = events_a.recv().await {
//!         if let NetworkEvent::InboundRequest { request: Request::Status(_), channel, .. } = event
//!         {
//!             let response = Response::Status(Status::default());
//!             let _ = node_a.respond(channel, response).await;
//!         }
//!     }
//! });
//!
//! // Node B boots with A as its bootnode — same network name, or gossip is discarded.
//! let mut config_b = NetworkConfig::new(
//!     identity::Keypair::generate_secp256k1(),
//!     "/ip4/127.0.0.1/udp/0/quic-v1".parse()?,
//!     "00000000".to_string(),
//! );
//! config_b.bootnodes = vec![addr_a];
//! let (node_b, mut _events_b) = verity_p2p::spawn(config_b)?;
//!
//! // Ask A for its status, and publish a block payload (raw SSZ bytes — the caller
//! // encodes; this crate only compresses). Publishing can fail with
//! // `PublishError::InsufficientPeers` until the gossip mesh forms; retry on it.
//! let status = node_b.request(peer_a, Request::Status(Status::default())).await?;
//! node_b.publish(GossipKind::Block, vec![0u8; 64]).await?;
//! # Ok(()) }
//! ```
//!
//! # Discovery is out of scope, deliberately
//!
//! Peers are reached by dialing configured multiaddrs (bootnodes). leanSpec carries an ENR
//! module and ethlambda runs discv5, but no Verity design document commits to a discovery
//! mechanism and a devnet-sized network is fully connected by static configuration.
//! Revisit trigger: joining a network whose peer set is not known at configuration time.

pub mod behaviour;
pub mod config;
pub mod error;
pub mod gossip;
pub mod reqresp;
pub mod service;
pub mod wire;

pub use behaviour::{Behaviour, BehaviourEvent};
pub use config::{
    MAX_ERROR_MESSAGE_SIZE, MAX_PAYLOAD_SIZE, MAX_REQUEST_BLOCKS, MIN_SLOTS_FOR_BLOCK_REQUESTS,
    NetworkConfig, RESP_TIMEOUT, max_compressed_len,
};
pub use error::{BuildError, CommandError, PublishError, RequestError};
pub use gossip::message_id::{MESSAGE_ID_LEN, compute_message_id, message_id_with_domain};
pub use gossip::topic::{GossipKind, GossipTopic};
pub use reqresp::messages::{
    BlocksByRangeRequest, BlocksByRootRequest, ErrorCode, Protocol, Request, RequestedBlockRoots,
    Response, Status,
};
pub use service::{
    NetworkCommand, NetworkCounters, NetworkEvent, NetworkHandle, ResponseChannel, spawn,
};

// Re-exported so consumers can name addresses and peers without depending on libp2p
// directly; the version is pinned once, in the workspace manifest.
pub use libp2p::{Multiaddr, PeerId, identity};
