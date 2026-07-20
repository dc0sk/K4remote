//! Panadapter / spectrum packet decoder (payload type `0x02`). Phase-2 feature,
//! but the codec is pure and testable now (FR-PAN-01).
//!
//! Layout (R-EXT-01): type0, ver1, seq2, pan_type3, receiver4, data_len5(u16 LE),
//! reserved7(4), center_freq11(i64 LE Hz), sample_rate19(i32 LE), noise_floor23
//! (i32 LE, ÷10 = dB), bins@27 — one byte per bin where `dBm = byte − 146`.

/// PAN payload type byte.
pub const PAN_TYPE: u8 = 0x02;
/// Mini-pan payload type byte: a wide-span overview strip. **Its header is a
/// different, much shorter layout than the main pan's** — see
/// [`MINI_PAN_HEADER_SIZE`].
pub const MINI_PAN_TYPE: u8 = 0x03;
/// Header length before the bin data in a main-pan (`0x02`) payload.
pub const PAN_HEADER_SIZE: usize = 27;
/// Header length before the bin data in a mini-pan (`0x03`) payload.
///
/// MiniPAN carries only `type, version, sequence, reserved, receiver` and then
/// its bins (`R-EXT-01`) — no `data_len`, centre frequency, sample rate or
/// noise floor. Decoding it with the 27-byte main-pan header consumes 22 bins
/// as phantom metadata and shifts the rest, or rejects the frame outright when
/// it is shorter than 27 bytes.
pub const MINI_PAN_HEADER_SIZE: usize = 5;
/// Per-bin dBm offset: `dBm = raw_byte − K4_DBM_OFFSET`.
pub const K4_DBM_OFFSET: f32 = 146.0;

/// A decoded spectrum frame: metadata plus per-bin levels in dBm.
#[derive(Debug, Clone, PartialEq)]
pub struct PanFrame {
    /// `0` = Main (VFO A), `1` = Sub (VFO B).
    pub receiver: u8,
    /// Centre frequency, Hz. Always `0` on a mini-pan frame, which carries no
    /// geometry — see [`MINI_PAN_HEADER_SIZE`].
    pub center_freq_hz: i64,
    /// Sample-rate tier (the streamed span is `sample_rate × 1000` Hz).
    /// Always `0` on a mini-pan frame, which carries no geometry.
    pub sample_rate: i32,
    /// Noise floor, dB. Always `0.0` on a mini-pan frame.
    pub noise_floor_db: f32,
    /// Per-bin levels, dBm.
    pub bins_dbm: Vec<f32>,
    /// `true` if this is a mini-pan (`0x03`) frame rather than the main pan.
    pub mini: bool,
}

impl PanFrame {
    /// Decode a pan payload: either a `0x02` main pan or a `0x03` mini-pan.
    ///
    /// The two do **not** share a header layout. The main pan carries geometry
    /// (centre frequency, sample rate, noise floor) in a 27-byte header; the
    /// mini-pan carries none of it and its bins start at offset 5
    /// (`R-EXT-01`). Geometry fields are therefore `0` on a mini-pan frame.
    ///
    /// Returns `None` if the payload is neither type or is shorter than the
    /// header for its type.
    ///
    /// trace: FR-PAN-01, FR-UI-14
    pub fn decode(payload: &[u8]) -> Option<Self> {
        let mini = match *payload.first()? {
            PAN_TYPE => false,
            MINI_PAN_TYPE => true,
            _ => return None,
        };
        let header = if mini {
            MINI_PAN_HEADER_SIZE
        } else {
            PAN_HEADER_SIZE
        };
        if payload.len() < header {
            return None;
        }
        // Only the main pan carries geometry; the mini-pan header ends at the
        // receiver byte.
        let (center_freq_hz, sample_rate, noise_raw) = if mini {
            (0, 0, 0)
        } else {
            (
                i64::from_le_bytes(payload[11..19].try_into().ok()?),
                i32::from_le_bytes(payload[19..23].try_into().ok()?),
                i32::from_le_bytes(payload[23..27].try_into().ok()?),
            )
        };
        let bins_dbm = payload[header..]
            .iter()
            .map(|&b| b as f32 - K4_DBM_OFFSET)
            .collect();
        Some(PanFrame {
            receiver: payload[4],
            center_freq_hz,
            sample_rate,
            noise_floor_db: noise_raw as f32 / 10.0,
            bins_dbm,
            mini,
        })
    }

    /// **Tier** span in Hz (`sample_rate × 1000`) — the width the radio is
    /// streaming, which is *not* the displayed span.
    ///
    /// `#SPN` selects a narrower display window and the client shows the centre
    /// crop of these bins (`R-EXT-01`); see
    /// [`crate::render::crop_to_span`]. Treating this as the display span
    /// scales the frequency axis and click-to-QSY by `tier / #SPN`.
    pub fn span_hz(&self) -> i64 {
        self.sample_rate as i64 * 1000
    }
}
