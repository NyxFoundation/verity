//! The node runtime: five libraries, one running process.
//!
//! # What belongs here
//!
//! Wiring, and the two tasks that have nowhere else to live: the chain task that owns
//! consensus state, and the verification stage in front of it. Every other crate in the
//! workspace does one job and does not know the others exist — `verity-chain` decides,
//! `verity-crypto` signs and proves, `verity-db` records, `verity-p2p` carries bytes,
//! `verity-validator` produces duties. This crate is where they meet, and it is the only one
//! that depends on all of them.
//!
//! # The shape of a running node
//!
//! ```text
//!            ┌─ network task ─┐  raw bytes   ┌─ verification ─┐  Verified*   ┌─ chain ─┐
//!  peers ───▶│ topic + dedup  │─────────────▶│ decode, verify │─────────────▶│  single │
//!            └───┬───┬────────┘   try_send   └───────┬────────┘   (③)        │  writer │
//!                │   │                ▲              │ gaps                  └────┬────┘
//!                │   │ block requests │ fetched      ▼                            │
//!                │   ▼                │ blocks   ┌─ sync ─┐  requests              │
//!                │  ┌── responder ──┐ └──────────│ state  │──────────▶ peers       │
//!                │  │ read-only DB  │            │ peers  │                        │
//!                │  └───────────────┘            └────────┘  synced ──▶ duties     │
//!                │ publish                            ▲                            │
//!            ┌───┴────────────┐                       │ watch: Arc<ChainView>      │
//!            │ product relay  │◀── duties (②) ────────┴────────────────────────────┘
//!            └────────────────┘
//!                                          ▲
//!                                    clock (①) ── one ticker, two consumers
//! ```
//!
//! The arrows are the whole design: work flows one way, reads leave as immutable snapshots,
//! and the only thing that mutates consensus state is the task that owns it. See
//! `docs/design/concurrency.md` for why each channel is the primitive it is.
//!
//! # Shutdown
//!
//! Channel closure, and nothing else. Stopping the producers at the edge — the clock and the
//! network bridge — closes each downstream input in turn, and every task exits when its own
//! inputs run out. Duty products are drained to the end: they are the one thing in the
//! pipeline no peer can give back.

pub mod bootstrap;
pub mod chain;
pub mod clock;
pub mod config;
pub mod error;
pub mod network;
pub mod store_open;
pub mod sync;
pub mod verification;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use verity_chain::{ChainView, SlotClock, generate_genesis};

use verity_db::RocksBackend;
use verity_p2p::{NetworkConfig, PeerId, identity::Keypair};
use verity_types::ValidatorIndex;
use verity_validator::{DutyService, Keyring, Prover};

use crate::chain::{Aggregator, ChainTask};
use crate::error::NodeError;
use crate::network::{
    ATTESTATION_SUBNET, BridgeChannels, BridgeCounters, NetworkBridge, ProductRelay,
};
use crate::sync::checkpoint::CheckpointAnchor;
use crate::sync::responder::BlockResponder;
use crate::sync::{SyncCounters, SyncService};
use crate::verification::{StageCounters, VerificationStage};

pub use bootstrap::{
    check_aggregate_subnets, check_committee_count, parse_bootnode, read_bootnodes, read_node_key,
};
pub use config::{ASSIGNMENT_FILE_NAME, GenesisFile, assigned_validators};
pub use error::ConfigError;

// Re-exported so the binary can name addresses and identities without a second dependency on
// the networking crate; the libp2p version is pinned once, in the workspace manifest.
pub use verity_p2p::{Multiaddr, identity};
// Re-exported for the same reason: the binary defaults its topic segment to the fork's digest
// and does not otherwise depend on the types crate.
pub use verity_types::config::GOSSIP_DIGEST;

/// Capacity of the duty-product channel (②).
///
/// A handful of events per slot — a proposal, a vote, an aggregation round — so tens are
/// plenty. Its sender awaits rather than sheds, which is what the small size is safe with.
const PRODUCT_CAPACITY: usize = 32;

