//! Consensus decisions over the `verity-types` container shapes.
//!
//! This crate holds the pure functions the state transition and fork choice are built from.
//! It is where leanSpec's container methods live in Verity: `verity-types` carries shape and
//! serialization only, and every decision over those shapes sits behind the capability that
//! owns it, so re-binding one to the Verified Core later touches this crate and not the type
//! every other crate depends on.
//!
//! Nothing here reads a clock, a socket, or a database. `slot_clock` takes the instant it
//! should reason about as an argument for exactly that reason.

pub mod justification;
pub mod slot_clock;

pub use justification::{
    IMMEDIATE_JUSTIFICATION_WINDOW, advance_checkpoint, is_justifiable_after, justified_index_after,
};
pub use slot_clock::{SlotClock, intervals_at_slot_start};
