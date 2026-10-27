//! Held entries saved as ADIF (FR-LOG-04): no QSO is lost to a configuration change — an entry
//! for a server or location that can no longer take it is saved to a file first, then discarded.
//!
//! ADIF differs from the API's JSON in two places: `QSO_DATE` is `YYYYMMDD` and `FREQ`/`FREQ_RX`
//! are in **MHz**. A field's length is its byte length (UTF-8 names stay intact).

use crate::queue::Entry;

fn field(out: &mut String, name: &str, value: &str) {
    if !value.is_empty() {
        out.push_str(&format!("<{name}:{}>{value} ", value.len()));
    }
}

fn mhz(hz: u64) -> String {
    format!("{}.{:06}", hz / 1_000_000, hz % 1_000_000)
}

/// The entries as an ADIF file (header plus one record each).
pub fn to_adif(entries: &[Entry], program_version: &str) -> String {
    let mut out = String::from("K4 Remote: QSOs held in the Wavelog queue\n");
    field(&mut out, "ADIF_VER", "3.1.4");
    field(&mut out, "PROGRAMID", "K4Remote");
    field(&mut out, "PROGRAMVERSION", program_version);
    out.push_str("<EOH>\n");
    for e in entries {
        let q = &e.qso;
        field(&mut out, "CALL", &q.call);
        field(&mut out, "QSO_DATE", &q.qso_date.replace('-', ""));
        field(&mut out, "TIME_ON", &q.time_on);
        if let Some(t) = &q.time_off {
            field(&mut out, "TIME_OFF", t);
        }
        field(&mut out, "BAND", &q.band);
        field(&mut out, "FREQ", &mhz(q.freq));
        if let Some(rx) = q.freq_rx {
            field(&mut out, "FREQ_RX", &mhz(rx));
        }
        field(&mut out, "MODE", &q.mode);
        if let Some(s) = &q.submode {
            field(&mut out, "SUBMODE", s);
        }
        field(&mut out, "RST_SENT", &q.rst_sent);
        field(&mut out, "RST_RCVD", &q.rst_rcvd);
        field(&mut out, "NAME", &q.name);
        field(&mut out, "COMMENT", &q.comment);
        if let Some(p) = q.tx_pwr {
            field(&mut out, "TX_PWR", &format!("{p}"));
        }
        if let Some(o) = &q.operator {
            field(&mut out, "OPERATOR", o);
        }
        out.push_str("<EOR>\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qso::Qso;
    use crate::queue::{State, VERSION};
    use crate::reply::LastAttempt;

    /// FR-LOG-04: a held entry becomes an ADIF record — date without dashes, frequencies in MHz,
    /// lengths in bytes, optional fields only when set.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_held_entries_save_as_adif() {
        let q = Qso {
            call: "DL1ABC".into(),
            band: "20m".into(),
            mode: "SSB".into(),
            submode: Some("USB".into()),
            freq: 14_250_000,
            freq_rx: Some(14_200_500),
            qso_date: "2026-10-27".into(),
            time_on: "075000".into(),
            time_off: None,
            rst_sent: "59".into(),
            rst_rcvd: "57".into(),
            name: "Jürgen".into(),
            comment: String::new(),
            tx_pwr: Some(5.0),
            operator: Some("DC0SK".into()),
        };
        let e = Entry {
            v: VERSION,
            id: "x".into(),
            queued_at: 1,
            url: "u".into(),
            station_profile_id: 1,
            qso: q,
            state: State::Held,
            reason: Some("r".into()),
            attempts: 1,
            last_attempt: LastAttempt::NotSent,
        };
        let adif = to_adif(&[e], "0.11.0");
        assert!(adif.contains("<PROGRAMVERSION:6>0.11.0 <EOH>"), "{adif}");
        for f in [
            "<CALL:6>DL1ABC ",
            "<QSO_DATE:8>20261027 ",
            "<TIME_ON:6>075000 ",
            "<BAND:3>20m ",
            "<FREQ:9>14.250000 ",
            "<FREQ_RX:9>14.200500 ",
            "<MODE:3>SSB ",
            "<SUBMODE:3>USB ",
            "<NAME:7>Jürgen ",
            "<TX_PWR:1>5 ",
            "<OPERATOR:5>DC0SK ",
            "<EOR>",
        ] {
            assert!(adif.contains(f), "missing {f:?} in {adif}");
        }
        assert!(
            !adif.contains("TIME_OFF") && !adif.contains("COMMENT"),
            "{adif}"
        );
        assert_eq!(mhz(7_000), "0.007000");
    }
}
