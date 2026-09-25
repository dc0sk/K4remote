//! A FreeDV Reporter spot source (FR-SPOT-08): connects to `qso.freedv.org` over a WebSocket —
//! plain `ws`, or `wss` through the same encrypted connector and certificate approval as PSK
//! Reporter (FR-SPOT-13) — joins in the **read-only `view` role**, keeps a roster of who is on
//! which frequency and delivers them as spots.
//!
//! Built on [`crate::ws`] (the WebSocket), [`crate::sio`] (Engine.IO / Socket.IO packets),
//! [`crate::freedv`] (the roster) and the [`SpotSource`] interface, and shaped like
//! [`crate::mqtt_source::MqttSource`]: each [`poll`] does a bounded amount of work, a failure is
//! *returned* so it can be shown against its network (FR-SPOT-09), and the source reconnects by
//! itself with backoff.
//!
//! **Receive-only, and anonymous.** The only thing sent beyond the upgrade request is the `view`
//! connect (see [`crate::sio::connect_view`]) and the replies the protocol requires — a pong to each
//! ping. Nothing identifies the operator, and the `report` role that would make a station publicly
//! visible is not reachable from here (`FR-SPOT-12`).
//!
//! **Presence.** The reporter says who is on the air *now*, not when they last did something. A
//! station's spot is stamped when its event arrives and again on a periodic refresh (configurable,
//! 30 s to 5 min, once a minute by default) while it stays on the roster, so a station that has left stops being refreshed and fades with age like any
//! other spot, instead of every idle station disappearing after the age limit.
//!
//! [`poll`]: SpotSource::poll

use std::io::{ErrorKind, Read, Write};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::cluster::RateGate;
use crate::freedv::Roster;
use crate::mqtt_source::{plain_connector, CertInfo, ConnectError, Connector, Wire};
use crate::sio::{self, Packet};
use crate::telnet::{ConnState, Stats, Timing};
use crate::ws::{self, FrameReader, Message, Rng};
use crate::{Network, SourceError, Spot, SpotSource};

/// The path of the Socket.IO endpoint on a WebSocket (Engine.IO revision 4).
pub const PATH: &str = "/socket.io/?EIO=4&transport=websocket";

/// How often the spots of stations still on the roster are re-stamped, seconds: the default and the
/// bounds a configured value is kept inside (the default was 30 s until DC0SK found it too fast,
/// 2026-09-24).
pub const DEFAULT_REFRESH_SECS: u64 = 60;
pub const MIN_REFRESH_SECS: u64 = 30;
pub const MAX_REFRESH_SECS: u64 = 300;

/// A configured refresh as a duration, kept inside [`MIN_REFRESH_SECS`]–[`MAX_REFRESH_SECS`].
pub fn clamp_refresh(secs: u64) -> Duration {
    Duration::from_secs(secs.clamp(MIN_REFRESH_SECS, MAX_REFRESH_SECS))
}

/// What to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeDvConfig {
    pub host: String,
    pub port: u16,
    /// Sent as the `User-Agent`: the program and its version, nothing about the operator.
    pub user_agent: String,
    /// Seconds between re-stamps of the stations still on the roster (see [`clamp_refresh`]).
    pub refresh_secs: u64,
    /// Connect over `wss` through the encrypted connector (see [`FreeDvSource::set_tls_connector`])
    /// instead of plain `ws`.
    pub tls: bool,
}

enum Phase {
    /// The upgrade request is sent; its reply is being read.
    Upgrade {
        key: String,
        buf: Vec<u8>,
    },
    /// Upgraded; waiting for Engine.IO's `open`.
    Open,
    /// `view` connect sent; waiting for Socket.IO's `connect`.
    Connect,
    Live,
}

struct Conn {
    stream: Box<dyn Wire>,
    phase: Phase,
    reader: FrameReader,
    gate: RateGate,
    rng: Rng,
    since: Instant,
    last_rx: Instant,
    /// The server's ping interval plus its timeout, once it has said (`open`).
    watchdog: Option<Duration>,
}

impl Conn {
    fn send(&mut self, opcode: u8, payload: &[u8]) -> Result<(), SourceError> {
        let mask = self.rng.mask();
        self.stream
            .write_all(&ws::encode(opcode, payload, mask))
            .map_err(|e| SourceError(format!("could not send to the server: {e}")))
    }
}

