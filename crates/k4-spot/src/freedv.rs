//! FreeDV Reporter's station table (FR-SPOT-08): the events of `qso.freedv.org` become a roster of
//! who is on which frequency, and a roster row becomes a [`Spot`]. Pure and offline.
//!
//! The format is from another client's public source, read for interface facts only
//! (`docs/references/external-references.md`, R-EXT-05) — FreeDV Reporter publishes no
//! specification. The events used, and their fields: `new_connection` (`sid`, `callsign`,
//! `grid_square`), `remove_connection` (`sid`), `freq_change` (`sid`, `freq` in **hertz**),
//! `tx_report` (`sid`, `transmitting`, `mode`), `rx_report` (`callsign` heard, `receiver_callsign`,
//! `snr`), `message_update` (`sid`, `message`) and `bulk_update` (a list of `[name, args]` pairs,
//! the roster as it stands when the session begins).
//!
//! **What a row is.** A station that is connected *now* and has said what frequency it is on — a
//! presence list, not a stream of timestamped reports. The caller stamps the spot with the time it
//! last confirmed the row (when its event arrived, or on its periodic refresh), so a station that
//! has left stops being refreshed and fades with age like any other spot.
//!
//! Everything here is **untrusted**: a field of the wrong type drops that event whole (counted in
//! [`Roster::rejected`]) and leaves the roster as it was; ids, text and numbers are bounded; and
//! the roster itself is capped at [`MAX_STATIONS`].

use std::collections::{BTreeMap, HashMap};

use crate::json::Value;
use crate::{sanitise_text, Network, Spot};

/// Most stations kept. A busy day has a few hundred; the cap is what stops a flood of invented
/// session ids from growing the table without end.
pub const MAX_STATIONS: usize = 4096;

/// Most items of one `bulk_update` read.
pub const MAX_BULK: usize = 10_000;

/// Lowest and highest frequency accepted, Hz (the same guard as the other networks: a value in
/// kilohertz by mistake is below the floor).
const MIN_FREQ_HZ: u64 = 100_000;
const MAX_FREQ_HZ: u64 = 300_000_000_000;

/// Longest session id, callsign, grid, mode and message kept.
const MAX_SID: usize = 64;
const MAX_CALL: usize = 32;
const MAX_GRID: usize = 8;
const MAX_MODE: usize = 16;
const MAX_MESSAGE: usize = 128;

#[derive(Debug, Default, Clone, PartialEq)]
struct Station {
    call: String,
    grid: String,
    freq_hz: u64,
    tx: bool,
    mode: String,
    message: String,
    /// The last receiver to report hearing this station, and the signal report.
    heard_by: Option<(String, i16)>,
}

/// The stations on the air, by the server's session id.
#[derive(Debug, Default)]
pub struct Roster {
    by_sid: HashMap<String, Station>,
    /// Events whose fields were the wrong type or out of range, and dropped whole.
    pub rejected: u64,
    /// Stations not added because the roster was full.
    pub dropped: u64,
    /// Text fields that were well-typed but unusable (too long, or not printable ASCII) and were
    /// dropped while the rest of their event applied. Not an error, but counted: it is the class of
    /// thing an over-strict parser used to reject whole.
    pub degraded: u64,
    /// As `rejected_shapes`, for degraded fields.
    degraded_shapes: BTreeMap<String, (u64, String)>,
    /// For diagnosis: per event name, how many were rejected and the *shape* of the last one (its
    /// field names and the kinds of their values — never a value, so nothing an operator or a
    /// station typed can end up in a log).
    rejected_shapes: BTreeMap<String, (u64, String)>,
}

/// What kind of value this is, for a shape — its kind and a coarse class, never the value. A string
/// gives its length and whether it is printable ASCII; a whole number says whether it is zero,
/// below the frequency floor, in range or above the ceiling (which is what decides an event).
fn kind(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(_) => "bool".into(),
        Value::Num(n) if n.fract() == 0.0 => {
            let class = match n {
                n if *n < 0.0 => "negative",
                n if *n == 0.0 => "zero",
                n if *n < MIN_FREQ_HZ as f64 => "below-floor",
                n if *n <= MAX_FREQ_HZ as f64 => "in-range",
                _ => "above-ceiling",
            };
            format!("int({class})")
        }
        Value::Num(_) => "float".into(),
        Value::Str(t) => {
            let ascii = t.bytes().all(|b| (b' '..=b'~').contains(&b));
            format!(
                "str(len={},{})",
                t.len(),
                if ascii { "ascii" } else { "non-ascii" }
            )
        }
        Value::Arr(_) => "array".into(),
        Value::Obj(_) => "object".into(),
    }
}

/// `key:kind,key:kind` for an object, the kind for anything else.
fn shape(args: &Value) -> String {
    match args {
        Value::Obj(members) => {
            let mut parts: Vec<String> = members
                .iter()
                .map(|(k, v)| format!("{k}:{}", kind(v)))
                .collect();
            parts.sort();
            parts.join(",")
        }
        other => kind(other).to_string(),
    }
}

fn sid_of(args: &Value) -> Option<String> {
    args.get("sid")
        .and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty() && s.len() <= MAX_SID && s.bytes().all(|b| (b'!'..=b'~').contains(&b))
        })
        .map(str::to_string)
}

