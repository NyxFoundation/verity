//! Conversion between wall-clock time and consensus slots and intervals.
//!
//! The clock holds no time source. Every accessor takes the instant it should reason about,
//! in milliseconds since the Unix epoch, so the arithmetic stays pure and testable against
//! the spec's vectors. Reading the actual clock belongs to the orchestrator that drives the
//! node — see `ARCHITECTURE.md`, "I/O Edge".
//!
//! Transcribed from leanSpec `src/lean_spec/node/chain/clock.py`, read at commit
//! `0588c2d215a955a516378677a92db2a5666802f3`.

use std::time::Duration;

use verity_types::config::{INTERVALS_PER_SLOT, MILLISECONDS_PER_INTERVAL, MILLISECONDS_PER_SLOT};
use verity_types::{Interval, Slot};

/// Converts wall-clock time to consensus slots and intervals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotClock {
    genesis_time: u64,
}

impl SlotClock {
    /// Builds a clock anchored at `genesis_time`, a Unix timestamp in **seconds**.
    #[must_use]
    pub const fn new(genesis_time: u64) -> Self {
        Self { genesis_time }
    }

    /// The Unix timestamp, in seconds, at which slot 0 began.
    #[must_use]
    pub const fn genesis_time(&self) -> u64 {
        self.genesis_time
    }

    /// Milliseconds elapsed since genesis, saturating to 0 before it.
    ///
    /// Clamping rather than signing is what makes every accessor below total: a node started
    /// ahead of genesis reports slot 0 and interval 0 instead of failing.
    #[must_use]
    pub const fn milliseconds_since_genesis(&self, now_milliseconds: u64) -> u64 {
        now_milliseconds.saturating_sub(self.genesis_time * 1000)
    }

    /// The slot containing `now_milliseconds`, or slot 0 before genesis.
    #[must_use]
    pub const fn current_slot(&self, now_milliseconds: u64) -> Slot {
        Slot(self.milliseconds_since_genesis(now_milliseconds) / MILLISECONDS_PER_SLOT)
    }

    /// The interval within the current slot, in `0..INTERVALS_PER_SLOT`.
    #[must_use]
    pub const fn current_interval(&self, now_milliseconds: u64) -> Interval {
        let into_slot = self.milliseconds_since_genesis(now_milliseconds) % MILLISECONDS_PER_SLOT;
        Interval(into_slot / MILLISECONDS_PER_INTERVAL)
    }

    /// Intervals elapsed since genesis, counting across slot boundaries.
    #[must_use]
    pub const fn total_intervals(&self, now_milliseconds: u64) -> Interval {
        Interval(self.milliseconds_since_genesis(now_milliseconds) / MILLISECONDS_PER_INTERVAL)
    }

    /// Time remaining until the next interval boundary.
    ///
    /// Before genesis the next boundary is genesis itself. Exactly on a boundary this returns
    /// a full interval rather than zero, so a caller looping on it always advances.
    #[must_use]
    pub const fn until_next_interval(&self, now_milliseconds: u64) -> Duration {
        let genesis_milliseconds = self.genesis_time * 1000;
        if now_milliseconds < genesis_milliseconds {
            return Duration::from_millis(genesis_milliseconds - now_milliseconds);
        }
        let into_interval =
            self.milliseconds_since_genesis(now_milliseconds) % MILLISECONDS_PER_INTERVAL;
        Duration::from_millis(MILLISECONDS_PER_INTERVAL - into_interval)
    }
}

/// The interval count at the start of `slot`.
///
/// Slot boundaries fall on exact multiples of the per-slot interval count.
#[must_use]
pub const fn intervals_at_slot_start(slot: Slot) -> Interval {
    Interval(slot.0 * INTERVALS_PER_SLOT)
}

#[cfg(test)]
mod tests {
    use super::{SlotClock, intervals_at_slot_start};
    use std::time::Duration;
    use verity_types::{Interval, Slot};

    /// Genesis used by leanSpec's own slot-clock vectors.
    const GENESIS: u64 = 1_700_000_000;
    const GENESIS_MS: u64 = GENESIS * 1000;

    #[test]
    fn should_report_slot_zero_when_the_instant_precedes_genesis() {
        let clock = SlotClock::new(GENESIS);
        assert_eq!(clock.current_slot(GENESIS_MS - 10_000), Slot(0));
        assert_eq!(clock.current_interval(GENESIS_MS - 10_000), Interval(0));
        assert_eq!(clock.total_intervals(GENESIS_MS - 10_000), Interval(0));
    }

    #[test]
    fn should_advance_the_slot_exactly_on_the_boundary_and_not_before() {
        let clock = SlotClock::new(GENESIS);
        assert_eq!(clock.current_slot(GENESIS_MS + 3_999), Slot(0));
        assert_eq!(clock.current_slot(GENESIS_MS + 4_000), Slot(1));
    }

    #[test]
    fn should_wrap_the_interval_when_the_slot_rolls_over() {
        let clock = SlotClock::new(GENESIS);
        assert_eq!(clock.current_interval(GENESIS_MS + 3_200), Interval(4));
        assert_eq!(clock.current_interval(GENESIS_MS + 4_000), Interval(0));
        assert_eq!(clock.total_intervals(GENESIS_MS + 4_000), Interval(5));
    }

    #[test]
    fn should_return_a_full_interval_when_sitting_exactly_on_a_boundary() {
        let clock = SlotClock::new(GENESIS);
        assert_eq!(
            clock.until_next_interval(GENESIS_MS + 800),
            Duration::from_millis(800)
        );
    }

    #[test]
    fn should_count_down_to_genesis_when_called_before_it() {
        let clock = SlotClock::new(GENESIS);
        assert_eq!(
            clock.until_next_interval(GENESIS_MS - 1_500),
            Duration::from_millis(1_500)
        );
    }

    #[test]
    fn should_land_on_the_slot_start_when_converting_a_slot_to_intervals() {
        assert_eq!(intervals_at_slot_start(Slot(0)), Interval(0));
        assert_eq!(intervals_at_slot_start(Slot(10)), Interval(50));
    }
}
