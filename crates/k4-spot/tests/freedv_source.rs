//! The FreeDV Reporter source against a scripted mock server on loopback (FR-SPOT-08, FR-SPOT-09,
//! FR-SPOT-12).
//!
//! The server side is written here by hand — its frames are built and the client's are read
//! byte by byte — independently of the crate's own encoder and decoder, so what the client puts on
//! the wire is checked against RFC 6455 and not against itself. (It borrows only `accept_key`, which
//! is verified against the RFC's worked example in the crate's own tests.)
//! trace: FR-SPOT-08, FR-SPOT-09, FR-SPOT-12

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use k4_spot::freedv_source::{FreeDvConfig, FreeDvSource};
use k4_spot::telnet::{ConnState, Timing};
use k4_spot::ws::accept_key;
use k4_spot::{Network, Spot, SpotSource};

fn fast() -> Timing {
    Timing {
        connect_timeout: Duration::from_secs(1),
        prompt_timeout: Duration::from_millis(500),
        initial_backoff: Duration::from_millis(30),
        max_backoff: Duration::from_millis(120),
        read_budget: Duration::from_millis(50),
        ..Timing::default()
    }
}

fn cfg(port: u16) -> FreeDvConfig {
    FreeDvConfig {
        host: "127.0.0.1".into(),
        port,
        user_agent: "K4remote/test".into(),
    }
}

/// An unmasked server frame (what a server sends), built by hand.
fn frame(op: u8, payload: &[u8]) -> Vec<u8> {
    let mut f = vec![0x80 | op];
    match payload.len() {
        n if n < 126 => f.push(n as u8),
        n if n <= 0xffff => {
            f.push(126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            f.push(127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    f.extend_from_slice(payload);
    f
}

fn text(s: &str) -> Vec<u8> {
    frame(0x1, s.as_bytes())
}

/// Read the client's upgrade request; returns its text and its `Sec-WebSocket-Key`.
fn read_upgrade(s: &mut TcpStream) -> (String, String) {
    s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    let mut req = Vec::new();
    let mut b = [0u8; 1];
    while !req.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).expect("the upgrade request");
        req.push(b[0]);
    }
    let req = String::from_utf8(req).unwrap();
    let key = req
        .lines()
        .find_map(|l| l.strip_prefix("Sec-WebSocket-Key: "))
        .expect("a key")
        .trim()
        .to_string();
    (req, key)
}

fn accept(s: &mut TcpStream, key: &str) {
    write!(
        s,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key)
    )
    .unwrap();
}

/// Read one client frame, checking it is masked as RFC 6455 requires, and unmask it by hand.
fn read_frame(s: &mut TcpStream, wait: Duration) -> Option<(u8, Vec<u8>)> {
    s.set_read_timeout(Some(wait)).ok()?;
    let mut h = [0u8; 2];
    s.read_exact(&mut h).ok()?;
    assert_eq!(h[0] & 0x80, 0x80, "the client's frames are final");
    assert_eq!(h[0] & 0x70, 0, "no reserved bits");
    assert_eq!(h[1] & 0x80, 0x80, "a client frame must be masked");
    let len = match h[1] & 0x7f {
        126 => {
            let mut l = [0u8; 2];
            s.read_exact(&mut l).ok()?;
            usize::from(u16::from_be_bytes(l))
        }
        127 => {
            let mut l = [0u8; 8];
            s.read_exact(&mut l).ok()?;
            u64::from_be_bytes(l) as usize
        }
        n => usize::from(n),
    };
    let mut mask = [0u8; 4];
    s.read_exact(&mut mask).ok()?;
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).ok()?;
    for (i, b) in body.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    Some((h[0] & 0x0f, body))
}

fn serve(scripts: Vec<Box<dyn FnOnce(TcpStream) + Send>>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for script in scripts {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            script(stream);
        }
    });
    port
}

const OPEN: &str = r#"0{"sid":"srv1","upgrades":[],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}"#;
const VIEW: &str = r#"40{"role":"view","protocol_version":2}"#;

