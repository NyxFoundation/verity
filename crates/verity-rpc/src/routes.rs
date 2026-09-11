//! The routes, and the handler behind each.
//!
//! Every handler takes the shared [`ApiContext`], reads the view current at that moment,
//! and answers. The two SSZ routes encode a full state or block, which is CPU-heavy; leanSpec
//! moves that off its event loop, and so does this — onto the blocking pool, holding only a
//! clone of the snapshot's `Arc`.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use libssz::SszEncode;
use serde::Serialize;
use verity_metrics::TEXT_CONTENT_TYPE;

use crate::ApiContext;
use crate::json::{
    AggregatorStatusBody, AggregatorToggleBody, CheckpointBody, ForkChoiceBody, ForkChoiceNode,
    HealthBody, Hex32,
};
use crate::sample::on_scrape;

type Shared = Arc<ApiContext>;

/// The status leanSpec's health probe always reports.
const STATUS_HEALTHY: &str = "healthy";
/// The service identifier leanSpec's health probe always reports.
const SERVICE_NAME: &str = "lean-rpc-api";
/// The media type of the SSZ routes.
const SSZ_CONTENT_TYPE: &str = "application/octet-stream";

/// The REST API and the scrape endpoint together, for the API port.
///
/// `/v0/health` is served beside `/lean/v0/health` because leanpoint, as lean-quickstart
/// configures it (`convert-validator-config.py`), probes the shorter path.
pub fn api_router(context: Shared) -> Router {
    Router::new()
        .route("/lean/v0/health", get(health))
        .route("/v0/health", get(health))
        .route("/lean/v0/states/finalized", get(finalized_state))
        .route("/lean/v0/blocks/finalized", get(finalized_block))
        .route("/lean/v0/checkpoints/justified", get(justified_checkpoint))
        .route("/lean/v0/fork_choice", get(fork_choice))
        .route("/metrics", get(metrics))
        .route(
            "/lean/v0/admin/aggregator",
            get(aggregator_status).post(aggregator_toggle),
        )
        .with_state(context)
}

/// The scrape endpoint and the health probe alone, for the metrics port.
pub fn metrics_router(context: Shared) -> Router {
    Router::new()
        .route("/metrics", get(metrics))
        .route("/lean/v0/health", get(health))
        .with_state(context)
}

async fn health() -> Response {
    json(HealthBody {
        status: STATUS_HEALTHY,
        service: SERVICE_NAME,
    })
}

async fn metrics(State(context): State<Shared>) -> Response {
    on_scrape(&context);
    let mut response = context.metrics.render().into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(TEXT_CONTENT_TYPE),
    );
    response
}

async fn justified_checkpoint(State(context): State<Shared>) -> Response {
    let view = context.view.borrow().clone();
    json(CheckpointBody::from(view.latest_justified()))
}

async fn fork_choice(State(context): State<Shared>) -> Response {
    let view = context.view.borrow().clone();
    let finalized = view.latest_finalized();
    let weights = view.block_weights();

    let nodes = view
        .blocks()
        .filter(|(_, block)| block.slot.0 >= finalized.slot.0)
        .map(|(root, block)| ForkChoiceNode {
            root: Hex32(*root),
            slot: block.slot.0,
            parent_root: Hex32(block.parent_root),
            proposer_index: block.proposer_index.0,
            weight: weights.get(root).copied().unwrap_or(0),
        })
        .collect();

    json(ForkChoiceBody {
        nodes,
        head: Hex32(view.head()),
        justified: CheckpointBody::from(view.latest_justified()),
        finalized: CheckpointBody::from(finalized),
        safe_target: Hex32(view.safe_target()),
        validator_count: view.head_state().map(|state| state.validators.len()),
    })
}

async fn finalized_state(State(context): State<Shared>) -> Response {
    let view = context.view.borrow().clone();
    let encoded = tokio::task::spawn_blocking(move || {
        let root = view.latest_finalized().root;
        view.state(root).map(SszEncode::to_ssz)
    })
    .await;

    match encoded {
        Ok(Some(bytes)) => ssz(bytes),
        Ok(None) => problem(StatusCode::NOT_FOUND, "Finalized state not available"),
        Err(_) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Encoding failed"),
    }
}

async fn finalized_block(State(context): State<Shared>) -> Response {
    let root = context.view.borrow().latest_finalized().root;
    let source = Arc::clone(&context.signed_block);
    let encoded =
        tokio::task::spawn_blocking(move || source(root).map(|block| block.to_ssz())).await;

    match encoded {
        Ok(Some(bytes)) => ssz(bytes),
        Ok(None) => problem(
            StatusCode::NOT_FOUND,
            "Finalized signed block not available",
        ),
        Err(_) => problem(StatusCode::INTERNAL_SERVER_ERROR, "Encoding failed"),
    }
}

async fn aggregator_status(State(context): State<Shared>) -> Response {
    json(AggregatorStatusBody {
        is_aggregator: context.aggregator.load(Ordering::Relaxed),
    })
}

/// Sets the role from `{"enabled": <bool>}` and reports the value it replaced.
///
/// The body is parsed by hand so that `0` and `1`, which a lenient decoder would accept as
/// booleans, are refused as leanSpec refuses them.
async fn aggregator_toggle(State(context): State<Shared>, body: Bytes) -> Response {
    let Ok(request) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return problem(StatusCode::BAD_REQUEST, "Invalid JSON body");
    };
    let Some(enabled) = request.get("enabled") else {
        return problem(StatusCode::BAD_REQUEST, "Missing 'enabled' field in body");
    };
    let Some(enabled) = enabled.as_bool() else {
        return problem(StatusCode::BAD_REQUEST, "'enabled' must be a boolean");
    };

    let previous = context.aggregator.swap(enabled, Ordering::Relaxed);
    if previous != enabled {
        tracing::info!(
            "aggregator role {} via admin API",
            if enabled { "activated" } else { "deactivated" }
        );
    }
    json(AggregatorToggleBody {
        is_aggregator: enabled,
        previous,
    })
}

fn json<T: Serialize>(body: T) -> Response {
    Json(body).into_response()
}

fn ssz(bytes: Vec<u8>) -> Response {
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(SSZ_CONTENT_TYPE),
    );
    response
}

fn problem(status: StatusCode, reason: &'static str) -> Response {
    (status, reason).into_response()
}
