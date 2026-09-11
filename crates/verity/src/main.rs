//! The `verity` binary: parse arguments, start the node, wait for a signal, stop it.
//!
//! # The command line is the cross-client one
//!
//! The flags are the set lean-quickstart drives every client with (`docs/adding-a-new-client.md`
//! there), so that a Verity process can be dropped into a devnet in place of any other client:
//! the same `config.yaml` behind `--genesis`, the same `nodes.yaml` behind `--bootnodes`, the
//! same `<node>.key` behind `--node-key`, the same `validators.yaml` and `hash-sig-keys/`
//! layout behind `--validator-keys`, the same `--node-id` lookup. Two flags are Verity's own:
//! `--data-dir`, because Verity persists what the reference node keeps in memory, and
//! `--network-name`, which defaults to the fork's gossip digest and exists only so that a
//! private network can partition itself off.
//!
//! Two flags are accepted for the deployment's sake and then checked rather than used:
//! `--attestation-committee-count` and `--aggregate-subnet-ids`. leanSpec fixes the committee
//! count as a constant of the fork, and this build transcribes it; a deployment asking for
//! another value is refused at startup, with the reason, rather than joined.
//!
//! Sync has exactly one flag, `--checkpoint-sync-url`, and that is deliberate: every other
//! number the sync service uses is a constant in the code, because `docs/design/sync.md` puts
//! the thresholds outside the design's commitments. An operator can choose where the node
//! starts; the rate at which it catches up is not a choice the wire format leaves open.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use verity_node::{
    ASSIGNMENT_FILE_NAME, ConfigError, GOSSIP_DIGEST, GenesisFile, Multiaddr, Node, NodeConfig,
    assigned_validators, check_aggregate_subnets, check_committee_count, config::KEY_SUBDIRECTORY,
    identity::Keypair, parse_bootnode, read_bootnodes, read_node_key,
};

/// A lean consensus node.
#[derive(Debug, Parser)]
#[command(name = "verity", version, about = "The Verity lean consensus client")]
struct Args {
    /// Path to the genesis file (`config.yaml`).
    #[arg(long, value_name = "PATH")]
    genesis: PathBuf,

    /// Directory the chain database lives in.
    #[arg(long, value_name = "DIR", default_value = "verity-data")]
    data_dir: PathBuf,

    /// Address to listen on for inbound QUIC connections.
    #[arg(
        long,
        value_name = "MULTIADDR",
        default_value = "/ip4/0.0.0.0/udp/9001/quic-v1",
        conflicts_with = "listen_port"
    )]
    listen: Multiaddr,

    /// UDP port to listen on for inbound QUIC connections, on every interface.
    #[arg(long, value_name = "PORT")]
    listen_port: Option<u16>,

    /// Peer to dial at startup, as a multiaddr or an ENR. Repeatable.
    #[arg(long = "bootnode", value_name = "MULTIADDR|ENR")]
    bootnode: Vec<String>,

    /// File listing peers to dial at startup (`nodes.yaml`): a YAML list of ENRs or multiaddrs.
    #[arg(long, value_name = "PATH")]
    bootnodes: Option<PathBuf>,

    /// The network segment of every gossip topic. Peers that disagree exchange no gossip.
    #[arg(long, value_name = "NAME", default_value = GOSSIP_DIGEST)]
    network_name: String,

    /// File holding this node's secp256k1 secret as hex (`<node>.key`). Omit for a fresh
    /// identity each run.
    #[arg(long, value_name = "PATH")]
    node_key: Option<PathBuf>,

    /// Directory holding `validators.yaml` and `hash-sig-keys/`. Omit to follow without signing.
    #[arg(long, value_name = "DIR")]
    validator_keys: Option<PathBuf>,

    /// This node's identifier, looked up in `validators.yaml` to find its validator indices.
    #[arg(long, value_name = "ID", default_value = "verity_0")]
    node_id: String,

    /// Run the interval-2 aggregation round.
    #[arg(long)]
    is_aggregator: bool,

    /// Subnets to aggregate for, comma-separated. Accepted for deployment compatibility and
    /// checked against the fork's committee count.
    #[arg(
        long,
        value_name = "IDS",
        value_delimiter = ',',
        requires = "is_aggregator"
    )]
    aggregate_subnet_ids: Vec<u64>,

    /// The deployment's attestation committee count. Checked against the fork's constant.
    #[arg(long, value_name = "N")]
    attestation_committee_count: Option<u64>,

    /// Base URL of a node to fetch the finalized anchor from, instead of replaying from
    /// genesis. A fetch or verification failure stops the node; there is no fallback.
    #[arg(long, value_name = "URL")]
    checkpoint_sync_url: Option<String>,

    /// Log at DEBUG instead of INFO.
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    init_logging(args.verbose);

    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "verity could not start");
            ExitCode::FAILURE
        }
    }
}

