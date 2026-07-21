//! Audio engine (ARC-09).
//!
//! Pure, always-available, tested:
//! - [`jitter`] — sequence-ordered jitter buffer (FR-AUD-02/05).
//! - [`ring`] — bounded sample ring buffer shared with audio callbacks.
//! - [`resample`] — linear resampler between 12 kHz and the device rate.
//!
//! Feature-gated:
//! - [`codec`] — Opus encode/decode (`opus` feature, default on).
//! - [`device`] — cpal capture/playback (`device` feature).
//!
//! RX path: Opus decode → resample 12 kHz→device → [`ring`] → speaker callback.
//! TX path: mic callback → resample device→12 kHz → [`ring`] → Opus encode.

pub mod jitter;
pub mod resample;
pub mod ring;

pub use jitter::JitterBuffer;
pub use resample::LinearResampler;
pub use ring::SampleRing;

#[cfg(feature = "opus")]
pub mod codec;
#[cfg(feature = "opus")]
pub use codec::{OpusDecoder, OpusEncoder};

/// Maximum local playback gain, as a linear factor (+12 dB).
///
/// The K4's streamed RX audio can arrive far below full scale — measured at
/// about -45 dBFS on one radio — and the client has no way to raise it at
/// source beyond what `AG` provides. Headroom here lets the operator make a
/// quiet stream usable. Digital gain amplifies the stream's own noise equally,
/// so this is a fallback for a quiet source, not a substitute for setting the
/// level on the radio.
pub const MAX_GAIN: f32 = 4.0;

/// Scale one receiver's channel by its local playback gain (FR-RX-VOL-01).
///
/// Separate from the master volume so the two compose: a per-receiver gain
/// balances the receivers against each other, the master moves both.
pub fn apply_rx_gains(ch: &mut [f32], gain: f32) {
    if gain == 1.0 {
        return;
    }
    for s in ch.iter_mut() {
        *s *= gain;
    }
}

#[cfg(test)]
mod rx_volume_tests {
    use super::apply_rx_gains;

    /// A gain scales the channel, and unity leaves it untouched.
    /// trace: FR-RX-VOL-01
    #[test]
    fn fr_rx_vol_01_gain_scales_one_channel() {
        let mut ch = [1.0f32, -0.5, 0.25];
        apply_rx_gains(&mut ch, 1.0);
        assert_eq!(ch, [1.0, -0.5, 0.25], "unity must not alter the samples");

        apply_rx_gains(&mut ch, 0.5);
        assert_eq!(ch, [0.5, -0.25, 0.125]);
    }

    /// Zero mutes that receiver completely — the point of a per-receiver
    /// control is being able to silence one and keep the other.
    /// trace: FR-RX-VOL-01
    #[test]
    fn fr_rx_vol_01_zero_mutes_only_that_receiver() {
        let mut main = [0.8f32, -0.8];
        let mut sub = [0.4f32, -0.4];
        apply_rx_gains(&mut main, 0.0);
        apply_rx_gains(&mut sub, 1.0);
        assert_eq!(main, [0.0, 0.0], "muted receiver is silent");
        assert_eq!(sub, [0.4, -0.4], "the other receiver is untouched");
    }
}

#[cfg(feature = "device")]
pub mod device;
#[cfg(feature = "device")]
pub use device::{input_device_names, output_device_names, AudioInput, AudioOutput};
