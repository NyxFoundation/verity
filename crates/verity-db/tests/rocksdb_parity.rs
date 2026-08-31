//! The RocksDB backend must behave as the in-memory one does.
//!
//! `docs/design/storage.md` allows no RocksDB-specific behavior past the backend contract.
//! The value of the in-memory sibling rests entirely on that, so it is checked rather than
//! assumed: the same commits run against both, and the results are compared.

mod common;

use common::{anchor_root, chain, identity_of, proof};
use tempfile::tempdir;
use verity_db::backend::StorageBackend;
use verity_db::{AnchorCommit, BlockCommit, MemoryBackend, Repository, RocksBackend, TickCommit};
use verity_types::Checkpoint;
use verity_types::primitives::{Interval, Slot};

/// Runs one chain through a repository and reports what a reader would see.
fn run<B: StorageBackend>(backend: B) -> Vec<(Slot, [u8; 32])> {
    let (genesis, links) = chain(4, &[1, 2, 4]);
    let mut repository = Repository::open(backend, identity_of(&genesis)).unwrap();
    repository
        .commit_anchor(&AnchorCommit {
            block_root: anchor_root(&genesis),
            state: &genesis,
            body: None,
            served_from_slot: Slot(0),
        })
        .unwrap();
    for (index, link) in links.iter().enumerate() {
        repository
            .commit_block(&BlockCommit {
                block_root: link.root,
                body: &link.body,
                proof: &proof(u8::try_from(index).unwrap()),
                post_state: &link.post,
                parent_slot: link.parent_slot,
            })
            .unwrap();
    }
    repository
        .commit_tick(&TickCommit {
            head: links[2].root,
            latest_justified: Checkpoint::default(),
            latest_finalized: Checkpoint::default(),
            interval: Interval(24),
            merge_pending_votes: false,
        })
        .unwrap();

    // Reconstruction is the strongest single check available: it reads headers, diffs, and a
    // snapshot, and fails unless every one of them came back byte-identical.
    for link in &links {
        assert_eq!(repository.state_at(link.root).unwrap(), link.post);
    }
    repository.canonical_range(Slot(0), Slot(8)).unwrap()
}

#[test]
fn should_produce_the_same_canonical_chain_on_both_backends() {
    let directory = tempdir().expect("a writable temporary directory");
    let rocks = RocksBackend::open(directory.path()).expect("rocksdb opens a fresh directory");
    assert_eq!(run(rocks), run(MemoryBackend::new()));
}

#[test]
fn should_reopen_a_rocksdb_directory_it_wrote() {
    let directory = tempdir().expect("a writable temporary directory");
    let (genesis, _) = chain(4, &[]);
    let identity = identity_of(&genesis);
    let root = anchor_root(&genesis);

    {
        let backend = RocksBackend::open(directory.path()).unwrap();
        let mut repository = Repository::open(backend, identity).unwrap();
        repository
            .commit_anchor(&AnchorCommit {
                block_root: root,
                state: &genesis,
                body: None,
                served_from_slot: Slot(0),
            })
            .unwrap();
    }

    let reopened = Repository::open(RocksBackend::open(directory.path()).unwrap(), identity)
        .expect("its own directory reopens");
    assert_eq!(reopened.head().unwrap(), Some(root));
    assert_eq!(reopened.state_at(root).unwrap(), genesis);
}

#[test]
fn should_refuse_a_second_handle_on_a_directory_already_open() {
    // The single-writer rule is carried by ownership inside a process. This is the other half
    // of it: two processes cannot interleave writes into one directory either, because the
    // engine will not hand out a second handle. The claim is in `lib.rs`, so it is checked
    // here rather than trusted.
    let directory = tempdir().expect("a writable temporary directory");
    let _held = RocksBackend::open(directory.path()).expect("the first handle opens");

    assert!(
        RocksBackend::open(directory.path()).is_err(),
        "a directory under an open handle must not open a second time"
    );
}
