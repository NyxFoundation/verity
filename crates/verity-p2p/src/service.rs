//! The network task: one tokio task owning the swarm, commands in, events out.
//!
//! This is the "network task" of `docs/design/concurrency.md` Decision 2. Its inbound
//! contract is deliberately minimal — topic check and deduplication only, then `try_send`
//! of raw bytes — so that network liveness (mesh maintenance, keep-alives) never waits on
//! anything downstream. The event channel is the single drop point of the inbound
//! pipeline: when it is full, raw gossip is dropped and counted, never awaited.
//!
//! Shutdown follows the lifecycle rule that channel closure is the only signal: dropping
//! every [`NetworkHandle`] (and the command senders inside pending requests) closes the
//! command channel, the task breaks out of its loop, and dropping its event sender closes
//! the event stream for downstream consumers.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use libp2p::gossipsub::{self, IdentTopic};
use libp2p::request_response::{self, OutboundRequestId};
use libp2p::swarm::SwarmEvent;
use libp2p::{Multiaddr, PeerId, Swarm, SwarmBuilder};
use tokio::sync::{mpsc, oneshot};

use crate::behaviour::{Behaviour, BehaviourEvent};
use crate::config::{MAX_PAYLOAD_SIZE, NetworkConfig};
use crate::error::{BuildError, CommandError, PublishError, RequestError};
use crate::gossip::topic::{GossipKind, GossipTopic};
use crate::reqresp::messages::{Protocol, Request, Response};
use crate::wire::snappy::{compress_block, decompress_block};

/// The channel on which an inbound request awaits its response. Carried inside
/// [`NetworkEvent::InboundRequest`] and returned through [`NetworkHandle::respond`].
pub type ResponseChannel = request_response::ResponseChannel<Response>;

/// Commands into the network task.
pub enum NetworkCommand {
    /// Dial a peer at an address.
    Dial {
        /// The address to dial, e.g. `/ip4/10.0.0.1/udp/9000/quic-v1/p2p/16Uiu...`.
        address: Multiaddr,
        /// Resolves once the dial is initiated (not established).
        reply: oneshot::Sender<Result<(), CommandError>>,
    },
    /// Publish uncompressed SSZ bytes on a gossip channel.
    Publish {
        /// The gossip channel.
        kind: GossipKind,
        /// The uncompressed SSZ payload; this crate compresses it.
        payload: Vec<u8>,
        /// Resolves with the publish outcome.
        reply: oneshot::Sender<Result<(), PublishError>>,
    },
    /// Send a req/resp request to a peer.
    SendRequest {
        /// The peer to ask. Must be connected or dialable by its known addresses.
        peer: PeerId,
        /// The request; its variant selects the protocol.
        request: Request,
        /// Resolves with the peer's response or the transport failure.
        reply: oneshot::Sender<Result<Response, RequestError>>,
    },
    /// Answer an inbound request received via [`NetworkEvent::InboundRequest`].
    Respond {
        /// The channel that arrived with the request.
        channel: ResponseChannel,
        /// The response to send.
        response: Response,
    },
}

/// Events out of the network task.
///
/// The receiver returned by [`spawn`] must be drained continuously — typically by a
/// dedicated task looping on `recv()`. Gossip, inbound requests, and connection changes
/// all arrive here; a consumer that stops receiving stops answering peers, and a full
/// event channel sheds gossip (by design) and then other events (counted).
#[derive(Debug)]
pub enum NetworkEvent {
    /// The swarm bound a listen address. With port 0 in the configuration, this is where
    /// the actual port becomes known.
    NewListenAddr(Multiaddr),
    /// First connection to a peer established.
    PeerConnected(PeerId),
    /// Last connection to a peer closed.
    PeerDisconnected(PeerId),
    /// A gossip message on a subscribed topic of this network, decompressed. Raw SSZ
    /// bytes — decoding and verification happen downstream, never here.
    Gossip {
        /// The gossip channel the message arrived on.
        kind: GossipKind,
        /// The uncompressed SSZ payload.
        payload: Vec<u8>,
    },
    /// A peer sent a request; answer it via [`NetworkHandle::respond`]. Dropping the
    /// channel instead lets the peer's request time out.
    InboundRequest {
        /// The requesting peer.
        peer: PeerId,
        /// The decoded request envelope.
        request: Request,
        /// Where the response goes.
        channel: ResponseChannel,
    },
}

