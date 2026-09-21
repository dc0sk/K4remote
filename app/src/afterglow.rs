//! Spectrum afterglow (FR-PAN-14): a peak lingers on the trace and fades, so a brief signal —
//! a CW dit, an FT8 tone that comes and goes — can still be seen after it has passed.
//!
//! The *ghost* is a second trace kept beside the live one. Where the live trace rises above it, the
//! ghost jumps up to meet it at once (no attack delay); where the live trace falls below it, the
//! ghost falls slowly, at a constant number of dB per second. That is an exponential decay of
//! **power** with time constant τ, the operator's setting: a level falls `10·log10(e)·t/τ` dB in
//! `t` — about 4.34 dB per τ. The ghost is never below the live trace.
//!
//! It is advanced **once per row, when the row arrives** (from [`crate::worker::PanShared::push`]),
//! not once per drawn frame. How often the window redraws — which `FR-PAN-13` deliberately keeps
//! close to the row rate — therefore cannot change how the trace looks, and a peak in a row that
//! no frame happened to show is not lost.

use std::time::{Duration, Instant};

use crate::worker::PanRow;

/// The longest gap taken as time passing between two rows. After a longer stall (the app was
/// suspended, the link dropped) the ghost has faded as far as it will, and a huge step must not
/// be able to overflow or flatten it.
const MAX_STEP: Duration = Duration::from_secs(5);

/// A level cannot be below this, dBm. Keeps a run of non-finite input from poisoning the ghost.
const FLOOR_DBM: f32 = -300.0;

/// `10·log10(e)`: the dB an exponential decay of power falls in one time constant.
const DB_PER_TAU: f64 = 4.342_944_819_032_518;

/// How far the ghost falls in `dt`, dB, for a time constant `tau`. Zero for no time or no
/// trail.
pub fn fall_db(dt: Duration, tau: Duration) -> f32 {
    if tau.is_zero() {
        return 0.0;
    }
    (DB_PER_TAU * dt.as_secs_f64() / tau.as_secs_f64()) as f32
}

/// The ghost trace of one receiver.
#[derive(Debug, Default)]
pub struct Afterglow {
    /// The time constant; `None` is off.
    tau: Option<Duration>,
    ghost: Vec<f32>,
    /// Centre, span and width of the row the ghost was last matched to: a ghost drawn against a
    /// different span or width would put every peak at the wrong frequency.
    shape: Option<(i64, u32, usize)>,
    last: Option<Instant>,
}

impl Afterglow {
    /// Set the trail from the Settings value (milliseconds; `0` is off). The value is brought into
    /// range by [`k4_config::sanitise_afterglow_ms`]. Changing it — including turning it off —
    /// starts the ghost afresh.
    pub fn set_ms(&mut self, ms: u32) {
        let tau = match k4_config::sanitise_afterglow_ms(ms) {
            0 => None,
            ms => Some(Duration::from_millis(u64::from(ms))),
        };
        if tau != self.tau {
            self.tau = tau;
            self.clear();
        }
    }

    /// Forget the ghost (the pan was reset, or the trail was switched).
    pub fn clear(&mut self) {
        self.ghost.clear();
        self.shape = None;
        self.last = None;
    }

    /// Take in a newly arrived row at time `now`.
    pub fn feed(&mut self, row: &PanRow, now: Instant) {
        let Some(tau) = self.tau else { return };
        let shape = (row.center_hz, row.span_hz, row.bins.len());
        if self.shape != Some(shape) || self.ghost.len() != row.bins.len() {
            // The first row, or the pan moved or changed width: start from the live trace.
            self.ghost = row
                .bins
                .iter()
                .map(|b| if b.is_finite() { *b } else { FLOOR_DBM })
                .collect();
            self.shape = Some(shape);
            self.last = Some(now);
            return;
        }
        let dt = self
            .last
            .map_or(Duration::ZERO, |t| now.saturating_duration_since(t))
            .min(MAX_STEP);
        let fall = fall_db(dt, tau);
        for (g, &live) in self.ghost.iter_mut().zip(&row.bins) {
            let fallen = *g - fall;
            let next = if live.is_finite() {
                live.max(fallen)
            } else {
                fallen
            };
            *g = if next.is_nan() {
                FLOOR_DBM
            } else {
                next.max(FLOOR_DBM)
            };
        }
        self.last = Some(now);
    }