/// Starts the node and runs it until the process is interrupted.
async fn run(args: Args) -> Result<(), verity_node::error::NodeError> {
    let genesis = GenesisFile::read(&args.genesis)?;

    if let Some(count) = args.attestation_committee_count {
        check_committee_count(count)?;
    }
    check_aggregate_subnets(&args.aggregate_subnet_ids)?;

    // The keys directory decides whether this node signs at all: with no directory there is
    // no assignment to read and no key to load, which is a follower.
    let (validator_indices, key_directory) = match &args.validator_keys {
        Some(base) => (
            assigned_validators(&base.join(ASSIGNMENT_FILE_NAME), &args.node_id)?,
            Some(base.join(KEY_SUBDIRECTORY)),
        ),
        None => (Vec::new(), None),
    };

    // A configured key is what lets peers dial this node by the identity in `nodes.yaml`.
    // Without one, a fresh identity per run is fine: nothing upstream depends on this node
    // keeping the same peer id across restarts when it is only ever the dialler.
    let keypair = match &args.node_key {
        Some(path) => read_node_key(path)?,
        None => Keypair::generate_secp256k1(),
    };

    let listen = listen_address(&args);
    let bootnodes = bootnodes(&args)?;

    let node = Node::start(NodeConfig {
        genesis,
        data_directory: args.data_dir,
        listen,
        bootnodes,
        network_name: args.network_name,
        keypair,
        validator_indices,
        key_directory,
        is_aggregator: args.is_aggregator,
        checkpoint_sync_url: args.checkpoint_sync_url,
    })
    .await?;

    // The one shutdown trigger. Everything downstream of it is channel closure.
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "cannot listen for an interrupt; stopping");
    }
    tracing::info!("interrupted; shutting down");
    node.shutdown().await;
    Ok(())
}

/// The bind address: the port flag when given, the full multiaddr otherwise.
fn listen_address(args: &Args) -> Multiaddr {
    match args.listen_port {
        Some(port) => format!("/ip4/0.0.0.0/udp/{port}/quic-v1")
            .parse()
            .expect("a port number always forms a valid multiaddr"),
        None => args.listen.clone(),
    }
}

/// Every peer to dial at startup: the file's entries first, then the flag's.
fn bootnodes(args: &Args) -> Result<Vec<Multiaddr>, ConfigError> {
    let mut addresses = match &args.bootnodes {
        Some(path) => read_bootnodes(path)?,
        None => Vec::new(),
    };
    for entry in &args.bootnode {
        addresses.push(
            parse_bootnode(entry).map_err(|reason| ConfigError::MalformedBootnode {
                source: "--bootnode".to_string(),
                entry: entry.clone(),
                reason,
            })?,
        );
    }
    Ok(addresses)
}

/// Installs the subscriber. A library never does this — it takes the choice from whoever
/// embeds it — so it happens here, once, before anything can log.
fn init_logging(verbose: bool) {
    let default = if verbose { "debug" } else { "info" };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
