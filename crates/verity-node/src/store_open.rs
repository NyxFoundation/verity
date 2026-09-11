//! Getting from a directory on disk to a fork-choice store.
//!
//! Three paths, and `docs/design/sync.md`'s Decision 1 makes them mutually exclusive: a
//! fetched checkpoint anchor is used when the operator asked for one, a directory carrying
//! identity values is a restart and gets its chain read back, and an empty directory is a new
//! node anchored on genesis. A directory that disagrees with the configured chain stops the
//! node — it belongs to another chain, fork, or schema, and nothing local can tell which one
//! the operator meant (`docs/design/storage.md`).
//!
//! The checkpoint path has no fallbacks by design. A populated directory is not silently
//! resumed and genesis is not silently substituted: both would start the node from an anchor
//! the operator did not ask for, and choosing a different anchor is an operator's decision.
//!
//! # What a restart rebuilds
//!
//! The store is re-anchored on the block the *stored finalized checkpoint* names — nothing
//! below it can be reorganized — and the canonical blocks above it are replayed through
//! `on_block`. The checkpoint is resolved by slot rather than by root, because a genesis state
//! finalizes the zero placeholder rather than a block. Their
//! proofs are not re-checked: they were verified when they were first imported, and a block
//! in this node's own database is not gossip.
//!
//! Non-canonical branches are not rebuilt. They are peer-recoverable, and the head is
//! canonical by definition, so a restart resumes on the chain it was following.

use verity_chain::{Store, hash_tree_root, intervals_at_slot_start, on_block};
use verity_db::{AnchorCommit, Identity, Repository, StorageBackend, StorageReader, stored_header};
use verity_types::primitives::ZERO_HASH;
use verity_types::{Block, BlockBody, BlockHeader, Bytes32, Slot, State, ValidatorIndex};

use crate::error::NodeError;
use crate::sync::checkpoint::CheckpointAnchor;

/// The protocol fork this build speaks.
///
/// leanSpec defines no fork versioning yet — there is one fork, `lstar` — so this is a
/// placeholder in exactly one sense: it is the value that goes into the database's identity,
/// where its job is to stop a node from opening a directory written by a different protocol.
/// It changes when leanSpec grows a second fork, not before.
pub const FORK_VERSION: u64 = 0;

/// Opens the repository for a chain and produces the store to run it from.
///
/// `checkpoint` is the anchor a `--checkpoint-sync-url` start fetched, and its presence is
/// what selects the checkpoint path.
///
/// # Errors
///
/// [`NodeError::Storage`] when the directory belongs to another chain or cannot be written,
/// [`NodeError::Restore`] when the stored chain does not rebuild into a valid store,
/// [`NodeError::IncompleteChain`] when a block the canonical index names is not fully stored,
/// and [`NodeError::CheckpointOverPopulated`] when a checkpoint start is asked for on a
/// directory that already holds a chain.
pub fn open<B: StorageBackend>(
    backend: B,
    genesis_state: &State,
    checkpoint: Option<&CheckpointAnchor>,
    validator_index: Option<ValidatorIndex>,
) -> Result<(Repository<B>, Store), NodeError> {
    let mut repository = Repository::open(backend, identity_for(genesis_state))?;

    if let Some(anchor) = checkpoint {
        if repository.is_populated()? {
            return Err(NodeError::CheckpointOverPopulated {
                slot: anchor.state.slot,
            });
        }
        let store = anchor_on_checkpoint(&mut repository, anchor, validator_index)?;
        tracing::info!(
            anchor = %root_prefix(store.head),
            slot = anchor.state.slot.0,
            "anchored a new database on a fetched checkpoint"
        );
        return Ok((repository, store));
    }

    if repository.is_populated()? {
        let store = restore(&repository, validator_index)?;
        tracing::info!(
            head = %root_prefix(store.head),
            slot = store.current_slot().0,
            finalized = store.latest_finalized.slot.0,
            "resumed from the stored chain"
        );
        Ok((repository, store))
    } else {
        let store = anchor_on_genesis(&mut repository, genesis_state, validator_index)?;
        tracing::info!(
            genesis = %root_prefix(store.head),
            validators = genesis_state.validators.len(),
            "anchored a new database on genesis"
        );
        Ok((repository, store))
    }
}

/// Opens a second, read-only view of a database the writer has already opened.
///
/// The identity is derived exactly as [`open`] derives it, so a reader is subject to the same
/// refusal a writer is: a directory belonging to another chain does not open, whichever
/// handle asks. Nothing is written here — the type says so.
///
/// # Errors
///
/// [`NodeError::Storage`] when the directory belongs to another chain.
pub fn open_reader<B: StorageReader>(
    backend: B,
    genesis_state: &State,
) -> Result<Repository<B>, NodeError> {
    Ok(Repository::open(backend, identity_for(genesis_state))?)
}

