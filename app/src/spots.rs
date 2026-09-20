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

/// How far either side of a VFO the telnet feeds are kept, Hz. A pan is at most 368 kHz wide, so
/// this covers a whole view centred on the VFO. Dropping everything else before it is stored (and
/// before it can use up the rate budget) is what makes the unfiltered RBN relay affordable.
pub const SPOT_WINDOW_HALF_HZ: u64 = 300_000;

/// The frequency range worth keeping spots for, given the two VFOs: from the lower one minus the
/// half-width to the higher one plus it. With no VFO known there is no view to label, so the range
/// is empty (`lo > hi`) and nothing is kept.
pub fn spot_window(vfo_a_hz: Option<u64>, vfo_b_hz: Option<u64>) -> (u64, u64) {
    let known: Vec<u64> = [vfo_a_hz, vfo_b_hz]
        .into_iter()
        .flatten()
        .filter(|f| *f > 0)
        .collect();
    match (known.iter().min(), known.iter().max()) {
        (Some(&lo), Some(&hi)) => (
            lo.saturating_sub(SPOT_WINDOW_HALF_HZ),
            hi + SPOT_WINDOW_HALF_HZ,
        ),
        _ => (1, 0),
    }
}

/// Whether the worker needs telling about a new window: the first time, when it becomes empty or
/// stops being empty, or when an edge has moved by at least 20 kHz (so a slow tune does not send
/// one every tick).
pub fn window_needs_update(sent: Option<(u64, u64)>, new: (u64, u64)) -> bool {
    let Some((lo, hi)) = sent else { return true };
    let (nlo, nhi) = new;
    if (lo > hi) != (nlo > nhi) {
        return true;
    }
    lo.abs_diff(nlo) >= 20_000 || hi.abs_diff(nhi) >= 20_000
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-SPOT-07: the window kept from the telnet feeds follows the VFOs, is empty when none is
    /// known, and is only resent when it has moved enough to matter.
    /// trace: FR-SPOT-07
    #[test]
    fn fr_spot_07_window_follows_the_vfos() {
        let h = SPOT_WINDOW_HALF_HZ;
        assert_eq!(
            spot_window(Some(14_074_000), None),
            (14_074_000 - h, 14_074_000 + h)
        );
        assert_eq!(
            spot_window(None, Some(7_030_000)),
            (7_030_000 - h, 7_030_000 + h)
        );
        // Both VFOs: one window spanning them (split, or A and B on different bands).
        assert_eq!(
            spot_window(Some(14_074_000), Some(7_030_000)),
            (7_030_000 - h, 14_074_000 + h)
        );
        // Nothing known: an empty range that admits nothing.
        let (lo, hi) = spot_window(None, None);
        assert!(lo > hi);
        let (lo0, hi0) = spot_window(Some(0), Some(0));
        assert!(lo0 > hi0, "0 Hz is not a frequency");
        // Near 0 Hz the lower edge saturates instead of wrapping.
        assert_eq!(spot_window(Some(100_000), None).0, 0);

        // Resending: first time always; small moves not; a big move yes; emptiness changes yes.
        let w = spot_window(Some(14_074_000), None);
        assert!(window_needs_update(None, w));
        assert!(!window_needs_update(Some(w), w));
        let nudged = spot_window(Some(14_074_000 + 5_000), None);
        assert!(
            !window_needs_update(Some(w), nudged),
            "5 kHz is not worth a message"
        );
        let moved = spot_window(Some(14_074_000 + 25_000), None);
        assert!(window_needs_update(Some(w), moved));
        assert!(window_needs_update(Some(w), (1, 0)), "the radio went away");
        assert!(window_needs_update(Some((1, 0)), w), "the radio came back");
        assert!(!window_needs_update(Some((1, 0)), (1, 0)));
    }
}
