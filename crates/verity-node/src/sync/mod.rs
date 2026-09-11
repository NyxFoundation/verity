//! The sync service: where this node is relative to the network, and how it catches up.
//!
//! `docs/design/sync.md`. Four things live here, and the split between them is the design:
//!
//! - [`state`] — the three-state machine and the median-of-peers trigger. Pure.
//! - [`peers`] — what each peer claims, how reliably it answers, who gets asked next. Pure.
//! - [`fetch`] — which request closes a gap, and whether an answer is the shape that was
//!   asked for.
//! - [`responder`] — the serving side, answering other nodes out of this node's database.
//! - [`checkpoint`] — the HTTP entry that lets a node start above genesis.
//!
//! This file is the task that drives them: one sequential loop over peer changes, gap
//! signals, the status refresh, and new snapshots.
//!
//! # One request at a time
//!
//! The loop awaits each request it issues rather than running a window of them. That is
//! `docs/design/sync.md`'s "one batch in flight" taken literally, and it is what keeps the
//! service a small sequential machine — the property `docs/design/concurrency.md` asks of
//! every task that is not the network edge. A batch is up to `MAX_REQUEST_BLOCKS` blocks, so
//! the throughput ceiling is a round trip per 1,024 slots, not per slot.
//!
//! # Fetched blocks are not privileged
//!
//! Everything this service fetches goes into the verification stage's own inbox, the one
//! gossip arrives on. There is no side door into the chain task: the `Verified*` invariant of
//! `docs/design/concurrency.md` holds for the sync path with no exceptions.

pub mod checkpoint;
pub mod fetch;
pub mod peers;
pub mod responder;
pub mod state;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

use verity_chain::ChainView;
use verity_p2p::{GossipKind, NetworkHandle, PeerId, Request, Response};
use verity_types::Slot;
use verity_types::config::SECONDS_PER_SLOT;

use crate::network::status_of;
use crate::verification::GossipPayload;

use self::fetch::{FetchFailure, Gap, Plan};
use self::peers::{Outcome, PeerTable};
use self::state::SyncMachine;

pub use self::responder::{BlockRequest, BlockRequestKind, BlockResponder};
pub use self::state::SyncState;

/// How often the status handshake is re-exchanged with every connected peer.
///
/// The trigger is re-checked on every status update, so this interval is what "evaluated
/// continuously" costs on the wire: an 80-byte round trip per peer per four slots. The
/// once-per-connection handshake other clients use is the named counterexample — a node that
/// falls behind on a stable connection never notices.
pub const STATUS_REFRESH: Duration = Duration::from_secs(4 * SECONDS_PER_SLOT);

/// What sync did, on both sides of it.
///
/// The client side and the serving side are counted separately because they are separate
/// claims: that this node closed a gap by asking, and that it answered someone else's ask.
/// Both are otherwise invisible — a fetched block is indistinguishable from a gossiped one
/// once it is in the store, which is exactly what the end-to-end test needs to tell apart.
#[derive(Debug, Default)]
pub struct SyncCounters {
    fetched: AtomicU64,
    served_requests: AtomicU64,
    served_blocks: AtomicU64,
}

impl SyncCounters {
    /// Blocks this node obtained by asking a peer for them, rather than by gossip.
    pub fn fetched(&self) -> u64 {
        self.fetched.load(Ordering::Relaxed)
    }

    /// Block requests this node answered, refusals included.
    pub fn served_requests(&self) -> u64 {
        self.served_requests.load(Ordering::Relaxed)
    }

    /// Blocks this node handed to peers across those requests.
    pub fn served_blocks(&self) -> u64 {
        self.served_blocks.load(Ordering::Relaxed)
    }

    pub(crate) fn record_fetched(&self, blocks: usize) {
        self.fetched.fetch_add(blocks as u64, Ordering::Relaxed);
    }

    pub(crate) fn record_served(&self, blocks: usize) {
        self.served_requests.fetch_add(1, Ordering::Relaxed);
        self.served_blocks
            .fetch_add(blocks as u64, Ordering::Relaxed);
    }
}

/// A connection change, as the network bridge reports it.
#[derive(Debug, Clone, Copy)]
pub enum PeerEvent {
    /// First connection to a peer established.
    Connected(PeerId),
    /// Last connection to a peer closed.
    Disconnected(PeerId),
}

/// The task that decides whether this node is caught up, and closes the gap when it is not.
pub struct SyncService {
    handle: NetworkHandle,
    view: watch::Receiver<Arc<ChainView>>,
    gaps: mpsc::Receiver<Gap>,
    peer_events: mpsc::Receiver<PeerEvent>,
    blocks: mpsc::Sender<GossipPayload>,
    synced: watch::Sender<bool>,
    counters: Arc<SyncCounters>,
    peers: PeerTable,
    machine: SyncMachine,
}

impl SyncService {
    /// Wires the service to the network, the snapshot channel, and the verification stage.
    ///
    /// Returns the service and a `watch` carrying whether the node is caught up. That is an
    /// observation, not the duty gate: duties turn on head-versus-clock lag in
    /// `verity-validator` (`docs/design/sync.md`, Decision 1).
    #[must_use = "a service does nothing until it is run"]
    pub fn new(
        handle: NetworkHandle,
        view: watch::Receiver<Arc<ChainView>>,
        gaps: mpsc::Receiver<Gap>,
        peer_events: mpsc::Receiver<PeerEvent>,
        blocks: mpsc::Sender<GossipPayload>,
        counters: Arc<SyncCounters>,
    ) -> (Self, watch::Receiver<bool>) {
        let (synced, gate) = watch::channel(false);
        (
            Self {
                handle,
                view,
                gaps,
                peer_events,
                blocks,
                synced,
                counters,
                peers: PeerTable::new(),
                machine: SyncMachine::new(),
            },
            gate,
        )
    }

