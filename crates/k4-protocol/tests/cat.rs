//! CAT encoding tests. trace: FR-VFO-01, FR-VFO-04, FR-VFO-06, FR-MODE-01,
//! FR-MODE-02, FR-RX-01, FR-RX-02
use k4_protocol::cat::{
    atu_toggle, band_down, band_stack_next, band_up, clear_rit_xit, click_anchor, filter_normalize,
    menu_open, menu_query, menu_query_def, menu_set, passband_edges, rf_passband_hz, rx_eq_flat,
    send_text, set_af_gain, set_agc, set_antivox, set_apf, set_attenuator, set_atu_mode,
    set_auto_notch, set_band, set_band_sub, set_bandwidth_hz, set_compression, set_cw_pitch,
    set_diversity, set_dvr, set_filter_preset, set_keyer, set_keyer_speed, set_line_in,
    set_line_out, set_manual_notch, set_mic_gain, set_mic_input, set_mic_setup, set_mode,
    set_mode_sub, set_monitor, set_nb, set_nb_level, set_nr, set_pan_average, set_pan_mode,
    set_pan_nb, set_pan_nb_level, set_pan_peak, set_pan_ref, set_pan_scale, set_pan_span_hz,
    set_passband_edges_hz, set_pl_tone, set_power, set_preamp, set_qsk_delay, set_repeater,
    set_rf_gain, set_rit, set_rit_offset, set_rx_antenna, set_rx_antenna_sub, set_rx_eq,
    set_shift_hz, set_split, set_spot, set_squelch, set_sub_rx, set_text_decode,
    set_transverter_band, set_tx_antenna, set_tx_eq, set_tx_power, set_tx_power_range,
    set_vfo_a_hz, set_vfo_b_hz, set_vox, set_vox_gain, set_waterfall_height, set_waterfall_palette,
    set_xit, switch, tune, vfo_copy_swap, vfo_for_click, AtuMode, TuneAction,
};
use k4_protocol::Mode;

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

/// trace: FR-RX-SQL-01
#[test]
fn fr_rx_sql_01_squelch() {
    assert_eq!(set_squelch(0), "SQ000;"); // open
    assert_eq!(set_squelch(22), "SQ022;"); // typical
    assert_eq!(set_squelch(99), "SQ040;"); // clamped to 40
}

/// trace: FR-TX-02
#[test]
fn fr_tx_02_power() {
    assert_eq!(set_tx_power(5), "PC005H;");
    assert_eq!(set_tx_power(100), "PC100H;");
    assert_eq!(set_tx_power(255), "PC110H;"); // clamped to 110
    assert_eq!(set_tx_power_range(50, 'H'), "PC050H;"); // 50 W QRO
    assert_eq!(set_tx_power_range(50, 'L'), "PC050L;"); // 5.0 W QRP (0.1 W units)
    assert_eq!(set_tx_power_range(150, 'L'), "PC100L;"); // clamped to 100 in L/X
    assert_eq!(set_tx_power_range(150, 'H'), "PC110H;"); // clamped to 110 in H
}

/// trace: FR-VOX-02
#[test]
fn fr_vox_02_gain_and_antivox() {
    assert_eq!(set_vox_gain('V', 20), "VGV020;");
    assert_eq!(set_vox_gain('D', 99), "VGD060;"); // clamped to 60
    assert_eq!(set_antivox(15), "VI015;");
    assert_eq!(set_antivox(99), "VI060;"); // clamped
}

/// trace: FR-TX-CMP-01
#[test]
fn fr_tx_cmp_01_compression() {
    assert_eq!(set_compression(0), "CP000;");
    assert_eq!(set_compression(15), "CP015;");
    assert_eq!(set_compression(99), "CP030;"); // clamped to 30
}

/// trace: FR-TX-DLY-01
#[test]
fn fr_tx_dly_01_qsk_delay() {
    assert_eq!(set_qsk_delay(false, 'C', 25), "SD0C025;");
    assert_eq!(set_qsk_delay(true, 'V', 0), "SD1V000;");
    assert_eq!(set_qsk_delay(false, 'D', 200), "SD0D200;");
}

/// trace: FR-KEY-02
#[test]
fn fr_key_02_cw_pitch() {
    assert_eq!(set_cw_pitch(600), "CW60;");
    assert_eq!(set_cw_pitch(100), "CW25;"); // clamped low (250 Hz)
    assert_eq!(set_cw_pitch(9999), "CW95;"); // clamped high (950 Hz)
}

