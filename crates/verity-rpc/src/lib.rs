//! The node's HTTP surface: the cross-client REST API and the Prometheus scrape endpoint.
//!
//! Transcribed from leanSpec `src/lean_spec/node/api/{server,handlers,responses,context}.py`
//! at commit `0b7d33ec`. The routes, the JSON field names and the response media types are
//! the wire contract every lean client shares; a checkpoint-syncing peer fetches
//! `/lean/v0/states/finalized` from any of them, and leanpoint probes any of them for health.
//!
//! # What the handlers read
//!
//! Every answer comes from the [`ChainView`] current when the request arrives — the same
//! snapshot channel validator duties and the verification stage answer from
//! (`docs/design/concurrency.md`). There is no query into the chain task and nothing here
//! can block it. The one thing the view does not hold, the finalized block's proof, comes
//! from a [`SignedBlockSource`] the node wires from its read-only database handle.
//!
//! # Two listeners, one router shape
//!
//! lean-quickstart gives every node an API port and a metrics port. [`api_router`] serves
//! both the REST API and `/metrics`, as leanSpec's single server does; [`metrics_router`]
//! serves `/metrics` and the health probe alone, for the port Prometheus is pointed at.
//!
//! # Bind first, serve later
//!
//! [`bind`] takes the port and [`BoundListener::serve`] starts answering on it, as two steps:
//! a node binds before it opens its database so that a port in use fails it with nothing
//! started, and serves once the snapshot channel the handlers read from exists.
//!
//! # The admin route is unauthenticated
//!
//! As in leanSpec: `/lean/v0/admin/aggregator` toggles the aggregator role at runtime and
//! trusts whoever can reach the port. A deployment restricts it at the network layer.

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use verity_chain::ChainView;
use verity_metrics::Metrics;
use verity_types::{Bytes32, SignedBlock};

pub mod json;
mod routes;
mod sample;

pub use routes::{api_router, metrics_router};

/// Resolves a block root to its signed block, proof included, or to nothing.
///
/// The view holds blocks without their proofs — a proof is verified once and stored, never
/// read again on the consensus path — so serving a finalized `SignedBlock` is a database
/// read the node wires in. Returning `None` means "not available", which the route reports
/// as 404 exactly as leanSpec does for a root its source cannot produce.
pub type SignedBlockSource = Arc<dyn Fn(Bytes32) -> Option<SignedBlock> + Send + Sync>;

/// What the handlers need, resolved once when the node wires the server.
pub struct ApiContext {
    /// The snapshot channel every reader answers from.
    pub view: watch::Receiver<Arc<ChainView>>,
    /// Whether the node considers itself caught up.
    pub synced: watch::Receiver<bool>,
    /// Where a finalized block's proof comes from.
    pub signed_block: SignedBlockSource,
    /// The aggregator role, shared with the chain task so the admin route can flip it.
    pub aggregator: Arc<AtomicBool>,
    /// The process's metric registry.
    pub metrics: Arc<Metrics>,
}

/// A listener that could not be bound.
#[derive(Debug)]
pub struct BindError {
    /// The address that was requested.
    pub address: SocketAddr,
    /// The operating system's reason, rendered.
    pub reason: String,
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot bind {}: {}", self.address, self.reason)
    }
}

impl std::error::Error for BindError {}

/// A bound listener that is not serving yet.
#[derive(Debug)]
pub struct BoundListener {
    listener: TcpListener,
    local_addr: SocketAddr,
}

impl BoundListener {
    /// The address the listener actually bound. With port 0 requested, this is where the
    /// chosen port appears.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Starts serving `router` on this listener, until [`HttpServer::shutdown`].
    #[must_use = "the server stops when its handle is dropped"]
    pub fn serve(self, router: Router) -> HttpServer {
        let local_addr = self.local_addr;
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(async move {
            let served = axum::serve(self.listener, router)
                .with_graceful_shutdown(async move {
                    let _ = stopped.await;
                })
                .await;
            if let Err(error) = served {
                tracing::error!(%local_addr, %error, "http server stopped with an error");
            }
        });
        tracing::info!(%local_addr, "http server listening");

        HttpServer {
            local_addr,
            stop: Some(stop),
            task,
        }
    }
}

/// A running HTTP server, and the handle that stops it.
pub struct HttpServer {
    local_addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl HttpServer {
    /// The address the server is listening on.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stops accepting, lets in-flight requests finish, and waits for the task to end.
    pub async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = (&mut self.task).await;
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        // A server dropped without `shutdown` must not keep serving from a task nobody
        // holds; aborting is the only handle left at that point.
        self.task.abort();
    }
}

/// Binds `address` without serving anything on it yet.
///
/// # Errors
///
/// [`BindError`] when the listener cannot be bound — the port is taken, or the address is
/// not local. Nothing is spawned on failure.
pub async fn bind(address: SocketAddr) -> Result<BoundListener, BindError> {
    let listener = TcpListener::bind(address)
        .await
        .map_err(|error| BindError {
            address,
            reason: error.to_string(),
        })?;
    let local_addr = listener.local_addr().map_err(|error| BindError {
        address,
        reason: error.to_string(),
    })?;
    Ok(BoundListener {
        listener,
        local_addr,
    })
}

/// Binds `address` and serves `router` on it: [`bind`] and [`BoundListener::serve`] in one.
///
/// # Errors
///
/// [`BindError`] as for [`bind`].
pub async fn serve(address: SocketAddr, router: Router) -> Result<HttpServer, BindError> {
    Ok(bind(address).await?.serve(router))
}
