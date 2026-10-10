//! The HamQSL forecast source (FR-UI-25): fetches `solarxml.php` now and then, off the polling
//! thread, and keeps the last good forecast.
//!
//! The same mechanics as [`crate::polled`] — the request runs on its own short-lived thread through
//! an injected [`Fetcher`], `poll` only collects a finished answer, a failure backs off — with two
//! differences the plan (§6) requires. **Its interval is HamQSL's**: no more than hourly
//! ([`bandcond::HAMQSL_INTERVAL_MIN_SECS`]). And **it keeps its schedule across a reconfigure**:
//! the source is made once and told about setting changes, so ticking a box or nudging the interval
//! never fetches inside the hour. Time is passed in, so the schedule is tested without a clock.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::bandcond::{self, Forecast};
use crate::polled::Fetcher;

/// The feed.
pub const URL: &str = "https://www.hamqsl.com/solarxml.php";

/// Timing other than the interval.
#[derive(Debug, Clone)]
pub struct HamqslTiming {
    /// First wait after a failure; doubles, capped at the interval. Long, since a server that is
    /// answering errors should not be asked every minute.
    pub initial_backoff: Duration,
    /// A request with no answer after this long is given up on (the fetcher has its own timeout).
    pub deadline: Duration,
}

impl Default for HamqslTiming {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_secs(300),
            deadline: Duration::from_secs(30),
        }
    }
}

struct Job {
    rx: Receiver<Result<Vec<u8>, String>>,
    started: Instant,
}

/// The HamQSL source. Made once; [`set_interval`](Self::set_interval) and
/// [`set_enabled`](Self::set_enabled) change it without losing its schedule or its last forecast.
pub struct HamqslSource {
    fetch: Fetcher,
    timing: HamqslTiming,
    interval: Duration,
    enabled: bool,
    job: Option<Job>,
    next_attempt: Instant,
    backoff: Duration,
    /// When the last successful request was started, and the Unix time it finished.
    last_ok: Option<(Instant, u64)>,
    forecast: Option<Forecast>,
    error: Option<String>,
    attempts: u64,
}

impl HamqslSource {
    pub fn new(fetch: Fetcher, interval_secs: u64, now: Instant) -> Self {
        Self::with_timing(fetch, interval_secs, HamqslTiming::default(), now)
    }

    /// The first `poll` while enabled fetches at once.
    pub fn with_timing(
        fetch: Fetcher,
        interval_secs: u64,
        timing: HamqslTiming,
        now: Instant,
    ) -> Self {
        Self {
            fetch,
            backoff: timing.initial_backoff,
            timing,
            interval: Duration::from_secs(bandcond::hamqsl_interval(interval_secs)),
            enabled: true,
            job: None,
            next_attempt: now,
            last_ok: None,
            forecast: None,
            error: None,
            attempts: 0,
        }
    }

    /// A new interval (clamped to HamQSL's bounds). After a good fetch, the next one is due one
    /// new interval after it — never sooner than the hourly floor allows.
    pub fn set_interval(&mut self, secs: u64) {
        self.interval = Duration::from_secs(bandcond::hamqsl_interval(secs));
        if let (Some((at, _)), None) = (self.last_ok, &self.error) {
            self.next_attempt = at + self.interval;
        }
    }

    /// Switch fetching on or off. Off drops an unfinished request and fetches nothing; on again
    /// keeps the schedule, so the last forecast is reused rather than refetched inside the hour.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.job = None;
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// The interval in force, seconds.
    pub fn interval_secs(&self) -> u64 {
        self.interval.as_secs()
    }

