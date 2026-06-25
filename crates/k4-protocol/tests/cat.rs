//! CAT encoding tests. trace: FR-VFO-01, FR-VFO-04, FR-VFO-06, FR-MODE-01,
//! FR-MODE-02, FR-RX-01, FR-RX-02
use k4_protocol::cat::{
    band_down, band_up, clear_rit_xit, set_af_gain, set_agc, set_attenuator, set_bandwidth_hz,
    set_mode, set_mode_sub, set_nb, set_nr, set_preamp, set_rf_gain, set_rit, set_split,
    set_vfo_a_hz, set_vfo_b_hz, set_xit,
};

/// trace: FR-VFO-01
#[test]
fn fr_vfo_01_set_vfo_frequency_emits_canonical_11_digit_hz() {
    assert_eq!(set_vfo_a_hz(14_074_000), "FA00014074000;");
    assert_eq!(set_vfo_a_hz(7_100_000), "FA00007100000;");
    assert_eq!(set_vfo_b_hz(7_100_000), "FB00007100000;");
}

/// trace: FR-MODE-01
#[test]
fn fr_mode_01_set_mode_main_and_sub() {
    assert_eq!(set_mode(3), "MD3;");
    assert_eq!(set_mode_sub(2), "MD$2;");
}

/// trace: FR-MODE-02
#[test]
fn fr_mode_02_set_bandwidth_uses_x10_hz() {
    assert_eq!(set_bandwidth_hz(2700), "BW0270;");
    assert_eq!(set_bandwidth_hz(500), "BW0050;");
}

/// trace: FR-RX-01, FR-RX-02
#[test]
fn fr_rx_gain_and_attenuator() {
    assert_eq!(set_af_gain(30), "AG030;");
    assert_eq!(set_af_gain(99), "AG060;"); // clamped to 60
    assert_eq!(set_rf_gain(20), "RG-20;");
    assert_eq!(set_attenuator(12, true), "RA121;");
    assert_eq!(set_attenuator(0, false), "RA00;");
}

/// trace: FR-VFO-04, FR-VFO-06
#[test]
fn fr_vfo_band_and_split() {
    assert_eq!(band_up(), "BN+;");
    assert_eq!(band_down(), "BN-;");
    assert_eq!(set_split(true), "FT1;");
    assert_eq!(set_split(false), "FT0;");
}

/// trace: FR-RX-03, FR-RX-04
#[test]
fn fr_rx_dsp_encoders() {
    assert_eq!(set_agc(2), "GT2;"); // fast AGC
    assert_eq!(set_nb(true), "NB1;");
    assert_eq!(set_nr(2, 1), "NR021;");
    assert_eq!(set_preamp(1, true), "PA11;");
    assert_eq!(set_preamp(0, false), "PA00;");
}

/// trace: FR-VFO-05
#[test]
fn fr_vfo_05_rit_xit() {
    assert_eq!(set_rit(true), "RT1;");
    assert_eq!(set_xit(false), "XT0;");
    assert_eq!(clear_rit_xit(), "RC;");
}
