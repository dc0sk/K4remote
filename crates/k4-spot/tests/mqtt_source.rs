//! The MQTT spot source against a scripted mock broker on loopback (FR-SPOT-05, FR-SPOT-09).
//!
//! The broker side is parsed here by hand, independently of the crate's own decoder (which only
//! reads what a broker sends), so what the client puts on the wire is checked against the
//! MQTT 3.1.1 layout and not against itself.
//! trace: FR-SPOT-05, FR-SPOT-09

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use k4_spot::mqtt_source::{CertInfo, ConnectError, Connector, MqttConfig, MqttSource, Wire};
use k4_spot::telnet::{ConnState, Timing};
use k4_spot::{Network, Spot, SpotSource};

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

fn cfg(port: u16) -> MqttConfig {
    MqttConfig {
        network: Network::PskReporter,
        host: "127.0.0.1".into(),
        port,
        tls: false,
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A feed message in the live format, stamped now.
fn msg(call: &str, hz: u64) -> String {
    format!(
        r#"{{"sq":1,"f":{hz},"md":"FT8","rp":-9,"t":{},"sc":"{call}","rc":"BB2BBB","b":"20m"}}"#,
        now()
    )
}

/// A PUBLISH packet, QoS 0, built by hand from the specification.
fn publish(topic: &str, payload: &str) -> Vec<u8> {
    let mut body = (topic.len() as u16).to_be_bytes().to_vec();
    body.extend_from_slice(topic.as_bytes());
    body.extend_from_slice(payload.as_bytes());
    let mut out = vec![0x30];
    let mut n = body.len();
    loop {
        let mut b = (n % 128) as u8;
        n /= 128;
        if n > 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            break;
        }
    }
    out.extend(body);
    out
}

/// One packet from the client: its first byte and its body. `None` on end of stream or timeout.
fn read_packet(s: &mut TcpStream, wait: Duration) -> Option<(u8, Vec<u8>)> {
    s.set_read_timeout(Some(wait)).ok()?;
    let mut first = [0u8; 1];
    s.read_exact(&mut first).ok()?;
    let (mut len, mut mult) = (0usize, 1usize);
    loop {
        let mut b = [0u8; 1];
        s.read_exact(&mut b).ok()?;
        len += usize::from(b[0] & 0x7F) * mult;
        mult *= 128;
        if b[0] & 0x80 == 0 {
            break;
        }
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).ok()?;
    Some((first[0], body))
}

/// The topic filters in a SUBSCRIBE body (2-byte id, then length-prefixed topic + QoS byte each).
fn subscribe_topics(body: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 2;
    while i + 2 <= body.len() {
        let n = usize::from(u16::from_be_bytes([body[i], body[i + 1]]));
        out.push(String::from_utf8_lossy(&body[i + 2..i + 2 + n]).into_owned());
        i += 2 + n + 1;
    }
    out
}

/// The topic filters in an UNSUBSCRIBE body (no QoS byte).
fn unsubscribe_topics(body: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 2;
    while i + 2 <= body.len() {
        let n = usize::from(u16::from_be_bytes([body[i], body[i + 1]]));
        out.push(String::from_utf8_lossy(&body[i + 2..i + 2 + n]).into_owned());
        i += 2 + n;
    }
    out
}

const CONNACK_OK: [u8; 4] = [0x20, 0x02, 0x00, 0x00];
const SUBACK_OK: [u8; 5] = [0x90, 0x03, 0x00, 0x01, 0x00];
const TOPIC_20M: &str = "pskr/filter/v2/20m/#";

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

struct Run {
    spots: Vec<Spot>,
    errors: Vec<String>,
}

fn pump(
    src: &mut MqttSource,
    max: Duration,
    mut done: impl FnMut(&mut MqttSource, &Run) -> bool,
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

/// The source sends exactly a clean-session CONNECT with no credentials, subscribes to exactly
/// the topic it was given, delivers the in-window spots, and counts the bad and out-of-window ones.
#[test]
fn fr_spot_05_source_connects_subscribes_and_streams() {
    let (tx, rx) = mpsc::channel();
    let port = serve(vec![Box::new(move |mut s| {
        let (t, connect) = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        assert_eq!(t, 0x10, "the first packet is CONNECT");
        s.write_all(&CONNACK_OK).unwrap();
        let (t, sub) = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        assert_eq!(t, 0x82, "then SUBSCRIBE");
        s.write_all(&SUBACK_OK).unwrap();
        let topic = "pskr/filter/v2/20m/FT8/AA1AAA/BB2BBB/FN31/JO50/291/230";
        s.write_all(&publish(topic, &msg("AA1AAA", 14_074_742)))
            .unwrap();
        s.write_all(&publish(topic, "this is not json")).unwrap();
        s.write_all(&publish(topic, &msg("CC3CCC", 14_200_000)))
            .unwrap();
        s.write_all(&publish(topic, &msg("DD4DDD", 14_075_112)))
            .unwrap();
        // Anything else the client sends, for a while.
        let mut others = Vec::new();
        while let Some((t, _)) = read_packet(&mut s, Duration::from_millis(400)) {
            others.push(t);
        }
        tx.send((connect, subscribe_topics(&sub), others)).unwrap();
    })]);
    let mut src = MqttSource::with_timing(cfg(port), fast());
    src.set_topics(vec![TOPIC_20M.into()]);
    src.set_window(Some((14_000_000, 14_100_000)));
    let run = pump(&mut src, LONG, |_, r| r.spots.len() >= 2);

    let calls: Vec<&str> = run.spots.iter().map(|s| s.call.as_str()).collect();
    assert_eq!(calls, ["AA1AAA", "DD4DDD"]);
    assert_eq!(run.spots[0].freq_hz, 14_074_742);
    assert_eq!(run.spots[0].network, Network::PskReporter);
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(src.state(), ConnState::Connected);
    let st = src.stats();
    assert_eq!((st.spots, st.rejected, st.outside_window), (2, 1, 1));
    drop(src);

    let (connect, topics, others) = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    // CONNECT: protocol "MQTT" level 4, flags exactly clean-session (no username, password or will).
    assert_eq!(
        &connect[..8],
        &[0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02]
    );
    assert_eq!(&connect[8..10], &[0x00, 30], "keepalive 30 s");
    let id_len = usize::from(u16::from_be_bytes([connect[10], connect[11]]));
    let id = String::from_utf8_lossy(&connect[12..12 + id_len]).into_owned();
    assert!(id.starts_with("k4r-"), "{id}");
    assert_eq!(
        connect.len(),
        12 + id_len,
        "nothing after the client id: no credentials"
    );
    // SUBSCRIBE: exactly the one topic asked for, nothing broader.
    assert_eq!(topics, [TOPIC_20M]);
    // Nothing else is sent but keepalive pings and the closing DISCONNECT.
    assert!(
        others.iter().all(|t| *t == 0xC0 || *t == 0xE0),
        "unexpected packets: {others:?}"
    );
}

/// With no topics the source subscribes to nothing — it must never fall back to the whole feed.
#[test]
fn fr_spot_05_source_with_no_bands_subscribes_to_nothing() {
    let (tx, rx) = mpsc::channel();
    let port = serve(vec![Box::new(move |mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&CONNACK_OK).unwrap();
        let mut types = Vec::new();
        while let Some((t, _)) = read_packet(&mut s, Duration::from_millis(600)) {
            types.push(t);
        }
        tx.send(types).unwrap();
        // Keep the connection open: hanging up would make the client (rightly) report it.
        thread::sleep(Duration::from_secs(2));
    })]);
    let mut src = MqttSource::with_timing(cfg(port), fast());
    let _ = pump(&mut src, Duration::from_millis(900), |_, _| false);
    assert_eq!(src.state(), ConnState::Connected);
    drop(src);
    let types = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(!types.contains(&0x82), "no SUBSCRIBE at all: {types:?}");
}

