//! Validator duties: when this node signs, what it signs, and with which key.
//!
//! # What belongs here
//!
//! The production half of consensus. This crate decides that a signature is owed — because
//! the slot's proposer is one of ours, because the vote for this slot has not been cast yet,
//! because the aggregation round is due — and produces it. It decides nothing about whether
//! *someone else's* signature is valid: that is `verity-chain` and the verification stage.
//!
//! # What it deliberately does not touch
//!
//! No socket, no database, no node wiring. Duties arrive as clock ticks on a `watch` channel
//! and chain state as an immutable [`ChainView`] snapshot; products leave on an `mpsc`
//! channel the wiring owns. That is the whole interface, and it is why this crate does not
//! depend on `verity-p2p`, `verity-db`, or the node that runs it.
//!
//! # The two costs this crate manages
//!
//! **Proving.** Building a block's merged proof takes seconds, and leanVM's prover is a
//! single per-process arena — two proofs at once corrupt each other. Every proving path in
//! the node therefore goes through one [`Prover`], which serializes them and keeps them off
//! the runtime's async threads.
//!
//! **Key preparation.** An XMSS key can only sign inside a window that has to be rebuilt
//! about every three days, and the rebuild is slow enough that doing it inside `sign` stalls
//! the duty. [`Keyring`] does it on a copy while the original keeps signing.
//!
//! # Source
//!
//! Duty structure and the aggregation round are transcribed from leanSpec
//! `src/lean_spec/spec/forks/lstar/validator_duties.py` and `aggregation.py`, read at commit
//! `8603fa63`. The scheduling and key-lifecycle decisions are
//! `docs/design/key-management.md`, Decisions 1 to 3.
//!
//! [`ChainView`]: verity_chain::ChainView

pub mod aggregation;
pub mod duties;
pub mod error;
pub mod keys;
pub mod product;
pub mod proofs;
pub mod prover;

pub use aggregation::aggregate;
pub use duties::DutyService;
pub use error::DutyError;
pub use keys::Keyring;
pub use product::LocalProduct;
pub use prover::Prover;
