//! Opus codec tests (require the `opus` feature). trace: FR-AUD-04, FR-AUD-TX-01
#![cfg(feature = "opus")]

use k4_audio::codec::{OpusDecoder, OpusEncoder};

/// A 20 ms mono frame survives an encode → decode round-trip with the frame size
/// preserved and finite, bounded samples (Opus is lossy, so exact equality is
/// not expected).
///
/// trace: FR-AUD-TX-01, FR-AUD-04
#[test]
fn opus_mono_roundtrip_preserves_frame_size() {
    let frame: Vec<f32> = (0..240).map(|i| 0.2 * (i as f32 * 0.13).sin()).collect();

    let mut enc = OpusEncoder::mono().unwrap();
    let packet = enc.encode_float(&frame).unwrap();
    assert!(!packet.is_empty(), "encoder produced a packet");

    let mut dec = OpusDecoder::mono().unwrap();
    let pcm = dec.decode_float(&packet).unwrap();

    assert_eq!(pcm.len(), 240, "Opus preserves the 20 ms frame size");
    assert!(pcm.iter().all(|s| s.is_finite() && s.abs() <= 1.5));
}

/// The stereo RX decoder yields interleaved L/R (= Main/Sub): a 240-sample/ch
/// stereo frame decodes to 480 interleaved samples.
///
/// trace: FR-AUD-04, FR-AUD-RX-01
#[test]
fn opus_stereo_decode_interleaves_main_and_sub() {
    // Interleaved stereo frame: left ramps up, right ramps down.
    let mut frame = Vec::with_capacity(480);
    for i in 0..240 {
        frame.push(0.1 * (i as f32 / 240.0)); // L = Main
        frame.push(-0.1 * (i as f32 / 240.0)); // R = Sub
    }

    let mut enc = OpusEncoder::stereo().unwrap();
    let packet = enc.encode_float(&frame).unwrap();

    let mut dec = OpusDecoder::rx().unwrap();
    let pcm = dec.decode_float(&packet).unwrap();

    assert_eq!(pcm.len(), 480, "240 samples/ch × 2 channels, interleaved");
}
