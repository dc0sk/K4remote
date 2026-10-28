//! The callsign lookup while typing (FR-LOG-05; design §5).
//!
//! Wavelog's `GET /api/v2/lookup` answers with the DXCC entity, continent, name and grid it knows
//! for a call, and whether the call was worked before — at all, on this band, on this band and
//! mode. The mode asked is the **main** ADIF mode (`MFSK`, not `FT4`): Wavelog compares the main
//! mode, so `FT4` would never match a stored FT4 QSO (probed 2026-10-27).

use serde_json::Value;

/// Whether a call is worth asking about: at least three characters, one of them a digit, and only
/// the characters a callsign has (so it goes into the query unescaped).
pub fn wants_lookup(call: &str) -> bool {
    let c = call.trim();
    c.len() >= 3
        && c.bytes().any(|b| b.is_ascii_digit())
        && c.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'/')
}

/// The query string for a lookup of `call` on `band` in the main ADIF mode `mode`.
pub fn query(call: &str, band: &str, mode: &str) -> String {
    format!(
        "callsign={}&band={band}&mode={mode}&detail=full",
        call.trim()
    )
}

/// How far the call was worked before.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Worked {
    /// Never.
    New,
    /// On another band.
    Call,
    /// On this band, in another mode.
    Band,
    /// On this band and in this mode.
    BandMode,
}

/// What the lookup found for one call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lookup {
    pub call: String,
    pub dxcc: String,
    pub cont: String,
    pub name: String,
    pub grid: String,
    pub worked: Worked,
}

impl Lookup {
    /// One line for the log strip: "FEDERAL REPUBLIC OF GERMANY (EU) · Test · JO62 · worked on
    /// this band and mode".
    pub fn line(&self) -> String {
        let mut parts = Vec::new();
        match (self.dxcc.is_empty(), self.cont.is_empty()) {
            (false, false) => parts.push(format!("{} ({})", self.dxcc, self.cont)),
            (false, true) => parts.push(self.dxcc.clone()),
            _ => {}
        }
        for s in [&self.name, &self.grid] {
            if !s.is_empty() {
                parts.push(s.clone());
            }
        }
        parts.push(
            match self.worked {
                Worked::New => "new",
                Worked::Call => "worked before, not on this band",
                Worked::Band => "worked on this band, not in this mode",
                Worked::BandMode => "worked on this band and mode",
            }
            .to_string(),
        );
        parts.join(" · ")
    }
}

/// Parse a lookup reply (`{"data":{…}}`) for the call asked; `None` when it is not one, or is for
/// another call.
pub fn parse(body: &[u8], asked: &str) -> Option<Lookup> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let d = v.get("data")?;
    let call = d["callsign"].as_str()?.trim().to_ascii_uppercase();
    if call != asked.trim() {
        return None;
    }
    // `gridsquare` comes as null as well as "" (probed).
    let text = |k: &str| d[k].as_str().unwrap_or("").trim().to_string();
    let yes = |k: &str| d[k].as_bool().unwrap_or(false);
    let worked = if yes("call_worked_band_mode") {
        Worked::BandMode
    } else if yes("call_worked_band") {
        Worked::Band
    } else if yes("call_worked") {
        Worked::Call
    } else {
        Worked::New
    };
    Some(Lookup {
        call,
        dxcc: text("dxcc"),
        cont: text("cont"),
        name: text("name"),
        grid: text("gridsquare"),
        worked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed from a real reply of the local Wavelog (2026-10-28), `gridsquare` null as it came.
    const DL1ABC: &str = r#"{"data":{"callsign":"DL1ABC","dxcc":"FEDERAL REPUBLIC OF GERMANY","dxcc_id":"230","cont":"EU","name":"Test","gridsquare":null,"call_worked":true,"call_worked_band":true,"call_worked_band_mode":false,"lotw_member":false},"meta":{"resource":"lookup"}}"#;

    /// FR-LOG-05: from three characters with a digit, callsign characters only.
    /// trace: FR-LOG-05
    #[test]
    fn fr_log_05_which_calls_are_looked_up() {
        for c in [
            "DL1",
            "DL1ABC",
            "DL1ABC/P",
            "EA8/DL1ABC",
            "9A1A",
            " DL1ABC ",
        ] {
            assert!(wants_lookup(c), "{c}");
        }
        for c in [
            "",
            "DL",
            "D1",
            "DLABC",
            "dl1abc",
            "DL1 ABC",
            "DL1ABC&x=1",
            "DL1Ä",
        ] {
            assert!(!wants_lookup(c), "{c}");
        }
    }

    /// FR-LOG-05: the query carries the band and the main mode.
    /// trace: FR-LOG-05
    #[test]
    fn fr_log_05_the_query() {
        assert_eq!(
            query(" DL1ABC ", "20m", "MFSK"),
            "callsign=DL1ABC&band=20m&mode=MFSK&detail=full"
        );
    }

    /// FR-LOG-05: the reply's fields, the worked level from the most specific flag, and a reply for
    /// another call refused.
    /// trace: FR-LOG-05
    #[test]
    fn fr_log_05_the_reply() {
        let l = parse(DL1ABC.as_bytes(), "DL1ABC").expect("parsed");
        assert_eq!(
            l,
            Lookup {
                call: "DL1ABC".into(),
                dxcc: "FEDERAL REPUBLIC OF GERMANY".into(),
                cont: "EU".into(),
                name: "Test".into(),
                grid: String::new(),
                worked: Worked::Band,
            }
        );
        assert_eq!(
            l.line(),
            "FEDERAL REPUBLIC OF GERMANY (EU) · Test · worked on this band, not in this mode"
        );
        let flags = |c: bool, b: bool, m: bool| {
            let body = format!(
                r#"{{"data":{{"callsign":"X1X","call_worked":{c},"call_worked_band":{b},"call_worked_band_mode":{m}}}}}"#
            );
            parse(body.as_bytes(), "X1X").expect("parsed").worked
        };
        assert_eq!(flags(false, false, false), Worked::New);
        assert_eq!(flags(true, false, false), Worked::Call);
        assert_eq!(flags(true, true, false), Worked::Band);
        assert_eq!(flags(true, true, true), Worked::BandMode);
        assert_eq!(parse(DL1ABC.as_bytes(), "DL1AB"), None, "another call");
        assert_eq!(parse(br#"{"error":{"code":"x"}}"#, "DL1ABC"), None);
        assert_eq!(parse(b"<html>", "DL1ABC"), None);
    }
}
