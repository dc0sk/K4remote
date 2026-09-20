//! The line codec for DX-cluster and Reverse Beacon Network telnet feeds (FR-SPOT-07): parse a
//! spot line into a [`Spot`], split a telnet byte stream into lines under hard bounds, and shed
//! floods. Pure and offline — the socket lives elsewhere.
//!
//! **Everything here handles untrusted input.** A feed is a stranger's text arriving as fast as
//! it likes, so a line that is malformed, oversized or hostile is *rejected*, never repaired or
//! truncated into a different one, and no input can make memory grow without bound.
//!
//! The line format is the standard cluster one, `DX de <spotter>: <freq kHz> <call> <comment>
//! <HHMM>Z`, and skimmer spots carry `<mode> <snr> dB <speed> WPM|BPS CQ` in the comment. It is
//! read as whitespace-separated tokens, not fixed columns, so the padding a given server uses does
//! not matter. Sources: `docs/references/external-references.md`, R-EXT-05.

use crate::{Network, Spot};

/// Longest line accepted, bytes. A real cluster line is about 80 characters; longer than this is
/// not a spot.
pub const MAX_LINE_BYTES: usize = 256;

/// Largest frequency accepted, kHz (300 GHz): well past anything a cluster spots.
const MAX_FREQ_KHZ: f64 = 300_000_000.0;

/// A spot stamped this far ahead of `now` is taken to be from yesterday (the feed carries only a
/// time of day), so a slightly fast remote clock does not push a fresh spot back a day.
const FUTURE_SLACK_SECS: u64 = 300;

/// Turn a time of day into the Unix time of the most recent such moment not (meaningfully) in the
/// future. `secs_of_day` is seconds since 00:00 UTC.
pub fn spot_time(now: u64, secs_of_day: u32) -> u64 {
    let day_start = now - now % 86_400;
    let t = day_start + u64::from(secs_of_day);
    if t > now + FUTURE_SLACK_SECS {
        t.saturating_sub(86_400)
    } else {
        t
    }
}

/// `"2144Z"` → seconds since midnight.
fn parse_z(token: &str) -> Option<u32> {
    let b = token.as_bytes();
    if b.len() != 5 || b[4] != b'Z' || !b[..4].iter().all(u8::is_ascii_digit) {
        return None;
    }
    let n = |i: usize| u32::from(b[i] - b'0');
    let (h, m) = (n(0) * 10 + n(1), n(2) * 10 + n(3));
    (h < 24 && m < 60).then_some(h * 3600 + m * 60)
}

/// A frequency token in kHz (`"7000.7"`) → Hz. Only digits and one dot: no sign, exponent, `inf`
/// or `nan`.
fn parse_khz(token: &str) -> Option<u64> {
    let dots = token.bytes().filter(|&b| b == b'.').count();
    if token.is_empty() || dots > 1 || !token.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let khz: f64 = token.parse().ok()?;
    if !khz.is_finite() || khz <= 0.0 || khz > MAX_FREQ_KHZ {
        return None;
    }
    let hz = (khz * 1000.0).round() as u64;
    (hz > 0).then_some(hz)
}

/// The spotter as the cluster names it: letters, digits, `/`, `-` and `#` (skimmers are `CALL-#`).
fn spotter_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 16
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'#'))
}

/// Parse one cluster line into a spot, or `None` if it is not a well-formed spot.
///
/// `now` is the current Unix time; a line carries only a time of day, so the date comes from it
/// ([`spot_time`]). A skimmer comment (`CW 35 dB 26 WPM CQ`) yields the mode and SNR; a
/// hand-entered spot's comment is kept as text and yields neither.
pub fn parse_line(line: &str, now: u64, network: Network) -> Option<Spot> {
    if line.len() > MAX_LINE_BYTES {
        return None;
    }
    let rest = line.strip_prefix("DX de ")?;
    let (spotter, rest) = rest.split_once(':')?;
    let spotter = spotter.trim();
    if !spotter_ok(spotter) {
        return None;
    }
    let mut tokens: Vec<&str> = rest.split_whitespace().collect();
    if tokens.len() < 3 {
        return None; // need at least: frequency, callsign, time
    }
    let secs = parse_z(tokens.pop()?)?;
    let freq_hz = parse_khz(tokens[0])?;
    let mut spot =
        Spot::new(tokens[1], freq_hz, spot_time(now, secs), network)?.with_spotter(spotter);

    let comment = &tokens[2..];
    if !comment.is_empty() {
        spot = spot.with_comment(&comment.join(" "));
        // Skimmer style: `<mode> <snr> dB ...`.
        if let Some(db) = comment.iter().position(|t| *t == "dB") {
            if db >= 2 {
                let mode = comment[0];
                if (2..=8).contains(&mode.len())
                    && mode
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                {
                    spot = spot.with_mode(mode);
                }
                if let Some(snr) = comment[db - 1]
                    .parse::<i16>()
                    .ok()
                    .filter(|s| (-60..=200).contains(s))
                {
                    spot = spot.with_snr(snr);
                }
            }
        }
    }
    Some(spot)
}

