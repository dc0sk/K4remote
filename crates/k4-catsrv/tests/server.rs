//! The CAT server's listener (FR-CATSRV-01/02): plain TCP, clients kept apart, bounded.
//! Driven here by a stand-in for the worker that echoes each command back, so only the I/O shell
//! is under test.
//! trace: FR-CATSRV-01, FR-CATSRV-02

use k4_catsrv::server::{Event, Server};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

fn connect(s: &Server) -> TcpStream {
    let c = TcpStream::connect(s.addr()).expect("connect");
    c.set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    c
}

/// Drain events until `done` says so, or give up after 3 s.
fn events_until(s: &Server, mut done: impl FnMut(&[Event]) -> bool) -> Vec<Event> {
    let t0 = Instant::now();
    let mut all = Vec::new();
    while t0.elapsed() < Duration::from_secs(3) {
        all.extend(s.poll());
        if done(&all) {
            return all;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("timed out; events so far: {all:?}");
}

/// Read from a client until `want` has arrived or 3 s pass.
fn read_until(c: &mut TcpStream, want: &str) -> String {
    let t0 = Instant::now();
    let mut got = String::new();
    let mut buf = [0u8; 512];
    while t0.elapsed() < Duration::from_secs(3) && !got.contains(want) {
        if let Ok(n) = c.read(&mut buf) {
            if n == 0 {
                break;
            }
            got.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
    got
}

/// The listener binds where it is told (loopback here, port 0 = any free one), reports a client's
/// arrival, its commands one by one — split correctly across and within reads — and its leaving.
/// trace: FR-CATSRV-01
#[test]
fn fr_catsrv_01_commands_arrive_one_by_one_and_replies_go_back() {
    let s = Server::start("127.0.0.1", 0, 4).expect("start");
    assert!(s.addr().ip().is_loopback());
    let mut c = connect(&s);
    let evs = events_until(&s, |e| e.iter().any(|x| matches!(x, Event::Connected(..))));
    let Event::Connected(id, _) = evs[0] else {
        panic!("{evs:?}")
    };
    assert_eq!(s.client_count(), 1);
    c.write_all(b"FA;I").unwrap();
    c.write_all(b"D;\r\nMD;").unwrap();
    let evs = events_until(&s, |e| {
        e.iter().filter(|x| matches!(x, Event::Line(..))).count() == 3
    });
    let lines: Vec<&str> = evs
        .iter()
        .filter_map(|x| match x {
            Event::Line(i, l) if *i == id => Some(l.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "FA;");
    assert_eq!(lines[1], "ID;");
    assert_eq!(lines[2].trim(), "MD;");
    s.send(id, "FA00014074000;");
    assert_eq!(read_until(&mut c, "FA00014074000;"), "FA00014074000;");
    drop(c);
    events_until(&s, |e| {
        e.iter()
            .any(|x| matches!(x, Event::Disconnected(i) if *i == id))
    });
    assert_eq!(s.client_count(), 0);
}

/// Two clients are kept apart: each reply reaches only the client it is addressed to, and one
/// client's junk (an unterminated 70 KB blob, past the decoder's 64 KB bound) costs only itself.
/// trace: FR-CATSRV-02
#[test]
fn fr_catsrv_02_clients_are_kept_apart() {
    let s = Server::start("127.0.0.1", 0, 4).expect("start");
    let mut a = connect(&s);
    let evs = events_until(&s, |e| e.iter().any(|x| matches!(x, Event::Connected(..))));
    let Event::Connected(ida, _) = evs[0] else {
        panic!()
    };
    let mut b = connect(&s);
    let evs = events_until(&s, |e| e.iter().any(|x| matches!(x, Event::Connected(..))));
    let Event::Connected(idb, _) = evs[0] else {
        panic!()
    };
    assert_ne!(ida, idb);
    a.write_all(&vec![b'X'; 70_000]).unwrap();
    b.write_all(b"FB;").unwrap();
    let evs = events_until(&s, |e| e.iter().any(|x| matches!(x, Event::Line(..))));
    assert!(
        evs.iter()
            .all(|x| !matches!(x, Event::Line(i, _) if *i == ida)),
        "the blob produced a command: {evs:?}"
    );
    assert!(evs
        .iter()
        .any(|x| matches!(x, Event::Line(i, l) if *i == idb && l == "FB;")));
    s.send(idb, "FB00007074000;");
    s.send(ida, "FA00014074000;");
    assert_eq!(read_until(&mut b, ";"), "FB00007074000;");
    assert_eq!(read_until(&mut a, ";"), "FA00014074000;");
    // After the blob, client A still works: the junk runs to the next `;` (here the first
    // command's), and the command after that arrives whole.
    a.write_all(b"FA;ID;").unwrap();
    let evs = events_until(&s, |e| {
        e.iter()
            .any(|x| matches!(x, Event::Line(i, l) if *i == ida && l == "ID;"))
    });
    assert!(
        evs.iter()
            .all(|x| !matches!(x, Event::Line(i, l) if *i == ida && l.contains('X'))),
        "junk reached the caller: {evs:?}"
    );
}

/// Bounded: past the client limit a new connection is closed at once (and never reported), and a
/// client that stops reading is dropped once its outgoing queue fills, rather than growing it
/// without end or stalling the caller.
/// trace: FR-CATSRV-02
#[test]
fn fr_catsrv_02_client_count_and_queues_are_bounded() {
    let s = Server::start("127.0.0.1", 0, 1).expect("start");
    let _first = connect(&s);
    let evs = events_until(&s, |e| e.iter().any(|x| matches!(x, Event::Connected(..))));
    let Event::Connected(id, _) = evs[0] else {
        panic!()
    };
    let mut second = connect(&s);
    let mut buf = [0u8; 8];
    let t0 = Instant::now();
    let closed = loop {
        match second.read(&mut buf) {
            Ok(0) => break true,
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => break true,
            _ if t0.elapsed() > Duration::from_secs(3) => break false,
            _ => {}
        }
    };
    assert!(closed, "a client over the limit was kept");
    assert_eq!(s.client_count(), 1);
    assert!(s.poll().iter().all(|x| !matches!(x, Event::Connected(..))));

    // The first client never reads: flood it until the server gives up on it (here the write
    // timeout would do it too — the queue bound has its own test below).
    let line = format!("IF{};", "0".repeat(60));
    let t0 = Instant::now();
    while s.client_count() == 1 && t0.elapsed() < Duration::from_secs(10) {
        for _ in 0..1000 {
            s.send(id, &line);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        s.client_count(),
        0,
        "a client that stopped reading was kept"
    );
}

/// `close_all` ends every client's connection (they see the socket close), and dropping the server
/// stops the listener.
/// trace: FR-CATSRV-01
#[test]
fn fr_catsrv_01_close_all_and_drop_end_everything() {
    let s = Server::start("127.0.0.1", 0, 4).expect("start");
    let addr = s.addr();
    let mut c = connect(&s);
    events_until(&s, |e| e.iter().any(|x| matches!(x, Event::Connected(..))));
    s.close_all();
    let mut buf = [0u8; 8];
    let t0 = Instant::now();
    let closed = loop {
        match c.read(&mut buf) {
            Ok(0) => break true,
            Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => break true,
            _ if t0.elapsed() > Duration::from_secs(3) => break false,
            _ => {}
        }
    };
    assert!(closed, "close_all left the client connected");
    drop(s);
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err(),
        "the listener outlived the server"
    );
}

/// The outgoing-queue bound on its own: with writes allowed to block for a minute, only a full
/// queue can drop a client that has stopped reading — and it does, within seconds, instead of
/// the queue growing or the caller stalling.
/// trace: FR-CATSRV-02
#[test]
fn fr_catsrv_02_a_full_outgoing_queue_drops_the_client() {
    let s = Server::start_with_write_timeout("127.0.0.1", 0, 4, Duration::from_secs(60))
        .expect("start");
    let _c = connect(&s);
    let evs = events_until(&s, |e| e.iter().any(|x| matches!(x, Event::Connected(..))));
    let Event::Connected(id, _) = evs[0] else {
        panic!()
    };
    let line = format!("IF{};", "0".repeat(60));
    let t0 = Instant::now();
    while s.client_count() == 1 && t0.elapsed() < Duration::from_secs(20) {
        for _ in 0..1000 {
            s.send(id, &line);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        s.client_count(),
        0,
        "the queue bound did not drop the client"
    );
    assert!(
        t0.elapsed() < Duration::from_secs(20),
        "dropped only after {:?}",
        t0.elapsed()
    );
}
