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
//!            └────────────────┘   try_send   └────────────────┘   (③)        │  writer │
//!                    ▲                               ▲                       └────┬────┘
//!                    │ publish                       │ Arc<ChainView>             │
//!            ┌───────┴────────┐                      │                            │
//!            │ product relay  │◀── duties (②) ───────┴────────────────────────────┘
//!            └────────────────┘                  watch: Arc<ChainView>
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

pub mod chain;
pub mod clock;
pub mod config;
pub mod error;
pub mod network;
pub mod startup;
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
use crate::network::{ATTESTATION_SUBNET, BridgeCounters, NetworkBridge, ProductRelay};
use crate::verification::{StageCounters, VerificationStage};

pub use config::{ASSIGNMENT_FILE_NAME, GenesisFile, assigned_validators};
pub use error::ConfigError;

// Re-exported so the binary can name addresses and identities without a second dependency on
// the networking crate; the libp2p version is pinned once, in the workspace manifest.
pub use verity_p2p::{Multiaddr, identity};

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
}

/// A running node, and the handles that stop it.
pub struct Node {
    view: watch::Receiver<Arc<ChainView>>,
    peer_id: PeerId,
    network: Option<verity_p2p::NetworkHandle>,
    ticker: JoinHandle<()>,
    bridge: JoinHandle<()>,
    draining: Vec<JoinHandle<()>>,
    stage_counters: Arc<StageCounters>,
    bridge_counters: Arc<BridgeCounters>,
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

        // The store carries one validator index, which is only ever used to attribute the
        // node's own votes; the keyring below is what actually decides which duties run.
        let backend = RocksBackend::open(&config.data_directory)?;
        let (repository, store) = startup::open(
            backend,
            &genesis_state,
            config.validator_indices.first().copied(),
        )?;

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

        let bridge = tokio::spawn(
            NetworkBridge::new(
                events,
                raw_gossip,
                handle.clone(),
                view.clone(),
                Arc::clone(&bridge_counters),
            )
            .run(),
        );

        // Order matters only in one place: the chain task is spawned last so that every
        // sender into it already exists, and it therefore never sees an empty inbox that
        // looks like shutdown.
        let draining = vec![
            tokio::spawn(DutyService::new(keyring, prover, products, view.clone(), ticks).run()),
            tokio::spawn(ProductRelay::new(product_stream, local, handle.clone()).run()),
            tokio::spawn(
                VerificationStage::new(
                    raw_gossip_stream,
                    verified,
                    view.clone(),
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
            peer_id,
            network: Some(handle),
            ticker,
            bridge,
            draining,
            stage_counters,
            bridge_counters,
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