/// The server side of a good session up to and including the client's `view` connect and the
/// server's acknowledgement. Returns what the client sent.
fn join(s: &mut TcpStream) -> (String, (u8, Vec<u8>)) {
    let (req, key) = read_upgrade(s);
    accept(s, &key);
    s.write_all(&text(OPEN)).unwrap();
    let connect = read_frame(s, Duration::from_secs(3)).expect("the view connect");
    s.write_all(&text(r#"40{"sid":"me1"}"#)).unwrap();
    (req, connect)
}

fn ev(name: &str, args: &str) -> Vec<u8> {
    text(&format!("42[\"{name}\",{args}]"))
}

struct Run {
    spots: Vec<Spot>,
    errors: Vec<String>,
}

fn pump(
    src: &mut FreeDvSource,
    max: Duration,
    mut done: impl FnMut(&mut FreeDvSource, &Run) -> bool,
) -> Run {
    let mut run = Run {
        spots: Vec::new(),
        errors: Vec::new(),
    };
    let t0 = Instant::now();
    while t0.elapsed() < max {
        let mut got = Vec::new();
        if let Err(e) = src.poll(&mut |s| got.push(s)) {
            run.errors.push(e.to_string());
        }
        run.spots.extend(got);
        if done(src, &run) {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    run
}

const LONG: Duration = Duration::from_secs(5);

/// The source upgrades, joins **only** as a read-only viewer, sends nothing that identifies the
/// operator, and delivers the in-window stations from the server's `bulk_update` and later events
/// as spots — counting the bad and out-of-window ones.
#[test]
fn fr_spot_08_source_joins_as_a_viewer_and_delivers_spots() {
    let (tx, rx) = std::sync::mpsc::channel();
    let port = serve(vec![Box::new(move |mut s| {
        let (req, connect) = join(&mut s);
        // An event before anything else is fine to ignore; then the roster.
        s.write_all(&ev(
            "bulk_update",
            r#"[["new_connection",{"sid":"a","callsign":"aa1aaa","grid_square":"FN31"}],["freq_change",{"sid":"a","freq":14236000}],
                ["new_connection",{"sid":"b","callsign":"bb2bbb"}],["freq_change",{"sid":"b","freq":7177000}],
                ["freq_change",{"sid":"c","freq":"x"}],
                ["freq_change",{"sid":"lo","freq":14000000,"callsign":"lo1lo"}],
                ["freq_change",{"sid":"hi","freq":14300000,"callsign":"hi1hi"}],
                ["freq_change",{"sid":"below","freq":13999999,"callsign":"be1low"}],
                ["freq_change",{"sid":"above","freq":14300001,"callsign":"ab1ove"}]]"#,
        ))
        .unwrap();
        s.write_all(&ev(
            "tx_report",
            r#"{"sid":"a","transmitting":true,"mode":"RADEV1"}"#,
        ))
        .unwrap();
        s.write_all(&ev(
            "rx_report",
            r#"{"sid":"z","callsign":"AA1AAA","receiver_callsign":"cc3ccc","snr":-9}"#,
        ))
        .unwrap();
        // What else does the client send in the next moment? (Nothing.)
        let extra = read_frame(&mut s, Duration::from_millis(400));
        tx.send((req, connect, extra)).unwrap();
        thread::sleep(Duration::from_millis(300));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    src.set_window(Some((14_000_000, 14_300_000)));
    let run = pump(&mut src, LONG, |_, r| {
        r.spots.iter().any(|s| s.snr_db == Some(-9))
    });
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(src.state(), ConnState::Connected);
    assert_eq!(
        src.stations(),
        6,
        "a, b, lo, hi, below, above; the bad `c` was dropped"
    );
    // The window's edges are inclusive, and one hertz outside either is not.
    let calls: std::collections::HashSet<&str> =
        run.spots.iter().map(|s| s.call.as_str()).collect();
    assert!(
        calls.contains("LO1LO") && calls.contains("HI1HI"),
        "{calls:?}"
    );
    assert!(
        !calls.contains("BE1LOW") && !calls.contains("AB1OVE"),
        "{calls:?}"
    );
    assert_eq!(src.network(), Network::FreeDvReporter);
    // The station inside the window, with what the events said.
    let last = run.spots.iter().rev().find(|s| s.call == "AA1AAA").unwrap();
    assert_eq!(last.network, Network::FreeDvReporter);
    assert_eq!(last.freq_hz, 14_236_000);
    assert_eq!(last.mode.as_deref(), Some("RADEV1"));
    assert_eq!(last.comment.as_deref(), Some("TX"));
    assert_eq!(last.spotter.as_deref(), Some("CC3CCC"));
    assert_eq!(last.snr_db, Some(-9));
    assert!(
        run.spots.iter().all(|s| s.call != "BB2BBB"),
        "7.177 MHz is outside the window"
    );
    let st = src.stats();
    assert!(st.spots >= 1 && st.outside_window >= 3, "{st:?}");
    assert_eq!(st.rejected, 1, "the bad freq_change is counted");
    assert_eq!(st.connects, 1);

    // What the client sent, on the wire.
    let (req, (op, connect), extra) = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        req.starts_with("GET /socket.io/?EIO=4&transport=websocket HTTP/1.1\r\n"),
        "{req}"
    );
    for want in [
        "Upgrade: websocket\r\n",
        "Connection: Upgrade\r\n",
        "Sec-WebSocket-Version: 13\r\n",
        "User-Agent: K4remote/test\r\n",
    ] {
        assert!(req.contains(want), "{req}");
    }
    let lower = req.to_ascii_lowercase();
    for leak in ["cookie", "authorization", "callsign", "referer", "origin"] {
        assert!(!lower.contains(leak), "the request carries {leak}:\n{req}");
    }
    assert_eq!(op, 0x1, "the connect is a text frame");
    assert_eq!(
        String::from_utf8(connect).unwrap(),
        VIEW,
        "exactly the read-only view connect"
    );
    assert_eq!(extra, None, "the client sent nothing else, unprompted");
}

/// Engine.IO pings and WebSocket pings are each answered, with the right reply.
#[test]
fn fr_spot_08_source_answers_pings() {
    let (tx, rx) = std::sync::mpsc::channel();
    let port = serve(vec![Box::new(move |mut s| {
        join(&mut s);
        s.write_all(&text("2")).unwrap();
        let eio = read_frame(&mut s, Duration::from_secs(2));
        s.write_all(&frame(0x9, b"xyz")).unwrap();
        let wsp = read_frame(&mut s, Duration::from_secs(2));
        s.write_all(&frame(0xA, b"unsolicited")).unwrap();
        s.write_all(&text("3")).unwrap();
        let after = read_frame(&mut s, Duration::from_millis(300));
        tx.send((eio, wsp, after)).unwrap();
        thread::sleep(Duration::from_millis(200));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, Duration::from_secs(3), |_, _| {
        rx.try_recv().is_ok_and(|v| {
            assert_eq!(
                v.0,
                Some((0x1, b"3".to_vec())),
                "an Engine.IO ping is answered with a `3`"
            );
            assert_eq!(
                v.1,
                Some((0xA, b"xyz".to_vec())),
                "a WebSocket ping with a pong echoing it"
            );
            assert_eq!(v.2, None, "pongs are not answered");
            true
        })
    });
    assert!(run.errors.is_empty(), "{:?}", run.errors);
}

/// A reply to the upgrade that is wrong or hostile is an error worded for the operator, and the
/// source goes on to try again.
#[test]
fn fr_spot_08_source_refuses_a_bad_upgrade() {
    type Script = Box<dyn FnOnce(TcpStream) + Send>;
    let cases: Vec<(&str, Script, &str)> = vec![
        (
            "wrong accept key",
            Box::new(|mut s| {
                let (_, _) = read_upgrade(&mut s);
                accept(&mut s, "some other key");
                thread::sleep(Duration::from_millis(300));
            }),
            "does not match",
        ),
        (
            "HTTP 403",
            Box::new(|mut s| {
                read_upgrade(&mut s);
                s.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
                thread::sleep(Duration::from_millis(300));
            }),
            "HTTP 403",
        ),
        (
            "not HTTP at all",
            Box::new(|mut s| {
                read_upgrade(&mut s);
                s.write_all(b"SSH-2.0-OpenSSH_9.9\r\n\r\n").unwrap();
                thread::sleep(Duration::from_millis(300));
            }),
            "HTTP/1.1",
        ),
        (
            "an extension nobody asked for",
            Box::new(|mut s| {
                let (_, key) = read_upgrade(&mut s);
                write!(s, "HTTP/1.1 101 x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Extensions: permessage-deflate\r\n\r\n", accept_key(&key)).unwrap();
                thread::sleep(Duration::from_millis(300));
            }),
            "extension",
        ),
        (
            "the server not speaking Engine.IO after the upgrade",
            Box::new(|mut s| {
                let (_, key) = read_upgrade(&mut s);
                accept(&mut s, &key);
                s.write_all(&text("hello there")).unwrap();
                thread::sleep(Duration::from_millis(300));
            }),
            "unexpected data",
        ),
    ];
    for (name, script, want) in cases {
        let port = serve(vec![script]);
        let mut src = FreeDvSource::with_timing(cfg(port), fast());
        let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
        assert!(
            run.errors.first().is_some_and(|e| e.contains(want)),
            "{name}: {:?}",
            run.errors
        );
        assert_eq!(src.state(), ConnState::Disconnected, "{name}");
        assert!(run.spots.is_empty(), "{name}");
    }
}

/// A server that accepts the connection and then says nothing is given up on, and one that goes
/// quiet after joining is caught by the ping timing it announced.
#[test]
fn fr_spot_08_source_gives_up_on_a_silent_server() {
    // Silent from the start: no reply to the upgrade.
    let port = serve(vec![Box::new(|mut s| {
        read_upgrade(&mut s);
        thread::sleep(Duration::from_secs(2));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        run.errors[0].contains("no answer within") && run.errors[0].contains("upgrade"),
        "{:?}",
        run.errors
    );

    // Upgraded but never sends `open`.
    let port = serve(vec![Box::new(|mut s| {
        let (_, key) = read_upgrade(&mut s);
        accept(&mut s, &key);
        thread::sleep(Duration::from_secs(2));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        run.errors[0].contains("session to open"),
        "{:?}",
        run.errors
    );

    // Never accepts the `view` connect.
    let port = serve(vec![Box::new(|mut s| {
        let (_, key) = read_upgrade(&mut s);
        accept(&mut s, &key);
        s.write_all(&text(OPEN)).unwrap();
        let _ = read_frame(&mut s, Duration::from_secs(1));
        thread::sleep(Duration::from_secs(2));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(run.errors[0].contains("accepted"), "{:?}", run.errors);

    // Joined, then quiet for longer than the server said it would be (1 s + 1 s, the minimum).
    let port = serve(vec![Box::new(|mut s| {
        let (_, key) = read_upgrade(&mut s);
        accept(&mut s, &key);
        s.write_all(&text(
            r#"0{"sid":"s","pingInterval":1000,"pingTimeout":1000}"#,
        ))
        .unwrap();
        let _ = read_frame(&mut s, Duration::from_secs(1));
        s.write_all(&text("40")).unwrap();
        thread::sleep(Duration::from_secs(4));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let t0 = Instant::now();
    let run = pump(&mut src, Duration::from_secs(4), |_, r| {
        !r.errors.is_empty()
    });
    assert!(
        run.errors[0].contains("no ping from the server for 2 s"),
        "{:?}",
        run.errors
    );
    assert!(
        t0.elapsed() >= Duration::from_millis(1900),
        "gave up early: {:?}",
        t0.elapsed()
    );
    assert_eq!(src.state(), ConnState::Disconnected);
}

/// A server that ends the session, or refuses it, is reported as such.
#[test]
fn fr_spot_08_source_reports_a_refusal_and_an_end() {
    for (packet, want) in [
        (r#"44{"message":"no"}"#, "refused"),
        ("41", "ended the session"),
        ("1", "ended the session"),
    ] {
        let port = serve(vec![Box::new(move |mut s| {
            let (_, key) = read_upgrade(&mut s);
            accept(&mut s, &key);
            s.write_all(&text(OPEN)).unwrap();
            let _ = read_frame(&mut s, Duration::from_secs(1));
            s.write_all(&text(packet)).unwrap();
            thread::sleep(Duration::from_millis(300));
        })]);
        let mut src = FreeDvSource::with_timing(cfg(port), fast());
        let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
        assert!(run.errors[0].contains(want), "{packet}: {:?}", run.errors);
    }
    // A WebSocket close frame ends it at once. The server keeps the socket open afterwards, so only
    // the frame can be what the source reacts to (dropping the socket would give the same message).
    let port = serve(vec![Box::new(|mut s| {
        join(&mut s);
        s.write_all(&frame(0x8, &[0x03, 0xe8])).unwrap();
        thread::sleep(Duration::from_secs(3));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let t0 = Instant::now();
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        t0.elapsed() < Duration::from_secs(1),
        "waited {:?} for a close that had been sent",
        t0.elapsed()
    );
    assert!(
        run.errors[0].contains("closed the connection"),
        "{:?}",
        run.errors
    );
    let port = serve(vec![Box::new(|mut s| {
        join(&mut s);
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        run.errors[0].contains("closed the connection"),
        "{:?}",
        run.errors
    );
    // Nothing listening: a plain connect failure, and it backs off.
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut src = FreeDvSource::with_timing(cfg(closed), fast());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        run.errors[0].contains("connect to 127.0.0.1"),
        "{:?}",
        run.errors
    );
    // No host is a setting problem, not a network one.
    let mut src = FreeDvSource::with_timing(
        FreeDvConfig {
            host: " ".into(),
            ..cfg(80)
        },
        fast(),
    );
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert_eq!(run.errors[0], "no host is set");
}

/// A hostile server — a masked frame, bad UTF-8, an over-size frame, a reserved bit, garbage in a
/// running session — ends the connection with an error where it must, costs only the packet where
/// it can, and a good session afterwards works.
#[test]
fn fr_spot_08_source_survives_a_hostile_server() {
    let hostile: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "a masked frame",
            vec![0x81, 0x81, 1, 2, 3, 4, 0x41 ^ 1],
            "masked",
        ),
        ("bad UTF-8", vec![0x81, 0x02, 0xff, 0xfe], "UTF-8"),
        (
            "an over-size frame",
            vec![0x82, 0x7f, 0, 0, 0, 0, 0, 0x10, 0, 0],
            "larger than",
        ),
        ("a reserved bit", vec![0xc1, 0x00], "reserved"),
        ("a reserved opcode", vec![0x83, 0x00], "reserved opcode"),
    ];
    let mut scripts: Vec<Box<dyn FnOnce(TcpStream) + Send>> = Vec::new();
    for (_, bytes, _) in &hostile {
        let bytes = bytes.clone();
        scripts.push(Box::new(move |mut s| {
            join(&mut s);
            s.write_all(&bytes).unwrap();
            thread::sleep(Duration::from_millis(300));
        }));
    }
    // Then a good session that must work.
    scripts.push(Box::new(|mut s| {
        join(&mut s);
        s.write_all(&ev(
            "freq_change",
            r#"{"sid":"a","freq":14236000,"callsign":"DD4DDD"}"#,
        ))
        .unwrap();
        thread::sleep(Duration::from_millis(500));
    }));
    let port = serve(scripts);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, Duration::from_secs(15), |_, r| {
        r.spots.iter().any(|s| s.call == "DD4DDD")
    });
    assert_eq!(run.errors.len(), hostile.len(), "{:?}", run.errors);
    for (e, (name, _, want)) in run.errors.iter().zip(&hostile) {
        assert!(e.contains(want), "{name}: {e}");
    }
    assert!(
        run.spots.iter().any(|s| s.call == "DD4DDD"),
        "the good session after the bad ones works"
    );

    // Garbage *inside* a running session costs only that packet.
    let port = serve(vec![Box::new(|mut s| {
        join(&mut s);
        for junk in [
            "",
            "x",
            "42",
            "42nope",
            r#"42["e",]"#,
            r#"42[5]"#,
            "4",
            "0{",
        ] {
            s.write_all(&text(junk)).unwrap();
        }
        s.write_all(&ev(
            "freq_change",
            r#"{"sid":"a","freq":14236000,"callsign":"EE5EEE"}"#,
        ))
        .unwrap();
        thread::sleep(Duration::from_millis(600));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| {
        r.spots.iter().any(|s| s.call == "EE5EEE")
    });
    assert!(
        run.errors.is_empty(),
        "a bad packet in a running session must not end it: {:?}",
        run.errors
    );
    assert!(src.stats().rejected >= 5, "{:?}", src.stats());
}

/// After a lost connection the source reconnects with a fresh roster: a station that had left in
/// the meantime is not refreshed into the new session.
#[test]
fn fr_spot_08_source_reconnects_with_a_fresh_roster() {
    let port = serve(vec![
        Box::new(|mut s| {
            join(&mut s);
            s.write_all(&ev(
                "freq_change",
                r#"{"sid":"old","freq":14236000,"callsign":"AA1AAA"}"#,
            ))
            .unwrap();
            thread::sleep(Duration::from_millis(250));
        }),
        Box::new(|mut s| {
            join(&mut s);
            s.write_all(&ev(
                "freq_change",
                r#"{"sid":"new","freq":14236000,"callsign":"BB2BBB"}"#,
            ))
            .unwrap();
            thread::sleep(Duration::from_millis(1500));
        }),
    ]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    src.set_refresh(Duration::from_millis(60));
    let mut after_reconnect = Vec::new();
    let mut reconnected = false;
    let t0 = Instant::now();
    let mut errors = Vec::new();
    while t0.elapsed() < Duration::from_secs(5) && after_reconnect.len() < 4 {
        let mut got = Vec::new();
        if let Err(e) = src.poll(&mut |s| got.push(s)) {
            errors.push(e.to_string());
            reconnected = true;
        }
        if reconnected {
            after_reconnect.extend(got);
        }
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(after_reconnect.iter().any(|s| s.call == "BB2BBB"));
    assert!(
        after_reconnect.iter().all(|s| s.call != "AA1AAA"),
        "a station from the old session was refreshed into the new one"
    );
    assert_eq!(src.stats().connects, 2);
    assert!(src.attempts() >= 2);
}

/// A station that stays on the roster keeps being re-stamped, so it stays on the overlay; one that
/// leaves stops. The re-stamp carries the current time.
#[test]
fn fr_spot_08_source_refreshes_stations_that_are_still_there() {
    let port = serve(vec![Box::new(|mut s| {
        join(&mut s);
        s.write_all(&ev(
            "bulk_update",
            r#"[["freq_change",{"sid":"stay","freq":14236000,"callsign":"AA1AAA"}],["freq_change",{"sid":"go","freq":14237000,"callsign":"BB2BBB"}]]"#,
        ))
        .unwrap();
        thread::sleep(Duration::from_millis(500));
        s.write_all(&ev("remove_connection", r#"{"sid":"go"}"#))
            .unwrap();
        thread::sleep(Duration::from_millis(1500));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    src.set_refresh(Duration::from_millis(100));
    let start = Instant::now();
    let mut log: Vec<(Duration, String, u64)> = Vec::new();
    while start.elapsed() < Duration::from_millis(1800) {
        let mut got = Vec::new();
        let _ = src.poll(&mut |s| got.push(s));
        for s in got {
            log.push((start.elapsed(), s.call.clone(), s.time));
        }
        thread::sleep(Duration::from_millis(5));
    }
    let count = |call: &str| log.iter().filter(|(_, c, _)| c == call).count();
    // ...once per interval, not on every poll: 1.8 s at 100 ms is about 18.
    assert!(
        count("AA1AAA") <= 25,
        "refreshed far too often: {}",
        count("AA1AAA")
    );
    assert!(
        count("AA1AAA") >= 6,
        "a station that stays is refreshed: {}",
        count("AA1AAA")
    );
    assert!(
        count("BB2BBB") >= 2,
        "and so is one that has not yet left: {}",
        count("BB2BBB")
    );
    let last_b = log.iter().rev().find(|(_, c, _)| c == "BB2BBB").unwrap().0;
    let last_a = log.iter().rev().find(|(_, c, _)| c == "AA1AAA").unwrap().0;
    assert!(
        last_b < Duration::from_millis(900),
        "the station that left was refreshed until {last_b:?}"
    );
    assert!(
        last_a > Duration::from_millis(1400),
        "the one that stayed stopped at {last_a:?}"
    );
    // Re-stamps carry the time they were made, so age counts from the last confirmation.
    let times: Vec<u64> = log
        .iter()
        .filter(|(_, c, _)| c == "AA1AAA")
        .map(|(_, _, t)| *t)
        .collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "{times:?}");
}

/// A flood of events is shed at the rate limit and counted, not delivered.
#[test]
fn fr_spot_08_source_sheds_a_flood() {
    let port = serve(vec![Box::new(|mut s| {
        join(&mut s);
        for i in 0..300 {
            s.write_all(&ev(
                "freq_change",
                &format!(
                    r#"{{"sid":"s{i}","freq":14236000,"callsign":"AA{}AAA"}}"#,
                    i % 10
                ),
            ))
            .unwrap();
        }
        thread::sleep(Duration::from_millis(800));
    })]);
    let timing = Timing {
        rate_per_sec: 20,
        ..fast()
    };
    let mut src = FreeDvSource::with_timing(cfg(port), timing);
    let run = pump(&mut src, Duration::from_secs(2), |s, _| {
        s.stats().shed > 100
    });
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    let st = src.stats();
    assert!(st.shed > 100, "{st:?}");
    assert!(run.spots.len() < 100, "{} delivered", run.spots.len());
    assert_eq!(
        src.stations(),
        300,
        "the roster still holds everyone; only delivery is limited"
    );
}

/// A server that speaks out of turn does not get the client to treat it as joined: a Socket.IO
/// `connect` before Engine.IO's `open`, and events before the client has been accepted, are
/// ignored; the state says so until the acknowledgement really comes.
#[test]
fn fr_spot_08_source_ignores_a_server_that_speaks_out_of_turn() {
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let (seen_tx, seen_rx) = std::sync::mpsc::channel::<()>();
    let port = serve(vec![Box::new(move |mut s| {
        let (_, key) = read_upgrade(&mut s);
        accept(&mut s, &key);
        // Out of turn: a connect, and an event, before `open`.
        s.write_all(&text(r#"40{"sid":"early"}"#)).unwrap();
        s.write_all(&ev(
            "freq_change",
            r#"{"sid":"x","freq":14236000,"callsign":"EA1RLY"}"#,
        ))
        .unwrap();
        s.write_all(&text(OPEN)).unwrap();
        let connect = read_frame(&mut s, Duration::from_secs(3)).expect("the view connect");
        assert_eq!(connect.1, VIEW.as_bytes());
        // Still before the acknowledgement: another event.
        s.write_all(&ev(
            "freq_change",
            r#"{"sid":"y","freq":14236000,"callsign":"BE2FOR"}"#,
        ))
        .unwrap();
        seen_tx.send(()).unwrap();
        go_rx.recv_timeout(Duration::from_secs(5)).ok();
        s.write_all(&text(r#"40{"sid":"me"}"#)).unwrap();
        s.write_all(&ev(
            "freq_change",
            r#"{"sid":"z","freq":14236000,"callsign":"AF3TER"}"#,
        ))
        .unwrap();
        thread::sleep(Duration::from_millis(500));
    })]);
    let mut src = FreeDvSource::with_timing(cfg(port), fast());
    let mut spots = Vec::new();
    // Until the server has seen the client's connect and sent its early event, then a little more.
    let t0 = Instant::now();
    let mut waiting = true;
    while waiting && t0.elapsed() < LONG {
        src.poll(&mut |s| spots.push(s)).unwrap();
        waiting = seen_rx.try_recv().is_err();
        thread::sleep(Duration::from_millis(5));
    }
    let t1 = Instant::now();
    while t1.elapsed() < Duration::from_millis(300) {
        src.poll(&mut |s| spots.push(s)).unwrap();
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        src.state(),
        ConnState::AwaitingLogin,
        "not joined until the acknowledgement"
    );
    assert!(
        spots.is_empty(),
        "events before the acknowledgement were used: {spots:?}"
    );
    assert_eq!(src.stations(), 0);
    // Now the acknowledgement.
    go_tx.send(()).unwrap();
    let run = pump(&mut src, LONG, |_, r| {
        r.spots.iter().any(|s| s.call == "AF3TER")
    });
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(src.state(), ConnState::Connected);
    assert_eq!(
        src.stations(),
        1,
        "only the one sent after the acknowledgement"
    );
    assert_eq!(src.stats().connects, 1);
}

/// Backoff doubles up to its cap while attempts fail, waits at least that long each time, and starts
/// over from the initial wait once a session has succeeded.
#[test]
fn fr_spot_08_source_backs_off_and_starts_over_after_a_success() {
    use std::sync::{Arc, Mutex};
    type Marks = Arc<Mutex<Vec<(&'static str, Instant)>>>;
    let marks: Marks = Arc::default();
    let failing = |marks: &Marks| {
        let m = Arc::clone(marks);
        Box::new(move |mut s: TcpStream| {
            m.lock().unwrap().push(("accept", Instant::now()));
            read_upgrade(&mut s);
            // Close without answering: the client sees the connection end.
            drop(s);
            m.lock().unwrap().push(("close", Instant::now()));
        }) as Box<dyn FnOnce(TcpStream) + Send>
    };
    let good = {
        let m = Arc::clone(&marks);
        Box::new(move |mut s: TcpStream| {
            m.lock().unwrap().push(("accept", Instant::now()));
            join(&mut s);
            thread::sleep(Duration::from_millis(100));
            drop(s);
            m.lock().unwrap().push(("close", Instant::now()));
        }) as Box<dyn FnOnce(TcpStream) + Send>
    };
    let port = serve(vec![
        failing(&marks),
        failing(&marks),
        failing(&marks),
        failing(&marks),
        good,
        failing(&marks),
    ]);
    // A longer backoff than the other tests use (100 ms, capped at 400): a retry is scheduled from
    // the moment its poll *began*, which can be a read timeout before the close was noticed, and that
    // offset must be small next to the gap between "restarted" and "at the cap".
    let timing = Timing {
        initial_backoff: Duration::from_millis(100),
        max_backoff: Duration::from_millis(400),
        ..fast()
    };
    let mut src = FreeDvSource::with_timing(cfg(port), timing);
    let run = pump(&mut src, Duration::from_secs(10), |s, _| {
        s.attempts() >= 6 && marks.lock().unwrap().len() >= 12
    });
    assert!(run.errors.len() >= 6, "{:?}", run.errors);
    let m = marks.lock().unwrap().clone();
    let accepts: Vec<Instant> = m
        .iter()
        .filter(|(k, _)| *k == "accept")
        .map(|(_, t)| *t)
        .collect();
    let closes: Vec<Instant> = m
        .iter()
        .filter(|(k, _)| *k == "close")
        .map(|(_, t)| *t)
        .collect();
    assert_eq!(accepts.len(), 6);
    // The wait between a session ending and the next attempt: 100, 200, 400 (the cap), 400 again
    // (a fourth failure would be 800 without the cap), then — after the good session — 100 again.
    let gap = |i: usize| accepts[i + 1].duration_since(closes[i]).as_millis();
    let (g0, g1, g2, g3, g4) = (gap(0), gap(1), gap(2), gap(3), gap(4));
    assert!((70..170).contains(&g0), "first wait {g0} ms");
    assert!((170..280).contains(&g1), "second wait {g1} ms (doubled)");
    assert!((370..480).contains(&g2), "third wait {g2} ms (at the cap)");
    assert!(
        (370..480).contains(&g3),
        "fourth wait {g3} ms (held at the cap)"
    );
    assert!(
        (70..170).contains(&g4),
        "the wait after a success was {g4} ms, not restarted at 100"
    );
}

/// The per-poll byte cap holds: a server streaming as fast as it can does not let one poll read it
/// all, so the source stays responsive.
#[test]
fn fr_spot_08_source_bounds_what_one_poll_reads() {
    let port = serve(vec![Box::new(|mut s| {
        join(&mut s);
        let mut blob = Vec::new();
        for i in 0..3000 {
            blob.extend(ev(
                "freq_change",
                &format!(r#"{{"sid":"s{i}","freq":14236000,"callsign":"AA1AAA"}}"#),
            ));
        }
        s.write_all(&blob).unwrap();
        thread::sleep(Duration::from_millis(1500));
    })]);
    let timing = Timing {
        max_bytes_per_poll: 16 * 1024,
        // Long enough that only the byte cap, not the time budget, can end a read.
        read_budget: Duration::from_secs(5),
        rate_per_sec: 1_000_000,
        ..fast()
    };
    let mut src = FreeDvSource::with_timing(cfg(port), timing);
    // Get joined (no spots yet), then let the stream arrive.
    let run = pump(&mut src, LONG, |s, _| s.state() == ConnState::Connected);
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    thread::sleep(Duration::from_millis(400));
    let mut first = Vec::new();
    src.poll(&mut |s| first.push(s)).unwrap();
    assert!(!first.is_empty(), "the first poll read something");
    assert!(
        first.len() < 1000,
        "one poll delivered {} of 3000: the byte cap is not applied",
        first.len()
    );
    // Everything still arrives over the following polls.
    let rest = pump(&mut src, LONG, |s, _| s.stations() == 3000);
    assert_eq!(src.stations(), 3000, "{}", rest.errors.len());
}
