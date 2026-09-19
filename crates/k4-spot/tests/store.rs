//! The spot model, source interface and store, through the public API only.
//! trace: FR-SPOT-06

use k4_spot::{
    normalise_callsign, sanitise_text, Insert, Network, SourceError, Spot, SpotSource, SpotStore,
    DEDUPE_TOLERANCE_HZ, MAX_TEXT_LEN,
};

fn spot(call: &str, freq: u64, time: u64, net: Network) -> Spot {
    Spot::new(call, freq, time, net).expect("valid test spot")
}

/// FR-SPOT-06: callsigns are normalised to upper case and trimmed; anything that
/// is not a callsign — control, inner-space or bidi-override characters,
/// look-alikes, over-long, malformed slashes — is rejected, never repaired.
/// trace: FR-SPOT-06
#[test]
fn fr_spot_06_callsign_normalised_or_rejected() {
    assert_eq!(normalise_callsign("dc0sk").as_deref(), Some("DC0SK"));
    assert_eq!(
        normalise_callsign("  dl/dc0sk/p ").as_deref(),
        Some("DL/DC0SK/P")
    );
    assert_eq!(normalise_callsign("K1A").as_deref(), Some("K1A"));
    assert_eq!(normalise_callsign("DC0SK\n").as_deref(), Some("DC0SK"));

    let bad = [
        "",
        "AB",                // too short
        "CQ",                // no digit
        "TEST",              // no digit
        "73",                // no letter
        "DC 0SK",            // inner space
        "DC0\0SK",           // control
        "DC0\u{202e}SK",     // bidi override
        "DC0\u{200b}SK",     // zero-width space
        "D\u{0421}0SK",      // Cyrillic capital Es, a look-alike for C
        "DC\u{0660}SK",      // Arabic-indic zero
        "/DC0SK",            // leading slash
        "DC0SK/",            // trailing slash
        "DC0//SK",           // doubled slash
        "DC0SK;ls",          // separator
        "ABCDEFGHIJKLMNOP1", // 17 characters
    ];
    for raw in bad {
        assert_eq!(normalise_callsign(raw), None, "should reject {raw:?}");
    }
    // 16 characters is the limit and is accepted.
    assert!(normalise_callsign("ABCDEFGHIJKLMNO1").is_some());
}

/// FR-SPOT-06: free text is printable ASCII, non-empty and bounded, and is
/// rejected — not cut — when it is not. A bad optional field is dropped and the
/// spot survives; a zero frequency or bad callsign loses the spot.
/// trace: FR-SPOT-06
#[test]
fn fr_spot_06_text_and_spot_construction() {
    assert_eq!(sanitise_text("  CQ DX  ").as_deref(), Some("CQ DX"));
    assert_eq!(sanitise_text(""), None);
    assert_eq!(sanitise_text("   "), None);
    assert_eq!(sanitise_text("bell\u{7}"), None);
    assert_eq!(sanitise_text("caf\u{e9}"), None);
    assert!(sanitise_text(&"x".repeat(MAX_TEXT_LEN)).is_some());
    assert_eq!(
        sanitise_text(&"x".repeat(MAX_TEXT_LEN + 1)),
        None,
        "not cut"
    );

    let s = Spot::new("dc0sk", 14_074_000, 100, Network::PskReporter)
        .unwrap()
        .with_mode("FT8")
        .with_snr(-12)
        .with_spotter("K1ABC")
        .with_comment("bad\u{7}comment");
    assert_eq!(s.call, "DC0SK");
    assert_eq!(s.mode.as_deref(), Some("FT8"));
    assert_eq!(s.snr_db, Some(-12));
    assert_eq!(s.spotter.as_deref(), Some("K1ABC"));
    assert_eq!(
        s.comment, None,
        "a bad comment is dropped, the spot is kept"
    );

    assert!(Spot::new("dc0sk", 0, 100, Network::Rbn).is_none(), "0 Hz");
    assert!(Spot::new("CQ", 14_074_000, 100, Network::Rbn).is_none());
}

