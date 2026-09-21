//! A source for networks that are read by asking for a list now and then (POTA, FR-SPOT-08),
//! as opposed to the streams RBN, a DX cluster and PSK Reporter give.
//!
//! Shaped like [`crate::telnet::TelnetSource`]: each [`poll`] does a bounded amount of work, a
//! failure is *returned* so it can be shown against its network (FR-SPOT-09), and the source backs
//! off and tries again by itself.
//!
//! Two things are specific to it. **The request never runs on the polling thread**: the worker
//! serves every source, and a request can take seconds, so each one runs on its own short-lived
//! thread and [`poll`] only collects a finished answer. And **the network access is injected** as a
//! [`Fetcher`], which keeps this crate free of an HTTP or TLS dependency (the app supplies one) and
//! lets the timing, the backoff and the failure handling be tested without a network.
//!
//! [`poll`]: SpotSource::poll

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::telnet::{ConnState, Stats};
use crate::{Network, Parsed, SourceError, Spot, SpotSource};

/// Fetch a URL and return the body, reading no more than the given number of bytes. Failures are
/// worded for the operator.
pub type Fetcher = Arc<dyn Fn(&str, usize) -> Result<Vec<u8>, String> + Send + Sync>;

/// Turn a body into spots; the second argument is the current Unix time.
pub type Parser = fn(&[u8], u64) -> Result<Parsed, String>;

/// The shortest interval between requests the app allows, seconds. Only the network operator knows
/// what load is acceptable and none of these publishes a figure, so the floor is deliberately not
/// a busy one.
pub const MIN_INTERVAL_SECS: u64 = 30;
/// The longest, seconds.
pub const MAX_INTERVAL_SECS: u64 = 3600;
/// The default, seconds.
pub const DEFAULT_INTERVAL_SECS: u64 = 60;

/// A configured interval as a duration, kept inside [`MIN_INTERVAL_SECS`]–[`MAX_INTERVAL_SECS`].
pub fn clamp_interval(secs: u64) -> Duration {
    Duration::from_secs(secs.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS))
}

/// What to poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolledConfig {
    pub network: Network,
    pub url: String,
    /// Time between requests (use [`clamp_interval`]).
    pub interval: Duration,
    /// The most a reply may be, bytes.
    pub max_body: usize,
}

/// Timing other than the interval.
#[derive(Debug, Clone)]
pub struct PolledTiming {
    /// First wait after a failure; doubles up to `max_backoff`, resets after a good reply.
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// A request with no answer after this long is given up on (the fetcher has its own timeout;
    /// this is the backstop).
    pub deadline: Duration,
    /// The most spots taken from one reply after the window filter; the excess is shed and counted.
    pub max_spots_per_poll: usize,
}

impl Default for PolledTiming {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_secs(30),
            max_backoff: Duration::from_secs(600),
            deadline: Duration::from_secs(30),
            max_spots_per_poll: 500,
        }
    }
}

struct Job {
    rx: Receiver<Result<Vec<u8>, String>>,
    started: Instant,
}

/// A polled spot source.
pub struct PolledSource {
    cfg: PolledConfig,
    timing: PolledTiming,
    fetch: Fetcher,
    parse: Parser,
    job: Option<Job>,
    next_attempt: Instant,
    backoff: Duration,
    /// Whether the last completed request succeeded.
    ok: bool,
    window: Option<(u64, u64)>,
    stats: Stats,
    attempts: u64,
}

impl PolledSource {
    pub fn new(cfg: PolledConfig, fetch: Fetcher, parse: Parser) -> Self {
        Self::with_timing(cfg, fetch, parse, PolledTiming::default())
    }

    pub fn with_timing(
        cfg: PolledConfig,
        fetch: Fetcher,
        parse: Parser,
        timing: PolledTiming,
    ) -> Self {
        Self {
            backoff: timing.initial_backoff,
            cfg,
            timing,
            fetch,
            parse,
            job: None,
            next_attempt: Instant::now(),
            ok: false,
            window: None,
            stats: Stats::default(),
            attempts: 0,
        }
    }

    /// Keep only spots between `lo` and `hi` Hz (`None` = all).
    pub fn set_window(&mut self, window: Option<(u64, u64)>) {
        self.window = window;
    }

