//! Opening a database that does not belong to this node.
//!
//! `docs/design/storage.md` is unambiguous: a populated database opens only when every
//! identity value matches, and a mismatch is never treated as an empty database and never
//! overwritten. These tests pin that behavior, because the failure mode of getting it wrong
//! is silent — a node that adopts another chain's directory produces a state that verifies
//! against itself and against nothing else.

mod common;

use common::{anchor_root, chain, identity_of};
use verity_db::backend::StorageBackend;
use verity_db::{
    AnchorCommit, ColumnFamily, Durability, Identity, MemoryBackend, MetadataKey, Repository,
    StorageError, WriteBatch,
};
use verity_types::primitives::Slot;

/// An anchored database, and the identity it was anchored with.
fn anchored() -> (MemoryBackend, Identity) {
    let (genesis, _) = chain(4, &[]);
    let identity = identity_of(&genesis);
    let mut repository =
        Repository::open(MemoryBackend::new(), identity).expect("an empty database opens");
    repository
        .commit_anchor(&AnchorCommit {
            block_root: anchor_root(&genesis),
            state: &genesis,
            body: None,
            served_from_slot: Slot(0),
        })
        .expect("genesis anchors");
    (repository.backend().clone(), identity)
}

#[test]
fn should_open_an_empty_database_and_wait_for_an_anchor() {
    let (genesis, _) = chain(4, &[]);
    let repository = Repository::open(MemoryBackend::new(), identity_of(&genesis))
        .expect("an empty database is not a mismatched one");
    assert!(!repository.is_populated().unwrap());
}

#[test]
fn should_reopen_a_database_it_anchored_itself() {
    let (backend, identity) = anchored();
    let repository = Repository::open(backend, identity).expect("its own database reopens");
    assert!(repository.is_populated().unwrap());
}

#[test]
fn should_refuse_a_database_belonging_to_another_chain() {
    let (backend, identity) = anchored();
    let other = Identity {
        chain_fingerprint: [9u8; 32],
        ..identity
    };
    assert!(matches!(
        Repository::open(backend, other).unwrap_err(),
        StorageError::Identity(verity_db::IdentityMismatch::ChainFingerprint { .. })
    ));
}

#[test]
fn should_refuse_a_database_written_under_another_fork() {
    let (backend, identity) = anchored();
    let other = Identity {
        fork_version: identity.fork_version + 1,
        ..identity
    };
    assert!(matches!(
        Repository::open(backend, other).unwrap_err(),
        StorageError::Identity(verity_db::IdentityMismatch::ForkVersion { .. })
    ));
}

#[test]
fn should_refuse_a_populated_database_missing_an_identity_value() {
    let (genesis, _) = chain(4, &[]);
    let identity = identity_of(&genesis);

    // Only one of the four identity values is present. This is corruption, not an empty
    // database, and must not be adopted as one.
    let mut backend = MemoryBackend::new();
    let mut batch = WriteBatch::new();
    batch.queue_put(
        ColumnFamily::Metadata,
        MetadataKey::ChainFingerprint.as_bytes().to_vec(),
        identity.chain_fingerprint.to_vec(),
    );
    backend.write(batch, Durability::Synced).unwrap();

    assert_eq!(
        Repository::open(backend, identity).unwrap_err(),
        StorageError::MissingMetadata(MetadataKey::SchemaVersion)
    );
}

#[test]
fn should_refuse_a_database_whose_stored_containers_changed_shape() {
    let (genesis, _) = chain(4, &[]);
    let identity = identity_of(&genesis);
    let (mut backend, _) = anchored();

    // Stand in for a container whose SSZ shape moved between builds. The bytes still decode;
    // only the manifest digest says they no longer mean the same thing.
    let mut batch = WriteBatch::new();
    batch.queue_put(
        ColumnFamily::Metadata,
        MetadataKey::SszSchemaDigest.as_bytes().to_vec(),
        [1u8; 32].to_vec(),
    );
    backend.write(batch, Durability::Synced).unwrap();

    assert!(matches!(
        Repository::open(backend, identity).unwrap_err(),
        StorageError::Identity(verity_db::IdentityMismatch::SszSchemaDigest { .. })
    ));
}
