//! The CAT server's I/O shell (FR-CATSRV-01/02): a plain-TCP listener with one reader and one
//! writer thread per client, speaking raw `;`-terminated CAT (no framing, no authentication —
//! which is why the app binds it to loopback by default). It only moves lines: every decision is
//! the caller's, who drains [`Server::poll`] and answers with [`Server::send`], normally by way of
//! [`crate::handle`].
//!
//! Bounded throughout, since a client is untrusted: at most `max_clients` connections (one more
//! is closed at once and never reported), each command line at most the decoder's 64 KiB (junk
//! past that is discarded, costing only that client), and each client's outgoing queue at most
//! [`OUT_QUEUE`] lines — a client that stops reading is dropped rather than allowed to grow it or
//! to stall the caller.

use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use k4_protocol::cat::LineDecoder;

/// A connected client's number, unique for the server's lifetime.
pub type ClientId = u64;

/// Most lines queued to one client before it is dropped as not reading.
pub const OUT_QUEUE: usize = 256;

/// How long one write to a client may block before the client is dropped.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// What happened on the listener, in order per client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Connected(ClientId, SocketAddr),
    /// One command, `;` included.
    Line(ClientId, String),
    Disconnected(ClientId),
}

struct Peer {
    out: SyncSender<String>,
    stream: TcpStream,
}

type Peers = Arc<Mutex<HashMap<ClientId, Peer>>>;

/// A running CAT server. Dropping it stops the listener and closes every client.
pub struct Server {
    addr: SocketAddr,
    events: Receiver<Event>,
    peers: Peers,
    stop: Arc<AtomicBool>,
}

impl Server {
    /// Listen on `bind:port` (`port` 0 = any free port), accepting at most `max_clients` at once.
    pub fn start(bind: &str, port: u16, max_clients: usize) -> std::io::Result<Server> {
        Self::start_with_write_timeout(bind, port, max_clients, WRITE_TIMEOUT)
    }

    /// As [`Server::start`], with how long one write to a client may block before the client is
    /// dropped. (Two things drop a client that falls behind: a write that times out, and a full
    /// outgoing queue — the latter also catches a client that reads, but too slowly.)
    pub fn start_with_write_timeout(
        bind: &str,
        port: u16,
        max_clients: usize,
        write_timeout: Duration,
    ) -> std::io::Result<Server> {
        let listener = TcpListener::bind((bind, port))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let (tx, events) = mpsc::channel();
        let peers: Peers = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let (p, s) = (Arc::clone(&peers), Arc::clone(&stop));
        thread::spawn(move || accept_loop(listener, tx, p, s, max_clients, write_timeout));
        Ok(Server {
            addr,
            events,
            peers,
            stop,
        })
    }

    /// Where it listens.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Everything that happened since the last call, without blocking.
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Queue `line` to a client. A client whose queue is full is closed (it has stopped reading);
    /// an unknown or departed client is ignored.
    pub fn send(&self, id: ClientId, line: &str) {
        let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
        let full = match peers.get(&id) {
            Some(p) => matches!(p.out.try_send(line.to_string()), Err(TrySendError::Full(_))),
            None => return,
        };
        if full {
            if let Some(p) = peers.remove(&id) {
                let _ = p.stream.shutdown(Shutdown::Both);
            }
        }
    }

    /// Close one client.
    pub fn close(&self, id: ClientId) {
        let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = peers.remove(&id) {
            let _ = p.stream.shutdown(Shutdown::Both);
        }
    }

    /// Close every client (the listener keeps running).
    pub fn close_all(&self) {
        let mut peers = self.peers.lock().unwrap_or_else(|e| e.into_inner());
        for (_, p) in peers.drain() {
            let _ = p.stream.shutdown(Shutdown::Both);
        }
    }

    /// Clients connected now.
    pub fn client_count(&self) -> usize {
        self.peers.lock().map_or(0, |p| p.len())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.close_all();
    }
}

fn accept_loop(
    listener: TcpListener,
    events: Sender<Event>,
    peers: Peers,
    stop: Arc<AtomicBool>,
    max_clients: usize,
    write_timeout: Duration,
) {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, from)) => {
                let full = peers.lock().map_or(true, |p| p.len() >= max_clients);
                if full {
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                }
                let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
                if let Err(e) =
                    start_client(id, stream, from, &events, &peers, &stop, write_timeout)
                {
                    // A socket that cannot be set up is simply not taken.
                    let _ = e;
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(_) => thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn start_client(
    id: ClientId,
    stream: TcpStream,
    from: SocketAddr,
    events: &Sender<Event>,
    peers: &Peers,
    stop: &Arc<AtomicBool>,
    write_timeout: Duration,
) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    stream.set_write_timeout(Some(write_timeout))?;
    let reader = stream.try_clone()?;
    let mut writer = stream.try_clone()?;
    let (out, queue) = mpsc::sync_channel::<String>(OUT_QUEUE);
    peers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(id, Peer { out, stream });
    let _ = events.send(Event::Connected(id, from));

    // Writer: sends queued lines until the queue is gone (the peer was removed) or a write fails.
    let wpeers = Arc::clone(peers);
    thread::spawn(move || {
        for line in queue {
            if writer.write_all(line.as_bytes()).is_err() {
                if let Some(p) = wpeers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
                    let _ = p.stream.shutdown(Shutdown::Both);
                }
                break;
            }
        }
    });

    // Reader: decodes commands until the client leaves, is closed, or the server stops.
    let (events, peers, stop) = (events.clone(), Arc::clone(peers), Arc::clone(stop));
    thread::spawn(move || {
        let mut reader = reader;
        let mut decoder = LineDecoder::new();
        let mut buf = [0u8; 4096];
        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    for line in decoder.push(&buf[..n]) {
                        let _ = events.send(Event::Line(id, line));
                    }
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    // Closed from our side (send/close/close_all removed it): stop reading.
                    if !peers.lock().is_ok_and(|p| p.contains_key(&id)) {
                        break;
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        if let Some(p) = peers.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
            let _ = p.stream.shutdown(Shutdown::Both);
        }
        let _ = events.send(Event::Disconnected(id));
    });
    Ok(())
}