/// Drop and failure counters, shared between the task and its handle. The wiring point
/// for `verity-metrics` once it exists; until then the counts are at least observable.
#[derive(Debug, Default)]
pub struct NetworkCounters {
    gossip_dropped: AtomicU64,
    gossip_invalid: AtomicU64,
    events_dropped: AtomicU64,
}

impl NetworkCounters {
    /// Gossip messages dropped because the event channel was full — the pipeline's one
    /// deliberate load-shedding point.
    pub fn gossip_dropped(&self) -> u64 {
        self.gossip_dropped.load(Ordering::Relaxed)
    }

    /// Gossip messages discarded as undecodable: foreign or malformed topic, wrong
    /// network, or failed decompression. Counted, never peer-punished
    /// (`docs/design/sync.md`, Decision 3).
    pub fn gossip_invalid(&self) -> u64 {
        self.gossip_invalid.load(Ordering::Relaxed)
    }

    /// Non-gossip events dropped on a full event channel.
    pub fn events_dropped(&self) -> u64 {
        self.events_dropped.load(Ordering::Relaxed)
    }
}

/// Cloneable handle for talking to the network task.
#[derive(Clone)]
pub struct NetworkHandle {
    commands: mpsc::Sender<NetworkCommand>,
    local_peer_id: PeerId,
    counters: Arc<NetworkCounters>,
}

impl NetworkHandle {
    /// The local peer ID derived from the configured keypair.
    #[must_use]
    pub fn local_peer_id(&self) -> PeerId {
        self.local_peer_id
    }

    /// The task's drop and failure counters.
    #[must_use]
    pub fn counters(&self) -> Arc<NetworkCounters> {
        Arc::clone(&self.counters)
    }

    /// Dials a peer. Resolves when the dial is initiated; the connection itself is
    /// reported later as [`NetworkEvent::PeerConnected`].
    pub async fn dial(&self, address: Multiaddr) -> Result<(), CommandError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(NetworkCommand::Dial { address, reply })
            .await
            .map_err(|_| CommandError::ServiceStopped)?;
        response.await.map_err(|_| CommandError::ServiceStopped)?
    }

    /// Publishes uncompressed SSZ bytes on a gossip channel.
    pub async fn publish(&self, kind: GossipKind, payload: Vec<u8>) -> Result<(), PublishError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(NetworkCommand::Publish {
                kind,
                payload,
                reply,
            })
            .await
            .map_err(|_| PublishError::ServiceStopped)?;
        response.await.map_err(|_| PublishError::ServiceStopped)?
    }

    /// Sends a request and awaits the peer's response.
    pub async fn request(&self, peer: PeerId, request: Request) -> Result<Response, RequestError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(NetworkCommand::SendRequest {
                peer,
                request,
                reply,
            })
            .await
            .map_err(|_| RequestError::ServiceStopped)?;
        response.await.map_err(|_| RequestError::ServiceStopped)?
    }

    /// Answers an inbound request.
    pub async fn respond(
        &self,
        channel: ResponseChannel,
        response: Response,
    ) -> Result<(), CommandError> {
        self.commands
            .send(NetworkCommand::Respond { channel, response })
            .await
            .map_err(|_| CommandError::ServiceStopped)
    }
}

/// Builds the swarm, subscribes the configured topics, dials the bootnodes, and spawns
/// the network task. Returns the handle and the event stream.
pub fn spawn(
    config: NetworkConfig,
) -> Result<(NetworkHandle, mpsc::Receiver<NetworkEvent>), BuildError> {
    let mut swarm = build_swarm(&config)?;
    let local_peer_id = *swarm.local_peer_id();

    swarm
        .listen_on(config.listen.clone())
        .map_err(|e| BuildError::Listen(e.to_string()))?;
    subscribe_topics(&mut swarm, &config)?;
    for bootnode in &config.bootnodes {
        swarm
            .dial(bootnode.clone())
            .map_err(|e| BuildError::Bootnode(format!("{bootnode}: {e}")))?;
    }

    let (command_tx, command_rx) = mpsc::channel(config.command_buffer);
    let (event_tx, event_rx) = mpsc::channel(config.event_buffer);
    let counters = Arc::new(NetworkCounters::default());

    let service = Service {
        swarm,
        commands: command_rx,
        events: event_tx,
        pending: HashMap::new(),
        network_name: config.network_name,
        counters: Arc::clone(&counters),
    };
    tokio::spawn(service.run());

    Ok((
        NetworkHandle {
            commands: command_tx,
            local_peer_id,
            counters,
        },
        event_rx,
    ))
}