// ---------------------------------------------------------------------------------------------
// Splitting a telnet byte stream into lines

const IAC: u8 = 255;
const SB: u8 = 250;
const SE: u8 = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Telnet {
    Data,
    /// Saw IAC; the next byte is a command.
    Command,
    /// Saw IAC WILL/WONT/DO/DONT; the next byte is the option, to be skipped.
    Option,
    /// Inside IAC SB … IAC SE, being skipped.
    Subneg,
    /// Inside a subnegotiation, just saw IAC.
    SubnegIac,
}

/// Splits bytes from a telnet connection into text lines.
///
/// Telnet option negotiation (`IAC …`) is stripped; a server may send it at any point. Bounded:
/// a line longer than [`MAX_LINE_BYTES`] is **discarded to its end**, not cut short and parsed, and
/// the buffer never holds more than that however much arrives without a newline.
#[derive(Debug)]
pub struct LineSplitter {
    buf: Vec<u8>,
    state: Telnet,
    /// Throwing away an overlong line until its newline.
    discarding: bool,
    /// Lines discarded for being too long.
    pub overlong: u64,
}

impl Default for LineSplitter {
    fn default() -> Self {
        Self {
            buf: Vec::new(),
            state: Telnet::Data,
            discarding: false,
            overlong: 0,
        }
    }
}

impl LineSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; complete lines are appended to `out` (without their line ending).
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<String>) {
        for &b in bytes {
            match self.state {
                Telnet::Command => {
                    self.state = match b {
                        IAC => {
                            self.data(IAC, out); // an escaped 0xFF is a data byte
                            Telnet::Data
                        }
                        251..=254 => Telnet::Option, // WILL / WONT / DO / DONT
                        SB => Telnet::Subneg,
                        _ => Telnet::Data, // a one-byte command
                    };
                }
                Telnet::Option => self.state = Telnet::Data,
                Telnet::Subneg => {
                    if b == IAC {
                        self.state = Telnet::SubnegIac;
                    }
                }
                Telnet::SubnegIac => {
                    self.state = if b == SE {
                        Telnet::Data
                    } else {
                        Telnet::Subneg
                    };
                }
                Telnet::Data => {
                    if b == IAC {
                        self.state = Telnet::Command;
                    } else {
                        self.data(b, out);
                    }
                }
            }
        }
    }

    fn data(&mut self, b: u8, out: &mut Vec<String>) {
        match b {
            b'\n' => {
                if self.discarding {
                    self.discarding = false;
                } else {
                    if self.buf.last() == Some(&b'\r') {
                        self.buf.pop();
                    }
                    out.push(String::from_utf8_lossy(&self.buf).into_owned());
                }
                self.buf.clear();
            }
            0 => {} // telnet sends CR NUL for a bare carriage return
            _ if self.discarding => {}
            _ => {
                if self.buf.len() >= MAX_LINE_BYTES {
                    self.discarding = true;
                    self.overlong += 1;
                    self.buf.clear();
                } else {
                    self.buf.push(b);
                }
            }
        }
    }

    /// Throw away the unfinished line. After answering a prompt (which has no newline) the prompt
    /// text is still pending, and the server's next line would be glued onto it and lost.
    pub fn discard_pending(&mut self) {
        self.buf.clear();
        self.discarding = false;
    }

    /// The unfinished line so far, as text — where a login prompt (`login: `, which ends without a
    /// newline) shows up.
    pub fn pending(&self) -> String {
        String::from_utf8_lossy(&self.buf).into_owned()
    }
}