impl Roster {
    /// A text field. `Err` only if it is present with the **wrong type** — that is a malformed
    /// event. Absent, `null` or blank is `Ok(None)`. A string that cannot be kept (longer than
    /// `max`, or not printable ASCII) is also `Ok(None)` — the field degrades and the rest of the
    /// event still applies — but it is **counted** in [`Roster::degraded`], so a probe cannot report
    /// "nothing rejected" while dropping what operators wrote. (This client keeps only printable
    /// ASCII by choice, as a policy for untrusted text; it is not a limit of the display.)
    fn text(&mut self, args: &Value, key: &str, max: usize) -> Result<Option<String>, ()> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Str(s)) => {
                let t = s.trim();
                if t.len() <= max && t.bytes().all(|b| (b' '..=b'~').contains(&b)) {
                    Ok(Some(t.to_string()))
                } else {
                    self.degraded += 1;
                    Ok(None)
                }
            }
            Some(_) => Err(()),
        }
    }

    /// How many stations are known.
    pub fn len(&self) -> usize {
        self.by_sid.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_sid.is_empty()
    }

    /// Forget everyone (a new session starts from the server's `bulk_update`).
    pub fn clear(&mut self) {
        self.by_sid.clear();
    }

    /// The session ids known.
    pub fn sids(&self) -> impl Iterator<Item = &String> {
        self.by_sid.keys()
    }

    fn entry(&mut self, sid: String) -> Option<&mut Station> {
        if !self.by_sid.contains_key(&sid) && self.by_sid.len() >= MAX_STATIONS {
            self.dropped += 1;
            return None;
        }
        Some(self.by_sid.entry(sid).or_default())
    }

    /// Apply one server event and return the session ids whose row **changed** (so the caller can
    /// refresh just those spots). Unknown events change nothing; a known one with a bad field is
    /// counted in [`Roster::rejected`] and changes nothing.
    pub fn on_event(&mut self, name: &str, args: &Value) -> Vec<String> {
        if name == "bulk_update" {
            return self.bulk(args);
        }
        self.one(name, args)
    }

    fn bulk(&mut self, args: &Value) -> Vec<String> {
        let Some(items) = args.as_array() else {
            self.rejected += 1;
            return Vec::new();
        };
        let mut changed = Vec::new();
        for item in items.iter().take(MAX_BULK) {
            let (Some(name), args) = (
                item.at(0).and_then(Value::as_str),
                item.at(1).unwrap_or(&Value::Null),
            ) else {
                self.rejected += 1;
                continue;
            };
            // A `bulk_update` inside a `bulk_update` is not a thing the server sends, and letting
            // it nest would be an unbounded-recursion door.
            if name == "bulk_update" {
                self.rejected += 1;
                continue;
            }
            changed.extend(self.one(name, args));
        }
        changed.sort();
        changed.dedup();
        changed
    }

    /// Per event name, how many were rejected and the shape of the last one (diagnosis only).
    pub fn rejected_shapes(&self) -> &BTreeMap<String, (u64, String)> {
        &self.rejected_shapes
    }

    /// Per event name, how many had a field degraded and the shape of the last (diagnosis only).
    pub fn degraded_shapes(&self) -> &BTreeMap<String, (u64, String)> {
        &self.degraded_shapes
    }

    fn one(&mut self, name: &str, args: &Value) -> Vec<String> {
        let (rejected, degraded) = (self.rejected, self.degraded);
        let changed = self.apply(name, args);
        // Only the six known events can be rejected or degraded, so each map holds at most six.
        for (map, count) in [
            (&mut self.rejected_shapes, self.rejected - rejected),
            (&mut self.degraded_shapes, self.degraded - degraded),
        ] {
            if count > 0 {
                let entry = map.entry(name.to_string()).or_insert((0, String::new()));
                entry.0 += count;
                entry.1 = shape(args).chars().take(200).collect();
            }
        }
        changed
    }

    fn apply(&mut self, name: &str, args: &Value) -> Vec<String> {
        match name {
            "new_connection" => self.new_connection(args),
            "remove_connection" => match sid_of(args) {
                Some(sid) => {
                    self.by_sid.remove(&sid);
                    // Nothing to refresh: the station is gone and its spot will age out.
                    Vec::new()
                }
                None => self.reject(),
            },
            "freq_change" => self.freq_change(args),
            "tx_report" => self.tx_report(args),
            "rx_report" => self.rx_report(args),
            "message_update" => self.message_update(args),
            _ => Vec::new(),
        }
    }

    fn reject(&mut self) -> Vec<String> {
        self.rejected += 1;
        Vec::new()
    }

    fn new_connection(&mut self, args: &Value) -> Vec<String> {
        let (Some(sid), Ok(call), Ok(grid)) = (
            sid_of(args),
            self.text(args, "callsign", MAX_CALL),
            self.text(args, "grid_square", MAX_GRID),
        ) else {
            return self.reject();
        };
        // A reconnecting station keeps the frequency already known; the server re-sends its
        // `freq_change` straight after.
        let Some(s) = self.entry(sid.clone()) else {
            return Vec::new();
        };
        s.call = call.unwrap_or_default().to_ascii_uppercase();
        s.grid = grid.unwrap_or_default();
        vec![sid]
    }

    fn freq_change(&mut self, args: &Value) -> Vec<String> {
        let (Some(sid), Some(freq), Ok(call), Ok(grid)) = (
            sid_of(args),
            args.get("freq").and_then(Value::as_u64),
            self.text(args, "callsign", MAX_CALL),
            self.text(args, "grid_square", MAX_GRID),
        ) else {
            return self.reject();
        };
        // Zero is how a station says it has no frequency (not set yet, or cleared): a normal event
        // that removes it from the map, not a malformed one (seen on the real service). Any other
        // value outside the plausible range is refused — a unit mix-up must not become a spot.
        if freq != 0 && !(MIN_FREQ_HZ..=MAX_FREQ_HZ).contains(&freq) {
            return self.reject();
        }
        let Some(s) = self.entry(sid.clone()) else {
            return Vec::new();
        };
        // A station first seen through `freq_change` still gets an identity, when it is given.
        if let Some(c) = call {
            s.call = c.to_ascii_uppercase();
        }
        if let Some(g) = grid {
            s.grid = g;
        }
        s.freq_hz = freq;
        vec![sid]
    }

    fn tx_report(&mut self, args: &Value) -> Vec<String> {
        let (Some(sid), Some(tx), Ok(mode), Ok(call), Ok(grid)) = (
            sid_of(args),
            args.get("transmitting").and_then(Value::as_bool),
            self.text(args, "mode", MAX_MODE),
            self.text(args, "callsign", MAX_CALL),
            self.text(args, "grid_square", MAX_GRID),
        ) else {
            return self.reject();
        };
        let Some(s) = self.entry(sid.clone()) else {
            return Vec::new();
        };
        if let Some(c) = call {
            s.call = c.to_ascii_uppercase();
        }
        if let Some(g) = grid {
            s.grid = g;
        }
        s.tx = tx;
        if let Some(m) = mode {
            s.mode = m;
        }
        vec![sid]
    }

    fn rx_report(&mut self, args: &Value) -> Vec<String> {
        // Keyed by the *heard* station's callsign, not by session: `sid` here is the receiver.
        let (Ok(Some(heard)), Ok(Some(receiver)), Some(snr)) = (
            self.text(args, "callsign", MAX_CALL),
            self.text(args, "receiver_callsign", MAX_CALL),
            args.get("snr").and_then(Value::as_f64),
        ) else {
            return self.reject();
        };
        if !(-60.0..=200.0).contains(&snr) {
            return self.reject();
        }
        // An empty callsign is a routine part of the event (a receiver saying it hears *something*,
        // or the server clearing a station's receive data when it retunes): no sighting to record,
        // and not a malformed event either.
        if heard.is_empty() {
            return Vec::new();
        }
        let (heard, receiver) = (heard.to_ascii_uppercase(), receiver.to_ascii_uppercase());
        let mut changed = Vec::new();
        for (sid, s) in &mut self.by_sid {
            if s.call == heard {
                s.heard_by = Some((receiver.clone(), snr.round() as i16));
                changed.push(sid.clone());
            }
        }
        changed
    }

    fn message_update(&mut self, args: &Value) -> Vec<String> {
        // The key must be there (an event without it is malformed); its value may be empty, `null`
        // or text this client does not keep (printable ASCII only, by policy), all of which clear it.
        let (Some(sid), Some(_), Ok(message)) = (
            sid_of(args),
            args.get("message"),
            self.text(args, "message", MAX_MESSAGE),
        ) else {
            return self.reject();
        };
        let Some(s) = self.entry(sid.clone()) else {
            return Vec::new();
        };
        s.message = message.unwrap_or_default();
        vec![sid]
    }

    /// The spot for a station, stamped `now` (Unix seconds), or `None` if it cannot be placed: no
    /// usable callsign, or no frequency yet.
    pub fn spot(&self, sid: &str, now: u64) -> Option<Spot> {
        let s = self.by_sid.get(sid)?;
        // No frequency yet is a zero one, which `Spot::new` refuses.
        let mut spot = Spot::new(&s.call, s.freq_hz, now, Network::FreeDvReporter)?;
        if !s.mode.is_empty() {
            spot = spot.with_mode(&s.mode);
        }
        // What the overlay has no column for goes in the comment: the operator's own status line,
        // and that the station is transmitting.
        let mut parts: Vec<&str> = Vec::new();
        if !s.message.is_empty() {
            parts.push(&s.message);
        }
        if s.tx {
            parts.push("TX");
        }
        if !parts.is_empty() {
            let joined = parts.join(" - ");
            // A message too long to keep is dropped rather than cut, and the TX marker stays.
            let comment = sanitise_text(&joined).or_else(|| s.tx.then(|| "TX".to_string()));
            if let Some(c) = comment {
                spot = spot.with_comment(&c);
            }
        }
        if let Some((by, snr)) = &s.heard_by {
            spot = spot.with_spotter(by).with_snr(*snr);
        }
        Some(spot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    fn ev(r: &mut Roster, name: &str, args: &str) -> Vec<String> {
        r.on_event(name, &parse(args).unwrap())
    }

    /// FR-SPOT-08: events build the roster the way the server describes it, and a row becomes a
    /// spot with the frequency, mode, message, TX marker and who heard it.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_freedv_events_build_the_roster_and_spots() {
        let mut r = Roster::default();
        assert_eq!(
            ev(
                &mut r,
                "new_connection",
                r#"{"sid":"s1","callsign":"aa1aaa","grid_square":"FN31"}"#
            ),
            ["s1"]
        );
        // No frequency yet: no spot.
        assert_eq!(r.spot("s1", 1000), None);
        assert_eq!(
            ev(&mut r, "freq_change", r#"{"sid":"s1","freq":14236000}"#),
            ["s1"]
        );
        let s = r.spot("s1", 1000).unwrap();
        assert_eq!(
            (s.call.as_str(), s.freq_hz, s.time, s.network),
            ("AA1AAA", 14_236_000, 1000, Network::FreeDvReporter)
        );
        assert_eq!(
            (
                s.mode.as_deref(),
                s.comment.as_deref(),
                s.spotter.as_deref(),
                s.snr_db
            ),
            (None, None, None, None)
        );
        // Mode and TX from a tx_report; the message from a message_update.
        ev(
            &mut r,
            "tx_report",
            r#"{"sid":"s1","transmitting":true,"mode":"RADEV1"}"#,
        );
        ev(
            &mut r,
            "message_update",
            r#"{"sid":"s1","message":"CQ FreeDV"}"#,
        );
        let s = r.spot("s1", 2000).unwrap();
        assert_eq!(s.mode.as_deref(), Some("RADEV1"));
        assert_eq!(s.comment.as_deref(), Some("CQ FreeDV - TX"));
        ev(&mut r, "tx_report", r#"{"sid":"s1","transmitting":false}"#);
        assert_eq!(
            r.spot("s1", 2000).unwrap().comment.as_deref(),
            Some("CQ FreeDV")
        );
        assert_eq!(
            r.spot("s1", 2000).unwrap().mode.as_deref(),
            Some("RADEV1"),
            "the mode is kept"
        );
        // Who heard it, and how well — matched by the *heard* callsign.
        assert_eq!(
            ev(
                &mut r,
                "rx_report",
                r#"{"sid":"s9","callsign":"aa1aaa","receiver_callsign":"bb2bbb","snr":-7.4}"#
            ),
            ["s1"]
        );
        let s = r.spot("s1", 2000).unwrap();
        assert_eq!((s.spotter.as_deref(), s.snr_db), (Some("BB2BBB"), Some(-7)));
        // A retune changes the frequency; the station stays one row.
        ev(&mut r, "freq_change", r#"{"sid":"s1","freq":7177000}"#);
        assert_eq!(r.len(), 1);
        assert_eq!(r.spot("s1", 2000).unwrap().freq_hz, 7_177_000);
        // Leaving removes the row.
        assert_eq!(
            ev(&mut r, "remove_connection", r#"{"sid":"s1"}"#),
            Vec::<String>::new()
        );
        assert!(r.is_empty());
        assert_eq!(r.spot("s1", 2000), None);
        assert_eq!(r.rejected, 0);
        // A frequency event may carry the identity too (a station first seen that way).
        assert_eq!(
            ev(
                &mut r,
                "freq_change",
                r#"{"sid":"s2","freq":14236000,"callsign":"cc3ccc","grid_square":"JO50"}"#
            ),
            ["s2"]
        );
        assert_eq!(r.spot("s2", 1).unwrap().call, "CC3CCC");
        // A message that cannot be kept (too long for a spot) is dropped, the TX marker stays.
        ev(
            &mut r,
            "message_update",
            &format!(r#"{{"sid":"s2","message":"{}"}}"#, "x".repeat(100)),
        );
        ev(&mut r, "tx_report", r#"{"sid":"s2","transmitting":true}"#);
        assert_eq!(r.spot("s2", 1).unwrap().comment.as_deref(), Some("TX"));
        // Unknown events change nothing and are not errors.
        assert_eq!(
            ev(&mut r, "something_new", r#"{"sid":"s2"}"#),
            Vec::<String>::new()
        );
        assert_eq!(r.rejected, 0);
    }

    /// FR-SPOT-08: `bulk_update` is a list of events applied in order; a `bulk_update` inside one
    /// is refused; and an item that is malformed costs only itself.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_freedv_bulk_update() {
        let mut r = Roster::default();
        let changed = ev(
            &mut r,
            "bulk_update",
            r#"[["new_connection",{"sid":"a","callsign":"AA1AAA"}],["freq_change",{"sid":"a","freq":14236000}],
                ["new_connection",{"sid":"b","callsign":"BB2BBB"}],["freq_change",{"sid":"b","freq":7177000}],
                ["freq_change",{"sid":"a","freq":14236000}]]"#,
        );
        assert_eq!(changed, ["a", "b"], "each changed row once, in order");
        assert_eq!(r.len(), 2);
        assert_eq!(r.spot("b", 5).unwrap().freq_hz, 7_177_000);
        // Malformed items are counted and skipped; the rest still applies.
        let before = r.rejected;
        let changed = ev(
            &mut r,
            "bulk_update",
            r#"[5, ["freq_change"], [1,2], ["freq_change",{"sid":"a","freq":"x"}], ["freq_change",{"sid":"a","freq":14074000}], null]"#,
        );
        assert_eq!(changed, ["a"]);
        assert_eq!(r.spot("a", 5).unwrap().freq_hz, 14_074_000);
        assert_eq!(r.rejected - before, 5, "five bad items counted");
        // Nested bulk_update is refused.
        let before = r.rejected;
        assert!(ev(
            &mut r,
            "bulk_update",
            r#"[["bulk_update",[["new_connection",{"sid":"z","callsign":"ZZ9ZZZ"}]]]]"#
        )
        .is_empty());
        assert_eq!(r.rejected - before, 1);
        assert_eq!(r.len(), 2, "the nested one was not applied");
        // Not a list.
        let before = r.rejected;
        assert!(ev(&mut r, "bulk_update", r#"{"a":1}"#).is_empty());
        assert_eq!(r.rejected - before, 1);
        // Only MAX_BULK items are read. Every item retunes the *same* station, so the roster cannot
        // hide the count (it is capped lower than MAX_BULK): the last frequency it holds says how
        // many items were applied.
        let mut r = Roster::default();
        let items: Vec<Value> = (0..10_005)
            .map(|i| {
                parse(&format!(
                    r#"["freq_change",{{"sid":"one","freq":{},"callsign":"AA1AAA"}}]"#,
                    1_000_000 + i
                ))
                .unwrap()
            })
            .collect();
        // (The JSON node cap stops a list this long at parse time, so the value is built by hand.)
        r.on_event("bulk_update", &Value::Arr(items));
        assert_eq!(
            r.spot("one", 1).unwrap().freq_hz,
            1_000_000 + 10_000 - 1,
            "exactly 10 000 items are applied"
        );
    }

    /// FR-SPOT-08: the frequency bounds are exact, and an `rx_report` finds every session of the
    /// heard station, rounds the report rather than cutting it, and never matches a station that
    /// has no callsign.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_freedv_boundaries_and_reports() {
        let mut r = Roster::default();
        // The floor and ceiling themselves are accepted, one either side is not.
        for (freq, ok) in [
            (99_999u64, false),
            (100_000, true),
            (300_000_000_000, true),
            (300_000_000_001, false),
        ] {
            let before = r.rejected;
            let changed = ev(
                &mut r,
                "freq_change",
                &format!(r#"{{"sid":"f","freq":{freq},"callsign":"AA1AAA"}}"#),
            );
            assert_eq!(!changed.is_empty(), ok, "{freq}");
            assert_eq!(r.rejected - before, u64::from(!ok), "{freq}");
        }
        // A session id is kept up to 64 characters and refused beyond.
        let mut r = Roster::default();
        for (len, ok) in [(64usize, true), (65, false)] {
            let sid = "s".repeat(len);
            let changed = ev(
                &mut r,
                "freq_change",
                &format!(r#"{{"sid":"{sid}","freq":14236000}}"#),
            );
            assert_eq!(!changed.is_empty(), ok, "{len}");
        }
        // Two sessions of one operator, and a station with no callsign at all.
        let mut r = Roster::default();
        ev(
            &mut r,
            "freq_change",
            r#"{"sid":"one","freq":14236000,"callsign":"aa1aaa"}"#,
        );
        ev(
            &mut r,
            "freq_change",
            r#"{"sid":"two","freq":7177000,"callsign":"AA1AAA"}"#,
        );
        ev(&mut r, "freq_change", r#"{"sid":"anon","freq":14236000}"#);
        ev(
            &mut r,
            "freq_change",
            r#"{"sid":"other","freq":14236000,"callsign":"BB2BBB"}"#,
        );
        let changed = ev(
            &mut r,
            "rx_report",
            r#"{"callsign":"AA1AAA","receiver_callsign":"dd4ddd","snr":-7.6}"#,
        );
        let mut changed_sorted = changed.clone();
        changed_sorted.sort();
        assert_eq!(
            changed_sorted,
            ["one", "two"],
            "both sessions of the heard station"
        );
        for sid in ["one", "two"] {
            let s = r.spot(sid, 1).unwrap();
            assert_eq!(
                (s.spotter.as_deref(), s.snr_db),
                (Some("DD4DDD"), Some(-8)),
                "-7.6 rounds to -8"
            );
        }
        assert_eq!(
            r.spot("other", 1).unwrap().spotter,
            None,
            "another station is untouched"
        );
        // Half rounds away from zero, as `f64::round` does; positive too.
        ev(
            &mut r,
            "rx_report",
            r#"{"callsign":"BB2BBB","receiver_callsign":"dd4ddd","snr":3.5}"#,
        );
        assert_eq!(r.spot("other", 1).unwrap().snr_db, Some(4));
        // An empty heard callsign is not a sighting, and does not match the station with none.
        let before = r.rejected;
        assert!(ev(
            &mut r,
            "rx_report",
            r#"{"callsign":"","receiver_callsign":"ee5eee","snr":1}"#
        )
        .is_empty());
        assert_eq!(r.rejected, before, "routine, not malformed");
        assert_eq!(r.spot("one", 1).unwrap().spotter.as_deref(), Some("DD4DDD"));
        // (`anon` has no usable callsign so has no spot to inspect; the empty report must simply
        // not have changed anything, which the empty result above shows.)
        assert_eq!(r.spot("anon", 1), None);
    }

    /// FR-SPOT-08: a field of the wrong type, a bad id, an out-of-range number or a hostile string
    /// drops the event whole and leaves what was known alone.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_freedv_hostile_events_are_dropped_whole() {
        let mut r = Roster::default();
        ev(
            &mut r,
            "freq_change",
            r#"{"sid":"good","freq":14236000,"callsign":"AA1AAA"}"#,
        );
        let snapshot = r.spot("good", 1).unwrap();
        let long = "x".repeat(200);
        let bad = vec![
            ("new_connection", r#"{"callsign":"AA1AAA"}"#.to_string()),
            (
                "new_connection",
                r#"{"sid":"","callsign":"AA1AAA"}"#.to_string(),
            ),
            (
                "new_connection",
                r#"{"sid":"has space","callsign":"AA1AAA"}"#.to_string(),
            ),
            (
                "new_connection",
                format!(r#"{{"sid":"{long}","callsign":"AA1AAA"}}"#),
            ),
            (
                "new_connection",
                r#"{"sid":5,"callsign":"AA1AAA"}"#.to_string(),
            ),
            ("new_connection", r#"{"sid":"a","callsign":5}"#.to_string()),
            ("new_connection", "[]".to_string()),
            ("new_connection", "null".to_string()),
            ("remove_connection", r#"{"sid":7}"#.to_string()),
            ("freq_change", r#"{"sid":"a"}"#.to_string()),
            (
                "freq_change",
                r#"{"sid":"a","freq":"14236000"}"#.to_string(),
            ),
            ("freq_change", r#"{"sid":"a","freq":-1}"#.to_string()),
            ("freq_change", r#"{"sid":"a","freq":14236.5}"#.to_string()),
            ("freq_change", r#"{"sid":"a","freq":99999}"#.to_string()),
            (
                "freq_change",
                r#"{"sid":"a","freq":300000000001}"#.to_string(),
            ),
            ("freq_change", r#"{"sid":"a","freq":1e30}"#.to_string()),
            ("tx_report", r#"{"sid":"a"}"#.to_string()),
            (
                "tx_report",
                r#"{"sid":"a","transmitting":"yes"}"#.to_string(),
            ),
            ("tx_report", r#"{"sid":"a","transmitting":1}"#.to_string()),
            (
                "tx_report",
                r#"{"sid":"a","transmitting":true,"mode":5}"#.to_string(),
            ),
            ("rx_report", r#"{"callsign":"AA1AAA"}"#.to_string()),
            (
                "rx_report",
                r#"{"callsign":"AA1AAA","receiver_callsign":"BB2BBB"}"#.to_string(),
            ),
            (
                "rx_report",
                r#"{"callsign":"AA1AAA","receiver_callsign":"BB2BBB","snr":"9"}"#.to_string(),
            ),
            (
                "rx_report",
                r#"{"callsign":"AA1AAA","receiver_callsign":"BB2BBB","snr":-61}"#.to_string(),
            ),
            (
                "rx_report",
                r#"{"callsign":"AA1AAA","receiver_callsign":"BB2BBB","snr":201}"#.to_string(),
            ),
            (
                "rx_report",
                r#"{"callsign":"AA1AAA","receiver_callsign":5,"snr":1}"#.to_string(),
            ),
            ("message_update", r#"{"sid":"a"}"#.to_string()),
            ("message_update", r#"{"sid":"a","message":5}"#.to_string()),
        ];
        for (name, args) in &bad {
            let before = r.rejected;
            let changed = ev(&mut r, name, args);
            assert!(changed.is_empty(), "{name} {args}");
            assert_eq!(r.rejected, before + 1, "{name} {args} must be counted");
            assert_eq!(r.len(), 1, "{name} {args} added a station");
        }
        assert_eq!(
            r.spot("good", 1).unwrap(),
            snapshot,
            "the known station is untouched"
        );
        // Empty callsign in an rx_report is routine: not a sighting, and not counted as bad.
        let before = r.rejected;
        assert!(ev(
            &mut r,
            "rx_report",
            r#"{"callsign":"","receiver_callsign":"BB2BBB","snr":3}"#
        )
        .is_empty());
        assert_eq!(r.rejected, before);
        // A callsign that is not a callsign is stored but never becomes a spot.
        ev(
            &mut r,
            "freq_change",
            r#"{"sid":"odd","freq":14236000,"callsign":"CQ"}"#,
        );
        assert_eq!(r.spot("odd", 1), None);
        ev(&mut r, "freq_change", r#"{"sid":"none","freq":14236000}"#);
        assert_eq!(r.spot("none", 1), None, "no callsign, no spot");
        assert_eq!(r.spot("missing", 1), None);
    }

    /// FR-SPOT-08: shapes seen on the **real service** (`freedv_live`, 2026-09-21): a `freq_change`
    /// with `freq` 0 clears the frequency and is not malformed; a `message_update` with non-ASCII
    /// text clears the message and is not malformed; an unusable well-typed field degrades that
    /// field only while the rest of the event applies; a wrong *type* still rejects the event; and
    /// the fields this client does not use (`last_update`) are ignored.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_freedv_real_service_shapes() {
        let mut r = Roster::default();
        ev(
            &mut r,
            "freq_change",
            r#"{"sid":"s","freq":14236000,"callsign":"aa1aaa","grid_square":"FN31pr","last_update":"2026-09-21T05:07:00.000000000Z"}"#,
        );
        assert_eq!(r.spot("s", 1).unwrap().freq_hz, 14_236_000);
        // Zero clears the frequency: the row changed (so the caller is told), it has no spot any
        // more, and nothing was counted as malformed.
        assert_eq!(
            ev(
                &mut r,
                "freq_change",
                r#"{"sid":"s","freq":0,"callsign":"AA1AAA","grid_square":"FN31pr","last_update":"x"}"#
            ),
            ["s"]
        );
        assert_eq!(
            r.spot("s", 2),
            None,
            "a station with no frequency has no plate"
        );
        assert_eq!(r.rejected, 0);
        // A frequency again puts it back.
        ev(&mut r, "freq_change", r#"{"sid":"s","freq":7177000}"#);
        assert_eq!(r.spot("s", 3).unwrap().freq_hz, 7_177_000);
        // A non-ASCII message clears the message; the event is accepted.
        ev(&mut r, "message_update", r#"{"sid":"s","message":"CQ"}"#);
        assert_eq!(r.spot("s", 3).unwrap().comment.as_deref(), Some("CQ"));
        let accent = "73 de F5ABC \u{e9}t\u{e9} \u{1f600}";
        let changed = ev(
            &mut r,
            "message_update",
            &format!(r#"{{"sid":"s","message":"{accent}","last_update":"x"}}"#),
        );
        assert_eq!(changed, ["s"]);
        assert_eq!(
            r.spot("s", 3).unwrap().comment,
            None,
            "the message that cannot be shown is gone"
        );
        assert_eq!(r.rejected, 0);
        // An empty and a null message also clear it.
        for msg in ["\"\"", "null"] {
            ev(&mut r, "message_update", r#"{"sid":"s","message":"CQ"}"#);
            ev(
                &mut r,
                "message_update",
                &format!(r#"{{"sid":"s","message":{msg}}}"#),
            );
            assert_eq!(r.spot("s", 3).unwrap().comment, None, "{msg}");
        }
        assert_eq!(r.rejected, 0);
        // An unusable well-typed field degrades that field only: the frequency still applies.
        let long = "x".repeat(200);
        ev(
            &mut r,
            "freq_change",
            &format!(r#"{{"sid":"t","freq":14074000,"callsign":"F5é","grid_square":"{long}"}}"#),
        );
        assert_eq!(r.len(), 2);
        assert_eq!(
            r.spot("t", 1),
            None,
            "no usable callsign, so no plate — but the row is there"
        );
        ev(
            &mut r,
            "freq_change",
            r#"{"sid":"t","freq":14074000,"callsign":"BB2BBB"}"#,
        );
        assert_eq!(r.spot("t", 1).unwrap().call, "BB2BBB");
        ev(
            &mut r,
            "tx_report",
            &format!(r#"{{"sid":"t","transmitting":true,"mode":"{long}"}}"#),
        );
        let t = r.spot("t", 1).unwrap();
        assert_eq!(
            (t.mode.as_deref(), t.comment.as_deref()),
            (None, Some("TX")),
            "the mode is dropped, TX kept"
        );
        assert_eq!(r.rejected, 0);
        // A wrong *type* is still a malformed event, and changes nothing.
        for (name, args) in [
            ("message_update", r#"{"sid":"s","message":5}"#),
            ("message_update", r#"{"sid":"s"}"#),
            ("freq_change", r#"{"sid":"s","freq":14236000,"callsign":5}"#),
            ("tx_report", r#"{"sid":"s","transmitting":true,"mode":5}"#),
            (
                "new_connection",
                r#"{"sid":"s","callsign":"AA1AAA","grid_square":[]}"#,
            ),
        ] {
            let before = r.rejected;
            assert!(ev(&mut r, name, args).is_empty(), "{name} {args}");
            assert_eq!(r.rejected, before + 1, "{name} {args}");
        }
        // Nonzero and outside the plausible range is refused, not turned into a spot.
        let before = r.rejected;
        assert!(ev(&mut r, "freq_change", r#"{"sid":"s","freq":14236}"#).is_empty());
        assert_eq!(r.rejected, before + 1);
        assert_eq!(
            r.spot("s", 3).unwrap().freq_hz,
            7_177_000,
            "the frequency it had is kept"
        );
        // The diagnostic says which events were refused and their shape, never a value.
        let shapes = r.rejected_shapes();
        assert_eq!(shapes["freq_change"].0, 2);
        assert!(
            shapes["freq_change"].1.contains("freq:int(below-floor)"),
            "{shapes:?}"
        );
        // Two message_updates were refused (a number, and no message at all); the shape kept is the
        // last one's: a session id and nothing else.
        assert_eq!(shapes["message_update"].0, 2);
        assert_eq!(shapes["message_update"].1, "sid:str(len=1,ascii)");
        assert!(
            !format!("{shapes:?}").contains("F5ABC"),
            "no value leaks into the diagnostic"
        );
    }

    /// FR-SPOT-08: the diagnostic shape names each field and the *kind* of its value — a coarse
    /// class for numbers, a length and character class for text — in a fixed order, and contains
    /// no value.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_freedv_diagnostic_shapes() {
        let sh = |json: &str| shape(&parse(json).unwrap());
        // Every kind of value, and every class of whole number.
        assert_eq!(sh(r#"{"a":null}"#), "a:null");
        assert_eq!(sh(r#"{"a":true}"#), "a:bool");
        assert_eq!(sh(r#"{"a":1.5}"#), "a:float");
        assert_eq!(sh(r#"{"a":[1]}"#), "a:array");
        assert_eq!(sh(r#"{"a":{"b":1}}"#), "a:object");
        assert_eq!(sh(r#"{"a":-1}"#), "a:int(negative)");
        assert_eq!(sh(r#"{"a":0}"#), "a:int(zero)");
        assert_eq!(sh(r#"{"a":99999}"#), "a:int(below-floor)");
        assert_eq!(sh(r#"{"a":100000}"#), "a:int(in-range)");
        assert_eq!(sh(r#"{"a":300000000000}"#), "a:int(in-range)");
        assert_eq!(sh(r#"{"a":300000000001}"#), "a:int(above-ceiling)");
        assert_eq!(sh(r#"{"a":"abc"}"#), "a:str(len=3,ascii)");
        assert_eq!(sh(r#"{"a":""}"#), "a:str(len=0,ascii)");
        assert_eq!(sh("{\"a\":\"\u{e9}\"}"), "a:str(len=2,non-ascii)");
        // Keys are sorted, so the shape does not depend on the order the server wrote them in.
        assert_eq!(
            sh(r#"{"z":1,"a":"x","m":null}"#),
            "a:str(len=1,ascii),m:null,z:int(below-floor)"
        );
        // Not an object: just the kind.
        assert_eq!(sh("[1,2]"), "array");
        assert_eq!(sh("\"secret text\""), "str(len=11,ascii)");
        assert_eq!(sh("null"), "null");
        // No value ever appears.
        for json in [r#"{"callsign":"AA1AAA","message":"hello"}"#, r#""AA1AAA""#] {
            let out = sh(json);
            assert!(!out.contains("AA1AAA") && !out.contains("hello"), "{out}");
        }
    }

    /// FR-SPOT-08: what the roster *keeps* of a text field is bounded and clean — the exact limits,
    /// no surrounding whitespace, nothing that is not printable ASCII — checked on the stored rows
    /// (the public spot would hide a difference, since building a spot sanitises again).
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_freedv_stored_text_is_bounded_and_clean() {
        let mut r = Roster::default();
        let s = |n: usize| "x".repeat(n);
        let feed = |r: &mut Roster, sid: &str, call: &str, grid: &str, mode: &str, msg: &str| {
            ev(
                r,
                "freq_change",
                &format!(
                    r#"{{"sid":"{sid}","freq":14236000,"callsign":"{call}","grid_square":"{grid}"}}"#
                ),
            );
            ev(
                r,
                "tx_report",
                &format!(r#"{{"sid":"{sid}","transmitting":false,"mode":"{mode}"}}"#),
            );
            ev(
                r,
                "message_update",
                &format!(r#"{{"sid":"{sid}","message":"{msg}"}}"#),
            );
        };
        // At every limit: kept (and the callsign upper-cased).
        feed(&mut r, "at", &s(32), &s(8), &s(16), &s(128));
        let st = &r.by_sid["at"];
        assert_eq!(st.call, s(32).to_ascii_uppercase());
        assert_eq!(
            (st.grid.len(), st.mode.len(), st.message.len()),
            (8, 16, 128)
        );
        // One over: dropped, not cut.
        feed(
            &mut r,
            "over",
            &s(32 + 1),
            &s(8 + 1),
            &s(16 + 1),
            &s(128 + 1),
        );
        let st = &r.by_sid["over"];
        assert_eq!(
            (
                st.call.as_str(),
                st.grid.as_str(),
                st.mode.as_str(),
                st.message.as_str()
            ),
            ("", "", "", "")
        );
        assert_eq!(
            st.freq_hz, 14_236_000,
            "the rest of the events still applied"
        );
        // Not printable ASCII: dropped. Surrounding whitespace: trimmed.
        feed(
            &mut r,
            "odd",
            "F5\u{e9}",
            "FN\u{e9}31",
            "RA\u{e9}",
            "caf\u{e9}",
        );
        let st = &r.by_sid["odd"];
        assert_eq!(
            (
                st.call.as_str(),
                st.grid.as_str(),
                st.mode.as_str(),
                st.message.as_str()
            ),
            ("", "", "", "")
        );
        feed(
            &mut r,
            "pad",
            "  aa1aaa ",
            " FN31 ",
            " RADEV1 ",
            "  hi there  ",
        );
        let st = &r.by_sid["pad"];
        assert_eq!(
            (
                st.call.as_str(),
                st.grid.as_str(),
                st.mode.as_str(),
                st.message.as_str()
            ),
            ("AA1AAA", "FN31", "RADEV1", "hi there")
        );
        // Whitespace only is empty, not a message of spaces; and a limit counts the trimmed text.
        feed(&mut r, "blank", "   ", "  ", "  ", "     ");
        let st = &r.by_sid["blank"];
        assert_eq!(
            (st.grid.as_str(), st.mode.as_str(), st.message.as_str()),
            ("", "", "")
        );
        feed(&mut r, "trim", "a", "a", "a", &format!("  {}  ", s(128)));
        assert_eq!(r.by_sid["trim"].message.len(), 128);
        assert_eq!(r.rejected, 0);
    }

    /// FR-SPOT-08: a field that is well-typed but cannot be kept is **counted** as degraded, with a
    /// value-free shape, so it stays visible: without this the probe that found the strict-parser
    /// problem would report "0 rejected" while every accented message was being cleared. Blank,
    /// `null` and absent are ordinary and are not counted; a wrong type is a rejection, not a
    /// degrade.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_freedv_degraded_fields_are_counted() {
        let mut r = Roster::default();
        // Ordinary: usable, blank, whitespace only, null, absent — none is degraded.
        ev(&mut r, "message_update", r#"{"sid":"s","message":"CQ"}"#);
        ev(&mut r, "message_update", r#"{"sid":"s","message":""}"#);
        ev(&mut r, "message_update", r#"{"sid":"s","message":"   "}"#);
        ev(&mut r, "message_update", r#"{"sid":"s","message":null}"#);
        ev(
            &mut r,
            "freq_change",
            r#"{"sid":"s","freq":14236000,"callsign":"aa1aaa"}"#,
        );
        assert_eq!((r.degraded, r.rejected), (0, 0));
        assert!(r.degraded_shapes().is_empty());

        // One unusable field per event: a non-ASCII message, a long grid, a long mode.
        ev(
            &mut r,
            "message_update",
            "{\"sid\":\"s\",\"message\":\"caf\u{e9}\"}",
        );
        assert_eq!(r.degraded, 1);
        let long = "x".repeat(200);
        ev(
            &mut r,
            "freq_change",
            &format!(r#"{{"sid":"s","freq":14236000,"grid_square":"{long}"}}"#),
        );
        assert_eq!(r.degraded, 2);
        ev(
            &mut r,
            "tx_report",
            &format!(r#"{{"sid":"s","transmitting":true,"mode":"{long}"}}"#),
        );
        assert_eq!(r.degraded, 3);
        // Two unusable fields in one event count twice, and the event still applies.
        ev(
            &mut r,
            "freq_change",
            &format!(
                r#"{{"sid":"s","freq":7177000,"callsign":"F5{accent}","grid_square":"{long}"}}"#,
                accent = "\u{e9}"
            ),
        );
        assert_eq!(r.degraded, 5);
        assert_eq!(r.rejected, 0);
        assert_eq!(r.spot("s", 1).unwrap().freq_hz, 7_177_000);

        // The shapes name the event and the kind of each field, never a value.
        let shapes = r.degraded_shapes();
        assert_eq!(
            shapes["message_update"],
            (
                1,
                "message:str(len=5,non-ascii),sid:str(len=1,ascii)".to_string()
            )
        );
        assert_eq!(shapes["tx_report"].0, 1);
        assert_eq!(
            shapes["freq_change"].0, 3,
            "the long grid, then the event with two"
        );
        assert!(
            shapes["freq_change"]
                .1
                .contains("callsign:str(len=4,non-ascii)"),
            "{shapes:?}"
        );
        assert!(
            shapes["freq_change"]
                .1
                .contains("grid_square:str(len=200,ascii)"),
            "{shapes:?}"
        );
        assert!(!format!("{shapes:?}").contains("xxxx"), "no value leaks");

        // A wrong type is a rejection, not a degrade; the two counts stay apart.
        let before = (r.degraded, r.rejected);
        ev(&mut r, "message_update", r#"{"sid":"s","message":5}"#);
        assert_eq!((r.degraded, r.rejected), (before.0, before.1 + 1));
        assert_eq!(r.rejected_shapes()["message_update"].0, 1);
        assert_eq!(
            r.degraded_shapes()["message_update"].0,
            1,
            "the degrade map is untouched"
        );

        // Inside a bulk_update each item is counted on its own.
        let mut r = Roster::default();
        ev(
            &mut r,
            "bulk_update",
            "[[\"message_update\",{\"sid\":\"a\",\"message\":\"\u{e9}\"}],[\"message_update\",{\"sid\":\"b\",\"message\":\"\u{e9}\"}],[\"message_update\",{\"sid\":\"c\",\"message\":\"ok\"}]]",
        );
        assert_eq!(r.degraded, 2);
        assert_eq!(r.degraded_shapes()["message_update"].0, 2);
    }

    /// FR-SPOT-08: the roster is capped; a flood of invented session ids cannot grow it, and the
    /// stations already there keep working.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_freedv_roster_is_capped() {
        let mut r = Roster::default();
        for i in 0..4146 {
            ev(
                &mut r,
                "new_connection",
                &format!(r#"{{"sid":"s{i}","callsign":"AA1AAA"}}"#),
            );
        }
        assert_eq!(r.len(), 4096);
        assert_eq!(r.dropped, 50);
        // Every entry point is capped, not just the first.
        ev(&mut r, "freq_change", r#"{"sid":"new1","freq":14236000}"#);
        ev(&mut r, "tx_report", r#"{"sid":"new2","transmitting":true}"#);
        ev(&mut r, "message_update", r#"{"sid":"new3","message":"hi"}"#);
        assert_eq!(r.len(), 4096);
        assert_eq!(r.dropped, 53);
        // An existing station is still updated at the cap, and one leaving makes room.
        assert_eq!(
            ev(&mut r, "freq_change", r#"{"sid":"s0","freq":14236000}"#),
            ["s0"]
        );
        ev(&mut r, "remove_connection", r#"{"sid":"s1"}"#);
        ev(
            &mut r,
            "new_connection",
            r#"{"sid":"fresh","callsign":"BB2BBB"}"#,
        );
        assert_eq!(r.len(), 4096);
        assert!(r.sids().any(|s| s == "fresh"));
        r.clear();
        assert!(r.is_empty());
    }
}