/// A FreeDV Reporter source.
pub struct FreeDvSource {
    cfg: FreeDvConfig,
    timing: Timing,
    refresh: Duration,
    connector: Connector,
    tls: Option<Connector>,
    /// The certificate the last attempt was refused for, while that is why it is not connected.
    pending_cert: Option<CertInfo>,
    conn: Option<Conn>,
    next_attempt: Instant,
    backoff: Duration,
    /// Only spots inside `[lo, hi]` Hz are kept; `None` keeps everything.
    window: Option<(u64, u64)>,
    roster: Roster,
    stats: Stats,
    /// Packets that were not valid once the session was running (a bad event costs only itself).
    packets_rejected: u64,
    attempts: u64,
    epoch: Instant,
    last_refresh: Instant,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl FreeDvSource {
    pub fn new(cfg: FreeDvConfig) -> Self {
        Self::with_timing(cfg, Timing::default())
    }

    pub fn with_timing(cfg: FreeDvConfig, timing: Timing) -> Self {
        let now = Instant::now();
        Self {
            backoff: timing.initial_backoff,
            refresh: clamp_refresh(cfg.refresh_secs),
            cfg,
            timing,
            connector: plain_connector(),
            tls: None,
            pending_cert: None,
            conn: None,
            next_attempt: now,
            window: None,
            roster: Roster::default(),
            stats: Stats::default(),
            packets_rejected: 0,
            attempts: 0,
            epoch: now,
            last_refresh: now,
        }
    }

    /// Change how often stations still on the roster are re-stamped, unclamped (tests).
    pub fn set_refresh(&mut self, refresh: Duration) {
        self.refresh = refresh;
    }

    /// How often stations still on the roster are re-stamped.
    pub fn refresh(&self) -> Duration {
        self.refresh
    }

    /// Replace the plain connector (tests).
    pub fn set_connector(&mut self, connector: Connector) {
        self.connector = connector;
    }

    /// The connector used when the configuration asks for TLS. Without one, asking for TLS is an
    /// error, never a silent fall back to plain text.
    pub fn set_tls_connector(&mut self, connector: Connector) {
        self.tls = Some(connector);
    }

    /// The certificate the last attempt was refused for, if that is why it is not connected.
    pub fn pending_cert(&self) -> Option<&CertInfo> {
        self.pending_cert.as_ref()
    }

    /// Try again at the next poll instead of waiting out the backoff — for when the operator has
    /// just approved a certificate.
    pub fn retry_now(&mut self) {
        self.next_attempt = Instant::now();
        self.backoff = self.timing.initial_backoff;
    }

    /// Keep only spots between `lo` and `hi` Hz (`None` = all).
    pub fn set_window(&mut self, window: Option<(u64, u64)>) {
        self.window = window;
    }

    pub fn state(&self) -> ConnState {
        match self.conn.as_ref().map(|c| &c.phase) {
            None => ConnState::Disconnected,
            Some(Phase::Live) => ConnState::Connected,
            Some(_) => ConnState::AwaitingLogin,
        }
    }

    pub fn stats(&self) -> Stats {
        Stats {
            rejected: self.stats.rejected + self.packets_rejected + self.roster.rejected,
            ..self.stats
        }
    }

    /// Per event name, how many were rejected and the shape of the last (field names and value
    /// kinds only), for diagnosing a format the source does not understand.
    pub fn rejected_shapes(&self) -> &std::collections::BTreeMap<String, (u64, String)> {
        self.roster.rejected_shapes()
    }

    /// Text fields dropped for being unusable (too long, or not printable ASCII) while the rest of
    /// their event applied: not an error, but counted so it can be seen.
    pub fn degraded(&self) -> u64 {
        self.roster.degraded
    }

    /// As [`rejected_shapes`](Self::rejected_shapes), for degraded fields.
    pub fn degraded_shapes(&self) -> &std::collections::BTreeMap<String, (u64, String)> {
        self.roster.degraded_shapes()
    }

    /// Stations on the roster now.
    pub fn stations(&self) -> usize {
        self.roster.len()
    }

    /// Connection attempts made so far (including failed ones).
    pub fn attempts(&self) -> u64 {
        self.attempts
    }

    pub fn config(&self) -> &FreeDvConfig {
        &self.cfg
    }

    fn fail(&mut self, now: Instant, msg: String) -> SourceError {
        self.conn = None;
        // A new session starts from the server's `bulk_update`; stations from this one that have
        // left meanwhile must not be refreshed into the next.
        self.roster.clear();
        self.next_attempt = now + self.backoff;
        self.backoff = (self.backoff * 2).min(self.timing.max_backoff);
        SourceError(msg)
    }

    fn try_connect(&mut self, now: Instant) -> Result<(), SourceError> {
        let host = self.cfg.host.trim().to_string();
        if host.is_empty() {
            self.next_attempt = now + self.timing.max_backoff;
            return Err(SourceError("no host is set".into()));
        }
        self.attempts += 1;
        let connector = if self.cfg.tls {
            match self.tls.clone() {
                Some(c) => c,
                None => {
                    self.next_attempt = now + self.timing.max_backoff;
                    return Err(SourceError(
                        "encrypted connections are not available".into(),
                    ));
                }
            }
        } else {
            self.connector.clone()
        };
        let stream = match connector(&host, self.cfg.port, self.timing.connect_timeout) {
            Ok(s) => s,
            Err(ConnectError::Failed(why)) => return Err(self.fail(now, why)),
            Err(ConnectError::Untrusted(info)) if self.cfg.tls => {
                let msg = format!(
                    "the server's certificate is not trusted ({}){}",
                    info.reason,
                    if info.changed {
                        " — and it is not the one you approved"
                    } else {
                        ""
                    }
                );
                self.pending_cert = Some(info);
                return Err(self.fail(now, msg));
            }
            Err(ConnectError::Untrusted(_)) => {
                return Err(self.fail(
                    now,
                    "an unexpected certificate on a plain connection".into(),
                ))
            }
        };
        self.pending_cert = None;
        let mut rng = Rng::seeded();
        let key = rng.key();
        let request = match ws::request(
            &host,
            self.cfg.port,
            self.cfg.tls,
            PATH,
            &key,
            &self.cfg.user_agent,
        ) {
            Ok(r) => r,
            // A setting that cannot make a request: no point retrying quickly.
            Err(why) => {
                self.next_attempt = now + self.timing.max_backoff;
                return Err(SourceError(format!("cannot connect: {why}")));
            }
        };
        let mut conn = Conn {
            stream,
            phase: Phase::Upgrade {
                key,
                buf: Vec::new(),
            },
            reader: FrameReader::new(),
            gate: RateGate::new(self.timing.rate_per_sec),
            rng,
            since: now,
            last_rx: now,
            watchdog: None,
        };
        if let Err(e) = conn.stream.write_all(&request) {
            return Err(self.fail(now, format!("could not send to {host}: {e}")));
        }
        self.conn = Some(conn);
        Ok(())
    }

    /// Hand a spot to the sink if it is inside the window and the rate allows.
    fn emit(&mut self, spot: Spot, now_ms: u64, sink: &mut dyn FnMut(Spot)) {
        if let Some((lo, hi)) = self.window {
            if spot.freq_hz < lo || spot.freq_hz > hi {
                self.stats.outside_window += 1;
                return;
            }
        }
        let allowed = self.conn.as_mut().is_none_or(|c| c.gate.allow(now_ms));
        if !allowed {
            self.stats.shed += 1;
            return;
        }
        self.stats.spots += 1;
        sink(spot);
    }

    fn handle(
        &mut self,
        msg: Message,
        now_ms: u64,
        unix: u64,
        connected: &mut bool,
        sink: &mut dyn FnMut(Spot),
    ) -> Result<(), SourceError> {
        let conn = self.conn.as_mut().expect("handled only while connected");
        match msg {
            Message::Close(_) => {
                return Err(SourceError("the server closed the connection".into()))
            }
            Message::Ping(p) => conn.send(ws::OP_PONG, &p)?,
            Message::Pong(_) | Message::Binary(_) => {}
            Message::Text(text) => {
                let live = matches!(conn.phase, Phase::Live);
                let packet = match sio::parse(&text) {
                    Ok(p) => p,
                    // Before the session is running, a packet we cannot read means it is not the
                    // service we expect; once it is running, a bad packet costs only itself.
                    Err(e) if !live => {
                        return Err(SourceError(format!("unexpected data from the server: {e}")))
                    }
                    Err(_) => {
                        self.packets_rejected += 1;
                        return Ok(());
                    }
                };
                match packet {
                    Packet::Open {
                        ping_interval_ms,
                        ping_timeout_ms,
                        ..
                    } if matches!(conn.phase, Phase::Open) => {
                        conn.watchdog =
                            Some(Duration::from_millis(ping_interval_ms + ping_timeout_ms));
                        let connect = sio::connect_view();
                        conn.send(ws::OP_TEXT, connect.as_bytes())?;
                        conn.phase = Phase::Connect;
                    }
                    Packet::Connect { .. } if matches!(conn.phase, Phase::Connect) => {
                        conn.phase = Phase::Live;
                        *connected = true;
                    }
                    Packet::Ping => conn.send(ws::OP_TEXT, sio::PONG.as_bytes())?,
                    Packet::Close | Packet::Disconnect => {
                        return Err(SourceError("the server ended the session".into()))
                    }
                    Packet::ConnectError => {
                        return Err(SourceError("the server refused the connection".into()))
                    }
                    Packet::Event { name, args } if live => {
                        let changed = self.roster.on_event(&name, &args);
                        for sid in changed {
                            if let Some(spot) = self.roster.spot(&sid, unix) {
                                self.emit(spot, now_ms, sink);
                            }
                        }
                    }
                    // Anything else — an event before the session is running, a second `open`, a
                    // pong, something this client does not use — is ignored.
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn service(&mut self, now: Instant, sink: &mut dyn FnMut(Spot)) -> Result<(), SourceError> {
        let unix = unix_now();
        let now_ms = self.epoch.elapsed().as_millis() as u64;
        let timing = self.timing.clone();
        let mut messages: Vec<Message> = Vec::new();

        {
            let conn = self
                .conn
                .as_mut()
                .expect("service runs only when connected");
            let deadline = Instant::now() + timing.read_budget;
            let mut buf = [0u8; 8192];
            let mut total = 0usize;
            loop {
                match conn.stream.read(&mut buf) {
                    Ok(0) => return Err(SourceError("the server closed the connection".into())),
                    Ok(n) => {
                        total += n;
                        conn.last_rx = Instant::now();
                        let bytes = &buf[..n];
                        match &mut conn.phase {
                            Phase::Upgrade { key, buf: head } => {
                                head.extend_from_slice(bytes);
                                match ws::check_response(head, key) {
                                    Ok(None) => {}
                                    Ok(Some(end)) => {
                                        let rest = head.split_off(end);
                                        conn.phase = Phase::Open;
                                        messages.extend(conn.reader.push(&rest).map_err(|e| {
                                            SourceError(format!("protocol error: {e}"))
                                        })?);
                                    }
                                    Err(e) => return Err(SourceError(e)),
                                }
                            }
                            _ => messages.extend(
                                conn.reader
                                    .push(bytes)
                                    .map_err(|e| SourceError(format!("protocol error: {e}")))?,
                            ),
                        }
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

            // Liveness: before the session runs, the connection must complete in time; after, the
            // server's own ping timing says how long silence may last.
            if let Some(watchdog) = conn.watchdog {
                if now.saturating_duration_since(conn.last_rx) > watchdog {
                    return Err(SourceError(format!(
                        "no ping from the server for {} s",
                        watchdog.as_secs()
                    )));
                }
            }
            if !matches!(conn.phase, Phase::Live)
                && now.saturating_duration_since(conn.since) > timing.prompt_timeout
            {
                let what = match conn.phase {
                    Phase::Upgrade { .. } => "the upgrade to a WebSocket",
                    Phase::Open => "the session to open",
                    _ => "the connection to be accepted",
                };
                return Err(SourceError(format!(
                    "no answer within {} s waiting for {what}",
                    timing.prompt_timeout.as_secs_f32().round()
                )));
            }
        }

        let mut connected_now = false;
        for msg in messages {
            self.handle(msg, now_ms, unix, &mut connected_now, sink)?;
        }
        if connected_now {
            self.backoff = timing.initial_backoff;
            self.stats.connects += 1;
            self.last_refresh = now;
        }

        // Re-stamp the stations that are still there, so a station that stays connected stays
        // on the overlay and one that has left does not.
        // (Nothing needs checking that the session is live: the roster is empty until it is, because
        // events before the acknowledgement are ignored and a lost connection clears it.)
        if now.saturating_duration_since(self.last_refresh) >= self.refresh {
            self.last_refresh = now;
            let sids: Vec<String> = self.roster.sids().cloned().collect();
            for sid in sids {
                if let Some(spot) = self.roster.spot(&sid, unix) {
                    self.emit(spot, now_ms, sink);
                }
            }
        }
        Ok(())
    }
}

impl SpotSource for FreeDvSource {
    fn network(&self) -> Network {
        Network::FreeDvReporter
    }

    fn poll(&mut self, sink: &mut dyn FnMut(Spot)) -> Result<(), SourceError> {
        let now = Instant::now();
        if self.conn.is_none() {
            if now < self.next_attempt {
                return Ok(());
            }
            return self.try_connect(now);
        }
        match self.service(now, sink) {
            Ok(()) => Ok(()),
            Err(e) => Err(self.fail(now, e.0)),
        }
    }
}
