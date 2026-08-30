//! What this build of the repository writes, and how a database says what it holds.
//!
//! Four values identify a database: the repository schema version, the chain, the protocol
//! fork, and the shape of the SSZ containers stored inside it. All four are written once by
//! the anchor commit and checked on every subsequent open.
//!
//! The last of them is the interesting one. A stored value is raw SSZ, so a field added to
//! `State` or a reordered `StateDiff` does not fail to decode — it decodes into a *different*
//! value. Hashing a description of every stored container turns that silent reinterpretation
//! into a refusal to open, and the digest constant in this module's test into a review
//! artifact: a shape change that does not move the digest is a shape change that was not
//! actually made.
//!
//! Transcribed from `docs/design/storage.md`, "Metadata and identity".

use sha2::{Digest, Sha256};
use verity_types::Bytes32;
use verity_types::config::{
    BYTE_LIST_512_KIB_LIMIT, HISTORICAL_ROOTS_LIMIT, JUSTIFICATION_VALIDATORS_LIMIT,
    VALIDATOR_REGISTRY_LIMIT,
};

/// The repository schema version this build writes.
///
/// Bump it whenever the key layout, the table set, or the metadata key set changes. A
/// container *shape* change does not need a bump — the manifest digest already catches it —
/// but bumping is harmless and the two are checked independently.
pub const SCHEMA_VERSION: u32 = 1;

/// The stored container types, in the fixed order `docs/design/storage.md` lists them.
///
/// Order is part of the digest. It is the doc's order, not alphabetical and not dependency
/// order, so the two can be diffed against each other by eye.
pub const STORED_TYPES: [&str; 7] = [
    "BlockHeader",
    "BlockBody",
    "MultiMessageAggregate",
    "State",
    "StateDiff",
    "Checkpoint",
    "AttestationData",
];

/// The field list of each stored type, paired with [`STORED_TYPES`] by position.
///
/// Written out rather than derived. The SSZ traits expose an encoding, not a description of
/// the shape that produced it, so there is nothing to reflect over: two containers with the
/// same field types in a different order are indistinguishable at the trait level and would
/// hash identically. Transcribing the fields is what makes the reorder visible.
///
/// Collection limits are interpolated from the constants rather than typed as literals, so a
/// limit change moves the digest on its own — it changes merkle depth, hence every root, and
/// must not be able to slip past this check just because nobody edited this file.
fn field_lists() -> [String; STORED_TYPES.len()] {
    let registry = VALIDATOR_REGISTRY_LIMIT;
    let roots = HISTORICAL_ROOTS_LIMIT;
    let votes = JUSTIFICATION_VALIDATORS_LIMIT;
    let proof = BYTE_LIST_512_KIB_LIMIT;

    [
        "slot:uint64,proposer_index:uint64,parent_root:Bytes32,state_root:Bytes32,\
         body_root:Bytes32"
            .to_owned(),
        format!(
            "attestations:List[AggregatedAttestation{{aggregation_bits:Bitlist[{registry}],\
             data:AttestationData}},{registry}]"
        ),
        format!("proof:List[uint8,{proof}]"),
        format!(
            "config:GenesisConfig{{genesis_time:uint64}},slot:uint64,\
             latest_block_header:BlockHeader,latest_justified:Checkpoint,\
             latest_finalized:Checkpoint,historical_block_hashes:List[Bytes32,{roots}],\
             justified_slots:Bitlist[{roots}],\
             validators:List[Validator{{attestation_public_key:Bytes52,\
             proposal_public_key:Bytes52,index:uint64}},{registry}],\
             justifications_roots:List[Bytes32,{roots}],\
             justifications_validators:Bitlist[{votes}]"
        ),
        format!(
            "base_block_root:Bytes32,slot:uint64,latest_justified:Checkpoint,\
             latest_finalized:Checkpoint,justified_slots:Bitlist[{roots}],\
             justifications_roots:List[Bytes32,{roots}],\
             justifications_validators:Bitlist[{votes}]"
        ),
        "root:Bytes32,slot:uint64".to_owned(),
        "slot:uint64,head:Checkpoint,target:Checkpoint,source:Checkpoint".to_owned(),
    ]
}

/// SHA-256 over the versioned stored-type manifest.
///
/// The version is hashed in as well, so a key-layout bump and a shape change are not able to
/// cancel each other out.
#[must_use]
pub fn ssz_schema_digest() -> Bytes32 {
    let mut hasher = Sha256::new();
    hasher.update(b"verity-db stored-type manifest v");
    hasher.update(SCHEMA_VERSION.to_le_bytes());
    for (name, fields) in STORED_TYPES.iter().zip(field_lists()) {
        hasher.update(name.as_bytes());
        hasher.update(b"(");
        hasher.update(fields.as_bytes());
        hasher.update(b")");
    }
    hasher.finalize().into()
}

/// What a node expects the database it opens to hold.
///
/// `chain_fingerprint` is the genesis state's `hash_tree_root`, which already commits to the
/// genesis time, the configuration, and the validator registry — so it identifies the chain
/// without introducing a second chain-identity format beside the state root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Identity {
    /// The genesis state root of the chain this node is configured for.
    pub chain_fingerprint: Bytes32,
    /// The canonical protocol fork version.
    pub fork_version: u64,
}

#[cfg(test)]
mod tests {
    use super::{STORED_TYPES, field_lists, ssz_schema_digest};

    /// The digest this build produces.
    ///
    /// This constant is the review artifact. Any change to a stored container's shape, or to
    /// a collection limit, moves it — and a diff that touches a container without touching
    /// this line is a diff that did not change the shape it appears to change.
    const EXPECTED: &str = "6f93f71bb43fcd15cb541ef5f10ceb49863521aa2f9553f5f4fd1da5685d3084";

    #[test]
    fn should_pair_every_stored_type_with_a_field_list() {
        assert_eq!(field_lists().len(), STORED_TYPES.len());
    }

    #[test]
    fn should_be_deterministic_across_calls() {
        assert_eq!(ssz_schema_digest(), ssz_schema_digest());
    }

    #[test]
    fn should_describe_every_field_of_every_stored_type() {
        // The digest can only catch a reordered container if the description it hashes names
        // the fields in order. A field list that lost its names would still hash, and would
        // still look like it was doing its job.
        for fields in field_lists() {
            assert!(fields.contains(':'), "a field list must name field types");
        }
    }

    #[test]
    fn should_match_the_digest_recorded_for_this_build() {
        let digest: String = ssz_schema_digest()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(digest, EXPECTED);
    }
}
