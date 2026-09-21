//! POTA spots (FR-SPOT-08): turn the reply of `api.pota.app` into [`Spot`]s. Pure and offline.
//!
//! The format is from observing the live endpoint once (`docs/references/external-references.md`,
//! R-EXT-05) — POTA publishes no schema. The reply is a JSON **array** of flat objects. The fields
//! read are `activator` (the callsign), `frequency` (**a string, in kHz**, e.g. `"10136.0"`), `mode`,
//! `reference` (the park), `spotTime` (`2026-09-21T05:07:00`, no zone: read as UTC), `spotter`,
//! `comments` and `invalid`. Everything else is skipped.
//!
//! The reply is **untrusted**. It is read by a small strict scanner, not a general JSON parser:
//! anything that is not an array of flat objects, any nesting, invalid UTF-8 or a reply past a size
//! cap is an error the operator is shown (the format has changed, or something is not POTA), while a
//! single object that does not make a valid spot is counted and dropped without costing the others.

use crate::{sanitise_text, Network, Parsed, Spot};

/// Where the list of current spots is. From one observation of the live service — POTA documents
/// no schema and no path (R-EXT-05).
pub const URL: &str = "https://api.pota.app/spot/activator";

/// Largest reply read, bytes. A real one is about 5 KB for a dozen spots.
pub const MAX_BODY: usize = 512 * 1024;

/// Most objects read from one reply.
pub const MAX_SPOTS: usize = 2000;

/// Longest string read as a value. A longer one in a field that is used drops that field; the
/// strings that are only skipped are bounded by [`MAX_BODY`].
const MAX_TOKEN: usize = 64;

/// Lowest and highest frequency accepted, Hz. The floor also catches a reply that has switched to
/// megahertz: `"14.074"` would read as 14 kHz, which is no amateur spot.
const MIN_FREQ_HZ: u64 = 100_000;
const MAX_FREQ_HZ: u64 = 300_000_000_000;

enum Value<'a> {
    /// A string with no escape sequence in it.
    Str(&'a str),
    /// A string with escapes, or too long to use: skipped, never read as text.
    Skipped,
    Num(&'a str),
    Bool(bool),
    Null,
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

    fn eat(&mut self, b: u8) -> bool {
        let hit = self.s.get(self.i) == Some(&b);
        if hit {
            self.i += 1;
        }
        hit
    }

    fn string(&mut self) -> Option<Value<'a>> {
        if !self.eat(b'"') {
            return None;
        }
        let start = self.i;
        let mut escaped = false;
        loop {
            let b = *self.s.get(self.i)?;
            match b {
                b'"' => break,
                b'\\' => {
                    escaped = true;
                    self.i += 1;
                    match *self.s.get(self.i)? {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
                        b'u' => {
                            for _ in 0..4 {
                                self.i += 1;
                                if !self.s.get(self.i)?.is_ascii_hexdigit() {
                                    return None;
                                }
                            }
                        }
                        _ => return None,
                    }
                }
                0..=0x1f => return None,
                _ => {}
            }
            self.i += 1;
        }
        let raw = &self.s[start..self.i];
        self.i += 1;
        if escaped || raw.len() > MAX_TOKEN {
            return Some(Value::Skipped);
        }
        // The whole body was checked to be UTF-8 before scanning, and a slice between two ASCII
        // quotes of valid UTF-8 is valid.
        std::str::from_utf8(raw).ok().map(Value::Str)
    }

    fn value(&mut self) -> Option<Value<'a>> {
        match *self.s.get(self.i)? {
            b'"' => self.string(),
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
            b't' if self.s[self.i..].starts_with(b"true") => {
                self.i += 4;
                Some(Value::Bool(true))
            }
            b'f' if self.s[self.i..].starts_with(b"false") => {
                self.i += 5;
                Some(Value::Bool(false))
            }
            b'n' if self.s[self.i..].starts_with(b"null") => {
                self.i += 4;
                Some(Value::Null)
            }
            _ => None, // an object, an array, or garbage: not a flat object
        }
    }
}

