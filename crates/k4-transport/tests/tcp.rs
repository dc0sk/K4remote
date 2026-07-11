//! L2 live integration: real TCP transport against the `k4-sim` loopback server.
//! No hardware (NFR-TEST-02). trace: FR-CONN-01, FR-CONN-02, FR-AUTH-03, FR-SES-PING
use k4_sim::SimServer;
use k4_transport::{ConnectConfig, TcpRemoteTransport};

/// Read CAT responses until one starts with `prefix` (bounded so a failure does
/// not hang on the socket read timeout indefinitely).
fn read_until(t: &mut TcpRemoteTransport, prefix: &str) -> Option<String> {
    for _ in 0..10 {
        if let Ok(messages) = t.poll_cat() {
            if let Some(hit) = messages.into_iter().find(|m| m.starts_with(prefix)) {
                return Some(hit);
            }
        }
    }
    None
}

/// Full happy-path: connect + auth + init sequence, a GET round-trip, and a
/// keep-alive round-trip, plus verification that the server received the init
/// commands in the documented order.
///
/// trace: FR-CONN-01, FR-AUTH-03, FR-SES-PING
#[test]
fn l2_live_connect_handshake_get_and_ping_roundtrip() {
    let server = SimServer::start("secret", 14_074_000).unwrap();
    let cfg = ConnectConfig {
        password: "secret".into(),
        ..Default::default()
    };

    let mut t = TcpRemoteTransport::connect(server.addr(), &cfg).unwrap();

    // GET VFO A.
    t.send_cat("FA;").unwrap();
    assert_eq!(read_until(&mut t, "FA").as_deref(), Some("FA00014074000;"));

    // Keep-alive (FR-SES-PING): timestamped PING, server replies PONG.
    t.send_cat("PING1000;").unwrap();
    assert_eq!(read_until(&mut t, "PONG").as_deref(), Some("PONG;"));

    // FR-AUTH-03: the post-auth init sequence reached the server, in order.
    let received = server.received();
    let init: Vec<&str> = received
        .iter()
        .map(String::as_str)
        .filter(|c| ["RDY;", "K41;", "ER1;", "EM3;", "SL2;"].contains(c))
        .collect();
    assert_eq!(init, vec!["RDY;", "K41;", "ER1;", "EM3;", "SL2;"]);
}

/// Clean disconnect emits `RRN;` (FR-CONN-02).
///
/// trace: FR-CONN-02
#[test]
fn l2_disconnect_sends_rrn() {
    let server = SimServer::start("pw", 7_000_000).unwrap();
    let cfg = ConnectConfig {
        password: "pw".into(),
        ..Default::default()
    };
    let mut t = TcpRemoteTransport::connect(server.addr(), &cfg).unwrap();

    // Round-trip first so the server has surely processed the init burst.
    t.send_cat("FA;").unwrap();
    let _ = read_until(&mut t, "FA");

    t.disconnect().unwrap();
    // Give the server thread a moment to read the RRN; frame, then assert.
    let mut saw_rrn = false;
    for _ in 0..10 {
        if server.received().iter().any(|c| c == "RRN;") {
            saw_rrn = true;
            break;
        }
        let _ = t.poll_cat();
    }
    assert!(saw_rrn, "server should have received RRN;");
}

/// A connect to an unreachable (blackhole) address fails within the configured
/// connect timeout rather than blocking on the OS default.
///
/// trace: FR-CONN-05
#[test]
fn fr_conn_05_connect_respects_timeout() {
    use std::time::{Duration, Instant};
    // 192.0.2.1 is TEST-NET-1 (RFC 5737) — guaranteed non-routable, so the
    // connect either drops (times out) or is refused; both must be prompt.
    let cfg = ConnectConfig {
        connect_timeout: Duration::from_millis(300),
        ..Default::default()
    };
    let start = Instant::now();
    let r = TcpRemoteTransport::connect("192.0.2.1:9205", &cfg);
    assert!(r.is_err(), "connect to a blackhole must fail");
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "must honour the connect timeout, not the multi-minute OS default (took {:?})",
        start.elapsed()
    );
}
