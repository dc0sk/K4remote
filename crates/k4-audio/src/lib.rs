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

/// Gain at the top of the master volume control, as a linear factor (+24 dB).
///
/// The K4's streamed RX audio can arrive far below full scale — measured at
/// about -45 dBFS on one radio — and `AG` on the radio does not always bring it
/// up far enough for a noisy room or outdoor use. Digital gain amplifies the
/// stream's own noise equally, so this is headroom for a quiet source rather
/// than a substitute for setting the level at the radio.
pub const MAX_GAIN: f32 = 16.0;

/// Convert a volume **control position** (0–100 %) to a playback gain.
///
/// The control reads 0–100 % because that is what a volume control means to
/// an operator; a slider whose maximum says "400 %" invites the question of
/// what 100 % was supposed to be. The range lives in the curve instead.
///
/// Cubic, so the useful settings are spread across the travel rather than
/// bunched at the bottom: **unity sits near 40 %**, below that attenuates, and
/// the top of the travel reaches [`MAX_GAIN`]. This is the same shape a
/// physical audio taper has, and roughly matches loudness perception.
///
/// trace: FR-AUD-LVL-01
pub fn gain_from_level(level_pct: u8) -> f32 {
    let s = f32::from(level_pct.min(100)) / 100.0;
    MAX_GAIN * s * s * s
}

/// The control position that best reproduces an existing linear `gain` — the
/// inverse of [`gain_from_level`], for migrating settings saved when the
/// sliders stored a raw multiplier.
///
/// trace: FR-AUD-LVL-01
pub fn level_from_gain(gain: f32) -> u8 {
    let g = (gain / MAX_GAIN).clamp(0.0, 1.0);
    (g.cbrt() * 100.0).round().clamp(0.0, 100.0) as u8
}

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

#[cfg(test)]
mod volume_curve_tests {
    use super::*;

    /// The ends of the travel mean what they say.
    /// trace: FR-AUD-LVL-01
    #[test]
    fn fr_aud_lvl_01_curve_endpoints() {
        assert_eq!(gain_from_level(0), 0.0, "0 % is silent");
        assert!(
            (gain_from_level(100) - MAX_GAIN).abs() < 1e-6,
            "100 % is the top of the range"
        );
        assert_eq!(gain_from_level(200), gain_from_level(100), "clamped");
    }

    /// Unity gain sits partway up the travel, so there is room to cut as well
    /// as boost — the control is a volume knob, not a boost-only slider.
    /// trace: FR-AUD-LVL-01
    #[test]
    fn fr_aud_lvl_01_unity_is_reachable_mid_travel() {
        let unity = (1..=100).find(|&p| gain_from_level(p) >= 1.0).unwrap();
        assert!(
            (30..=50).contains(&unity),
            "unity at {unity} % — expected it around the middle-low of the travel"
        );
        assert!(gain_from_level(unity - 5) < 1.0, "below unity attenuates");
    }

    /// Monotonic: turning it up never makes it quieter.
    /// trace: FR-AUD-LVL-01
    #[test]
    fn fr_aud_lvl_01_curve_is_monotonic() {
        for p in 0..100u8 {
            assert!(
                gain_from_level(p + 1) > gain_from_level(p),
                "{p} % -> {} % did not increase",
                p + 1
            );
        }
    }

    /// A setting saved under the old raw-multiplier scheme maps back to the
    /// same loudness, so upgrading does not change how loud the radio is.
    /// trace: FR-AUD-LVL-01
    #[test]
    fn fr_aud_lvl_01_migration_preserves_loudness() {
        for gain in [0.0f32, 0.5, 1.0, 1.6, 2.0, 4.0] {
            let level = level_from_gain(gain);
            let back = gain_from_level(level);
            assert!(
                (back - gain).abs() < 0.15 * gain.max(0.1),
                "gain {gain} -> {level} % -> {back}"
            );
        }
        assert_eq!(level_from_gain(99.0), 100, "above the range clamps to full");
    }
}
