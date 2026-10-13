//! Bounded FIFO storage for verification inputs waiting on chain state.

use std::collections::VecDeque;

#[derive(Debug)]
pub(super) struct PendingBuffer<T> {
    entries: VecDeque<T>,
    capacity: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ParkResult<T> {
    Added { evicted: Option<T> },
    Duplicate,
    Unbuffered(T),
}

impl<T: PartialEq> PendingBuffer<T> {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn park(&mut self, entry: T) -> ParkResult<T> {
        if self.capacity == 0 {
            return ParkResult::Unbuffered(entry);
        }
        if self.entries.contains(&entry) {
            return ParkResult::Duplicate;
        }
        let evicted = (self.entries.len() >= self.capacity)
            .then(|| self.entries.pop_front())
            .flatten();
        self.entries.push_back(entry);
        ParkResult::Added { evicted }
    }

    pub(super) fn take_resolvable(&mut self, mut is_resolvable: impl FnMut(&T) -> bool) -> Vec<T> {
        let mut ready = Vec::new();
        let mut retained = VecDeque::with_capacity(self.entries.len());
        for entry in core::mem::take(&mut self.entries) {
            if is_resolvable(&entry) {
                ready.push(entry);
            } else {
                retained.push_back(entry);
            }
        }
        self.entries = retained;
        ready
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::{ParkResult, PendingBuffer};

    #[test]
    fn should_leave_a_zero_capacity_pending_buffer_empty() {
        let mut pending = PendingBuffer::new(0);
        assert_eq!(pending.park(7), ParkResult::Unbuffered(7));
        assert!(pending.len() == 0);
    }

    #[test]
    fn should_deduplicate_and_evict_pending_items_in_fifo_order() {
        let mut pending = PendingBuffer::new(2);
        assert_eq!(pending.park(1), ParkResult::Added { evicted: None });
        assert_eq!(pending.park(1), ParkResult::Duplicate);
        assert_eq!(pending.park(2), ParkResult::Added { evicted: None });
        assert_eq!(pending.park(3), ParkResult::Added { evicted: Some(1) });
        assert_eq!(pending.entries, VecDeque::from([2, 3]));
    }

    #[test]
    fn should_take_only_resolvable_pending_items() {
        let mut pending = PendingBuffer::new(3);
        for value in [1u8, 2, 3] {
            let _ = pending.park(value);
        }
        assert_eq!(pending.take_resolvable(|value| value % 2 == 0), vec![2]);
        assert_eq!(pending.entries, VecDeque::from([1, 3]));
    }
}

#[cfg(kani)]
mod harnesses {
    use super::{ParkResult, PendingBuffer};

    /// Parking is bounded, deduplicated, and evicts exactly the oldest item.
    #[kani::proof]
    #[kani::unwind(5)]
    fn pending_mutation_preserves_capacity_and_fifo_order() {
        let first: u8 = kani::any();
        let second: u8 = kani::any();
        kani::assume(first != second);
        let mut pending = PendingBuffer::new(1);
        assert!(matches!(
            pending.park(first),
            ParkResult::Added { evicted: None }
        ));
        assert!(matches!(pending.park(first), ParkResult::Duplicate));
        assert!(matches!(
            pending.park(second),
            ParkResult::Added {
                evicted: Some(value)
            } if value == first
        ));
        assert!(pending.len() == 1);
        assert!(pending.entries.front() == Some(&second));
    }

    /// Resolution removes exactly matching entries and preserves retained FIFO order.
    #[kani::proof]
    #[kani::unwind(7)]
    fn pending_resolution_partitions_without_loss() {
        let mut pending = PendingBuffer::new(3);
        let _ = pending.park(1u8);
        let _ = pending.park(2u8);
        let _ = pending.park(3u8);
        let ready = pending.take_resolvable(|value| value % 2 == 0);
        assert!(ready == [2]);
        assert!(pending.entries == [1, 3]);
    }

    /// A disabled buffer explicitly returns the item and never exceeds its zero bound.
    #[kani::proof]
    fn zero_capacity_never_retains_an_item() {
        let value: u8 = kani::any();
        let mut pending = PendingBuffer::new(0);
        assert!(matches!(
            pending.park(value),
            ParkResult::Unbuffered(returned) if returned == value
        ));
        assert!(pending.len() == 0);
    }
}
