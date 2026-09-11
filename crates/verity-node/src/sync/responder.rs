//! The serving side: answering another node's block requests out of this node's database.
//!
//! leanSpec's one MUST for the ReqResp suite is here — a responder serves `BlocksByRange`
//! over the sliding `MIN_SLOTS_FOR_BLOCK_REQUESTS` window, and answers `RESOURCE_UNAVAILABLE`
//! below it rather than with a short response. The window itself, and the floor a
//! checkpoint-synced node advertises instead of slot zero, are `verity-db`'s
//! (`docs/design/storage.md`); this module is the protocol face of them.
//!
//! # Why it is a task of its own
//!
//! A full window is 1,024 blocks, and a block carries an aggregate proof of 155–236 KB. One
//! answered request can therefore be hundreds of megabytes off disk, which is not work that
//! may happen on the chain writer's thread — nor on the network bridge, which has to keep
//! draining gossip. The responder reads through a read-only handle on the same open database
//! (`verity-db`'s [`StorageReader`]), so it neither blocks the writer nor can write itself.

use std::sync::Arc;

use libssz::SszEncode;
use tokio::sync::{mpsc, watch};

use verity_chain::ChainView;
use verity_db::{Repository, StorageError, StorageReader};
use verity_p2p::{
    BlocksByRangeRequest, BlocksByRootRequest, ErrorCode, MAX_REQUEST_BLOCKS, NetworkHandle,
    PeerId, Response, ResponseChannel,
};
use verity_types::{Bytes32, SignedBlock, Slot};

use crate::store_open::{block_from, root_prefix};
use crate::sync::SyncCounters;

/// A block request taken off the network bridge, with the channel its answer goes back on.
///
/// Only the two block protocols reach here. Status is answered from the snapshot on the
/// bridge itself, so there is no third variant and no impossible case to handle.
#[derive(Debug)]
pub struct BlockRequest {
    /// The peer that asked.
    pub peer: PeerId,
    /// What it asked for.
    pub kind: BlockRequestKind,
    /// Where the answer goes.
    pub channel: ResponseChannel,
}

/// The two block protocols.
#[derive(Debug)]
pub enum BlockRequestKind {
    /// Specific blocks, by root.
    ByRoot(BlocksByRootRequest),
    /// A contiguous slot range.
    ByRange(BlocksByRangeRequest),
}

/// Serves block requests from the database, one at a time.
pub struct BlockResponder<B> {
    repository: Arc<Repository<B>>,
    view: watch::Receiver<Arc<ChainView>>,
    requests: mpsc::Receiver<BlockRequest>,
    handle: NetworkHandle,
    counters: Arc<SyncCounters>,
}

impl<B: StorageReader + Send + Sync + 'static> BlockResponder<B> {
    /// Wires the responder to its read-only database handle and the bridge's request stream.
    #[must_use = "a responder does nothing until it is run"]
    pub fn new(
        repository: Arc<Repository<B>>,
        view: watch::Receiver<Arc<ChainView>>,
        requests: mpsc::Receiver<BlockRequest>,
        handle: NetworkHandle,
        counters: Arc<SyncCounters>,
    ) -> Self {
        Self {
            repository,
            view,
            requests,
            handle,
            counters,
        }
    }

    /// Answers requests until the bridge stops forwarding them.
    ///
    /// Requests are served one at a time on purpose: the point of the separate task is to
    /// keep large reads off the writer and off the gossip path, not to let a peer open as
    /// many concurrent disk walks as it likes.
    pub async fn run(mut self) {
        while let Some(BlockRequest {
            peer,
            kind,
            channel,
        }) = self.requests.recv().await
        {
            let current_slot = self.view.borrow().slot();
            let repository = Arc::clone(&self.repository);

            // Off the async threads: this is disk I/O measured in hundreds of megabytes. A
            // join error means the blocking pool is gone, which only happens as the runtime
            // shuts down.
            let read =
                tokio::task::spawn_blocking(move || answer(&repository, current_slot, &kind));
            let Ok(response) = read.await else {
                break;
            };

            self.counters.record_served(match &response {
                Response::Blocks(chunks) => chunks.len(),
                // A refusal is a served request that carried no blocks.
                _ => 0,
            });
            if self.handle.respond(channel, response).await.is_err() {
                break;
            }
            tracing::trace!(%peer, "served a block request");
        }
    }
}

