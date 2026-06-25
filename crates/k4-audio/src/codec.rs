//! Opus codec wrappers for K4 audio.
//!
//! RX is 12 kHz **stereo** Opus (left = Main, right = Sub); TX is 12 kHz **mono**.
//! Application profile is VOIP. Valid frame sizes at 12 kHz are 30/60/120/240/
//! 480/720 samples per channel (2.5–60 ms); 240 = the default 20 ms tier.

use opus::{Application, Channels, Decoder, Encoder};

/// Re-exported Opus error type.
pub use opus::Error as OpusError;

/// K4 audio sample rate (Hz).
pub const SAMPLE_RATE: u32 = 12_000;
/// Max decoded samples per channel we allocate for (120 ms @ 12 kHz).
const MAX_SAMPLES_PER_CHANNEL: usize = 1_440;
/// Max encoded packet size we allocate for.
const MAX_PACKET: usize = 4_000;

fn channel_count(channels: Channels) -> usize {
    match channels {
        Channels::Mono => 1,
        Channels::Stereo => 2,
    }
}

/// Opus decoder. Produces interleaved float PCM.
pub struct OpusDecoder {
    inner: Decoder,
    channels: usize,
}

impl OpusDecoder {
    /// New decoder with the given channel layout.
    pub fn new(channels: Channels) -> Result<Self, OpusError> {
        Ok(Self {
            inner: Decoder::new(SAMPLE_RATE, channels)?,
            channels: channel_count(channels),
        })
    }
    /// Stereo decoder for RX from the radio (L = Main, R = Sub).
    pub fn rx() -> Result<Self, OpusError> {
        Self::new(Channels::Stereo)
    }
    /// Mono decoder.
    pub fn mono() -> Result<Self, OpusError> {
        Self::new(Channels::Mono)
    }

    /// Decode one Opus frame into interleaved float PCM
    /// (length = samples_per_channel × channels).
    ///
    /// trace: FR-AUD-04, FR-AUD-RX-01
    pub fn decode_float(&mut self, opus_frame: &[u8]) -> Result<Vec<f32>, OpusError> {
        let mut out = vec![0f32; MAX_SAMPLES_PER_CHANNEL * self.channels];
        let per_channel = self.inner.decode_float(opus_frame, &mut out, false)?;
        out.truncate(per_channel * self.channels);
        Ok(out)
    }
}

/// Opus encoder (VOIP profile). Produces an Opus packet from interleaved PCM.
pub struct OpusEncoder {
    inner: Encoder,
}

impl OpusEncoder {
    /// New encoder with the given channel layout.
    pub fn new(channels: Channels) -> Result<Self, OpusError> {
        Ok(Self {
            inner: Encoder::new(SAMPLE_RATE, channels, Application::Voip)?,
        })
    }
    /// Mono encoder for TX to the radio.
    pub fn mono() -> Result<Self, OpusError> {
        Self::new(Channels::Mono)
    }
    /// Stereo encoder.
    pub fn stereo() -> Result<Self, OpusError> {
        Self::new(Channels::Stereo)
    }

    /// Encode one interleaved float PCM frame into an Opus packet. The input
    /// length must be a valid Opus frame size (× channels).
    ///
    /// trace: FR-AUD-TX-01, FR-AUD-04
    pub fn encode_float(&mut self, pcm: &[f32]) -> Result<Vec<u8>, OpusError> {
        let mut out = vec![0u8; MAX_PACKET];
        let len = self.inner.encode_float(pcm, &mut out)?;
        out.truncate(len);
        Ok(out)
    }
}
