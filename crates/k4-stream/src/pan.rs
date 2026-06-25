//! Panadapter / spectrum packet decoder (payload type `0x02`). Phase-2 feature,
//! but the codec is pure and testable now (FR-PAN-01).
//!
//! Layout (R-EXT-01): type0, ver1, seq2, pan_type3, receiver4, data_len5(u16 LE),
//! reserved7(4), center_freq11(i64 LE Hz), sample_rate19(i32 LE), noise_floor23
//! (i32 LE, ÷10 = dB), bins@27 — one byte per bin where `dBm = byte − 146`.

/// PAN payload type byte.
pub const PAN_TYPE: u8 = 0x02;
/// Header length before the bin data.
pub const PAN_HEADER_SIZE: usize = 27;
/// Per-bin dBm offset: `dBm = raw_byte − K4_DBM_OFFSET`.
pub const K4_DBM_OFFSET: f32 = 146.0;

/// A decoded spectrum frame: metadata plus per-bin levels in dBm.
#[derive(Debug, Clone, PartialEq)]
pub struct PanFrame {
    /// `0` = Main (VFO A), `1` = Sub (VFO B).
    pub receiver: u8,
    /// Center frequency, Hz.
    pub center_freq_hz: i64,
    /// Sample-rate tier (the displayed span is `sample_rate × 1000` Hz).
    pub sample_rate: i32,
    /// Noise floor, dB.
    pub noise_floor_db: f32,
    /// Per-bin levels, dBm.
    pub bins_dbm: Vec<f32>,
}

impl PanFrame {
    /// Decode a PAN payload (the body of a `0x02` frame). Returns `None` if the
    /// payload is not PAN or is shorter than the header.
    ///
    /// trace: FR-PAN-01
    pub fn decode(payload: &[u8]) -> Option<Self> {
        if payload.len() < PAN_HEADER_SIZE || payload[0] != PAN_TYPE {
            return None;
        }
        let center_freq_hz = i64::from_le_bytes(payload[11..19].try_into().ok()?);
        let sample_rate = i32::from_le_bytes(payload[19..23].try_into().ok()?);
        let noise_raw = i32::from_le_bytes(payload[23..27].try_into().ok()?);
        let bins_dbm = payload[PAN_HEADER_SIZE..]
            .iter()
            .map(|&b| b as f32 - K4_DBM_OFFSET)
            .collect();
        Some(PanFrame {
            receiver: payload[4],
            center_freq_hz,
            sample_rate,
            noise_floor_db: noise_raw as f32 / 10.0,
            bins_dbm,
        })
    }

    /// Displayed span in Hz (`sample_rate × 1000`).
    pub fn span_hz(&self) -> i64 {
        self.sample_rate as i64 * 1000
    }
}
