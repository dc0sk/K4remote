//! Reconnect backoff tests. trace: FR-SES-RECONNECT
use std::time::Duration;

use k4_session::Backoff;

/// Delays double from base and cap at max; reset restarts the sequence.
///
/// trace: FR-SES-RECONNECT
#[test]
fn fr_ses_reconnect_backoff_doubles_and_caps() {
    let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(30));

    assert_eq!(b.next_delay(), Duration::from_secs(1));
    assert_eq!(b.next_delay(), Duration::from_secs(2));
    assert_eq!(b.next_delay(), Duration::from_secs(4));
    assert_eq!(b.next_delay(), Duration::from_secs(8));
    assert_eq!(b.next_delay(), Duration::from_secs(16));
    assert_eq!(b.next_delay(), Duration::from_secs(30)); // capped (32 → 30)
    assert_eq!(b.next_delay(), Duration::from_secs(30)); // stays capped
    assert_eq!(b.attempts(), 7);

    b.reset();
    assert_eq!(b.attempts(), 0);
    assert_eq!(b.next_delay(), Duration::from_secs(1));
}

/// The default backoff is 1 s → 30 s.
///
/// trace: FR-SES-RECONNECT
#[test]
fn fr_ses_reconnect_default_range() {
    let mut b = Backoff::default();
    assert_eq!(b.next_delay(), Duration::from_secs(1));
}