/// A broker that refuses the connection is reported with its code, and the source backs off.
#[test]
fn fr_spot_05_source_reports_a_refused_connection() {
    let port = serve(vec![Box::new(|mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3));
        s.write_all(&[0x20, 0x02, 0x00, 0x05]).unwrap(); // not authorised
        thread::sleep(Duration::from_millis(300));
    })]);
    let mut src = MqttSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        run.errors[0].contains("refused the connection (code 5)"),
        "{:?}",
        run.errors
    );
    assert_eq!(src.state(), ConnState::Disconnected);
}

/// When the wanted bands change, the source drops the old topic and adds the new one, and only
/// those — it does not resubscribe to what it already has.
#[test]
fn fr_spot_05_source_follows_a_change_of_topics() {
    let (tx, rx) = mpsc::channel();
    let port = serve(vec![Box::new(move |mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&CONNACK_OK).unwrap();
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap(); // the first SUBSCRIBE
        s.write_all(&SUBACK_OK).unwrap();
        s.write_all(&publish("t", &msg("AA1AAA", 14_074_742)))
            .unwrap();
        let mut seen = Vec::new();
        while let Some((t, body)) = read_packet(&mut s, Duration::from_millis(800)) {
            match t {
                0x82 => seen.push(("subscribe", subscribe_topics(&body))),
                0xA2 => seen.push(("unsubscribe", unsubscribe_topics(&body))),
                _ => {}
            }
        }
        tx.send(seen).unwrap();
    })]);
    let mut src = MqttSource::with_timing(cfg(port), fast());
    src.set_topics(vec![TOPIC_20M.into(), "pskr/filter/v2/40m/#".into()]);
    let _ = pump(&mut src, LONG, |_, r| !r.spots.is_empty());
    // Keep 40m, drop 20m, add 15m.
    src.set_topics(vec![
        "pskr/filter/v2/40m/#".into(),
        "pskr/filter/v2/15m/#".into(),
    ]);
    let _ = pump(&mut src, Duration::from_millis(400), |_, _| false);
    drop(src);
    let seen = rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(
        seen.contains(&("unsubscribe", vec![TOPIC_20M.to_string()])),
        "{seen:?}"
    );
    assert!(
        seen.contains(&("subscribe", vec!["pskr/filter/v2/15m/#".to_string()])),
        "{seen:?}"
    );
    assert!(
        !seen
            .iter()
            .any(|(_, t)| t.contains(&"pskr/filter/v2/40m/#".to_string())),
        "the topic that stayed is not touched: {seen:?}"
    );
}

