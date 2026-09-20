//! An MQTT spot source (FR-SPOT-05): connects to PSK Reporter's public feed, subscribes to the
//! band topics it is told to, and delivers the spots that arrive.
//!
//! Built on [`crate::mqtt`] (the codec), [`crate::psk`] (the message format) and the
//! [`SpotSource`] interface, and shaped like [`crate::telnet::TelnetSource`]: each [`poll`] does a
//! bounded amount of work, a failure is *returned* so it can be shown against its network
//! (FR-SPOT-09), and the source reconnects by itself with backoff.
//!
//! **It subscribes to exactly the topics it is given and nothing broader.** With no topics it
//! subscribes to nothing: an unfiltered subscription to the feed is a firehose (about 130–290
//! messages a second observed), so "no bands wanted" must mean silence, not everything.
//!
//! Nothing identifying is sent: no username, no password, and a client id made only from the
//! process id and a clock reading.
//!
//! [`poll`]: SpotSource::poll

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::cluster::RateGate;
use crate::mqtt::{self, Packet, PacketReader};
use crate::psk::parse_payload;
use crate::telnet::{ConnState, Stats, Timing};
use crate::{Network, SourceError, Spot, SpotSource};

/// What to connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttConfig {
    /// Which network the spots are attributed to.
    pub network: Network,
    pub host: String,
    pub port: u16,
}

enum Phase {
    AwaitConnAck,
    Streaming,
}

struct Conn {
    stream: TcpStream,
    reader: PacketReader,
    gate: RateGate,
    phase: Phase,
    since: Instant,
    last_rx: Instant,
    last_tx: Instant,
    /// The topics the broker has been asked for on this connection.
    subscribed: Vec<String>,
    next_id: u16,
}

impl Conn {
    fn send(&mut self, bytes: &[u8]) -> Result<(), SourceError> {
        self.stream
            .write_all(bytes)
            .map_err(|e| SourceError(format!("could not send to the broker: {e}")))?;
        self.last_tx = Instant::now();
        Ok(())
    }

    fn id(&mut self) -> u16 {
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.next_id
    }
}

/// An MQTT spot source.
pub struct MqttSource {
    cfg: MqttConfig,
    timing: Timing,
    conn: Option<Conn>,
    next_attempt: Instant,
    backoff: Duration,
    /// The topics wanted. Empty means subscribe to nothing.
    topics: Vec<String>,
    /// Only spots inside `[lo, hi]` Hz are kept; `None` keeps everything.
    window: Option<(u64, u64)>,
    stats: Stats,
    attempts: u64,
    epoch: Instant,
    client_id: String,
}

impl MqttSource {
    pub fn new(cfg: MqttConfig) -> Self {
        Self::with_timing(cfg, Timing::default())
    }

    pub fn with_timing(cfg: MqttConfig, timing: Timing) -> Self {
        let now = Instant::now();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        Self {
            backoff: timing.initial_backoff,
            client_id: format!("k4r-{:x}-{:x}", std::process::id(), nanos),
            cfg,
            timing,
            conn: None,
            next_attempt: now,
            topics: Vec::new(),
            window: None,
            stats: Stats::default(),
            attempts: 0,
            epoch: now,
        }
    }

    /// The topics to subscribe to (empty = none). Applied to a live connection on the next poll.
    pub fn set_topics(&mut self, topics: Vec<String>) {
        self.topics = topics;
    }

    /// Keep only spots between `lo` and `hi` Hz (`None` = all).
    pub fn set_window(&mut self, window: Option<(u64, u64)>) {
        self.window = window;
    }

