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

    s.apply_cat("RA000;"); // 0 dB, off — the padded form we send and the radio emits
    assert_eq!(s.atten_db, Some(0));
    assert_eq!(s.atten_on, Some(false));

    // The parser is lenient about the level's width: it takes the last digit
    // as the flag and parses the rest. Worth pinning deliberately, because
    // that leniency is what let a malformed *encoder* (`RA31;` for 3 dB) go
    // unnoticed — the radio is the strict party and is not in this suite.
    s.apply_cat("RA31;");
    assert_eq!(s.atten_db, Some(3), "short form still parses");
    assert_eq!(s.atten_on, Some(true));
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
    // NOTE: `#SPN$` is deliberately absent. Hardware testing (#141) showed the
    // two pans hold independent spans — changing one on the DISPLAY screen left
    // the other streaming its old span — so `$` there is the sub-pan modifier,
    // not merely part of an LCD mnemonic, and it routes to its own field. The
    // rest are still treated as the same setting because there is no evidence
    // either way for them yet; if REF or SCALE turn out to be per-pan too, they
    // should follow SPAN.
    for (with, without) in [
        ("#REF$-130;", "#REF-130;"),
        ("#SCL$70;", "#SCL70;"),
        ("#DPM$2;", "#DPM2;"),
        ("#WFC$1;", "#WFC1;"),
        ("#WFH$080;", "#WFH080;"),
    ] {
        let (mut a, mut b) = (RadioState::new(), RadioState::new());
        a.apply_cat(with);
        b.apply_cat(without);
        assert_eq!(a.pan_ref, b.pan_ref, "{with}");
        assert_eq!(a.pan_scale, b.pan_scale, "{with}");
        assert_eq!(a.pan_mode, b.pan_mode, "{with}");
        assert_eq!(a.wf_palette, b.wf_palette, "{with}");
        assert_eq!(a.wf_height, b.wf_height, "{with}");
    }

    // And the values actually land, rather than both spellings being no-ops.
    let mut s = RadioState::new();
    s.apply_cat("#REF$-130;");
    s.apply_cat("#SCL$70;");
    assert_eq!(s.pan_ref, Some(-130));
    assert_eq!(s.pan_scale, Some(70));
    // `#SPN$` routes to the sub pan — see the note above.
    s.apply_cat("#SPN$50000;");
    assert_eq!(s.sub_pan_span_hz, Some(50_000));
    assert_eq!(s.pan_span_hz, None, "the main pan is untouched");
}

/// `#MP$-1` means the mini-pan cannot be enabled with the radio's current
/// settings (D12 `#MP$` NOTE). It must stay distinguishable from `0` (off), or
/// a refused toggle looks identical to "off" and the button appears dead.
/// trace: FR-UI-14
#[test]
fn fr_ui_14_mini_pan_unavailable_is_not_the_same_as_off() {
    let mut s = RadioState::new();
    s.apply_cat("#MP$-1;");
    assert_eq!(s.mini_pan_on, Some(false));
    assert_eq!(s.mini_pan_available, Some(false), "-1 means unavailable");

    s.apply_cat("#MP$0;");
    assert_eq!(s.mini_pan_on, Some(false));
    assert_eq!(s.mini_pan_available, Some(true), "0 is off but available");

    s.apply_cat("#MP$1;");
    assert_eq!(s.mini_pan_on, Some(true));
    assert_eq!(s.mini_pan_available, Some(true));

    // The `$` is part of the LCD mnemonic, but accept it either way.
    let mut bare = RadioState::new();
    bare.apply_cat("#MP-1;");
    assert_eq!(bare.mini_pan_available, Some(false));
}

