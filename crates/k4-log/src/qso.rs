//! The QSO record and its mapping from the radio (FR-LOG-01, FR-LOG-08).
//!
//! Field names are ADIF names in lower case, as Wavelog's API v2 takes them; frequencies in Hz.
//! Modes go as ADIF mode/submode pairs, because Wavelog stores them so and compares the **main**
//! mode for duplicates and for the worked-before lookup (probed 2026-10-27: FT4 and JS8 are both
//! `MFSK`; a lookup with `mode=FT4` does not match, `mode=MFSK` does).

use k4_protocol::state::Mode;
use serde::{Deserialize, Serialize};

use crate::band::band_for;

/// The digital mode the operator picks for DATA / DATA-R (the K4 does not know which one runs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DigitalMode {
    #[default]
    Ft8,
    Ft4,
    Js8,
    Rtty,
    Psk31,
}

impl DigitalMode {
    pub const ALL: [DigitalMode; 5] = [Self::Ft8, Self::Ft4, Self::Js8, Self::Rtty, Self::Psk31];

    /// The name shown in the form.
    pub fn label(self) -> &'static str {
        match self {
            Self::Ft8 => "FT8",
            Self::Ft4 => "FT4",
            Self::Js8 => "JS8",
            Self::Rtty => "RTTY",
            Self::Psk31 => "PSK31",
        }
    }
}

/// ADIF mode and submode for the radio's mode (and the digital mode chosen for DATA).
pub fn adif_mode(mode: Mode, digital: DigitalMode) -> (&'static str, Option<&'static str>) {
    match mode {
        Mode::Lsb => ("SSB", Some("LSB")),
        Mode::Usb => ("SSB", Some("USB")),
        Mode::Cw | Mode::CwRev => ("CW", None),
        Mode::Am => ("AM", None),
        Mode::Fm => ("FM", None),
        Mode::Data | Mode::DataRev => match digital {
            DigitalMode::Ft8 => ("FT8", None),
            DigitalMode::Ft4 => ("MFSK", Some("FT4")),
            DigitalMode::Js8 => ("MFSK", Some("JS8")),
            DigitalMode::Rtty => ("RTTY", None),
            DigitalMode::Psk31 => ("PSK", Some("PSK31")),
        },
    }
}

/// The RST the form starts with for an ADIF mode/submode: 59 for phone, 599 for CW, RTTY and
/// PSK, empty for the weak-signal modes (FT8, FT4, JS8 report dB).
pub fn default_rst(mode: &str, submode: Option<&str>) -> &'static str {
    match (mode, submode) {
        ("SSB" | "AM" | "FM", _) => "59",
        ("CW" | "RTTY" | "PSK", _) => "599",
        _ => "",
    }
}

/// Transmit power in watts from the K4's `PC` field and range (state.rs: `H` watts, `L` tenths
/// of a watt, `X` tenths of a milliwatt); `None` when either is unknown.
pub fn power_watts(raw: Option<u16>, range: Option<char>) -> Option<f64> {
    let n = f64::from(raw?);
    match range? {
        'H' => Some(n),
        'L' => Some(n / 10.0),
        'X' => Some(n / 10_000.0),
        _ => None,
    }
}

/// Transmit and receive frequency: the K4 transmits on VFO A, or on VFO B in split; the receive
/// frequency is given only when it differs.
pub fn frequencies(vfo_a: u64, vfo_b: Option<u64>, split: bool) -> (u64, Option<u64>) {
    match (split, vfo_b) {
        (true, Some(b)) if b != vfo_a => (b, Some(vfo_a)),
        (true, Some(b)) => (b, None),
        _ => (vfo_a, None),
    }
}

/// UTC date (`YYYY-MM-DD`) and time (`HHMMSS`) of a Unix time.
pub fn utc_date_time(unix: u64) -> (String, String) {
    let (days, secs) = ((unix / 86_400) as i64, unix % 86_400);
    // Civil date from days since 1970-01-01 (H. Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (
        format!("{year:04}-{month:02}-{day:02}"),
        format!("{:02}{:02}{:02}", secs / 3600, secs % 3600 / 60, secs % 60),
    )
}

/// What the log form holds when Log is pressed, plus the radio's state.
#[derive(Debug, Clone)]
pub struct Draft {
    pub call: String,
    pub rst_sent: String,
    pub rst_rcvd: String,
    pub name: String,
    pub comment: String,
    /// When the call was first typed, and when Log was pressed (Unix seconds, UTC).
    pub time_on: u64,
    pub time_off: u64,
    pub tx_hz: u64,
    pub rx_hz: Option<u64>,
    pub mode: Mode,
    pub digital: DigitalMode,
    pub tx_pwr: Option<f64>,
    /// The operator's callsign (a club logbook attributes the QSO by it).
    pub operator: Option<String>,
}

/// Why the form cannot be logged as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormError {
    NoCall,
    /// Characters a callsign does not have.
    BadCall(String),
    /// The frequency is in no band Wavelog knows.
    NoBand(u64),
    /// `time_off` before `time_on`.
    TimeOrder,
}

