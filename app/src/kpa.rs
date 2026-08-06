//! KPA1500 amplifier client worker (FR-AMP-03).
//!
//! A background thread that owns the TCP socket to the amplifier's own remote
//! command server (`k4_kpa::TCP_PORT`), polls it on an interval with
//! [`k4_kpa::POLL`], parses the replies into a [`k4_kpa::KpaState`], and
//! publishes the result plus a connection status into a shared snapshot the UI
//! reads on its tick — mirroring the K4 worker's `Arc<Mutex<…>>` model.
//!
//! It is deliberately independent of the K4 link at the socket level; the app
//! decides *when* to connect (only while the K4 is up and the operator has
//! enabled support — FR-AMP-01) by sending [`Cmd`]s.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use k4_kpa::KpaState;

/// Connection lifecycle, shown by the amp indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Conn {
    /// No target set, or intentionally disconnected.
    #[default]
    Disconnected,
    /// A target is set and the socket is being (re)established.
    Connecting,
    /// Connected and polling.
    Connected,
}

/// The snapshot the UI reads: connection status plus the latest telemetry.
#[derive(Debug, Clone, Default)]
pub struct Shared {
    pub conn: Conn,
    pub state: KpaState,
    /// Last transport error, for the diagnostics/status line.
    pub error: Option<String>,
}

/// Commands from the app to the worker.
pub enum Cmd {
    /// (Re)connect to `host:port`, polling every `interval_ms`. A repeat of the
    /// current target while connected is ignored.
    Connect {
        host: String,
        port: u16,
        interval_ms: u64,
    },
    /// Drop the socket and stop polling.
    Disconnect,
    /// Send a raw control command (already `^…;`-framed) to the amplifier.
    Send(String),
}

/// How long to wait for the initial TCP connect before reporting failure.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(4);
/// Read timeout per service slice — short so commands stay responsive.
const READ_SLICE: Duration = Duration::from_millis(100);
/// Minimum gap between reconnect attempts after a failure.
const RECONNECT_BACKOFF: Duration = Duration::from_secs(3);

/// Spawn the worker thread.
pub fn spawn(rx: Receiver<Cmd>, shared: Arc<Mutex<Shared>>) -> JoinHandle<()> {
    thread::spawn(move || run(&rx, &shared))
}

struct Target {
    host: String,
    port: u16,
    interval: Duration,
}