/// trace: FR-MODE-03, FR-FIL-01
#[test]
fn fr_filter_presets_and_shift() {
    assert_eq!(set_filter_preset(2), "FP2;");
    assert_eq!(set_filter_preset(9), "FP3;"); // clamped to 1–3
    assert_eq!(filter_normalize(), "FP~;");
    assert_eq!(set_shift_hz(1500), "IS0150;"); // ×10 encoding
    assert_eq!(set_shift_hz(600), "IS0060;");
}

/// trace: FR-FIL-02
#[test]
fn fr_fil_02_passband_edges() {
    // Edges → (BW, IS) pair.
    assert_eq!(
        set_passband_edges_hz(300, 2700),
        ("BW0240;".into(), "IS0150;".into())
    );
    assert_eq!(set_passband_edges_hz(250, 2700).1, "IS0148;"); // midpoint 1475→1480
                                                               // Swapped args normalise; equal edges clamp to the 50 Hz minimum width.
    assert_eq!(set_passband_edges_hz(2700, 300).0, "BW0240;");
    assert_eq!(set_passband_edges_hz(1000, 1000).0, "BW0005;");
    // BW + center → edges, round-trip.
    assert_eq!(passband_edges(2400, 1500), (300, 2700));
}

/// trace: FR-RX-06, FR-DIV-01
#[test]
fn fr_sub_rx_and_diversity() {
    assert_eq!(set_sub_rx(true), "SB1;");
    assert_eq!(set_sub_rx(false), "SB0;");
    assert_eq!(set_diversity(true), "DV1;");
    assert_eq!(set_diversity(false), "DV0;");
}

/// trace: FR-RX-NOTCH-01, FR-RX-APF-01
#[test]
fn fr_notch_and_apf() {
    assert_eq!(set_manual_notch(true, 1000), "NM10001;");
    assert_eq!(set_manual_notch(false, 100), "NM01500;"); // pitch clamped to 150
    assert_eq!(set_manual_notch(true, 9999), "NM50001;"); // clamped to 5000
    assert_eq!(set_auto_notch(true), "NA1;");
    assert_eq!(set_auto_notch(false), "NA0;");
    assert_eq!(set_apf(true, 0), "AP10;");
    assert_eq!(set_apf(true, 2), "AP12;");
    assert_eq!(set_apf(false, 9), "AP02;"); // width clamped to 2
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
    assert_eq!(set_nb_level(8, true, 1), "NB0811;"); // level 8, on, narrow
    assert_eq!(set_nb_level(99, true, 9), "NB1512;"); // clamped (15, wide)
    assert_eq!(set_nr(2, 1), "NR021;");
    assert_eq!(set_preamp(1, true), "PA11;");
    assert_eq!(set_preamp(0, false), "PA00;");
}

/// trace: FR-TX-MON-01, FR-CW-SPOT-01
#[test]
fn fr_tx_mon_and_spot() {
    assert_eq!(set_monitor(0, 20), "ML0020;"); // CW monitor
    assert_eq!(set_monitor(2, 100), "ML2100;"); // voice, max
    assert_eq!(set_monitor(9, 200), "ML2100;"); // clamped
    assert_eq!(set_spot(3), "SP3;"); // autospot
    assert_eq!(set_spot(9), "SP3;"); // clamped
}

/// trace: FR-VFO-05
#[test]
fn fr_vfo_05_rit_xit() {
    assert_eq!(set_rit(true), "RT1;");
    assert_eq!(set_xit(false), "XT0;");
    assert_eq!(clear_rit_xit(), "RC;");
    assert_eq!(set_rit_offset(0), "RO+0000;");
    assert_eq!(set_rit_offset(250), "RO+0250;");
    assert_eq!(set_rit_offset(-1500), "RO-1500;");
    assert_eq!(set_rit_offset(30000), "RO+9999;"); // clamped
}

// --- Phase-0 configuration-screen commands (FR-UI-19) ----------------------

/// trace: FR-EQ-01
#[test]
fn fr_eq_01_rx_tx_graphic_equalizer() {
    // 8 signed 3-char band fields; positive/negative/zero all width-3.
    assert_eq!(
        set_rx_eq([0, 2, 5, 1, 0, -1, -2, -4]),
        "RE+00+02+05+01+00-01-02-04;"
    );
    // Clamp to ±16.
    assert_eq!(
        set_tx_eq([99, -99, 0, 0, 0, 0, 0, 0]),
        "TE+16-16+00+00+00+00+00+00;"
    );
    assert_eq!(rx_eq_flat(), "REF;");
}