    pub fn state(&self) -> ConnState {
        if self.ok {
            ConnState::Connected
        } else {
            ConnState::Disconnected
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Requests started so far (including failed ones).
    pub fn attempts(&self) -> u64 {
        self.attempts
    }

    pub fn config(&self) -> &PolledConfig {
        &self.cfg
    }

    fn fail(&mut self, now: Instant, msg: String) -> SourceError {
        self.ok = false;
        self.next_attempt = now + self.backoff;
        self.backoff = (self.backoff * 2).min(self.timing.max_backoff);
        SourceError(msg)
    }

    fn start(&mut self, now: Instant) -> Result<(), SourceError> {
        self.attempts += 1;
        let (tx, rx) = mpsc::channel();
        let fetch = Arc::clone(&self.fetch);
        let url = self.cfg.url.clone();
        let max = self.cfg.max_body;
        let spawned = thread::Builder::new()
            .name("spot-fetch".into())
            .spawn(move || {
                // The receiver is gone if the source gave up or was replaced: nobody wants it.
                let _ = tx.send(fetch(&url, max));
            });
        match spawned {
            Ok(_) => {
                self.job = Some(Job { rx, started: now });
                Ok(())
            }
            Err(e) => Err(self.fail(now, format!("could not start a request: {e}"))),
        }
    }

    fn finish(
        &mut self,
        now: Instant,
        body: &[u8],
        sink: &mut dyn FnMut(Spot),
    ) -> Result<(), SourceError> {
        let unix_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let parsed = match (self.parse)(body, unix_now) {
            Ok(p) => p,
            Err(e) => return Err(self.fail(now, format!("unexpected reply: {e}"))),
        };
        self.stats.rejected += parsed.rejected;
        let mut taken = 0usize;
        for spot in parsed.spots {
            if let Some((lo, hi)) = self.window {
                if spot.freq_hz < lo || spot.freq_hz > hi {
                    self.stats.outside_window += 1;
                    continue;
                }
            }
            if taken >= self.timing.max_spots_per_poll {
                self.stats.shed += 1;
                continue;
            }
            taken += 1;
            self.stats.spots += 1;
            sink(spot);
        }
        self.stats.connects += 1;
        self.ok = true;
        self.backoff = self.timing.initial_backoff;
        self.next_attempt = now + self.cfg.interval;
        Ok(())
    }
}

impl SpotSource for PolledSource {
    fn network(&self) -> Network {
        self.cfg.network
    }

    fn poll(&mut self, sink: &mut dyn FnMut(Spot)) -> Result<(), SourceError> {
        let now = Instant::now();
        let Some(job) = self.job.take() else {
            if now < self.next_attempt {
                return Ok(());
            }
            return self.start(now);
        };
        match job.rx.try_recv() {
            Ok(Ok(body)) => self.finish(now, &body, sink),
            Ok(Err(e)) => Err(self.fail(now, e)),
            Err(TryRecvError::Empty) => {
                if now.saturating_duration_since(job.started) > self.timing.deadline {
                    let secs = self.timing.deadline.as_secs_f32().round();
                    return Err(self.fail(now, format!("no reply within {secs} s")));
                }
                self.job = Some(job);
                Ok(())
            }
            Err(TryRecvError::Disconnected) => {
                Err(self.fail(now, "the request stopped unexpectedly".into()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pota;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    fn body(spots: &[(&str, &str)]) -> Vec<u8> {
        let recs: Vec<String> = spots
            .iter()
            .map(|(call, khz)| {
                format!(
                    r#"{{"activator":"{call}","frequency":"{khz}","mode":"CW","reference":"US-0001","spotTime":"2026-09-21T05:07:00","spotter":"BB2BBB","comments":"","invalid":null}}"#
                )
            })
            .collect();
        format!("[{}]", recs.join(",")).into_bytes()
    }

    fn cfg(interval_ms: u64) -> PolledConfig {
        PolledConfig {
            network: Network::Pota,
            url: "http://example.invalid/spot".into(),
            interval: Duration::from_millis(interval_ms),
            max_body: pota::MAX_BODY,
        }
    }

    fn timing() -> PolledTiming {
        PolledTiming {
            initial_backoff: Duration::from_millis(40),
            max_backoff: Duration::from_millis(160),
            deadline: Duration::from_millis(150),
            max_spots_per_poll: 500,
        }
    }

    /// Poll until `done` says so or two seconds pass; return what was delivered and what failed.
    fn drive(
        src: &mut PolledSource,
        mut done: impl FnMut(&PolledSource, &[Spot], &[String]) -> bool,
    ) -> (Vec<Spot>, Vec<String>) {
        let (mut spots, mut errs) = (Vec::new(), Vec::new());
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(2) {
            match src.poll(&mut |s| spots.push(s)) {
                Ok(()) => {}
                Err(e) => errs.push(e.0),
            }
            if done(src, &spots, &errs) {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        (spots, errs)
    }

    /// FR-SPOT-08: the source asks once, delivers what it got, waits the interval, and asks again;
    /// never more often, and never two requests at once.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_polled_source_repeats_on_its_interval() {
        let calls = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&calls);
        let fetch: Fetcher = Arc::new(move |_, _| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(body(&[("aa1aaa", "14074.0")]))
        });
        let mut src = PolledSource::with_timing(cfg(300), fetch, pota::parse_spots, timing());
        assert_eq!(src.state(), ConnState::Disconnected, "nothing yet");
        let (spots, errs) = drive(&mut src, |_, s, _| !s.is_empty());
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(spots[0].call, "AA1AAA");
        assert_eq!(spots[0].network, Network::Pota);
        assert_eq!(src.state(), ConnState::Connected);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // Polling hard for most of an interval makes no second request...
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(200) {
            assert!(src.poll(&mut |_| {}).is_ok());
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "not before the interval");
        // ...and after it, exactly one more.
        let (more, _) = drive(&mut src, |_, s, _| !s.is_empty());
        assert_eq!(more.len(), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(src.attempts(), 2);
        assert_eq!(src.stats().spots, 2);
        assert_eq!(src.stats().connects, 2);
    }

    /// FR-SPOT-08: `poll` returns at once while a request is slow — the worker serves other
    /// sources — and the answer arrives on a later poll.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_a_slow_request_does_not_block_poll() {
        let fetch: Fetcher = Arc::new(|_, _| {
            thread::sleep(Duration::from_millis(80));
            Ok(body(&[("aa1aaa", "14074.0")]))
        });
        let mut src = PolledSource::with_timing(cfg(10_000), fetch, pota::parse_spots, timing());
        let t = Instant::now();
        for _ in 0..20 {
            assert!(src.poll(&mut |_| {}).is_ok());
        }
        assert!(
            t.elapsed() < Duration::from_millis(60),
            "20 polls took {:?} with an 80 ms request outstanding",
            t.elapsed()
        );
        assert_eq!(src.attempts(), 1, "one request in flight, not twenty");
        let (spots, _) = drive(&mut src, |_, s, _| !s.is_empty());
        assert_eq!(spots.len(), 1);
    }

    /// FR-SPOT-08: a failed request is reported, backs off with doubling up to a cap, and the
    /// source recovers on the first good answer and starts its backoff over; a reply that is not
    /// the expected format is an error, not silence.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_failures_back_off_and_recover() {
        let good = body(&[("aa1aaa", "14074.0")]);
        let script = Arc::new(Mutex::new(vec![
            Err("HTTP 503".to_string()),
            Ok(b"<html>maintenance</html>".to_vec()),
            Err("connection refused".to_string()),
            Err("connection refused".to_string()),
            Ok(good.clone()),
            Err("HTTP 500".to_string()),
            Ok(good),
        ]));
        let s = Arc::clone(&script);
        let stamps = Arc::new(Mutex::new(Vec::<Instant>::new()));
        let st = Arc::clone(&stamps);
        let fetch: Fetcher = Arc::new(move |_, _| {
            st.lock().unwrap().push(Instant::now());
            let mut s = s.lock().unwrap();
            if s.is_empty() {
                Err("script ran out".into())
            } else {
                s.remove(0)
            }
        });
        let mut src = PolledSource::with_timing(cfg(30), fetch, pota::parse_spots, timing());
        let (spots, errs) = drive(&mut src, |_, sp, _| sp.len() >= 2);
        assert_eq!(spots.len(), 2, "recovered twice: {errs:?}");
        assert_eq!(errs.len(), 5, "{errs:?}");
        assert_eq!(errs[0], "HTTP 503");
        assert!(errs[1].starts_with("unexpected reply:"), "{}", errs[1]);
        assert_eq!(errs[2], "connection refused");
        assert_eq!(errs[4], "HTTP 500");
        // The gaps between requests: the backoff 40, 80, 160, then held at the 160 cap; the
        // 30 ms interval after a good reply; and the backoff back at 40 after the next failure.
        let t = stamps.lock().unwrap().clone();
        let gaps: Vec<u128> = t.windows(2).map(|w| (w[1] - w[0]).as_millis()).collect();
        assert!(gaps.len() >= 6, "{gaps:?}");
        let (floor, ceil) = (
            [40u128, 80, 160, 160, 30, 40],
            [120u128, 160, 260, 260, 100, 120],
        );
        for i in 0..6 {
            assert!(
                (floor[i]..ceil[i]).contains(&gaps[i]),
                "gap {i} was {} ms, wanted {}..{}: {gaps:?}",
                gaps[i],
                floor[i],
                ceil[i]
            );
        }
    }

    /// FR-SPOT-08: a request that never answers is given up on after the deadline, and a late
    /// answer to it is not delivered afterwards.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_a_hung_request_is_abandoned() {
        let n = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&n);
        let fetch: Fetcher = Arc::new(move |_, _| {
            if c.fetch_add(1, Ordering::SeqCst) == 0 {
                thread::sleep(Duration::from_millis(400));
                Ok(body(&[("zz9zzz", "14074.0")])) // arrives long after it was given up on
            } else {
                Ok(body(&[("aa1aaa", "14074.0")]))
            }
        });
        let mut src = PolledSource::with_timing(cfg(10_000), fetch, pota::parse_spots, timing());
        let (spots, errs) = drive(&mut src, |_, sp, _| !sp.is_empty());
        assert!(errs[0].starts_with("no reply within"), "{errs:?}");
        assert_eq!(spots.len(), 1);
        assert_eq!(
            spots[0].call, "AA1AAA",
            "the abandoned request's answer was used"
        );
        thread::sleep(Duration::from_millis(350));
        let mut late = Vec::new();
        src.poll(&mut |s| late.push(s)).unwrap();
        assert!(late.is_empty());
    }

    /// FR-SPOT-08: only spots inside the window are kept, the rest counted; the per-reply cap
    /// sheds the excess and says so; bad records are counted.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_window_and_caps() {
        let recs: Vec<(String, String)> = (0..8)
            .map(|i| (format!("aa{i}aaa"), format!("{}.0", 14_000 + i * 10)))
            .collect();
        let mut all: Vec<(&str, &str)> = recs.iter().map(|(a, b)| (&a[..], &b[..])).collect();
        all.push(("bb1bbb", "7030.0")); // outside the window
        all.push(("CQ", "14100.0")); // not a callsign
        let b = body(&all);
        let fetch: Fetcher = Arc::new(move |_, _| Ok(b.clone()));
        let mut t = timing();
        t.max_spots_per_poll = 5;
        let mut src = PolledSource::with_timing(cfg(10_000), fetch, pota::parse_spots, t);
        src.set_window(Some((14_000_000, 14_100_000)));
        let (spots, errs) = drive(&mut src, |s, _, _| s.stats().connects == 1);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(spots.len(), 5, "capped");
        let st = src.stats();
        assert_eq!(st.spots, 5);
        assert_eq!(st.shed, 3, "8 in the window, 5 kept");
        assert_eq!(st.outside_window, 1);
        assert_eq!(st.rejected, 1);
    }

    /// FR-SPOT-08: a configured interval stays inside the bounds, so a setting cannot make the app
    /// hammer a network or stop polling.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_interval_is_clamped() {
        assert_eq!(clamp_interval(0), Duration::from_secs(MIN_INTERVAL_SECS));
        assert_eq!(clamp_interval(1), Duration::from_secs(MIN_INTERVAL_SECS));
        assert_eq!(clamp_interval(30), Duration::from_secs(30));
        assert_eq!(clamp_interval(60), Duration::from_secs(60));
        assert_eq!(clamp_interval(3600), Duration::from_secs(3600));
        assert_eq!(clamp_interval(3601), Duration::from_secs(MAX_INTERVAL_SECS));
        assert_eq!(
            clamp_interval(u64::MAX),
            Duration::from_secs(MAX_INTERVAL_SECS)
        );
        const { assert!(MIN_INTERVAL_SECS <= DEFAULT_INTERVAL_SECS) };
        const { assert!(DEFAULT_INTERVAL_SECS <= MAX_INTERVAL_SECS) };
    }
}
