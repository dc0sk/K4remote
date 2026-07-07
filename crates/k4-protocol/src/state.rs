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
    /// Squelch threshold, 0–40 (`SQ`).
    pub squelch: Option<u8>,
    /// Transmit power, watts (`PC`).
    pub tx_power: Option<u16>,
    /// Speech compression, 0–30 (`CP`).
    pub compression: Option<u8>,
    /// CW sidetone pitch, Hz (`CW`).
    pub cw_pitch: Option<u16>,
    /// Passband shift / AF center pitch, Hz (`IS`).
    pub shift_hz: Option<u16>,
    /// Full break-in QSK on (`SD` x=1).
    pub qsk_full: Option<bool>,
    /// VOX/QSK delay, 10-ms units (`SD` zzz).
    pub qsk_delay: Option<u8>,
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

    // --- Configuration-screen state (read-back for FR-UI-19 screens) ---
    /// RX graphic-EQ, 8 bands, dB (`RE`).
    pub rx_eq: Option<[i8; 8]>,
    /// TX graphic-EQ, 8 bands, dB (`TE`).
    pub tx_eq: Option<[i8; 8]>,
    /// Keyer: iambic-B (`true`) vs A, paddle reversed, weight 90–125 (`KP`).
    pub keyer_iambic_b: Option<bool>,
    pub keyer_paddle_rev: Option<bool>,
    pub keyer_weight: Option<u16>,
    /// Keyer speed, WPM (`KS`).
    pub keyer_speed: Option<u8>,
    /// Mic input 0–4 (`MI`) and mic gain 0–80 (`MG`).
    pub mic_input: Option<u8>,
    pub mic_gain: Option<u8>,
    /// Line-out left/right level and RIGHT=LEFT gang (`LO`).
    pub line_out_left: Option<u16>,
    pub line_out_right: Option<u16>,
    pub line_out_gang: Option<bool>,
    /// Panadapter reference level, dBm (`#REF`).
    pub pan_ref: Option<i16>,
    /// Panadapter span, Hz (`#SPN`).
    pub pan_span_hz: Option<u32>,
    /// Panadapter scale, dB (`#SCL`).
    pub pan_scale: Option<u16>,
    /// Panadapter mode: 0=A, 1=B, 2=dual (`#DPM`).
    pub pan_mode: Option<u8>,
    /// Waterfall palette 0–4 (`#WFC`) and height 0–100 (`#WFH`).
    pub wf_palette: Option<u8>,
    pub wf_height: Option<u8>,
    /// TX antenna 1–3 (`AN`), main/sub RX antenna 0–7 (`AR`/`AR$`).
    pub tx_antenna: Option<u8>,
    pub rx_antenna: Option<u8>,
    pub rx_antenna_sub: Option<u8>,
    /// Voice VOX on/off (`VX` mode `V`).
    pub vox_voice: Option<bool>,
    /// Current band number 00–25 (`BN`).
    pub band: Option<u8>,
}

