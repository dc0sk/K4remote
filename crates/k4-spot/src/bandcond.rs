//! Band conditions for the band buttons (FR-UI-25): the HamQSL forecast, spot activity, and the
//! rule that turns them into one rating per band. Pure — time is passed in, nothing touches a
//! network or a clock — so every rule is tested here. The design and its review are in
//! `docs/concept/band-conditions-plan.md` (v0.2).

use std::collections::HashMap;

use crate::{normalise_callsign, psk, Network};

/// A band's rating. Ordered: `Poor < Fair < Good`. "Unrated" is `None` wherever a rating is
/// optional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Rating {
    Poor,
    Fair,
    Good,
}

impl Rating {
    fn parse(s: &str) -> Option<Rating> {
        match s.trim() {
            "Poor" => Some(Rating::Poor),
            "Fair" => Some(Rating::Fair),
            "Good" => Some(Rating::Good),
            _ => None,
        }
    }

    /// One step better, saturating at `Good`.
    fn up(self) -> Rating {
        match self {
            Rating::Poor => Rating::Fair,
            _ => Rating::Good,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Rating::Poor => "Poor",
            Rating::Fair => "Fair",
            Rating::Good => "Good",
        }
    }
}

/// HamQSL's four HF band groups, in feed order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    B80_40,
    B30_20,
    B17_15,
    B12_10,
}

impl Group {
    const ALL: [Group; 4] = [Group::B80_40, Group::B30_20, Group::B17_15, Group::B12_10];

    fn feed_name(self) -> &'static str {
        match self {
            Group::B80_40 => "80m-40m",
            Group::B30_20 => "30m-20m",
            Group::B17_15 => "17m-15m",
            Group::B12_10 => "12m-10m",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// The bands the K4's direct band buttons select (`BN` 00–10), as [`psk::band_token`]s — the
/// bands activity is reported for.
pub const BANDS: [&str; 11] = [
    "160m", "80m", "60m", "40m", "30m", "20m", "17m", "15m", "12m", "10m", "6m",
];

/// The HamQSL group a band (a [`psk::band_token`], e.g. `"20m"`) is rated by. 160 m and 6 m are
/// not rated by HamQSL's HF groups, and its VHF values are not used (plan §1.1, §2).
pub fn group_for_band(band: &str) -> Option<Group> {
    Some(match band {
        "80m" | "60m" | "40m" => Group::B80_40,
        "30m" | "20m" => Group::B30_20,
        "17m" | "15m" => Group::B17_15,
        "12m" | "10m" => Group::B12_10,
        _ => return None,
    })
}

/// The largest HamQSL body the parser reads (the feed is ~1.6 KB), independent of the fetcher's.
pub const MAX_BODY: usize = 64 * 1024;

/// One HamQSL reading: each group's day and night rating, and when the feed says it was updated.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Forecast {
    day: [Option<Rating>; 4],
    night: [Option<Rating>; 4],
    /// The feed's own `<updated>`, Unix seconds; `None` if absent or unreadable.
    pub updated: Option<u64>,
}

impl Forecast {
    /// A group's rating for day (`true`) or night.
    pub fn rating(&self, group: Group, day: bool) -> Option<Rating> {
        if day {
            self.day[group.index()]
        } else {
            self.night[group.index()]
        }
    }
}

/// Parse HamQSL's `solarxml.php`. Reads only the `<band name=… time=…>` elements and `<updated>`;
/// a body past [`MAX_BODY`], one that is not UTF-8, or one without `<calculatedconditions>` is an
/// error (a failed fetch). An unknown rating value leaves that group unrated.
pub fn parse_hamqsl(body: &[u8]) -> Result<Forecast, String> {
    if body.len() > MAX_BODY {
        return Err(format!("HamQSL reply too large ({} bytes)", body.len()));
    }
    let text = std::str::from_utf8(body).map_err(|_| "HamQSL reply is not UTF-8".to_string())?;
    let start = text
        .find("<calculatedconditions>")
        .ok_or("HamQSL reply has no band conditions")?;
    let end = text[start..]
        .find("</calculatedconditions>")
        .ok_or("HamQSL reply is truncated")?
        + start;
    let mut f = Forecast {
        updated: element(text, "updated").and_then(parse_updated),
        ..Forecast::default()
    };
    let mut rest = &text[start..end];
    while let Some(i) = rest.find("<band ") {
        rest = &rest[i + 6..];
        let Some(close) = rest.find('>') else { break };
        let attrs = &rest[..close];
        let Some(stop) = rest[close..].find("</band>") else {
            break;
        };
        let value = &rest[close + 1..close + stop];
        rest = &rest[close + stop..];
        let (Some(name), Some(time)) = (attr(attrs, "name"), attr(attrs, "time")) else {
            continue;
        };
        let Some(group) = Group::ALL.into_iter().find(|g| g.feed_name() == name) else {
            continue;
        };
        let slot = match time {
            "day" => &mut f.day,
            "night" => &mut f.night,
            _ => continue,
        };
        slot[group.index()] = Rating::parse(value);
    }
    Ok(f)
}

fn element<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let i = text.find(&open)? + open.len();
    let j = text[i..].find(&format!("</{tag}>"))? + i;
    Some(text[i..j].trim())
}

fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let i = attrs.find(&key)? + key.len();
    let j = attrs[i..].find('"')? + i;
    Some(&attrs[i..j])
}

/// `04 Oct 2026 1626 GMT` → Unix seconds.
fn parse_updated(s: &str) -> Option<u64> {
    let mut it = s.split_whitespace();
    let day: u32 = it.next()?.parse().ok()?;
    let month = match it.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = it.next()?.parse().ok()?;
    let hhmm = it.next()?;
    // Four ASCII digits before slicing: a length check alone counts bytes, and a four-byte string
    // with a multi-byte character would be sliced inside it (a panic on the spot-sources thread).
    // The year is bounded so the date arithmetic cannot overflow, and the day must exist.
    if hhmm.len() != 4
        || !hhmm.bytes().all(|b| b.is_ascii_digit())
        || it.next()? != "GMT"
        || !(1970..=9999).contains(&year)
        || day == 0
        || day > days_in_month(year, month)
    {
        return None;
    }
    let (h, m): (u64, u64) = (hhmm[..2].parse().ok()?, hhmm[2..].parse().ok()?);
    if h > 23 || m > 59 {
        return None;
    }
    let days = days_from_civil(year, month, day);
    u64::try_from(days)
        .ok()
        .map(|d| d * 86_400 + h * 3600 + m * 60)
}

/// The days in `month` (1–12) of `year`, Gregorian.
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = i64::from((m + 9) % 12);
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// ---------------------------------------------------------------------------------------------
// Day or night at the station.

/// The centre of a Maidenhead square (4 or 6 characters, any case) as (latitude, longitude)
/// degrees; `None` for anything else.
pub fn locator_centre(loc: &str) -> Option<(f64, f64)> {
    let b = loc.trim().as_bytes();
    if b.len() != 4 && b.len() != 6 {
        return None;
    }
    let field = |c: u8| match c.to_ascii_uppercase() {
        c @ b'A'..=b'R' => Some(f64::from(c - b'A')),
        _ => None,
    };
    let digit = |c: u8| c.is_ascii_digit().then(|| f64::from(c - b'0'));
    let sub = |c: u8| match c.to_ascii_lowercase() {
        c @ b'a'..=b'x' => Some(f64::from(c - b'a')),
        _ => None,
    };
    let mut lon = field(b[0])? * 20.0 - 180.0 + digit(b[2])? * 2.0;
    let mut lat = field(b[1])? * 10.0 - 90.0 + digit(b[3])?;
    if b.len() == 6 {
        lon += sub(b[4])? * (5.0 / 60.0) + 2.5 / 60.0;
        lat += sub(b[5])? * (2.5 / 60.0) + 1.25 / 60.0;
    } else {
        lon += 1.0;
        lat += 0.5;
    }
    Some((lat, lon))
}

/// The sun's elevation above the horizon, degrees, at (`lat`, `lon`) and Unix time `t` — the
/// low-precision NOAA / Astronomical Almanac formula, good to a fraction of a degree.
pub fn sun_elevation_deg(lat: f64, lon: f64, t: u64) -> f64 {
    let d = t as f64 / 86_400.0 - 10_957.5; // days since J2000.0
    let rad = f64::to_radians;
    let g = rad((357.529 + 0.985_600_28 * d).rem_euclid(360.0));
    let q = (280.459 + 0.985_647_36 * d).rem_euclid(360.0);
    let l = rad(q + 1.915 * g.sin() + 0.020 * (2.0 * g).sin());
    let e = rad(23.439 - 0.000_000_36 * d);
    let ra = (e.cos() * l.sin()).atan2(l.cos());
    let dec = (e.sin() * l.sin()).asin();
    let gmst_deg = (18.697_374_558 + 24.065_709_824_419_08 * d) * 15.0;
    let h = rad(gmst_deg + lon) - ra;
    let lat = rad(lat);
    (lat.sin() * dec.sin() + lat.cos() * dec.cos() * h.cos())
        .asin()
        .to_degrees()
}