    /// Collect a finished request or start a due one. Never blocks.
    pub fn poll(&mut self, now: Instant, unix_now: u64) {
        if !self.enabled {
            return;
        }
        let Some(job) = self.job.take() else {
            if now >= self.next_attempt {
                self.start(now);
            }
            return;
        };
        match job.rx.try_recv() {
            Ok(Ok(body)) => match bandcond::parse_hamqsl(&body) {
                Ok(f) => {
                    self.forecast = Some(f);
                    self.last_ok = Some((job.started, unix_now));
                    self.error = None;
                    self.backoff = self.timing.initial_backoff;
                    self.next_attempt = job.started + self.interval;
                }
                Err(e) => self.fail(now, format!("unexpected reply: {e}")),
            },
            Ok(Err(e)) => self.fail(now, e),
            Err(TryRecvError::Empty) => {
                if now.saturating_duration_since(job.started) > self.timing.deadline {
                    let secs = self.timing.deadline.as_secs();
                    self.fail(now, format!("no reply within {secs} s"));
                } else {
                    self.job = Some(job);
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.fail(now, "the request ended without a reply".into())
            }
        }
    }

    fn start(&mut self, now: Instant) {
        self.attempts += 1;
        // A started request counts against HamQSL's rate even if it is later dropped (switched off
        // mid-flight): the next may not start before the back-off floor. Completion reschedules.
        self.next_attempt = now + self.timing.initial_backoff;
        let (tx, rx) = mpsc::channel();
        let fetch = Arc::clone(&self.fetch);
        let spawned = thread::Builder::new()
            .name("hamqsl-fetch".into())
            .spawn(move || {
                let _ = tx.send(fetch(URL, bandcond::MAX_BODY));
            });
        match spawned {
            Ok(_) => self.job = Some(Job { rx, started: now }),
            Err(e) => self.fail(now, format!("could not start a request: {e}")),
        }
    }

    fn fail(&mut self, now: Instant, msg: String) {
        self.error = Some(msg);
        self.next_attempt = now + self.backoff;
        self.backoff = (self.backoff * 2).min(self.interval);
    }

    /// The forecast if it still counts at `unix_now` (both freshness limits), else `None`.
    pub fn current(&self, unix_now: u64) -> Option<&Forecast> {
        let (_, ok_unix) = self.last_ok?;
        let f = self.forecast.as_ref()?;
        bandcond::forecast_current(ok_unix, f.updated, unix_now, self.interval.as_secs())
            .then_some(f)
    }

    /// Whether a forecast was fetched but no longer counts at `unix` — the feed's own data is
    /// older than its update cycle, or fetches have been failing for two intervals.
    pub fn stale(&self, unix_now: u64) -> bool {
        self.last_ok.is_some() && self.current(unix_now).is_none()
    }

    /// The last failure, kept until a request succeeds — for the PROPAGATION tab's status line.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// When the next request is due.
    pub fn next_attempt(&self) -> Instant {
        self.next_attempt
    }

    /// Requests started so far.
    pub fn attempts(&self) -> u64 {
        self.attempts
    }

    /// Whether a request is in flight.
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const FIXTURE: &[u8] = include_bytes!("../tests/fixtures/hamqsl-2026-10-04.xml");
    const UPDATED: u64 = 1_791_131_160;
    const H: Duration = Duration::from_secs(3600);

    fn counting(ok: bool) -> (Fetcher, Arc<AtomicU64>) {
        let n = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&n);
        let f: Fetcher = Arc::new(move |url, max| {
            c.fetch_add(1, Ordering::SeqCst);
            assert_eq!((url, max), (URL, bandcond::MAX_BODY));
            if ok {
                Ok(FIXTURE.to_vec())
            } else {
                Err("network unreachable".into())
            }
        });
        (f, n)
    }

    /// Poll until the in-flight request finishes (the fetcher runs on its own thread).
    fn settle(s: &mut HamqslSource, now: Instant, unix: u64) {
        for _ in 0..500 {
            s.poll(now, unix);
            if !s.busy() {
                return;
            }
            thread::sleep(Duration::from_millis(2));
        }
        panic!("request never finished");
    }

    /// FR-UI-25: the first poll fetches; the next fetch waits a full interval; a reconfigure —
    /// interval change, off and on — never fetches inside the hour; the interval never goes below
    /// HamQSL's hourly floor.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_hamqsl_keeps_its_schedule() {
        let (fetch, n) = counting(true);
        let t0 = Instant::now();
        let mut s = HamqslSource::new(fetch, 60, t0);
        assert_eq!(
            s.interval_secs(),
            3600,
            "60 s asked for, the hourly floor kept"
        );
        s.poll(t0, UPDATED);
        settle(&mut s, t0, UPDATED);
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert!(s.current(UPDATED).is_some());

        let half = t0 + H / 2;
        s.set_interval(7200);
        s.set_enabled(false);
        s.poll(half, UPDATED);
        s.set_enabled(true);
        s.set_interval(3600);
        s.poll(half, UPDATED);
        assert!(
            !s.busy() && n.load(Ordering::SeqCst) == 1,
            "no fetch inside the hour"
        );
        assert!(
            s.current(UPDATED + 1800).is_some(),
            "the last forecast reused"
        );