/// Builds the answer to one block request.
///
/// # Panics
///
/// Never: every storage failure becomes a `SERVER_ERROR` response, because a peer's malformed
/// or unlucky request is not a reason to stop a consensus node.
#[must_use]
pub fn answer<B: StorageReader>(
    repository: &Repository<B>,
    current_slot: Slot,
    request: &BlockRequestKind,
) -> Response {
    let outcome = match request {
        BlockRequestKind::ByRange(range) => serve_range(repository, current_slot, *range),
        BlockRequestKind::ByRoot(roots) => serve_roots(repository, roots),
    };

    match outcome {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(%error, "a block request could not be served");
            Response::Error {
                code: ErrorCode::ServerError,
                message: "the block store could not be read".to_string(),
            }
        }
    }
}

/// Answers a `BlocksByRange`, or refuses it in the protocol's own terms.
fn serve_range<B: StorageReader>(
    repository: &Repository<B>,
    current_slot: Slot,
    request: BlocksByRangeRequest,
) -> Result<Response, StorageError> {
    if request.count == 0 || request.count > MAX_REQUEST_BLOCKS as u64 {
        return Ok(Response::Error {
            code: ErrorCode::InvalidRequest,
            message: format!("count must be 1..={MAX_REQUEST_BLOCKS}"),
        });
    }

    // The one MUST: below the advertised floor a responder refuses rather than answering
    // short, so a peer can tell "I do not keep that history" from "there were no blocks".
    if !repository.can_serve_range(current_slot, request.start_slot)? {
        let floor = repository.range_service_floor(current_slot)?;
        return Ok(Response::Error {
            code: ErrorCode::ResourceUnavailable,
            message: format!("history below slot {} is not served", floor.0),
        });
    }

    let end = Slot(request.start_slot.0.saturating_add(request.count));
    let anchor = anchor_slot(repository)?;
    let mut chunks = Vec::new();
    for (slot, root) in repository.canonical_range(request.start_slot, end)? {
        if let Some(block) = stored_signed_block(repository, slot, root, anchor)? {
            chunks.push(block.to_ssz());
        }
    }
    Ok(Response::Blocks(chunks))
}

/// Answers a `BlocksByRoot`. Roots this node does not hold are skipped silently.
fn serve_roots<B: StorageReader>(
    repository: &Repository<B>,
    request: &BlocksByRootRequest,
) -> Result<Response, StorageError> {
    let anchor = anchor_slot(repository)?;
    let mut chunks = Vec::new();
    for root in request.roots.iter() {
        let Some(header) = repository.block_header(*root)? else {
            continue;
        };
        if let Some(block) = stored_signed_block(repository, header.slot, *root, anchor)? {
            chunks.push(block.to_ssz());
        }
    }
    Ok(Response::Blocks(chunks))
}

/// Reassembles a stored block and its proof into the chunk a response carries.
///
/// `None` means the block is not servable rather than that the database is damaged, and there
/// is exactly one such block: the anchor. A genesis or checkpoint anchor is adopted from
/// configuration rather than received over the wire, so no proof was ever stored with it, and
/// no peer needs one. Above the anchor a missing part *is* damage — the read is inside the
/// window this node advertises — and the error propagates into a `SERVER_ERROR` rather than
/// a hole in the response.
fn stored_signed_block<B: StorageReader>(
    repository: &Repository<B>,
    slot: Slot,
    root: Bytes32,
    anchor: Slot,
) -> Result<Option<SignedBlock>, StorageError> {
    let (Some(header), Some(body)) = (repository.block_header(root)?, repository.block_body(root)?)
    else {
        return incomplete(slot, root, anchor, "block");
    };
    let Some(proof) = repository.block_proof(slot, root)? else {
        return incomplete(slot, root, anchor, "proof");
    };

    Ok(Some(SignedBlock {
        block: block_from(&header, body),
        proof,
    }))
}

/// The first slot this node holds proof-bearing history for.
fn anchor_slot<B: StorageReader>(repository: &Repository<B>) -> Result<Slot, StorageError> {
    Ok(repository.served_from_slot()?.unwrap_or(Slot(0)))
}

/// Decides whether a missing part is the anchor's absence or the database's damage.
fn incomplete(
    slot: Slot,
    root: Bytes32,
    anchor: Slot,
    missing: &str,
) -> Result<Option<SignedBlock>, StorageError> {
    if slot.0 <= anchor.0 {
        return Ok(None);
    }
    Err(StorageError::Backend(format!(
        "canonical block at slot {} is missing its {missing} ({})",
        slot.0,
        root_prefix(root)
    )))
}
