//! Two nodes, one gap: does a node that starts late reach the chain the first one built?
//!
//! This is the only test that exercises both halves of sync at once, and it exercises them
//! against each other. For B to catch up, A has to answer block requests out of its own
//! database — the responder — and B has to notice the gap, ask for what it is missing, and
//! feed the answers through its verification stage — the client. Neither half can pass this
//! alone, and neither is reachable from a unit test.
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

/// How long the pair is given to build a chain and then close the gap.
///
/// Bounded by proving on A's side and by verification on B's: every block B accepts costs it
/// the same aggregate-proof check a gossiped block would.
const BUDGET: Duration = Duration::from_secs(900);

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

    // A is the whole validator set: it proposes every slot it can sign for.
    let producer = Node::start(NodeConfig {
        genesis: genesis.clone(),
        data_directory: root.path().join("producer-db"),
        listen: "/ip4/127.0.0.1/udp/0/quic-v1".parse().expect("an address"),
        bootnodes: Vec::new(),
        network_name: "00000000".to_string(),
        keypair: Keypair::generate_secp256k1(),
        validator_indices: vec![ValidatorIndex(0)],
        key_directory: Some(key_base.join("hash-sig-keys")),
        // Not an aggregator, deliberately. The interval-2 round is a zk proof per slot and
        // has nothing to do with what this test claims; leaving it on made the producer prove
        // continuously underneath the follower's catch-up, and the two together do not fit in
        // a CI runner's memory. `single_node.rs` covers the aggregating path.
        is_aggregator: false,
        checkpoint_sync_url: None,
    })
    .await
    .expect("the producer starts");

    // Let a gap open before the follower exists. Everything below this point is history the
    // follower can only get by asking for it: gossip carries the current slot, not the past.
    let mut producer_view = producer.view();
    let built = tokio::time::timeout(BUDGET, async {
        loop {
            if producer_view.borrow_and_update().head_checkpoint().slot.0 >= GAP_BLOCKS {
                return;
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
        "the producer did not reach slot {GAP_BLOCKS} within {BUDGET:?}; it is at slot {}",
        producer_view.borrow().head_checkpoint().slot.0
    );

    let target = producer_view.borrow().head_checkpoint();
    let bootnode = dialable(&producer);

    // B holds no keys: it follows, and everything it ends up with came from A.
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
    let caught_up = tokio::time::timeout(BUDGET, async {
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
        let counters = producer.sync_counters();
        (counters.served_requests(), counters.served_blocks())
    };
    follower.shutdown().await;
    producer.shutdown().await;

    assert!(
        caught_up.is_ok(),
        "the follower never imported the producer's block at slot {}; its head is at slot {}",
        target.slot.0,
        reached.slot.0
    );
    assert!(
        reached.slot.0 >= target.slot.0,
        "the follower holds the block but its head is behind it"
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
        "the producer answered {served_requests} block request(s) and served no blocks"
    );
}

/// The producer's bound address, with its peer id appended so it can be dialled.
fn dialable(producer: &Node) -> Multiaddr {
    let address = producer
        .listen_addresses()
        .into_iter()
        .next()
        .expect("the producer is listening");
    address
        .with_p2p(producer.peer_id())
        .expect("an address with a peer id")
}
