//! Elecraft **KPA1500** linear-amplifier remote control — pure CAT codec
//! (FR-AMP-02).
//!
//! Per the *KPA1500 Programming Reference* (V3). The amplifier exposes one
//! `^`-prefixed command set over three transports — its Host-PC USB serial, its
//! XCVR serial, and a **TCP server on port 1500** — and this app talks to it
//! over that TCP server, a second connection alongside the K4 (see FR-AMP-01).
//! This crate is **dependency-free and fully unit-tested**; the socket I/O and
//! the poll loop live in the app.
//!
//! Commands and responses begin with a caret (`^`) and end with a semicolon
//! (`;`). GETs retrieve information (the amp replies with a RESP); SETs change
//! state and usually get no reply. Commands may be upper, lower or mixed case;
//! responses are UPPER case except the boot-block identity `^kpa1500;`. This
//! module encodes the SETs [`cat`] and parses the RESPs into a [`KpaState`].

#![forbid(unsafe_code)]

/// The amplifier's TCP command-server port (its remote-head interface).
pub const TCP_PORT: u16 = 1500;

/// The pipelined GET string sent each poll interval — one write, many RESPs.
///
/// `^WS` returns forward power **and** SWR in a single reply (KPA500-compatible),
/// so it stands in for both; the rest cover mode, temperature, fault, band, ATU
/// state, antenna, fan and tune-in-progress. Chosen to match a monitoring panel;
/// unrecognised replies are ignored by [`apply`], so extending this is safe.
pub const POLL: &str = "^OS;^WS;^TM;^FL;^BN;^AI;^AM;^AN;^FS;^TP;";

/// One-time GETs sent right after connecting: identity, firmware, serial.
pub const IDENT: &str = "^I;^RVM;^SN;";

/// Operating mode reported by `^OS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Amplifier bypassed — RF passes straight through (`^OS0`).
    Standby,
    /// Amplifier in line and amplifying (`^OS1`).
    Operate,
}

/// A telemetry + status snapshot. Every field is `Option`, `None` until the
/// amplifier has reported it, so a fresh connection never shows a stale or
/// invented value (the same discipline as the K4 [`RadioState`] seed).
///
/// [`RadioState`]: https://docs.rs/k4-protocol
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KpaState {
    /// `^I` confirmed the peer really is a KPA1500 (guards against pointing the
    /// client at some other `^`-speaking device).
    pub identified: bool,
    /// Operate / Standby (`^OS`).
    pub mode: Option<Mode>,
    /// Forward RF power, watts (`^WS`/`^PWF`).
    pub forward_w: Option<u16>,
    /// Reflected RF power, watts (`^PWR`).
    pub reflected_w: Option<u16>,
    /// SWR in tenths — `14` is 1.4:1 (`^WS`/`^SW`).
    pub swr_tenths: Option<u16>,
    /// PA heat-sink temperature, °C (`^TM`).
    pub temp_c: Option<u16>,
    /// PA current, whole amperes (`^PC`).
    pub pa_current_a: Option<u16>,
    /// Current fault code, a hex byte; `0x00` is "no fault" (`^FL`).
    pub fault: Option<u8>,
    /// Band number `0..=10` (`^BN`); see [`band_name`].
    pub band: Option<u8>,
    /// ATU **relays** in-circuit (`true`) vs bypassed (`^AI`).
    pub atu_inline: Option<bool>,
    /// ATU **mode** inline (`true`) vs bypassed (`^AM`). The mode and the relay
    /// state can differ — e.g. mode Inline but "bypassed" after a low-SWR tune.
    pub atu_mode_inline: Option<bool>,
    /// Selected antenna number `1..=32` (`^AN`).
    pub antenna: Option<u8>,
    /// Fan speed `0..=5` (`^FS`).
    pub fan: Option<u8>,
    /// An ATU tune is in progress (`^TP`).
    pub tuning: Option<bool>,
    /// Firmware version string, e.g. `"01.23"` (`^RVM`).
    pub firmware: Option<String>,
    /// Amplifier serial number (`^SN`).
    pub serial: Option<String>,
}

impl KpaState {
    /// SWR as a ratio (`14` tenths → `1.4`), if reported.
    #[must_use]
    pub fn swr(&self) -> Option<f32> {
        self.swr_tenths.map(|t| f32::from(t) / 10.0)
    }

    /// Whether a fault is currently active (a non-zero `^FL` code).
    #[must_use]
    pub fn is_faulted(&self) -> bool {
        matches!(self.fault, Some(c) if c != 0)
    }

    /// Human-readable description of the current fault, if any is active.
    #[must_use]
    pub fn fault_text(&self) -> Option<&'static str> {
        match self.fault {
            Some(c) if c != 0 => Some(fault_description(c)),
            _ => None,
        }
    }

    /// The current band's name, if reported and known.
    #[must_use]
    pub fn band_name(&self) -> Option<&'static str> {
        self.band.and_then(band_name)
    }
}

