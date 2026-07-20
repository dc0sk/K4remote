//! StreamCodec tests. trace: FR-AUD-04, FR-PAN-01
use k4_stream::audio::{AudioPacket, EncodeMode, AUDIO_HEADER_SIZE};
use k4_stream::pan::{PanFrame, K4_DBM_OFFSET};

/// Encode → decode round-trips all audio header fields and the data body.
///
/// trace: FR-AUD-04
#[test]
fn fr_aud_04_audio_packet_roundtrip() {
    let data = [0xAA, 0xBB, 0xCC, 0xDD];
    let payload = AudioPacket::encode(42, EncodeMode::OpusFloat, 240, &data);

    // Header is laid out exactly per R-EXT-01.
    assert_eq!(payload[0], 0x01, "type");
    assert_eq!(payload[1], 0x01, "version");
    assert_eq!(payload[2], 42, "sequence");
    assert_eq!(payload[3], 3, "mode = Opus float");
    assert_eq!(&payload[4..6], &240u16.to_le_bytes(), "frame size LE");
    assert_eq!(payload[6], 0x00, "12 kHz sample-rate code");

    let pkt = AudioPacket::decode(&payload).expect("decodes");
    assert_eq!(pkt.sequence, 42);
    assert_eq!(pkt.mode, EncodeMode::OpusFloat);
    assert!(pkt.mode.is_opus());
    assert_eq!(pkt.frame_samples, 240);
    assert_eq!(pkt.data, &data);
}

/// Decode rejects non-audio payloads and short headers.
///
/// trace: FR-AUD-04
#[test]
fn fr_aud_04_audio_decode_rejects_invalid() {
    assert!(AudioPacket::decode(&[]).is_none());
    assert!(AudioPacket::decode(&[0u8; AUDIO_HEADER_SIZE]).is_none()); // wrong type 0x00
    assert!(AudioPacket::decode(&[0x01, 0x01]).is_none()); // too short
}

/// PAN packet decodes metadata and maps bins via `dBm = byte − 146`.
///
/// trace: FR-PAN-01
#[test]
fn fr_pan_01_pan_packet_decode() {
    // Build a PAN payload: receiver=1 (sub), center=14_074_000 Hz,
    // sample_rate=46 (→ 46 kHz span), noise_floor=-1234 raw (→ -123.4 dB).
    let mut p = vec![0u8; 27];
    p[0] = 0x02; // type
    p[4] = 1; // receiver = sub
    p[11..19].copy_from_slice(&14_074_000i64.to_le_bytes());
    p[19..23].copy_from_slice(&46i32.to_le_bytes());
    p[23..27].copy_from_slice(&(-1234i32).to_le_bytes());
    // bins: 146 → 0 dBm, 46 → -100 dBm, 246 → +100 dBm
    p.extend_from_slice(&[146, 46, 246]);

    let frame = PanFrame::decode(&p).expect("decodes");
    assert_eq!(frame.receiver, 1);
    assert_eq!(frame.center_freq_hz, 14_074_000);
    assert_eq!(frame.sample_rate, 46);
    assert_eq!(frame.span_hz(), 46_000);
    assert!((frame.noise_floor_db - (-123.4)).abs() < 1e-4);
    assert_eq!(frame.bins_dbm, vec![0.0, -100.0, 100.0]);
    assert_eq!(K4_DBM_OFFSET, 146.0);
    assert!(!frame.mini);
}

/// MiniPAN (`0x03`) does **not** share the main pan's header: it carries only
/// `type, version, sequence, reserved, receiver` and then its bins, so the bins
/// begin at offset 5 and there is no centre/span/noise-floor (`R-EXT-01`).
///
/// Regression guard: this was decoded with the 27-byte main-pan header, which
/// swallowed 22 bins as phantom metadata and rejected any mini frame shorter
/// than 27 bytes outright — so the mini-pan never displayed.
///
/// trace: FR-PAN-01, FR-UI-14
#[test]
fn fr_ui_14_mini_pan_has_its_own_short_header() {
    // 5-byte header, receiver = sub, then three bins.
    let mut p = vec![0u8; 5];
    p[0] = 0x03;
    p[4] = 1;
    p.extend_from_slice(&[146, 46, 246]);

    let mini = PanFrame::decode(&p).expect("mini decodes");
    assert!(mini.mini);
    assert_eq!(mini.receiver, 1);
    // Every bin after the 5-byte header, none consumed as metadata.
    assert_eq!(mini.bins_dbm, vec![0.0, -100.0, 100.0]);
    // MiniPAN carries no geometry.
    assert_eq!(mini.center_freq_hz, 0);
    assert_eq!(mini.sample_rate, 0);
    assert_eq!(mini.noise_floor_db, 0.0);

    // A realistic mini-pan is far shorter than a main-pan header; it must
    // still decode rather than being dropped.
    let mut short = vec![0u8; 5];
    short[0] = 0x03;
    short.extend_from_slice(&[146; 12]); // 17 bytes total, < PAN_HEADER_SIZE
    let s = PanFrame::decode(&short).expect("a short mini-pan must still decode");
    assert_eq!(s.bins_dbm.len(), 12);

    // The main pan keeps its 27-byte header: same bytes, different type byte,
    // different bin count.
    let mut main = vec![0u8; 27];
    main[0] = 0x02;
    main.extend_from_slice(&[146, 46, 246]);
    assert_eq!(PanFrame::decode(&main).unwrap().bins_dbm.len(), 3);
    // Too short for its own header → rejected.
    assert!(PanFrame::decode(&[0x02, 0, 0, 0, 0]).is_none());
    // Neither type → rejected.
    assert!(PanFrame::decode(&[0x01, 0, 0, 0, 0, 1, 2]).is_none());
}

/// PAN decode rejects non-PAN / short payloads.
///
/// trace: FR-PAN-01
#[test]
fn fr_pan_01_pan_decode_rejects_invalid() {
    assert!(PanFrame::decode(&[0x02; 10]).is_none()); // shorter than header
    assert!(PanFrame::decode(&[0x01; 27]).is_none()); // wrong type
}