/// Whether it is day at the station: the sun above the horizon at the locator's centre, or —
/// with no locator — 06:00–18:00 on this computer's clock (`local_hour`, 0–23).
pub fn is_day(locator: Option<(f64, f64)>, t: u64, local_hour: u8) -> bool {
    match locator {
        Some((lat, lon)) => sun_elevation_deg(lat, lon, t) > 0.0,
        None => (6..18).contains(&local_hour),
    }
}

// ---------------------------------------------------------------------------------------------
// Spot activity.

/// How long a heard call counts as activity, seconds (tuned).
pub const ACTIVITY_WINDOW_SECS: u64 = 15 * 60;
/// Distinct calls in the window that read as *Fair* (tuned).
pub const ACTIVITY_FAIR: usize = 3;
/// Distinct calls in the window that read as *Good* (tuned).
pub const ACTIVITY_GOOD: usize = 10;
/// The most (call, network) entries held per band — a bound on a flood, far above any real band.
pub const MAX_CALLS_PER_BAND: usize = 4096;

/// Who has been heard on which band, by which network, and when — the union across networks is
/// taken when counting, so one station heard by two networks counts once.
#[derive(Debug, Default)]
pub struct Activity {
    bands: HashMap<&'static str, HashMap<(String, Network), u64>>,
}

impl Activity {
    pub fn new() -> Self {
        Self::default()
    }

    /// Note `call` heard on `freq_hz` by `network` at `t`. Calls that do not normalise, and
    /// frequencies outside every band, are ignored. Entries older than the window are dropped;
    /// a band already holding [`MAX_CALLS_PER_BAND`] fresh entries takes no new ones.
    pub fn note(&mut self, call: &str, freq_hz: u64, network: Network, t: u64) {
        let (Some(band), Some(call)) = (psk::band_token(freq_hz), normalise_callsign(call)) else {
            return;
        };
        let calls = self.bands.entry(band).or_default();
        let key = (call, network);
        if !calls.contains_key(&key) && calls.len() >= MAX_CALLS_PER_BAND {
            calls.retain(|_, &mut seen| t.saturating_sub(seen) < ACTIVITY_WINDOW_SECS);
            if calls.len() >= MAX_CALLS_PER_BAND {
                return;
            }
        }
        let seen = calls.entry(key).or_insert(t);
        *seen = (*seen).max(t);
    }

    /// Distinct calls heard on `band` in the window before `now`, by any of `networks`.
    pub fn count(&self, band: &str, now: u64, networks: &[Network]) -> usize {
        let Some(calls) = self.bands.get(band) else {
            return 0;
        };
        let mut distinct: Vec<&str> = calls
            .iter()
            .filter(|((_, net), &seen)| {
                networks.contains(net) && now.saturating_sub(seen) < ACTIVITY_WINDOW_SECS
            })
            .map(|((call, _), _)| call.as_str())
            .collect();
        distinct.sort_unstable();
        distinct.dedup();
        distinct.len()
    }

    /// The networks among `networks` that heard anyone on `band` in the window before `now`, in
    /// the order given — what a band's tooltip credits, rather than every network that is ticked.
    pub fn networks_on(&self, band: &str, now: u64, networks: &[Network]) -> Vec<Network> {
        let Some(calls) = self.bands.get(band) else {
            return Vec::new();
        };
        networks
            .iter()
            .copied()
            .filter(|n| {
                calls.iter().any(|((_, net), &seen)| {
                    net == n && now.saturating_sub(seen) < ACTIVITY_WINDOW_SECS
                })
            })
            .collect()
    }

    /// Drop every entry older than the window.
    pub fn purge(&mut self, now: u64) {
        for calls in self.bands.values_mut() {
            calls.retain(|_, &mut seen| now.saturating_sub(seen) < ACTIVITY_WINDOW_SECS);
        }
    }
}

/// What a count of distinct calls is evidence of: *Good*, *Fair*, or nothing — never *Poor*,
/// since a quiet band may be open with nobody reporting.
pub fn activity_rating(distinct_calls: usize) -> Option<Rating> {
    if distinct_calls >= ACTIVITY_GOOD {
        Some(Rating::Good)
    } else if distinct_calls >= ACTIVITY_FAIR {
        Some(Rating::Fair)
    } else {
        None
    }
}

