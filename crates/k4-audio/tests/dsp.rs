//! Ring buffer + resampler tests. trace: FR-AUD-02, FR-AUD-RX-01
use k4_audio::resample::LinearResampler;
use k4_audio::ring::SampleRing;

/// FIFO order and overflow-drops-oldest.
///
/// trace: FR-AUD-02
#[test]
fn fr_aud_02_ring_fifo_and_overflow() {
    let mut r = SampleRing::new(3);
    r.push_slice(&[1.0, 2.0, 3.0]);
    assert_eq!(r.len(), 3);
    r.push_slice(&[4.0]); // overflow → drops 1.0
    assert_eq!(r.len(), 3);
    assert_eq!(r.pop(), Some(2.0));
    assert_eq!(r.pop(), Some(3.0));
    assert_eq!(r.pop(), Some(4.0));
    assert_eq!(r.pop(), None);
}

/// Upsampling 12 k → 48 k yields ~4× the samples.
///
/// trace: FR-AUD-RX-01
#[test]
fn fr_aud_rx_01_resampler_upsamples_length() {
    let mut rs = LinearResampler::new(12_000, 48_000);
    let input = vec![0.0f32; 1_000];
    let mut out = Vec::new();
    rs.process(&input, &mut out);
    // ~4000 expected (boundary effects allow a few samples of slack).
    assert!(
        (3_990..=4_000).contains(&out.len()),
        "got {} samples",
        out.len()
    );
}

/// A constant (DC) signal resamples to the same constant.
///
/// trace: FR-AUD-RX-01
#[test]
fn fr_aud_rx_01_resampler_preserves_dc() {
    let mut rs = LinearResampler::new(12_000, 24_000);
    let input = vec![0.5f32; 500];
    let mut out = Vec::new();
    rs.process(&input, &mut out);
    assert!(!out.is_empty());
    assert!(out.iter().all(|&s| (s - 0.5).abs() < 1e-6));
}

/// Continuity across two blocks: a steadily rising ramp stays monotonic.
///
/// trace: FR-AUD-RX-01
#[test]
fn fr_aud_rx_01_resampler_is_continuous_across_blocks() {
    let mut rs = LinearResampler::new(12_000, 24_000);
    let first: Vec<f32> = (0..100).map(|i| i as f32).collect();
    let second: Vec<f32> = (100..200).map(|i| i as f32).collect();
    let mut out = Vec::new();
    rs.process(&first, &mut out);
    rs.process(&second, &mut out);
    assert!(
        out.windows(2).all(|w| w[1] >= w[0] - 1e-3),
        "non-monotonic output"
    );
}
