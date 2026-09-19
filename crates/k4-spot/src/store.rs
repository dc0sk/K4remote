//! The bounded, de-duplicating spot store (FR-SPOT-06).

use crate::{retain_fresh, Spot};

/// Two reports of the same callsign this close in frequency are one station:
/// skimmers and receivers hear the same signal a few hundred hertz apart. A
/// starting value, to be tuned against real feeds (`OP-7`).
pub const DEDUPE_TOLERANCE_HZ: u64 = 500;

/// Default bound on the number of spots held.
pub const DEFAULT_CAPACITY: usize = 2_000;

/// What [`SpotStore::insert`] did with a spot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Insert {
    /// A new station: added.
    Added,
    /// The same station was already held and this report is newer: it replaced it.
    Replaced,
    /// The same station was already held with a newer or equal report: ignored.
    Ignored,
}

/// The spots currently known, at most `capacity` of them.
#[derive(Debug, Clone)]
pub struct SpotStore {
    spots: Vec<Spot>,
    capacity: usize,
}

impl Default for SpotStore {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }
}

impl SpotStore {
    /// A store holding at most `capacity` spots (at least 1).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            spots: Vec::new(),
            capacity: capacity.max(1),
        }
    }

    /// Add a report, merging it with a held report of the same station.
    ///
    /// The same station is the same callsign within [`DEDUPE_TOLERANCE_HZ`],
    /// **whichever network reported it**. The newer report is kept; on a tie the
    /// incoming one wins, since the later arrival is the fresher knowledge. When
    /// the store is over capacity the **oldest** spot is evicted.
    pub fn insert(&mut self, spot: Spot) -> Insert {
        let same = |held: &Spot| {
            held.call == spot.call && held.freq_hz.abs_diff(spot.freq_hz) <= DEDUPE_TOLERANCE_HZ
        };
        if let Some(held) = self.spots.iter_mut().find(|h| same(h)) {
            if spot.time >= held.time {
                *held = spot;
                return Insert::Replaced;
            }
            return Insert::Ignored;
        }
        self.spots.push(spot);
        if self.spots.len() > self.capacity {
            if let Some(oldest) = self
                .spots
                .iter()
                .enumerate()
                .min_by_key(|(_, s)| s.time)
                .map(|(i, _)| i)
            {
                self.spots.swap_remove(oldest);
            }
        }
        Insert::Added
    }

    /// Drop every spot older than `max_age_secs` (FR-SPOT-03).
    pub fn purge(&mut self, now: u64, max_age_secs: u64) {
        retain_fresh(&mut self.spots, now, max_age_secs, |s| s.time);
    }

    /// Every spot held, in no particular order.
    pub fn spots(&self) -> &[Spot] {
        &self.spots
    }

    pub fn len(&self) -> usize {
        self.spots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spots.is_empty()
    }
}