/// Capacity of the verified channel (③) and of the stage's raw input.
///
/// Sized against per-slot gossip volume rather than anything structural. The raw input is
/// where the pipeline sheds load, so its depth is how much of a burst is absorbed before
/// bytes start being dropped.
const GOSSIP_CAPACITY: usize = 256;

/// How many items may wait on a state that is not in view yet.
const PENDING_CAPACITY: usize = 256;

/// How many gap signals may queue for the sync service.
///
/// Sized to match the pending buffer: one signal per parked item is the worst case, and the
/// sender sheds rather than waits, because a full queue means the sync service is already
/// busy closing gaps and the next park raises the same one again.
const GAP_CAPACITY: usize = PENDING_CAPACITY;

/// How many inbound block requests may queue for the responder.
///
/// Small on purpose. One request is up to 1,024 blocks of proof-bearing history off disk, so
/// a deep queue would only turn a burst into timeouts on the peers' side; past this depth the
/// bridge answers `SERVER_ERROR` at once.
const BLOCK_REQUEST_CAPACITY: usize = 32;

/// How many connection changes may queue for the sync service.
const PEER_EVENT_CAPACITY: usize = 64;

/// Everything a node needs to start.
#[derive(Debug)]
pub struct NodeConfig {
    /// The parsed genesis file: when slot 0 began, and the registry.
    pub genesis: GenesisFile,
    /// Where the database lives.
    pub data_directory: PathBuf,
    /// The address the node binds for inbound QUIC connections.
    pub listen: Multiaddr,
    /// Peers to dial at startup.
    pub bootnodes: Vec<Multiaddr>,
    /// The network segment of every gossip topic. Peers that disagree never exchange gossip.
    pub network_name: String,
    /// This node's libp2p identity.
    pub keypair: Keypair,
    /// The validators this node runs, from the assignment file.
    pub validator_indices: Vec<ValidatorIndex>,
    /// Where the validator keys live. Absent means this node follows without signing.
    pub key_directory: Option<PathBuf>,
    /// Whether this node runs the interval-2 aggregation round.
    pub is_aggregator: bool,
    /// Base URL to fetch a finalized anchor from instead of replaying from genesis.
    ///
    /// Present means a checkpoint start, and that path has no fallbacks: a fetch or
    /// verification failure stops the node (`docs/design/sync.md`, Decision 1).
    pub checkpoint_sync_url: Option<String>,
}

/// A running node, and the handles that stop it.
pub struct Node {
    view: watch::Receiver<Arc<ChainView>>,
    synced: watch::Receiver<bool>,
    listening: watch::Receiver<Vec<Multiaddr>>,
    peer_id: PeerId,
    network: Option<verity_p2p::NetworkHandle>,
    ticker: JoinHandle<()>,
    bridge: JoinHandle<()>,
    draining: Vec<JoinHandle<()>>,
    stage_counters: Arc<StageCounters>,
    bridge_counters: Arc<BridgeCounters>,
    sync_counters: Arc<SyncCounters>,
}