/// trace: FR-KEY-01
#[test]
fn fr_key_01_keyer_config() {
    assert_eq!(set_keyer(false, false, 110), "KPAN110;");
    assert_eq!(set_keyer(true, true, 90), "KPBR090;");
    assert_eq!(set_keyer(false, false, 999), "KPAN125;"); // weight clamped
    assert_eq!(set_keyer_speed(20), "KS020;");
    assert_eq!(set_keyer_speed(200), "KS100;"); // clamped
}

/// trace: FR-AUD-CFG-01
#[test]
fn fr_aud_cfg_01_mic_and_line() {
    assert_eq!(set_mic_input(2), "MI2;");
    assert_eq!(set_mic_gain(15), "MG015;");
    assert_eq!(set_mic_gain(200), "MG080;"); // clamped
    assert_eq!(set_mic_setup(2, true, false, 1, true), "MS21011;");
    assert_eq!(set_line_in(20, 5, true), "LI0200051;");
    assert_eq!(set_line_out(10, 12, false), "LO0100120;");
}

/// trace: FR-VFO-04
#[test]
fn fr_vfo_04_direct_band_and_stack() {
    assert_eq!(set_band(0), "BN00;"); // 160 m
    assert_eq!(set_band(10), "BN10;"); // 6 m
    assert_eq!(set_band_sub(5), "BN$05;");
    assert_eq!(band_stack_next(), "BN^;");
    assert_eq!(set_transverter_band(3), "XV03;");
    assert_eq!(set_transverter_band(12), "XV12;");
}

/// trace: FR-VOX-01, FR-TX-MSG-01
#[test]
fn fr_vox_and_text_message() {
    assert_eq!(set_vox('V', true), "VXV1;");
    assert_eq!(set_vox('C', false), "VXC0;");
    assert_eq!(send_text("CQ CQ"), "KY CQ CQ;");
    // Truncated to 60 chars.
    let long = "X".repeat(80);
    assert_eq!(send_text(&long).len(), "KY ".len() + 60 + 1);
}

/// The waterfall palette (`#WFC`) and height (`#WFH`) round-trip (FR-PAN-CTL-02).
/// trace: FR-PAN-CTL-01, FR-PAN-CTL-02
#[test]
fn fr_pan_ctl_01_display_family() {
    assert_eq!(set_pan_mode(2), "#DPM2;"); // dual
    assert_eq!(set_pan_span_hz(50_000), "#SPN50000;");
    assert_eq!(set_pan_span_hz(1), "#SPN6000;"); // clamped low
    assert_eq!(set_pan_ref(-130), "#REF-130;");
    assert_eq!(set_pan_scale(70), "#SCL70;");
    assert_eq!(set_pan_average(10), "#AVG10;");
    assert_eq!(set_pan_peak(true), "#PKM1;");
    assert_eq!(set_waterfall_palette(1), "#WFC1;");
    assert_eq!(set_waterfall_height(100), "#WFH100;");
    assert_eq!(set_pan_nb(2), "#NB2;");
    assert_eq!(set_pan_nb_level(14), "#NBL14;");
}

/// A panadapter click anchors the passband per mode: USB/DATA place its low
/// edge on the click, LSB/DATA-REV its high edge, CW/CW-REV/AM/FM its centre.
/// trace: FR-PAN-05
#[test]
fn fr_pan_05_click_anchor_per_mode() {
    use k4_protocol::cat::ClickAnchor::*;
    assert_eq!(click_anchor(Mode::Usb), LowEdge);
    assert_eq!(click_anchor(Mode::Data), LowEdge);
    assert_eq!(click_anchor(Mode::Lsb), HighEdge);
    assert_eq!(click_anchor(Mode::DataRev), HighEdge);
    for m in [Mode::Cw, Mode::CwRev, Mode::Am, Mode::Fm] {
        assert_eq!(click_anchor(m), Center);
    }
}

