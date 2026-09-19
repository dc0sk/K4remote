//! Spot nameplates: the shared store handle, the clock, and the injected demo spots that stand in
//! for the network feeds until those exist (FR-SPOT-01/02).

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use k4_spot::{Network, Spot, SpotStore};

/// Shared handle to the spot store: the sources fill it, the panadapter reads it at draw time.
pub type SpotHandle = Arc<Mutex<SpotStore>>;

/// Now, in Unix seconds — the clock spots are aged against (FR-SPOT-03).
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `(callsign, offset from 14.074 MHz in Hz, network, age in seconds)`.
///
/// Chosen to exercise the overlay, not to look plausible: clusters that force plates into several
/// lanes, a spot right at each edge of a 48 kHz view, one just outside it, and some older than the
/// default 15-minute limit (which must not be shown).
const DEMO: &[(&str, i64, Network, u64)] = &[
    ("DL1ABC", -21_500, Network::Rbn, 20),
    ("K1XYZ", -17_800, Network::PskReporter, 45),
    // A cluster of three within ~300 Hz.
    ("EA3FOO", -12_000, Network::Rbn, 90),
    ("OK1ZZ", -11_800, Network::PskReporter, 30),
    ("G4XYZ", -11_700, Network::Rbn, 300),
    ("HB9QQ", -3_000, Network::PskReporter, 10),
    // A tight cluster of five, more than three lanes can hold.
    ("JA1AAA", 1_200, Network::Rbn, 5),
    ("W1AW", 1_350, Network::PskReporter, 60),
    ("N2XX", 1_500, Network::Rbn, 120),
    ("VE3CCC", 1_620, Network::PskReporter, 200),
    ("VK2DDD", 1_700, Network::Rbn, 400),
    ("SM5DDD", 9_500, Network::Rbn, 400),
    ("PY2EEE", 15_000, Network::PskReporter, 600),
    ("ZL1FFF", 23_900, Network::Rbn, 800), // just inside the right edge
    ("VK9GGG", 40_000, Network::Rbn, 15),  // outside a 48 kHz view
    ("OH2HHH", 5_000, Network::PskReporter, 3_600), // older than the default limit
];

/// Keep [`DEMO`] in the store for `--demo`, refreshed every few seconds so each spot holds its
/// nominal age instead of ageing out while you look at it.
pub fn spawn_demo_spots(store: SpotHandle) {
    thread::spawn(move || loop {
        let now = unix_now();
        if let Ok(mut s) = store.lock() {
            for &(call, offset, network, age) in DEMO {
                let freq = (14_074_000i64 + offset) as u64;
                if let Some(spot) = Spot::new(call, freq, now.saturating_sub(age), network) {
                    s.insert(spot);
                }
            }
        }
        thread::sleep(Duration::from_secs(5));
    });
}
