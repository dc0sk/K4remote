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

/// `$` RESP variants route to the sub-receiver fields, leaving main untouched.
///
/// trace: FR-CAT-05
#[test]
fn fr_cat_05_sub_variants_route_to_sub_fields() {
    let mut s = RadioState::new();
    s.apply_cat("AG030;"); // main AF gain
    s.apply_cat("AG$045;"); // sub AF gain
    assert_eq!(s.af_gain, Some(30));
    assert_eq!(s.sub_af_gain, Some(45));
    s.apply_cat("NM$10001;"); // sub manual notch on, pitch 1000
    assert_eq!(s.sub_notch_on, Some(true));
    assert_eq!(s.sub_notch_pitch, Some(1000));
    assert_eq!(s.notch_on, None); // main notch untouched
}

/// The `IF` `s` flag (index 29) drives the scan-in-progress indicator.
///
/// trace: FR-SCAN-01
#[test]
fn fr_scan_01_if_scan_flag() {
    let mut s = RadioState::new();
    s.apply_cat("IF00014074000     +000000 0003010001;"); // s = 1
    assert_eq!(s.scanning, Some(true));
    s.apply_cat("IF00014074000     +000000 0003000001;"); // s = 0
    assert_eq!(s.scanning, Some(false));
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

/// Read-back of the configuration-screen commands (FR-UI-19 screens): the RESP
/// forms parse into `RadioState`, seeding the EQ / keyer / mic / display / antenna
/// / band / VOX screens on connect.
///
/// trace: FR-CAT-06, FR-UI-19, FR-UI-20
#[test]
fn fr_ui_19_config_screen_readback() {
    let mut s = RadioState::new();
    s.apply_cat("RE+00+02+05+01+00-01-02-04;");
    s.apply_cat("TE+16-16+00+00+00+00+00+00;");
    s.apply_cat("KPBR090;"); // iambic B, paddle reversed, weight 90
    s.apply_cat("KS022;");
    s.apply_cat("MI2;");
    s.apply_cat("MG015;");
    s.apply_cat("LO0100120;"); // left 10, right 12, gang off
    s.apply_cat("AN2;");
    s.apply_cat("AR4;");
    s.apply_cat("AR$1;");
    s.apply_cat("VXV1;");
    s.apply_cat("BN14;");
    s.apply_cat("BN$03;"); // sub band must NOT clobber the main band
    s.apply_cat("#REF-130;");
    s.apply_cat("#SPN50000;");
    s.apply_cat("#SCL70;");
    s.apply_cat("#DPM2;");
    s.apply_cat("#WFC1;");
    s.apply_cat("#WFH080;");

    assert_eq!(s.rx_eq, Some([0, 2, 5, 1, 0, -1, -2, -4]));
    assert_eq!(s.tx_eq, Some([16, -16, 0, 0, 0, 0, 0, 0]));
    assert_eq!(s.keyer_iambic_b, Some(true));
    assert_eq!(s.keyer_paddle_rev, Some(true));
    assert_eq!(s.keyer_weight, Some(90));
    assert_eq!(s.keyer_speed, Some(22));
    assert_eq!(s.mic_input, Some(2));
    assert_eq!(s.mic_gain, Some(15));
    assert_eq!(s.line_out_left, Some(10));
    assert_eq!(s.line_out_right, Some(12));
    assert_eq!(s.line_out_gang, Some(false));
    assert_eq!(s.tx_antenna, Some(2));
    assert_eq!(s.rx_antenna, Some(4));
    assert_eq!(s.rx_antenna_sub, Some(1));
    assert_eq!(s.vox_voice, Some(true));
    assert_eq!(s.band, Some(14));
    assert_eq!(s.pan_ref, Some(-130));
    assert_eq!(s.pan_span_hz, Some(50_000));
    assert_eq!(s.pan_scale, Some(70));
    assert_eq!(s.pan_mode, Some(2));
    assert_eq!(s.wf_palette, Some(1));
    assert_eq!(s.wf_height, Some(80));

    // The connect seed now requests these screens' values.
    let seed = connect_state_seed();
    for g in ["RE;", "TE;", "KP;", "#REF;", "AR$;", "BN;"] {
        assert!(seed.contains(&g), "seed missing {g}");
    }
}

/// `TB` decoded text is appended to the buffer, `;` inside the text preserved.
///
/// trace: FR-TXT-01
#[test]
fn fr_txt_01_tb_decode_buffer() {
    let mut s = RadioState::new();
    s.apply_cat("TB$005CQ CQ;"); // t=0, rr=05, s="CQ CQ"
    assert_eq!(s.decode_text, "CQ CQ");
    s.apply_cat("TB$003 DE;"); // append
    assert_eq!(s.decode_text, "CQ CQ DE");
}

/// `TM` TX-metering auto-response populates ALC / CMP / FWD power / SWR.
///
/// trace: FR-MTR-03
#[test]
fn fr_mtr_03_tx_metering() {
    let mut s = RadioState::new();
    s.apply_cat("TM005012050015;"); // ALC 5, CMP 12, FWD 50 W, SWR 1.5
    assert_eq!(s.tx_alc, Some(5));
    assert_eq!(s.tx_cmp, Some(12));
    assert_eq!(s.tx_fwd_w, Some(50));
    assert_eq!(s.tx_swr_x10, Some(15));
}

/// The `ACM` RX-antenna access mask limits which AR$ values are in rotation.
///
/// trace: FR-ANT-01
#[test]
fn fr_ant_01_rx_antenna_access_mask() {
    let mut s = RadioState::new();
    // USE SUBSET: ANT1 (→AR$5), RX1 (→AR$4), =TX ANT (→AR$2) enabled.
    s.apply_cat("ACM01001010;");
    let avail = s.rx_ant_avail.unwrap();
    assert!(avail & (1 << 5) != 0);
    assert!(avail & (1 << 4) != 0);
    assert!(avail & (1 << 2) != 0);
    assert!(avail & (1 << 6) == 0); // ANT2 not enabled
                                    // DISPLAY ALL → all of AR$1..=7.
    s.apply_cat("ACM10000000;");
    assert_eq!(s.rx_ant_avail, Some(0b1111_1110));
}

/// trace: FR-CFG-07
#[test]
fn fr_cfg_07_menu_value_capture() {
    let mut s = RadioState::new();
    s.apply_cat("ME0030.0002;");
    assert_eq!(s.menu_values.get(&30).map(String::as_str), Some("0002"));
    // MEDF definitions are not captured as menu values.
    s.apply_cat("MEDF0030;");
    assert_eq!(s.menu_values.len(), 1);
}

/// DATA sub-mode (`DT`/`DT$`) responses parse for main and sub receivers.
///
/// trace: FR-DATA-01
#[test]
fn fr_data_submode_parses() {
    let mut s = RadioState::new();
    s.apply_cat("DT2;"); // FSK D on main
    s.apply_cat("DT$1;"); // AFSK A on sub
    assert_eq!(s.data_submode, Some(2));
    assert_eq!(s.sub_data_submode, Some(1));
    // Setter round-trips the wire form.
    assert_eq!(k4_protocol::cat::set_data_submode(false, 0), "DT0;");
    assert_eq!(k4_protocol::cat::set_data_submode(true, 3), "DT$3;");
}

/// The K4 error reply `<cmd>?;` is recorded against the originating command
/// mnemonic and does not disturb prior good state (PRG Error Checking).
///
/// trace: FR-CAT-03
#[test]
fn fr_cat_03_error_reply_is_surfaced() {
    let mut s = RadioState::new();
    s.apply_cat("FA00014074000;");
    assert_eq!(s.vfo_a_hz, Some(14_074_000));
    // An out-of-range / unparsable FA elicits `FA?;`.
    assert!(
        s.apply_cat("FA?;"),
        "error reply changes state (last_error)"
    );
    assert_eq!(s.last_error.as_deref(), Some("FA"));
    assert_eq!(
        s.vfo_a_hz,
        Some(14_074_000),
        "error reply must not clobber state"
    );
}

/// An unknown/unsupported command frame is ignored without crashing or
/// desynchronising the parser — the next valid frame still parses.
///
/// trace: FR-CAT-04
#[test]
fn fr_cat_04_unknown_frame_does_not_desync() {
    let mut s = RadioState::new();
    assert!(
        !s.apply_cat("ZZ9;"),
        "unknown command must not change state"
    );
    assert_eq!(s.last_error, None, "unknown command is not an error reply");
    assert!(s.apply_cat("FA00007035000;"), "parser is not wedged");
    assert_eq!(s.vfo_a_hz, Some(7_035_000));
}

/// Fuzz: arbitrary bytes through the frame decoder and arbitrary text through
/// `apply_cat` must never panic and must leave the parser usable (a valid frame
/// still parses afterwards).
///
/// trace: NFR-REL-01
#[test]
fn nfr_rel_01_random_input_never_panics() {
    use k4_protocol::cat::decode_cat_text;
    use k4_protocol::frame::FrameDecoder;
    // Deterministic PRNG (no rand dep; reproducible) over several seeds.
    for seed in [
        0x1234_5678u64,
        0xdead_beef,
        0x0f0f_0f0f,
        0xa5a5_1234,
        7,
        42,
        999,
        0xffff,
    ] {
        let mut st = seed ^ 0x9abc_def0;
        let mut next = || {
            st = st
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (st >> 33) as u32
        };
        let mut dec = FrameDecoder::new();
        let mut radio = RadioState::new();
        for _ in 0..8_000 {
            let len = (next() % 96) as usize;
            let buf: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
            for payload in dec.push(&buf) {
                if let Some(t) = decode_cat_text(&payload) {
                    radio.apply_cat(&t);
                }
            }
            // Random printable-ASCII CAT text straight into the parser.
            let n = (next() % 16) as usize;
            let txt: String = (0..n)
                .map(|_| char::from(0x20u8 + (next() % 0x5f) as u8))
                .collect();
            radio.apply_cat(&txt);
            // Prefix-biased text: force valid mnemonics with junk args.
            for m in [
                "FA", "MD", "NB$", "AP", "PA", "IF", "SM", "TM", "RP", "VT", "DT",
            ] {
                let k = (next() % 10) as usize;
                let junk: String = (0..k)
                    .map(|_| char::from(0x20u8 + (next() % 0x5f) as u8))
                    .collect();
                radio.apply_cat(&format!("{m}{junk};"));
            }
        }
    }
    let mut radio = RadioState::new();
    // Reaching here = no panic (NFR-REL-01); the parser is still usable:
    assert!(radio.apply_cat("FA00014074000;"));
    assert_eq!(radio.vfo_a_hz, Some(14_074_000));
}

/// The state model absorbs a sustained AI-update burst without unbounded growth:
/// the only growable field (`menu_values`, from `ME`) stays bounded by its key
/// space no matter how many updates arrive.
///
/// trace: NFR-PERF-AI
#[test]
fn nfr_perf_ai_burst_bounded_memory() {
    let mut s = RadioState::new();
    for i in 0..50_000u32 {
        s.apply_cat(&format!("FA{:011};", 14_000_000 + i % 1000));
        s.apply_cat(&format!("SM{:02};", i % 30));
        s.apply_cat(&format!("ME{}.{};", i % 50, i % 10)); // menu ids 0..50 only
    }
    assert!(
        !s.menu_values.is_empty(),
        "ME burst should populate menu_values"
    );
    assert!(
        s.menu_values.len() <= 50,
        "AI burst must not grow state unboundedly (got {})",
        s.menu_values.len()
    );
}

/// The panadapter display commands are named `#REF$` / `#SPN$` / `#WFC$` in
/// D12 — there the `$` is part of the *mnemonic* (LCD, against `#HREF` /
/// `#HWFC` for the external monitor), not the sub-receiver modifier. The
/// `$` spelling used to fail the integer parse and be dropped in silence, so
/// the read-back that span/reference/scale sync from never arrived.
/// trace: FR-PAN-07
#[test]
fn fr_pan_07_display_readback_accepts_the_dollar_spelling() {
    for (with, without) in [
        ("#REF$-130;", "#REF-130;"),
        ("#SPN$50000;", "#SPN50000;"),
        ("#SCL$70;", "#SCL70;"),
        ("#DPM$2;", "#DPM2;"),
        ("#WFC$1;", "#WFC1;"),
        ("#WFH$080;", "#WFH080;"),
    ] {
        let (mut a, mut b) = (RadioState::new(), RadioState::new());
        a.apply_cat(with);
        b.apply_cat(without);
        assert_eq!(a.pan_ref, b.pan_ref, "{with}");
        assert_eq!(a.pan_span_hz, b.pan_span_hz, "{with}");
        assert_eq!(a.pan_scale, b.pan_scale, "{with}");
        assert_eq!(a.pan_mode, b.pan_mode, "{with}");
        assert_eq!(a.wf_palette, b.wf_palette, "{with}");
        assert_eq!(a.wf_height, b.wf_height, "{with}");
    }

    // And the values actually land, rather than both spellings being no-ops.
    let mut s = RadioState::new();
    s.apply_cat("#REF$-130;");
    s.apply_cat("#SPN$50000;");
    s.apply_cat("#SCL$70;");
    assert_eq!(s.pan_ref, Some(-130));
    assert_eq!(s.pan_span_hz, Some(50_000));
    assert_eq!(s.pan_scale, Some(70));
}
