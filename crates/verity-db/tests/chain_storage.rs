//! Storing a chain the state transition actually produced, and getting it back.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use common::{Link, anchor_root, chain, corrupt_parent, extend, identity_of, proof};
use verity_db::{
    AnchorCommit, BlockCommit, ColumnFamily, Durability, MemoryBackend, Repository, StorageBackend,
    StorageError, StorageReader, TickCommit, WriteBatch,
};
use verity_types::primitives::{Interval, Slot};
use verity_types::{AttestationData, Checkpoint, State, ValidatorIndex};

#[derive(Debug, Clone)]
struct FailingBackend {
    inner: MemoryBackend,
    fail_writes: Arc<AtomicBool>,
}

impl StorageReader for FailingBackend {
    fn get(&self, table: ColumnFamily, key: &[u8]) -> Result<Option<Vec<u8>>, StorageError> {
        self.inner.get(table, key)
    }

    fn table_is_empty(&self, table: ColumnFamily) -> Result<bool, StorageError> {
        self.inner.table_is_empty(table)
    }

    fn range(
        &self,
        table: ColumnFamily,
        start: &[u8],
        end: &[u8],
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, StorageError> {
        self.inner.range(table, start, end)
    }
}

impl StorageBackend for FailingBackend {
    fn write(&mut self, batch: WriteBatch, durability: Durability) -> Result<(), StorageError> {
        if self.fail_writes.load(Ordering::Relaxed) {
            return Err(StorageError::Backend("injected write failure".to_string()));
        }
        self.inner.write(batch, durability)
    }
}

/// An anchored repository holding `links`, in order.
fn stored(genesis: &State, links: &[Link]) -> Repository<MemoryBackend> {
    let mut repository = Repository::open(MemoryBackend::new(), identity_of(genesis))
        .expect("an empty database opens");
    repository
        .commit_anchor(&AnchorCommit {
            block_root: anchor_root(genesis),
            state: genesis,
            body: None,
            served_from_slot: Slot(0),
        })
        .expect("genesis anchors");

    for (index, link) in links.iter().enumerate() {
        repository
            .commit_block(&BlockCommit {
                block_root: link.root,
                body: &link.body,
                proof: &proof(u8::try_from(index).unwrap()),
                post_state: &link.post,
                parent_slot: link.parent_slot,
            })
            .expect("a block from the state transition commits");
    }
    repository
}

#[test]
fn should_anchor_genesis_as_head_and_as_the_canonical_block_at_its_slot() {
    let (genesis, _) = chain(4, &[]);
    let repository = stored(&genesis, &[]);
    let root = anchor_root(&genesis);

    assert_eq!(repository.head().unwrap(), Some(root));
    assert_eq!(repository.safe_target().unwrap(), Some(root));
    assert_eq!(repository.canonical_root(Slot(0)).unwrap(), Some(root));
    assert_eq!(
        repository.state_snapshot(root).unwrap().as_ref(),
        Some(&genesis)
    );
    assert_eq!(repository.served_from_slot().unwrap(), Some(Slot(0)));
}

#[test]
fn should_leave_every_commit_path_unchanged_when_the_backend_rejects_its_batch() {
    let (genesis, links) = chain(4, &[1]);
    let fail_writes = Arc::new(AtomicBool::new(false));
    let backend = FailingBackend {
        inner: MemoryBackend::new(),
        fail_writes: Arc::clone(&fail_writes),
    };
    let mut repository =
        Repository::open(backend, identity_of(&genesis)).expect("an empty database opens");
    repository
        .commit_anchor(&AnchorCommit {
            block_root: anchor_root(&genesis),
            state: &genesis,
            body: None,
            served_from_slot: Slot(0),
        })
        .expect("anchor succeeds before failures are enabled");

    let link = &links[0];
    fail_writes.store(true, Ordering::Relaxed);
    assert!(matches!(
        repository.commit_block(&BlockCommit {
            block_root: link.root,
            body: &link.body,
            proof: &proof(1),
            post_state: &link.post,
            parent_slot: link.parent_slot,
        }),
        Err(StorageError::Backend(_))
    ));
    assert!(repository.block_header(link.root).unwrap().is_none());

    fail_writes.store(false, Ordering::Relaxed);
    repository
        .commit_block(&BlockCommit {
            block_root: link.root,
            body: &link.body,
            proof: &proof(1),
            post_state: &link.post,
            parent_slot: link.parent_slot,
        })
        .expect("block setup");

    fail_writes.store(true, Ordering::Relaxed);
    let validator = ValidatorIndex(0);
    assert!(matches!(
        repository.record_pending_votes(&[(validator, AttestationData::default())], Interval(2),),
        Err(StorageError::Backend(_))
    ));
    assert_eq!(repository.pending_vote(validator).unwrap(), None);

    let checkpoint = Checkpoint {
        root: link.root,
        slot: link.slot(),
    };
    assert!(matches!(
        repository.commit_tick(&TickCommit {
            head: link.root,
            safe_target: Some(link.root),
            latest_justified: checkpoint,
            latest_finalized: checkpoint,
            interval: Interval(3),
            merge_pending_votes: false,
        }),
        Err(StorageError::Backend(_))
    ));
    assert_eq!(repository.head().unwrap(), Some(anchor_root(&genesis)));
    assert_eq!(
        repository.safe_target().unwrap(),
        Some(anchor_root(&genesis))
    );
    assert_eq!(
        repository.last_processed_interval().unwrap(),
        Some(Interval(0))
    );
}

#[test]
fn should_restore_the_complete_interval_three_view_after_reopening() {
    let (genesis, links) = chain(4, &[1]);
    let mut repository = stored(&genesis, &links);
    let checkpoint = Checkpoint {
        root: links[0].root,
        slot: links[0].slot(),
    };
    repository
        .commit_tick(&TickCommit {
            head: links[0].root,
            safe_target: Some(links[0].root),
            latest_justified: checkpoint,
            latest_finalized: checkpoint,
            interval: Interval(3),
            merge_pending_votes: false,
        })
        .expect("interval three commits atomically");

    let reopened = Repository::open(repository.backend().clone(), identity_of(&genesis))
        .expect("repository reopens");
    assert_eq!(reopened.head().unwrap(), Some(links[0].root));
    assert_eq!(reopened.safe_target().unwrap(), Some(links[0].root));
    assert_eq!(reopened.latest_justified().unwrap(), Some(checkpoint));
    assert_eq!(reopened.latest_finalized().unwrap(), Some(checkpoint));
    assert_eq!(
        reopened.last_processed_interval().unwrap(),
        Some(Interval(3))
    );
    assert_eq!(
        reopened.canonical_root(links[0].slot()).unwrap(),
        Some(links[0].root)
    );
}

#[test]
fn should_rebuild_every_post_state_from_its_snapshot_and_diffs() {
    let (genesis, links) = chain(4, &[1, 2, 3, 5, 8]);
    let repository = stored(&genesis, &links);

    for link in &links {
        assert_eq!(
            repository.state_at(link.root).unwrap(),
            link.post,
            "the state at slot {} did not survive the round trip",
            link.slot().0
        );
    }
}

#[test]
fn should_store_a_diff_and_no_snapshot_inside_one_snapshot_interval() {
    let (genesis, links) = chain(4, &[1, 2]);
    let repository = stored(&genesis, &links);

    for link in &links {
        assert!(repository.state_diff(link.root).unwrap().is_some());
        assert!(
            repository.state_snapshot(link.root).unwrap().is_none(),
            "no edge here crosses a 1,024-slot boundary"
        );
    }
}

#[test]
fn should_store_a_snapshot_when_a_block_edge_crosses_the_interval_boundary() {
    let (genesis, links) = chain(4, &[1_000, 1_100]);
    let repository = stored(&genesis, &links);

    assert!(
        repository.state_snapshot(links[0].root).unwrap().is_none(),
        "slot 1,000 is inside the first interval"
    );
    assert_eq!(
        repository.state_snapshot(links[1].root).unwrap().as_ref(),
        Some(&links[1].post),
        "the 1,000 -> 1,100 edge crosses 1,024"
    );
    // The snapshot must still be the state the transition produced, not merely present.
    assert_eq!(repository.state_at(links[1].root).unwrap(), links[1].post);
}

#[test]
fn should_index_a_block_by_its_state_root_and_by_its_parent() {
    let (genesis, links) = chain(4, &[1, 2]);
    let repository = stored(&genesis, &links);
    let child = &links[1];

    let header = repository.block_header(child.root).unwrap().unwrap();
    assert_eq!(
        repository
            .block_root_for_state_root(header.state_root)
            .unwrap(),
        Some(child.root)
    );
    assert_eq!(
        repository
            .fork_choice_parent(child.slot(), child.root)
            .unwrap(),
        Some(links[0].root)
    );
}

#[test]
fn should_keep_the_proof_of_every_stored_block() {
    let (genesis, links) = chain(4, &[1, 2]);
    let repository = stored(&genesis, &links);

    for (index, link) in links.iter().enumerate() {
        assert_eq!(
            repository.block_proof(link.slot(), link.root).unwrap(),
            Some(proof(u8::try_from(index).unwrap()))
        );
    }
}

#[test]
fn should_report_missing_data_rather_than_inventing_a_state() {
    let (genesis, _) = chain(4, &[]);
    let repository = stored(&genesis, &[]);
    assert!(matches!(
        repository.state_at([7u8; 32]).unwrap_err(),
        StorageError::MissingRow { .. }
    ));
}

#[test]
fn should_refuse_a_block_committed_under_the_wrong_root() {
    let (genesis, links) = chain(4, &[1]);
    let mut repository = stored(&genesis, &[]);

    let error = repository
        .commit_block(&BlockCommit {
            block_root: [1u8; 32],
            body: &links[0].body,
            proof: &proof(0),
            post_state: &links[0].post,
            parent_slot: links[0].parent_slot,
        })
        .unwrap_err();
    assert!(matches!(error, StorageError::RejectedBatch(_)));
    assert!(
        repository.block_header([1u8; 32]).unwrap().is_none(),
        "a refused batch writes nothing at all"
    );
}

#[test]
fn should_refuse_an_anchor_whose_root_does_not_follow_from_its_state() {
    let (genesis, _) = chain(4, &[]);
    let mut repository = Repository::open(MemoryBackend::new(), identity_of(&genesis)).unwrap();

    let error = repository
        .commit_anchor(&AnchorCommit {
            block_root: [3u8; 32],
            state: &genesis,
            body: None,
            served_from_slot: Slot(0),
        })
        .unwrap_err();
    assert!(matches!(error, StorageError::RejectedBatch(_)));
    assert!(!repository.is_populated().unwrap());
}

#[test]
fn should_move_the_canonical_index_to_the_new_branch_on_a_reorg() {
    // Two children of one parent, at different slots: switching between them exercises both
    // halves of the walk to the common ancestor.
    let (genesis, base) = chain(4, &[1]);
    let short = extend(&base[0].post, 2);
    let long = extend(&base[0].post, 3);

    let mut repository = stored(&genesis, &[base[0].clone(), short.clone()]);
    repository
        .commit_block(&BlockCommit {
            block_root: long.root,
            body: &long.body,
            proof: &proof(9),
            post_state: &long.post,
            parent_slot: long.parent_slot,
        })
        .unwrap();

    let tick = |head, interval| TickCommit {
        head,
        safe_target: None,
        latest_justified: Checkpoint::default(),
        latest_finalized: Checkpoint::default(),
        interval: Interval(interval),
        merge_pending_votes: false,
    };

    repository.commit_tick(&tick(short.root, 9)).unwrap();
    assert_eq!(
        repository.canonical_root(Slot(2)).unwrap(),
        Some(short.root)
    );

    repository.commit_tick(&tick(long.root, 14)).unwrap();
    assert_eq!(
        repository.canonical_root(Slot(2)).unwrap(),
        None,
        "slot 2 left the canonical chain"
    );
    assert_eq!(repository.canonical_root(Slot(3)).unwrap(), Some(long.root));
    assert_eq!(
        repository.canonical_root(Slot(1)).unwrap(),
        Some(base[0].root),
        "the common ancestor is untouched"
    );
    assert_eq!(repository.head().unwrap(), Some(long.root));
}

#[test]
fn should_reject_a_non_decreasing_parent_during_a_canonical_switch() {
    let (genesis, base) = chain(4, &[1]);
    let head = extend(&base[0].post, 2);
    let impossible_parent = extend(&base[0].post, 3);
    let mut repository = stored(
        &genesis,
        &[base[0].clone(), head.clone(), impossible_parent.clone()],
    );
    repository
        .commit_tick(&TickCommit {
            head: head.root,
            safe_target: None,
            latest_justified: Checkpoint::default(),
            latest_finalized: Checkpoint::default(),
            interval: Interval(14),
            merge_pending_votes: false,
        })
        .unwrap();

    let mut repository = corrupt_parent(&repository, head.root, impossible_parent.root);
    let error = repository
        .commit_tick(&TickCommit {
            head: anchor_root(&genesis),
            safe_target: None,
            latest_justified: Checkpoint::default(),
            latest_finalized: Checkpoint::default(),
            interval: Interval(15),
            merge_pending_votes: false,
        })
        .unwrap_err();

    assert_eq!(
        error,
        StorageError::NonDecreasingAncestry {
            child: head.root,
            child_slot: Slot(2),
            parent: impossible_parent.root,
            parent_slot: Slot(3),
        }
    );
}

#[test]
fn should_serve_a_slot_range_as_the_canonical_roots_it_actually_holds() {
    let (genesis, links) = chain(4, &[1, 3]);
    let mut repository = stored(&genesis, &links);
    repository
        .commit_tick(&TickCommit {
            head: links[1].root,
            safe_target: None,
            latest_justified: Checkpoint::default(),
            latest_finalized: Checkpoint::default(),
            interval: Interval(19),
            merge_pending_votes: false,
        })
        .unwrap();

    assert_eq!(
        repository.canonical_range(Slot(0), Slot(4)).unwrap(),
        vec![
            (Slot(0), anchor_root(&genesis)),
            (Slot(1), links[0].root),
            (Slot(3), links[1].root),
        ],
        "slot 2 is empty, so it is absent rather than zero-filled"
    );
}