    /// The ghost, dBm per bin like a row's `bins`; `None` when nothing has arrived yet — which is
    /// always the case while the trail is off, because [`feed`](Self::feed) does nothing then and
    /// switching it off cleared what there was.
    pub fn ghost(&self) -> Option<&[f32]> {
        (!self.ghost.is_empty()).then_some(self.ghost.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(bins: Vec<f32>) -> PanRow {
        PanRow {
            bins,
            center_hz: 14_074_000,
            span_hz: 48_000,
        }
    }

    /// FR-PAN-14: the fall rate is the exponential-decay one: 10·log10(e) dB per time constant,
    /// proportional to the time and inversely to τ, and nothing for no time or no trail.
    /// trace: FR-PAN-14
    #[test]
    fn fr_pan_14_fall_is_exponential_power_decay() {
        let s = Duration::from_secs;
        // One time constant: 4.3429 dB (a power ratio of 1/e).
        assert!((fall_db(s(1), s(1)) - 4.3429).abs() < 1e-3);
        // The ratio it implies is 1/e, checked the other way round.
        let power_ratio = 10f64.powf(-f64::from(fall_db(s(1), s(1))) / 10.0);
        assert!((power_ratio - (-1f64).exp()).abs() < 1e-4, "{power_ratio}");
        // Proportional to time, inverse to τ.
        assert!((fall_db(s(3), s(1)) - 3.0 * fall_db(s(1), s(1))).abs() < 1e-3);
        assert!((fall_db(s(1), s(4)) - fall_db(s(1), s(1)) / 4.0).abs() < 1e-3);
        assert_eq!(fall_db(Duration::ZERO, s(1)), 0.0);
        assert_eq!(fall_db(s(1), Duration::ZERO), 0.0, "no trail, no fall");
    }

    /// FR-PAN-14: off does nothing; on, a rising level is met at once and a falling one decays at
    /// the set rate — against the closed-form value — and never below the live trace.
    /// trace: FR-PAN-14
    #[test]
    fn fr_pan_14_the_ghost_attacks_at_once_and_decays_at_the_set_rate() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);

        // Off (the default): nothing is kept.
        let mut off = Afterglow::default();
        off.feed(&row(vec![-100.0; 4]), at(0));
        assert!(off.ghost().is_none());

        let mut g = Afterglow::default();
        g.set_ms(1000);
        assert!(g.ghost().is_none(), "nothing has arrived yet");
        // A quiet floor with one strong bin.
        g.feed(&row(vec![-120.0, -120.0, -50.0, -120.0]), at(0));
        assert_eq!(g.ghost().unwrap(), [-120.0, -120.0, -50.0, -120.0]);

        // The peak goes away; the floor stays. Rows every 33 ms for one second.
        let mut t = 0;
        for _ in 0..30 {
            t += 33;
            g.feed(&row(vec![-120.0; 4]), at(t));
        }
        // After 990 ms the peak has fallen 990/1000 · 4.3429 dB, to within f32 rounding.
        let want = -50.0 - 4.3429 * 0.99;
        let got = g.ghost().unwrap()[2];
        assert!((got - want).abs() < 0.02, "peak at {got}, wanted {want}");
        // The floor bins are exactly the live trace: the ghost is never below it.
        assert_eq!(g.ghost().unwrap()[0], -120.0);
        assert!(g.ghost().unwrap().iter().all(|v| *v >= -120.0));

        // A new, higher peak is met at once, in the same row.
        g.feed(&row(vec![-120.0, -30.0, -120.0, -120.0]), at(t + 33));
        assert_eq!(g.ghost().unwrap()[1], -30.0);
        // A level that rises *slowly* is followed exactly, never lagging.
        let mut slow = Afterglow::default();
        slow.set_ms(500);
        for (i, v) in [-100.0, -90.0, -80.0, -70.0].into_iter().enumerate() {
            slow.feed(&row(vec![v]), at(i as u64 * 33));
            assert_eq!(slow.ghost().unwrap()[0], v);
        }
    }

    /// FR-PAN-14: a longer time constant holds a peak longer; the fall is the same however the
    /// time is cut into rows, and a long stall cannot flatten the ghost or overflow.
    /// trace: FR-PAN-14
    #[test]
    fn fr_pan_14_time_constant_and_row_rate() {
        let t0 = Instant::now();
        let fell = |tau_ms: u32, rows: u64, step_ms: u64| {
            let mut g = Afterglow::default();
            g.set_ms(tau_ms);
            g.feed(&row(vec![-40.0]), t0);
            for i in 1..=rows {
                g.feed(&row(vec![-140.0]), t0 + Duration::from_millis(i * step_ms));
            }
            -40.0 - g.ghost().unwrap()[0]
        };
        // Longer τ, slower fall.
        let (short, long) = (fell(300, 30, 33), fell(3000, 30, 33));
        assert!(
            short > long * 9.0 && short < long * 11.0,
            "{short} vs {long}"
        );
        // The same second, as 30 rows or as 4: the same fall.
        let (fine, coarse) = (fell(1000, 30, 33), fell(1000, 4, 247));
        assert!((fine - coarse).abs() < 0.05, "{fine} vs {coarse}");
        // A one-minute stall is a five-second step at most.
        let stalled = {
            let mut g = Afterglow::default();
            g.set_ms(1000);
            g.feed(&row(vec![-40.0]), t0);
            g.feed(&row(vec![-140.0]), t0 + Duration::from_secs(60));
            -40.0 - g.ghost().unwrap()[0]
        };
        assert!((stalled - 5.0 * 4.3429).abs() < 0.01, "{stalled}");
        // The longest trail still fades: 5 s of τ, one minute later, is well down.
        assert!(fell(5000, 1800, 33) > 10.0);
    }