/// The identity a directory must agree with to be this chain's.
///
/// Computed in one place because the writer and the reader have to derive it identically:
/// a reader that computed it differently would open a directory the writer would refuse.
fn identity_for(genesis_state: &State) -> Identity {
    Identity {
        chain_fingerprint: hash_tree_root(genesis_state),
        fork_version: FORK_VERSION,
    }
}

/// Writes a fetched checkpoint as the anchor and starts a store on it.
///
/// The anchor's body is stored alongside it, so this node can serve the anchor block itself
/// over `BlocksByRoot`. Its proof is not: no proof was fetched, none is needed to run, and
/// `served_from_slot` is what tells a peer that history below here is not on offer.
fn anchor_on_checkpoint<B: StorageBackend>(
    repository: &mut Repository<B>,
    anchor: &CheckpointAnchor,
    validator_index: Option<ValidatorIndex>,
) -> Result<Store, NodeError> {
    let block_root = hash_tree_root(&anchor.block.block);

    repository.commit_anchor(&AnchorCommit {
        block_root,
        state: &anchor.state,
        body: Some(&anchor.block.block.body),
        // Nothing below the anchor was ever fetched, so range service starts here rather
        // than at slot zero: advertising lower would promise history this node cannot serve.
        served_from_slot: anchor.state.slot,
    })?;

    Ok(Store::new(
        &anchor.state,
        &anchor.block.block,
        validator_index,
    )?)
}

/// Writes the genesis anchor and starts a store on it.
fn anchor_on_genesis<B: StorageBackend>(
    repository: &mut Repository<B>,
    genesis_state: &State,
    validator_index: Option<ValidatorIndex>,
) -> Result<Store, NodeError> {
    let body = BlockBody::default();
    let anchor = block_from(&stored_header(genesis_state), body.clone());
    let block_root = hash_tree_root(&anchor);

    repository.commit_anchor(&AnchorCommit {
        block_root,
        state: genesis_state,
        body: Some(&body),
        // A chain starting at genesis has history from slot zero, so there is nothing below
        // the anchor a peer could ask for and be refused.
        served_from_slot: Slot(0),
    })?;

    Ok(Store::new(genesis_state, &anchor, validator_index)?)
}

/// Rebuilds the store from the stored finalized checkpoint and the canonical chain above it.
fn restore<B: StorageReader>(
    repository: &Repository<B>,
    validator_index: Option<ValidatorIndex>,
) -> Result<Store, NodeError> {
    let finalized = repository
        .latest_finalized()?
        .ok_or(NodeError::IncompleteChain { root: ZERO_HASH })?;

    // The checkpoint's *slot* names the anchor, not its root. A genesis state finalizes the
    // zero placeholder — leanSpec's genesis carries no real finalized root — so resolving the
    // block through the canonical index is what works on a fresh chain and a running one
    // alike.
    let anchor_root =
        repository
            .canonical_root(finalized.slot)?
            .ok_or(NodeError::IncompleteChain {
                root: finalized.root,
            })?;

    let anchor_state = repository.state_at(anchor_root)?;
    let anchor = stored_block(repository, anchor_root)?;
    let mut store = Store::new(&anchor_state, &anchor, validator_index)?;

    if let Some(head) = repository.head()? {
        replay_canonical(repository, &mut store, finalized.slot, head)?;
    }

    // The safe target is not derivable from blocks — it is what interval 3 concluded — so it
    // is read back rather than recomputed. Absent means no interval 3 has completed yet.
    if let Some(safe_target) = repository.safe_target()? {
        store.safe_target = safe_target;
    }

    // Time moves forward to the last interval whose batch completed. The intervals below it
    // already ran and were committed; re-running them would redo work, not correct it.
    if let Some(interval) = repository.last_processed_interval()?
        && interval.0 > store.time.0
    {
        store.time = interval;
    }

    Ok(store)
}

/// Replays the canonical blocks between the anchor and the stored head.
fn replay_canonical<B: StorageReader>(
    repository: &Repository<B>,
    store: &mut Store,
    anchor_slot: Slot,
    head: Bytes32,
) -> Result<(), NodeError> {
    let Some(head_header) = repository.block_header(head)? else {
        return Err(NodeError::IncompleteChain { root: head });
    };

    for (_, root) in
        repository.canonical_range(Slot(anchor_slot.0 + 1), Slot(head_header.slot.0 + 1))?
    {
        let block = stored_block(repository, root)?;
        // The store admits nothing beyond its own clock, so time follows the replay rather
        // than the replay waiting for time.
        store.time = intervals_at_slot_start(block.slot);
        on_block(store, &block)?;
    }
    Ok(())
}