/// Whether the unfinished line looks like a login prompt asking for a callsign.
///
/// The RBN relay says exactly `Please enter your call: ` (observed 2026-09-19, R-EXT-05) and sends no
/// newline after it, so this reads [`LineSplitter::pending`]. The check is deliberately loose — a
/// short line ending in a colon that mentions `call` or `login` — because retail clusters word it
/// differently (`login:`, `Enter your callsign:`) and no documentation gives their text.
pub fn is_call_prompt(pending: &str) -> bool {
    let p = pending.trim_end();
    if p.is_empty() || p.len() > 80 || !p.ends_with(':') {
        return false;
    }
    let lower = p.to_ascii_lowercase();
    lower.contains("call") || lower.contains("login")
}

// ---------------------------------------------------------------------------------------------
// Shedding a flood

/// A token bucket: at most `per_sec` spots a second, with a burst of one second's worth. The
/// unfiltered RBN relay can carry far more than a desktop needs; what exceeds the rate is shed and
/// counted rather than queued.
#[derive(Debug)]
pub struct RateGate {
    per_sec: u32,
    /// Available tokens, in thousandths of a spot.
    milli: u64,
    last_ms: Option<u64>,
    /// Spots shed for being over the rate.
    pub shed: u64,
}

impl RateGate {
    pub fn new(per_sec: u32) -> Self {
        Self {
            per_sec,
            milli: u64::from(per_sec) * 1000,
            last_ms: None,
            shed: 0,
        }
    }

