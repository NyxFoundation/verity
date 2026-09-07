//! One gap, three nodes' worth of work: does a node that starts late reach the chain that was
//! built before it existed?
//!
//! This is the only test that exercises both halves of sync at once, and it exercises them
//! against each other. For the follower to catch up, the server has to answer block requests
//! out of a database — the responder — and the follower has to notice the gap, ask for what
//! it is missing, and feed the answers through its verification stage — the client. Neither
//! half can pass this alone, and neither is reachable from a unit test.
//!
//! # Why the chain is built by a node that is then shut down
//!
//! The producer holds validator keys, so it proves: a block proof per slot it proposes in.
//! Leaving it running underneath the follower's catch-up means proving and verifying
//! concurrently in one process for as long as the catch-up takes — which on a four-core CI
//! runner is where this test spent 75 minutes and most of its memory.
//!
//! Nothing about the claim needs it. Serving is a database read, so the server is started on
//! the producer's data directory *without keys*: it restores the chain, answers requests, and
//! proves nothing. That also makes the test stricter than it was — the blocks the follower
//! receives come out of persisted history through a restart, not out of the memory of the
//! node that just made them.
//!
//! # Gated, and slow on purpose
//!
//! Like `single_node.rs`, and for the same reason: production-scheme keys are 33.5 MB each
//! and a block proof is seconds of zk proving. `VERITY_TEST_KEYS` supplies the keys
//! (<https://github.com/leanEthereum/leansig-test-keys>, `prod_scheme.tar.gz`, sha256-pinned
//! in `crates/verity-crypto/test-keys.sha256`); with the variable unset the test returns.

mod common;

use std::time::Duration;

use verity_node::{GenesisFile, Multiaddr, Node, NodeConfig, identity::Keypair};
use verity_types::ValidatorIndex;
use verity_types::config::SECONDS_PER_SLOT;

/// How long the producer is given to put a block on the chain.
///
/// Bounded by proving: one block proof is seconds of zk work, more on a slow machine.
const BUILD_BUDGET: Duration = Duration::from_secs(900);

/// How long the follower is given to close the gap once it has a server to ask.
///
/// Bounded by verification, not by proving: nothing in this phase produces a block.
const CATCH_UP_BUDGET: Duration = Duration::from_secs(300);