/// The RF passband sits above the VFO on USB, below it on LSB, straddles it on
/// CW (a signal at the VFO sounds at the sidetone pitch), and is symmetric
/// about the carrier on AM/FM.
/// trace: FR-PAN-05
#[test]
fn fr_pan_05_rf_passband_follows_sideband_sense() {
    // BW 2.7 kHz centred at 1.5 kHz audio ⇒ audio passband 150…2850 Hz.
    let (bw, is, pitch) = (2_700, 1_500, 600);
    let vfo = 14_200_000;

    assert_eq!(
        rf_passband_hz(vfo, Mode::Usb, bw, is, pitch),
        (14_200_150, 14_202_850)
    );
    assert_eq!(
        rf_passband_hz(vfo, Mode::Lsb, bw, is, pitch),
        (14_197_150, 14_199_850)
    );
    // CW: audio shifted down by the 600 Hz pitch ⇒ −450…+2250 about the VFO.
    assert_eq!(
        rf_passband_hz(vfo, Mode::Cw, bw, is, pitch),
        (14_199_550, 14_202_250)
    );
    // CW-REV mirrors CW about the VFO.
    assert_eq!(
        rf_passband_hz(vfo, Mode::CwRev, bw, is, pitch),
        (14_197_750, 14_200_450)
    );
    // AM/FM are carrier-centred and ignore the IF centre pitch.
    assert_eq!(
        rf_passband_hz(vfo, Mode::Am, bw, is, pitch),
        (14_198_650, 14_201_350)
    );
    assert_eq!(
        rf_passband_hz(vfo, Mode::Fm, bw, is, pitch),
        rf_passband_hz(vfo, Mode::Am, bw, 9_999, pitch)
    );
}

/// `vfo_for_click` is the inverse of `rf_passband_hz`: after tuning, the edge
/// (or centre) the mode anchors lands exactly on the clicked frequency, so the
/// shaded overlay sits under the cursor.
/// trace: FR-PAN-05
#[test]
fn fr_pan_05_click_round_trips_to_the_anchored_edge() {
    let (bw, is, pitch) = (2_700, 1_500, 600);
    let clicked = 14_200_000;

    for mode in [
        Mode::Usb,
        Mode::Lsb,
        Mode::Cw,
        Mode::CwRev,
        Mode::Am,
        Mode::Fm,
        Mode::Data,
        Mode::DataRev,
    ] {
        let vfo = vfo_for_click(clicked, mode, bw, is, pitch);
        let (lo, hi) = rf_passband_hz(vfo, mode, bw, is, pitch);
        match click_anchor(mode) {
            k4_protocol::cat::ClickAnchor::LowEdge => assert_eq!(lo, clicked, "{mode:?}"),
            k4_protocol::cat::ClickAnchor::HighEdge => assert_eq!(hi, clicked, "{mode:?}"),
            k4_protocol::cat::ClickAnchor::Center => {
                assert_eq!((lo + hi) / 2, clicked, "{mode:?}")
            }
        }
        assert!(hi > lo, "{mode:?}: passband must have positive width");
    }
}

/// Clicking near 0 Hz saturates instead of wrapping the unsigned VFO.
/// trace: FR-PAN-05
#[test]
fn fr_pan_05_click_below_zero_saturates() {
    // USB anchors the low edge 150 Hz above the VFO, so a click at 100 Hz
    // would imply a negative VFO.
    assert_eq!(vfo_for_click(100, Mode::Usb, 2_700, 1_500, 600), 0);
    // LSB puts the VFO *above* the click, so the same click needs no clamping.
    assert_eq!(vfo_for_click(100, Mode::Lsb, 2_700, 1_500, 600), 250);
    // A passband edge reaching below 0 Hz saturates rather than wrapping.
    assert_eq!(rf_passband_hz(10, Mode::Lsb, 2_700, 1_500, 600).0, 0);
}

/// trace: FR-VFO-07
#[test]
fn fr_vfo_07_copy_swap() {
    assert_eq!(vfo_copy_swap(0), "AB0;"); // A → B freq
    assert_eq!(vfo_copy_swap(2), "AB2;"); // swap freq
    assert_eq!(vfo_copy_swap(9), "AB5;"); // clamped
}

/// trace: FR-ANT-01
#[test]
fn fr_ant_01_antenna_select() {
    assert_eq!(set_tx_antenna(2), "AN2;");
    assert_eq!(set_tx_antenna(9), "AN3;"); // clamped
    assert_eq!(set_rx_antenna(4), "AR4;");
    assert_eq!(set_rx_antenna_sub(1), "AR$1;");
}

