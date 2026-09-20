//! A telnet spot source (FR-SPOT-07): connects to a DX cluster or the Reverse Beacon Network,
//! logs in with a callsign, and delivers the spots it streams.
//!
//! Built on the codec in [`crate::cluster`] and the [`SpotSource`] interface: each [`poll`] does a
//! bounded amount of work — one connection attempt, or one bounded read — so the worker thread that
//! calls it stays responsive. Everything from the network is untrusted and bounded (see
//! [`crate::cluster`]); a failure is *returned*, not swallowed, so it can be shown against its
//! network (FR-SPOT-09), and the source reconnects by itself with backoff.
//!
//! [`poll`]: SpotSource::poll

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::cluster::{is_call_prompt, parse_line, LineSplitter, RateGate};
use crate::{normalise_callsign, Network, SourceError, Spot, SpotSource};

/// What to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelnetConfig {
    /// Which network the spots are attributed to.
    pub network: Network,
    pub host: String,
    pub port: u16,
    /// The callsign to log in with. A cluster requires one; it is the only thing sent.
    pub login: String,
}

/// Timeouts and limits. The defaults suit a real feed; tests shorten them.
#[derive(Debug, Clone)]
pub struct Timing {
    /// Per address, when connecting.
    pub connect_timeout: Duration,
    /// How long to wait for the login prompt after connecting.
    pub prompt_timeout: Duration,
    /// First wait after a failure; doubles up to `max_backoff`, resets once logged in.
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// A logged-in connection that delivers no bytes for this long is treated as dead.
    pub idle_timeout: Duration,
    /// The most one `poll` reads for, so it returns promptly.
    pub read_budget: Duration,
    /// The most one `poll` reads, bytes.
    pub max_bytes_per_poll: usize,
    /// Spots delivered per second at most; the excess is shed and counted.
    pub rate_per_sec: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(5),
            prompt_timeout: Duration::from_secs(15),
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(300),
            read_budget: Duration::from_millis(100),
            max_bytes_per_poll: 256 * 1024,
            rate_per_sec: 300,
        }
    }
}

/// Where the connection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    /// Not connected: never tried, failed, or waiting to retry.
    Disconnected,
    /// Connected, waiting for the login prompt.
    AwaitingLogin,
    /// Logged in and receiving spots.
    Connected,
}

/// Running counts, for showing what a source is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Spots delivered to the sink.
    pub spots: u64,
    /// `DX de` lines that were not well-formed spots.
    pub rejected: u64,
    /// Well-formed spots outside the wanted frequency window.
    pub outside_window: u64,
    /// Spots shed for exceeding the rate.
    pub shed: u64,
    /// Lines discarded for being too long.
    pub overlong: u64,
    /// Connections that reached the login prompt.
    pub connects: u64,
}

enum Phase {
    AwaitPrompt,
    Streaming,
}

struct Conn {
    stream: TcpStream,
    splitter: LineSplitter,
    gate: RateGate,
    phase: Phase,
    since: Instant,
    last_data: Instant,
}

/// A telnet cluster / RBN source.
pub struct TelnetSource {
    cfg: TelnetConfig,
    timing: Timing,
    conn: Option<Conn>,
    next_attempt: Instant,
    backoff: Duration,
    /// Only spots inside `[lo, hi]` Hz are kept; `None` keeps everything. Applied before the rate
    /// gate, so an unfiltered relay's other bands cannot use up the budget.
    window: Option<(u64, u64)>,
    stats: Stats,
    attempts: u64,
    epoch: Instant,
}

impl TelnetSource {
    pub fn new(cfg: TelnetConfig) -> Self {
        Self::with_timing(cfg, Timing::default())
    }

    pub fn with_timing(cfg: TelnetConfig, timing: Timing) -> Self {
        let now = Instant::now();
        Self {
            backoff: timing.initial_backoff,
            cfg,
            timing,
            conn: None,
            next_attempt: now,
            window: None,
            stats: Stats::default(),
            attempts: 0,
            epoch: now,
        }
    }

    /// Keep only spots between `lo` and `hi` Hz (`None` = all).
    pub fn set_window(&mut self, window: Option<(u64, u64)>) {
        self.window = window;
    }

