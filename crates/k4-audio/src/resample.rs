//! Linear resampler for voice audio (e.g. 12 kHz ↔ device rate).
//!
//! Single-channel, stateful across calls so consecutive blocks join seamlessly
//! (it remembers the last input sample to interpolate across the boundary).
//! Linear interpolation is adequate for narrowband voice; a higher-quality
//! polyphase filter can replace it later behind the same API.

/// A stateful single-channel linear resampler.
#[derive(Debug, Clone)]
pub struct LinearResampler {
    /// Input samples consumed per output sample (`in_rate / out_rate`).
    step: f64,
    /// Fractional read position within the current input block.
    pos: f64,
    /// Last sample of the previous block (index −1 for interpolation).
    prev: f32,
    has_prev: bool,
}

impl LinearResampler {
    /// Create a resampler from `in_rate` to `out_rate` (Hz).
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        let out_rate = out_rate.max(1);
        Self {
            step: in_rate as f64 / out_rate as f64,
            pos: 0.0,
            prev: 0.0,
            has_prev: false,
        }
    }

    /// Resample one `input` block, appending output samples to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let n = input.len();
        if n == 0 {
            return;
        }
        let sample = |idx: isize| -> f32 {
            if idx < 0 {
                if self.has_prev {
                    self.prev
                } else {
                    input[0]
                }
            } else {
                input[idx as usize]
            }
        };

        // Emit output while the upper interpolation index is within this block
        // (upper = floor(pos)+1 must be ≤ n-1, i.e. floor(pos) < n-1).
        while (self.pos.floor() as isize) < n as isize - 1 {
            let base = self.pos.floor();
            let lo = sample(base as isize);
            let hi = sample(base as isize + 1);
            let frac = (self.pos - base) as f32;
            out.push(lo + (hi - lo) * frac);
            self.pos += self.step;
        }

        // Carry the fractional position into the next block, retain boundary.
        self.pos -= n as f64;
        self.prev = input[n - 1];
        self.has_prev = true;
    }
}