/// trace: FR-MENU-01
#[test]
fn fr_menu_01_menu_addressed_access() {
    assert_eq!(menu_open(101), "MO0101;");
    assert_eq!(menu_query_def(73), "MEDF0073;");
    assert_eq!(menu_set(101, "0005"), "ME0101.0005;");
    assert_eq!(menu_query(30), "ME0030;"); // FR-CFG-07 value GET
}

/// trace: FR-SW-01
#[test]
fn fr_sw_01_switch_emulation() {
    assert_eq!(switch(17), "SW17;"); // tap M1
    assert_eq!(switch(162), "SW162;"); // hold M1 (store)
    assert_eq!(switch(153), "SW153;"); // PF1
}

/// trace: FR-TXT-01
#[test]
fn fr_txt_01_text_decode() {
    assert_eq!(set_text_decode(2, 0, 3), "TD203;"); // CW RX, auto threshold, 3 lines
    assert_eq!(set_text_decode(0, 0, 0), "TD000;"); // off
}

/// trace: FR-PWR-01
#[test]
fn fr_pwr_01_power_control() {
    assert_eq!(set_power(0), "PS0;"); // off
    assert_eq!(set_power(8), "PS8;"); // restart
    assert_eq!(set_power(88), "PS88;"); // auto-update + restart
}

/// trace: FR-FM-01
#[test]
fn fr_fm_01_repeater_and_tone() {
    assert_eq!(set_repeater('+', 600), "RP+00600;");
    assert_eq!(set_repeater('S', 0), "RPS00000;");
    assert_eq!(set_pl_tone(13, true), "PL131;"); // 100.0 Hz on
    assert_eq!(set_pl_tone(99, false), "PL500;"); // clamped to 50
}

/// trace: FR-DVR-01
#[test]
fn fr_dvr_01_playback() {
    assert_eq!(set_dvr(1), "PB1;");
    assert_eq!(set_dvr(8), "PB8;");
    assert_eq!(set_dvr(0), "PB0;"); // cancel
    assert_eq!(set_dvr(99), "PB8;"); // clamped
}

/// The VFO tuning-step (VT) setter encodes the rate index + mode, per receiver.
///
/// trace: FR-VFO-03
#[test]
fn fr_vfo_03_set_tune_step() {
    // index 2 = 100 Hz, mode 3 (CW), main VFO.
    assert_eq!(k4_protocol::cat::set_tune_step(false, 2, 3), "VT23;");
    // sub VFO, index 1 (10 Hz), mode 1 (LSB).
    assert_eq!(k4_protocol::cat::set_tune_step(true, 1, 1), "VT$11;");
    // index clamps to 5 (100 kHz max).
    assert_eq!(k4_protocol::cat::set_tune_step(false, 9, 2), "VT52;");
}

/// The per-mode tune-step ALT GET encodes the receiver + mode (`VT[$]Xm`).
///
/// trace: FR-VFO-03
#[test]
fn fr_vfo_03_query_tune_step_per_mode() {
    assert_eq!(k4_protocol::cat::query_tune_step(false, 3), "VTX3;");
    assert_eq!(k4_protocol::cat::query_tune_step(true, 2), "VT$X2;");
}

/// ATU mode (`AT`) and the in/bypass toggle. `AT0` (NOT INSTALLED) is not
/// representable: D12 says it "should not be sent under normal circumstances".
/// trace: FR-ATU-01
#[test]
fn fr_atu_01_atu_mode_and_toggle() {
    assert_eq!(set_atu_mode(AtuMode::Bypass), "AT1;");
    assert_eq!(set_atu_mode(AtuMode::Auto), "AT2;");
    assert_eq!(atu_toggle(), "AT/;");
}

/// Tune actions (`TU`), and which of them key the transmitter.
/// trace: FR-TX-TUNE-01
#[test]
fn fr_tx_tune_01_tune_actions() {
    assert_eq!(tune(TuneAction::Exit), "TU0;");
    assert_eq!(tune(TuneAction::Tune), "TU1;");
    assert_eq!(tune(TuneAction::TuneLp), "TU2;");
    assert_eq!(tune(TuneAction::AtuTune), "TU3;");
    assert_eq!(tune(TuneAction::AtuExtended), "TU4;");

    // Everything but Exit puts the radio on air — this is what the session
    // gates on, so it must not drift.
    assert!(!TuneAction::Exit.transmits());
    for a in [
        TuneAction::Tune,
        TuneAction::TuneLp,
        TuneAction::AtuTune,
        TuneAction::AtuExtended,
    ] {
        assert!(a.transmits(), "{a:?} keys the transmitter");
    }
}