impl std::fmt::Display for FormError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCall => f.write_str("enter the call"),
            Self::BadCall(c) => write!(f, "{c:?} is not a callsign (letters, digits and /)"),
            Self::NoBand(hz) => {
                write!(f, "{:.3} MHz is in no band Wavelog knows", *hz as f64 / 1e6)
            }
            Self::TimeOrder => f.write_str("the end time is before the start time"),
        }
    }
}

/// A QSO as the API takes it (without the station location, which the queue entry carries).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Qso {
    pub call: String,
    pub band: String,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submode: Option<String>,
    pub freq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freq_rx: Option<u64>,
    pub qso_date: String,
    pub time_on: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_off: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rst_sent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rst_rcvd: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_pwr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
}

impl Qso {
    /// Build the QSO from the form and the radio, or say what is missing.
    pub fn from_draft(d: &Draft) -> Result<Self, FormError> {
        let call = d.call.trim().to_ascii_uppercase();
        if call.is_empty() {
            return Err(FormError::NoCall);
        }
        if !call.chars().all(|c| c.is_ascii_alphanumeric() || c == '/')
            || !call.chars().any(|c| c.is_ascii_digit())
        {
            return Err(FormError::BadCall(call));
        }
        let band = band_for(d.tx_hz).ok_or(FormError::NoBand(d.tx_hz))?;
        if d.time_off < d.time_on {
            return Err(FormError::TimeOrder);
        }
        let (mode, submode) = adif_mode(d.mode, d.digital);
        let (qso_date, time_on) = utc_date_time(d.time_on);
        let (_, time_off) = utc_date_time(d.time_off);
        let clean = |s: &str| s.trim().to_string();
        Ok(Self {
            call,
            band: band.to_string(),
            mode: mode.to_string(),
            submode: submode.map(str::to_string),
            freq: d.tx_hz,
            freq_rx: d.rx_hz.filter(|&rx| rx != d.tx_hz),
            qso_date,
            time_on,
            // Wavelog takes time_off as a time of day on the QSO's date; past midnight it is left
            // out rather than sent wrong.
            time_off: (d.time_off / 86_400 == d.time_on / 86_400).then_some(time_off),
            rst_sent: clean(&d.rst_sent),
            rst_rcvd: clean(&d.rst_rcvd),
            name: clean(&d.name),
            comment: clean(&d.comment),
            tx_pwr: d.tx_pwr,
            operator: d
                .operator
                .as_deref()
                .map(str::trim)
                .filter(|o| !o.is_empty())
                .map(str::to_ascii_uppercase),
        })
    }

