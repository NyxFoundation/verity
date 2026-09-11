//! The two bridges between the node and the wire.
//!
//! [`NetworkBridge`] carries gossip inward: raw bytes to the verification stage, and nothing
//! else — no decode, no crypto, so the swarm's own liveness never waits on a proof. It also
//! sorts the rest of the event stream to the task that owns each concern: the status
//! handshake it answers itself from the snapshot, block requests go to the responder, and
//! connection changes go to the sync service.
//!
//! [`ProductRelay`] carries this node's own duty products outward, and inward to the chain
//! task, from the single channel the validator client sends on.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use libssz::SszEncode;
use tokio::sync::{mpsc, watch};

use verity_chain::ChainView;
use verity_metrics::{Metrics, prometheus_gauge};
use verity_p2p::{ErrorCode, GossipKind, NetworkEvent, NetworkHandle, Request, Response, Status};
use verity_types::SubnetId;
use verity_validator::LocalProduct;

use crate::sync::{BlockRequest, BlockRequestKind, PeerEvent};
use crate::verification::GossipPayload;

/// The subnet this node's votes are published on.
///
/// leanSpec fixes `ATTESTATION_COMMITTEE_COUNT = 1`, so there is exactly one attestation
/// subnet and every validator uses it. This becomes a computation the day that constant moves.
pub const ATTESTATION_SUBNET: SubnetId = SubnetId(0);

/// The `client` label of `lean_connected_peers` for a peer whose client is not announced.
const PEER_CLIENT_UNKNOWN: &str = "unknown";

/// Gossip the bridge could not hand downstream.
#[derive(Debug, Default)]
pub struct BridgeCounters {
    dropped: AtomicU64,
}

impl BridgeCounters {
    /// Payloads dropped because the verification stage's queue was full.
    ///
    /// This is the pipeline's deliberate load-shedding point: what is discarded here is raw
    /// bytes nobody has spent verification effort on yet, and all of it is peer-recoverable.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Where the bridge sorts the event stream to.
///
/// One value rather than four parameters, because they are one thing: the set of tasks the
/// wire is fanned out to, decided once when the node is wired.
pub struct BridgeChannels {
    /// Raw gossip payloads, to the verification stage.
    pub gossip: mpsc::Sender<GossipPayload>,
    /// Inbound block requests, to the responder.
    pub blocks: mpsc::Sender<BlockRequest>,
    /// Connection changes, to the sync service.
    pub peers: mpsc::Sender<PeerEvent>,
    /// The addresses the swarm bound, published for whoever needs a dialable one.
    pub listening: watch::Sender<Vec<verity_p2p::Multiaddr>>,
}

/// Drains the network task's event stream.
pub struct NetworkBridge {
    events: mpsc::Receiver<NetworkEvent>,
    inbound: mpsc::Sender<GossipPayload>,
    blocks: mpsc::Sender<BlockRequest>,
    peers: mpsc::Sender<PeerEvent>,
    listening: watch::Sender<Vec<verity_p2p::Multiaddr>>,
    handle: NetworkHandle,
    view: watch::Receiver<Arc<ChainView>>,
    counters: Arc<BridgeCounters>,
    metrics: Arc<Metrics>,
}

impl NetworkBridge {
    /// Wires the network task's events to the verification stage.
    #[must_use = "a bridge does nothing until it is run"]
    pub fn new(
        events: mpsc::Receiver<NetworkEvent>,
        channels: BridgeChannels,
        handle: NetworkHandle,
        view: watch::Receiver<Arc<ChainView>>,
        counters: Arc<BridgeCounters>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            events,
            inbound: channels.gossip,
            blocks: channels.blocks,
            peers: channels.peers,
            listening: channels.listening,
            handle,
            view,
            counters,
            metrics,
        }
    }

    /// `lean_connected_peers`. leanMetrics labels the gauge by the peer's client, which the
    /// lean transport does not announce, so every peer is `unknown`.
    fn connected_peers(&self) -> prometheus_gauge::IntGauge {
        self.metrics
            .connected_peers
            .with_label_values(&[PEER_CLIENT_UNKNOWN])
    }