/// Fault-code description, from the `^FL` table in the Programming Reference.
///
/// Unknown codes fall through to a generic label rather than panicking — a
/// firmware revision may add codes this build has never seen.
#[must_use]
pub fn fault_description(code: u8) -> &'static str {
    match code {
        0x00 => "no fault",
        0x10 => "watchdog timer reset",
        0x20 => "PA current too high",
        0x40 => "temperature too high",
        0x60 => "input power too high",
        0x61 => "gain too low",
        0x70 => "invalid frequency",
        0x80 => "50 V supply out of range",
        0x81 => "5 V supply out of range",
        0x82 => "10 V supply out of range",
        0x83 => "12 V supply out of range",
        0x84 => "−12 V supply out of range",
        0x85 => "LPF board supply not detected",
        0x90 => "reflected power too high",
        0x91 => "very high SWR (antenna not connected?)",
        0x92 => "ATU could not match (no match)",
        0xB0 => "dissipated power too high",
        0xC0 => "forward power too high",
        0xC1 => "forward power too high for ATU setting",
        0xF0 => "gain too high",
        _ => "fault",
    }
}

/// Band name for a `^BN` band number, or `None` if out of range (`0..=10`).
#[must_use]
pub fn band_name(code: u8) -> Option<&'static str> {
    Some(match code {
        0 => "160 m",
        1 => "80 m",
        2 => "60 m",
        3 => "40 m",
        4 => "30 m",
        5 => "20 m",
        6 => "17 m",
        7 => "15 m",
        8 => "12 m",
        9 => "10 m",
        10 => "6 m",
        _ => return None,
    })
}

/// Apply every `;`-terminated response in `data` to `state`, returning how many
/// were recognised. Partial or unknown tokens are skipped, so a mid-stream
/// fragment or a reply for a command this build does not model is harmless.
pub fn apply(state: &mut KpaState, data: &str) -> usize {
    data.split(';')
        .filter(|t| !t.trim().is_empty())
        .filter(|t| apply_one(state, t))
        .count()
}

/// Apply a single response token (no trailing `;`). Returns `true` if it matched
/// a field this codec models.
fn apply_one(state: &mut KpaState, token: &str) -> bool {
    // Responses lead with `^`; tolerate its absence and surrounding space.
    let t = token.trim().strip_prefix('^').unwrap_or(token.trim());

    // Identity is the one case-varying reply (`^KPA1500` app / `^kpa1500` boot).
    if t.eq_ignore_ascii_case("KPA1500") {
        state.identified = true;
        return true;
    }

    // Longer prefixes must be tried before their shorter namesakes (RVM before
    // RV; the 3-char PW* powers before any 2-char match).
    if let Some(v) = t.strip_prefix("RVM") {
        return set_text(&mut state.firmware, v);
    }
    if let Some(v) = t.strip_prefix("PWR") {
        return match v.trim().parse::<u16>() {
            Ok(w) => set(&mut state.reflected_w, w),
            Err(_) => false,
        };
    }
    if let Some(v) = t.strip_prefix("PWF") {
        return match v.trim().parse::<u16>() {
            Ok(w) => set(&mut state.forward_w, w),
            Err(_) => false,
        };
    }

    let (tag, rest) = t.split_at(t.len().min(2));
    match tag {
        "OS" => match rest {
            "0" => set(&mut state.mode, Mode::Standby),
            "1" => set(&mut state.mode, Mode::Operate),
            _ => false,
        },
        // `^WSwwww swr;` — forward power (watts) and SWR (tenths) in one reply.
        "WS" => {
            let mut parts = rest.split_ascii_whitespace();
            match (
                parts.next().and_then(|w| w.parse::<u16>().ok()),
                parts.next().and_then(|s| s.parse::<u16>().ok()),
            ) {
                (Some(w), Some(s)) => {
                    state.forward_w = Some(w);
                    state.swr_tenths = Some(s);
                    true
                }
                _ => false,
            }
        }
        "SW" => match rest.trim().parse::<u16>() {
            Ok(v) => set(&mut state.swr_tenths, v),
            Err(_) => false,
        },
        "TM" => match rest.trim().parse::<u16>() {
            Ok(v) => set(&mut state.temp_c, v),
            Err(_) => false,
        },
        "PC" => match rest.trim().parse::<u16>() {
            Ok(v) => set(&mut state.pa_current_a, v),
            Err(_) => false,
        },
        "FL" => match u8::from_str_radix(rest.trim(), 16) {
            Ok(code) => set(&mut state.fault, code),
            Err(_) => false,
        },
        "BN" => match rest.trim().parse::<u8>() {
            Ok(b) if band_name(b).is_some() => set(&mut state.band, b),
            _ => false,
        },
        "AI" => match rest {
            "0" => set(&mut state.atu_inline, false),
            "1" => set(&mut state.atu_inline, true),
            _ => false,
        },
        // `^AMI;` / `^AMB;` — the current-band mode. The all-bands form
        // (`^AMAB…`) has a longer body and is deliberately not matched here.
        "AM" => match rest {
            "I" => set(&mut state.atu_mode_inline, true),
            "B" => set(&mut state.atu_mode_inline, false),
            _ => false,
        },
        "AN" => match rest.trim().parse::<u8>() {
            Ok(n) if (1..=32).contains(&n) => set(&mut state.antenna, n),
            _ => false,
        },
        "FS" => match rest.trim().parse::<u8>() {
            Ok(f) if f <= 5 => set(&mut state.fan, f),
            _ => false,
        },
        "TP" => match rest {
            "0" => set(&mut state.tuning, false),
            "1" => set(&mut state.tuning, true),
            _ => false,
        },
        "SN" => set_text(&mut state.serial, rest),
        // Bare `^RV` firmware (RVM was handled above).
        "RV" => set_text(&mut state.firmware, rest),
        _ => false,
    }
}