    pub fn state(&self) -> ConnState {
        match self.conn.as_ref().map(|c| &c.phase) {
            None => ConnState::Disconnected,
            Some(Phase::AwaitPrompt) => ConnState::AwaitingLogin,
            Some(Phase::Streaming) => ConnState::Connected,
        }
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Connection attempts made so far (including failed ones).
    pub fn attempts(&self) -> u64 {
        self.attempts
    }

    pub fn config(&self) -> &TelnetConfig {
        &self.cfg
    }

    /// Record a failure: drop the connection and wait before trying again.
    fn fail(&mut self, now: Instant, msg: String) -> SourceError {
        self.conn = None;
        self.next_attempt = now + self.backoff;
        self.backoff = (self.backoff * 2).min(self.timing.max_backoff);
        SourceError(msg)
    }

    fn try_connect(&mut self, now: Instant) -> Result<(), SourceError> {
        // A cluster needs a callsign, and it is the only thing we send: check it before touching
        // the network, and do not keep retrying until the settings change.
        let Some(login) = normalise_callsign(&self.cfg.login) else {
            self.next_attempt = now + self.timing.max_backoff;
            return Err(SourceError(if self.cfg.login.trim().is_empty() {
                "a login callsign is required".into()
            } else {
                "the login is not a valid callsign".into()
            }));
        };
        if self.cfg.host.trim().is_empty() {
            self.next_attempt = now + self.timing.max_backoff;
            return Err(SourceError("no host is set".into()));
        }
        self.attempts += 1;
        let target = (self.cfg.host.trim(), self.cfg.port);
        let addrs: Vec<SocketAddr> = match target.to_socket_addrs() {
            Ok(a) => a.collect(),
            Err(e) => return Err(self.fail(now, format!("cannot resolve {}: {e}", self.cfg.host))),
        };
        // A host can have several addresses (IPv6 and IPv4); try each before giving up.
        let mut last = None;
        for addr in addrs {
            match TcpStream::connect_timeout(&addr, self.timing.connect_timeout) {
                Ok(stream) => {
                    let _ = stream.set_nodelay(true);
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                    if stream
                        .set_read_timeout(Some(Duration::from_millis(20)))
                        .is_err()
                    {
                        continue;
                    }
                    self.stats.connects += 1;
                    self.conn = Some(Conn {
                        stream,
                        splitter: LineSplitter::new(),
                        gate: RateGate::new(self.timing.rate_per_sec),
                        phase: Phase::AwaitPrompt,
                        since: now,
                        last_data: now,
                    });
                    // Keep the validated login for the prompt: store it normalised.
                    self.cfg.login = login;
                    return Ok(());
                }
                Err(e) => last = Some(e),
            }
        }
        let why = last.map_or_else(|| "no address to connect to".to_string(), |e| e.to_string());
        Err(self.fail(
            now,
            format!(
                "connect to {}:{} failed: {why}",
                self.cfg.host, self.cfg.port
            ),
        ))
    }

    fn service(&mut self, now: Instant, sink: &mut dyn FnMut(Spot)) -> Result<(), SourceError> {
        let unix_now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let now_ms = self.epoch.elapsed().as_millis() as u64;
        let timing = self.timing.clone();
        let window = self.window;
        let network = self.cfg.network;
        let login = self.cfg.login.clone();
        let mut lines = Vec::new();
        let mut reset_backoff = false;

        {
            let conn = self
                .conn
                .as_mut()
                .expect("service is only called when connected");
            let deadline = Instant::now() + timing.read_budget;
            let mut buf = [0u8; 8192];
            let mut total = 0usize;
            loop {
                match conn.stream.read(&mut buf) {
                    Ok(0) => return Err(SourceError("the server closed the connection".into())),
                    Ok(n) => {
                        total += n;
                        conn.last_data = Instant::now();
                        conn.splitter.push(&buf[..n], &mut lines);
                    }
                    Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                        break
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(e) => return Err(SourceError(format!("read failed: {e}"))),
                }
                if total >= timing.max_bytes_per_poll || Instant::now() >= deadline {
                    break;
                }
            }
            self.stats.overlong = conn.splitter.overlong;

            match conn.phase {
                Phase::AwaitPrompt => {
                    let prompted = is_call_prompt(&conn.splitter.pending())
                        || lines.iter().any(|l| is_call_prompt(l));
                    if prompted {
                        conn.stream
                            .write_all(format!("{login}\r\n").as_bytes())
                            .map_err(|e| SourceError(format!("could not send the login: {e}")))?;
                        // The prompt has no newline, so it is still the unfinished line: drop it,
                        // or the server's first line is glued onto it and lost.
                        conn.splitter.discard_pending();
                        conn.phase = Phase::Streaming;
                        reset_backoff = true;
                        lines.clear(); // everything so far was banner and the prompt itself
                    } else if now.saturating_duration_since(conn.since) > timing.prompt_timeout {
                        return Err(SourceError(format!(
                            "no login prompt within {} s",
                            timing.prompt_timeout.as_secs_f32().round()
                        )));
                    }
                }
                Phase::Streaming => {
                    if now.saturating_duration_since(conn.last_data) > timing.idle_timeout {
                        return Err(SourceError(format!(
                            "no data for {} s",
                            timing.idle_timeout.as_secs()
                        )));
                    }
                }
            }
        }
        if reset_backoff {
            self.backoff = timing.initial_backoff;
        }

        // Turn lines into spots (only once logged in).
        if matches!(self.conn.as_ref().map(|c| &c.phase), Some(Phase::Streaming)) {
            for line in lines {
                match parse_line(&line, unix_now, network) {
                    None => {
                        if line.starts_with("DX de ") {
                            self.stats.rejected += 1;
                        }
                    }
                    Some(spot) => {
                        if let Some((lo, hi)) = window {
                            if spot.freq_hz < lo || spot.freq_hz > hi {
                                self.stats.outside_window += 1;
                                continue;
                            }
                        }
                        let conn = self.conn.as_mut().expect("still connected");
                        if !conn.gate.allow(now_ms) {
                            self.stats.shed = conn.gate.shed;
                            continue;
                        }
                        self.stats.spots += 1;
                        sink(spot);
                    }
                }
            }
            if let Some(conn) = self.conn.as_ref() {
                self.stats.shed = conn.gate.shed;
            }
        }
        Ok(())
    }
}

impl SpotSource for TelnetSource {
    fn network(&self) -> Network {
        self.cfg.network
    }

    fn poll(&mut self, sink: &mut dyn FnMut(Spot)) -> Result<(), SourceError> {
        let now = Instant::now();
        if self.conn.is_none() {
            if now < self.next_attempt {
                return Ok(()); // waiting out a backoff: quiet, not an error every call
            }
            return self.try_connect(now);
        }
        match self.service(now, sink) {
            Ok(()) => Ok(()),
            Err(e) => Err(self.fail(now, e.0)),
        }
    }
}
