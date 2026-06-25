//! Jitter buffer tests. trace: FR-AUD-02, FR-AUD-05
use k4_audio::JitterBuffer;

fn f(tag: u8) -> Vec<u8> {
    vec![tag]
}

/// In-order frames come out in order.
///
/// trace: FR-AUD-02
#[test]
fn fr_aud_02_in_order_passthrough() {
    let mut jb = JitterBuffer::new(4);
    for s in 0..3u8 {
        jb.push(s, f(s));
    }
    assert_eq!(jb.pop(), Some(f(0)));
    assert_eq!(jb.pop(), Some(f(1)));
    assert_eq!(jb.pop(), Some(f(2)));
    assert_eq!(jb.pop(), None);
}

/// Out-of-order arrivals are released in sequence order.
///
/// trace: FR-AUD-02
#[test]
fn fr_aud_02_reorders_late_arrival() {
    let mut jb = JitterBuffer::new(4);
    jb.push(0, f(0));
    jb.push(2, f(2));
    jb.push(1, f(1)); // arrives late
    assert_eq!(jb.pop(), Some(f(0)));
    assert_eq!(jb.pop(), Some(f(1)));
    assert_eq!(jb.pop(), Some(f(2)));
}

/// Duplicate sequence numbers are dropped (first one wins).
///
/// trace: FR-AUD-05
#[test]
fn fr_aud_05_drops_duplicates() {
    let mut jb = JitterBuffer::new(4);
    jb.push(0, f(0));
    jb.push(0, vec![0xFF]); // duplicate seq — ignored
    assert_eq!(jb.len(), 1);
    assert_eq!(jb.pop(), Some(f(0)));
    assert_eq!(jb.pop(), None);
}

/// Frames for an already-played sequence are dropped as "late".
///
/// trace: FR-AUD-05
#[test]
fn fr_aud_05_drops_late_frames() {
    let mut jb = JitterBuffer::new(4);
    jb.push(0, f(0));
    jb.push(1, f(1));
    assert_eq!(jb.pop(), Some(f(0)));
    assert_eq!(jb.pop(), Some(f(1))); // next = 2

    jb.push(0, vec![0xFF]); // seq 0 is now in the past → dropped
    assert!(jb.is_empty());
    assert_eq!(jb.pop(), None);
}

/// A persistent gap is concealed once the buffer fills, bounding latency.
///
/// trace: FR-AUD-02
#[test]
fn fr_aud_02_conceals_gap_at_capacity() {
    let mut jb = JitterBuffer::new(2);
    jb.push(5, f(5));
    assert_eq!(jb.pop(), Some(f(5))); // next = 6

    // 6 and 7 are lost; 8 and 9 arrive and fill capacity.
    jb.push(8, f(8));
    jb.push(9, f(9));
    // Gap at 6, but len (2) >= capacity (2) → conceal, jump to 8.
    assert_eq!(jb.pop(), Some(f(8)));
    assert_eq!(jb.pop(), Some(f(9)));
}

/// Wrapping across the 255→0 boundary preserves order.
///
/// trace: FR-AUD-02
#[test]
fn fr_aud_02_wraps_around_255() {
    let mut jb = JitterBuffer::new(4);
    jb.push(254, f(254));
    jb.push(255, f(255));
    jb.push(0, f(0));
    jb.push(1, f(1));
    assert_eq!(jb.pop(), Some(f(254)));
    assert_eq!(jb.pop(), Some(f(255)));
    assert_eq!(jb.pop(), Some(f(0)));
    assert_eq!(jb.pop(), Some(f(1)));
}