/// After the broker drops the connection the source reconnects by itself, subscribes again to the
/// same topics, and the spots keep coming.
#[test]
fn fr_spot_05_source_reconnects_and_resubscribes() {
    let (tx, rx) = mpsc::channel();
    let tx2 = tx.clone();
    let first: Box<dyn FnOnce(TcpStream) + Send> = Box::new(move |mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&CONNACK_OK).unwrap();
        let (_, sub) = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        tx.send(subscribe_topics(&sub)).unwrap();
        s.write_all(&SUBACK_OK).unwrap();
        s.write_all(&publish("t", &msg("AA1AAA", 14_074_742)))
            .unwrap();
        thread::sleep(Duration::from_millis(100));
        // dropped
    });
    let second: Box<dyn FnOnce(TcpStream) + Send> = Box::new(move |mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&CONNACK_OK).unwrap();
        let (_, sub) = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        tx2.send(subscribe_topics(&sub)).unwrap();
        s.write_all(&SUBACK_OK).unwrap();
        s.write_all(&publish("t", &msg("BB2BBB", 14_075_112)))
            .unwrap();
        thread::sleep(Duration::from_millis(800));
    });
    let port = serve(vec![first, second]);
    let mut src = MqttSource::with_timing(cfg(port), fast());
    src.set_topics(vec![TOPIC_20M.into()]);
    let run = pump(&mut src, LONG, |_, r| r.spots.len() >= 2);
    let calls: Vec<&str> = run.spots.iter().map(|s| s.call.as_str()).collect();
    assert_eq!(calls, ["AA1AAA", "BB2BBB"]);
    assert_eq!(src.stats().connects, 2);
    assert_eq!(
        run.errors.len(),
        1,
        "the drop is reported once: {:?}",
        run.errors
    );
    assert!(run.errors[0].contains("closed"), "{:?}", run.errors);
    assert_eq!(rx.recv().unwrap(), [TOPIC_20M]);
    assert_eq!(
        rx.recv().unwrap(),
        [TOPIC_20M],
        "subscribed again after reconnecting"
    );
}