/// The fields of one object that are read, as the text they were given in.
#[derive(Default)]
struct Fields<'a> {
    activator: Option<&'a str>,
    frequency: Option<&'a str>,
    mode: Option<&'a str>,
    reference: Option<&'a str>,
    spot_time: Option<&'a str>,
    spotter: Option<&'a str>,
    comments: Option<&'a str>,
    /// Set when a field that is needed was given twice, was not text, or `invalid` was not
    /// null/false: the object is refused, but scanning goes on to its end.
    bad: bool,
}

fn object<'a>(sc: &mut Scanner<'a>) -> Option<Fields<'a>> {
    if !sc.eat(b'{') {
        return None;
    }
    let mut got = Fields::default();
    sc.ws();
    if sc.eat(b'}') {
        return Some(got);
    }
    loop {
        sc.ws();
        let Value::Str(key) = sc.string()? else {
            return None; // an escaped or overlong key is not one of ours and is not trusted
        };
        sc.ws();
        if !sc.eat(b':') {
            return None;
        }
        sc.ws();
        let value = sc.value()?;
        let slot = match key {
            "activator" => Some(&mut got.activator),
            "frequency" => Some(&mut got.frequency),
            "mode" => Some(&mut got.mode),
            "reference" => Some(&mut got.reference),
            "spotTime" => Some(&mut got.spot_time),
            "spotter" => Some(&mut got.spotter),
            "comments" => Some(&mut got.comments),
            _ => None,
        };
        if let Some(slot) = slot {
            match value {
                Value::Str(t) | Value::Num(t) => {
                    if slot.replace(t).is_some() {
                        got.bad = true; // given twice: ambiguous, refuse the object
                    }
                }
                // Escaped or overlong text, a null or a bool where text belongs: the field is
                // absent as far as anything downstream is concerned.
                Value::Skipped | Value::Null | Value::Bool(_) => {}
            }
        } else if key == "invalid" {
            // Observed as null. Anything other than null or false is read as "flagged invalid".
            if !matches!(value, Value::Null | Value::Bool(false)) {
                got.bad = true;
            }
        }
        sc.ws();
        if sc.eat(b',') {
            continue;
        }
        if sc.eat(b'}') {
            return Some(got);
        }
        return None;
    }
}

