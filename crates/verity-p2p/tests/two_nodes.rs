//! Two real nodes on localhost QUIC: gossip and all three req/resp protocols, end to end.
//!
//! This is the crate's "does it actually network" test: two swarms, real UDP sockets on
//! 127.0.0.1, the full stack from topic subscription through snappy framing. Everything
//! here drives the public service API only — no reaching into the swarm.

use std::time::Duration;

use libssz::SszEncode;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};
use verity_p2p::{
    BlocksByRangeRequest, BlocksByRootRequest, BuildError, CommandError, ErrorCode, GossipKind,
    Multiaddr, NetworkConfig, NetworkEvent, NetworkEvents, NetworkHandle, PublishError, Request,
    RequestError, Response, Status, identity,
};
use verity_types::{Checkpoint, SignedBlock, Slot};

/// One network name for the whole test; both nodes must agree or gossip is discarded.
const NETWORK_NAME: &str = "0badf00d";

/// Generous per-stage deadline; QUIC handshakes and gossipsub heartbeats are fast, but CI
/// machines are not.
const STAGE_TIMEOUT: Duration = Duration::from_secs(30);

fn node_config() -> NetworkConfig {
    NetworkConfig::new(
        identity::Keypair::generate_secp256k1(),
        "/ip4/127.0.0.1/udp/0/quic-v1".parse().expect("multiaddr"),
        NETWORK_NAME.to_string(),
    )
}

/// Reads events until `select` yields, discarding everything else.
async fn wait_for<T>(
    events: &mut NetworkEvents,
    mut select: impl FnMut(NetworkEvent) -> Option<T>,
) -> T {
    timeout(STAGE_TIMEOUT, async {
        loop {
            let event = events.recv().await.expect("event stream closed");
            if let Some(found) = select(event) {
                return found;
            }
        }
    })
    .await
    .expect("timed out waiting for an event")
}

fn sample_status() -> Status {
    Status {
        finalized: Checkpoint {
            root: [0xaa; 32],
            slot: Slot(96),
        },
        head: Checkpoint {
            root: [0xbb; 32],
            slot: Slot(128),
        },
    }
}

/// Answers every inbound request on `events`, and forwards gossip payloads for the test
/// body to assert on.
fn run_responder(
    handle: NetworkHandle,
    mut events: NetworkEvents,
    gossip: mpsc::Sender<(GossipKind, Vec<u8>)>,
    served_blocks: Vec<Vec<u8>>,
) {
    tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            match event {
                NetworkEvent::InboundRequest {
                    request, channel, ..
                } => {
                    let response = match request {
                        Request::Status(_) => Response::Status(sample_status()),
                        Request::BlocksByRange(_) => Response::Blocks(served_blocks.clone()),
                        Request::BlocksByRoot(_) => Response::Error {
                            code: ErrorCode::ResourceUnavailable,
                            message: "pruned below the serving window".to_string(),
                        },
                    };
                    handle.respond(channel, response).await.expect("respond");
                }
                NetworkEvent::Gossip { kind, payload } => {
                    gossip.send((kind, payload)).await.expect("forward gossip");
                }
                _ => {}
            }
        }
    });
}

async fn connected_nodes() -> (
    NetworkHandle,
    NetworkEvents,
    NetworkHandle,
    NetworkEvents,
    verity_p2p::PeerId,
) {
    let (handle_a, mut events_a, _task_a) = verity_p2p::spawn(node_config())
        .await
        .expect("spawn node A");
    let listen_a = wait_for(&mut events_a, |event| match event {
        NetworkEvent::NewListenAddr(address) => Some(address),
        _ => None,
    })
    .await;
    let addr_a = listen_a
        .with_p2p(handle_a.local_peer_id())
        .expect("address with peer id");

    let mut config_b = node_config();
    config_b.bootnodes = vec![addr_a];
    let (handle_b, mut events_b, _task_b) =
        verity_p2p::spawn(config_b).await.expect("spawn node B");
    let peer_a = handle_a.local_peer_id();
    wait_for(&mut events_b, |event| match event {
        NetworkEvent::PeerConnected { peer, .. } if peer == peer_a => Some(()),
        _ => None,
    })
    .await;
    (handle_a, events_a, handle_b, events_b, peer_a)
}

/// Publishes with retries while the mesh forms; gossipsub refuses to publish before a
/// peer subscription for the topic is known.
async fn publish_when_meshed(handle: &NetworkHandle, kind: GossipKind, payload: Vec<u8>) {
    timeout(STAGE_TIMEOUT, async {
        loop {
            match handle.publish(kind, payload.clone()).await {
                Ok(()) => return,
                Err(PublishError::InsufficientPeers) => sleep(Duration::from_millis(200)).await,
                Err(other) => panic!("publish failed: {other}"),
            }
        }
    })
    .await
    .expect("mesh never formed");
}

