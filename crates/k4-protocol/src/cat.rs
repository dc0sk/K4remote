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

// --- K4 configuration-screen commands (FR-UI-19 screens) --------------------
// Syntax per K4 Programmer's Reference rev. D12, cross-checked vs QK4
// (see docs/concept/k4-screens.md §3.2). Confirm the ranges marked "verify"
// against a real radio (ASM-05).

/// Format one graphic-EQ band gain as a signed 3-char field (`+00`, `-16`).
fn eq_field(db: i8) -> String {
    format!("{:+03}", db.clamp(-16, 16))
}

/// Set the 8-band **RX graphic equalizer** (`RE`): bands
/// 100/200/400/800/1200/1600/2400/3200 Hz, each −16..+16 dB.
///
/// Example: all-flat `[0;8]` → `"RE+00+00+00+00+00+00+00+00;"`.
///
/// trace: FR-EQ-01
pub fn set_rx_eq(bands: [i8; 8]) -> String {
    let mut s = String::from("RE");
    for b in bands {
        s.push_str(&eq_field(b));
    }
    s.push(';');
    s
}

/// Flatten the RX graphic equalizer (`REF`; all bands → 0 dB).
///
/// trace: FR-EQ-01
pub fn rx_eq_flat() -> &'static str {
    "REF;"
}

/// Set the 8-band **TX graphic equalizer** (`TE`), same bands/range as RX EQ.
///
/// trace: FR-EQ-01
pub fn set_tx_eq(bands: [i8; 8]) -> String {
    let mut s = String::from("TE");
    for b in bands {
        s.push_str(&eq_field(b));
    }
    s.push(';');
    s
}

/// Configure the **CW keyer** (`KP`): iambic mode (A/B), paddle normal/reversed,
/// and weight (90–125, i.e. 0.90–1.25).
///
/// Example: `(false, false, 110)` → `"KPAN110;"`.
///
/// trace: FR-KEY-01
pub fn set_keyer(iambic_b: bool, paddle_reverse: bool, weight: u16) -> String {
    let i = if iambic_b { 'B' } else { 'A' };
    let o = if paddle_reverse { 'R' } else { 'N' };
    format!("KP{i}{o}{:03};", weight.clamp(90, 125))
}

/// Set the **keyer speed** in WPM (`KS`, 8–100).
///
/// trace: FR-KEY-01
pub fn set_keyer_speed(wpm: u8) -> String {
    format!("KS{:03};", wpm.clamp(8, 100))
}

/// Select the **mic input** source (`MI`): 0 front, 1 rear, 2 line, 3 front+line,
/// 4 rear+line.
///
/// trace: FR-AUD-CFG-01
pub fn set_mic_input(source: u8) -> String {
    format!("MI{};", source.min(4))
}

/// Set the **mic gain** (`MG`, 0–80).
///
/// trace: FR-AUD-CFG-01
pub fn set_mic_gain(gain: u8) -> String {
    format!("MG{:03};", gain.min(80))
}

/// Configure **mic setup** (`MS`): front preamp (0/1/2 = 0/10/20 dB), front bias,
/// front UP/DN buttons, rear preamp (0/1 = 0/14 dB), rear bias.
///
/// trace: FR-AUD-CFG-01
pub fn set_mic_setup(
    front_preamp: u8,
    front_bias: bool,
    front_buttons: bool,
    rear_preamp: u8,
    rear_bias: bool,
) -> String {
    format!(
        "MS{}{}{}{}{};",
        front_preamp.min(2),
        front_bias as u8,
        front_buttons as u8,
        rear_preamp.min(1),
        rear_bias as u8,
    )
}

/// Configure **line-in** (`LI`): USB-B level, LINE-IN jack level, and source
/// (`false` = USB sound card, `true` = LINE IN jack). Level ranges: verify.
///
/// trace: FR-AUD-CFG-01
pub fn set_line_in(usb_level: u16, jack_level: u16, use_jack: bool) -> String {
    format!(
        "LI{:03}{:03}{};",
        usb_level.min(999),
        jack_level.min(999),
        use_jack as u8
    )
}

/// Configure **line-out** (`LO`): left/right levels (0–40) and gang mode
/// (`true` = right follows left).
///
/// trace: FR-AUD-CFG-01
pub fn set_line_out(left: u8, right: u8, gang: bool) -> String {
    format!("LO{:03}{:03}{};", left.min(40), right.min(40), gang as u8)
}

/// Select a band directly by band number (`BN`, 00 = 160 m … 10 = 6 m,
/// 16–25 = transverter).
///
/// trace: FR-VFO-04
pub fn set_band(band: u8) -> String {
    format!("BN{:02};", band.min(25))
}

/// Select a band on the sub receiver / VFO B (`BN$`).
///
/// trace: FR-VFO-04
pub fn set_band_sub(band: u8) -> String {
    format!("BN${:02};", band.min(25))
}

/// Recall the next **band-stacking register** on the current band (`BN^`).
///
/// trace: FR-VFO-04
pub fn band_stack_next() -> &'static str {
    "BN^;"
}

/// Select a **transverter band** (`XV`, 01–12; D12 shows a 2-digit field).
///
/// trace: FR-VFO-04
pub fn set_transverter_band(n: u8) -> String {
    format!("XV{:02};", n.clamp(1, 12))
}

