//! Radio-state tests. trace: FR-CAT-05, FR-CAT-06, FR-CAT-07, FR-CAT-AI,
//! FR-MTR-01, FR-MTR-02, FR-MTR-04
use k4_protocol::state::{connect_state_seed, s_unit_label, Mode, RadioState};

/// A sequence of GET responses leaves the state coherent and `apply_cat`
/// reports change vs. no-change.
///
/// trace: FR-CAT-06
#[test]
fn fr_cat_06_state_updates_coherently_from_responses() {
    let mut s = RadioState::new();
    assert!(s.apply_cat("FA00014074000;"));
    assert!(s.apply_cat("MD3;"));
    assert!(s.apply_cat("FT1;"));

    assert_eq!(s.vfo_a_hz, Some(14_074_000));
    assert_eq!(s.mode_a, Some(Mode::Cw));
    assert_eq!(s.split, Some(true));

    // Re-applying the same value reports "no change".
    assert!(!s.apply_cat("FA00014074000;"));
}

/// VFO B frequency parses from an `FB` response (FR-VFO-02).
///
/// trace: FR-VFO-02
#[test]
fn fr_vfo_02_parses_vfo_b_frequency() {
    let mut s = RadioState::new();
    s.apply_cat("FB00007100000;");
    assert_eq!(s.vfo_b_hz, Some(7_100_000));
}

/// The `$` sub-RX variant targets VFO B and must not be shadowed by the base
/// command (longest-prefix-first).
///
/// trace: FR-CAT-05
#[test]
fn fr_cat_05_dollar_variant_targets_sub_rx() {
    let mut s = RadioState::new();
    s.apply_cat("MD2;"); // main RX → USB
    s.apply_cat("MD$3;"); // sub RX → CW

    assert_eq!(s.mode_a, Some(Mode::Usb));
    assert_eq!(s.mode_b, Some(Mode::Cw));
}

/// An *unsolicited* response (Auto-Info push) updates the state through the same
/// path as a GET reply — no poll required.
///
/// trace: FR-CAT-AI
#[test]
fn fr_cat_ai_unsolicited_response_updates_state() {
    let mut s = RadioState::new();
    // Simulates a pushed AI frame after the operator tunes at the radio.
    assert!(s.apply_cat("FA00007030000;"));
    assert_eq!(s.vfo_a_hz, Some(7_030_000));
}

/// The consolidated `IF;` response seeds frequency, mode, TX, and split.
///
/// trace: FR-CAT-07
#[test]
fn fr_cat_07_if_response_seeds_state() {
    // freq=14074000, RIT/XIT off, not transmitting, mode 3 (CW), split off.
    let if_resp = "IF00014074000     +000000 0003000001;";
    let mut s = RadioState::new();
    s.apply_cat(if_resp);

    assert_eq!(s.vfo_a_hz, Some(14_074_000));
    assert_eq!(s.transmitting, Some(false));
    assert_eq!(s.mode_a, Some(Mode::Cw));
    assert_eq!(s.split, Some(false));
}

/// The connect-time seed burst leads with `IF;` and includes the S-meter.
///
/// trace: FR-CAT-07
#[test]
fn fr_cat_07_seed_burst_starts_with_if() {
    let seed = connect_state_seed();
    assert_eq!(seed.first(), Some(&"IF;"));
    assert!(seed.contains(&"FA;"));
    assert!(seed.contains(&"SMH;"));
}

/// S-meter bar responses parse for main and sub RX (`SM` / `SM$`), and `SMH`
/// (high-res dBm) is not shadowed by the `SM` prefix.
///
/// trace: FR-MTR-01, FR-MTR-02
#[test]
fn fr_mtr_01_s_meter_responses_parse() {
    let mut s = RadioState::new();
    s.apply_cat("SM08;");
    s.apply_cat("SM$15;");
    s.apply_cat("SMH-073;");
    s.apply_cat("SMH$+009;");

    assert_eq!(s.s_meter_bars, Some(8));
    assert_eq!(s.s_meter_bars_sub, Some(15));
    assert_eq!(s.s_meter_dbm, Some(-73));
    assert_eq!(s.s_meter_dbm_sub, Some(9));
}

/// Bandwidth, gains, and attenuator responses parse into state.
///
/// trace: FR-MODE-02, FR-RX-01, FR-RX-02
#[test]
fn fr_rx_control_responses_parse() {
    let mut s = RadioState::new();
    s.apply_cat("BW0270;"); // ×10 Hz → 2700 Hz
    s.apply_cat("AG030;");
    s.apply_cat("RG-20;");
    s.apply_cat("RA121;"); // 12 dB, on

    assert_eq!(s.bandwidth_hz, Some(2700));
    assert_eq!(s.af_gain, Some(30));
    assert_eq!(s.rf_gain_db, Some(20));
    assert_eq!(s.atten_db, Some(12));
    assert_eq!(s.atten_on, Some(true));

    s.apply_cat("RA00;"); // 0 dB, off
    assert_eq!(s.atten_db, Some(0));
    assert_eq!(s.atten_on, Some(false));
}

/// AGC, NB, NR, preamp, RIT, XIT responses parse into state.
///
/// trace: FR-RX-03, FR-RX-04, FR-VFO-05
#[test]
fn fr_rx_dsp_responses_parse() {
    let mut s = RadioState::new();
    s.apply_cat("GT2;"); // fast AGC
    s.apply_cat("NB0510;"); // level 5, on, filter 0
    s.apply_cat("NR031;"); // level 3, on
    s.apply_cat("PA11;"); // preamp level 1, on
    s.apply_cat("RT1;");
    s.apply_cat("XT0;");

    assert_eq!(s.agc_mode, Some(2));
    assert_eq!(s.nb_on, Some(true));
    assert_eq!(s.nb_level, Some(5));
    assert_eq!(s.nr_on, Some(true));
    assert_eq!(s.nr_level, Some(3));
    assert_eq!(s.preamp_on, Some(true));
    assert_eq!(s.rit_on, Some(true));
    assert_eq!(s.xit_on, Some(false));
}

/// dBm → S-unit mapping: S9 = −73 dBm, 6 dB/unit, excess shown in dB.
///
/// trace: FR-MTR-04
#[test]
fn fr_mtr_04_s_unit_label_mapping() {
    assert_eq!(s_unit_label(-73), "S9");
    assert_eq!(s_unit_label(-67), "S9+6dB"); // 6 dB over S9
    assert_eq!(s_unit_label(-79), "S8");
    assert_eq!(s_unit_label(-121), "S1");
    assert_eq!(s_unit_label(-130), "S0");
}
