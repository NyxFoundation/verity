//! Consensus decisions over the `verity-types` container shapes.
//!
//! This crate holds the pure functions the state transition and fork choice are built from.
//! It is where leanSpec's container methods live in Verity: `verity-types` carries shape and
//! serialization only, and every decision over those shapes sits behind the capability that
//! owns it, so re-binding one to the Verified Core later touches this crate and not the type
//! every other crate depends on.
//!
//! Nothing here reads a clock, a socket, or a database. `slot_clock` takes the instant it
//! should reason about as an argument, `state_transition` takes the block, and `fork_choice`
//! takes the interval to advance to, for exactly that reason. Signature verification happens
//! before any of them is called, so this crate carries no cryptographic dependency either —
//! see `fork_choice` for where leanSpec's three verifying entry points are split apart.

pub mod block_production;
pub mod error;
pub mod fork_choice;
pub mod justification;
pub mod merkle;
pub mod proposer;
pub mod slot_clock;
pub mod state_transition;
pub mod view;

pub use block_production::{BuiltBlock, build_block, select_proofs_for_coverage};
pub use error::RejectionReason;
pub use fork_choice::{
    AttestationSignature, AttestationSignatureEntry, Store, accept_new_attestations,
    attestation_data, attestation_target, block_weights, on_block, on_tick,
    prune_stale_attestation_data, record_aggregated_payload, record_attestation_signature,
    update_head, update_safe_target, validate_attestation, validate_attestation_signer,
};
pub use justification::{
    IMMEDIATE_JUSTIFICATION_WINDOW, advance_checkpoint, extend_justified_slots_to,
    is_justifiable_after, is_slot_justified, justified_index_after,
};
pub use merkle::hash_tree_root;
pub use proposer::proposer_for_slot;
pub use slot_clock::{SlotClock, intervals_at_slot_start};
pub use state_transition::{
    generate_genesis, process_attestations, process_block, process_block_header, process_slots,
    state_transition,
};
pub use view::ChainView;
