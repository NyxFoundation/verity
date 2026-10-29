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
    BlocksByRangeRequest, BlocksByRootRequest, CommandError, ErrorCode, MAX_REQUEST_BLOCKS,
    NetworkHandle, PeerId, Response, ResponseChannel,
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

            let block_count = match &response {
                Response::Blocks(chunks) => chunks.len(),
                // A refusal is a served request that carried no blocks.
                _ => 0,
            };
            match self.handle.respond(channel, response).await {
                Ok(()) => {
                    self.counters.record_served(block_count);
                    tracing::trace!(%peer, "served a block request");
                }
                Err(CommandError::Failed(error)) => {
                    tracing::debug!(%peer, %error, "block response was rejected");
                }
                Err(CommandError::ServiceStopped) => break,
            }
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

/// Whether a range count is inside the protocol's non-empty response bound.
const fn range_count_is_valid(count: u64) -> bool {
    count > 0 && count <= MAX_REQUEST_BLOCKS as u64
}

/// Answers a `BlocksByRange`, or refuses it in the protocol's own terms.
fn serve_range<B: StorageReader>(
    repository: &Repository<B>,
    current_slot: Slot,
    request: BlocksByRangeRequest,
) -> Result<Response, StorageError> {
    if !range_count_is_valid(request.count) {
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

/// One stored block with its proof, for the HTTP API's finalized-block route.
///
/// The same read the range responder does per block, keyed by root alone: the header is
/// looked up first to learn the slot the proof is filed under. `None` means the root is not
/// stored, or names the anchor — which was adopted from configuration and has no proof.
///
/// # Errors
///
/// [`StorageError`] when a block above the anchor is missing a part, which is damage.
pub fn signed_block<B: StorageReader>(
    repository: &Repository<B>,
    root: Bytes32,
) -> Result<Option<SignedBlock>, StorageError> {
    let Some(header) = repository.block_header(root)? else {
        return Ok(None);
    };
    stored_signed_block(repository, header.slot, root, anchor_slot(repository)?)
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

#[cfg(test)]
mod tests {
    use libssz::SszDecode;
    use verity_chain::{generate_genesis, hash_tree_root, process_block, process_slots};
    use verity_db::{
        AnchorCommit, BlockCommit, ColumnFamily, Identity, MemoryBackend, Repository, StorageError,
        StorageReader, TickCommit, stored_header,
    };
    use verity_p2p::{
        BlocksByRangeRequest, BlocksByRootRequest, ErrorCode, MAX_REQUEST_BLOCKS,
        RequestedBlockRoots, Response,
    };
    use verity_types::{
        Block, BlockBody, Checkpoint, Interval, MultiMessageAggregate, SignedBlock, Slot,
        Validator, ValidatorIndex, Validators,
    };

    use super::{BlockRequestKind, answer, signed_block};

    struct StoredChain {
        repository: Repository<MemoryBackend>,
        anchor_root: [u8; 32],
        roots: Vec<[u8; 32]>,
    }

    fn stored_chain() -> StoredChain {
        let validators = Validators::try_from(vec![Validator {
            attestation_public_key: [1; 52],
            proposal_public_key: [2; 52],
            index: ValidatorIndex(0),
        }])
        .expect("one validator");
        let genesis = generate_genesis(0, validators);
        let anchor_body = BlockBody::default();
        let anchor_root = hash_tree_root(&stored_header(&genesis));
        let mut repository = Repository::open(
            MemoryBackend::new(),
            Identity {
                chain_fingerprint: hash_tree_root(&genesis),
                fork_version: 1,
            },
        )
        .expect("empty repository");
        repository
            .commit_anchor(&AnchorCommit {
                block_root: anchor_root,
                state: &genesis,
                body: Some(&anchor_body),
                served_from_slot: Slot(0),
            })
            .expect("anchor");

        let mut parent = genesis;
        let mut roots = Vec::new();
        for slot in [1, 3] {
            let advanced = process_slots(&parent, Slot(slot)).expect("future slot");
            let block = Block {
                slot: Slot(slot),
                proposer_index: ValidatorIndex(0),
                parent_root: hash_tree_root(&advanced.latest_block_header),
                state_root: [0; 32],
                body: BlockBody::default(),
            };
            let post = process_block(&advanced, &block).expect("valid empty block");
            let root = hash_tree_root(&stored_header(&post));
            repository
                .commit_block(&BlockCommit {
                    block_root: root,
                    body: &block.body,
                    proof: &MultiMessageAggregate::default(),
                    post_state: &post,
                    parent_slot: parent.slot,
                })
                .expect("block");
            let checkpoint = Checkpoint {
                root,
                slot: Slot(slot),
            };
            repository
                .commit_tick(&TickCommit {
                    head: root,
                    safe_target: None,
                    latest_justified: checkpoint,
                    latest_finalized: Checkpoint::default(),
                    interval: Interval(slot * 5),
                    merge_pending_votes: false,
                })
                .expect("canonical index");
            parent = post;
            roots.push(root);
        }
        StoredChain {
            repository,
            anchor_root,
            roots,
        }
    }

    fn blocks(response: Response) -> Vec<Vec<u8>> {
        match response {
            Response::Blocks(chunks) => chunks,
            other => panic!("expected blocks, got {other:?}"),
        }
    }

    #[test]
    fn should_serve_only_canonical_blocks_present_in_a_valid_range() {
        let chain = stored_chain();
        let response = answer(
            &chain.repository,
            Slot(3),
            &BlockRequestKind::ByRange(BlocksByRangeRequest {
                start_slot: Slot(1),
                count: 3,
            }),
        );
        let decoded: Vec<_> = blocks(response)
            .iter()
            .map(|chunk| {
                SignedBlock::from_ssz_bytes(chunk)
                    .expect("signed block")
                    .block
                    .slot
            })
            .collect();
        assert_eq!(decoded, vec![Slot(1), Slot(3)]);
    }

    #[test]
    fn should_reject_empty_and_oversized_range_counts() {
        let chain = stored_chain();
        for count in [0, MAX_REQUEST_BLOCKS as u64 + 1] {
            let response = answer(
                &chain.repository,
                Slot(3),
                &BlockRequestKind::ByRange(BlocksByRangeRequest {
                    start_slot: Slot(1),
                    count,
                }),
            );
            assert!(matches!(
                response,
                Response::Error {
                    code: ErrorCode::InvalidRequest,
                    ..
                }
            ));
        }
    }

    #[test]
    fn should_skip_unknown_roots_and_the_proofless_anchor() {
        let chain = stored_chain();
        let roots = RequestedBlockRoots::try_from(vec![chain.anchor_root, [9; 32], chain.roots[0]])
            .expect("three roots");
        let chunks = blocks(answer(
            &chain.repository,
            Slot(3),
            &BlockRequestKind::ByRoot(BlocksByRootRequest { roots }),
        ));
        assert_eq!(chunks.len(), 1);
        let block = SignedBlock::from_ssz_bytes(&chunks[0]).expect("signed block");
        assert_eq!(hash_tree_root(&block.block), chain.roots[0]);
    }

    #[test]
    fn should_return_none_for_an_unknown_or_proofless_anchor_block() {
        let chain = stored_chain();
        assert!(signed_block(&chain.repository, [8; 32]).unwrap().is_none());
        assert!(
            signed_block(&chain.repository, chain.anchor_root)
                .unwrap()
                .is_none()
        );
        assert!(
            signed_block(&chain.repository, chain.roots[1])
                .unwrap()
                .is_some()
        );
    }

    #[derive(Debug)]
    struct FailingReader;

    impl StorageReader for FailingReader {
        fn get(&self, _table: ColumnFamily, _key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
            Err(StorageError::Backend("injected read failure".to_string()))
        }

        fn table_is_empty(&self, _table: ColumnFamily) -> Result<bool, StorageError> {
            Ok(true)
        }

        fn range(
            &self,
            _table: ColumnFamily,
            _start: &[u8],
            _end: &[u8],
        ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
            Err(StorageError::Backend("injected range failure".to_string()))
        }
    }

    #[test]
    fn should_translate_storage_failures_to_server_error() {
        let repository = Repository::open(
            FailingReader,
            Identity {
                chain_fingerprint: [0; 32],
                fork_version: 1,
            },
        )
        .expect("the first identity probe sees an empty repository");
        let roots = RequestedBlockRoots::try_from(vec![[1; 32]]).expect("one root");

        assert!(matches!(
            answer(
                &repository,
                Slot(1),
                &BlockRequestKind::ByRoot(BlocksByRootRequest { roots })
            ),
            Response::Error {
                code: ErrorCode::ServerError,
                ..
            }
        ));
    }
}

#[cfg(kani)]
mod harnesses {
    use super::{MAX_REQUEST_BLOCKS, range_count_is_valid};

    /// A range responder admits exactly the non-empty request counts allowed by the protocol.
    // Lean overlap: request-bound portion of NET-1. Future Lean-adoption deletion candidate.
    #[kani::proof]
    fn range_response_counts_are_admitted_exactly_inside_the_bound() {
        let count: u64 = kani::any();
        assert!(range_count_is_valid(count) == (count > 0 && count <= MAX_REQUEST_BLOCKS as u64));
    }
}