/// Builds the QUIC swarm. Transport security and multiplexing come from QUIC itself —
/// no separate noise or yamux layer, matching leanSpec's transport stack.
fn build_swarm(config: &NetworkConfig) -> Result<Swarm<Behaviour>, BuildError> {
    let swarm = SwarmBuilder::with_existing_identity(config.keypair.clone())
        .with_tokio()
        .with_quic()
        .with_behaviour(|key| {
            Behaviour::new(key.public())
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })
        })
        .map_err(|e| BuildError::Behaviour(e.to_string()))?
        .with_swarm_config(|c| {
            // Consensus connections are long-lived by design; without this, libp2p's
            // idle timeout would sever a healthy but momentarily quiet peer.
            c.with_idle_connection_timeout(Duration::from_secs(24 * 60 * 60))
        })
        .build();
    Ok(swarm)
}

/// Subscribes the always-on block and aggregation topics plus the configured attestation
/// subnets.
fn subscribe_topics(
    swarm: &mut Swarm<Behaviour>,
    config: &NetworkConfig,
) -> Result<(), BuildError> {
    let mut kinds = vec![GossipKind::Block, GossipKind::Aggregation];
    kinds.extend(
        config
            .attestation_subnets
            .iter()
            .map(|subnet| GossipKind::Attestation(*subnet)),
    );
    for kind in kinds {
        let topic = GossipTopic::new(kind, &config.network_name);
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&IdentTopic::new(topic.to_string()))
            .map_err(|e| BuildError::Behaviour(format!("subscribing {topic}: {e:?}")))?;
    }
    Ok(())
}

/// The state owned by the network task.
struct Service {
    swarm: Swarm<Behaviour>,
    commands: mpsc::Receiver<NetworkCommand>,
    events: mpsc::Sender<NetworkEvent>,
    /// Outbound requests awaiting a response. Keyed by protocol as well as ID because
    /// each per-protocol behaviour numbers its requests independently.
    pending:
        HashMap<(Protocol, OutboundRequestId), oneshot::Sender<Result<Response, RequestError>>>,
    network_name: String,
    counters: Arc<NetworkCounters>,
}

impl Service {
    async fn run(mut self) {
        loop {
            tokio::select! {
                // Commands first: the node's own work (duty products, sync requests)
                // ahead of any volume of network input, mirroring the chain task's bias.
                biased;
                command = self.commands.recv() => match command {
                    Some(command) => self.handle_command(command),
                    // Every handle dropped: shutdown. Pending oneshots drop with the
                    // task, resolving their awaiters to ServiceStopped.
                    None => break,
                },
                event = self.swarm.select_next_some() => self.handle_swarm_event(event),
            }
        }
    }

    fn handle_command(&mut self, command: NetworkCommand) {
        match command {
            NetworkCommand::Dial { address, reply } => {
                let result = self
                    .swarm
                    .dial(address)
                    .map_err(|e| CommandError::Failed(e.to_string()));
                let _ = reply.send(result);
            }
            NetworkCommand::Publish {
                kind,
                payload,
                reply,
            } => {
                let _ = reply.send(self.publish(kind, &payload));
            }
            NetworkCommand::SendRequest {
                peer,
                request,
                reply,
            } => {
                let protocol = request.protocol();
                let behaviour = self.swarm.behaviour_mut();
                let id = match protocol {
                    Protocol::Status => behaviour.status.send_request(&peer, request),
                    Protocol::BlocksByRoot => behaviour.blocks_by_root.send_request(&peer, request),
                    Protocol::BlocksByRange => {
                        behaviour.blocks_by_range.send_request(&peer, request)
                    }
                };
                self.pending.insert((protocol, id), reply);
            }
            NetworkCommand::Respond { channel, response } => {
                // `send_response` only feeds the channel that arrived with the request;
                // which behaviour instance relays it is immaterial, so the status
                // behaviour serves all three protocols here. A `Err` return means the
                // inbound stream already timed out — nothing to do but let it go.
                let _ = self
                    .swarm
                    .behaviour_mut()
                    .status
                    .send_response(channel, response);
            }
        }
    }

