//! Bounded sample ring buffer (FR-AUD-02).
//!
//! Shared between the worker thread and a cpal callback via `Arc<Mutex<_>>`.
//! On overflow it drops the oldest samples (favouring fresh audio over unbounded
//! latency); on underflow `pop` yields `None` (the callback substitutes silence).

use std::collections::VecDeque;

/// A fixed-capacity FIFO of `f32` samples.
#[derive(Debug)]
pub struct SampleRing {
    buf: VecDeque<f32>,
    capacity: usize,
}

impl SampleRing {
    /// Create a ring holding at most `capacity` samples (clamped to ≥1).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            buf: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Append samples, dropping the oldest if capacity is exceeded.
    pub fn push_slice(&mut self, samples: &[f32]) {
        for &s in samples {
            if self.buf.len() == self.capacity {
                self.buf.pop_front();
            }
            self.buf.push_back(s);
        }
    }

    /// Remove and return the oldest sample, or `None` if empty.
    pub fn pop(&mut self) -> Option<f32> {
        self.buf.pop_front()
    }

    /// Number of buffered samples.
    pub fn len(&self) -> usize {
        self.buf.len()
    }
    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}
