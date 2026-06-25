//! Radio state model (ARC-05) and CAT-response application.
//!
//! [`RadioState`] is the single source of truth (ADR-04): it is updated by both
//! GET responses and unsolicited Auto-Info messages through the *same*
//! [`RadioState::apply_cat`] path (FR-CAT-06, FR-CAT-AI). The UI is a pure
//! projection of this struct.

/// Operating mode (`MD` command; K4 Programmer's Reference rev. D12:
/// 1=LSB, 2=USB, 3=CW, 4=FM, 5=AM, 6=DATA, 7=CW-REV, 9=DATA-REV; 0/8 = N/A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Lsb,
    Usb,
    Cw,
    Fm,
    Am,
    Data,
    CwRev,
    DataRev,
}

impl Mode {
    /// Parse a single ASCII `MD` digit.
    pub fn from_md_digit(d: u8) -> Option<Mode> {
        match d {
            b'1' => Some(Mode::Lsb),
            b'2' => Some(Mode::Usb),
            b'3' => Some(Mode::Cw),
            b'4' => Some(Mode::Fm),
            b'5' => Some(Mode::Am),
            b'6' => Some(Mode::Data),
            b'7' => Some(Mode::CwRev),
            b'9' => Some(Mode::DataRev),
            _ => None,
        }
    }
}

/// Authoritative radio state (subset; grows per requirement). `None` = unknown
/// (not yet reported by the radio).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RadioState {
    /// VFO A frequency, Hz.
    pub vfo_a_hz: Option<u64>,
    /// VFO B frequency, Hz.
    pub vfo_b_hz: Option<u64>,
    /// Main-RX operating mode.
    pub mode_a: Option<Mode>,
    /// Sub-RX operating mode.
    pub mode_b: Option<Mode>,
    /// Split on/off.
    pub split: Option<bool>,
    /// Transmit (`true`) vs receive (`false`).
    pub transmitting: Option<bool>,
    /// Main-RX S-meter, bar count 0–42 (`SM`).
    pub s_meter_bars: Option<u8>,
    /// Sub-RX S-meter, bar count 0–42 (`SM$`).
    pub s_meter_bars_sub: Option<u8>,
    /// Main-RX high-resolution S-meter, dBm (`SMH`).
    pub s_meter_dbm: Option<i32>,
    /// Sub-RX high-resolution S-meter, dBm (`SMH$`).
    pub s_meter_dbm_sub: Option<i32>,
    /// Receive bandwidth, Hz (`BW`).
    pub bandwidth_hz: Option<u32>,
    /// AF gain, 0–60 (`AG`).
    pub af_gain: Option<u8>,
    /// RF-gain attenuation, dB (`RG`).
    pub rf_gain_db: Option<u8>,
    /// RX attenuator value, dB (`RA`).
    pub atten_db: Option<u8>,
    /// RX attenuator on/off (`RA`).
    pub atten_on: Option<bool>,
    /// AGC mode: 0 = off, 1 = slow, 2 = fast (`GT`).
    pub agc_mode: Option<u8>,
    /// Noise blanker on/off (`NB`).
    pub nb_on: Option<bool>,
    /// Noise blanker level 0–15 (`NB`).
    pub nb_level: Option<u8>,
    /// LMS noise reduction on/off (`NR`).
    pub nr_on: Option<bool>,
    /// LMS noise reduction level 0–10 (`NR`).
    pub nr_level: Option<u8>,
    /// Preamp on/off (`PA`).
    pub preamp_on: Option<bool>,
    /// RIT on/off (`RT`).
    pub rit_on: Option<bool>,
    /// XIT on/off (`XT`).
    pub xit_on: Option<bool>,
}

