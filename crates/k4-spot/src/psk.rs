//! PSK Reporter's live feed (FR-SPOT-05): turn a message from `mqtt.pskreporter.info` into a
//! [`Spot`], and work out which band topics to subscribe to. Pure and offline.
//!
//! The format is from PSK Reporter's own MQTT page and from observing the live feed once
//! (`docs/references/external-references.md`, R-EXT-05): a topic
//! `pskr/filter/v2/{band}/{mode}/{tx_call}/{rx_call}/…` and a **flat** JSON payload with `f`
//! (frequency, Hz), `md` (mode), `rp` (SNR, dB), `t` (epoch seconds), `sc` (sender callsign), `rc`
//! (receiver callsign) and more.
//!
//! The payload is **untrusted**. It is read by a small strict scanner, not a general JSON parser: a
//! nested value, an escape sequence, a duplicated field, or anything past a size cap rejects the
//! whole message, so nothing here can be made to allocate a lot or to read one field two ways.

use crate::{Network, Spot};

/// Largest message read, bytes. Real ones are about 200.
pub const MAX_PAYLOAD: usize = 1024;

/// Longest key or string value read.
const MAX_TOKEN: usize = 64;

/// Largest frequency accepted, Hz (300 GHz).
const MAX_FREQ_HZ: u64 = 300_000_000_000;

/// A spot stamped later than `now` is taken to be `now`: a hostile or wrong clock must not make a
/// spot outlive the age limit by claiming to be from the future.
fn clamp_time(t: u64, now: u64) -> u64 {
    t.min(now)
}

enum Value<'a> {
    Str(&'a str),
    Num(&'a str),
    Other,
}

struct Scanner<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Scanner<'a> {
    fn ws(&mut self) {
        while self
            .s
            .get(self.i)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) -> Option<()> {
        (self.s.get(self.i) == Some(&b)).then(|| self.i += 1)
    }

    /// A string with no escapes and no control characters.
    fn string(&mut self) -> Option<&'a str> {
        self.eat(b'"')?;
        let start = self.i;
        loop {
            let b = *self.s.get(self.i)?;
            if b == b'"' {
                break;
            }
            if b == b'\\' || b < 0x20 || self.i - start >= MAX_TOKEN {
                return None;
            }
            self.i += 1;
        }
        let out = std::str::from_utf8(&self.s[start..self.i]).ok();
        self.i += 1;
        out
    }

    fn value(&mut self) -> Option<Value<'a>> {
        match *self.s.get(self.i)? {
            b'"' => self.string().map(Value::Str),
            b'-' | b'0'..=b'9' => {
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|b| matches!(b, b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'))
                {
                    self.i += 1;
                    if self.i - start > 24 {
                        return None;
                    }
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .ok()
                    .map(Value::Num)
            }
            b't' | b'f' | b'n' => {
                for lit in [&b"true"[..], b"false", b"null"] {
                    if self.s[self.i..].starts_with(lit) {
                        self.i += lit.len();
                        return Some(Value::Other);
                    }
                }
                None
            }
            _ => None, // an object, an array, or garbage: not a flat message
        }
    }
}

/// The fields of a message that are read, as the text they were given in.
#[derive(Default)]
struct Fields<'a> {
    f: Option<&'a str>,
    md: Option<&'a str>,
    rp: Option<&'a str>,
    t: Option<&'a str>,
    sc: Option<&'a str>,
    rc: Option<&'a str>,
}

