//! Telling a caller what the transition is doing, without the transition keeping time.
//!
//! leanMetrics asks for the duration of each transition stage and for every finalization
//! attempt. Both are facts about *when* the stages ran, and this crate reads no clock (see
//! the crate docs). So the transition reports the boundaries of its stages as they happen and
//! leaves timestamping to whoever holds the clock: the node's observer records an `Instant`
//! on each event, and the transition stays a pure function of its arguments.
//!
//! `state_transition` and `on_block` keep their plain signatures; the observed variants take
//! the observer as one extra argument and the plain ones pass a no-op.

/// A boundary the transition crosses, in the order it crosses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionEvent {
    /// The whole transition begins.
    TransitionBegin,
    /// Empty-slot processing begins.
    SlotsBegin,
    /// Empty-slot processing ends, having advanced this many slots.
    SlotsEnd {
        /// Slots advanced through, the block's own included.
        processed: u64,
    },
    /// Block processing — header and body — begins.
    BlockBegin,
    /// Attestation processing begins, inside block processing.
    AttestationsBegin,
    /// Attestation processing ends, having applied this many aggregates.
    AttestationsEnd {
        /// Aggregates in the body.
        processed: usize,
    },
    /// Block processing ends.
    BlockEnd,
    /// The whole transition ends. Not reached when a stage rejects the block.
    TransitionEnd,
    /// A vote justified its target with a source above the finalized boundary, which is a
    /// finalization attempt: `advanced` says whether it moved the boundary.
    FinalizationAttempt {
        /// Whether the finalized checkpoint advanced to the source.
        advanced: bool,
    },
}

/// Whoever wants to know when the transition's stages run.
pub trait TransitionObserver {
    /// Called at each boundary, in order.
    fn observe(&mut self, event: TransitionEvent);
}

impl<F: FnMut(TransitionEvent)> TransitionObserver for F {
    fn observe(&mut self, event: TransitionEvent) {
        self(event);
    }
}

/// The observer the plain entry points use: it hears everything and keeps nothing.
#[derive(Debug, Default, Clone, Copy)]
pub struct Unobserved;

impl TransitionObserver for Unobserved {
    fn observe(&mut self, _: TransitionEvent) {}
}
