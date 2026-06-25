//! Sequence-ordered jitter buffer (FR-AUD-02, FR-AUD-05).
//!
//! Inbound audio frames carry a wrapping 0–255 sequence byte. This buffer:
//! - releases frames in sequence order (reordering late arrivals),
//! - drops duplicates and frames already played ("late"),
//! - and, when it has buffered up to `capacity` frames across a gap, conceals
//!   the missing frame by skipping to the next available one (bounding latency).
//!
//! Sequence comparisons are done modulo 256: `x` is "ahead" of `base` when
//! `x.wrapping_sub(base) < 128`.

use std::collections::BTreeMap;

/// A reordering buffer keyed by the wrapping audio sequence number.
#[derive(Debug)]
pub struct JitterBuffer {
    capacity: usize,
    next: Option<u8>,
    pending: BTreeMap<u8, Vec<u8>>,
}

impl JitterBuffer {
    /// Create a buffer that holds at most `capacity` frames before concealing a
    /// gap. `capacity` is clamped to at least 1.
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next: None,
            pending: BTreeMap::new(),
        }
    }

    /// Number of frames currently buffered.
    pub fn len(&self) -> usize {
        self.pending.len()
    }
    /// Whether the buffer holds no frames.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Insert a frame. Duplicates and late frames (already played) are dropped.
    ///
    /// trace: FR-AUD-02, FR-AUD-05
    pub fn push(&mut self, sequence: u8, frame: Vec<u8>) {
        let next = *self.next.get_or_insert(sequence);
        // Distance from the expected sequence: 0 = expected, <128 = future.
        if sequence.wrapping_sub(next) >= 128 {
            return; // late / already played → drop
        }
        // Dedup: keep the first frame seen for a sequence.
        self.pending.entry(sequence).or_insert(frame);
    }

    /// Pop the next in-order frame if available. Returns `None` while waiting for
    /// a missing frame, unless the buffer is full — then it conceals the gap by
    /// jumping to the nearest buffered future frame.
    ///
    /// trace: FR-AUD-02
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        let next = self.next?;

        if let Some(frame) = self.pending.remove(&next) {
            self.next = Some(next.wrapping_add(1));
            return Some(frame);
        }

        // Gap at `next`. Hold unless we've buffered up to capacity.
        if self.pending.len() < self.capacity {
            return None;
        }

        // Conceal: jump to the buffered frame with the smallest forward distance.
        let target = *self
            .pending
            .keys()
            .min_by_key(|&&seq| seq.wrapping_sub(next))?;
        let frame = self.pending.remove(&target);
        self.next = Some(target.wrapping_add(1));
        frame
    }
}
