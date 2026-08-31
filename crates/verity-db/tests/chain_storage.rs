//! Storing a chain the state transition actually produced, and getting it back.

mod common;

use common::{Link, anchor_root, chain, extend, identity_of, proof};
use verity_db::{AnchorCommit, BlockCommit, MemoryBackend, Repository, StorageError, TickCommit};
use verity_types::primitives::{Interval, Slot};
use verity_types::{Checkpoint, State};

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
fn should_serve_a_slot_range_as_the_canonical_roots_it_actually_holds() {
    let (genesis, links) = chain(4, &[1, 3]);
    let mut repository = stored(&genesis, &links);
    repository
        .commit_tick(&TickCommit {
            head: links[1].root,
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
