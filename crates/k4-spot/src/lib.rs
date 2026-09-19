//! Spot-nameplate logic (FR-SPOT-*): pure and dependency-free, so it is testable
//! offline. It holds the **age rule** (FR-SPOT-03) and the network-neutral
//! **spot model, source interface and store** (FR-SPOT-06); the network sources
//! and the overlay build on them.
//!
//! All times are Unix seconds. The caller supplies `now`, so nothing here reads a
//! clock and every result is deterministic.

mod model;
mod store;

pub use model::{
    normalise_callsign, sanitise_text, Network, SourceError, Spot, SpotSource, MAX_TEXT_LEN,
};
pub use store::{Insert, SpotStore, DEDUPE_TOLERANCE_HZ, DEFAULT_CAPACITY};

/// How long ago a spot was stamped, in seconds.
///
/// A spot stamped **in the future** — the operator's clock is behind the
/// network's — is age 0, never invalid: dropping it would hide exactly the
/// freshest spots on a machine whose clock runs slow.
pub fn age_secs(now: u64, stamped: u64) -> u64 {
    now.saturating_sub(stamped)
}

/// Whether a spot is still shown: its age is at most `max_age_secs`. A spot
/// exactly at the limit is kept; one second past it is not.
pub fn is_fresh(now: u64, stamped: u64, max_age_secs: u64) -> bool {
    age_secs(now, stamped) <= max_age_secs
}

/// Drop every item older than `max_age_secs` from `items` (the purge that keeps
/// a store from holding spots that can never be shown again). `stamp` reads an
/// item's Unix-seconds timestamp.
pub fn retain_fresh<T>(items: &mut Vec<T>, now: u64, max_age_secs: u64, stamp: impl Fn(&T) -> u64) {
    items.retain(|item| is_fresh(now, stamp(item), max_age_secs));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-SPOT-03: a spot at the limit is kept, one second past it is dropped, a
    /// future stamp is fresh (age 0), and the purge removes only the stale ones.
    /// trace: FR-SPOT-03
    #[test]
    fn fr_spot_03_age_filter_and_purge() {
        let now = 10_000;
        let max = 900; // 15 min

        assert!(
            is_fresh(now, now - 900, max),
            "exactly at the limit is kept"
        );
        assert!(!is_fresh(now, now - 901, max), "one second past is dropped");
        assert!(is_fresh(now, now, max), "a brand-new spot is fresh");

        // Clock skew: stamped after `now` is age 0, never a panic or a drop.
        assert_eq!(age_secs(now, now + 3_600), 0);
        assert!(is_fresh(now, now + 3_600, max));

        // A stamp of 0 against a small `now` must not underflow.
        assert_eq!(age_secs(5, 0), 5);

        // The purge keeps the fresh, drops the stale, and leaves order alone.
        let mut store = vec![
            ("OLD", now - 5_000),
            ("EDGE", now - 900),
            ("PAST", now - 901),
            ("NEW", now - 10),
            ("SKEW", now + 60),
        ];
        retain_fresh(&mut store, now, max, |s| s.1);
        let kept: Vec<_> = store.iter().map(|s| s.0).collect();
        assert_eq!(kept, ["EDGE", "NEW", "SKEW"]);
    }

    /// FR-SPOT-03: a zero limit keeps only spots stamped this very second (or in
    /// the future) — the degenerate extreme behaves, it does not keep everything.
    /// trace: FR-SPOT-03
    #[test]
    fn fr_spot_03_zero_limit_keeps_only_current() {
        assert!(is_fresh(100, 100, 0));
        assert!(!is_fresh(100, 99, 0));
    }
}