    fn publish(&mut self, kind: GossipKind, payload: &[u8]) -> Result<(), PublishError> {
        if payload.len() > MAX_PAYLOAD_SIZE {
            return Err(PublishError::PayloadTooLarge);
        }
        let topic = GossipTopic::new(kind, &self.network_name);
        self.swarm
            .behaviour_mut()
            .gossipsub
            .publish(IdentTopic::new(topic.to_string()), compress_block(payload))
            .map(|_| ())
            .map_err(PublishError::from)
    }

    fn handle_swarm_event(&mut self, event: SwarmEvent<BehaviourEvent>) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                self.emit(NetworkEvent::NewListenAddr(address));
            }
            SwarmEvent::ConnectionEstablished {
                peer_id,
                num_established,
                ..
            } if num_established.get() == 1 => {
                self.emit(NetworkEvent::PeerConnected(peer_id));
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                num_established: 0,
                ..
            } => {
                self.emit(NetworkEvent::PeerDisconnected(peer_id));
            }
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
                message,
                ..
            })) => {
                self.handle_gossip(&message);
            }
            SwarmEvent::Behaviour(BehaviourEvent::Status(event)) => {
                self.handle_reqresp(Protocol::Status, event);
            }
            SwarmEvent::Behaviour(BehaviourEvent::BlocksByRoot(event)) => {
                self.handle_reqresp(Protocol::BlocksByRoot, event);
            }
            SwarmEvent::Behaviour(BehaviourEvent::BlocksByRange(event)) => {
                self.handle_reqresp(Protocol::BlocksByRange, event);
            }
            // Identify runs for gossipsub interop only; gossipsub's own subscription
            // and peer bookkeeping events need no reaction; the remaining swarm-level
            // events (dial progress, listener lifecycle, ...) carry no obligation for
            // this task.
            _ => {}
        }
    }

    /// Topic check and deduplication only — the gossipsub behaviour has already
    /// deduplicated by message ID; what remains is deciding whether the topic is ours
    /// and handing the bytes on.
    fn handle_gossip(&mut self, message: &gossipsub::Message) {
        let Some(topic) = GossipTopic::parse(message.topic.as_str()) else {
            self.counters.gossip_invalid.fetch_add(1, Ordering::Relaxed);
            return;
        };
        if topic.network_name != self.network_name {
            self.counters.gossip_invalid.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let Ok(payload) = decompress_block(&message.data, MAX_PAYLOAD_SIZE) else {
            self.counters.gossip_invalid.fetch_add(1, Ordering::Relaxed);
            return;
        };
        // The single deliberate drop point: full channel means the node is behind, and
        // what is shed is raw gossip — peer-recoverable by range sync.
        if self
            .events
            .try_send(NetworkEvent::Gossip {
                kind: topic.kind,
                payload,
            })
            .is_err()
        {
            self.counters.gossip_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn handle_reqresp(
        &mut self,
        protocol: Protocol,
        event: request_response::Event<Request, Response>,
    ) {
        match event {
            request_response::Event::Message {
                peer,
                message:
                    request_response::Message::Request {
                        request, channel, ..
                    },
                ..
            } => {
                // Dropping the event (full channel) drops the response channel with it;
                // the peer sees a timeout — the same outcome as any overloaded responder.
                self.emit(NetworkEvent::InboundRequest {
                    peer,
                    request,
                    channel,
                });
            }
            request_response::Event::Message {
                message:
                    request_response::Message::Response {
                        request_id,
                        response,
                    },
                ..
            } => {
                if let Some(reply) = self.pending.remove(&(protocol, request_id)) {
                    let _ = reply.send(Ok(response));
                }
            }
            request_response::Event::OutboundFailure {
                request_id, error, ..
            } => {
                if let Some(reply) = self.pending.remove(&(protocol, request_id)) {
                    let _ = reply.send(Err(RequestError::from_outbound_failure(&error)));
                }
            }
            // An inbound failure is the *peer's* problem to retry; the response-sent
            // acknowledgement carries no obligation.
            request_response::Event::InboundFailure { .. }
            | request_response::Event::ResponseSent { .. } => {}
        }
    }

    fn emit(&mut self, event: NetworkEvent) {
        if self.events.try_send(event).is_err() {
            self.counters.events_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}
