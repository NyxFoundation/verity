//! The `verity` binary: parse arguments, start the node, wait for a signal, stop it.
//!
//! # The command line is the cross-client one
//!
//! The flags mirror leanSpec's node so that a Verity process can be dropped into a
//! lean-quickstart devnet in place of any other client: the same `--genesis` file, the same
//! `--validator-keys` layout, the same `--node-id` lookup. Two flags are Verity's own —
//! `--data-dir`, because Verity persists what the reference node keeps in memory, and
//! `--network-name`, because the topic segment is a caller string with no computation behind
//! it yet.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use verity_node::{
    ASSIGNMENT_FILE_NAME, GenesisFile, Multiaddr, Node, NodeConfig, assigned_validators,
    config::KEY_SUBDIRECTORY, identity::Keypair,
};

/// A lean consensus node.
#[derive(Debug, Parser)]
#[command(name = "verity", version, about = "The Verity lean consensus client")]
struct Args {
    /// Path to the genesis YAML file.
    #[arg(long, value_name = "PATH")]
    genesis: PathBuf,

    /// Directory the chain database lives in.
    #[arg(long, value_name = "DIR", default_value = "verity-data")]
    data_dir: PathBuf,

    /// Address to listen on for inbound QUIC connections.
    #[arg(
        long,
        value_name = "MULTIADDR",
        default_value = "/ip4/0.0.0.0/udp/9001/quic-v1"
    )]
    listen: Multiaddr,

    /// Peer to dial at startup. Repeatable.
    #[arg(long = "bootnode", value_name = "MULTIADDR")]
    bootnodes: Vec<Multiaddr>,

    /// The network segment of every gossip topic. Peers that disagree exchange no gossip.
    #[arg(long, value_name = "NAME", default_value = "00000000")]
    network_name: String,

    /// Directory holding `validators.yaml` and `hash-sig-keys/`. Omit to follow without signing.
    #[arg(long, value_name = "DIR")]
    validator_keys: Option<PathBuf>,

    /// This node's identifier, looked up in `validators.yaml` to find its validator indices.
    #[arg(long, value_name = "ID", default_value = "verity_0")]
    node_id: String,

    /// Run the interval-2 aggregation round.
    #[arg(long)]
    is_aggregator: bool,

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

    // The keys directory decides whether this node signs at all: with no directory there is
    // no assignment to read and no key to load, which is a follower.
    let (validator_indices, key_directory) = match &args.validator_keys {
        Some(base) => (
            assigned_validators(&base.join(ASSIGNMENT_FILE_NAME), &args.node_id)?,
            Some(base.join(KEY_SUBDIRECTORY)),
        ),
        None => (Vec::new(), None),
    };

    let node = Node::start(NodeConfig {
        genesis,
        data_directory: args.data_dir,
        listen: args.listen,
        bootnodes: args.bootnodes,
        network_name: args.network_name,
        // A fresh identity per run. Peers are reached by dialling configured addresses, so
        // nothing upstream depends on this node keeping the same peer id across restarts.
        keypair: Keypair::generate_secp256k1(),
        validator_indices,
        key_directory,
        is_aggregator: args.is_aggregator,
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