    /// Runs until the network bridge and the verification stage are both gone.
    pub async fn run(mut self) {
        let mut refresh = tokio::time::interval(STATUS_REFRESH);
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            let carry_on = tokio::select! {
                biased;

                event = self.peer_events.recv() => match event {
                    Some(event) => self.on_peer_event(event).await,
                    None => false,
                },

                gap = self.gaps.recv() => match gap {
                    Some(gap) => self.on_gap(gap).await,
                    None => false,
                },

                _ = refresh.tick() => self.refresh_status().await,

                changed = self.view.changed() => {
                    if changed.is_err() {
                        false
                    } else {
                        self.reevaluate().await
                    }
                },
            };

            if !carry_on {
                break;
            }
        }
        tracing::debug!(state = %self.machine.state(), "sync service stopping");
    }

    /// A peer arrived or left. A new peer is handshaked at once rather than at the next
    /// refresh: it is the fastest this node can learn it is behind.
    async fn on_peer_event(&mut self, event: PeerEvent) -> bool {
        match event {
            PeerEvent::Connected(peer) => {
                self.peers.connected(peer);
                self.exchange_status(peer).await;
            }
            PeerEvent::Disconnected(peer) => self.peers.disconnected(&peer),
        }
        self.reevaluate().await
    }

    /// Re-exchanges status with every connected peer.
    async fn refresh_status(&mut self) -> bool {
        for peer in self.peers.connected_peers() {
            self.exchange_status(peer).await;
        }
        self.reevaluate().await
    }

    /// One status handshake, and what it does to the peer's standing.
    async fn exchange_status(&mut self, peer: PeerId) {
        let ours = status_of(&self.view);
        self.peers.begin_request(&peer);
        let outcome = match self.handle.request(peer, Request::Status(ours)).await {
            Ok(Response::Status(theirs)) => {
                self.peers.observe_status(peer, theirs);
                Outcome::Success
            }
            Ok(Response::Error { code, .. }) => {
                tracing::debug!(%peer, %code, "peer refused the status handshake");
                Outcome::Failure
            }
            Ok(Response::Blocks(_)) => Outcome::Failure,
            Err(error) => {
                tracing::debug!(%peer, %error, "status handshake failed");
                Outcome::of_request_error(&error)
            }
        };
        self.peers.finish_request(&peer, outcome, Instant::now());
    }

    /// The verification stage could not resolve an item's ancestry.
    async fn on_gap(&mut self, gap: Gap) -> bool {
        let plan = fetch::plan_for_gap(self.head_slot(), gap);
        self.execute(plan).await
    }

    /// Re-runs the trigger, publishes the gate, and issues the next batch while behind.
    ///
    /// This is the one place the machine moves. It runs on every input the trigger depends
    /// on — a status update, a connection change, a new snapshot — which is what makes the
    /// condition continuously evaluated rather than checked once per connection.
    async fn reevaluate(&mut self) -> bool {
        let head = self.head_slot();
        let state = self
            .machine
            .observe(head, self.peers.network_finalized_slot());
        self.synced.send_replace(state.is_caught_up());

        if state != SyncState::Syncing {
            return true;
        }

        // Pagination terminates on the same condition the machine watches: after each batch
        // the loop comes back here with a new snapshot and asks again.
        let Some(target) = self.peers.highest_claimed_head() else {
            return true;
        };
        if target.0 <= head.0 {
            return true;
        }
        self.execute(Plan::ByRange(fetch::range_from(head, target)))
            .await
    }

    /// Issues one request and feeds whatever comes back into the verification stage.
    async fn execute(&mut self, plan: Plan) -> bool {
        let Some(peer) = self.peers.select_for_blocks(Instant::now()) else {
            return true;
        };

        self.peers.begin_request(&peer);
        let fetched = match plan {
            Plan::ByRoot(roots) => fetch::fetch_by_root(&self.handle, peer, roots).await,
            Plan::ByRange(request) => fetch::fetch_range(&self.handle, peer, request).await,
        };

        match fetched {
            Ok(chunks) => {
                self.peers
                    .finish_request(&peer, Outcome::Success, Instant::now());
                self.forward(chunks).await
            }
            Err(failure) => {
                self.report(&peer, &failure);
                true
            }
        }
    }

    /// Hands fetched blocks to the verification stage, on the channel gossip uses.
    ///
    /// The send awaits rather than shedding: unlike gossip, nothing else is going to deliver
    /// these blocks, and the backpressure is what stops a range walk from outrunning
    /// verification.
    async fn forward(&self, chunks: Vec<Vec<u8>>) -> bool {
        self.counters.record_fetched(chunks.len());
        for payload in chunks {
            let block = GossipPayload {
                kind: GossipKind::Block,
                payload,
            };
            if self.blocks.send(block).await.is_err() {
                return false;
            }
        }
        true
    }

    /// Scores a failed request and says why, once.
    fn report(&mut self, peer: &PeerId, failure: &FetchFailure) {
        let outcome = failure.outcome();
        self.peers.finish_request(peer, outcome, Instant::now());
        tracing::debug!(%peer, %failure, score = ?self.peers.score(peer), "block request failed");
    }

    /// The slot of this node's head block, which is what the trigger compares.
    fn head_slot(&self) -> Slot {
        self.view.borrow().head_checkpoint().slot
    }
}