#[tokio::test(flavor = "multi_thread")]
async fn should_serve_gossip_and_all_reqresp_protocols_between_two_nodes() {
    // Node A listens; its bound address becomes known through the event stream.
    let (handle_a, mut events_a, _task_a) = verity_p2p::spawn(node_config())
        .await
        .expect("spawn node A");
    let listen_a = wait_for(&mut events_a, |event| match event {
        NetworkEvent::NewListenAddr(address) => Some(address),
        _ => None,
    })
    .await;
    let addr_a: Multiaddr = listen_a
        .with_p2p(handle_a.local_peer_id())
        .expect("address with peer id");

    // Node B boots with A as its only bootnode.
    let mut config_b = node_config();
    config_b.bootnodes = vec![addr_a];
    let (handle_b, mut events_b, _task_b) =
        verity_p2p::spawn(config_b).await.expect("spawn node B");
    let peer_a = handle_a.local_peer_id();

    let direction = wait_for(&mut events_b, |event| match event {
        NetworkEvent::PeerConnected { peer, direction } if peer == peer_a => Some(direction),
        _ => None,
    })
    .await;
    assert_eq!(direction, verity_p2p::Direction::Outbound, "B dialed A");

    // From here node A is driven by a responder task; the test body plays node B.
    let served_blocks = vec![
        SignedBlock::default().to_ssz(),
        SignedBlock::default().to_ssz(),
    ];
    let (gossip_tx, mut gossip_rx) = mpsc::channel(8);
    run_responder(handle_a.clone(), events_a, gossip_tx, served_blocks.clone());

    // Status: B asks, A answers with its checkpoints.
    let response = handle_b
        .request(peer_a, Request::Status(Status::default()))
        .await
        .expect("status request");
    assert_eq!(response, Response::Status(sample_status()));

    // BlocksByRange: chunks come back byte-for-byte, in order.
    let response = handle_b
        .request(
            peer_a,
            Request::BlocksByRange(BlocksByRangeRequest {
                start_slot: Slot(0),
                count: 2,
            }),
        )
        .await
        .expect("range request");
    assert_eq!(response, Response::Blocks(served_blocks));

    // BlocksByRoot: the responder's error crosses intact, code and message.
    let response = handle_b
        .request(
            peer_a,
            Request::BlocksByRoot(BlocksByRootRequest::default()),
        )
        .await
        .expect("root request");
    assert_eq!(
        response,
        Response::Error {
            code: ErrorCode::ResourceUnavailable,
            message: "pruned below the serving window".to_string(),
        }
    );

    // Gossip: B publishes a block payload; A receives it decompressed on the block topic.
    let payload = SignedBlock::default().to_ssz();
    publish_when_meshed(&handle_b, GossipKind::Block, payload.clone()).await;
    let (kind, received) = timeout(STAGE_TIMEOUT, gossip_rx.recv())
        .await
        .expect("timed out waiting for gossip")
        .expect("gossip channel closed");
    assert_eq!(kind, GossipKind::Block);
    assert_eq!(received, payload);
}

#[tokio::test(flavor = "multi_thread")]
async fn should_report_a_response_sent_after_the_request_timeout() {
    let (handle_a, mut events_a, handle_b, _events_b, peer_a) = connected_nodes().await;
    let request = tokio::spawn(async move {
        handle_b
            .request(peer_a, Request::Status(Status::default()))
            .await
    });
    let channel = wait_for(&mut events_a, |event| match event {
        NetworkEvent::InboundRequest { channel, .. } => Some(channel),
        _ => None,
    })
    .await;

    let request_result = timeout(STAGE_TIMEOUT, request)
        .await
        .expect("request timeout was not reported")
        .expect("request task");
    // Both ends share the deadline: the requester can report its own timeout, or EOF when
    // the responder expires the stream first.
    assert!(matches!(
        request_result,
        Err(RequestError::Timeout | RequestError::Io(_))
    ));
    // Allow A's swarm to consume its timeout event before submitting the late response.
    sleep(Duration::from_secs(1)).await;

    let result = handle_a
        .respond(channel, Response::Status(sample_status()))
        .await;
    assert!(matches!(result, Err(CommandError::Failed(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn should_report_a_response_channel_closed_with_its_peer() {
    let (handle_a, mut events_a, handle_b, events_b, peer_a) = connected_nodes().await;
    let request = tokio::spawn({
        let handle = handle_b.clone();
        async move {
            handle
                .request(peer_a, Request::Status(Status::default()))
                .await
        }
    });
    let (peer_b, channel) = wait_for(&mut events_a, |event| match event {
        NetworkEvent::InboundRequest { peer, channel, .. } => Some((peer, channel)),
        _ => None,
    })
    .await;

    request.abort();
    let _ = request.await;
    drop(handle_b);
    drop(events_b);
    wait_for(&mut events_a, |event| match event {
        NetworkEvent::PeerDisconnected { peer, .. } if peer == peer_b => Some(()),
        _ => None,
    })
    .await;

    let result = handle_a
        .respond(channel, Response::Status(sample_status()))
        .await;
    assert!(matches!(result, Err(CommandError::Failed(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn should_reject_a_listen_address_whose_udp_port_is_occupied() {
    let occupied = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("reserve a UDP port");
    let port = occupied.local_addr().expect("local address").port();
    let mut config = node_config();
    config.listen = format!("/ip4/127.0.0.1/udp/{port}/quic-v1")
        .parse()
        .expect("multiaddr");

    let result = verity_p2p::spawn(config).await;

    assert!(matches!(result, Err(BuildError::Listen(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn should_report_dial_failure_when_peer_has_no_known_address() {
    let (handle, _events, _task) = verity_p2p::spawn(node_config()).await.expect("spawn");
    let stranger = identity::Keypair::generate_secp256k1()
        .public()
        .to_peer_id();
    let result = timeout(
        STAGE_TIMEOUT,
        handle.request(stranger, Request::Status(Status::default())),
    )
    .await
    .expect("request never resolved");
    assert_eq!(result, Err(RequestError::DialFailure));
}