impl Node {
    /// Opens the database, rebuilds the chain, and starts every task.
    ///
    /// Returns once the node is running: the database is open, the store is built, the first
    /// `ChainView` exists, and the network is listening. Nothing is left half-started — a
    /// failure here has written no anchor and spawned no task that outlives the call.
    ///
    /// # Errors
    ///
    /// [`NodeError::Config`] when the genesis file cannot be turned into a registry,
    /// [`NodeError::Storage`] or [`NodeError::Restore`] when the data directory cannot be
    /// used, [`NodeError::Validator`] when the configured keys cannot be loaded, and
    /// [`NodeError::Network`] when the listen address cannot be bound.
    pub async fn start(config: NodeConfig) -> Result<Self, NodeError> {
        let genesis_state =
            generate_genesis(config.genesis.genesis_time, config.genesis.to_validators()?);
        let clock = SlotClock::new(config.genesis.genesis_time);

        // Before the database, because a checkpoint start that cannot be completed must stop
        // the node without having written an anchor of any kind.
        let checkpoint = fetch_checkpoint(&config, &genesis_state).await?;

        // The store carries one validator index, which is only ever used to attribute the
        // node's own votes; the keyring below is what actually decides which duties run.
        let backend = RocksBackend::open(&config.data_directory)?;
        // Taken before the backend moves into the writer: this is the same open database,
        // and it is the only handle the responder ever gets.
        let reader = backend.reader();
        let (repository, store) = store_open::open(
            backend,
            &genesis_state,
            checkpoint.as_ref(),
            config.validator_indices.first().copied(),
        )?;
        let served = Arc::new(store_open::open_reader(reader, &genesis_state)?);

        let keyring = load_keys(&config)?;
        let prover = Prover::new();
        if !keyring.is_empty() {
            // Paid once, here, rather than by the first duty of the node's life.
            prover.warm_up().await?;
        }

        let (ticks, ticker) = clock::spawn(clock);
        let (products, product_stream) = mpsc::channel(PRODUCT_CAPACITY);
        let (local, local_stream) = mpsc::channel(PRODUCT_CAPACITY);
        let (verified, verified_stream) = mpsc::channel(GOSSIP_CAPACITY);
        let (raw_gossip, raw_gossip_stream) = mpsc::channel(GOSSIP_CAPACITY);
        let (gaps, gap_stream) = mpsc::channel(GAP_CAPACITY);
        let (block_requests, block_request_stream) = mpsc::channel(BLOCK_REQUEST_CAPACITY);
        let (peer_events, peer_event_stream) = mpsc::channel(PEER_EVENT_CAPACITY);

        let aggregator = config.is_aggregator.then(|| Aggregator {
            prover: prover.clone(),
            products: products.clone(),
        });
        let (chain, view) = ChainTask::new(
            store,
            repository,
            ticks.clone(),
            local_stream,
            verified_stream,
            aggregator,
        );

        let (handle, events) = verity_p2p::spawn(network_config(&config))?;
        let peer_id = handle.local_peer_id();

        let stage_counters = Arc::new(StageCounters::default());
        let bridge_counters = Arc::new(BridgeCounters::default());
        let sync_counters = Arc::new(SyncCounters::default());

        let (listening, bound) = watch::channel(Vec::new());
        let bridge = tokio::spawn(
            NetworkBridge::new(
                events,
                BridgeChannels {
                    gossip: raw_gossip.clone(),
                    blocks: block_requests,
                    peers: peer_events,
                    listening,
                },
                handle.clone(),
                view.clone(),
                Arc::clone(&bridge_counters),
            )
            .run(),
        );

        // Fetched blocks re-enter on the same channel gossip arrives on, so the sync path has
        // no side door into the chain task.
        let (sync, synced) = SyncService::new(
            handle.clone(),
            view.clone(),
            gap_stream,
            peer_event_stream,
            raw_gossip,
            Arc::clone(&sync_counters),
        );

        // Order matters only in one place: the chain task is spawned last so that every
        // sender into it already exists, and it therefore never sees an empty inbox that
        // looks like shutdown.
        let draining = vec![
            tokio::spawn(DutyService::new(keyring, prover, products, view.clone(), ticks).run()),
            tokio::spawn(ProductRelay::new(product_stream, local, handle.clone()).run()),
            tokio::spawn(sync.run()),
            tokio::spawn(
                BlockResponder::new(
                    served,
                    view.clone(),
                    block_request_stream,
                    handle.clone(),
                    Arc::clone(&sync_counters),
                )
                .run(),
            ),
            tokio::spawn(
                VerificationStage::new(
                    raw_gossip_stream,
                    verified,
                    view.clone(),
                    gaps,
                    PENDING_CAPACITY,
                    Arc::clone(&stage_counters),
                )
                .run(),
            ),
            tokio::spawn(chain.run()),
        ];

        tracing::info!(%peer_id, "node started");

        Ok(Self {
            view,
            synced,
            listening: bound,
            peer_id,
            network: Some(handle),
            ticker,
            bridge,
            draining,
            stage_counters,
            bridge_counters,
            sync_counters,
        })
    }

