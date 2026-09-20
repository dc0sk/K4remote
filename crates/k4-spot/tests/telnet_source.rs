//! The telnet spot source against a scripted mock cluster on loopback (FR-SPOT-07, FR-SPOT-09).
//! The mock behaves as the real RBN relay was observed to: it sends `Please enter your call: `
//! with no newline, waits for a line, then streams `DX de` lines.
//! trace: FR-SPOT-07, FR-SPOT-09

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use k4_spot::telnet::{ConnState, TelnetConfig, TelnetSource, Timing};
use k4_spot::{Network, Spot, SpotSource};

/// Short timeouts so failure cases finish quickly; rates and sizes as the real defaults.
fn fast() -> Timing {
    Timing {
        connect_timeout: Duration::from_secs(1),
        prompt_timeout: Duration::from_millis(400),
        initial_backoff: Duration::from_millis(30),
        max_backoff: Duration::from_millis(120),
        idle_timeout: Duration::from_secs(5),
        read_budget: Duration::from_millis(50),
        ..Timing::default()
    }
}

fn cfg(port: u16) -> TelnetConfig {
    TelnetConfig {
        network: Network::Rbn,
        host: "127.0.0.1".into(),
        port,
        login: "dc0sk".into(),
    }
}

/// A cluster line; the time of day is irrelevant here.
fn line(call: &str, khz: f64) -> String {
    format!("DX de K1TTT-#: {khz:>9.1} {call:<12} CW 30 dB 20 WPM CQ 1200Z\r\n")
}

/// Listen on a free loopback port and run one script per accepted connection, in order.
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

/// Send the prompt, read the login line, and return it.
fn prompt_and_login(s: &mut TcpStream, prompt: &[u8]) -> String {
    s.write_all(prompt).unwrap();
    let mut login = String::new();
    BufReader::new(s.try_clone().unwrap())
        .read_line(&mut login)
        .unwrap();
    login
}

struct Run {
    spots: Vec<Spot>,
    errors: Vec<String>,
}

/// Poll until `done` says so (or `max` passes), collecting spots and errors.
fn pump(
    src: &mut TelnetSource,
    max: Duration,
    mut done: impl FnMut(&TelnetSource, &Run) -> bool,
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

/// The source logs in with exactly the callsign and nothing else, delivers the spots, counts the
/// malformed ones and the over-long line, and reports no error.
#[test]
fn fr_spot_07_source_logs_in_and_streams() {
    let (tx, rx) = mpsc::channel();
    let port = serve(vec![Box::new(move |mut s| {
        let login = prompt_and_login(&mut s, b"Please enter your call: ");
        // Anything more the client sends after the login line?
        s.set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let mut extra = Vec::new();
        s.write_all(b"Hello DC0SK, welcome\r\n").unwrap();
        s.write_all(line("NP2X", 7000.7).as_bytes()).unwrap();
        s.write_all(b"DX de garbage that is not a spot\r\n")
            .unwrap();
        s.write_all(format!("{}\r\n", "x".repeat(400)).as_bytes())
            .unwrap();
        s.write_all(line("OM7CM", 3580.9).as_bytes()).unwrap();
        s.write_all(line("W1AW", 14074.0).as_bytes()).unwrap();
        let _ = s.read_to_end(&mut extra); // ends on the read timeout or when the client drops
        tx.send((login, extra)).unwrap();
    })]);
    let mut src = TelnetSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| r.spots.len() >= 3);

    let calls: Vec<&str> = run.spots.iter().map(|s| s.call.as_str()).collect();
    assert_eq!(calls, ["NP2X", "OM7CM", "W1AW"]);
    assert_eq!(run.spots[0].freq_hz, 7_000_700);
    assert_eq!(run.spots[0].network, Network::Rbn);
    assert!(
        run.errors.is_empty(),
        "no errors on a healthy feed: {:?}",
        run.errors
    );
    assert_eq!(src.state(), ConnState::Connected);
    let st = src.stats();
    assert_eq!(
        (st.spots, st.rejected, st.overlong, st.connects),
        (3, 1, 1, 1)
    );
    assert_eq!(src.attempts(), 1);
    drop(src);
    let (login, extra) = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        login, "DC0SK\r\n",
        "the login is the callsign, normalised, and a line ending"
    );
    assert!(extra.is_empty(), "nothing else is ever sent: {extra:?}");
}

