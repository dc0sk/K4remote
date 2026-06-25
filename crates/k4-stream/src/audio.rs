//! Audio packet codec (payload type `0x01`).
//!
//! Layout (R-EXT-01): `[0x01][ver][seq][mode][frame_size u16 LE][sr_code][data…]`.
//! RX is 12 kHz stereo Opus (left = Main, right = Sub); TX is 12 kHz mono. The
//! `data` bytes are Opus or raw PCM per `mode` — decoding them is the audio
//! engine's job (`k4-audio`); this module only frames/deframes the packet.

/// Audio encode mode (matches the `EM` command; FR-AUD-ENC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeMode {
    /// `0` — raw 32-bit signed PCM.
    Raw32,
    /// `1` — raw 16-bit signed PCM.
    Raw16,
    /// `2` — Opus, 16-bit samples.
    OpusInt,
    /// `3` — Opus, float samples (default).
    OpusFloat,
    /// Any other/unknown mode byte.
    Other(u8),
}

impl EncodeMode {
    /// Classify a mode byte.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0 => EncodeMode::Raw32,
            1 => EncodeMode::Raw16,
            2 => EncodeMode::OpusInt,
            3 => EncodeMode::OpusFloat,
            other => EncodeMode::Other(other),
        }
    }
    /// The wire byte for this mode.
    pub fn to_byte(self) -> u8 {
        match self {
            EncodeMode::Raw32 => 0,
            EncodeMode::Raw16 => 1,
            EncodeMode::OpusInt => 2,
            EncodeMode::OpusFloat => 3,
            EncodeMode::Other(b) => b,
        }
    }
    /// Whether this mode is Opus-encoded.
    pub fn is_opus(self) -> bool {
        matches!(self, EncodeMode::OpusInt | EncodeMode::OpusFloat)
    }
}

/// Audio payload type byte.
pub const AUDIO_TYPE: u8 = 0x01;
/// Header length before the audio data.
pub const AUDIO_HEADER_SIZE: usize = 7;
/// Sample-rate code for 12 kHz (the only value the K4 currently uses).
pub const SAMPLE_RATE_CODE_12K: u8 = 0x00;

/// A decoded audio packet borrowing its payload's data bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioPacket<'a> {
    /// Protocol version byte.
    pub version: u8,
    /// Wrapping 0–255 sequence number (ordering; FR-AUD-05).
    pub sequence: u8,
    /// Encode mode of `data`.
    pub mode: EncodeMode,
    /// Samples per channel in this frame (matches the `SL` tier).
    pub frame_samples: u16,
    /// Sample-rate code (`0` = 12 kHz).
    pub sample_rate_code: u8,
    /// Encoded audio bytes (Opus or raw PCM per `mode`).
    pub data: &'a [u8],
}

impl<'a> AudioPacket<'a> {
    /// Decode an audio payload (the body of a `0x01` frame). Returns `None` if
    /// the payload is not audio or is shorter than the header.
    ///
    /// trace: FR-AUD-04
    pub fn decode(payload: &'a [u8]) -> Option<Self> {
        if payload.len() < AUDIO_HEADER_SIZE || payload[0] != AUDIO_TYPE {
            return None;
        }
        Some(AudioPacket {
            version: payload[1],
            sequence: payload[2],
            mode: EncodeMode::from_byte(payload[3]),
            frame_samples: u16::from_le_bytes([payload[4], payload[5]]),
            sample_rate_code: payload[6],
            data: &payload[AUDIO_HEADER_SIZE..],
        })
    }

    /// Build a TX audio payload (wrap with `k4_protocol::frame::encode_frame`).
    /// Always emits version `0x01` and the 12 kHz sample-rate code.
    ///
    /// trace: FR-AUD-TX-01, FR-AUD-04
    pub fn encode(sequence: u8, mode: EncodeMode, frame_samples: u16, data: &[u8]) -> Vec<u8> {
        let mut payload = Vec::with_capacity(AUDIO_HEADER_SIZE + data.len());
        payload.push(AUDIO_TYPE);
        payload.push(0x01); // version
        payload.push(sequence);
        payload.push(mode.to_byte());
        payload.extend_from_slice(&frame_samples.to_le_bytes());
        payload.push(SAMPLE_RATE_CODE_12K);
        payload.extend_from_slice(data);
        payload
    }
}