    /// The snapshot channel every reader answers from.
    #[must_use]
    pub fn view(&self) -> watch::Receiver<Arc<ChainView>> {
        self.view.clone()
    }

    /// This node's libp2p identity.
    #[must_use]
    pub const fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// The addresses the network service actually bound.
    ///
    /// Empty until the swarm reports its first listener, which happens shortly *after*
    /// `start` returns: binding is synchronous but the address arrives as an event. A caller
    /// that needs a dialable address should await [`Node::listening`] rather than read this
    /// once. With port 0 in the configuration this is the only place the bound port appears,
    /// so it is read here rather than from a log line.
    #[must_use]
    pub fn listen_addresses(&self) -> Vec<Multiaddr> {
        self.listening.borrow().clone()
    }

    /// The channel the bound addresses are published on, for a caller that has to wait for
    /// one.
    #[must_use]
    pub fn listening(&self) -> watch::Receiver<Vec<Multiaddr>> {
        self.listening.clone()
    }

    /// Whether the node considers itself caught up with the network.
    ///
    /// `false` covers both "behind" and "has not met the network yet". It is an observation
    /// for operators, not the validator duty gate — a node alone at genesis reports `false`
    /// here and still proposes, because it is not behind anything (`docs/design/sync.md`,
    /// Decision 1).
    #[must_use]
    pub fn is_synced(&self) -> bool {
        *self.synced.borrow()
    }

    /// What the verification stage discarded.
    #[must_use]
    pub fn stage_counters(&self) -> &StageCounters {
        &self.stage_counters
    }

    /// What the network bridge shed.
    #[must_use]
    pub fn bridge_counters(&self) -> &BridgeCounters {
        &self.bridge_counters
    }

    /// What sync fetched, and what it served.
    #[must_use]
    pub fn sync_counters(&self) -> &SyncCounters {
        &self.sync_counters
    }

    /// Stops the node, and waits for the work already in flight to finish.
    ///
    /// Only the two edge producers are stopped outright — the clock and the network bridge.
    /// Everything else drains: the duty loop's products are followed all the way into the
    /// store, and the chain task persists what it was given before it exits.
    pub async fn shutdown(mut self) {
        self.ticker.abort();
        self.bridge.abort();
        // The last handle: dropping it closes the swarm's command channel, which is how the
        // network task learns to stop.
        drop(self.network.take());

        for task in self.draining {
            let _ = task.await;
        }
        tracing::info!("node stopped");
    }
}

/// Fetches the checkpoint anchor when the operator asked for one.
///
/// Verified against the local genesis file before it is returned, so a failure here has
/// written nothing and started nothing (`docs/design/sync.md`, Decision 1).
async fn fetch_checkpoint(
    config: &NodeConfig,
    genesis_state: &verity_types::State,
) -> Result<Option<CheckpointAnchor>, NodeError> {
    let Some(url) = &config.checkpoint_sync_url else {
        return Ok(None);
    };
    tracing::info!(%url, "starting from a fetched checkpoint, not from genesis");
    Ok(Some(
        sync::checkpoint::fetch_anchor(url, genesis_state).await?,
    ))
}

/// Loads the keys this node signs with, or none when it is configured to follow.
fn load_keys(config: &NodeConfig) -> Result<Keyring, NodeError> {
    match &config.key_directory {
        Some(directory) if !config.validator_indices.is_empty() => {
            Ok(Keyring::load(directory, &config.validator_indices)?)
        }
        _ => Ok(Keyring::empty()),
    }
}

/// The network service's configuration, derived from the node's.
fn network_config(config: &NodeConfig) -> NetworkConfig {
    let mut network = NetworkConfig::new(
        config.keypair.clone(),
        config.listen.clone(),
        config.network_name.clone(),
    );
    network.bootnodes = config.bootnodes.clone();
    network.attestation_subnets = vec![ATTESTATION_SUBNET];
    network
}