/// Set **VOX** on/off for a transmit mode (`VX`): `mode` = `'C'` CW/direct-data,
/// `'V'` voice, `'D'` AF-data.
///
/// Example: `set_vox('V', true)` → `"VXV1;"`.
///
/// trace: FR-VOX-01
pub fn set_vox(mode: char, on: bool) -> String {
    format!("VX{mode}{};", on as u8)
}

/// Send a **CW/DATA text message** (`KY`, up to 60 chars, blank message-type).
/// Prosigns are entered as `( )/+ /= /% /*` etc. per the Programmer's Reference.
///
/// Example: `send_text("CQ CQ")` → `"KY CQ CQ;"`.
///
/// trace: FR-TX-MSG-01
pub fn send_text(text: &str) -> String {
    let t: String = text.chars().take(60).collect();
    format!("KY {t};")
}

/// Set the panadapter **display mode** (`#DPM`): 0 single-A, 1 single-B, 2 dual.
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_mode(mode: u8) -> String {
    format!("#DPM{};", mode.min(2))
}

/// Set the panadapter **span** in Hz (`#SPN`, 6000–368000).
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_span_hz(hz: u32) -> String {
    format!("#SPN{};", hz.clamp(6000, 368_000))
}

/// Set the panadapter **reference level** (`#REF`, −200..+60).
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_ref(db: i16) -> String {
    format!("#REF{};", db.clamp(-200, 60))
}

/// Set the panadapter **vertical scale** (`#SCL`, 10–150).
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_scale(scale: u8) -> String {
    format!("#SCL{};", scale.clamp(10, 150))
}

/// Set panadapter **averaging** (`#AVG`, 1–20).
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_average(n: u8) -> String {
    format!("#AVG{:02};", n.clamp(1, 20))
}

/// Set panadapter **peak-hold** on/off (`#PKM`).
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_peak(on: bool) -> String {
    format!("#PKM{};", on as u8)
}

/// Set panadapter **fixed-tune** mode on/off (`#FXT`).
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_fixed(on: bool) -> String {
    format!("#FXT{};", on as u8)
}

/// **Freeze**/run the panadapter + waterfall (`#FRZ`).
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_freeze(on: bool) -> String {
    format!("#FRZ{};", on as u8)
}

/// Set the **waterfall palette** (`#WFC`): 0 gray, 1 color, 2 teal, 3 blue,
/// 4 sepia.
///
/// trace: FR-PAN-CTL-01
pub fn set_waterfall_palette(palette: u8) -> String {
    format!("#WFC{};", palette.min(4))
}

/// Set the **waterfall height** as a percentage (`#WFH`, 0–100).
///
/// trace: FR-PAN-CTL-01
pub fn set_waterfall_height(pct: u8) -> String {
    format!("#WFH{:03};", pct.min(100))
}

/// Set the panadapter **noise blanker** mode (`#NB`): 0 off, 1 on, 2 auto.
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_nb(mode: u8) -> String {
    format!("#NB{};", mode.min(2))
}

/// Set the panadapter noise-blanker **level** (`#NBL`, 0–14).
///
/// trace: FR-PAN-CTL-01
pub fn set_pan_nb_level(level: u8) -> String {
    format!("#NBL{};", level.min(14))
}

/// **Copy/swap** VFOs (`AB`): 0 A→B freq, 1 B→A freq, 2 swap freq, 3 A→B all,
/// 4 B→A all, 5 swap all.
///
/// trace: FR-VFO-07
pub fn vfo_copy_swap(op: u8) -> String {
    format!("AB{};", op.min(5))
}

/// Select the **transmit antenna** (`AN`, 1–3).
///
/// trace: FR-ANT-01
pub fn set_tx_antenna(n: u8) -> String {
    format!("AN{};", n.clamp(1, 3))
}

/// Select the **RX antenna** for the main receiver (`AR`, 0–7).
///
/// trace: FR-ANT-01
pub fn set_rx_antenna(n: u8) -> String {
    format!("AR{};", n.min(7))
}

/// Select the **RX antenna** for the sub receiver (`AR$`, 0–7).
///
/// trace: FR-ANT-01
pub fn set_rx_antenna_sub(n: u8) -> String {
    format!("AR${};", n.min(7))
}

/// **Open** a configuration-menu item on the radio screen by id (`MO`, 4-digit).
///
/// trace: FR-MENU-01
pub fn menu_open(id: u16) -> String {
    format!("MO{id:04};")
}

/// Query a menu item's **definition** (`MEDF`, 4-digit id).
///
/// trace: FR-MENU-01
pub fn menu_query_def(id: u16) -> String {
    format!("MEDF{id:04};")
}

/// **Set** a menu item's value (`ME<id>.<value>`); `value` is the item's
/// already-formatted value field.
///
/// trace: FR-MENU-01
pub fn menu_set(id: u16, value: &str) -> String {
    format!("ME{id:04}.{value};")
}

/// Emulate a front-panel **switch** press by its code (`SW`, 1–3 digits). Reaches
/// functions that have no dedicated CAT command — e.g. the quick-memory keys
/// M1–M4 (tap = recall/play, hold codes = store) and PF1–PF4.
///
/// Example: `switch(17)` → `"SW17;"` (tap M1).
///
/// trace: FR-SW-01
pub fn switch(code: u16) -> String {
    format!("SW{code};")
}