/// FR-SPOT-06: the same callsign within the tolerance is one station whichever
/// network reported it, the newer report wins, and a different callsign or a
/// frequency past the tolerance is a different station.
/// trace: FR-SPOT-06
#[test]
fn fr_spot_06_store_dedupe_bound_and_sanitise() {
    let mut store = SpotStore::default();
    let f = 14_074_000;

    assert_eq!(
        store.insert(spot("DC0SK", f, 100, Network::PskReporter)),
        Insert::Added
    );
    // The same station from another network, newer: replaces.
    assert_eq!(
        store.insert(spot("dc0sk", f + 100, 200, Network::Rbn)),
        Insert::Replaced
    );
    assert_eq!(store.len(), 1);
    assert_eq!(store.spots()[0].time, 200);
    assert_eq!(
        store.spots()[0].network,
        Network::Rbn,
        "the newest report is kept"
    );

    // An older report of the same station is ignored.
    assert_eq!(
        store.insert(spot("DC0SK", f, 150, Network::DxCluster)),
        Insert::Ignored
    );
    assert_eq!(store.spots()[0].time, 200);

    // A tie: the incoming report wins.
    assert_eq!(
        store.insert(spot("DC0SK", f, 200, Network::PskReporter)),
        Insert::Replaced
    );
    assert_eq!(store.spots()[0].network, Network::PskReporter);

    // The tolerance edge: exactly at it merges, one hertz past it does not.
    assert_eq!(
        store.insert(spot("DC0SK", f + DEDUPE_TOLERANCE_HZ, 300, Network::Rbn)),
        Insert::Replaced
    );
    assert_eq!(store.len(), 1);
    let far = store.spots()[0].freq_hz + DEDUPE_TOLERANCE_HZ + 1;
    assert_eq!(
        store.insert(spot("DC0SK", far, 300, Network::Rbn)),
        Insert::Added
    );
    assert_eq!(store.len(), 2, "beyond the tolerance is another signal");

    // A different callsign on the same frequency is a different station.
    assert_eq!(
        store.insert(spot("K1ABC", f, 300, Network::Rbn)),
        Insert::Added
    );
    assert_eq!(store.len(), 3);
}

/// FR-SPOT-06: the store is bounded — over capacity the oldest spot goes — so a
/// flood of distinct stations cannot grow it without limit.
/// trace: FR-SPOT-06
#[test]
fn fr_spot_06_store_is_bounded_oldest_out() {
    let mut store = SpotStore::with_capacity(3);
    for i in 0..10u64 {
        let call = format!("K{}ABC", i);
        store.insert(spot(&call, 7_000_000 + i * 10_000, 100 + i, Network::Rbn));
        assert!(store.len() <= 3, "never exceeds capacity");
    }
    assert_eq!(store.len(), 3);
    let mut times: Vec<u64> = store.spots().iter().map(|s| s.time).collect();
    times.sort_unstable();
    assert_eq!(times, [107, 108, 109], "the three newest are kept");

    // A zero capacity still holds one rather than none.
    let mut tiny = SpotStore::with_capacity(0);
    tiny.insert(spot("DC0SK", 14_074_000, 1, Network::Rbn));
    assert_eq!(tiny.len(), 1);
}

/// FR-SPOT-03 / FR-SPOT-06: the store purges spots past the age limit.
/// trace: FR-SPOT-03, FR-SPOT-06
#[test]
fn fr_spot_06_store_purges_by_age() {
    let mut store = SpotStore::default();
    store.insert(spot("K1AAA", 7_010_000, 1_000, Network::Rbn)); // 900 s old at 1900
    store.insert(spot("K1BBB", 7_020_000, 999, Network::Rbn)); // 901 s old
    store.insert(spot("K1CCC", 7_030_000, 1_900, Network::Rbn)); // fresh
    store.purge(1_900, 900);
    let mut calls: Vec<&str> = store.spots().iter().map(|s| s.call.as_str()).collect();
    calls.sort_unstable();
    assert_eq!(
        calls,
        ["K1AAA", "K1CCC"],
        "the limit is kept, one second past is purged"
    );
}

/// A source that hands over a fixed batch, then fails.
struct Fake {
    net: Network,
    batch: Vec<Spot>,
    fail: bool,
}

impl SpotSource for Fake {
    fn network(&self) -> Network {
        self.net
    }
    fn poll(&mut self, sink: &mut dyn FnMut(Spot)) -> Result<(), SourceError> {
        if self.fail {
            return Err(SourceError("connection refused".into()));
        }
        for s in self.batch.drain(..) {
            sink(s);
        }
        Ok(())
    }
}

/// FR-SPOT-06: sources are interchangeable behind the interface — the store
/// takes spots from any of them without knowing which — and a failure comes back
/// as an error tied to its network instead of vanishing.
/// trace: FR-SPOT-06
#[test]
fn fr_spot_06_sources_are_interchangeable_and_report_failure() {
    let mut sources: Vec<Box<dyn SpotSource>> = vec![
        Box::new(Fake {
            net: Network::PskReporter,
            batch: vec![spot("DC0SK", 14_074_000, 10, Network::PskReporter)],
            fail: false,
        }),
        Box::new(Fake {
            net: Network::Rbn,
            batch: vec![spot("K1ABC", 7_030_000, 11, Network::Rbn)],
            fail: false,
        }),
        Box::new(Fake {
            net: Network::DxCluster,
            batch: vec![],
            fail: true,
        }),
    ];
    let mut store = SpotStore::default();
    let mut errors = Vec::new();
    for src in &mut sources {
        match src.poll(&mut |s| {
            store.insert(s);
        }) {
            Ok(()) => {}
            Err(e) => errors.push((src.network(), e.to_string())),
        }
    }
    assert_eq!(store.len(), 2, "both healthy networks contributed");
    assert_eq!(
        errors,
        [(Network::DxCluster, "connection refused".to_string())]
    );
}
