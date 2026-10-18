//! Finalized-block route behavior across the real HTTP boundary.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use tokio::sync::watch;
use verity_chain::{ChainView, Store, generate_genesis, hash_tree_root};
use verity_metrics::Metrics;
use verity_rpc::{ApiContext, SignedBlockSource, api_router};
use verity_types::{Block, Validators};

fn context(signed_block: SignedBlockSource) -> Arc<ApiContext> {
    let state = generate_genesis(0, Validators::default());
    let block = Block {
        state_root: hash_tree_root(&state),
        ..Block::default()
    };
    let store = Store::new(&state, &block, None).expect("valid anchor");
    let (_, view) = watch::channel(Arc::new(ChainView::of(&store)));
    let (_, synced) = watch::channel(false);

    Arc::new(ApiContext {
        view,
        synced,
        signed_block,
        aggregator: Arc::new(AtomicBool::new(false)),
        metrics: Arc::new(Metrics::new().expect("metrics")),
        samplers: Vec::new(),
    })
}

async fn finalized_block_response(source: SignedBlockSource) -> reqwest::Response {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let address = "127.0.0.1:0".parse().expect("socket address");
    let server = verity_rpc::serve(address, api_router(context(source)))
        .await
        .expect("server");
    let response = reqwest::get(format!(
        "http://{}/lean/v0/blocks/finalized",
        server.local_addr()
    ))
    .await
    .expect("request");
    server.shutdown().await;
    response
}

#[tokio::test]
async fn should_return_not_found_when_the_finalized_block_is_absent() {
    let source: SignedBlockSource = Arc::new(|_| Ok(None));

    let response = finalized_block_response(source).await;

    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn should_return_internal_error_when_the_finalized_block_read_fails() {
    let source: SignedBlockSource =
        Arc::new(|_| Err(std::io::Error::other("injected storage failure").into()));

    let response = finalized_block_response(source).await;

    assert_eq!(
        response.status(),
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert!(
        !response
            .text()
            .await
            .expect("response text")
            .contains("injected storage failure")
    );
}
