//! The node's one wall clock.
//!
//! `docs/design/concurrency.md` makes this the sole source of consensus time: the chain task
//! reads it as channel ①, and the validator duty loop reads the same channel to know which
//! duty is owed. Two consumers, one clock — a duty is never triggered by a callback from the
//! chain task, so the two can never disagree about what time it is.
//!
//! # Latest-only is sound here
//!
//! The channel is a `watch`, so a consumer that was busy sees only the newest interval. That
//! loses nothing, because `on_tick` steps to its target one interval at a time and skips no
//! interval's action: the target is all the chain task needs to replay everything between.
//! What it buys is that time is never queued behind itself — a node catching up does not
//! first work through a backlog of stale ticks.

use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tokio::task::JoinHandle;

use verity_chain::SlotClock;
use verity_types::Interval;

/// Milliseconds since the Unix epoch, saturating at zero before it.
///
/// A clock set before 1970 is not a case worth a `Result`: every accessor downstream already
/// clamps to slot 0, which is what this saturation feeds.
#[must_use]
pub fn now_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// Starts the interval ticker, and returns the channel it publishes on.
///
/// The channel opens holding the current interval rather than a placeholder, so a consumer
/// that reads before the first tick still sees the right time.
///
/// The returned handle is how the ticker is stopped: aborting it drops the sender, which
/// closes the channel and is the only shutdown signal the consumers get
/// (`docs/design/concurrency.md`, Lifecycle).
#[must_use = "dropping the handle leaves the ticker running with no way to stop it"]
pub fn spawn(clock: SlotClock) -> (watch::Receiver<Interval>, JoinHandle<()>) {
    let (sender, receiver) = watch::channel(clock.total_intervals(now_milliseconds()));

    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(clock.until_next_interval(now_milliseconds())).await;

            // A send fails only when every consumer is gone, which means the node is already
            // shutting down and there is nothing left to tick for.
            if sender
                .send(clock.total_intervals(now_milliseconds()))
                .is_err()
            {
                break;
            }
        }
    });

    (receiver, ticker)
}

#[cfg(test)]
mod tests {
    use verity_chain::SlotClock;
    use verity_types::config::INTERVALS_PER_SLOT;

    use super::{now_milliseconds, spawn};

    #[tokio::test]
    async fn should_open_the_channel_at_the_current_interval() {
        // Genesis two slots ago, so the ticker's opening value is a known interval count.
        let genesis = now_milliseconds() / 1000 - 2 * verity_types::config::SECONDS_PER_SLOT;
        let (receiver, ticker) = spawn(SlotClock::new(genesis));

        assert!(receiver.borrow().0 >= 2 * INTERVALS_PER_SLOT);
        ticker.abort();
    }

    #[tokio::test]
    async fn should_advance_when_an_interval_boundary_passes() {
        let genesis = now_milliseconds() / 1000;
        let (mut receiver, ticker) = spawn(SlotClock::new(genesis));
        let opening = *receiver.borrow_and_update();

        receiver.changed().await.expect("the ticker is running");
        assert!(*receiver.borrow() > opening);
        ticker.abort();
    }
}
