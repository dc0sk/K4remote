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
    /// Scan in progress (`IF` `s` flag).
    pub scanning: Option<bool>,
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
    /// Transmit power, raw `nnn` field for the current range (`PC`).
    pub tx_power: Option<u16>,
    /// Transmit power range (`PC` `r`): `H` QRO / `L` QRP / `X` mW.
    pub tx_power_range: Option<char>,
    /// VOX gain 0–60 (`VG`) and anti-VOX/inhibit level 0–60 (`VI`).
    pub vox_gain: Option<u8>,
    pub anti_vox: Option<u8>,
    /// VFO tuning step, Hz (`VT`/`VT$`: 10^n for step index n).
    pub tune_step_hz: Option<u32>,
    pub sub_tune_step_hz: Option<u32>,
    /// DATA sub-mode (`DT`/`DT$`): 0=DATA A, 1=AFSK A, 2=FSK D, 3=PSK D.
    pub data_submode: Option<u8>,
    pub sub_data_submode: Option<u8>,
    /// Mnemonic of the most recent command the K4 rejected with `<cmd>?;`
    /// (PRG Error Checking, FR-CAT-03). `None` = no error since the last clear.
    pub last_error: Option<String>,
    /// Speech compression, 0–30 (`CP`).
    pub compression: Option<u8>,
    /// CW sidetone pitch, Hz (`CW`).
    pub cw_pitch: Option<u16>,
    /// Passband shift / AF center pitch, Hz (`IS`).
    pub shift_hz: Option<u16>,
    /// Sub receiver on (`SB`).
    pub sub_rx: Option<bool>,
    /// Diversity mode on (`DV`).
    pub diversity: Option<bool>,
    /// Manual notch on (`NM`).
    pub notch_on: Option<bool>,
    /// Manual notch pitch, Hz (`NM`).
    pub notch_pitch: Option<u16>,
    /// Auto notch on (`NA`).
    pub auto_notch: Option<bool>,
    /// APF on (`AP`).
    pub apf_on: Option<bool>,
    /// APF bandwidth code 0/1/2 (`AP`).
    pub apf_width: Option<u8>,
    /// Sub-receiver (`$`) read-back of the RX controls above, for RX-B display.
    pub sub_bandwidth_hz: Option<u32>,
    pub sub_af_gain: Option<u8>,
    pub sub_rf_gain_db: Option<u8>,
    pub sub_squelch: Option<u8>,
    pub sub_shift_hz: Option<u16>,
    pub sub_notch_on: Option<bool>,
    pub sub_notch_pitch: Option<u16>,
    pub sub_auto_notch: Option<bool>,
    pub sub_apf_on: Option<bool>,
    pub sub_apf_width: Option<u8>,
    pub sub_atten_db: Option<u8>,
    pub sub_atten_on: Option<bool>,
    pub sub_agc_mode: Option<u8>,
    pub sub_nb_level: Option<u8>,
    pub sub_nb_on: Option<bool>,
    pub sub_nr_level: Option<u8>,
    pub sub_nr_on: Option<bool>,
    pub sub_preamp_on: Option<bool>,
    /// Full break-in QSK on (`SD` x=1).
    pub qsk_full: Option<bool>,
    /// VOX/QSK delay, 10-ms units (`SD` zzz).
    pub qsk_delay: Option<u8>,
    /// TX metering (`TM`), delivered during transmit.
    pub tx_alc: Option<u16>, // ALC (bars)
    pub tx_cmp: Option<u16>,     // compression, dB
    pub tx_fwd_w: Option<u16>,   // forward power (W in QRO)
    pub tx_swr_x10: Option<u16>, // SWR in 1/10 units
    /// Text-decode mode (`TD`); 0 = off.
    pub decode_mode: Option<u8>,
    /// Accumulated decoded receive text (`TB` `s` field), newest at the end.
    pub decode_text: String,
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
    /// Preamp on/off (`PA`) and level 0–3.
    pub preamp_on: Option<bool>,
    pub preamp_level: Option<u8>,
    /// Noise-blanker filter mode (`NB` `f`): 0=none/1=narrow/2=wide.
    pub nb_filter: Option<u8>,
    /// TX/sidetone monitor level 0–100 (`ML`, most recent mode class).
    pub monitor_level: Option<u8>,
    /// RIT/XIT offset, Hz (from `IF`/`RO`; ±9999).
    pub rit_offset: Option<i16>,
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
    /// Mini-pan display on/off (`#MP`).
    pub mini_pan_on: Option<bool>,
    /// ATU mode (`AT`): 1 = bypass (out of line), 2 = auto (in line).
    pub atu_mode: Option<u8>,
    /// TX antenna 1–3 (`AN`), main/sub RX antenna 0–7 (`AR`/`AR$`).
    pub tx_antenna: Option<u8>,
    pub rx_antenna: Option<u8>,
    pub rx_antenna_sub: Option<u8>,
    /// Enabled RX-antenna values as a bitmask over `AR$` values 0–7, from the
    /// main (`ACM`) and sub (`ACS`) RX-antenna access masks. Bit `v` set = the
    /// `AR$v` antenna is in the switch rotation.
    pub rx_ant_avail: Option<u8>,
    pub sub_ant_avail: Option<u8>,
    /// FM repeater offset mode (`RP`): `S`/`+`/`-`, and shift kHz.
    pub repeater_mode: Option<char>,
    pub repeater_offset_khz: Option<u32>,
    /// PL/CTCSS tone (`PL`): table index 1–50 + on/off.
    pub pl_index: Option<u8>,
    pub pl_on: Option<bool>,
    /// Voice VOX on/off (`VX` mode `V`).
    pub vox_voice: Option<bool>,
    /// Current band number 00–25 (`BN`).
    pub band: Option<u8>,
    /// K4 serial number (`SN`).
    pub serial: Option<String>,
    /// Radio UTC time, Unix seconds (`UT`).
    pub utc_unix: Option<u64>,
    /// Remote client count (`CC`): ≥1 server w/ n clients, −1 client, 0 else.
    pub client_count: Option<i8>,
    /// Captured menu-item values (`ME<id>.<value>`), id → value, for full-menu
    /// config backup. The value string is already in `ME` SET format.
    pub menu_values: std::collections::BTreeMap<u16, String>,
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

        // Error reply `<cmd>?;` (PRG Error Checking): the K4 echoes the offending
        // command mnemonic followed by `?`. Record it so the UI can surface which
        // request was rejected — the reply is self-identifying, so no separate
        // pending-request map is needed (FR-CAT-03).
        if let Some(mnemonic) = cmd.strip_suffix('?') {
            self.last_error = Some(mnemonic.to_string());
            return *self != before;
        }

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
            let (sub, a) = split_sub(arg);
            if let Ok(v) = a.parse::<u32>() {
                *sub_or(&mut self.bandwidth_hz, &mut self.sub_bandwidth_hz, sub) = Some(v * 10);
            }
        } else if let Some(arg) = cmd.strip_prefix("AG") {
            let (sub, a) = split_sub(arg);
            if let Ok(v) = a.parse::<u8>() {
                *sub_or(&mut self.af_gain, &mut self.sub_af_gain, sub) = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("RG") {
            let (sub, a) = split_sub(arg);
            if let Ok(v) = a.trim_start_matches('-').parse::<u8>() {
                *sub_or(&mut self.rf_gain_db, &mut self.sub_rf_gain_db, sub) = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("SQ") {
            let (sub, a) = split_sub(arg);
            if let Ok(v) = a.parse::<u8>() {
                *sub_or(&mut self.squelch, &mut self.sub_squelch, sub) = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("PC") {
            // `nnn` + optional range letter L/H/X (L assumed if omitted). Store
            // watts: H = watts as-is, L/(none) = 0.1 W units, X (mW) ≈ 0.
            let (digits, range) = match arg.as_bytes().last() {
                Some(b'H') | Some(b'L') | Some(b'X') => arg.split_at(arg.len() - 1),
                _ => (arg, ""),
            };
            if let Ok(n) = digits.parse::<u16>() {
                self.tx_power = Some(n); // raw nnn for the current range
                self.tx_power_range = Some(match range {
                    "H" => 'H',
                    "X" => 'X',
                    _ => 'L',
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
            let (sub, a) = split_sub(arg);
            if let Ok(v) = a.parse::<u16>() {
                *sub_or(&mut self.shift_hz, &mut self.sub_shift_hz, sub) = Some(v * 10);
            }
        } else if let Some(arg) = cmd.strip_prefix("SN") {
            if !arg.is_empty() {
                self.serial = Some(arg.to_string());
            }
        } else if let Some(arg) = cmd.strip_prefix("SB") {
            self.sub_rx = Some(arg == "1");
        } else if let Some(arg) = cmd.strip_prefix("DV") {
            self.diversity = Some(arg == "1");
        } else if let Some(arg) = cmd.strip_prefix("NM") {
            // `nnnnm` (pitch + on) or the alternate `m` (on/off only); `$`=sub.
            let (sub, a) = split_sub(arg);
            let b = a.as_bytes();
            if b.len() >= 5 {
                if let Ok(p) = a[..4].parse::<u16>() {
                    *sub_or(&mut self.notch_pitch, &mut self.sub_notch_pitch, sub) = Some(p);
                }
                *sub_or(&mut self.notch_on, &mut self.sub_notch_on, sub) = Some(b[4] == b'1');
            } else if b.len() == 1 {
                *sub_or(&mut self.notch_on, &mut self.sub_notch_on, sub) = Some(b[0] == b'1');
            }
        } else if let Some(arg) = cmd.strip_prefix("NA") {
            let (sub, a) = split_sub(arg);
            *sub_or(&mut self.auto_notch, &mut self.sub_auto_notch, sub) = Some(a == "1");
        } else if let Some(arg) = cmd.strip_prefix("AP") {
            let (sub, a) = split_sub(arg);
            let b = a.as_bytes();
            if !b.is_empty() {
                *sub_or(&mut self.apf_on, &mut self.sub_apf_on, sub) = Some(b[0] == b'1');
                if b.len() >= 2 && b[1].is_ascii_digit() {
                    *sub_or(&mut self.apf_width, &mut self.sub_apf_width, sub) =
                        Some((b[1] - b'0').min(2));
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("TM") {
            // `aaabbbcccddd`: ALC, CMP(dB), FWD power, SWR(×0.1). Auto-delivered.
            if arg.len() >= 12 {
                self.tx_alc = arg[0..3].parse().ok();
                self.tx_cmp = arg[3..6].parse().ok();
                self.tx_fwd_w = arg[6..9].parse().ok();
                self.tx_swr_x10 = arg[9..12].parse().ok();
            }
        } else if let Some(arg) = cmd.strip_prefix("TB") {
            // `TB[$]trrs`: t=TX queue, rr=RX char count, s=decoded text (may
            // itself contain `;`). Append s to the rolling decode buffer.
            let (_sub, a) = split_sub(arg);
            if a.len() > 3 {
                self.decode_text.push_str(&a[3..]);
                let len = self.decode_text.len();
                if len > 4000 {
                    self.decode_text = self.decode_text.split_off(len - 2000);
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("TD") {
            let (_sub, a) = split_sub(arg);
            if let Some(c) = a.bytes().next() {
                self.decode_mode = Some(c.wrapping_sub(b'0'));
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
            // `nn` (dB) followed by a single on/off digit `m`; `$`=sub.
            let (sub, a) = split_sub(arg);
            if let Some((m, head)) = a.as_bytes().split_last() {
                if let Ok(db) = std::str::from_utf8(head).unwrap_or("x").parse::<u8>() {
                    *sub_or(&mut self.atten_db, &mut self.sub_atten_db, sub) = Some(db);
                    *sub_or(&mut self.atten_on, &mut self.sub_atten_on, sub) = Some(*m == b'1');
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("GT") {
            let (sub, a) = split_sub(arg);
            if let Ok(v) = a.parse::<u8>() {
                *sub_or(&mut self.agc_mode, &mut self.sub_agc_mode, sub) = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("NB") {
            // Full RESP form `NB[$]nnmf`: level, on, filter.
            let (sub, a) = split_sub(arg);
            let b = a.as_bytes();
            if b.len() >= 3 {
                if let Ok(level) = std::str::from_utf8(&b[0..2]).unwrap_or("x").parse::<u8>() {
                    *sub_or(&mut self.nb_level, &mut self.sub_nb_level, sub) = Some(level);
                    *sub_or(&mut self.nb_on, &mut self.sub_nb_on, sub) = Some(b[2] == b'1');
                    if b.len() >= 4 && b[3].is_ascii_digit() {
                        self.nb_filter = Some((b[3] - b'0').min(2));
                    }
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("NR") {
            // RESP form `NR[$]nnm`: level, mode (1 = on).
            let (sub, a) = split_sub(arg);
            let b = a.as_bytes();
            if b.len() >= 3 {
                if let Ok(level) = std::str::from_utf8(&b[0..2]).unwrap_or("x").parse::<u8>() {
                    *sub_or(&mut self.nr_level, &mut self.sub_nr_level, sub) = Some(level);
                    *sub_or(&mut self.nr_on, &mut self.sub_nr_on, sub) = Some(b[2] == b'1');
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("PA") {
            // RESP form `PA[$]nm`: level, on.
            let (sub, a) = split_sub(arg);
            let b = a.as_bytes();
            if b.len() >= 2 {
                *sub_or(&mut self.preamp_on, &mut self.sub_preamp_on, sub) = Some(b[1] == b'1');
                if !sub && b[0].is_ascii_digit() {
                    self.preamp_level = Some((b[0] - b'0').min(3));
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("ML") {
            // `MLmnnn` — monitor level; keep the level (nnn) for the mode class.
            if arg.len() >= 4 {
                if let Ok(v) = arg[1..4].parse::<u8>() {
                    self.monitor_level = Some(v);
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("RO") {
            // Bare `RO snnnn` — signed RIT/XIT offset (Hz) for the main VFO, the
            // one `IF` reports. `RO$…` is the sub-RX form; `split_sub` tolerates
            // the optional `$` so both parse here.
            let (_, a) = split_sub(arg);
            if let Ok(mag) = a.get(1..5).unwrap_or("").parse::<i16>() {
                self.rit_offset = Some(if a.starts_with('-') { -mag } else { mag });
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
            // D12 names these `#REF$` / `#SPN$` / `#WFC$` (LCD) against
            // `#HREF` / `#HWFC` (external monitor): here the `$` is part of
            // the *mnemonic*, not the sub-receiver modifier. Accept it either
            // way — a `$` response used to fail the integer parse and be
            // dropped silently, so the read-back these controls sync from
            // never arrived.
            if let Ok(v) = split_sub(arg).1.parse::<i16>() {
                self.pan_ref = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#SPN") {
            if let Ok(v) = split_sub(arg).1.parse::<u32>() {
                self.pan_span_hz = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#SCL") {
            if let Ok(v) = split_sub(arg).1.parse::<u16>() {
                self.pan_scale = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#DPM") {
            if let Ok(v) = split_sub(arg).1.parse::<u8>() {
                self.pan_mode = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#WFC") {
            if let Ok(v) = split_sub(arg).1.parse::<u8>() {
                self.wf_palette = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#WFH") {
            if let Ok(v) = split_sub(arg).1.parse::<u8>() {
                self.wf_height = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("#MP") {
            // `#MP$n` — mini-pan on/off (n = -1/0/1). `$` optional.
            let (_, a) = split_sub(arg);
            self.mini_pan_on = Some(a.starts_with('1'));
        } else if let Some(arg) = cmd.strip_prefix("ACM") {
            self.rx_ant_avail = Some(parse_ant_mask(arg));
        } else if let Some(arg) = cmd.strip_prefix("ACS") {
            self.sub_ant_avail = Some(parse_ant_mask(arg));
        } else if let Some(arg) = cmd.strip_prefix("AN") {
            if let Ok(v) = arg.parse::<u8>() {
                self.tx_antenna = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("AT") {
            if let Ok(v) = arg.parse::<u8>() {
                self.atu_mode = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("AR$") {
            if let Ok(v) = arg.parse::<u8>() {
                self.rx_antenna_sub = Some(v);
            }
        } else if let Some(arg) = cmd.strip_prefix("AR") {
            if let Ok(v) = arg.parse::<u8>() {
                self.rx_antenna = Some(v);
            }
        } else if cmd.starts_with("MEDF") {
            // Menu definition — not captured (values arrive via `ME<id>.<v>`).
        } else if let Some(arg) = cmd.strip_prefix("ME") {
            // `ME<id>.<value>` — capture the value for full-menu backup.
            if let Some((id_s, val)) = arg.split_once('.') {
                if let Ok(id) = id_s.parse::<u16>() {
                    self.menu_values.insert(id, val.to_string());
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("UT") {
            if let Ok(t) = arg.parse::<u64>() {
                self.utc_unix = Some(t);
            }
        } else if let Some(arg) = cmd.strip_prefix("CC") {
            if let Ok(n) = arg.parse::<i8>() {
                self.client_count = Some(n);
            }
        } else if let Some(arg) = cmd.strip_prefix("RP") {
            // `RPmnnnnn` — repeater mode + offset kHz.
            let b = arg.as_bytes();
            if !b.is_empty() {
                self.repeater_mode = Some(b[0] as char);
                if let Ok(k) = arg.get(1..6).unwrap_or("").parse::<u32>() {
                    self.repeater_offset_khz = Some(k);
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("PL") {
            // `PLnnm` (or `PL$nnm`) — tone index + on/off.
            let (_, a) = split_sub(arg);
            let b = a.as_bytes();
            if b.len() >= 3 {
                if let Ok(i) = a[0..2].parse::<u8>() {
                    self.pl_index = Some(i);
                    self.pl_on = Some(b[2] == b'1');
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("VT") {
            // `VT[$]nm` — n is the step index (0–5 → 1 Hz … 100 kHz).
            let (sub, a) = split_sub(arg);
            if let Some(nc) = a.bytes().next() {
                if nc.is_ascii_digit() {
                    let step = 10u32.pow(u32::from(nc - b'0'));
                    *sub_or(&mut self.tune_step_hz, &mut self.sub_tune_step_hz, sub) = Some(step);
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("DT") {
            // `DT[$]n` — DATA sub-mode (0=DATA A, 1=AFSK A, 2=FSK D, 3=PSK D).
            let (sub, a) = split_sub(arg);
            if let Some(nc) = a.bytes().next() {
                if nc.is_ascii_digit() {
                    *sub_or(&mut self.data_submode, &mut self.sub_data_submode, sub) =
                        Some(nc - b'0');
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("VG") {
            // `VGmnnn` — VOX gain (m = V/D); surface the level.
            if arg.len() >= 4 {
                if let Ok(v) = arg[1..4].parse::<u8>() {
                    self.vox_gain = Some(v);
                }
            }
        } else if let Some(arg) = cmd.strip_prefix("VI") {
            if let Ok(v) = arg.parse::<u8>() {
                self.anti_vox = Some(v);
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
        // RIT/XIT offset: sign at byte 16, magnitude (Hz) at bytes 17..21, then
        // the RIT-on (21) and XIT-on (22) flags.
        if let Ok(mag) = arg[17..21].parse::<i16>() {
            self.rit_offset = Some(if b[16] == b'-' { -mag } else { mag });
        }
        self.rit_on = Some(b[21] == b'1');
        self.xit_on = Some(b[22] == b'1');
        self.transmitting = Some(b[26] == b'1');
        self.mode_a = Mode::from_md_digit(b[27]);
        self.scanning = Some(b[29] == b'1'); // `s` scan-in-progress flag
        self.split = Some(b[30] == b'1');
    }
}

/// Parse an RX-antenna access mask (`ACM`/`ACS` = `zabcdefg`) into a bitmask
/// over `AR$` antenna values 0–7. `z=1` (DISPLAY ALL) enables all sources
/// (`AR$1..=7`); otherwise the a–g flags map to their `AR$` value.
/// NOTE: the a–g → `AR$` mapping is a documented best-effort (unverified live).
fn parse_ant_mask(arg: &str) -> u8 {
    let b = arg.as_bytes();
    if b.len() < 8 {
        return 0;
    }
    if b[0] == b'1' {
        return 0b1111_1110; // DISPLAY ALL → AR$ 1..=7
    }
    // (mask index within `zabcdefg`, AR$ value): a=ANT1→5, b=ANT2→6, c=ANT3→7,
    // d=RX1→4, e=RX2→1, f==TX ANT→2. (g ==OPP TX ANT has no distinct AR$.)
    const MAP: [(usize, u8); 6] = [(1, 5), (2, 6), (3, 7), (4, 4), (5, 1), (6, 2)];
    let mut avail = 0u8;
    for (i, ar) in MAP {
        if b[i] == b'1' {
            avail |= 1 << ar;
        }
    }
    avail
}

/// Split a RESP argument on a leading `$` (sub-receiver) modifier: returns
/// `(is_sub, remainder)`.
fn split_sub(arg: &str) -> (bool, &str) {
    match arg.strip_prefix('$') {
        Some(rest) => (true, rest),
        None => (false, arg),
    }
}

/// Pick the sub-receiver slot when `is_sub`, else the main slot.
fn sub_or<'a, T>(main: &'a mut T, sub: &'a mut T, is_sub: bool) -> &'a mut T {
    if is_sub {
        sub
    } else {
        main
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
        "IS;", "SB;", "DV;", // passband shift, sub-RX, diversity
        "NM;", "NA;", "AP;", // manual notch, auto notch, APF
        "RA;", "GT;", "NB;", "NR;", "PA;", // main atten/AGC/NB/NR/preamp state
        // Sub-receiver read-back of the same RX controls (RX-B display):
        "BW$;", "AG$;", "RG$;", "SQ$;", "IS$;", "NM$;", "NA$;", "AP$;", //
        "RA$;", "GT$;", "NB$;", "NR$;", "PA$;",
        "TM1;", // enable auto TX metering (RF/ALC/SWR/CMP during transmit)
        "AT;", "ACM;", "ACS;", // ATU mode + RX/sub antenna access masks
        "SN;",  // K4 serial number (for config-export filenames)
        "RO;",  // RIT/XIT offset (Hz) — bare `RO` is the main VFO's
        "ML0;", "ML1;", "ML2;", // monitor levels (CW / AF-data / voice)
        "VGV;", "VI;", // VOX gain (voice) + anti-VOX level
        "VT;", "VT$;", // VFO tuning step (for optimistic ◄► stepping)
        "DT;", "DT$;", // DATA sub-mode (DATA A / AFSK A / FSK D / PSK D)
        "RP;", "PL;", // FM repeater offset + PL/CTCSS tone
        "UT;", "CC;",   // radio UTC time + remote client count (status strip)
        "#MP$;", // mini-pan on/off
        // Configuration-screen read-back (FR-UI-19 screens):
        "RE;", "TE;", "KP;", "KS;", "MI;", "MG;", "LO;", "AN;", "AR;", "AR$;", "VXV;", "BN;",
        "#REF;", "#SPN;", "#SCL;", "#DPM;", "#WFC;", "#WFH;",
    ]
}
