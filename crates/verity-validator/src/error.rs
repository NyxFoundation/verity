//! What can go wrong while performing a duty, and which failures end the node.
//!
//! Only two of these are fatal, and both happen before the node serves: key loading
//! (`docs/design/key-management.md`, Decision 2 — every rejection means the node must not
//! start) and the startup preparation catch-up that follows it. Everything else is a missed
//! duty: the slot passes, the reason is logged, and the loop stays up. A validator that
//! aborted on a failed proof would take the node's whole consensus path down with it.

use core::fmt;

use verity_chain::RejectionReason;
use verity_crypto::{AggregationError, KeyLoadError, SignatureError};

/// A duty that could not be performed.
#[derive(Debug)]
pub enum DutyError {
    /// Key material could not be loaded. Fatal: the node must not start.
    KeyLoad(KeyLoadError),
    /// A key could not be brought far enough forward to sign for the current slot.
    Preparation(SignatureError),
    /// A key did not survive its own encoding when copied for an off-thread advance, which
    /// would be a library bug rather than an input problem.
    KeyDuplication,
    /// Signing failed.
    Signing(SignatureError),
    /// Aggregating or merging proofs failed.
    Aggregation(AggregationError),
    /// The block or vote this node would have produced is one no peer would accept.
    Rejected(RejectionReason),
    /// The chain view holds no post-state for its own head, so no duty can be resolved
    /// against a registry. Transient, and only around startup.
    HeadStateMissing,
    /// The proving worker did not return, which means the runtime is shutting down.
    ProverStopped,
}

impl fmt::Display for DutyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyLoad(error) => write!(f, "cannot load validator keys: {error}"),
            Self::Preparation(error) => write!(f, "cannot prepare a key: {error}"),
            Self::KeyDuplication => write!(f, "a key did not survive its own encoding"),
            Self::Signing(error) => write!(f, "cannot sign: {error}"),
            Self::Aggregation(error) => write!(f, "cannot aggregate: {error}"),
            Self::Rejected(reason) => {
                write!(f, "the duty would produce a rejected value: {reason}")
            }
            Self::HeadStateMissing => write!(f, "the chain view holds no state for its head"),
            Self::ProverStopped => write!(f, "the proving worker stopped"),
        }
    }
}

impl std::error::Error for DutyError {}

impl From<KeyLoadError> for DutyError {
    fn from(error: KeyLoadError) -> Self {
        Self::KeyLoad(error)
    }
}

impl From<AggregationError> for DutyError {
    fn from(error: AggregationError) -> Self {
        Self::Aggregation(error)
    }
}

impl From<RejectionReason> for DutyError {
    fn from(reason: RejectionReason) -> Self {
        Self::Rejected(reason)
    }
}