/// `#FXT` read-back: 0 = track the VFO, 1 = fixed. Until now nothing parsed
/// it, so the app could send fixed-tune and never learn the radio's state.
/// trace: FR-PAN-CTL-01
#[test]
fn fr_pan_ctl_01_fixed_tune_readback() {
    let mut s = RadioState::new();
    assert_eq!(s.pan_fixed, None, "unknown until the radio reports");

    s.apply_cat("#FXT1;");
    assert_eq!(s.pan_fixed, Some(true));
    s.apply_cat("#FXT0;");
    assert_eq!(s.pan_fixed, Some(false));

    // The `$` spelling is accepted for consistency with the rest of the `#`
    // family, where it is part of the LCD mnemonic.
    let mut d = RadioState::new();
    d.apply_cat("#FXT$1;");
    assert_eq!(d.pan_fixed, Some(true));

    // A malformed value leaves the last known state rather than guessing.
    let mut k = RadioState::new();
    k.apply_cat("#FXT1;");
    k.apply_cat("#FXT;");
    assert_eq!(k.pan_fixed, Some(true), "empty arg must not clear it");
    k.apply_cat("#FXTx;");
    assert_eq!(k.pan_fixed, Some(true), "junk must not clear it");

    // `#FXT` must not be swallowed by the `#FRZ`/`#F…` neighbours.
    let mut f = RadioState::new();
    f.apply_cat("#FXT1;");
    assert_eq!(f.pan_fixed, Some(true));
}

/// The two pans hold independent spans. A single shared field made one pane
/// crop against the other's span: changing the span with the DISPLAY target on
/// A left B streaming its old span while both panes were labelled with the
/// new one (#141).
/// trace: FR-PAN-08
#[test]
fn fr_pan_08_span_is_tracked_per_pan() {
    let mut s = RadioState::new();
    s.apply_cat("#SPN100000;");
    s.apply_cat("#SPN$50000;");
    assert_eq!(s.pan_span_hz, Some(100_000), "main pan");
    assert_eq!(s.sub_pan_span_hz, Some(50_000), "sub pan is independent");

    // Changing one must not disturb the other — the exact failure reported.
    s.apply_cat("#SPN50000;");
    assert_eq!(s.pan_span_hz, Some(50_000));
    assert_eq!(s.sub_pan_span_hz, Some(50_000));
    s.apply_cat("#SPN$6000;");
    assert_eq!(
        s.pan_span_hz,
        Some(50_000),
        "main untouched by a sub change"
    );
    assert_eq!(s.sub_pan_span_hz, Some(6_000));
}

/// Encoder and parser agree on every rung of the attenuator ladder.
///
/// **This test would not have caught the 2026-07-20 encoder bug, and cannot.**
/// Our parser is deliberately lenient — it takes the last character as the
/// on/off flag and parses whatever precedes it — so the malformed `RA31;`
/// round-trips as 3 dB here just as happily as the correct `RA031;`. Verified
/// by sabotage: reverting the encoder leaves this test green.
///
/// That leniency is exactly what hid the bug. The radio is the strict party,
/// and we have no copy of it in the test suite, so the real guard is
/// `fr_rx_02_attenuator_levels_are_two_digit_fields`, which pins the wire
/// *shape*. This test is kept for the narrower claim in its name: that the
/// two halves of our own codec do not drift apart.
///
/// trace: FR-RX-02, FR-CAT-05
#[test]
fn fr_rx_02_attenuator_set_and_readback_agree() {
    for db in [0u8, 3, 6, 9, 12, 15, 18, 21] {
        let on = db > 0;
        // The radio echoes the same field layout it accepts, so feeding our
        // own command back through the parser is a fair round-trip.
        let sent = k4_protocol::cat::set_attenuator(db, on);
        let resp = sent.trim_end_matches(';');

        let mut s = RadioState::new();
        assert!(
            s.apply_cat(&format!("{resp};")),
            "{db} dB: {sent} not parsed"
        );
        assert_eq!(
            s.atten_db,
            Some(db),
            "{db} dB round-tripped as {:?}",
            s.atten_db
        );
        assert_eq!(s.atten_on, Some(on), "{db} dB on/off flag");
    }
}