/// Different prompt wordings work, and telnet negotiation before the prompt is ignored.
#[test]
fn fr_spot_07_source_accepts_other_prompts_and_ignores_negotiation() {
    for prompt in [
        &b"login: "[..],
        &b"\xff\xfb\x01\xff\xfd\x18Enter your callsign: "[..],
    ] {
        let for_server = prompt.to_vec();
        let port = serve(vec![Box::new(move |mut s| {
            let _ = prompt_and_login(&mut s, &for_server);
            s.write_all(line("NP2X", 7000.7).as_bytes()).unwrap();
            thread::sleep(Duration::from_millis(500));
        })]);
        let mut src = TelnetSource::with_timing(cfg(port), fast());
        let run = pump(&mut src, LONG, |_, r| !r.spots.is_empty());
        assert_eq!(
            run.spots.len(),
            1,
            "prompt {:?}",
            String::from_utf8_lossy(prompt)
        );
    }
}

/// A refused connection is reported once, then the source backs off instead of hammering.
#[test]
fn fr_spot_07_source_reports_failure_and_backs_off() {
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead); // nothing listens here now
    let mut src = TelnetSource::with_timing(cfg(port), fast());

    let mut sink = |_: Spot| {};
    let first = src
        .poll(&mut sink)
        .expect_err("connection refused is an error");
    assert!(first.0.contains("connect to 127.0.0.1"), "{first}");
    assert_eq!(src.attempts(), 1);
    assert_eq!(src.state(), ConnState::Disconnected);
    // Straight away: quiet, no new attempt.
    for _ in 0..3 {
        assert!(src.poll(&mut sink).is_ok());
    }
    assert_eq!(src.attempts(), 1, "still inside the backoff");
    // Over half a second the backoff (30, 60, 120, 120 ms…) allows a handful of attempts, not hundreds.
    let run = pump(&mut src, Duration::from_millis(500), |_, _| false);
    assert!(
        (3..=8).contains(&src.attempts()),
        "attempts: {}",
        src.attempts()
    );
    assert!(!run.errors.is_empty());
}

/// A dropped connection is reported, then re-established by itself, and the spots keep coming.
#[test]
fn fr_spot_07_source_reconnects_after_a_drop() {
    let first: Box<dyn FnOnce(TcpStream) + Send> = Box::new(|mut s| {
        let _ = prompt_and_login(&mut s, b"Please enter your call: ");
        s.write_all(line("NP2X", 7000.7).as_bytes()).unwrap();
        thread::sleep(Duration::from_millis(100));
        // dropped: the server closes
    });
    let second: Box<dyn FnOnce(TcpStream) + Send> = Box::new(|mut s| {
        let _ = prompt_and_login(&mut s, b"Please enter your call: ");
        s.write_all(line("W1AW", 14074.0).as_bytes()).unwrap();
        thread::sleep(Duration::from_millis(800));
    });
    let port = serve(vec![first, second]);
    let mut src = TelnetSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| r.spots.len() >= 2);
    let calls: Vec<&str> = run.spots.iter().map(|s| s.call.as_str()).collect();
    assert_eq!(calls, ["NP2X", "W1AW"]);
    assert_eq!(src.stats().connects, 2);
    assert_eq!(
        run.errors.len(),
        1,
        "the drop is reported once: {:?}",
        run.errors
    );
    assert!(run.errors[0].contains("closed"), "{:?}", run.errors);
    assert_eq!(src.state(), ConnState::Connected);
}

/// A server that never asks for a login is abandoned with a clear message, not waited on forever.
#[test]
fn fr_spot_07_source_gives_up_on_a_silent_server() {
    let port = serve(vec![Box::new(|_s| thread::sleep(Duration::from_secs(2)))]);
    let mut src = TelnetSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        run.errors.iter().any(|e| e.contains("no login prompt")),
        "{:?}",
        run.errors
    );
    assert_eq!(src.state(), ConnState::Disconnected);
}

/// A logged-in connection that goes silent is treated as dead and re-established.
#[test]
fn fr_spot_07_source_detects_an_idle_connection() {
    let port = serve(vec![Box::new(|mut s| {
        let _ = prompt_and_login(&mut s, b"Please enter your call: ");
        thread::sleep(Duration::from_secs(2)); // says nothing more
    })]);
    let timing = Timing {
        idle_timeout: Duration::from_millis(250),
        ..fast()
    };
    let mut src = TelnetSource::with_timing(cfg(port), timing);
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        run.errors.iter().any(|e| e.contains("no data")),
        "{:?}",
        run.errors
    );
}

