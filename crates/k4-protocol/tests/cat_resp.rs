//! CAT server replies (FR-CATSRV-03): each formatter produces the K4 RESP wire form from a
//! `RadioState`, pinned byte for byte against the Programmer's Reference (D12), and round-trips
//! through the crate's own parser (`RadioState::apply_cat`) to the same fields.
//! trace: FR-CATSRV-03

use k4_protocol::cat_resp as resp;
use k4_protocol::state::{Mode, RadioState};

fn seeded() -> RadioState {
    RadioState {
        vfo_a_hz: Some(14_074_000),
        vfo_b_hz: Some(14_076_500),
        mode_a: Some(Mode::Data),
        mode_b: Some(Mode::Usb),
        split: Some(true),
        scanning: Some(false),
        transmitting: Some(false),
        bandwidth_hz: Some(2_800),
        sub_bandwidth_hz: Some(2_400),
        data_submode: Some(0),
        sub_data_submode: Some(3),
        rit_offset: Some(-150),
        rit_on: Some(true),
        xit_on: Some(false),
        ..RadioState::default()
    }
}

/// Every formatter's exact bytes (PRG D12: FA/FB 11 digits, MD digit, BW ×10 Hz 4 digits, DT
/// digit, FT/FR/TQ one digit), and a round trip through `apply_cat` restores the same fields.
/// trace: FR-CATSRV-03
#[test]
fn fr_catsrv_03_simple_replies_are_exact_and_round_trip() {
    let s = seeded();
    let cases = [
        (resp::fa(&s), "FA00014074000;"),
        (resp::fb(&s), "FB00014076500;"),
        (resp::md(&s, false), "MD6;"),
        (resp::md(&s, true), "MD$2;"),
        (resp::bw(&s, false), "BW0280;"),
        (resp::bw(&s, true), "BW$0240;"),
        (resp::dt(&s, false), "DT0;"),
        (resp::dt(&s, true), "DT$3;"),
        (resp::ft(&s), "FT1;"),
        (resp::tq(&s), "TQ0;"),
    ];
    for (got, want) in &cases {
        assert_eq!(got.as_deref(), Some(*want));
    }
    // FR is receive-VFO selection, "equivalent to FT0" (PRG): the K4 receives on VFO A.
    assert_eq!(resp::fr(), "FR0;");

    let mut back = RadioState::default();
    for (got, _) in &cases {
        back.apply_cat(got.as_deref().unwrap().trim_end_matches(';'));
    }
    assert_eq!(back.vfo_a_hz, s.vfo_a_hz);
    assert_eq!(back.vfo_b_hz, s.vfo_b_hz);
    assert_eq!((back.mode_a, back.mode_b), (s.mode_a, s.mode_b));
    assert_eq!(
        (back.bandwidth_hz, back.sub_bandwidth_hz),
        (s.bandwidth_hz, s.sub_bandwidth_hz)
    );
    assert_eq!(
        (back.data_submode, back.sub_data_submode),
        (s.data_submode, s.sub_data_submode)
    );
    assert_eq!(back.split, s.split);
    // (`TQ` is a GET-only query the radio never pushes, so the parser has no `TQ` arm; its bytes
    // are pinned above and the TX flag round-trips through `IF` below.)
}

/// A field the radio has not reported yet has no reply (never a made-up value); the caller
/// decides what to do. Every mode digit the K4 defines formats back to itself.
/// trace: FR-CATSRV-03
#[test]
fn fr_catsrv_03_unknown_state_has_no_reply_and_modes_round_trip() {
    let empty = RadioState::default();
    assert_eq!(resp::fa(&empty), None);
    assert_eq!(resp::md(&empty, false), None);
    assert_eq!(resp::bw(&empty, false), None);
    assert_eq!(resp::ft(&empty), None);
    assert_eq!(resp::tq(&empty), None);
    assert_eq!(resp::if_(&empty, false), None);
    for d in *b"12345679" {
        let m = Mode::from_md_digit(d).unwrap();
        assert_eq!(m.md_digit(), d, "mode {m:?}");
    }
    // Bandwidth is sent in 10 Hz units, clamped to the 4-digit field.
    let wide = RadioState {
        bandwidth_hz: Some(123_456),
        ..RadioState::default()
    };
    assert_eq!(resp::bw(&wide, false).as_deref(), Some("BW9999;"));
}

/// `IF` (PRG D12 p16): `IF[f]*****+yyyyrx*00tm0spbd1*;` — 37 characters before the `;` (Hamlib
/// requires exactly that), pinned byte for byte; `b` is always 0; `d` is the DATA sub-mode only
/// in K31 meta mode, else 0. It round-trips through `apply_if`.
/// trace: FR-CATSRV-03
#[test]
fn fr_catsrv_03_if_is_the_documented_fixed_layout() {
    let s = seeded();
    let basic = resp::if_(&s, false).unwrap();
    assert_eq!(basic, "IF00014074000     -015010 0006001001 ;");
    assert_eq!(basic.trim_end_matches(';').len(), 37);
    let k31 = resp::if_(
        &RadioState {
            data_submode: Some(3),
            ..s.clone()
        },
        true,
    )
    .unwrap();
    assert_eq!(k31, "IF00014074000     -015010 0006001031 ;");
    // Outside K31 the `d` byte stays 0 even when there is a DATA sub-mode to report.
    let basic3 = resp::if_(
        &RadioState {
            data_submode: Some(3),
            ..s.clone()
        },
        false,
    )
    .unwrap();
    assert_eq!(basic3, basic);

    let tx_plus = RadioState {
        transmitting: Some(true),
        split: Some(false),
        rit_offset: Some(9_999),
        rit_on: Some(false),
        xit_on: Some(true),
        mode_a: Some(Mode::Cw),
        ..s.clone()
    };
    let line = resp::if_(&tx_plus, false).unwrap();
    assert_eq!(line, "IF00014074000     +999901 0013000001 ;");

    let mut back = RadioState::default();
    back.apply_cat(line.trim_end_matches(';'));
    assert_eq!(back.vfo_a_hz, Some(14_074_000));
    assert_eq!(back.rit_offset, Some(9_999));
    assert_eq!((back.rit_on, back.xit_on), (Some(false), Some(true)));
    assert_eq!((back.transmitting, back.split), (Some(true), Some(false)));
    assert_eq!(back.mode_a, Some(Mode::Cw));
}
