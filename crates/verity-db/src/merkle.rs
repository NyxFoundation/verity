//! The one place this crate chooses a hash tree root hasher.
//!
//! `verity-chain` keeps the same single-function discipline for the same reason: routing
//! every call through one place makes a change of Serialization supplier a one-file change.
//! Nothing else in `verity-db` may name a hasher.

use libssz_merkle::{HashTreeRoot, Sha2Hasher};
use verity_types::Bytes32;

/// The SSZ hash tree root of a stored value.
#[must_use]
pub(crate) fn hash_tree_root<T: HashTreeRoot>(value: &T) -> Bytes32 {
    value.hash_tree_root(&Sha2Hasher)
}