/// One rating from a forecast and activity evidence: activity only ever **raises** the forecast,
/// by at most one step; alone, it gives at most *Fair*; with neither, the band is unrated.
pub fn combine(forecast: Option<Rating>, activity: Option<Rating>) -> Option<Rating> {
    match (forecast, activity) {
        (Some(f), Some(a)) => Some(f.max(a.min(f.up()))),
        (Some(f), None) => Some(f),
        (None, Some(a)) => Some(a.min(Rating::Fair)),
        (None, None) => None,
    }
}

// ---------------------------------------------------------------------------------------------
// HamQSL fetch schedule and freshness.

/// HamQSL asks clients to update no more than hourly.
pub const HAMQSL_INTERVAL_MIN_SECS: u64 = 3600;
pub const HAMQSL_INTERVAL_DEFAULT_SECS: u64 = 3600;
pub const HAMQSL_INTERVAL_MAX_SECS: u64 = 86_400;
/// HamQSL's slowest update cycle (its band conditions follow the 3-hourly indices).
pub const HAMQSL_CYCLE_SECS: u64 = 3 * 3600;

/// A configured HamQSL interval, kept inside its bounds.
pub fn hamqsl_interval(secs: u64) -> u64 {
    secs.clamp(HAMQSL_INTERVAL_MIN_SECS, HAMQSL_INTERVAL_MAX_SECS)
}