        s.set_interval(7200);
        s.poll(t0 + H + Duration::from_secs(1), UPDATED);
        assert!(
            !s.busy(),
            "the new two-hour interval runs from the last fetch"
        );
        s.poll(t0 + 2 * H, UPDATED);
        settle(&mut s, t0 + 2 * H, UPDATED);
        assert_eq!(n.load(Ordering::SeqCst), 2);
    }

    /// FR-UI-25: offline — every request fails — never blocks, never raises past the status line,
    /// backs off with growing gaps capped at the interval, and leaves no forecast.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_hamqsl_offline_backs_off() {
        let (fetch, n) = counting(false);
        let t0 = Instant::now();
        let mut s = HamqslSource::new(fetch, 3600, t0);
        let mut gaps = Vec::new();
        let mut now = t0;
        for _ in 0..6 {
            let before = Instant::now();
            s.poll(now, UPDATED);
            settle(&mut s, now, UPDATED);
            assert!(
                before.elapsed() < Duration::from_millis(900),
                "poll never blocks"
            );
            gaps.push(s.next_attempt().saturating_duration_since(now).as_secs());
            now = s.next_attempt();
        }
        assert_eq!(n.load(Ordering::SeqCst), 6);
        assert_eq!(
            gaps,
            [300, 600, 1200, 2400, 3600, 3600],
            "doubling, capped at the interval"
        );
        assert_eq!(s.error(), Some("network unreachable"));
        assert!(s.current(UPDATED).is_none(), "nothing to colour with");
        s.set_enabled(false);
        s.poll(now, UPDATED); // due now, but off
                              // The fetch runs on its own thread, so the counter alone could lag a request just started:
                              // check that none was started, and give a stray one time to show.
        assert!(!s.busy(), "off starts no request");
        thread::sleep(Duration::from_millis(50));
        assert_eq!(n.load(Ordering::SeqCst), 6, "off fetches nothing");
    }

    /// FR-UI-25: switching HamQSL off and on while a request is in flight does not send a second
    /// request at once (the dropped one still reached the server): the next waits at least the
    /// back-off floor.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_hamqsl_off_and_on_mid_request_does_not_refetch_at_once() {
        let n = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&n);
        let fetch: Fetcher = Arc::new(move |_, _| {
            c.fetch_add(1, Ordering::SeqCst);
            thread::sleep(Duration::from_millis(300)); // still in flight when toggled
            Ok(FIXTURE.to_vec())
        });
        let t0 = Instant::now();
        let mut s = HamqslSource::new(fetch, 3600, t0);
        s.poll(t0, UPDATED);
        assert!(s.busy(), "the first request is in flight");
        s.set_enabled(false);
        s.set_enabled(true);
        s.poll(t0 + Duration::from_secs(1), UPDATED);
        assert!(!s.busy(), "no second request at once");
        assert_eq!(s.attempts(), 1);
        s.poll(t0 + Duration::from_secs(301), UPDATED);
        assert!(
            s.busy() && s.attempts() == 2,
            "after the back-off floor it may ask again"
        );
    }

    /// FR-UI-25: a forecast stops counting when fetches keep failing past two intervals.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_hamqsl_forecast_expires_after_failures() {
        let ok = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let o = Arc::clone(&ok);
        let fetch: Fetcher = Arc::new(move |_, _| {
            if o.load(Ordering::SeqCst) {
                Ok(FIXTURE.to_vec())
            } else {
                Err("down".into())
            }
        });
        let t0 = Instant::now();
        let mut s = HamqslSource::new(fetch, 3600, t0);
        settle(&mut s, t0, UPDATED);
        ok.store(false, Ordering::SeqCst);
        settle(&mut s, t0 + H, UPDATED + 3600);
        assert!(s.error().is_some());
        assert!(
            s.current(UPDATED + 7200).is_some(),
            "one missed fetch is tolerated"
        );
        assert!(
            s.current(UPDATED + 7201).is_none(),
            "two intervals without a good fetch"
        );
        assert!(
            s.stale(UPDATED + 7201) && !s.stale(UPDATED + 7200),
            "and is reported stale"
        );
    }
}
