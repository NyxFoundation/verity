//! Startup is transactional with respect to durable chain state.

use std::fs;
use std::path::Path;

use tokio::net::UdpSocket;
use verity_node::error::NodeError;
use verity_node::{GenesisFile, Node, NodeConfig, identity};
use verity_types::ValidatorIndex;

fn genesis(root: &Path) -> GenesisFile {
    let path = root.join("genesis.yaml");
    let attestation = "00".repeat(52);
    let proposal = "01".repeat(52);
    fs::write(
        &path,
        format!(
            "GENESIS_TIME: 0\nGENESIS_VALIDATORS:\n  - attestation_public_key: \"0x{attestation}\"\n    proposal_public_key: \"0x{proposal}\"\n"
        ),
    )
    .expect("write genesis");
    GenesisFile::read(&path).expect("read genesis")
}

fn config(root: &Path, listen: verity_node::Multiaddr) -> NodeConfig {
    NodeConfig {
        genesis: genesis(root),
        data_directory: root.join("db"),
        listen,
        bootnodes: Vec::new(),
        network_name: "12345678".to_string(),
        keypair: identity::Keypair::generate_secp256k1(),
        validator_indices: Vec::new(),
        key_directory: None,
        is_aggregator: false,
        checkpoint_sync_url: None,
        api_address: None,
        metrics_address: None,
        version: "test".to_string(),
    }
}

fn ephemeral_listen() -> verity_node::Multiaddr {
    "/ip4/127.0.0.1/udp/0/quic-v1".parse().expect("multiaddr")
}

async fn failed_start(config: NodeConfig) -> NodeError {
    match Node::start(config).await {
        Err(error) => error,
        Ok(node) => {
            node.shutdown().await;
            panic!("node unexpectedly started");
        }
    }
}

#[tokio::test]
async fn should_not_write_an_anchor_when_validator_keys_cannot_be_loaded() {
    let root = tempfile::tempdir().expect("temporary directory");
    let key_directory = root.path().join("missing-keys");
    let data_directory = root.path().join("db");
    let mut config = config(root.path(), ephemeral_listen());
    config.validator_indices = vec![ValidatorIndex(0)];
    config.key_directory = Some(key_directory);

    let error = failed_start(config).await;

    assert!(matches!(error, NodeError::Validator(_)));
    assert!(!data_directory.exists());
}

#[tokio::test]
async fn should_not_write_an_anchor_when_the_quic_listener_cannot_bind() {
    let root = tempfile::tempdir().expect("temporary directory");
    let occupied = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("reserve UDP port");
    let port = occupied.local_addr().expect("local address").port();
    let listen = format!("/ip4/127.0.0.1/udp/{port}/quic-v1")
        .parse()
        .expect("multiaddr");
    let data_directory = root.path().join("db");

    let error = failed_start(config(root.path(), listen)).await;

    assert!(matches!(error, NodeError::Network(_)));
    assert!(!data_directory.exists());
}