/// Reads a block back out of the two tables it is split across.
fn stored_block<B: StorageReader>(
    repository: &Repository<B>,
    root: Bytes32,
) -> Result<Block, NodeError> {
    let (Some(header), Some(body)) = (repository.block_header(root)?, repository.block_body(root)?)
    else {
        return Err(NodeError::IncompleteChain { root });
    };
    Ok(block_from(&header, body))
}

/// The block a stored header and body describe.
///
/// Every header field is a block field, and a container's root is unchanged by carrying the
/// body whole instead of its root — which is why the two forms are interchangeable and why
/// the writer stores only the header. Shared with the range-sync responder, which reassembles
/// the same two rows into the chunks it serves.
pub(crate) fn block_from(header: &BlockHeader, body: BlockBody) -> Block {
    Block {
        slot: header.slot,
        proposer_index: header.proposer_index,
        parent_root: header.parent_root,
        state_root: header.state_root,
        body,
    }
}

/// The first four bytes of a root, hex, for a log line or an error message.
pub(crate) fn root_prefix(root: Bytes32) -> String {
    root.iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use verity_chain::{generate_genesis, hash_tree_root};
    use verity_db::{MemoryBackend, stored_header};
    use verity_types::{
        BlockBody, MultiMessageAggregate, SignedBlock, Slot, Validator, ValidatorIndex, Validators,
    };

    use crate::error::NodeError;
    use crate::sync::checkpoint::CheckpointAnchor;

    use super::{block_from, open};

    fn genesis(count: u64) -> verity_types::State {
        let mut validators = Validators::default();
        for index in 0..count {
            let seed = index as u8;
            validators
                .push(Validator {
                    attestation_public_key: [seed; 52],
                    proposal_public_key: [seed.wrapping_add(128); 52],
                    index: ValidatorIndex(index),
                })
                .expect("under the registry limit");
        }
        generate_genesis(0, validators)
    }

    #[test]
    fn should_anchor_an_empty_database_on_genesis() {
        let state = genesis(4);
        let (repository, store) =
            open(MemoryBackend::default(), &state, None, None).expect("a new node");

        assert_eq!(store.current_slot(), Slot(0));
        assert_eq!(store.latest_finalized.slot, Slot(0));
        assert_eq!(store.head, store.latest_finalized.root);
        assert!(repository.is_populated().expect("a populated database"));
    }

    #[test]
    fn should_refuse_a_database_belonging_to_another_chain() {
        let state = genesis(4);
        let (repository, _) =
            open(MemoryBackend::default(), &state, None, None).expect("a new node");
        let backend = repository.backend().clone();

        let other = genesis(5);
        assert_ne!(hash_tree_root(&other), hash_tree_root(&state));
        assert!(open(backend, &other, None, None).is_err());
    }

    /// An anchor pair shaped the way a fetched one is: the block the state's header names.
    fn checkpoint(base: &verity_types::State, slot: u64) -> CheckpointAnchor {
        let mut state = base.clone();
        state.slot = Slot(slot);
        state.latest_block_header.slot = Slot(slot);
        CheckpointAnchor {
            block: SignedBlock {
                block: block_from(&stored_header(&state), BlockBody::default()),
                proof: MultiMessageAggregate::default(),
            },
            state,
        }
    }

    #[test]
    fn should_anchor_an_empty_database_on_a_fetched_checkpoint() {
        let state = genesis(4);
        let anchor = checkpoint(&state, 512);
        let (repository, store) = open(MemoryBackend::default(), &state, Some(&anchor), None)
            .expect("a checkpoint start");

        assert_eq!(store.latest_finalized.slot, Slot(512));
        // Range service starts at the anchor: nothing below it was ever fetched.
        assert_eq!(
            repository.served_from_slot().expect("metadata"),
            Some(Slot(512))
        );
    }

    #[test]
    fn should_refuse_a_checkpoint_start_on_a_database_that_already_holds_a_chain() {
        let state = genesis(4);
        let (repository, _) =
            open(MemoryBackend::default(), &state, None, None).expect("a new node");
        let backend = repository.backend().clone();

        // Neither anchor may be picked silently: resuming would ignore the operator's flag,
        // and anchoring would leave a hole below the checkpoint.
        let anchor = checkpoint(&state, 512);
        assert!(matches!(
            open(backend, &state, Some(&anchor), None),
            Err(NodeError::CheckpointOverPopulated { slot: Slot(512) })
        ));
    }

    #[test]
    fn should_resume_from_its_own_anchor_when_reopened() {
        let state = genesis(4);
        let (repository, first) =
            open(MemoryBackend::default(), &state, None, None).expect("a new node");
        let backend = repository.backend().clone();

        let (_, second) = open(backend, &state, None, None).expect("a restart");
        assert_eq!(second.head, first.head);
        assert_eq!(second.latest_finalized, first.latest_finalized);
    }
}