/// A frequency given in kilohertz (`"10136.0"`, `"14074.742"`, `10136`) as hertz, or `None`: digits
/// with at most three after the point (hertz is the finest a spot can say), no sign, no exponent.
pub fn khz_to_hz(text: &str) -> Option<u64> {
    let (int, frac) = match text.split_once('.') {
        Some((i, f)) => (i, f),
        None => (text, ""),
    };
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if int.is_empty() || int.len() > 9 || !digits(int) || frac.len() > 3 || !digits(frac) {
        return None;
    }
    if text.ends_with('.') {
        return None;
    }
    let whole: u64 = int.parse().ok()?;
    let mut frac_hz: u64 = if frac.is_empty() {
        0
    } else {
        frac.parse().ok()?
    };
    for _ in frac.len()..3 {
        frac_hz *= 10;
    }
    let hz = whole * 1000 + frac_hz;
    (MIN_FREQ_HZ..=MAX_FREQ_HZ).contains(&hz).then_some(hz)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

/// `YYYY-MM-DDTHH:MM:SS`, optionally with fractional seconds and a trailing `Z`, as Unix seconds
/// **read as UTC** (the reply gives no zone; UTC is an assumption, recorded in R-EXT-05). Years
/// 2000–2100 only; any other offset, a space instead of `T`, or an impossible date is `None`.
pub fn parse_utc(text: &str) -> Option<u64> {
    let text = text.strip_suffix('Z').unwrap_or(text);
    let (main, frac) = match text.split_once('.') {
        Some((m, f)) => (m, Some(f)),
        None => (text, None),
    };
    if let Some(f) = frac {
        if f.is_empty() || f.len() > 9 || !f.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let b = main.as_bytes();
    // The separators are checked before the fields are sliced by byte position, so a multi-byte
    // character can never straddle a slice edge (`fr_spot_08_frequency_and_time_arithmetic`).
    if b.len() != 19
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
    {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<u64> {
        let s = &main[a..z];
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, s) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(2000..=2100).contains(&y) || !(1..=12).contains(&mo) || h > 23 || mi > 59 || s > 59 {
        return None;
    }
    let dim = match mo {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    };
    if d < 1 || d > dim {
        return None;
    }
    // Days from 1970-01-01, counting the years and months before this one.
    let mut days: u64 = 0;
    for yy in 1970..y {
        days += if is_leap(yy) { 366 } else { 365 };
    }
    for m in 1..mo {
        days += match m {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            _ if is_leap(y) => 29,
            _ => 28,
        };
    }
    days += d - 1;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

/// A park reference as POTA writes them (`CA-0040`, `US-1234`): 4–12 ASCII letters, digits and
/// hyphens, with at least one hyphen and one digit.
fn is_reference(text: &str) -> bool {
    (4..=12).contains(&text.len())
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && text.contains('-')
        && text.bytes().any(|b| b.is_ascii_digit())
}

fn spot_from(f: &Fields<'_>, now: u64) -> Option<Spot> {
    if f.bad {
        return None;
    }
    let freq = khz_to_hz(f.frequency?)?;
    let time = parse_utc(f.spot_time?)?.min(now);
    let mut spot = Spot::new(f.activator?, freq, time, Network::Pota)?;
    if let Some(mode) = f.mode {
        spot = spot.with_mode(mode);
    }
    if let Some(spotter) = f.spotter {
        spot = spot.with_spotter(spotter);
    }
    // The park is what an operator wants to see beside a POTA callsign: put it first in the
    // comment, with the spotter's words after it if they fit.
    if let Some(reference) = f.reference.filter(|r| is_reference(r)) {
        let comment = f
            .comments
            .and_then(|c| sanitise_text(&format!("{reference} {c}")))
            .unwrap_or_else(|| reference.to_string());
        spot = spot.with_comment(&comment);
    } else if let Some(c) = f.comments {
        spot = spot.with_comment(c);
    }
    Some(spot)
}

/// Read one reply. `now` (Unix seconds) caps every spot's time, so a wrong or hostile clock cannot
/// make a spot outlive the age limit by claiming to be from the future.
pub fn parse_spots(body: &[u8], now: u64) -> Result<Parsed, String> {
    if body.len() > MAX_BODY {
        return Err(format!("the reply is larger than {} KB", MAX_BODY / 1024));
    }
    if std::str::from_utf8(body).is_err() {
        return Err("the reply is not text".into());
    }
    let mut sc = Scanner { s: body, i: 0 };
    sc.ws();
    if !sc.eat(b'[') {
        return Err("the reply is not a list of spots".into());
    }
    let mut out = Parsed::default();
    let mut count = 0usize;
    sc.ws();
    if !sc.eat(b']') {
        loop {
            sc.ws();
            count += 1;
            if count > MAX_SPOTS {
                return Err(format!("more than {MAX_SPOTS} spots in one reply"));
            }
            let Some(fields) = object(&mut sc) else {
                return Err("the reply is not a list of flat spot records".into());
            };
            match spot_from(&fields, now) {
                Some(spot) => out.spots.push(spot),
                None => out.rejected += 1,
            }
            sc.ws();
            if sc.eat(b',') {
                continue;
            }
            if sc.eat(b']') {
                break;
            }
            return Err("the reply is cut off or malformed".into());
        }
    }
    sc.ws();
    if sc.i != body.len() {
        return Err("unexpected text after the list of spots".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record as the live endpoint gave it once, callsigns replaced.
    fn record(activator: &str, freq: &str, time: &str) -> String {
        format!(
            r#"{{"spotId":1,"activator":"{activator}","frequency":"{freq}","mode":"FT8","reference":"CA-0040","parkName":null,"spotTime":"{time}","spotter":"BB2BBB","comments":"CQ POTA","source":"Web","invalid":null,"name":"A Park","locationDesc":"CA-ON","grid4":"FN25","grid6":"FN25ab","latitude":45.5,"longitude":-75.1,"count":46,"expire":1796}}"#
        )
    }

    const NOW: u64 = 1_789_967_220 + 600;

    /// FR-SPOT-08: a reply shaped like the live one makes the right spots, with the frequency
    /// taken from kilohertz, the time read as UTC, and the park in the comment.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_pota_reply_parses() {
        let body = format!(
            "[{},{}]",
            record("aa1aaa", "10136.0", "2026-09-21T05:07:00"),
            record("W1AW/P", "14074.742", "2026-09-21T05:08:30")
        );
        let p = parse_spots(body.as_bytes(), NOW).unwrap();
        assert_eq!(p.rejected, 0);
        assert_eq!(p.spots.len(), 2);
        let s = &p.spots[0];
        assert_eq!(s.call, "AA1AAA");
        assert_eq!(s.freq_hz, 10_136_000);
        assert_eq!(s.time, 1_789_967_220);
        assert_eq!(s.network, Network::Pota);
        assert_eq!(s.mode.as_deref(), Some("FT8"));
        assert_eq!(s.spotter.as_deref(), Some("BB2BBB"));
        assert_eq!(s.comment.as_deref(), Some("CA-0040 CQ POTA"));
        assert_eq!(s.snr_db, None, "POTA gives no SNR");
        assert_eq!(p.spots[1].call, "W1AW/P");
        assert_eq!(p.spots[1].freq_hz, 14_074_742);
        // An empty list is a valid reply (nobody is spotted), not an error.
        assert_eq!(parse_spots(b" [ ] ", NOW).unwrap(), Parsed::default());
        // A number where the live reply has a string still reads.
        let numeric = record("aa1aaa", "10136.0", "2026-09-21T05:07:00")
            .replace(r#""frequency":"10136.0""#, r#""frequency":7030.5"#);
        let p = parse_spots(format!("[{numeric}]").as_bytes(), NOW).unwrap();
        assert_eq!(p.spots[0].freq_hz, 7_030_500);
    }

    /// FR-SPOT-08: the kilohertz reading is exact and strict, and the time reading is UTC to the
    /// second, checked against values computed independently with `date -u`.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_frequency_and_time_arithmetic() {
        for (text, want) in [
            ("10136.0", Some(10_136_000)),
            ("10136", Some(10_136_000)),
            ("14074.742", Some(14_074_742)),
            ("7030.5", Some(7_030_500)),
            ("144174.0", Some(144_174_000)),
            ("100.0", Some(100_000)),
            ("99.999", None), // below the floor
            ("14.074", None), // megahertz mistaken for kilohertz
            ("", None),
            (".5", None),
            ("10136.", None),
            ("10136.0000", None), // finer than a hertz
            ("1e4", None),
            ("-10136", None),
            ("+10136", None),
            (" 10136", None),
            ("10136 ", None),
            ("0x2f", None),
            ("1234567890", None),  // too many digits
            ("300000001.0", None), // above 300 GHz
        ] {
            assert_eq!(khz_to_hz(text), want, "{text:?}");
        }
        for (text, want) in [
            ("2000-01-01T00:00:00", Some(946_684_800)),
            ("2026-09-21T05:07:00", Some(1_789_967_220)),
            ("2026-09-21T05:07:00Z", Some(1_789_967_220)),
            ("2026-09-21T05:07:00.123", Some(1_789_967_220)),
            ("2024-02-29T12:00:00", Some(1_709_208_000)),
            ("2100-03-01T00:00:00", Some(4_107_542_400)),
            ("2026-12-31T23:59:59", Some(1_798_761_599)),
            ("2023-02-29T12:00:00", None), // not a leap year
            ("2100-02-29T12:00:00", None), // 2100 is not a leap year
            ("2026-04-31T00:00:00", None),
            ("2026-13-01T00:00:00", None),
            ("2026-00-10T00:00:00", None),
            ("2026-09-00T00:00:00", None),
            ("2026-09-21T24:00:00", None),
            ("2026-09-21T05:60:00", None),
            ("2026-09-21T05:07:60", None),
            ("1999-12-31T23:59:59", None),
            ("2101-01-01T00:00:00", None),
            ("2026-09-21 05:07:00", None),       // a space, not T
            ("2026-09-21T05:07:00+02:00", None), // an offset is not read
            ("2026-09-21T05:07:00.", None),
            ("2026-09-21T05:07", None),
            ("", None),
        ] {
            assert_eq!(parse_utc(text), want, "{text:?}");
        }
        // A multi-byte character in place of one or of two characters, at every position: refused,
        // and never a panic on a slice inside it (the string is sliced by byte position).
        let valid = "2026-09-21T05:07:00";
        for at in 0..valid.len() {
            for width in [1, 2, 3] {
                if at + width > valid.len() {
                    continue;
                }
                for ch in ['\u{e9}', '\u{20ac}', '\u{1f600}'] {
                    let mut s = String::new();
                    s.push_str(&valid[..at]);
                    s.push(ch);
                    s.push_str(&valid[at + width..]);
                    assert_eq!(parse_utc(&s), None, "{s:?}");
                }
            }
        }
    }

    /// FR-SPOT-08: one bad record costs only itself; hostile or malformed replies are errors, not
    /// half-read lists.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_pota_rejects_hostile_input() {
        let good = record("aa1aaa", "10136.0", "2026-09-21T05:07:00");
        let bad_call = record("CQ", "10136.0", "2026-09-21T05:07:00");
        let bad_freq = record("bb2bbb", "14.074", "2026-09-21T05:07:00");
        let bad_time = record("cc3ccc", "10136.0", "yesterday");
        let flagged = good.replace(r#""invalid":null"#, r#""invalid":true"#);
        let non_null = good.replace(r#""invalid":null"#, r#""invalid":1"#);
        let twice = good.replace(r#""mode":"FT8""#, r#""mode":"FT8","activator":"dd4ddd""#);
        let missing = good.replace(r#""frequency":"10136.0","#, "");
        let body = format!(
            "[{bad_call},{good},{bad_freq},{bad_time},{flagged},{non_null},{twice},{missing}]"
        );
        let p = parse_spots(body.as_bytes(), NOW).unwrap();
        assert_eq!(p.spots.len(), 1, "only the good one survives");
        assert_eq!(p.spots[0].call, "AA1AAA");
        assert_eq!(p.rejected, 7);
        // `invalid: false` is accepted.
        let f = good.replace(r#""invalid":null"#, r#""invalid":false"#);
        assert_eq!(
            parse_spots(format!("[{f}]").as_bytes(), NOW)
                .unwrap()
                .spots
                .len(),
            1
        );

        // Structural damage is an error for the whole reply.
        let backslash = '\\';
        let cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", b"".to_vec()),
            ("not a list", b"{}".to_vec()),
            ("an object list of numbers", b"[1,2]".to_vec()),
            ("nested value", br#"[{"a":{"b":1}}]"#.to_vec()),
            ("nested array", br#"[{"a":[1]}]"#.to_vec()),
            ("trailing comma", format!("[{good},]").into_bytes()),
            ("no comma", format!("[{good}{good}]").into_bytes()),
            ("cut off", format!("[{good}").into_bytes()),
            ("text after", format!("[{good}] x").into_bytes()),
            ("two lists", format!("[{good}][{good}]").into_bytes()),
            ("unquoted key", br#"[{a:1}]"#.to_vec()),
            ("bad literal", br#"[{"a":tru}]"#.to_vec()),
            ("control char in string", b"[{\"a\":\"x\ty\"}]".to_vec()),
            (
                "bad escape",
                format!(r#"[{{"a":"x{backslash}qy"}}]"#).into_bytes(),
            ),
            (
                "short unicode escape",
                format!(r#"[{{"a":"x{backslash}u12"}}]"#).into_bytes(),
            ),
            ("invalid utf-8", b"[{\"a\":\"\xff\xfe\"}]".to_vec()),
            ("unterminated string", br#"[{"a":"xyz}]"#.to_vec()),
            (
                "number too long",
                format!(r#"[{{"a":{}}}]"#, "1".repeat(40)).into_bytes(),
            ),
        ];
        for (name, body) in cases {
            assert!(parse_spots(&body, NOW).is_err(), "{name} must be an error");
        }
        // Over the size cap, and over the count cap.
        // A valid list padded past the cap: only the size check can refuse it.
        let mut huge = format!("[{good}]").into_bytes();
        assert!(
            parse_spots(&huge, NOW).is_ok(),
            "the unpadded list is valid"
        );
        huge.resize(MAX_BODY + 1, b' ');
        assert!(parse_spots(&huge, NOW).is_err());
        huge.truncate(MAX_BODY);
        assert!(
            parse_spots(&huge, NOW).is_ok(),
            "exactly at the cap is read"
        );
        let many = format!("[{}]", vec!["{}"; MAX_SPOTS + 1].join(","));
        assert!(parse_spots(many.as_bytes(), NOW).is_err());
        let ok_many = format!("[{}]", vec!["{}"; MAX_SPOTS].join(","));
        assert_eq!(
            parse_spots(ok_many.as_bytes(), NOW).unwrap().rejected,
            MAX_SPOTS as u64
        );
    }

    /// FR-SPOT-08: text with escapes, over-long text, or non-ASCII loses only that field; a
    /// future stamp is capped at now; the reference is used only if it looks like one.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_optional_fields_and_time_cap() {
        let backslash = '\\';
        // A comment with an escape sequence is skipped, the spot and the park survive.
        let escaped = record("aa1aaa", "10136.0", "2026-09-21T05:07:00").replace(
            "CQ POTA",
            &format!("caf{backslash}u00e9 {backslash}\"x{backslash}\""),
        );
        let p = parse_spots(format!("[{escaped}]").as_bytes(), NOW).unwrap();
        assert_eq!(p.spots[0].comment.as_deref(), Some("CA-0040"));
        // A long comment is dropped rather than cut; the park stays.
        let long =
            record("aa1aaa", "10136.0", "2026-09-21T05:07:00").replace("CQ POTA", &"x".repeat(70));
        let p = parse_spots(format!("[{long}]").as_bytes(), NOW).unwrap();
        assert_eq!(p.spots[0].comment.as_deref(), Some("CA-0040"));
        // Not a reference: the spotter's words stand alone; not text at all: no comment.
        let odd =
            record("aa1aaa", "10136.0", "2026-09-21T05:07:00").replace("CA-0040", "not a park");
        let p = parse_spots(format!("[{odd}]").as_bytes(), NOW).unwrap();
        assert_eq!(p.spots[0].comment.as_deref(), Some("CQ POTA"));
        let none = record("aa1aaa", "10136.0", "2026-09-21T05:07:00")
            .replace(r#""comments":"CQ POTA""#, r#""comments":null"#);
        let p = parse_spots(format!("[{none}]").as_bytes(), NOW).unwrap();
        assert_eq!(p.spots[0].comment.as_deref(), Some("CA-0040"));
        // A stamp in the future is capped at now.
        let future = record("aa1aaa", "10136.0", "2026-09-21T23:00:00");
        let p = parse_spots(format!("[{future}]").as_bytes(), NOW).unwrap();
        assert_eq!(p.spots[0].time, NOW);
        assert!(is_reference("CA-0040") && is_reference("US-1234") && is_reference("K-0001"));
        for bad in [
            "",
            "CA0040",
            "CA-",
            "ABCD-EFGH",
            "CA-0040 x",
            "CA-00400000000",
        ] {
            assert!(!is_reference(bad), "{bad:?}");
        }
    }
}
