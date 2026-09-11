//! The sync mode lifecycle: three states, one trigger, evaluated continuously.
//!
//! `docs/design/sync.md`, Decision 1. The whole of the machine is here, deliberately without
//! I/O: it is fed a head slot and what peers claim, and it answers with a state. Everything
//! that decides *when* to feed it lives in [`crate::sync`].

use verity_types::Slot;

/// Where a node is relative to the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    /// Nothing is known yet: no peer has answered a status handshake.
    Idle,
    /// The node is behind, or has not yet confirmed that it is not.
    Syncing,
    /// The node's head is at or above what the network claims is final.
    Synced,
}

impl SyncState {
    /// Whether this node considers itself caught up with the network.
    ///
    /// Observability, not a gate. Validator duties turn on head-versus-clock lag, which is a
    /// different question and lives in `verity-validator` — see `docs/design/sync.md`,
    /// Decision 1, on why a peer-derived state cannot answer "is this node's view stale".
    #[must_use]
    pub const fn is_caught_up(self) -> bool {
        matches!(self, Self::Synced)
    }
}

impl core::fmt::Display for SyncState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Idle => "idle",
            Self::Syncing => "syncing",
            Self::Synced => "synced",
        })
    }
}

/// The state machine itself.
#[derive(Debug, Default)]
pub struct SyncMachine {
    state: Option<SyncState>,
}

impl SyncMachine {
    /// A machine that has heard from nobody yet.
    #[must_use]
    pub const fn new() -> Self {
        Self { state: None }
    }

    /// The current state.
    #[must_use]
    pub fn state(&self) -> SyncState {
        self.state.unwrap_or(SyncState::Idle)
    }

    /// Re-evaluates the trigger against one observation, and answers with the new state.
    ///
    /// `network_finalized` is the median of connected peers' claimed finalized slots, or
    /// `None` while no peer has answered. With no peers there is no network to be behind, so
    /// the state does not move — a node alone on a segment stays where it is rather than
    /// declaring itself synced against nothing.
    ///
    /// The one transition that does not exist is `IDLE → SYNCED`: a node that has just met
    /// the network passes through `SYNCING` even when it turns out to be caught up already.
    /// The reference node's guard, kept, and it costs one further observation — which arrives
    /// on the next status refresh or the next snapshot, not on a timer of its own.
    pub fn observe(&mut self, head_slot: Slot, network_finalized: Option<Slot>) -> SyncState {
        let Some(network_finalized) = network_finalized else {
            return self.state();
        };

        let behind = head_slot.0 < network_finalized.0;
        let next = match (self.state, behind) {
            (None, _) => SyncState::Syncing,
            (Some(SyncState::Syncing), false) => SyncState::Synced,
            (Some(SyncState::Synced), true) => SyncState::Syncing,
            (Some(current), _) => current,
        };

        if self.state != Some(next) {
            tracing::info!(
                from = %self.state(),
                to = %next,
                head = head_slot.0,
                network_finalized = network_finalized.0,
                "sync state changed"
            );
        }
        self.state = Some(next);
        next
    }
}

/// The network's finalized slot: the median of what peers claim.
///
/// The median is what "majority vote" means here — no minority of lying peers can move it
/// outside the range the honest peers occupy. For an even count the lower middle value is
/// taken, which is the conservative choice: it under-states the network rather than holding
/// the node in `SYNCING` against a claim nobody can serve.
#[must_use]
pub fn median_finalized(mut claims: Vec<Slot>) -> Option<Slot> {
    if claims.is_empty() {
        return None;
    }
    claims.sort_unstable_by_key(|slot| slot.0);
    Some(claims[(claims.len() - 1) / 2])
}

#[cfg(test)]
mod tests {
    use super::{SyncMachine, SyncState, median_finalized};
    use verity_types::Slot;

    #[test]
    fn should_take_the_lower_middle_claim_when_the_peer_count_is_even() {
        let claims = vec![Slot(10), Slot(20), Slot(30), Slot(40)];
        assert_eq!(median_finalized(claims), Some(Slot(20)));
    }

    #[test]
    fn should_take_the_middle_claim_when_the_peer_count_is_odd() {
        let claims = vec![Slot(30), Slot(10), Slot(20)];
        assert_eq!(median_finalized(claims), Some(Slot(20)));
    }

    #[test]
    fn should_ignore_a_single_lying_peer_in_a_healthy_set() {
        let honest = vec![Slot(100), Slot(101), Slot(100)];
        let mut lying = honest.clone();
        lying.push(Slot(9_000_000));
        // The liar moves the median by at most one honest position, never outside the set.
        assert!(median_finalized(lying).expect("a median").0 <= 101);
    }

    #[test]
    fn should_answer_none_when_no_peer_has_claimed_anything() {
        assert_eq!(median_finalized(Vec::new()), None);
    }

    #[test]
    fn should_stay_idle_while_no_peer_has_answered() {
        let mut machine = SyncMachine::new();
        assert_eq!(machine.observe(Slot(0), None), SyncState::Idle);
        assert!(!machine.state().is_caught_up());
    }

    #[test]
    fn should_never_shortcut_from_idle_to_synced() {
        let mut machine = SyncMachine::new();
        // Caught up on the very first observation, and still not synced.
        assert_eq!(
            machine.observe(Slot(50), Some(Slot(50))),
            SyncState::Syncing
        );
        assert_eq!(machine.observe(Slot(50), Some(Slot(50))), SyncState::Synced);
    }

    #[test]
    fn should_demote_to_syncing_when_a_gap_reappears() {
        let mut machine = SyncMachine::new();
        machine.observe(Slot(50), Some(Slot(50)));
        assert_eq!(machine.observe(Slot(50), Some(Slot(50))), SyncState::Synced);
        assert_eq!(
            machine.observe(Slot(50), Some(Slot(120))),
            SyncState::Syncing
        );
    }

    #[test]
    fn should_hold_its_state_when_every_peer_disconnects() {
        let mut machine = SyncMachine::new();
        machine.observe(Slot(50), Some(Slot(50)));
        machine.observe(Slot(50), Some(Slot(50)));
        assert_eq!(machine.observe(Slot(50), None), SyncState::Synced);
    }
}
