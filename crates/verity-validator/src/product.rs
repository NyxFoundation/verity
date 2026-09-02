//! What a duty produces, and why none of it may be dropped.
//!
//! These are the node's own signatures. No peer holds them, range sync cannot recover them,
//! and losing one is a missed duty — so the channel they travel on has an awaiting sender and
//! no shedding policy, unlike the network path where load is deliberately shed
//! (`docs/design/concurrency.md`, Decision 3, channel ②).

use verity_crypto::containers::SignedAttestation;
use verity_types::{SignedAggregatedAttestation, SignedBlock};

/// One finished duty, on its way to the chain task and the network.
#[derive(Debug, Clone)]
pub enum LocalProduct {
    /// A block this node proposed, with the merged proof that binds its votes and its
    /// proposer signature.
    Block(SignedBlock),
    /// A vote this node cast, carrying its raw XMSS signature.
    Attestation(SignedAttestation),
    /// A proof this node's aggregation round produced.
    Aggregate(SignedAggregatedAttestation),
}

impl LocalProduct {
    /// A short name for logs and metrics.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Block(_) => "block",
            Self::Attestation(_) => "attestation",
            Self::Aggregate(_) => "aggregate",
        }
    }
}