    /// FR-PAN-14: when the pan moves or changes width the ghost starts from the live trace instead
    /// of being drawn at the wrong frequencies; changing or switching off the trail starts afresh;
    /// receivers do not share a ghost.
    /// trace: FR-PAN-14
    #[test]
    fn fr_pan_14_the_ghost_follows_the_pan_and_the_setting() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut g = Afterglow::default();
        g.set_ms(2000);
        g.feed(&row(vec![-40.0, -40.0]), at(0));
        // The same shape keeps its ghost.
        g.feed(&row(vec![-140.0, -140.0]), at(33));
        assert!(g.ghost().unwrap()[0] > -41.0);
        // A different centre, a different span or a different width each start over.
        for changed in [
            PanRow {
                center_hz: 14_075_000,
                ..row(vec![-140.0, -140.0])
            },
            PanRow {
                span_hz: 96_000,
                ..row(vec![-140.0, -140.0])
            },
            row(vec![-140.0, -140.0, -140.0]),
        ] {
            let mut h = Afterglow::default();
            h.set_ms(2000);
            h.feed(&row(vec![-40.0, -40.0]), at(0));
            h.feed(&changed, at(33));
            assert_eq!(h.ghost().unwrap(), &changed.bins[..], "{changed:?}");
        }
        // The same trail setting again keeps the ghost; a different one, or off, drops it.
        g.set_ms(2000);
        assert!(g.ghost().is_some());
        g.set_ms(1000);
        assert!(g.ghost().is_none(), "changed: starts afresh");
        g.feed(&row(vec![-90.0, -90.0]), at(100));
        assert_eq!(g.ghost().unwrap(), [-90.0, -90.0]);
        g.set_ms(0);
        assert!(g.ghost().is_none(), "off");
        g.feed(&row(vec![-90.0, -90.0]), at(133));
        assert!(g.ghost().is_none(), "and stays off");
        g.clear();
        assert!(g.ghost().is_none());

        // A value out of range is clamped, not refused: 1 ms behaves as the 50 ms minimum and a
        // huge one as the 5 s maximum, measured by how far a peak falls in one second.
        let fell_in_a_second = |ms: u32| {
            let mut g = Afterglow::default();
            g.set_ms(ms);
            g.feed(&row(vec![-40.0]), at(0));
            g.feed(&row(vec![-140.0]), at(1000));
            -40.0 - g.ghost().unwrap()[0]
        };
        assert_eq!(
            fell_in_a_second(1),
            fell_in_a_second(k4_config::AFTERGLOW_MIN_MS)
        );
        assert_eq!(
            fell_in_a_second(99_999),
            fell_in_a_second(k4_config::AFTERGLOW_MAX_MS)
        );
        assert!(fell_in_a_second(1) > fell_in_a_second(60));
        assert!(fell_in_a_second(99_999) < fell_in_a_second(4000));
    }

    /// FR-PAN-14: bad input — NaN, infinities, an empty row — never panics or leaves a
    /// non-finite ghost.
    /// trace: FR-PAN-14
    #[test]
    fn fr_pan_14_bad_input_cannot_poison_the_ghost() {
        let t0 = Instant::now();
        let mut g = Afterglow::default();
        g.set_ms(500);
        g.feed(
            &row(vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -80.0]),
            t0,
        );
        assert!(g.ghost().unwrap().iter().all(|v| v.is_finite()));
        for i in 1..100u64 {
            g.feed(
                &row(vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX]),
                t0 + Duration::from_millis(i * 33),
            );
            assert!(g
                .ghost()
                .unwrap()
                .iter()
                .all(|v| v.is_finite() && *v >= FLOOR_DBM));
        }
        // An empty row, a row after time went backwards, and a zero-length step.
        let mut h = Afterglow::default();
        h.set_ms(500);
        h.feed(&row(Vec::new()), t0);
        assert!(h.ghost().is_none());
        h.feed(&row(vec![-50.0]), t0 + Duration::from_secs(10));
        h.feed(&row(vec![-140.0]), t0);
        h.feed(&row(vec![-140.0]), t0);
        assert_eq!(h.ghost().unwrap()[0], -50.0, "no time passed, no fall");
    }
}
