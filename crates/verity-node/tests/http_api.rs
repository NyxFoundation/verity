//! The HTTP surface, driven over real sockets the way its consumers drive it: leanpoint's
//! health probe, Prometheus' scrape, and a peer's checkpoint fetch.
//!
//! A follower node at genesis is enough: every route answers from the snapshot, and the
//! snapshot at genesis is fully known — one block, one state, both checkpoints on it. No keys
//! are needed, so this runs in the fast gate.

mod common;

use std::net::SocketAddr;

use libssz::SszDecode;
use verity_node::{GenesisFile, Node, NodeConfig, identity::Keypair};
use verity_types::State;

/// A genesis file with one validator whose keys are never used: this node follows.
fn genesis_file(root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("config.yaml");
    std::fs::write(
        &path,
        format!(
            "GENESIS_TIME: {}\nGENESIS_VALIDATORS:\n  - attestation_pubkey: \"{}\"\n    proposal_pubkey: \"{}\"\n",
            common::now_seconds(),
            "11".repeat(52),
            "22".repeat(52)
        ),
    )
    .expect("the genesis file");
    path
}

async fn start_follower(root: &std::path::Path) -> Node {
    Node::start(NodeConfig {
        genesis: GenesisFile::read(&genesis_file(root)).expect("the genesis file"),
        data_directory: root.join("db"),
        listen: "/ip4/127.0.0.1/udp/0/quic-v1"
            .parse()
            .expect("a listen address"),
        bootnodes: Vec::new(),
        network_name: "12345678".to_string(),
        keypair: Keypair::generate_secp256k1(),
        validator_indices: Vec::new(),
        key_directory: None,
        is_aggregator: false,
        checkpoint_sync_url: None,
        api_address: Some(loopback()),
        metrics_address: Some(loopback()),
        version: "test".to_string(),
    })
    .await
    .expect("the node starts")
}

/// reqwest is built with `rustls-no-provider`, so a client cannot be built until a provider
/// is installed — the node does this itself before a checkpoint fetch, and a test that never
/// fetches one has to do it here.
fn http_client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}

fn loopback() -> SocketAddr {
    "127.0.0.1:0".parse().expect("a socket address")
}

fn url(address: SocketAddr, path: &str) -> String {
    format!("http://{address}{path}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn should_answer_every_cross_client_route_from_the_genesis_snapshot() {
    common::init_logging();
    let root = tempfile::tempdir().expect("a temporary directory");
    let node = start_follower(root.path()).await;
    let api = node.api_address().expect("the API is served");
    let client = http_client();
    let view = node.view().borrow().clone();

    // Health, on both spellings.
    for path in ["/lean/v0/health", "/v0/health"] {
        let response = client.get(url(api, path)).send().await.expect("health");
        assert_eq!(response.status(), 200, "{path}");
        assert_eq!(
            response.headers()["content-type"],
            "application/json",
            "{path}"
        );
        let body: serde_json::Value =
            serde_json::from_str(&response.text().await.expect("text")).expect("json");
        assert_eq!(body["status"], "healthy");
        assert_eq!(body["service"], "lean-rpc-api");
    }

    // The justified checkpoint is the genesis anchor.
    let anchor = view.latest_finalized();
    let body = client
        .get(url(api, "/lean/v0/checkpoints/justified"))
        .send()
        .await
        .expect("justified")
        .text()
        .await
        .expect("text");
    let body: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(body["slot"], 0);
    assert_eq!(body["root"], format!("0x{}", common::hex(&anchor.root)));

    // The fork-choice tree is the anchor alone.
    let body = client
        .get(url(api, "/lean/v0/fork_choice"))
        .send()
        .await
        .expect("fork choice")
        .text()
        .await
        .expect("text");
    let body: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(body["nodes"].as_array().map(Vec::len), Some(1));
    assert_eq!(body["nodes"][0]["slot"], 0);
    assert_eq!(body["nodes"][0]["weight"], 0);
    assert_eq!(body["head"], body["nodes"][0]["root"]);
    assert_eq!(body["validator_count"], 1);
    assert_eq!(body["finalized"]["slot"], 0);

    // The finalized state is the genesis state, byte for byte.
    let response = client
        .get(url(api, "/lean/v0/states/finalized"))
        .send()
        .await
        .expect("state");
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    let state = State::from_ssz_bytes(&response.bytes().await.expect("bytes")).expect("a state");
    assert_eq!(Some(&state), view.state(anchor.root));

    // The anchor was adopted from configuration and carries no proof, so there is no signed
    // block to serve — which leanSpec reports as 404, not as an empty proof.
    let response = client
        .get(url(api, "/lean/v0/blocks/finalized"))
        .send()
        .await
        .expect("block");
    assert_eq!(response.status(), 404);

    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn should_expose_the_lean_metrics_on_both_ports() {
    common::init_logging();
    let root = tempfile::tempdir().expect("a temporary directory");
    let node = start_follower(root.path()).await;
    let client = http_client();

    for address in [
        node.api_address().expect("api"),
        node.metrics_address().expect("metrics"),
    ] {
        let response = client
            .get(url(address, "/metrics"))
            .send()
            .await
            .expect("scrape");
        assert_eq!(response.status(), 200);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .expect("ascii")
                .starts_with("text/plain; version=0.0.4")
        );
        let text = response.text().await.expect("text");
        for line in [
            "lean_node_info{name=\"verity\",version=\"test\"} 1",
            "lean_head_slot 0",
            "lean_latest_finalized_slot 0",
            "lean_validators_count 0",
            "lean_is_aggregator 0",
            "lean_attestation_committee_count 1",
            "lean_node_sync_status{status=\"syncing\"} 1",
            "lean_current_slot",
        ] {
            assert!(
                text.contains(line),
                "missing `{line}` on {address}:\n{text}"
            );
        }
    }

    node.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn should_toggle_the_aggregator_role_through_the_admin_route() {
    common::init_logging();
    let root = tempfile::tempdir().expect("a temporary directory");
    let node = start_follower(root.path()).await;
    let api = node.api_address().expect("api");
    let client = http_client();
    let admin = url(api, "/lean/v0/admin/aggregator");

    let body = client
        .get(&admin)
        .send()
        .await
        .expect("status")
        .text()
        .await
        .expect("text");
    let body: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(body["is_aggregator"], false);

    let body = client
        .post(&admin)
        .body("{\"enabled\": true}")
        .send()
        .await
        .expect("toggle")
        .text()
        .await
        .expect("text");
    let body: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(body["is_aggregator"], true);
    assert_eq!(body["previous"], false);
    assert!(
        node.is_aggregator(),
        "the chain task's flag follows the route"
    );

    // Integers are not booleans, and neither is a missing field or a broken body.
    for body in ["{\"enabled\": 1}", "{}", "not json"] {
        let response = client.post(&admin).body(body).send().await.expect("post");
        assert_eq!(response.status(), 400, "{body}");
    }
    assert!(node.is_aggregator(), "a refused request changes nothing");

    node.shutdown().await;
}
