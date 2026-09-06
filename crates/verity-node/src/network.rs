//! The two bridges between the node and the wire.
//!
//! [`NetworkBridge`] carries gossip inward: raw bytes to the verification stage, and nothing
//! else — no decode, no crypto, so the swarm's own liveness never waits on a proof. It also
//! answers the one request this node can answer from a snapshot.
//!
//! [`ProductRelay`] carries this node's own duty products outward, and inward to the chain
//! task, from the single channel the validator client sends on.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use libssz::SszEncode;
use tokio::sync::{mpsc, watch};

use verity_chain::ChainView;
use verity_p2p::{ErrorCode, GossipKind, NetworkEvent, NetworkHandle, Request, Response, Status};
use verity_types::SubnetId;
use verity_validator::LocalProduct;

use crate::verification::GossipPayload;

/// The subnet this node's votes are published on.
///
/// leanSpec fixes `ATTESTATION_COMMITTEE_COUNT = 1`, so there is exactly one attestation
/// subnet and every validator uses it. This becomes a computation the day that constant moves.
pub const ATTESTATION_SUBNET: SubnetId = SubnetId(0);

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

/// Drains the network task's event stream.
pub struct NetworkBridge {
    events: mpsc::Receiver<NetworkEvent>,
    inbound: mpsc::Sender<GossipPayload>,
    handle: NetworkHandle,
    view: watch::Receiver<Arc<ChainView>>,
    counters: Arc<BridgeCounters>,
}

impl NetworkBridge {
    /// Wires the network task's events to the verification stage.
    #[must_use = "a bridge does nothing until it is run"]
    pub fn new(
        events: mpsc::Receiver<NetworkEvent>,
        inbound: mpsc::Sender<GossipPayload>,
        handle: NetworkHandle,
        view: watch::Receiver<Arc<ChainView>>,
        counters: Arc<BridgeCounters>,
    ) -> Self {
        Self {
            events,
            inbound,
            handle,
            view,
            counters,
        }
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
                    let response = self.answer(&request);
                    if self.handle.respond(channel, response).await.is_err() {
                        break;
                    }
                    tracing::trace!(%peer, "answered a request");
                }
                NetworkEvent::NewListenAddr(address) => {
                    tracing::info!(%address, "listening");
                }
                NetworkEvent::PeerConnected(peer) => tracing::info!(%peer, "peer connected"),
                NetworkEvent::PeerDisconnected(peer) => tracing::info!(%peer, "peer disconnected"),
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

    /// Answers a peer's request.
    ///
    /// Status is answerable from the snapshot alone. The two block protocols are not: a
    /// response chunk is a `SignedBlock`, proof included, and the proofs live in the database
    /// behind the chain task's single writer. Serving them — with the retention window and
    /// the refusal floor that go with it — arrives with the sync service
    /// (`docs/design/sync.md`). Until then this node refuses them in the protocol's own
    /// terms rather than leaving the peer to time out.
    fn answer(&self, request: &Request) -> Response {
        match request {
            Request::Status(_) => {
                let view = self.view.borrow();
                Response::Status(Status {
                    finalized: view.latest_finalized(),
                    head: view.head_checkpoint(),
                })
            }
            Request::BlocksByRoot(_) | Request::BlocksByRange(_) => Response::Error {
                code: ErrorCode::ResourceUnavailable,
                message: "this node does not serve blocks yet".to_string(),
            },
        }
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