    /// Runs until the network task stops.
    ///
    /// The stream has to be drained continuously: an inbound request nobody takes off it is a
    /// request that times out on the peer's side.
    pub async fn run(mut self) {
        while let Some(event) = self.events.recv().await {
            match event {
                NetworkEvent::Gossip { kind, payload } => self.forward(kind, payload),
                NetworkEvent::InboundRequest {
                    peer,
                    request,
                    channel,
                } => {
                    if !self.dispatch(peer, request, channel).await {
                        break;
                    }
                }
                NetworkEvent::NewListenAddr(address) => {
                    tracing::info!(%address, "listening");
                    // Published rather than only logged: with port 0 in the configuration
                    // this event is the only place the bound port exists.
                    self.listening.send_modify(|bound| bound.push(address));
                }
                NetworkEvent::PeerConnected(peer) => {
                    tracing::info!(%peer, "peer connected");
                    self.connected_peers().inc();
                    if self.peers.send(PeerEvent::Connected(peer)).await.is_err() {
                        break;
                    }
                }
                NetworkEvent::PeerDisconnected(peer) => {
                    tracing::info!(%peer, "peer disconnected");
                    self.connected_peers().dec();
                    if self
                        .peers
                        .send(PeerEvent::Disconnected(peer))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    }

    /// Hands raw bytes to the verification stage, or sheds them.
    ///
    /// `try_send`, never an await: blocking here would put verification latency on the path
    /// that keeps the gossip mesh alive.
    fn forward(&self, kind: GossipKind, payload: Vec<u8>) {
        if self
            .inbound
            .try_send(GossipPayload { kind, payload })
            .is_err()
        {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Sends one inbound request to whoever owns the data it asks for.
    ///
    /// Status is answerable from the snapshot alone, so it is answered here and costs the
    /// bridge nothing. A block request is hundreds of megabytes of disk read in the worst
    /// case, so it goes to the responder's own task — by `try_send`, because a bridge that
    /// waits for the responder is a bridge that stops draining gossip. A full queue is
    /// answered `SERVER_ERROR` at once: telling a peer this node is saturated is better than
    /// letting its request time out.
    ///
    /// Returns whether the bridge can carry on.
    async fn dispatch(
        &self,
        peer: verity_p2p::PeerId,
        request: Request,
        channel: verity_p2p::ResponseChannel,
    ) -> bool {
        let kind = match request {
            Request::Status(_) => {
                let response = Response::Status(status_of(&self.view));
                let answered = self.handle.respond(channel, response).await.is_ok();
                tracing::trace!(%peer, "answered a status handshake");
                return answered;
            }
            Request::BlocksByRoot(roots) => BlockRequestKind::ByRoot(roots),
            Request::BlocksByRange(range) => BlockRequestKind::ByRange(range),
        };

        let Err(returned) = self.blocks.try_send(BlockRequest {
            peer,
            kind,
            channel,
        }) else {
            return true;
        };

        match returned {
            mpsc::error::TrySendError::Full(request) => {
                let response = Response::Error {
                    code: ErrorCode::ServerError,
                    message: "the block responder is saturated".to_string(),
                };
                self.handle.respond(request.channel, response).await.is_ok()
            }
            // The responder is gone, which means the node is shutting down.
            mpsc::error::TrySendError::Closed(_) => false,
        }
    }
}

/// This node's checkpoints, as a peer sees them.
///
/// One function rather than one per caller: the bridge answers handshakes with it and the
/// sync service opens them with it, and a `Status` that differed between the two would be
/// this node describing itself two ways.
pub fn status_of(view: &watch::Receiver<Arc<ChainView>>) -> Status {
    let view = view.borrow();
    Status {
        finalized: view.latest_finalized(),
        head: view.head_checkpoint(),
    }
}

/// Carries the validator client's products to the chain task and to the network.
pub struct ProductRelay {
    products: mpsc::Receiver<LocalProduct>,
    chain: mpsc::Sender<LocalProduct>,
    handle: NetworkHandle,
}

impl ProductRelay {
    /// Wires the duty channel to its two consumers.
    #[must_use = "a relay does nothing until it is run"]
    pub fn new(
        products: mpsc::Receiver<LocalProduct>,
        chain: mpsc::Sender<LocalProduct>,
        handle: NetworkHandle,
    ) -> Self {
        Self {
            products,
            chain,
            handle,
        }
    }

    /// Runs until the validator client stops producing.
    ///
    /// The chain task is served first and the network second. Neither send may be skipped:
    /// these are the node's own signatures, and no peer holds a copy to give back.
    pub async fn run(mut self) {
        while let Some(product) = self.products.recv().await {
            let published = self.publish(&product).await;
            if self.chain.send(product).await.is_err() {
                break;
            }
            if let Err(error) = published {
                // A publish failure is a delivery problem, not a consensus one: the value is
                // already on its way into our own store.
                tracing::warn!(%error, "duty product not published");
            }
        }
    }

    async fn publish(&self, product: &LocalProduct) -> Result<(), verity_p2p::PublishError> {
        let (kind, payload) = match product {
            LocalProduct::Block(signed) => (GossipKind::Block, signed.to_ssz()),
            LocalProduct::Attestation(signed) => {
                (GossipKind::Attestation(ATTESTATION_SUBNET), signed.to_ssz())
            }
            LocalProduct::Aggregate(signed) => (GossipKind::Aggregation, signed.to_ssz()),
        };
        self.handle.publish(kind, payload).await
    }
}