    /// Whether a spot arriving at `now_ms` (any monotonic millisecond clock) is admitted.
    pub fn allow(&mut self, now_ms: u64) -> bool {
        let cap = u64::from(self.per_sec) * 1000;
        if let Some(last) = self.last_ms {
            let dt = now_ms.saturating_sub(last);
            self.milli = (self.milli + dt * u64::from(self.per_sec)).min(cap);
        }
        self.last_ms = Some(now_ms);
        if self.milli >= 1000 {
            self.milli -= 1000;
            true
        } else {
            self.shed += 1;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-09-19 12:00:00 UTC.
    const NOON: u64 = 1_789_819_200;

    /// FR-SPOT-07: the three sample lines from the AR-Cluster manual — a hand-entered spot, a CW
    /// skimmer spot and a BPSK skimmer spot — parse to the right fields; the time of day resolves to
    /// the right date, including across midnight; and lines that are not well-formed spots are
    /// rejected.
    /// trace: FR-SPOT-07
    #[test]
    fn fr_spot_07_cluster_line_parse() {
        assert_eq!(NOON % 86_400, 43_200, "test setup: NOON is 12:00 UTC");

        // Hand-entered.
        let s = parse_line(
            "DX de KC7ZON:     18140.0    WP3GW         Centenial 30 pts             +KP4 2132Z",
            NOON,
            Network::DxCluster,
        )
        .expect("hand-entered spot");
        assert_eq!((s.call.as_str(), s.freq_hz), ("WP3GW", 18_140_000));
        assert_eq!(s.spotter.as_deref(), Some("KC7ZON"));
        assert_eq!(s.comment.as_deref(), Some("Centenial 30 pts +KP4"));
        assert_eq!(
            (s.mode, s.snr_db),
            (None, None),
            "free text is not a mode or an SNR"
        );
        assert_eq!(s.network, Network::DxCluster);
        assert_eq!(
            s.time,
            NOON - 43_200 - 86_400 + 21 * 3600 + 32 * 60,
            "21:32Z is yesterday's, 12 hours before noon's midnight"
        );

        // CW skimmer.
        let s = parse_line(
            "DX de K1TTT-#:      7000.7   NP2X          CW 35 dB 26 WPM CQ           +KP2 2144Z",
            NOON,
            Network::Rbn,
        )
        .expect("CW skimmer spot");
        assert_eq!((s.call.as_str(), s.freq_hz), ("NP2X", 7_000_700));
        assert_eq!(s.spotter.as_deref(), Some("K1TTT-#"));
        assert_eq!((s.mode.as_deref(), s.snr_db), (Some("CW"), Some(35)));

        // BPSK skimmer, a different column layout.
        let s = parse_line(
            "DX de DL9GTB-#:     3580.9   OM7CM         BPSK 20 dB 63 BPS CQ           OM 2144Z",
            NOON,
            Network::Rbn,
        )
        .expect("BPSK skimmer spot");
        assert_eq!((s.call.as_str(), s.freq_hz), ("OM7CM", 3_580_900));
        assert_eq!((s.mode.as_deref(), s.snr_db), (Some("BPSK"), Some(20)));

        // Column padding is irrelevant: the same spot with single spaces.
        let a = parse_line(
            "DX de K1TTT-#: 7000.7 NP2X CW 35 dB 26 WPM CQ 2144Z",
            NOON,
            Network::Rbn,
        )
        .unwrap();
        assert_eq!(
            (a.call.as_str(), a.freq_hz, a.snr_db),
            ("NP2X", 7_000_700, Some(35))
        );
        // A negative SNR (FT8 reports go below zero).
        let a = parse_line(
            "DX de W3LPL-#: 14074.0 JA1ABC FT8 -12 dB 0 BPS CQ 1150Z",
            NOON,
            Network::Rbn,
        )
        .unwrap();
        assert_eq!((a.mode.as_deref(), a.snr_db), (Some("FT8"), Some(-12)));

        // The date: the feed carries only a time of day.
        let at = |z: &str| {
            parse_line(
                &format!("DX de K1TTT-#: 7000.7 NP2X CW 30 dB 20 WPM CQ {z}"),
                NOON,
                Network::Rbn,
            )
            .map(|s| s.time)
        };
        assert_eq!(at("1159Z"), Some(NOON - 60), "a minute ago, today");
        assert_eq!(
            at("1204Z"),
            Some(NOON + 240),
            "a little in the future is clock skew, still today"
        );
        assert_eq!(
            at("1206Z"),
            Some(NOON + 360 - 86_400),
            "further ahead is yesterday's"
        );
        assert_eq!(at("0000Z"), Some(NOON - 43_200));
        // Across midnight: 00:02 UTC, a spot stamped 23:59Z is yesterday's, three minutes old.
        let just_after_midnight = NOON - 43_200 + 120;
        let s = parse_line(
            "DX de K1TTT-#: 7000.7 NP2X CW 30 dB 20 WPM CQ 2359Z",
            just_after_midnight,
            Network::Rbn,
        )
        .unwrap();
        assert_eq!(just_after_midnight - s.time, 180);
        assert_eq!(spot_time(NOON, 0), NOON - 43_200);

        // Not spots.
        for bad in [
            "",
            "login: ",
            "Welcome to the cluster",
            "WWV de W0MU <18>: SFI=150",
            "DX de K1TTT-#:",             // nothing after the colon
            "DX de K1TTT-#: 7000.7 NP2X", // no time
            "DX de K1TTT-#: 7000.7 NP2X CW 30 dB 20 WPM CQ 2560Z", // minute 60
            "DX de K1TTT-#: 7000.7 NP2X CW 30 dB 20 WPM CQ 2400Z", // hour 24
            "DX de K1TTT-#: 7000.7 NP2X CW 30 dB 20 WPM CQ 12:00Z", // wrong shape
            "DX de K1TTT-#: 7000.7 NP2X CW 30 dB 20 WPM CQ 1200", // no Z
            "DX de K1TTT-#: 7000.7 CQ CW 30 dB 20 WPM CQ 1200Z", // 'CQ' is not a callsign
            "DX de K1TTT-#: abc NP2X CW 30 dB 20 WPM CQ 1200Z", // frequency not a number
            "DX de K1TTT-#: -7000.7 NP2X CW 30 dB 20 WPM CQ 1200Z", // negative
            "DX de K1TTT-#: 1e5 NP2X CW 30 dB 20 WPM CQ 1200Z", // exponent form
            "DX de K1TTT-#: inf NP2X CW 30 dB 20 WPM CQ 1200Z",
            "DX de K1TTT-#: nan NP2X CW 30 dB 20 WPM CQ 1200Z",
            "DX de K1TTT-#: 0 NP2X CW 30 dB 20 WPM CQ 1200Z", // zero
            "DX de K1TTT-#: 7000.7.1 NP2X CW 30 dB 20 WPM CQ 1200Z", // two dots
            "DX de K1TTT-#: 999999999999 NP2X CW 30 dB 20 WPM CQ 1200Z", // past 300 GHz
            "DX de K1\u{0}TTT-#: 7000.7 NP2X CW 30 dB 20 WPM CQ 1200Z", // control in the spotter
            "DX de K1TTT-#: 7000.7 NP\u{0421}X CW 30 dB 20 WPM CQ 1200Z", // Cyrillic look-alike in the call
            "DX de K1TTT-#: 7000.7 NP2X\u{202e} CW 30 dB 20 WPM CQ 1200Z", // bidi override in the call
            "dx de K1TTT-#: 7000.7 NP2X CW 30 dB 20 WPM CQ 1200Z",         // the prefix is exact
        ] {
            assert!(
                parse_line(bad, NOON, Network::Rbn).is_none(),
                "should reject {bad:?}"
            );
        }

        // A bad comment costs the comment, not the spot.
        let s = parse_line(
            "DX de K1TTT-#: 7000.7 NP2X caf\u{e9} \u{202e}x 1200Z",
            NOON,
            Network::DxCluster,
        )
        .unwrap();
        assert_eq!(s.comment, None);
        assert_eq!(s.call, "NP2X");
        // An absurd SNR is dropped, the spot kept.
        let s = parse_line(
            "DX de K1TTT-#: 7000.7 NP2X CW 999 dB 20 WPM CQ 1200Z",
            NOON,
            Network::Rbn,
        )
        .unwrap();
        assert_eq!(s.snr_db, None);
    }

    /// FR-SPOT-07: the login prompt the RBN relay actually sends is recognised, so are the usual
    /// wordings of a retail cluster, and ordinary lines are not mistaken for one.
    /// trace: FR-SPOT-07
    #[test]
    fn fr_spot_07_login_prompt() {
        // Observed from telnet.reversebeacon.net:7000 on 2026-09-19: 24 bytes, no newline.
        assert!(is_call_prompt("Please enter your call: "));
        // Retail-cluster wordings (not observed; typical).
        assert!(is_call_prompt("login: "));
        assert!(is_call_prompt("Enter your callsign:"));
        assert!(is_call_prompt("callsign: "));
        // Through the splitter, as the source will see it.
        let mut sp = LineSplitter::new();
        let mut out = Vec::new();
        sp.push(b"Please enter your call: ", &mut out);
        assert!(out.is_empty());
        assert!(is_call_prompt(&sp.pending()));

        for not in [
            "",
            "   ",
            "Welcome to the cluster:",
            "Please enter your call",                                   // no colon
            "DX de K1TTT-#: 7000.7 NP2X CW 30 dB 20 WPM CQ 1200Z",       // a spot, not a prompt
            "hello DX de K1TTT-#: callsign is NP2X and this line is far too long to be any prompt at all: ",
        ] {
            assert!(!is_call_prompt(not), "should not be a prompt: {not:?}");
        }
    }

    /// FR-SPOT-07: nothing a feed sends can make memory grow without bound or crash the parser.
    /// An over-long line is discarded whole, not cut and parsed; telnet negotiation is stripped;
    /// lines split across reads and CRLF/LF/CR-NUL endings all work; and a flood is shed at the
    /// rate cap and counted.
    /// trace: FR-SPOT-07
    #[test]
    fn fr_spot_07_bounds() {
        let good = "DX de K1TTT-#: 7000.7 NP2X CW 35 dB 26 WPM CQ 1144Z";

        // A line over the cap, in a parse-able shape: rejected, not truncated into a shorter spot.
        let long = format!(
            "DX de K1TTT-#: 7000.7 NP2X {} 1144Z",
            "x".repeat(MAX_LINE_BYTES)
        );
        assert!(long.len() > MAX_LINE_BYTES);
        assert!(parse_line(&long, NOON, Network::Rbn).is_none());

        // Splitting: LF, CRLF, and a line split across reads.
        let mut sp = LineSplitter::new();
        let mut out = Vec::new();
        sp.push(format!("{good}\r\nsecond line\nthi").as_bytes(), &mut out);
        assert_eq!(out, [good, "second line"]);
        sp.push(b"rd\r\n", &mut out);
        assert_eq!(out.last().map(String::as_str), Some("third"));
        assert_eq!(sp.pending(), "", "nothing pending after a complete line");

        // After answering a prompt the leftover text must not swallow the next line.
        let mut sp = LineSplitter::new();
        let mut out = Vec::new();
        sp.push(b"Please enter your call: ", &mut out);
        sp.discard_pending();
        sp.push(format!("{good}\r\n").as_bytes(), &mut out);
        assert_eq!(out, [good], "the first line after the prompt arrives whole");

        // A prompt has no newline: it is visible as pending text.
        let mut sp = LineSplitter::new();
        let mut out = Vec::new();
        sp.push(b"Welcome\r\nPlease enter your call: ", &mut out);
        assert_eq!(out, ["Welcome"]);
        assert_eq!(sp.pending(), "Please enter your call: ");

        // Telnet negotiation, anywhere: WILL ECHO, DO TTYPE, a subnegotiation, an escaped 0xFF.
        let mut sp = LineSplitter::new();
        let mut out = Vec::new();
        let mut bytes = vec![255, 251, 1, 255, 253, 24];
        bytes.extend_from_slice(b"log");
        bytes.extend_from_slice(&[255, 250, 24, 1, 255, 240]); // IAC SB TTYPE SEND IAC SE, mid-word
        bytes.extend_from_slice(b"in:\r\n");
        sp.push(&bytes, &mut out);
        assert_eq!(
            out,
            ["login:"],
            "negotiation is stripped and the text is intact"
        );
        let mut out = Vec::new();
        sp.push(&[b'a', 255, 255, b'b', b'\n'], &mut out);
        assert_eq!(out.len(), 1);
        assert!(
            out[0].starts_with('a') && out[0].ends_with('b') && out[0].contains('\u{fffd}'),
            "an escaped 0xFF is data, and not valid text"
        );
        // CR NUL (a bare carriage return in telnet) does not corrupt the line.
        let mut out = Vec::new();
        sp.push(b"ab\r\0\n", &mut out);
        assert_eq!(out, ["ab"]);

        // An over-long line is discarded to its end and counted; the next line is fine.
        let mut sp = LineSplitter::new();
        let mut out = Vec::new();
        sp.push("y".repeat(MAX_LINE_BYTES + 50).as_bytes(), &mut out);
        assert!(out.is_empty());
        assert_eq!(sp.overlong, 1);
        sp.push(format!("still junk\n{good}\n").as_bytes(), &mut out);
        assert_eq!(
            out,
            [good],
            "the rest of the long line is thrown away, then normal service"
        );

        // Memory: ten megabytes with no newline never grows the buffer past the cap.
        let mut sp = LineSplitter::new();
        let mut out = Vec::new();
        let chunk = vec![b'z'; 64 * 1024];
        for _ in 0..160 {
            sp.push(&chunk, &mut out);
            assert!(sp.pending().len() <= MAX_LINE_BYTES);
        }
        assert!(out.is_empty());
        assert_eq!(
            sp.overlong, 1,
            "one endless line is one overlong event, not thousands"
        );

        // Flood: 10 000 spots in one millisecond against 200/s admits one second's burst and no more.
        let mut gate = RateGate::new(200);
        let admitted = (0..10_000).filter(|_| gate.allow(1_000)).count();
        assert_eq!(admitted, 200);
        assert_eq!(gate.shed, 9_800);
        // Half a second later, half a second's worth is available again.
        let later = (0..10_000).filter(|_| gate.allow(1_500)).count();
        assert_eq!(later, 100);
        // Steady arrivals under the rate are never shed.
        let mut gate = RateGate::new(200);
        assert!(
            (0..2_000).all(|i| gate.allow(i * 10)),
            "100/s against a 200/s cap"
        );
        assert_eq!(gate.shed, 0);
        // A zero rate admits nothing; a clock that goes backwards does not panic or mint tokens.
        let mut gate = RateGate::new(0);
        assert!(!gate.allow(5));
        let mut gate = RateGate::new(10);
        assert!(gate.allow(10_000));
        let _ = gate.allow(5);
        assert!(
            !(0..100).all(|_| gate.allow(5)),
            "no free tokens from a backwards clock"
        );
    }
}
