//! K4 binary frame envelope (FR-STREAM-01/02/03).
//!
//! Every byte the K4/0 server emits — CAT ASCII, Opus audio, PAN/MiniPAN
//! spectrum — is wrapped in a fixed frame so one TCP read can contain any mix:
//!
//! ```text
//! [ START 4B ][ payload length 4B BE ][ payload ][ END 4B ]
//!   FE FD FC FB                                    FB FC FD FE   (mirror image)
//! ```
//!
//! The first payload byte is the [`PayloadType`]. Source: R-EXT-01.

/// Frame start marker.
pub const START_MARKER: [u8; 4] = [0xFE, 0xFD, 0xFC, 0xFB];
/// Frame end marker (mirror image of [`START_MARKER`] so corruption can't alias).
pub const END_MARKER: [u8; 4] = [0xFB, 0xFC, 0xFD, 0xFE];

/// Maximum decoder buffer before it is reset, guarding against malformed data
/// (FR-STREAM-03).
pub const MAX_BUFFER_SIZE: usize = 1024 * 1024;

/// Payload kind, taken from the first byte of a frame payload (FR-STREAM-02).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadType {
    /// `0x00` — CAT command/response (ASCII).
    Cat,
    /// `0x01` — audio (Opus/PCM).
    Audio,
    /// `0x02` — panadapter/spectrum (dB/bin).
    Pan,
    /// `0x03` — mini panadapter.
    MiniPan,
    /// Any other, unrecognised type byte (logged and skipped, never fatal).
    Unknown(u8),
}

impl PayloadType {
    /// Classify a payload's leading type byte.
    pub fn from_byte(b: u8) -> Self {
        match b {
            0x00 => PayloadType::Cat,
            0x01 => PayloadType::Audio,
            0x02 => PayloadType::Pan,
            0x03 => PayloadType::MiniPan,
            other => PayloadType::Unknown(other),
        }
    }
}

/// Length of the fixed header (START marker + u32 length) plus the END marker.
const FRAME_OVERHEAD: usize = START_MARKER.len() + 4 + END_MARKER.len();

/// Wrap a payload in a K4 binary frame.
///
/// trace: FR-STREAM-01
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(payload.len() + FRAME_OVERHEAD);
    frame.extend_from_slice(&START_MARKER);
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame.extend_from_slice(&END_MARKER);
    frame
}

/// Find the first occurrence of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Incremental decoder that reassembles complete frames across TCP read
/// boundaries and recovers from corruption (FR-STREAM-01/03).
#[derive(Debug, Default)]
pub struct FrameDecoder {
    // Retains any partial-frame tail between `push` calls, including 1–3 bytes
    // of a split START marker so sync is never lost across reads.
    buffer: Vec<u8>,
}

impl FrameDecoder {
    /// Create an empty decoder.
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    /// Append `data` and return every complete payload now decodable, in order.
    ///
    /// trace: FR-STREAM-01, FR-STREAM-03
    pub fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buffer.extend_from_slice(data);

        // Guard against unbounded growth from malformed input (FR-STREAM-03).
        if self.buffer.len() > MAX_BUFFER_SIZE {
            self.buffer.clear();
            return Vec::new();
        }

        let mut payloads = Vec::new();
        loop {
            // Locate the next frame start.
            let Some(start) = find(&self.buffer, &START_MARKER) else {
                // No marker yet: retain only enough tail to complete a marker
                // that may have been split across reads.
                let keep = START_MARKER.len().saturating_sub(1);
                if self.buffer.len() > keep {
                    self.buffer.drain(..self.buffer.len() - keep);
                }
                break;
            };

            // Discard anything before the marker.
            if start > 0 {
                self.buffer.drain(..start);
            }

            // Need the full header (markers + length) before reading length.
            let header = START_MARKER.len() + 4;
            if self.buffer.len() < header {
                break;
            }
            let len = u32::from_be_bytes([
                self.buffer[4],
                self.buffer[5],
                self.buffer[6],
                self.buffer[7],
            ]) as usize;

            let total = header + len + END_MARKER.len();
            if self.buffer.len() < total {
                break; // wait for the rest of the frame
            }

            // Verify the END marker; on mismatch skip past this START and resync.
            let end = &self.buffer[total - END_MARKER.len()..total];
            if end != END_MARKER {
                self.buffer.drain(..START_MARKER.len());
                continue;
            }

            payloads.push(self.buffer[header..header + len].to_vec());
            self.buffer.drain(..total);
        }
        payloads
    }
}