/// Parse one feed message into a spot, or `None` if it is not a well-formed one.
///
/// A field that fails validation but is not essential (mode, SNR, the receiver) is dropped and the
/// spot kept; a missing or invalid frequency, time or sender callsign loses the spot.
pub fn parse_payload(payload: &[u8], now: u64) -> Option<Spot> {
    if payload.len() > MAX_PAYLOAD {
        return None;
    }
    let mut sc = Scanner { s: payload, i: 0 };
    sc.ws();
    sc.eat(b'{')?;

    let mut got = Fields::default();
    let mut first = true;
    loop {
        sc.ws();
        // A closing brace is only valid straight after the opening one (an empty object): after a
        // comma a key must follow, so a trailing comma is refused.
        if first && sc.eat(b'}').is_some() {
            break;
        }
        first = false;
        let key = sc.string()?;
        sc.ws();
        sc.eat(b':')?;
        sc.ws();
        let value = sc.value()?;
        let slot = match key {
            "f" => Some(&mut got.f),
            "md" => Some(&mut got.md),
            "rp" => Some(&mut got.rp),
            "t" => Some(&mut got.t),
            "sc" => Some(&mut got.sc),
            "rc" => Some(&mut got.rc),
            _ => None,
        };
        if let Some(slot) = slot {
            let text = match value {
                Value::Str(s) | Value::Num(s) => s,
                Value::Other => return None,
            };
            if slot.replace(text).is_some() {
                return None; // a field given twice: ambiguous, refuse
            }
        }
        sc.ws();
        if sc.eat(b',').is_some() {
            continue;
        }
        sc.eat(b'}')?;
        break;
    }
    sc.ws();
    if sc.i != payload.len() {
        return None; // anything after the object
    }

    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let freq: u64 = got.f.filter(|s| digits(s))?.parse().ok()?;
    if freq == 0 || freq > MAX_FREQ_HZ {
        return None;
    }
    let time: u64 = got.t.filter(|s| digits(s))?.parse().ok()?;
    let mut spot = Spot::new(got.sc?, freq, clamp_time(time, now), Network::PskReporter)?;
    if let Some(mode) = got.md {
        spot = spot.with_mode(mode);
    }
    if let Some(snr) = got
        .rp
        .and_then(|s| s.parse::<i16>().ok())
        .filter(|s| (-60..=200).contains(s))
    {
        spot = spot.with_snr(snr);
    }
    if let Some(rc) = got.rc {
        spot = spot.with_spotter(rc);
    }
    Some(spot)
}

/// An amateur band as PSK Reporter names it in a topic, with the frequency range it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Band {
    pub token: &'static str,
    pub lo_hz: u64,
    pub hi_hz: u64,
}

/// The bands a K4 covers. Tokens `160m`, `80m`, `40m`, `30m`, `20m`, `17m`, `15m`, `12m`, `10m` and
/// `2m` were **seen on the live feed** (2026-09-20); **`60m` and `6m` were not seen** and are
/// assumed to follow the same `<n>m` pattern — unverified, and a wrong token would simply deliver
/// nothing for that band.
pub const BANDS: &[Band] = &[
    Band {
        token: "160m",
        lo_hz: 1_800_000,
        hi_hz: 2_000_000,
    },
    Band {
        token: "80m",
        lo_hz: 3_500_000,
        hi_hz: 4_000_000,
    },
    Band {
        token: "60m",
        lo_hz: 5_060_000,
        hi_hz: 5_450_000,
    },
    Band {
        token: "40m",
        lo_hz: 7_000_000,
        hi_hz: 7_300_000,
    },
    Band {
        token: "30m",
        lo_hz: 10_100_000,
        hi_hz: 10_150_000,
    },
    Band {
        token: "20m",
        lo_hz: 14_000_000,
        hi_hz: 14_350_000,
    },
    Band {
        token: "17m",
        lo_hz: 18_068_000,
        hi_hz: 18_168_000,
    },
    Band {
        token: "15m",
        lo_hz: 21_000_000,
        hi_hz: 21_450_000,
    },
    Band {
        token: "12m",
        lo_hz: 24_890_000,
        hi_hz: 24_990_000,
    },
    Band {
        token: "10m",
        lo_hz: 28_000_000,
        hi_hz: 29_700_000,
    },
    Band {
        token: "6m",
        lo_hz: 50_000_000,
        hi_hz: 54_000_000,
    },
    Band {
        token: "2m",
        lo_hz: 144_000_000,
        hi_hz: 148_000_000,
    },
];

/// The band a frequency is in, if it is in one.
pub fn band_token(freq_hz: u64) -> Option<&'static str> {
    BANDS
        .iter()
        .find(|b| (b.lo_hz..=b.hi_hz).contains(&freq_hz))
        .map(|b| b.token)
}

/// Every band that overlaps `[lo, hi]` Hz. An empty range (`lo > hi`) overlaps none.
pub fn bands_overlapping(lo_hz: u64, hi_hz: u64) -> Vec<&'static str> {
    if lo_hz > hi_hz {
        return Vec::new();
    }
    BANDS
        .iter()
        .filter(|b| b.lo_hz <= hi_hz && b.hi_hz >= lo_hz)
        .map(|b| b.token)
        .collect()
}

