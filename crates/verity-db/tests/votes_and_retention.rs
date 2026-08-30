//! The two things the repository reduces, and the two things it deletes.

mod common;

use common::{anchor_root, chain, identity_of, proof};
use verity_db::{
    AnchorCommit, BlockCommit, MemoryBackend, PROOF_RETENTION_SLOTS, Repository, TickCommit,
};
use verity_types::primitives::{Interval, Slot, ValidatorIndex};
use verity_types::{AttestationData, Checkpoint};

/// A vote naming `head` at `slot`.
fn vote(slot: u64, head: Checkpoint) -> AttestationData {
    AttestationData {
        slot: Slot(slot),
        head,
        ..AttestationData::default()
    }
}

/// A repository holding a chain at the given slots, with a head at the last of them.
fn with_chain(slots: &[u64]) -> (Repository<MemoryBackend>, Vec<common::Link>) {
    let (genesis, links) = chain(4, slots);
    let mut repository = Repository::open(MemoryBackend::new(), identity_of(&genesis)).unwrap();
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
    (repository, links)
}

#[test]
fn should_keep_only_the_newest_vote_per_validator() {
    let (mut repository, _) = with_chain(&[1]);
    let validator = ValidatorIndex(2);
    let old = vote(4, Checkpoint::default());
    let new = vote(5, Checkpoint::default());

    repository
        .record_pending_votes(&[(validator, new)], Interval(1))
        .unwrap();
    repository
        .record_pending_votes(&[(validator, old)], Interval(2))
        .unwrap();

    assert_eq!(
        repository.pending_vote(validator).unwrap(),
        Some(new),
        "an older vote must not displace a newer one"
    );
}

#[test]
fn should_merge_pending_votes_into_the_counted_map_at_the_tick() {
    let (mut repository, links) = with_chain(&[1]);
    let validator = ValidatorIndex(0);
    let cast = vote(1, Checkpoint::default());
    repository
        .record_pending_votes(&[(validator, cast)], Interval(3))
        .unwrap();

    repository
        .commit_tick(&TickCommit {
            head: links[0].root,
            latest_justified: Checkpoint::default(),
            latest_finalized: Checkpoint::default(),
            interval: Interval(4),
            merge_pending_votes: true,
        })
        .unwrap();

    assert_eq!(repository.known_vote(validator).unwrap(), Some(cast));
    assert!(
        repository.pending_votes().unwrap().is_empty(),
        "the pending map is cleared whether or not the vote won"
    );
    assert_eq!(
        repository.last_processed_interval().unwrap(),
        Some(Interval(4))
    );
}

#[test]
fn should_discard_votes_the_fork_choice_can_no_longer_be_moved_by() {
    let (mut repository, links) = with_chain(&[1, 2]);
    let finalized = Checkpoint {
        root: links[0].root,
        slot: Slot(1),
    };

    let live = vote(
        2,
        Checkpoint {
            root: links[1].root,
            slot: Slot(2),
        },
    );
    let below_finalized = vote(1, finalized);
    let off_chain = vote(
        5,
        Checkpoint {
            root: [9u8; 32],
            slot: Slot(5),
        },
    );
    repository
        .record_pending_votes(
            &[
                (ValidatorIndex(0), live),
                (ValidatorIndex(1), below_finalized),
                (ValidatorIndex(2), off_chain),
            ],
            Interval(6),
        )
        .unwrap();
    repository
        .commit_tick(&TickCommit {
            head: links[1].root,
            latest_justified: finalized,
            latest_finalized: finalized,
            interval: Interval(9),
            merge_pending_votes: true,
        })
        .unwrap();

    assert_eq!(repository.prune_stale_votes().unwrap(), 2);
    assert_eq!(
        repository.known_vote(ValidatorIndex(0)).unwrap(),
        Some(live)
    );
    assert_eq!(repository.known_vote(ValidatorIndex(1)).unwrap(), None);
    assert_eq!(
        repository.known_vote(ValidatorIndex(2)).unwrap(),
        None,
        "descent that cannot be shown is not descent"
    );
}

#[test]
fn should_not_prune_proofs_before_the_window_has_passed() {
    let (mut repository, _) = with_chain(&[1]);
    assert_eq!(
        repository
            .prune_block_proofs(Slot(PROOF_RETENTION_SLOTS))
            .unwrap(),
        None,
        "the cutoff saturates at zero for a young chain"
    );
}

#[test]
fn should_not_prune_a_proof_inside_the_non_finalized_range() {
    let (mut repository, _) = with_chain(&[1]);
    // Finalization is still at genesis, so a cutoff above it must not be acted on.
    assert_eq!(
        repository
            .prune_block_proofs(Slot(PROOF_RETENTION_SLOTS + 8_400))
            .unwrap(),
        None
    );
}

#[test]
fn should_prune_only_proofs_below_the_cutoff() {
    let (mut repository, links) = with_chain(&[1, 9_000]);
    let finalized = Checkpoint {
        root: links[1].root,
        slot: Slot(9_000),
    };
    repository
        .commit_tick(&TickCommit {
            head: links[1].root,
            latest_justified: finalized,
            latest_finalized: finalized,
            interval: Interval(45_004),
            merge_pending_votes: false,
        })
        .unwrap();

    let cutoff = repository
        .prune_block_proofs(Slot(PROOF_RETENTION_SLOTS + 8_400))
        .unwrap();
    assert_eq!(cutoff, Some(Slot(8_400)));
    assert_eq!(
        repository.block_proof(Slot(1), links[0].root).unwrap(),
        None
    );
    assert_eq!(
        repository
            .block_proof(Slot(9_000), links[1].root)
            .unwrap()
            .as_ref(),
        Some(&proof(1)),
        "a proof above the cutoff survives"
    );
    assert!(
        repository.block_header(links[0].root).unwrap().is_some(),
        "only the proof expires; the block is retained"
    );
}

#[test]
fn should_refuse_a_range_request_below_the_advertised_floor() {
    let (repository, _) = with_chain(&[1]);
    // A node anchored at genesis is bound by the spec floor alone.
    assert_eq!(
        repository.range_service_floor(Slot(10_000)).unwrap(),
        Slot(6_400)
    );
    assert!(
        !repository
            .can_serve_range(Slot(10_000), Slot(6_399))
            .unwrap()
    );
    assert!(
        repository
            .can_serve_range(Slot(10_000), Slot(6_400))
            .unwrap()
    );
}