    pub fn state(&self) -> ConnState {
        match self.conn.as_ref().map(|c| &c.phase) {
            None => ConnState::Disconnected,
            Some(Phase::AwaitConnAck) => ConnState::AwaitingLogin,
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

    pub fn config(&self) -> &MqttConfig {
        &self.cfg
    }

    fn fail(&mut self, now: Instant, msg: String) -> SourceError {
        self.conn = None;
        self.next_attempt = now + self.backoff;
        self.backoff = (self.backoff * 2).min(self.timing.max_backoff);
        SourceError(msg)
    }

    fn try_connect(&mut self, now: Instant) -> Result<(), SourceError> {
        if self.cfg.host.trim().is_empty() {
            self.next_attempt = now + self.timing.max_backoff;
            return Err(SourceError("no host is set".into()));
        }
        self.attempts += 1;
        let addrs: Vec<SocketAddr> = match (self.cfg.host.trim(), self.cfg.port).to_socket_addrs() {
            Ok(a) => a.collect(),
            Err(e) => return Err(self.fail(now, format!("cannot resolve {}: {e}", self.cfg.host))),
        };
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
                    let mut conn = Conn {
                        stream,
                        reader: PacketReader::new(),
                        gate: RateGate::new(self.timing.rate_per_sec),
                        phase: Phase::AwaitConnAck,
                        since: now,
                        last_rx: now,
                        last_tx: now,
                        subscribed: Vec::new(),
                        next_id: 0,
                    };
                    let keepalive = self.timing.keepalive.as_secs().clamp(1, 65_535) as u16;
                    if let Err(e) = conn.send(&mqtt::connect(&self.client_id, keepalive)) {
                        last = Some(std::io::Error::other(e.0));
                        continue;
                    }
                    self.stats.connects += 1;
                    self.conn = Some(conn);
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
        let wanted = self.topics.clone();
        let mut packets = Vec::new();
        let mut connected_now = false;

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
                    Ok(0) => return Err(SourceError("the broker closed the connection".into())),
                    Ok(n) => {
                        total += n;
                        conn.last_rx = Instant::now();
                        match conn.reader.push(&buf[..n]) {
                            Ok(p) => packets.extend(p),
                            Err(e) => return Err(SourceError(format!("protocol error: {e}"))),
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

            match conn.phase {
                Phase::AwaitConnAck => {
                    if now.saturating_duration_since(conn.since) > timing.prompt_timeout {
                        return Err(SourceError(format!(
                            "no reply to the connection request within {} s",
                            timing.prompt_timeout.as_secs_f32().round()
                        )));
                    }
                }
                Phase::Streaming => {
                    if now.saturating_duration_since(conn.last_rx) > timing.keepalive * 2 {
                        return Err(SourceError(format!(
                            "no data from the broker for {} s",
                            (timing.keepalive * 2).as_secs()
                        )));
                    }
                    if now.saturating_duration_since(conn.last_tx) > timing.keepalive / 2 {
                        conn.send(&mqtt::pingreq())?;
                    }
                }
            }
        }

        // Handle what arrived.
        for packet in packets {
            match packet {
                Packet::ConnAck { code, .. } => {
                    if code != 0 {
                        return Err(SourceError(format!(
                            "the broker refused the connection (code {code})"
                        )));
                    }
                    if let Some(conn) = self.conn.as_mut() {
                        conn.phase = Phase::Streaming;
                    }
                    connected_now = true;
                }
                Packet::SubAck { codes, .. } => {
                    if codes.contains(&0x80) {
                        return Err(SourceError("the broker refused a subscription".into()));
                    }
                }
                Packet::Publish { payload, .. } => {
                    let Some(spot) = parse_payload(&payload, unix_now) else {
                        self.stats.rejected += 1;
                        continue;
                    };
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
                Packet::UnsubAck { .. } | Packet::PingResp | Packet::Other(_) => {}
            }
        }
        if connected_now {
            self.backoff = timing.initial_backoff;
        }
        if let Some(conn) = self.conn.as_ref() {
            self.stats.shed = conn.gate.shed;
        }

        // Keep the subscriptions equal to the topics wanted: ask for only what is added, and let
        // go of what is no longer wanted.
        if let Some(conn) = self.conn.as_mut() {
            if matches!(conn.phase, Phase::Streaming) && conn.subscribed != wanted {
                let removed: Vec<String> = conn
                    .subscribed
                    .iter()
                    .filter(|t| !wanted.contains(t))
                    .cloned()
                    .collect();
                let added: Vec<String> = wanted
                    .iter()
                    .filter(|t| !conn.subscribed.contains(t))
                    .cloned()
                    .collect();
                if !removed.is_empty() {
                    let id = conn.id();
                    let topics: Vec<&str> = removed.iter().map(String::as_str).collect();
                    conn.send(&mqtt::unsubscribe(id, &topics))?;
                }
                if !added.is_empty() {
                    let id = conn.id();
                    let topics: Vec<&str> = added.iter().map(String::as_str).collect();
                    conn.send(&mqtt::subscribe(id, &topics))?;
                }
                conn.subscribed = wanted;
            }
        }
        Ok(())
    }
}

impl SpotSource for MqttSource {
    fn network(&self) -> Network {
        self.cfg.network
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