/// The topic that delivers everything on one band.
pub fn topic_for_band(band: &str) -> String {
    format!("pskr/filter/v2/{band}/#")
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_789_899_200;

    /// A message shaped exactly like the live feed's (observed 2026-09-20), callsigns replaced.
    const LIVE: &str = r#"{"sq":72916355844,"f":14074742,"md":"FT8","rp":-9,"t":1789899105,"t_tx":1789899090,"sc":"AA1AAA","sl":"FN31pr","rc":"BB2BBB","rl":"JO50ab","sa":291,"ra":230,"b":"20m"}"#;

    fn ok(s: &str) -> bool {
        parse_payload(s.as_bytes(), NOW).is_some()
    }

    /// FR-SPOT-05: a live-shaped message becomes the right spot, and field order, spacing and extra
    /// fields do not matter.
    /// trace: FR-SPOT-05
    #[test]
    fn fr_spot_05_pskreporter_payload_parse() {
        let s = parse_payload(LIVE.as_bytes(), NOW).expect("a live-shaped message");
        assert_eq!(
            (s.call.as_str(), s.freq_hz, s.time),
            ("AA1AAA", 14_074_742, 1_789_899_105)
        );
        assert_eq!((s.mode.as_deref(), s.snr_db), (Some("FT8"), Some(-9)));
        assert_eq!(s.spotter.as_deref(), Some("BB2BBB"));
        assert_eq!(s.network, Network::PskReporter);

        // Reordered, spaced, extra and unknown fields, a bool and a null.
        let odd = r#"  { "b" : "40m" , "x":true, "y":null, "sc":"k1abc", "t": 1789899100, "f" : 7040021, "rp": 12, "md":"CW", "z":"whatever" }  "#;
        let s = parse_payload(odd.as_bytes(), NOW).unwrap();
        assert_eq!(
            (s.call.as_str(), s.freq_hz, s.snr_db),
            ("K1ABC", 7_040_021, Some(12))
        );

        // Only the essentials: frequency, time, sender.
        let s = parse_payload(br#"{"f":14074000,"t":1789899100,"sc":"W1AW"}"#, NOW).unwrap();
        assert_eq!((s.mode, s.snr_db, s.spotter), (None, None, None));

        // A bad optional field costs the field, not the spot.
        let s = parse_payload(
            br#"{"f":14074000,"t":1789899100,"sc":"W1AW","md":"FT8","rp":"loud","rc":"BB2BBB"}"#,
            NOW,
        )
        .unwrap();
        assert_eq!(
            (s.snr_db, s.mode.as_deref()),
            (None, Some("FT8")),
            "an unparseable SNR is dropped"
        );
        let s = parse_payload(
            br#"{"f":14074000,"t":1789899100,"sc":"W1AW","rp":999}"#,
            NOW,
        )
        .unwrap();
        assert_eq!(s.snr_db, None, "an absurd SNR is dropped");

        // A future timestamp cannot make a spot outlive the age limit: it is clamped to now.
        let s = parse_payload(br#"{"f":14074000,"t":9999999999,"sc":"W1AW"}"#, NOW).unwrap();
        assert_eq!(s.time, NOW);
    }

    /// FR-SPOT-05: hostile or malformed messages are rejected whole, and none can panic.
    /// trace: FR-SPOT-05
    #[test]
    fn fr_spot_05_pskreporter_payload_rejects_hostile_input() {
        assert!(
            ok(r#"{"f":14074000,"t":1789899100,"sc":"W1AW"}"#),
            "the control"
        );
        let bad_cases: &[&str] = &[
            "",
            "not json",
            "[]",
            "{",
            "}",
            r#"{"f":14074000,"t":1789899100,"sc":"W1AW""#,
            r#"{"f":14074000,"t":1789899100,"sc":"W1AW"} trailing"#,
            r#"{"f":14074000,"t":1789899100,"sc":"W1AW",}"#,
            r#"{"f":14074000,"t":1789899100,"sc":{"x":1}}"#,
            r#"{"f":14074000,"t":1789899100,"sc":"W1AW","x":[1,2]}"#,
            r#"{"t":1789899100,"sc":"W1AW"}"#,
            r#"{"f":14074000,"sc":"W1AW"}"#,
            r#"{"f":14074000,"t":1789899100}"#,
            r#"{"f":-14074000,"t":1789899100,"sc":"W1AW"}"#,
            r#"{"f":14074000.5,"t":1789899100,"sc":"W1AW"}"#,
            r#"{"f":1.4e7,"t":1789899100,"sc":"W1AW"}"#,
            r#"{"f":0,"t":1789899100,"sc":"W1AW"}"#,
            r#"{"f":999999999999999,"t":1789899100,"sc":"W1AW"}"#,
            r#"{"f":14074000,"f":7000000,"t":1789899100,"sc":"W1AW"}"#,
            r#"{"f":14074000,"t":1789899100,"sc":"W1AW","sc":"K1ABC"}"#,
            r#"{"f":14074000,"t":1789899100,"sc":"CQ"}"#,
            r#"{"f":true,"t":1789899100,"sc":"W1AW"}"#,
            r#"{"f":null,"t":1789899100,"sc":"W1AW"}"#,
        ];
        for bad in bad_cases {
            assert!(!ok(bad), "should reject {bad:?}");
        }
        // A JSON escape sequence anywhere is refused. Built from a backslash made from its byte
        // value, so no escape appears literally in this file (a tool once decoded them, silently
        // turning these two hostile cases into valid ones).
        let bs = char::from(0x5C_u8);
        let escaped_call = format!(r#"{{"f":14074000,"t":1789899100,"sc":"W1{bs}u0041W"}}"#);
        assert!(
            !ok(&escaped_call),
            "an escape in the callsign: {escaped_call}"
        );
        let escaped_mode =
            format!(r#"{{"f":14074000,"t":1789899100,"sc":"W1AW","md":"F{bs}u0054"}}"#);
        assert!(
            !ok(&escaped_mode),
            "an escape in an optional field still refuses the message"
        );
        let escaped_quote = format!(r#"{{"f":14074000,"t":1789899100,"sc":"W1AW","x":"a{bs}"b"}}"#);
        assert!(!ok(&escaped_quote));
        // Characters that are hard to type in a literal: build the messages from code points.
        for hostile in ['\u{0}', '\u{7}', '\u{202e}', '\u{200b}', '\u{0410}'] {
            let msg = format!(r#"{{"f":14074000,"t":1789899100,"sc":"W1{hostile}AW"}}"#);
            assert!(
                !ok(&msg),
                "should reject a callsign containing U+{:04X}",
                hostile as u32
            );
        }
        // A number given as a quoted string is accepted (the scanner reads text, not types).
        assert!(ok(r#"{"f":"14074000","t":1789899100,"sc":"W1AW"}"#));

        // Size: over the cap is refused outright; a long key or string is refused.
        assert!(parse_payload(&vec![b' '; MAX_PAYLOAD + 1], NOW).is_none());
        let long_val = format!(
            r#"{{"f":14074000,"t":1789899100,"sc":"W1AW","x":"{}"}}"#,
            "y".repeat(200)
        );
        assert!(!ok(&long_val));
        let long_key = format!(
            r#"{{"{}":1,"f":14074000,"t":1789899100,"sc":"W1AW"}}"#,
            "k".repeat(200)
        );
        assert!(!ok(&long_key));

        // Invalid UTF-8 and arbitrary bytes never panic.
        assert!(parse_payload(&[0xFF, 0xFE, b'{'], NOW).is_none());
        let mut s = 0x2545_F491_4F6C_DD1Du64;
        for n in 0..500usize {
            let junk: Vec<u8> = (0..n % 200)
                .map(|_| {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    (s >> 16) as u8
                })
                .collect();
            let _ = parse_payload(&junk, NOW);
        }
    }

    /// FR-SPOT-05: a frequency maps to its band's topic token, the bands overlapping a window are
    /// found, an empty window needs none, and the topic is the documented band-scoped wildcard.
    /// trace: FR-SPOT-05
    #[test]
    fn fr_spot_05_bands_and_topics() {
        // The frequencies observed on the live feed (2026-09-20) fall in the tokens they arrived with.
        for (hz, tok) in [
            (1_840_482, "160m"),
            (3_575_417, "80m"),
            (7_076_866, "40m"),
            (10_140_254, "30m"),
            (14_074_742, "20m"),
            (14_298_185, "20m"),
            (18_102_869, "17m"),
            (21_077_214, "15m"),
            (24_917_459, "12m"),
            (28_076_906, "10m"),
            (144_461_337, "2m"),
        ] {
            assert_eq!(band_token(hz), Some(tok), "{hz}");
        }
        assert_eq!(band_token(14_000_000), Some("20m"));
        assert_eq!(band_token(14_350_000), Some("20m"));
        assert_eq!(band_token(14_350_001), None);
        assert_eq!(band_token(13_999_999), None);
        assert_eq!(band_token(9_000_000), None, "outside every amateur band");

        assert_eq!(
            bands_overlapping(14_074_000 - 300_000, 14_074_000 + 300_000),
            ["20m"]
        );
        assert_eq!(bands_overlapping(9_900_000, 14_500_000), ["30m", "20m"]);
        assert_eq!(
            bands_overlapping(1, 0),
            Vec::<&str>::new(),
            "an empty window needs no band"
        );
        assert_eq!(
            bands_overlapping(30_000_000, 40_000_000),
            Vec::<&str>::new()
        );

        assert_eq!(topic_for_band("20m"), "pskr/filter/v2/20m/#");
    }
}