/// Store a trimmed, **non-empty** string into `slot`. A bare-tag fragment
/// (e.g. `^RVM` with no body) must not blank an already-known value.
fn set_text(slot: &mut Option<String>, value: &str) -> bool {
    let v = value.trim();
    if v.is_empty() {
        return false;
    }
    *slot = Some(v.to_string());
    true
}

/// Store `value` into `slot` and report `true` (a matched field). A tiny helper
/// so each arm above reads as "recognised → set".
fn set<T>(slot: &mut Option<T>, value: T) -> bool {
    *slot = Some(value);
    true
}

/// Control-command (SET) encoders. Every returned string is a complete,
/// ready-to-send `^…;` command.
pub mod cat {
    /// Operate (`^OS1;`) or Standby (`^OS0;`).
    #[must_use]
    pub fn set_mode(operate: bool) -> String {
        format!("^OS{};", u8::from(operate))
    }

    /// Start a full-search ATU tune (`^FT;`). The amp replies `^FT;` when the
    /// tune completes or is cancelled. Requires continuous exciter RF, so the
    /// caller must key the K4 for the duration.
    #[must_use]
    pub fn start_tune() -> &'static str {
        "^FT;"
    }

    /// Cancel an in-progress ATU tune (`^FE;`).
    #[must_use]
    pub fn cancel_tune() -> &'static str {
        "^FE;"
    }

    /// Put the ATU mode inline (`^AMI;`) or bypassed (`^AMB;`) for the current
    /// band and antenna.
    #[must_use]
    pub fn set_atu_mode(inline: bool) -> String {
        format!("^AM{};", if inline { 'I' } else { 'B' })
    }

    /// Select antenna `n` (`1..=32`), using the two-digit form (`^AN01;`..
    /// `^AN32;`) that is accepted for every antenna — this avoids `^AN0;`/
    /// `^AN00;`, which mean "advance to the next enabled antenna". Returns
    /// `None` for an out-of-range number rather than emitting a bad command.
    #[must_use]
    pub fn select_antenna(n: u8) -> Option<String> {
        (1..=32).contains(&n).then(|| format!("^AN{n:02};"))
    }

    /// Turn the amplifier's main power supplies on (`^ON1;`) or off (`^ON0;`).
    #[must_use]
    pub fn set_power(on: bool) -> String {
        format!("^ON{};", u8::from(on))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Control encoders emit exactly the documented `^…;` wire forms.
    /// trace: FR-AMP-02
    #[test]
    fn fr_amp_02_control_encoders_match_reference() {
        assert_eq!(cat::set_mode(true), "^OS1;");
        assert_eq!(cat::set_mode(false), "^OS0;");
        assert_eq!(cat::start_tune(), "^FT;");
        assert_eq!(cat::cancel_tune(), "^FE;");
        assert_eq!(cat::set_atu_mode(true), "^AMI;");
        assert_eq!(cat::set_atu_mode(false), "^AMB;");
        assert_eq!(cat::set_power(true), "^ON1;");
        assert_eq!(cat::set_power(false), "^ON0;");
    }

    /// Antenna select uses the two-digit form for all of 1..=32 (never `^AN0`,
    /// which means "next antenna"), and refuses out-of-range numbers.
    /// trace: FR-AMP-02
    #[test]
    fn fr_amp_02_antenna_select_is_two_digit_and_bounded() {
        assert_eq!(cat::select_antenna(1).as_deref(), Some("^AN01;"));
        assert_eq!(cat::select_antenna(9).as_deref(), Some("^AN09;"));
        assert_eq!(cat::select_antenna(32).as_deref(), Some("^AN32;"));
        assert_eq!(cat::select_antenna(0), None);
        assert_eq!(cat::select_antenna(33), None);
    }

    /// Each telemetry RESP populates the matching field.
    /// trace: FR-AMP-02
    #[test]
    fn fr_amp_02_parses_each_telemetry_field() {
        let mut s = KpaState::default();
        apply(&mut s, "^OS1;^TM045;^FL00;^BN05;^AI1;^AMI;^AN02;^FS3;^TP0;");
        assert_eq!(s.mode, Some(Mode::Operate));
        assert_eq!(s.temp_c, Some(45));
        assert_eq!(s.fault, Some(0));
        assert_eq!(s.band, Some(5));
        assert_eq!(s.band_name(), Some("20 m"));
        assert_eq!(s.atu_inline, Some(true));
        assert_eq!(s.atu_mode_inline, Some(true));
        assert_eq!(s.antenna, Some(2));
        assert_eq!(s.fan, Some(3));
        assert_eq!(s.tuning, Some(false));
        assert!(!s.is_faulted());
    }

    /// `^WS` carries forward power and SWR together (`^WS1204 014;`).
    /// trace: FR-AMP-02
    #[test]
    fn fr_amp_02_ws_carries_power_and_swr() {
        let mut s = KpaState::default();
        assert_eq!(apply(&mut s, "^WS1204 014;"), 1);
        assert_eq!(s.forward_w, Some(1204));
        assert_eq!(s.swr_tenths, Some(14));
        assert_eq!(s.swr(), Some(1.4));
    }

    /// A fault code is decoded from hex and described; standby is reported.
    /// trace: FR-AMP-02
    #[test]
    fn fr_amp_02_fault_is_hex_and_described() {
        let mut s = KpaState::default();
        apply(&mut s, "^OS0;^FL91;");
        assert_eq!(s.mode, Some(Mode::Standby));
        assert_eq!(s.fault, Some(0x91));
        assert!(s.is_faulted());
        assert_eq!(
            s.fault_text(),
            Some("very high SWR (antenna not connected?)")
        );
        // No fault → no text.
        apply(&mut s, "^FL00;");
        assert!(!s.is_faulted());
        assert_eq!(s.fault_text(), None);
    }

    /// Identity, firmware and serial parse; RVM is not mistaken for RV.
    /// trace: FR-AMP-02
    #[test]
    fn fr_amp_02_identity_firmware_serial() {
        let mut s = KpaState::default();
        apply(&mut s, "^KPA1500;^RVM01.23;^SN00042;");
        assert!(s.identified);
        assert_eq!(s.firmware.as_deref(), Some("01.23"));
        assert_eq!(s.serial.as_deref(), Some("00042"));
        // Lower-case boot-block identity also counts.
        let mut boot = KpaState::default();
        apply(&mut boot, "^kpa1500;");
        assert!(boot.identified);
    }

    /// Antenna 10..=32 parse as two digits; band out of range is rejected.
    /// trace: FR-AMP-02
    #[test]
    fn fr_amp_02_parse_bounds() {
        let mut s = KpaState::default();
        apply(&mut s, "^AN27;");
        assert_eq!(s.antenna, Some(27));
        // Out-of-range band is ignored, not stored.
        assert_eq!(apply(&mut s, "^BN11;"), 0);
        assert_eq!(s.band, None);
    }

    /// Unknown, empty and fragmentary tokens are skipped without disturbing
    /// already-known fields.
    /// trace: FR-AMP-02
    #[test]
    fn fr_amp_02_ignores_noise_and_fragments() {
        let mut s = KpaState::default();
        apply(&mut s, "^OS1;");
        // A junk reply and a bare fragment change nothing.
        assert_eq!(apply(&mut s, "^ZZ9;;^OS"), 0);
        assert_eq!(s.mode, Some(Mode::Operate));
        // A trailing partial before the final ';' is not applied.
        let mut p = KpaState::default();
        assert_eq!(apply(&mut p, "^TM050;^TM"), 1);
        assert_eq!(p.temp_c, Some(50));
    }

    /// The poll and ident strings are well-formed `^…;` command runs.
    /// trace: FR-AMP-02
    #[test]
    fn fr_amp_02_poll_strings_well_formed() {
        for s in [POLL, IDENT] {
            assert!(s.starts_with('^'));
            assert!(s.ends_with(';'));
            // Every ';'-delimited command begins with a caret.
            for cmd in s.split_inclusive(';').filter(|c| !c.is_empty()) {
                assert!(cmd.starts_with('^'), "malformed command in {s:?}: {cmd:?}");
            }
        }
    }
}