impl RadioState {
    /// Empty state (everything unknown).
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one CAT response/command text (trailing `;` optional) to the state.
    /// Unknown or unparsable commands are ignored (tolerant parser, FR-CAT-04).
    /// Returns `true` if any field changed.
    ///
    /// trace: FR-CAT-05, FR-CAT-06, FR-CAT-AI
    pub fn apply_cat(&mut self, text: &str) -> bool {
        let before = self.clone();
        let cmd = text.strip_suffix(';').unwrap_or(text);

        // Longest-prefix-first so `MD$` is matched before `MD` (the `$` sub-RX
        // convention; see external-references R-EXT-01).
        if let Some(arg) = cmd.strip_prefix("FA") {
            if let Ok(hz) = arg.parse::<u64>() {
                self.vfo_a_hz = Some(hz);
            }
        } else if let Some(arg) = cmd.strip_prefix("FB") {
            if let Ok(hz) = arg.parse::<u64>() {
                self.vfo_b_hz = Some(hz);
            }
        } else if let Some(arg) = cmd.strip_prefix("MD$") {
            if let Some(d) = arg.bytes().next() {
                self.mode_b = Mode::from_md_digit(d);
            }
        } else if let Some(arg) = cmd.strip_prefix("MD") {
            if let Some(d) = arg.bytes().next() {
                self.mode_a = Mode::from_md_digit(d);
            }
        } else if let Some(arg) = cmd.strip_prefix("FT") {
            match arg.bytes().next() {
                Some(b'1') => self.split = Some(true),
                Some(b'0') => self.split = Some(false),
                _ => {}
            }
        // SMH* before SM* (longest prefix first) so the high-res form is not
        // shadowed by the bar form.
        } else if let Some(arg) = cmd.strip_prefix("SMH$") {
            if let Ok(v) = arg.parse::<i32>() {
                self.s_meter_dbm_sub = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("SMH") {
            if let Ok(v) = arg.parse::<i32>() {
                self.s_meter_dbm = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("SM$") {
            if let Ok(v) = arg.parse::<u8>() {
                self.s_meter_bars_sub = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("SM") {
            if let Ok(v) = arg.parse::<u8>() {
                self.s_meter_bars = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("BW") {
            if let Ok(v) = arg.parse::<u32>() {
                self.bandwidth_hz = Some(v * 10);
            }
        } else if let Some(arg) = cmd.strip_prefix("AG") {
            if let Ok(v) = arg.parse::<u8>() {
                self.af_gain = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("RG") {
            if let Ok(v) = arg.trim_start_matches('-').parse::<u8>() {
                self.rf_gain_db = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("RA") {
            // `nn` (dB) followed by a single on/off digit `m`.
            if let Some((m, head)) = arg.as_bytes().split_last() {
                if let Ok(db) = std::str::from_utf8(head).unwrap_or("x").parse::<u8>() {
                    self.atten_db = Some(db);
                    self.atten_on = Some(*m == b'1');
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("GT") {
            if let Ok(v) = arg.parse::<u8>() {
                self.agc_mode = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("NB") {
            // Full RESP form `NBnnmf`: level, on, filter.
            let b = arg.as_bytes();
            if b.len() >= 3 {
                if let Ok(level) = std::str::from_utf8(&b[0..2]).unwrap_or("x").parse::<u8>() {
                    self.nb_level = Some(level);
                    self.nb_on = Some(b[2] == b'1');
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("NR") {
            // RESP form `NRnnm`: level, mode (1 = on).
            let b = arg.as_bytes();
            if b.len() >= 3 {
                if let Ok(level) = std::str::from_utf8(&b[0..2]).unwrap_or("x").parse::<u8>() {
                    self.nr_level = Some(level);
                    self.nr_on = Some(b[2] == b'1');
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("PA") {
            // RESP form `PAnm`: level, on.
            let b = arg.as_bytes();
            if b.len() >= 2 {
                self.preamp_on = Some(b[1] == b'1');
            }
        } else if let Some(arg) = cmd.strip_prefix("RT") {
            if let Some(m) = arg.bytes().next() {
                self.rit_on = Some(m == b'1');
            }
        } else if let Some(arg) = cmd.strip_prefix("XT") {
            if let Some(m) = arg.bytes().next() {
                self.xit_on = Some(m == b'1');
            }
        } else if let Some(arg) = cmd.strip_prefix("IF") {
            self.apply_if(arg);
        }

        *self != before
    }

    /// Parse the fixed-width `IF` status response. Layout after the `IF` prefix
    /// (K4 Programmer's Reference): `[f×11][5 sp]+yyyy r x [sp]00 t m 0 s p b d 1`,
    /// i.e. freq at 0..11, TX flag at index 26, mode at 27, split at 30.
    ///
    /// trace: FR-CAT-07
    fn apply_if(&mut self, arg: &str) {
        let b = arg.as_bytes();
        if b.len() < 31 {
            return; // too short to contain the split flag
        }
        if let Ok(hz) = arg[0..11].parse::<u64>() {
            self.vfo_a_hz = Some(hz);
        }
        self.transmitting = Some(b[26] == b'1');
        self.mode_a = Mode::from_md_digit(b[27]);
        self.split = Some(b[30] == b'1');
    }
}

/// Map a high-resolution S-meter reading (dBm) to an S-unit label, using the HF
/// convention S9 = −73 dBm and 6 dB per S-unit (S0 ≈ −127 dBm). Above S9 the
/// excess is shown in dB (e.g. `"S9+20dB"`).
///
/// trace: FR-MTR-04
pub fn s_unit_label(dbm: i32) -> String {
    if dbm >= -73 {
        let over = dbm + 73;
        if over <= 0 {
            "S9".to_string()
        } else {
            format!("S9+{over}dB")
        }
    } else {
        // (dbm + 127) / 6, floored, clamped to 0..=9.
        let unit = ((dbm + 127).max(0)) / 6;
        format!("S{}", unit.clamp(0, 9))
    }
}

/// The GET burst issued on (re)connect to seed [`RadioState`]: `IF;` first for a
/// consolidated snapshot, then per-field GETs incl. the S-meter (FR-CAT-07).
///
/// trace: FR-CAT-07
pub fn connect_state_seed() -> &'static [&'static str] {
    &["IF;", "FA;", "FB;", "MD;", "MD$;", "FT;", "SM;", "SMH;"]
}