/// Parse 8 consecutive signed 3-char EQ fields (`+00-01…`) into `[i8; 8]`.
fn parse_eq8(arg: &str) -> Option<[i8; 8]> {
    let b = arg.as_bytes();
    if b.len() < 24 {
        return None;
    }
    let mut out = [0i8; 8];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = arg[i * 3..i * 3 + 3].parse::<i8>().ok()?;
    }
    Some(out)
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
        } else if let Some(arg) = cmd.strip_prefix("SQ") {
            // Main squelch only; `SQ$…` (sub) starts with `$` and is skipped.
            if let Ok(v) = arg.parse::<u8>() {
                self.squelch = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("PC") {
            // `nnn` + optional range letter L/H/X (L assumed if omitted). Store
            // watts: H = watts as-is, L/(none) = 0.1 W units, X (mW) ≈ 0.
            let (digits, range) = match arg.as_bytes().last() {
                Some(b'H') | Some(b'L') | Some(b'X') => arg.split_at(arg.len() - 1),
                _ => (arg, ""),
            };
            if let Ok(n) = digits.parse::<u16>() {
                self.tx_power = Some(match range {
                    "H" => n,
                    "X" => 0,
                    _ => n / 10,
                });
            }
        } else if let Some(arg) = cmd.strip_prefix("CP") {
            if let Ok(v) = arg.parse::<u8>() {
                self.compression = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("CW") {
            if let Ok(v) = arg.parse::<u16>() {
                self.cw_pitch = Some(v * 10);
            }
        } else if let Some(arg) = cmd.strip_prefix("IS") {
            // Main center pitch only; `IS$…` (sub) starts with `$` and is skipped.
            if let Ok(v) = arg.parse::<u16>() {
                self.shift_hz = Some(v * 10);
            }
        } else if let Some(arg) = cmd.strip_prefix("SD") {
            // `x y zzz`: full-QSK flag, mode class, then the 10-ms delay.
            let b = arg.as_bytes();
            if b.len() >= 5 {
                self.qsk_full = Some(b[0] == b'1');
                if let Ok(v) = arg[2..].parse::<u8>() {
                    self.qsk_delay = Some(v);
                }
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
        } else if let Some(arg) = cmd.strip_prefix("RE") {
            if let Some(eq) = parse_eq8(arg) {
                self.rx_eq = Some(eq);
            }
        } else if let Some(arg) = cmd.strip_prefix("TE") {
            if let Some(eq) = parse_eq8(arg) {
                self.tx_eq = Some(eq);
            }
        } else if let Some(arg) = cmd.strip_prefix("KP") {
            // `KPionnn`: iambic (A/B), paddle (N/R), weight (3 digits).
            let b = arg.as_bytes();
            if b.len() >= 5 {
                self.keyer_iambic_b = Some(b[0] == b'B');
                self.keyer_paddle_rev = Some(b[1] == b'R');
                if let Ok(w) = arg[2..5].parse::<u16>() {
                    self.keyer_weight = Some(w);
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("KS") {
            if let Ok(v) = arg.parse::<u8>() {
                self.keyer_speed = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("MI") {
            if let Ok(v) = arg.parse::<u8>() {
                self.mic_input = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("MG") {
            if let Ok(v) = arg.parse::<u8>() {
                self.mic_gain = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("LO") {
            // `LO<left3><right3><gang1>`.
            let b = arg.as_bytes();
            if b.len() >= 7 {
                if let Ok(l) = arg[0..3].parse::<u16>() {
                    self.line_out_left = Some(l);
                }
                if let Ok(r) = arg[3..6].parse::<u16>() {
                    self.line_out_right = Some(r);
                }
                self.line_out_gang = Some(b[6] == b'1');
            }
        } else if let Some(arg) = cmd.strip_prefix("#REF") {
            if let Ok(v) = arg.parse::<i16>() {
                self.pan_ref = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#SPN") {
            if let Ok(v) = arg.parse::<u32>() {
                self.pan_span_hz = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#SCL") {
            if let Ok(v) = arg.parse::<u16>() {
                self.pan_scale = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#DPM") {
            if let Ok(v) = arg.parse::<u8>() {
                self.pan_mode = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#WFC") {
            if let Ok(v) = arg.parse::<u8>() {
                self.wf_palette = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#WFH") {
            if let Ok(v) = arg.parse::<u8>() {
                self.wf_height = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("AN") {
            if let Ok(v) = arg.parse::<u8>() {
                self.tx_antenna = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("AR$") {
            if let Ok(v) = arg.parse::<u8>() {
                self.rx_antenna_sub = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("AR") {
            if let Ok(v) = arg.parse::<u8>() {
                self.rx_antenna = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("VX") {
            // `VXmn`: only the voice (`V`) mode is surfaced.
            let b = arg.as_bytes();
            if b.len() >= 2 && b[0] == b'V' {
                self.vox_voice = Some(b[1] == b'1');
            }
        } else if let Some(arg) = cmd.strip_prefix("BN$") {
            // Sub-RX band; kept distinct so it does not clobber the main band.
            let _ = arg;
        } else if let Some(arg) = cmd.strip_prefix("BN") {
            if let Ok(v) = arg.parse::<u8>() {
                self.band = Some(v);
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
    &[
        "IF;", "FA;", "FB;", "MD;", "MD$;", "FT;", "SM;", "SMH;", // core
        "BW;", "AG;", "RG;", "SQ;", // RX levels (bandwidth, AF/RF gain, squelch)
        "PC;", "CP;", "CW;", "SD;", // TX power, compression, CW pitch, QSK delay
        "IS;", // passband shift / AF center pitch
        // Configuration-screen read-back (FR-UI-19 screens):
        "RE;", "TE;", "KP;", "KS;", "MI;", "MG;", "LO;", "AN;", "AR;", "AR$;", "VXV;", "BN;",
        "#REF;", "#SPN;", "#SCL;", "#DPM;", "#WFC;", "#WFH;",
    ]
}