    /// The JSON body of `POST /api/v2/qso` for station location `station_profile_id`.
    pub fn body(&self, station_profile_id: u32) -> String {
        let mut v = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        if let Some(o) = v.as_object_mut() {
            o.insert("station_profile_id".into(), station_profile_id.into());
        }
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> Draft {
        Draft {
            call: " dl1abc ".into(),
            rst_sent: "59".into(),
            rst_rcvd: " 57".into(),
            name: "Hans".into(),
            comment: "".into(),
            time_on: 1_793_087_400, // 2026-10-27 07:50:00 UTC (checked with Python datetime)
            time_off: 1_793_087_745,
            tx_hz: 14_250_000,
            rx_hz: None,
            mode: Mode::Usb,
            digital: DigitalMode::Ft8,
            tx_pwr: Some(100.0),
            operator: Some("dc0sk".into()),
        }
    }

    /// FR-LOG-08: every K4 mode maps to Wavelog's ADIF mode/submode pair — DATA by the operator's
    /// choice, FT4 and JS8 under MFSK as Wavelog stores them — and the RST defaults follow.
    /// trace: FR-LOG-08
    #[test]
    fn fr_log_08_modes_map_to_adif_pairs() {
        use DigitalMode::*;
        for (mode, digital, want) in [
            (Mode::Lsb, Ft8, ("SSB", Some("LSB"))),
            (Mode::Usb, Ft8, ("SSB", Some("USB"))),
            (Mode::Cw, Ft8, ("CW", None)),
            (Mode::CwRev, Ft8, ("CW", None)),
            (Mode::Am, Ft8, ("AM", None)),
            (Mode::Fm, Ft8, ("FM", None)),
            (Mode::Data, Ft8, ("FT8", None)),
            (Mode::DataRev, Ft4, ("MFSK", Some("FT4"))),
            (Mode::Data, Js8, ("MFSK", Some("JS8"))),
            (Mode::Data, Rtty, ("RTTY", None)),
            (Mode::Data, Psk31, ("PSK", Some("PSK31"))),
        ] {
            assert_eq!(adif_mode(mode, digital), want, "{mode:?} {digital:?}");
        }
        // Every K4 mode is covered (a new variant fails to compile in `adif_mode`); every digital
        // choice is in ALL exactly once.
        for d in DigitalMode::ALL {
            assert_eq!(DigitalMode::ALL.iter().filter(|x| **x == d).count(), 1);
        }
        assert_eq!(default_rst("SSB", Some("USB")), "59");
        assert_eq!(default_rst("CW", None), "599");
        assert_eq!(default_rst("RTTY", None), "599");
        assert_eq!(default_rst("MFSK", Some("FT4")), "");
        assert_eq!(default_rst("FT8", None), "");
    }

    /// FR-LOG-08: power in watts from the K4's range (H watts, L tenths of a watt, X tenths of a
    /// milliwatt — the same raw 050 is 50 W, 5 W and 5 mW); unknown when either part is.
    /// trace: FR-LOG-08
    #[test]
    fn fr_log_08_power_in_watts_per_range() {
        assert_eq!(power_watts(Some(50), Some('H')), Some(50.0));
        assert_eq!(power_watts(Some(50), Some('L')), Some(5.0));
        assert_eq!(power_watts(Some(50), Some('X')), Some(0.005));
        assert_eq!(power_watts(None, Some('H')), None);
        assert_eq!(power_watts(Some(50), None), None);
        assert_eq!(power_watts(Some(50), Some('?')), None);
    }

    /// FR-LOG-08: the transmit frequency is VFO A, or VFO B in split; the receive frequency only
    /// when it differs.
    /// trace: FR-LOG-08
    #[test]
    fn fr_log_08_split_sends_tx_and_rx() {
        assert_eq!(
            frequencies(14_200_000, Some(14_205_000), false),
            (14_200_000, None)
        );
        assert_eq!(
            frequencies(14_200_000, Some(14_205_000), true),
            (14_205_000, Some(14_200_000))
        );
        assert_eq!(
            frequencies(14_200_000, Some(14_200_000), true),
            (14_200_000, None)
        );
        assert_eq!(frequencies(14_200_000, None, true), (14_200_000, None));
    }

    /// FR-LOG-01: UTC date and time with seconds, across a year end and a leap day.
    /// trace: FR-LOG-01
    #[test]
    fn fr_log_01_utc_date_and_time() {
        assert_eq!(utc_date_time(0), ("1970-01-01".into(), "000000".into()));
        assert_eq!(
            utc_date_time(1_793_087_400),
            ("2026-10-27".into(), "075000".into())
        );
        assert_eq!(
            utc_date_time(1_798_761_599),
            ("2026-12-31".into(), "235959".into())
        );
        assert_eq!(
            utc_date_time(1_798_761_600),
            ("2027-01-01".into(), "000000".into())
        );
        assert_eq!(
            utc_date_time(1_835_395_200),
            ("2028-02-29".into(), "000000".into())
        );
        assert_eq!(
            utc_date_time(1_835_481_600),
            ("2028-03-01".into(), "000000".into())
        );
    }

    /// FR-LOG-01, FR-LOG-08: a draft becomes the API's fields — call trimmed and upper-cased,
    /// band from the frequency, seconds kept, empty fields left out, the station location in the
    /// body — and a form that cannot be logged says why.
    /// trace: FR-LOG-01, FR-LOG-08
    #[test]
    fn fr_log_01_a_draft_becomes_the_api_fields() {
        let q = Qso::from_draft(&draft()).unwrap();
        assert_eq!(
            (
                q.call.as_str(),
                q.band.as_str(),
                q.mode.as_str(),
                q.submode.as_deref()
            ),
            ("DL1ABC", "20m", "SSB", Some("USB"))
        );
        assert_eq!(
            (q.qso_date.as_str(), q.time_on.as_str()),
            ("2026-10-27", "075000")
        );
        assert_eq!(q.time_off.as_deref(), Some("075545"));
        assert_eq!(q.rst_rcvd, "57");
        assert_eq!(q.operator.as_deref(), Some("DC0SK"));
        let body: serde_json::Value = serde_json::from_str(&q.body(3)).unwrap();
        assert_eq!(body["station_profile_id"], 3);
        assert_eq!(body["freq"], 14_250_000);
        assert!(
            body.get("comment").is_none() && body.get("freq_rx").is_none(),
            "{body}"
        );
        assert!(
            body.as_object()
                .unwrap()
                .values()
                .all(|v| !v.is_object() && !v.is_array()),
            "scalars only"
        );

        let with = |f: fn(&mut Draft)| {
            let mut d = draft();
            f(&mut d);
            Qso::from_draft(&d)
        };
        assert_eq!(with(|d| d.call = "  ".into()), Err(FormError::NoCall));
        assert_eq!(
            with(|d| d.call = "DL1 ABC".into()),
            Err(FormError::BadCall("DL1 ABC".into()))
        );
        assert_eq!(
            with(|d| d.call = "HELLO".into()),
            Err(FormError::BadCall("HELLO".into()))
        );
        assert_eq!(
            with(|d| d.tx_hz = 12_000_000),
            Err(FormError::NoBand(12_000_000))
        );
        assert_eq!(
            with(|d| d.time_off = d.time_on - 1),
            Err(FormError::TimeOrder)
        );
        assert!(with(|d| d.call = "dl1abc/p".into()).is_ok());
        // Past midnight the end time is left out rather than sent on the wrong date.
        let q = with(|d| {
            d.time_on = 1_798_761_590;
            d.time_off = 1_798_761_610;
        })
        .unwrap();
        assert_eq!(q.time_off, None);
    }
}