fn run(rx: &Receiver<Cmd>, shared: &Arc<Mutex<Shared>>) {
    let mut target: Option<Target> = None;
    let mut stream: Option<TcpStream> = None;
    let mut rxbuf = String::new();
    let mut state = KpaState::default();
    let mut last_poll = Instant::now();
    let mut last_attempt: Option<Instant> = None;

    loop {
        // 1. Drain pending commands (non-blocking).
        loop {
            match rx.try_recv() {
                Ok(Cmd::Connect {
                    host,
                    port,
                    interval_ms,
                }) => {
                    let same = target
                        .as_ref()
                        .is_some_and(|t| t.host == host && t.port == port);
                    if same && stream.is_some() {
                        // Already on this target — just update the interval.
                        if let Some(t) = target.as_mut() {
                            t.interval = Duration::from_millis(interval_ms.max(50));
                        }
                    } else {
                        target = Some(Target {
                            host,
                            port,
                            interval: Duration::from_millis(interval_ms.max(50)),
                        });
                        stream = None;
                        last_attempt = None;
                        state = KpaState::default();
                        publish(shared, Conn::Connecting, &state, None);
                    }
                }
                Ok(Cmd::Disconnect) => {
                    target = None;
                    stream = None;
                    rxbuf.clear();
                    state = KpaState::default();
                    publish(shared, Conn::Disconnected, &state, None);
                }
                Ok(Cmd::Send(cmd)) => {
                    if let Some(s) = stream.as_mut() {
                        if s.write_all(cmd.as_bytes()).is_err() {
                            stream = None;
                            publish(shared, Conn::Connecting, &state, Some("write failed"));
                        }
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return, // app gone
            }
        }

        // 2. Establish the socket if we have a target and none open.
        if let (Some(t), None) = (target.as_ref(), stream.as_ref()) {
            let due = last_attempt.is_none_or(|a| a.elapsed() >= RECONNECT_BACKOFF);
            if due {
                last_attempt = Some(Instant::now());
                match connect(t) {
                    Ok(s) => {
                        rxbuf.clear();
                        state = KpaState::default();
                        // Identity + firmware + serial once, then the first poll.
                        let _ = s.try_clone().map(|mut w| {
                            let _ = w.write_all(k4_kpa::IDENT.as_bytes());
                            let _ = w.write_all(k4_kpa::POLL.as_bytes());
                        });
                        last_poll = Instant::now();
                        stream = Some(s);
                        publish(shared, Conn::Connected, &state, None);
                    }
                    Err(e) => {
                        publish(shared, Conn::Connecting, &state, Some(&e));
                    }
                }
            }
        }

        // 3. Service an open socket: poll on interval, read + parse replies.
        if let (Some(t), Some(s)) = (target.as_ref(), stream.as_mut()) {
            if last_poll.elapsed() >= t.interval {
                if s.write_all(k4_kpa::POLL.as_bytes()).is_err() {
                    stream = None;
                    publish(shared, Conn::Connecting, &state, Some("poll write failed"));
                    continue;
                }
                last_poll = Instant::now();
            }
            match read_available(s, &mut rxbuf) {
                Ok(true) => {
                    // Apply complete `;`-terminated replies; keep any tail.
                    if let Some(cut) = rxbuf.rfind(';') {
                        let complete: String = rxbuf.drain(..=cut).collect();
                        k4_kpa::apply(&mut state, &complete);
                        publish(shared, Conn::Connected, &state, None);
                    }
                }
                Ok(false) => {} // just a timeout, no data
                Err(e) => {
                    stream = None;
                    publish(shared, Conn::Connecting, &state, Some(&e));
                }
            }
        }

        // Idle a beat when there is nothing to do, so a disconnected worker
        // does not spin.
        if stream.is_none() {
            thread::sleep(READ_SLICE);
        }
    }
}

fn connect(t: &Target) -> Result<TcpStream, String> {
    // Resolve then connect with a bounded timeout so a wrong host can't hang.
    let addr = format!("{}:{}", t.host, t.port);
    let sock = addr
        .to_socket_addrs_first()
        .ok_or_else(|| format!("cannot resolve {addr}"))?;
    let stream = TcpStream::connect_timeout(&sock, CONNECT_TIMEOUT)
        .map_err(|e| format!("connect failed: {e}"))?;
    stream
        .set_read_timeout(Some(READ_SLICE))
        .map_err(|e| format!("socket setup failed: {e}"))?;
    Ok(stream)
}

/// Read whatever is available within one [`READ_SLICE`], appending UTF-8 text to
/// `buf`. `Ok(true)` = bytes were read, `Ok(false)` = timed out with nothing,
/// `Err` = the connection is broken.
fn read_available(s: &mut TcpStream, buf: &mut String) -> Result<bool, String> {
    let mut tmp = [0u8; 512];
    match s.read(&mut tmp) {
        Ok(0) => Err("amplifier closed the connection".to_string()),
        Ok(n) => {
            buf.push_str(&String::from_utf8_lossy(&tmp[..n]));
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::TimedOut => Ok(false),
        Err(e) => Err(format!("read failed: {e}")),
    }
}

fn publish(shared: &Arc<Mutex<Shared>>, conn: Conn, state: &KpaState, error: Option<&str>) {
    if let Ok(mut g) = shared.lock() {
        g.conn = conn;
        g.state = state.clone();
        g.error = error.map(str::to_string);
    }
}

/// Minimal single-address resolution so the worker needs no async resolver.
trait ResolveFirst {
    fn to_socket_addrs_first(&self) -> Option<std::net::SocketAddr>;
}
impl ResolveFirst for str {
    fn to_socket_addrs_first(&self) -> Option<std::net::SocketAddr> {
        use std::net::ToSocketAddrs;
        self.to_socket_addrs().ok().and_then(|mut it| it.next())
    }
}