/// A quiet band is not a dead connection: the source pings, the broker answers, and it stays up.
#[test]
fn fr_spot_05_source_keeps_a_quiet_connection_alive_with_pings() {
    let (tx, rx) = mpsc::channel();
    let port = serve(vec![Box::new(move |mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&CONNACK_OK).unwrap();
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&SUBACK_OK).unwrap();
        let mut pings = 0;
        while let Some((t, _)) = read_packet(&mut s, Duration::from_millis(600)) {
            if t == 0xC0 {
                pings += 1;
                s.write_all(&[0xD0, 0x00]).unwrap(); // PINGRESP
            }
        }
        tx.send(pings).unwrap();
    })]);
    let timing = Timing {
        keepalive: Duration::from_millis(200),
        ..fast()
    };
    let mut src = MqttSource::with_timing(cfg(port), timing);
    src.set_topics(vec![TOPIC_20M.into()]);
    let run = pump(&mut src, Duration::from_millis(1200), |_, _| false);
    assert!(
        run.errors.is_empty(),
        "a quiet but answering connection is healthy: {:?}",
        run.errors
    );
    assert_eq!(src.state(), ConnState::Connected);
    drop(src);
    assert!(
        rx.recv_timeout(Duration::from_secs(3)).unwrap() >= 3,
        "it pinged repeatedly"
    );
}

/// A broker that stops answering is noticed after twice the keepalive, and reported.
#[test]
fn fr_spot_05_source_detects_a_dead_broker() {
    let port = serve(vec![Box::new(|mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&CONNACK_OK).unwrap();
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&SUBACK_OK).unwrap();
        thread::sleep(Duration::from_secs(3)); // says nothing more, answers no pings
    })]);
    let timing = Timing {
        keepalive: Duration::from_millis(200),
        ..fast()
    };
    let mut src = MqttSource::with_timing(cfg(port), timing);
    src.set_topics(vec![TOPIC_20M.into()]);
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        run.errors
            .iter()
            .any(|e| e.contains("no data from the broker")),
        "{:?}",
        run.errors
    );
}

/// A broker that never answers the connection request is given up on with a clear message.
#[test]
fn fr_spot_05_source_gives_up_on_a_silent_broker() {
    let port = serve(vec![Box::new(|_s| thread::sleep(Duration::from_secs(2)))]);
    let mut src = MqttSource::with_timing(cfg(port), fast());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert!(
        run.errors
            .iter()
            .any(|e| e.contains("no reply to the connection request")),
        "{:?}",
        run.errors
    );
    assert_eq!(src.state(), ConnState::Disconnected);
}

/// A flood is shed at the rate cap, and every spot is either delivered or counted.
#[test]
fn fr_spot_05_source_sheds_a_flood() {
    const N: u64 = 3000;
    let port = serve(vec![Box::new(|mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&CONNACK_OK).unwrap();
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&SUBACK_OK).unwrap();
        let mut burst = Vec::new();
        for _ in 0..N {
            burst.extend(publish("t", &msg("AA1AAA", 14_074_742)));
        }
        s.write_all(&burst).unwrap();
        thread::sleep(Duration::from_secs(2));
    })]);
    let timing = Timing {
        rate_per_sec: 100,
        ..fast()
    };
    let mut src = MqttSource::with_timing(cfg(port), timing);
    src.set_topics(vec![TOPIC_20M.into()]);
    let run = pump(&mut src, Duration::from_millis(1500), |s, _| {
        s.stats().spots + s.stats().shed >= N
    });
    let st = src.stats();
    assert_eq!(
        st.spots + st.shed,
        N,
        "every message is delivered or counted: {st:?}"
    );
    assert!(st.spots <= 250, "the cap held: {} delivered", st.spots);
    assert_eq!(run.spots.len() as u64, st.spots);
}

