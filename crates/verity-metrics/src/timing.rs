//! Recording elapsed time into a histogram.
//!
//! Every timing metric in the contract is in seconds, and every probe in the node measures
//! with an `Instant`; these two functions are the one place the conversion lives.

use std::time::{Duration, Instant};

use prometheus::Histogram;

/// Records the time since `started`.
pub fn observe_elapsed(histogram: &Histogram, started: Instant) {
    observe_duration(histogram, started.elapsed());
}

/// Records a duration already measured.
pub fn observe_duration(histogram: &Histogram, duration: Duration) {
    histogram.observe(duration.as_secs_f64());
}

/// A count as a gauge or histogram value. Nothing counted in a node approaches `i64::MAX`.
#[must_use]
pub fn count_value(count: usize) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

/// A slot or timestamp as a gauge value; saturating keeps the cast honest without a panic.
#[must_use]
pub fn gauge_value(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(kani)]
mod harnesses {
    use super::{count_value, gauge_value};

    /// A count is reported exactly while it fits, and saturates rather than wrapping after.
    #[kani::proof]
    fn counts_are_exact_or_saturated() {
        let count: usize = kani::any();
        let reported = count_value(count);
        assert!(reported >= 0);
        if let Ok(exact) = i64::try_from(count) {
            assert!(reported == exact);
        } else {
            assert!(reported == i64::MAX);
        }
    }

    /// A slot or timestamp is reported exactly while it fits, and saturates after.
    #[kani::proof]
    fn gauges_are_exact_or_saturated() {
        let value: u64 = kani::any();
        let reported = gauge_value(value);
        assert!(reported >= 0);
        if let Ok(exact) = i64::try_from(value) {
            assert!(reported == exact);
        } else {
            assert!(reported == i64::MAX);
        }
    }
}