/// A flood is shed at the rate cap, and every spot is either delivered or counted.
#[test]
fn fr_spot_07_source_sheds_a_flood() {
    const N: u64 = 3000;
    let port = serve(vec![Box::new(|mut s| {
        let _ = prompt_and_login(&mut s, b"Please enter your call: ");
        let mut burst = String::new();
        for _ in 0..N {
            burst.push_str(&line("N2XX", 14074.0));
        }
        s.write_all(burst.as_bytes()).unwrap();
        thread::sleep(Duration::from_secs(2));
    })]);
    let timing = Timing {
        rate_per_sec: 100,
        ..fast()
    };
    let mut src = TelnetSource::with_timing(cfg(port), timing);
    let run = pump(&mut src, Duration::from_millis(1200), |s, _| {
        s.stats().spots + s.stats().shed >= N
    });
    let st = src.stats();
    assert_eq!(
        st.spots + st.shed,
        N,
        "every spot is delivered or counted: {st:?}"
    );
    assert!(st.spots <= 250, "the cap held: {} delivered", st.spots);
    assert_eq!(run.spots.len() as u64, st.spots);
}

/// Spots outside the wanted window are dropped before they use up the rate budget.
#[test]
fn fr_spot_07_source_filters_to_a_window() {
    let port = serve(vec![Box::new(|mut s| {
        let _ = prompt_and_login(&mut s, b"Please enter your call: ");
        s.write_all(line("NP2X", 7000.7).as_bytes()).unwrap();
        s.write_all(line("W1AW", 14074.0).as_bytes()).unwrap();
        s.write_all(line("K1ABC", 14200.0).as_bytes()).unwrap();
        thread::sleep(Duration::from_millis(600));
    })]);
    let mut src = TelnetSource::with_timing(cfg(port), fast());
    src.set_window(Some((14_070_000, 14_078_000)));
    let run = pump(&mut src, LONG, |s, _| {
        s.stats().spots + s.stats().outside_window >= 3
    });
    let calls: Vec<&str> = run.spots.iter().map(|s| s.call.as_str()).collect();
    assert_eq!(calls, ["W1AW"]);
    assert_eq!(src.stats().outside_window, 2);
}

/// Two megabytes with no newline neither stalls the source nor grows it, and the feed carries on.
#[test]
fn fr_spot_07_source_survives_endless_junk() {
    let port = serve(vec![Box::new(|mut s| {
        let _ = prompt_and_login(&mut s, b"Please enter your call: ");
        let junk = vec![b'x'; 64 * 1024];
        for _ in 0..32 {
            if s.write_all(&junk).is_err() {
                return;
            }
        }
        s.write_all(b"\r\n").unwrap();
        s.write_all(line("W1AW", 14074.0).as_bytes()).unwrap();
        thread::sleep(Duration::from_millis(500));
    })]);
    let t0 = Instant::now();
    let mut src = TelnetSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| !r.spots.is_empty());
    assert_eq!(run.spots.len(), 1);
    assert_eq!(src.stats().overlong, 1, "one endless line is one event");
    assert!(
        t0.elapsed() < Duration::from_secs(4),
        "did not stall: {:?}",
        t0.elapsed()
    );
}

/// Bad settings are reported before any connection is made, and not retried in a loop.
#[test]
fn fr_spot_07_source_validates_its_settings() {
    for (login, host, want) in [
        ("", "127.0.0.1", "login callsign is required"),
        ("  ", "127.0.0.1", "login callsign is required"),
        ("CQ", "127.0.0.1", "not a valid callsign"),
        ("DC0\r\nSK", "127.0.0.1", "not a valid callsign"),
        ("DC0SK", "", "no host"),
    ] {
        let c = TelnetConfig {
            network: Network::Rbn,
            host: host.into(),
            port: 7000,
            login: login.into(),
        };
        let mut src = TelnetSource::with_timing(c, fast());
        let mut sink = |_: Spot| {};
        let e = src.poll(&mut sink).expect_err(login);
        assert!(e.0.contains(want), "{login:?}/{host:?}: {e}");
        assert_eq!(src.attempts(), 0, "no connection is attempted");
        assert!(
            src.poll(&mut sink).is_ok(),
            "and it does not repeat the error every call"
        );
    }
}