/// Whether a forecast still counts at `now`: the last successful fetch is no older than two
/// intervals, and the feed's own `updated` (when it gave one) no older than its cycle plus one
/// interval.
pub fn forecast_current(last_ok_fetch: u64, updated: Option<u64>, now: u64, interval: u64) -> bool {
    let interval = hamqsl_interval(interval);
    now.saturating_sub(last_ok_fetch) <= 2 * interval
        && updated.is_none_or(|u| now.saturating_sub(u) <= HAMQSL_CYCLE_SECS + interval)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/hamqsl-2026-10-04.xml");

    /// The capture's own `updated`: 04 Oct 2026 16:26 UTC.
    const UPDATED: u64 = 1_791_131_160;

    /// FR-UI-25: the committed HamQSL capture parses to its eight group ratings and its time.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_parses_the_hamqsl_capture() {
        let f = parse_hamqsl(FIXTURE).expect("parses");
        use Rating::*;
        let day = Group::ALL.map(|g| f.rating(g, true));
        let night = Group::ALL.map(|g| f.rating(g, false));
        assert_eq!(day, [Some(Poor), Some(Fair), Some(Fair), Some(Poor)]);
        assert_eq!(night, [Some(Fair), Some(Fair), Some(Fair), Some(Poor)]);
        assert_eq!(f.updated, Some(UPDATED));
        assert_eq!(parse_updated("01 Jan 1970 0000 GMT"), Some(0));
    }

    /// FR-UI-25: malformed replies are failed fetches; an unknown value is unrated, not guessed.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_rejects_malformed_replies() {
        assert!(
            parse_hamqsl(&vec![b' '; MAX_BODY + 1]).is_err(),
            "over the cap"
        );
        let text = std::str::from_utf8(FIXTURE).unwrap();
        let cut = text.find("</calculatedconditions>").unwrap();
        assert!(parse_hamqsl(&text.as_bytes()[..cut]).is_err(), "truncated");
        assert!(parse_hamqsl(b"<solar></solar>").is_err(), "no conditions");
        assert!(parse_hamqsl(&[0xff, 0xfe]).is_err(), "not UTF-8");
        let odd = text.replacen(
            r#"<band name="30m-20m" time="day">Fair</band>"#,
            r#"<band name="30m-20m" time="day">Excellent</band>"#,
            1,
        );
        let f = parse_hamqsl(odd.as_bytes()).unwrap();
        assert_eq!(
            f.rating(Group::B30_20, true),
            None,
            "unknown value is unrated"
        );
        assert_eq!(f.rating(Group::B30_20, false), Some(Rating::Fair));
    }

    /// FR-UI-25: a malformed `<updated>` — a non-ASCII time that is four bytes long, a year that
    /// would overflow the date arithmetic, a day the month does not have — is "no date", never a
    /// panic: the parser runs on the spot-sources thread, and a panic there would stop every spot
    /// source. The forecast itself still parses.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_a_malformed_update_time_is_no_date_not_a_panic() {
        for bad in [
            "04 Oct 2026 1\u{e9}5 GMT",     // 4 bytes, 3 chars: byte 2 is inside the é
            "04 Oct 2026 \u{e9}\u{e9} GMT", // 4 bytes, 2 chars
            "04 Oct 99999999999999 1626 GMT",
            "04 Oct -5 1626 GMT",
            "31 Feb 2026 1626 GMT",
            "31 Apr 2026 1626 GMT",
            "29 Feb 2027 1626 GMT", // not a leap year
        ] {
            let got = std::panic::catch_unwind(|| parse_updated(bad));
            assert_eq!(got.ok(), Some(None), "{bad:?}");
        }
        assert!(
            parse_updated("29 Feb 2028 1626 GMT").is_some(),
            "a leap day is a date"
        );
        let text = std::str::from_utf8(FIXTURE).unwrap();
        let odd = text.replacen("1626 GMT", "1\u{e9}5 GMT", 1);
        let f = std::panic::catch_unwind(|| parse_hamqsl(odd.as_bytes()))
            .expect("must not panic")
            .expect("the forecast still parses");
        assert_eq!(f.updated, None);
        assert_eq!(f.rating(Group::B30_20, true), Some(Rating::Fair));
    }

    /// FR-UI-25: every band maps to its HamQSL group; 160 m and 6 m are not rated by it.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_maps_bands_to_groups() {
        use Group::*;
        for (band, want) in [
            ("160m", None),
            ("80m", Some(B80_40)),
            ("60m", Some(B80_40)),
            ("40m", Some(B80_40)),
            ("30m", Some(B30_20)),
            ("20m", Some(B30_20)),
            ("17m", Some(B17_15)),
            ("15m", Some(B17_15)),
            ("12m", Some(B12_10)),
            ("10m", Some(B12_10)),
            ("6m", None),
            ("2m", None),
        ] {
            assert_eq!(group_for_band(band), want, "{band}");
        }
    }

    /// FR-UI-25: locators resolve to their square's centre; anything else is refused.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_locators() {
        let (lat, lon) = locator_centre("JO31").unwrap();
        assert!(
            (lat - 51.5).abs() < 1e-9 && (lon - 7.0).abs() < 1e-9,
            "{lat} {lon}"
        );
        let (lat, lon) = locator_centre("jo31lk").unwrap();
        assert!(
            (lat - (51.0 + 10.0 * 2.5 / 60.0 + 1.25 / 60.0)).abs() < 1e-9,
            "{lat}"
        );
        assert!(
            (lon - (6.0 + 11.0 * 5.0 / 60.0 + 2.5 / 60.0)).abs() < 1e-9,
            "{lon}"
        );
        for bad in ["", "JO3", "JO31L", "SO31", "JOA1", "JO31ly", "JO31lz"] {
            assert!(locator_centre(bad).is_none(), "{bad:?}");
        }
    }

    /// FR-UI-25: day follows the sun at the locator; without one, the local clock.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_day_follows_the_sun() {
        let jo31 = locator_centre("JO31");
        let june_21 = days_from_civil(2026, 6, 21) as u64 * 86_400;
        let el_noon = sun_elevation_deg(51.5, 7.0, june_21 + 12 * 3600);
        assert!(
            (55.0..65.0).contains(&el_noon),
            "summer noon at 51.5°N ≈ 61°: {el_noon}"
        );
        assert!(is_day(jo31, june_21 + 12 * 3600, 0));
        assert!(
            !is_day(jo31, june_21, 12),
            "midnight UTC is night at JO31 whatever the clock"
        );
        // Inside the polar night (Svalbard, 78°N, mid-December) the sun stays down at noon.
        let dec_15 = days_from_civil(2026, 12, 15) as u64 * 86_400;
        assert!(!is_day(locator_centre("JQ78"), dec_15 + 12 * 3600, 12));
        // No locator: 06:00–18:00 local.
        assert!(is_day(None, 0, 6) && is_day(None, 0, 17));
        assert!(!is_day(None, 0, 5) && !is_day(None, 0, 18));
    }

    /// FR-UI-25: one station heard by two networks counts once; the window ages calls out; a
    /// network not asked for is not counted; out-of-band frequencies and junk calls are ignored.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_activity_is_a_union_of_calls() {
        use Network::*;
        let mut a = Activity::new();
        let both = [Rbn, DxCluster];
        a.note("DL1ABC", 14_025_000, Rbn, 100);
        a.note("dl1abc", 14_030_000, DxCluster, 110);
        a.note("G4XYZ", 14_074_000, DxCluster, 120);
        a.note("K1JT", 7_074_000, Rbn, 120);
        a.note("N0CALL", 15_000_000, Rbn, 120); // between bands
        a.note("CQ", 14_074_000, Rbn, 120); // not a call
        assert_eq!(a.count("20m", 130, &both), 2, "DL1ABC once, G4XYZ");
        assert_eq!(a.count("20m", 130, &[Rbn]), 1, "only RBN asked for");
        assert_eq!(
            a.networks_on("20m", 130, &[Rbn, DxCluster, PskReporter]),
            [Rbn, DxCluster]
        );
        assert_eq!(
            a.networks_on("40m", 130, &[Rbn, DxCluster]),
            [Rbn],
            "only who heard someone there"
        );
        assert!(a.networks_on("15m", 130, &both).is_empty());
        assert_eq!(a.count("40m", 130, &both), 1);
        assert_eq!(
            a.count("20m", 110 + ACTIVITY_WINDOW_SECS, &both),
            1,
            "DL1ABC aged out"
        );
        a.purge(120 + ACTIVITY_WINDOW_SECS);
        assert_eq!(a.count("20m", 0, &both), 0, "purged");
    }

    /// FR-UI-25: a flood cannot grow a band past its cap, and stale entries make room.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_activity_is_bounded() {
        let mut a = Activity::new();
        for i in 0..MAX_CALLS_PER_BAND + 100 {
            a.note(&format!("DL{i}AA"), 14_025_000, Network::Rbn, 0);
        }
        assert_eq!(a.count("20m", 1, &[Network::Rbn]), MAX_CALLS_PER_BAND);
        a.note("DL9ZZZ", 14_025_000, Network::Rbn, ACTIVITY_WINDOW_SECS);
        assert_eq!(
            a.count("20m", ACTIVITY_WINDOW_SECS, &[Network::Rbn]),
            1,
            "room made"
        );
    }

    /// FR-UI-25: activity only raises, by one step at most, and alone gives at most *Fair*;
    /// sparse activity is no evidence; nothing at all is unrated.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_combination() {
        use Rating::*;
        assert_eq!(activity_rating(ACTIVITY_FAIR - 1), None);
        assert_eq!(activity_rating(ACTIVITY_FAIR), Some(Fair));
        assert_eq!(activity_rating(ACTIVITY_GOOD), Some(Good));
        assert_eq!(
            (ACTIVITY_FAIR, ACTIVITY_GOOD),
            (3, 10),
            "documented thresholds"
        );
        for (f, a, want) in [
            (Some(Poor), None, Some(Poor)),
            (Some(Poor), Some(Fair), Some(Fair)),
            (Some(Poor), Some(Good), Some(Fair)), // one step at most
            (Some(Fair), Some(Fair), Some(Fair)),
            (Some(Fair), Some(Good), Some(Good)),
            (Some(Good), Some(Fair), Some(Good)), // never lowers
            (None, Some(Good), Some(Fair)),       // alone, at most Fair
            (None, Some(Fair), Some(Fair)),
            (None, None, None),
        ] {
            assert_eq!(combine(f, a), want, "{f:?} + {a:?}");
        }
    }

    /// FR-UI-25: the hourly floor, and the two freshness limits.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_interval_and_freshness() {
        assert_eq!(
            HAMQSL_INTERVAL_MIN_SECS, 3600,
            "HamQSL: no more than hourly"
        );
        assert_eq!(hamqsl_interval(60), 3600);
        assert_eq!(hamqsl_interval(1_000_000), HAMQSL_INTERVAL_MAX_SECS);
        let (i, now) = (3600, 1_000_000);
        assert!(forecast_current(
            now - 2 * i,
            Some(now - 3 * 3600 - i),
            now,
            i
        ));
        assert!(
            !forecast_current(now - 2 * i - 1, Some(now), now, i),
            "fetches failing"
        );
        assert!(
            !forecast_current(now, Some(now - 3 * 3600 - i - 1), now, i),
            "feed stale"
        );
        assert!(
            forecast_current(now, None, now, i),
            "no updated: the fetch decides"
        );
        assert!(
            !forecast_current(now - 2 * 3600 - 1, None, now, 60),
            "interval clamped first"
        );
    }
}