/// A broker that sends garbage is dropped with a clear error and the source recovers on the next
/// connection: a packet claiming to be hundreds of megabytes never gets that much memory.
#[test]
fn fr_spot_05_source_survives_a_hostile_broker() {
    let first: Box<dyn FnOnce(TcpStream) + Send> = Box::new(|mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&CONNACK_OK).unwrap();
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&SUBACK_OK).unwrap();
        s.write_all(&[0x30, 0xFF, 0xFF, 0xFF, 0x7F]).unwrap(); // a PUBLISH claiming ~268 MB
        thread::sleep(Duration::from_millis(300));
    });
    let second: Box<dyn FnOnce(TcpStream) + Send> = Box::new(|mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&CONNACK_OK).unwrap();
        let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap();
        s.write_all(&SUBACK_OK).unwrap();
        s.write_all(&publish("t", &msg("AA1AAA", 14_074_742)))
            .unwrap();
        thread::sleep(Duration::from_millis(500));
    });
    let port = serve(vec![first, second]);
    let mut src = MqttSource::with_timing(cfg(port), fast());
    src.set_topics(vec![TOPIC_20M.into()]);
    let run = pump(&mut src, LONG, |_, r| !r.spots.is_empty());
    assert!(
        run.errors
            .iter()
            .any(|e| e.contains("larger than any spot feed")),
        "{:?}",
        run.errors
    );
    assert_eq!(run.spots.len(), 1, "and it recovered");
}

/// Nothing listening: reported once, then a backoff — not a tight loop.
#[test]
fn fr_spot_05_source_reports_failure_and_backs_off() {
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let mut src = MqttSource::with_timing(cfg(port), fast());
    let mut sink = |_: Spot| {};
    let first = src
        .poll(&mut sink)
        .expect_err("connection refused is an error");
    assert!(first.0.contains("connect to 127.0.0.1"), "{first}");
    assert_eq!(src.attempts(), 1);
    for _ in 0..3 {
        assert!(src.poll(&mut sink).is_ok(), "quiet inside the backoff");
    }
    assert_eq!(src.attempts(), 1);
    let _ = pump(&mut src, Duration::from_millis(500), |_, _| false);
    assert!(
        (3..=8).contains(&src.attempts()),
        "attempts: {}",
        src.attempts()
    );
}

/// A connector that counts its calls and hands out the scripted results in turn.
fn scripted(
    results: Vec<Result<u16, ConnectError>>,
) -> (Connector, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    let calls = Arc::new(AtomicUsize::new(0));
    let (c, queue) = (Arc::clone(&calls), Mutex::new(results.into_iter()));
    let connector: Connector = Arc::new(move |_host, _port, _timeout| {
        c.fetch_add(1, Ordering::SeqCst);
        match queue.lock().unwrap().next() {
            Some(Ok(port)) => {
                let s = TcpStream::connect(("127.0.0.1", port)).unwrap();
                s.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
                Ok(Box::new(s) as Box<dyn Wire>)
            }
            Some(Err(e)) => Err(e),
            None => Err(ConnectError::Failed("script ran out".into())),
        }
    });
    (connector, calls)
}

fn cert_info(changed: bool) -> CertInfo {
    CertInfo {
        host: "127.0.0.1".into(),
        port: 1884,
        sha256: "ab".repeat(32),
        reason: "unknown issuer".into(),
        changed,
    }
}