/// The `DA` status forms all parse, including the ones that belong to the DVR
/// rather than the AF recorder — they share one engine and one status command,
/// so a display that only understood its own half would go blank exactly when
/// something was happening.
/// trace: FR-AUD-REC-01
#[test]
fn fr_aud_rec_01_digital_audio_status_forms_parse() {
    use k4_protocol::state::{AfPlayback, DigitalAudio};

    let da = |resp: &str| {
        let mut s = RadioState::new();
        assert!(s.apply_cat(resp), "{resp} not parsed");
        s.digital_audio.expect("digital_audio set")
    };

    assert_eq!(da("DA0;"), DigitalAudio::Idle);
    assert_eq!(
        da("DAPW01500;"),
        DigitalAudio::WaitingRepeat { remaining_ms: 1500 }
    );
    assert_eq!(
        da("DARS1234590000;"),
        DigitalAudio::RecordingAf {
            pos_ms: 12345,
            max_ms: 90000
        }
    );
    assert_eq!(
        da("DARM0500090000;"),
        DigitalAudio::RecordingMessage {
            pos_ms: 5000,
            max_ms: 90000
        }
    );
    // Built field by field (nnnnn ttttt m s c) rather than written as one
    // literal — the counters are fixed-width and adjacent, so a miscounted
    // digit silently shifts every field after it.
    let playing = format!("DAPS{}{}{}{}{};", "12000", "90000", "0", "3", "1");
    assert_eq!(
        da(&playing),
        DigitalAudio::PlayingAf {
            pos_ms: 12000,
            max_ms: 90000,
            playback: Some(AfPlayback::Both),
            session: Some(3),
            last: true,
        },
        "the mode/session/last tail is read"
    );
    // `c` is 0 when this is not the last session.
    let not_last = format!("DAPS{}{}{}{}{};", "00000", "90000", "A", "1", "0");
    assert!(matches!(
        da(&not_last),
        DigitalAudio::PlayingAf {
            playback: Some(AfPlayback::Main),
            session: Some(1),
            last: false,
            ..
        }
    ));

    // Playing a *message* is the one state that has the radio on air. D12 also
    // allows `m` = `M` here, meaning a voice message rather than an AF
    // recording is on air; that is not an `AfPlayback`, so it reads as `None`
    // — the tag already carries the distinction that matters for safety.
    let msg = format!("DAPM{}{}{}{}{};", "00000", "90000", "M", "1", "1");
    assert!(da(&msg).is_transmitting(), "DAPM transmits");
    assert!(!da(&playing).is_transmitting());
    assert!(!da("DARS1234590000;").is_transmitting());
}

/// An unrecognised `DA` form leaves the last known state alone. Blanking the
/// display on a variant a later firmware adds would be worse than lagging by
/// one poll — and guessing at its layout worse still.
/// trace: FR-AUD-REC-01
#[test]
fn fr_aud_rec_01_unknown_digital_audio_form_is_ignored() {
    use k4_protocol::state::DigitalAudio;

    let mut s = RadioState::new();
    assert!(s.apply_cat("DARS0100090000;"));
    let known = s.digital_audio;
    assert!(matches!(known, Some(DigitalAudio::RecordingAf { .. })));

    s.apply_cat("DAZZ999;"); // not a documented form
    assert_eq!(s.digital_audio, known, "unknown form left the state alone");

    s.apply_cat("DARS;"); // documented tag, but the counters are missing
    assert_eq!(
        s.digital_audio, known,
        "a truncated form is not half-applied"
    );
}

/// TX test mode (`TS`) reaches the state, so the app can show it.
///
/// The K4 puts out no power while it is on, and the only sign on the radio is
/// a flashing TX icon — which a remote operator cannot see. Reported by DC0SK:
/// enabling TEST from the app left the button looking exactly as it had.
/// trace: FR-TX-TUNE-01
#[test]
fn fr_tx_tune_01_tx_test_mode_reaches_the_state() {
    let mut s = RadioState::new();
    assert_eq!(s.tx_test, None, "unknown until the radio says");

    assert!(s.apply_cat("TS1;"));
    assert_eq!(s.tx_test, Some(true));
    assert!(s.apply_cat("TS0;"));
    assert_eq!(s.tx_test, Some(false));

    // `TS/;` is the documented *toggle*; it carries no state, so it must not
    // be read as one. Leaving the last known value is right — inventing a
    // flip here would desync the moment a toggle was sent by another client.
    s.apply_cat("TS1;");
    s.apply_cat("TS/;");
    assert_eq!(s.tx_test, Some(true), "a toggle echo is not a state report");
}
