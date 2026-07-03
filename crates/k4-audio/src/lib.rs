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

#[cfg(feature = "device")]
pub mod device;
#[cfg(feature = "device")]
pub use device::{input_device_names, output_device_names, AudioInput, AudioOutput};