/// Asking for TLS with no TLS connector is an error, never a quiet fall back to plain text; and
/// each configuration uses only its own connector.
#[test]
fn fr_spot_05_tls_never_falls_back_to_plain_text() {
    use std::sync::atomic::Ordering;
    let (plain, plain_calls) = scripted(vec![Err(ConnectError::Failed("plain was used".into()))]);
    let mut src = MqttSource::with_timing(
        MqttConfig {
            tls: true,
            ..cfg(1884)
        },
        fast(),
    );
    src.set_plain_connector(plain.clone());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert_eq!(run.errors[0], "encrypted connections are not available");
    assert_eq!(
        plain_calls.load(Ordering::SeqCst),
        0,
        "plain text was tried"
    );

    // With a TLS connector, TLS is what is used and plain is not.
    let (tls, tls_calls) = scripted(vec![Err(ConnectError::Failed("tls was used".into()))]);
    let mut src = MqttSource::with_timing(
        MqttConfig {
            tls: true,
            ..cfg(1884)
        },
        fast(),
    );
    src.set_plain_connector(plain.clone());
    src.set_tls_connector(tls.clone());
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert_eq!(run.errors[0], "tls was used");
    assert_eq!(tls_calls.load(Ordering::SeqCst), 1);
    assert_eq!(plain_calls.load(Ordering::SeqCst), 0);

    // A plain configuration never touches the TLS connector.
    let (tls2, tls2_calls) = scripted(vec![Err(ConnectError::Failed("tls was used".into()))]);
    let (plain2, plain2_calls) = scripted(vec![Err(ConnectError::Failed("plain was used".into()))]);
    let mut src = MqttSource::with_timing(cfg(1883), fast());
    src.set_plain_connector(plain2);
    src.set_tls_connector(tls2);
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert_eq!(run.errors[0], "plain was used");
    assert_eq!(plain2_calls.load(Ordering::SeqCst), 1);
    assert_eq!(tls2_calls.load(Ordering::SeqCst), 0);
}

/// An untrusted certificate is reported with its reason and kept for the operator to decide on,
/// the source does not hammer the server while waiting, `retry_now` tries again at once, and a
/// good connection clears the pending certificate.
#[test]
fn fr_spot_05_untrusted_certificate_is_kept_until_decided() {
    use std::sync::atomic::Ordering;
    let port = serve(vec![Box::new(|mut s| {
        let _ = read_packet(&mut s, Duration::from_secs(3));
        s.write_all(&[0x20, 0x02, 0x00, 0x00]).unwrap();
        thread::sleep(Duration::from_millis(600));
    })]);
    let (tls, calls) = scripted(vec![
        Err(ConnectError::Untrusted(cert_info(false))),
        Err(ConnectError::Untrusted(cert_info(true))),
        Ok(port),
    ]);
    let timing = Timing {
        initial_backoff: Duration::from_millis(150),
        max_backoff: Duration::from_secs(60),
        ..fast()
    };
    let mut src = MqttSource::with_timing(
        MqttConfig {
            tls: true,
            ..cfg(1884)
        },
        timing,
    );
    src.set_tls_connector(tls);
    assert!(src.pending_cert().is_none(), "nothing pending at the start");

    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert_eq!(
        run.errors[0],
        "the server's certificate is not trusted (unknown issuer)"
    );
    assert_eq!(src.pending_cert(), Some(&cert_info(false)));
    assert_eq!(src.state(), ConnState::Disconnected);

    // Inside the backoff, polling does not reconnect.
    let t0 = Instant::now();
    while t0.elapsed() < Duration::from_millis(80) {
        assert!(src.poll(&mut |_| {}).is_ok());
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "retried during the backoff"
    );

    // Approved: retry at once. The second attempt shows the certificate has changed.
    src.retry_now();
    let run = pump(&mut src, LONG, |_, r| !r.errors.is_empty());
    assert_eq!(
        run.errors[0],
        "the server's certificate is not trusted (unknown issuer) — and it is not the one you approved"
    );
    assert_eq!(src.pending_cert(), Some(&cert_info(true)));

    // Then it is trusted. `retry_now` restarted the backoff, so the next attempt comes after the
    // initial 150 ms, not the doubled 300 ms it would be otherwise: connect within 250 ms.
    let run = pump(&mut src, Duration::from_millis(250), |s, _| {
        s.state() == ConnState::Connected
    });
    assert!(run.errors.is_empty(), "{:?}", run.errors);
    assert_eq!(src.state(), ConnState::Connected);
    assert!(src.pending_cert().is_none(), "cleared by a good connection");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}