/// How many blocks A puts on the chain before B is started.
///
/// One, and the reason is the cost of the second. A proposal is bounded by the slots its XMSS
/// key is prepared for; going past that window makes the producer advance the key first, which
/// is minutes of work and dominated this test's CI time by an order of magnitude. One block is
/// enough for the claim: it exists before B does, so B can only have it by asking, and the
/// counters at the end say whether it did.
const GAP_BLOCKS: u64 = 1;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn should_catch_up_to_a_running_node_when_started_after_it() {
    common::init_logging();
    let Some(keys) = common::test_keys() else {
        eprintln!("skipping: set VERITY_TEST_KEYS to run the catch-up test");
        return;
    };

    let first_signable = keys
        .iter()
        .map(|key| key.secret.prepared_interval().start)
        .max()
        .expect("two keys");
    let target_slot = first_signable + 1;
    assert!(
        target_slot < 16,
        "the supplied keys are prepared from slot {first_signable}; \
         starting a chain there would ask the nodes to tick through {target_slot} slots"
    );

    let root = tempfile::tempdir().expect("a temporary directory");
    let genesis_time = common::now_seconds() - target_slot * SECONDS_PER_SLOT;
    let (genesis_path, key_base) = common::write_configuration(root.path(), &keys, genesis_time);
    let genesis = GenesisFile::read(&genesis_path).expect("the genesis file");

    let chain_directory = root.path().join("chain-db");

    // The producer is the whole validator set, and it exists only to put history on disk.
    let producer = Node::start(NodeConfig {
        genesis: genesis.clone(),
        data_directory: chain_directory.clone(),
        listen: "/ip4/127.0.0.1/udp/0/quic-v1".parse().expect("an address"),
        bootnodes: Vec::new(),
        network_name: "00000000".to_string(),
        keypair: Keypair::generate_secp256k1(),
        validator_indices: vec![ValidatorIndex(0)],
        key_directory: Some(key_base.join("hash-sig-keys")),
        // Not an aggregator: the interval-2 round is another zk proof per slot and has
        // nothing to do with what this test claims. `single_node.rs` covers that path.
        is_aggregator: false,
        checkpoint_sync_url: None,
    })
    .await
    .expect("the producer starts");

    // Let a gap open before the follower exists. Everything below this point is history the
    // follower can only get by asking for it: gossip carries the current slot, not the past.
    //
    // The wait does not end at the head: it ends one tick later. A block is made durable by
    // its own commit, but the head pointer and the canonical index that a restart reads back
    // are written by the interval tick that follows it. Stopping in between leaves a database
    // holding the block and still pointing at genesis — correct, and useless to serve from.
    let mut producer_view = producer.view();
    let built = tokio::time::timeout(BUILD_BUDGET, async {
        let mut head_seen_at = None;
        loop {
            {
                let view = producer_view.borrow_and_update();
                match head_seen_at {
                    None if view.head_checkpoint().slot.0 >= GAP_BLOCKS => {
                        head_seen_at = Some(view.time());
                    }
                    Some(interval) if view.time().0 > interval.0 => return,
                    _ => {}
                }
            }
            producer_view
                .changed()
                .await
                .expect("the producer's chain task is running");
        }
    })
    .await;
    assert!(
        built.is_ok(),
        "the producer did not reach slot {GAP_BLOCKS} within {BUILD_BUDGET:?}; it is at slot {}",
        producer_view.borrow().head_checkpoint().slot.0
    );

    let target = producer_view.borrow().head_checkpoint();
    drop(producer_view);
    // The database has the chain now, and the keys have done their job. Everything after this
    // point is reads.
    producer.shutdown().await;

    // The server: the producer's history, no keys, nothing to prove.
    let server = Node::start(NodeConfig {
        genesis: genesis.clone(),
        data_directory: chain_directory,
        listen: "/ip4/127.0.0.1/udp/0/quic-v1".parse().expect("an address"),
        bootnodes: Vec::new(),
        network_name: "00000000".to_string(),
        keypair: Keypair::generate_secp256k1(),
        validator_indices: Vec::new(),
        key_directory: None,
        is_aggregator: false,
        checkpoint_sync_url: None,
    })
    .await
    .expect("the server starts on the producer's database");
    assert_eq!(
        server.view().borrow().head_checkpoint().slot,
        target.slot,
        "the server restored a different head than the producer left"
    );

    let bootnode = dialable(&server).await;

    // The follower holds no keys: it follows, and everything it ends up with came from the
    // server.
    let follower = Node::start(NodeConfig {
        genesis,
        data_directory: root.path().join("follower-db"),
        listen: "/ip4/127.0.0.1/udp/0/quic-v1".parse().expect("an address"),
        bootnodes: vec![bootnode],
        network_name: "00000000".to_string(),
        keypair: Keypair::generate_secp256k1(),
        validator_indices: Vec::new(),
        key_directory: None,
        is_aggregator: false,
        checkpoint_sync_url: None,
    })
    .await
    .expect("the follower starts");

    let mut follower_view = follower.view();
    let caught_up = tokio::time::timeout(CATCH_UP_BUDGET, async {
        loop {
            if follower_view
                .borrow_and_update()
                .block(target.root)
                .is_some()
            {
                return;
            }
            follower_view
                .changed()
                .await
                .expect("the follower's chain task is running");
        }
    })
    .await;

    let reached = follower_view.borrow().head_checkpoint();
    let fetched = follower.sync_counters().fetched();
    let (served_requests, served_blocks) = {
        let counters = server.sync_counters();
        (counters.served_requests(), counters.served_blocks())
    };
    follower.shutdown().await;
    server.shutdown().await;

    assert!(
        caught_up.is_ok(),
        "the follower never reached the producer's slot {}; its head is at slot {}",
        target.slot.0,
        reached.slot.0
    );

    // Arriving is not the claim; arriving *by asking* is. Gossipsub keeps a short history and
    // answers IWANT out of it, so a block that merely turned up could have come from the mesh
    // cache rather than from a request — and then this test would be checking gossip, not
    // sync. The two counters are what separate them, one per side of the exchange.
    assert!(
        fetched > 0,
        "the follower's head reached the target without fetching anything: \
         this run exercised gossip, not sync"
    );
    assert!(
        served_blocks > 0,
        "the server answered {served_requests} block request(s) and served no blocks"
    );
}

/// A node's bound address, with its peer id appended so it can be dialled.
///
/// Awaited rather than read: binding happens during `start`, but the address it produced
/// arrives on the event stream a moment later.
async fn dialable(node: &Node) -> Multiaddr {
    let mut listening = node.listening();
    let address = loop {
        if let Some(address) = listening.borrow_and_update().first().cloned() {
            break address;
        }
        listening
            .changed()
            .await
            .expect("the network bridge is running");
    };
    address
        .with_p2p(node.peer_id())
        .expect("an address with a peer id")
}
