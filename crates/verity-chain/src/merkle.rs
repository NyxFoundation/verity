//! The one place the hash tree root hasher is chosen.
//!
//! `hash_tree_root` is a capability contract in `ARCHITECTURE.md`, currently satisfied by the
//! external SSZ library. Routing every call in this crate through one function is what keeps
//! that swap a one-file change: nothing else names a hasher.

use libssz_merkle::{HashTreeRoot, Sha2Hasher};
use verity_types::Bytes32;

/// The SSZ hash tree root of a consensus value.
#[must_use = "this computes the root; it does not store or commit to it"]
pub fn hash_tree_root<T: HashTreeRoot>(value: &T) -> Bytes32 {
    value.hash_tree_root(&Sha2Hasher)
}
