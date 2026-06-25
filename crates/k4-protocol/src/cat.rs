//! CAT command encoding/decoding (subset; grows per requirement).
//! Source: K4 Programmer's Reference rev. D12.

use crate::frame::PayloadType;

/// Max buffer for the serial line decoder before reset (malformed-input guard).
const MAX_LINE_BUFFER: usize = 64 * 1024;

/// Incremental decoder for **raw serial CAT** (USB virtual COM / RS232): the K4
/// emits `;`-terminated ASCII with no binary framing. Accumulates bytes and
/// yields complete commands, leaving any partial tail buffered (FR-CAT-02).
#[derive(Debug, Default)]
pub struct LineDecoder {
    buf: Vec<u8>,
}

impl LineDecoder {
    /// Create an empty decoder.
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Append `data` and return every complete `;`-terminated command (each
    /// including its trailing `;`), leaving any partial command buffered.
    pub fn push(&mut self, data: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(data);
        if self.buf.len() > MAX_LINE_BUFFER {
            self.buf.clear();
            return Vec::new();
        }
        let mut out = Vec::new();
        let mut start = 0;
        for i in 0..self.buf.len() {
            if self.buf[i] == b';' {
                out.push(self.buf[start..=i].iter().map(|&b| b as char).collect());
                start = i + 1;
            }
        }
        if start > 0 {
            self.buf.drain(..start);
        }
        out
    }
}

/// Build the (unframed) payload for a CAT command: the CAT type byte, two zero
/// bytes (version/reserved), then the ASCII command including its `;`.
/// Wrap the result with [`crate::frame::encode_frame`] for the wire.
///
/// trace: FR-STREAM-02, FR-CAT-01
pub fn encode_cat_payload(command: &str) -> Vec<u8> {
    let mut payload = Vec::with_capacity(3 + command.len());
    payload.extend_from_slice(&[0x00, 0x00, 0x00]); // type=CAT, version, reserved
    payload.extend_from_slice(command.as_bytes());
    payload
}

/// Extract the ASCII text from a CAT payload (type `0x00`, text at offset 3).
/// Returns `None` if the payload is not CAT or is too short. Decoded as Latin-1
/// (the K4 uses 7-bit ASCII for CAT).
///
/// trace: FR-STREAM-02, FR-CAT-01
pub fn decode_cat_text(payload: &[u8]) -> Option<String> {
    if payload.len() <= 3 || PayloadType::from_byte(payload[0]) != PayloadType::Cat {
        return None;
    }
    Some(payload[3..].iter().map(|&b| b as char).collect())
}

/// Encode a VFO A set-frequency command in the canonical 11-digit Hz form.
///
/// Example: `14_074_000` → `"FA00014074000;"`.
///
/// trace: FR-VFO-01
pub fn set_vfo_a_hz(hz: u64) -> String {
    format!("FA{hz:011};")
}

/// Encode a VFO B set-frequency command (canonical 11-digit Hz).
///
/// trace: FR-VFO-01
pub fn set_vfo_b_hz(hz: u64) -> String {
    format!("FB{hz:011};")
}

/// Set the operating mode for main RX (`MD`, digit 1–9; see [`crate::Mode`]).
///
/// trace: FR-MODE-01
pub fn set_mode(digit: u8) -> String {
    format!("MD{digit};")
}

/// Set the operating mode for sub RX (`MD$`).
///
/// trace: FR-MODE-01
pub fn set_mode_sub(digit: u8) -> String {
    format!("MD${digit};")
}

/// Set receive bandwidth in Hz (`BW`; the wire value is ×10 Hz).
///
/// Example: `2700` → `"BW0270;"`.
///
/// trace: FR-MODE-02
pub fn set_bandwidth_hz(hz: u32) -> String {
    format!("BW{:04};", hz / 10)
}

/// Set AF gain, 0–60 (`AG`).
///
/// trace: FR-RX-01
pub fn set_af_gain(level: u8) -> String {
    format!("AG{:03};", level.min(60))
}

/// Set RF-gain attenuation, 0–60 dB (`RG`, encoded as `-nn`).
///
/// trace: FR-RX-01
pub fn set_rf_gain(db: u8) -> String {
    format!("RG-{:02};", db.min(60))
}

/// Set the RX attenuator: `db` ∈ {0,3,6,9,12,15,18,21}, on/off (`RA`).
///
/// Example: `(12, true)` → `"RA121;"`.
///
/// trace: FR-RX-02
pub fn set_attenuator(db: u8, on: bool) -> String {
    format!("RA{}{};", db, on as u8)
}

/// Select the next-higher band (`BN+`).
///
/// trace: FR-VFO-04
pub fn band_up() -> &'static str {
    "BN+;"
}

/// Select the next-lower band (`BN-`).
///
/// trace: FR-VFO-04
pub fn band_down() -> &'static str {
    "BN-;"
}

/// Set split on/off (`FT`).
///
/// trace: FR-VFO-06
pub fn set_split(on: bool) -> String {
    format!("FT{};", on as u8)
}

/// Set AGC mode (`GT`): 0 = off, 1 = slow, 2 = fast.
///
/// trace: FR-RX-03
pub fn set_agc(mode: u8) -> String {
    format!("GT{mode};")
}

/// Set the noise blanker on/off (`NB`, alternate single-digit form).
///
/// trace: FR-RX-04
pub fn set_nb(on: bool) -> String {
    format!("NB{};", on as u8)
}

/// Set LMS noise reduction (`NR`): `level` 0–10, `mode` 0/1/2 (off/on/off-last).
///
/// Example: `(2, 1)` → `"NR021;"`.
///
/// trace: FR-RX-04
pub fn set_nr(level: u8, mode: u8) -> String {
    format!("NR{:02}{};", level.min(10), mode)
}

/// Set the preamp (`PA`): `level` 0–3, on/off.
///
/// trace: FR-RX-04
pub fn set_preamp(level: u8, on: bool) -> String {
    format!("PA{}{};", level.min(3), on as u8)
}

/// Set RIT on/off (`RT`).
///
/// trace: FR-VFO-05
pub fn set_rit(on: bool) -> String {
    format!("RT{};", on as u8)
}

/// Set XIT on/off (`XT`).
///
/// trace: FR-VFO-05
pub fn set_xit(on: bool) -> String {
    format!("XT{};", on as u8)
}

/// Clear the RIT/XIT offset (`RC`; SET only).
///
/// trace: FR-VFO-05
pub fn clear_rit_xit() -> &'static str {
    "RC;"
}
