//! The HTTP access for the polled spot networks (FR-SPOT-08): one bounded, anonymous GET.
//!
//! `k4-spot` stays free of an HTTP and TLS stack by taking a [`Fetcher`]; this is the one the app
//! gives it, built on the `ureq`/rustls the update check already uses. What it sends is a plain GET
//! with a `User-Agent` naming this program and its version, and an `Accept` header — no cookies, no
//! credentials, no callsign, nothing about the radio or the operator.
//!
//! It is bounded three ways: a **timeout** on the whole exchange, a **size cap** on the body
//! (refused, not truncated: a cut list is a wrong list), and **no redirects** (a feed that answers
//! with a redirect is reported, not followed to somewhere the operator never chose).

use std::sync::Arc;
use std::time::Duration;

use k4_spot::polled::Fetcher;

/// How long the whole request may take.
const TIMEOUT: Duration = Duration::from_secs(15);

/// The fetcher the app hands to a polled source.
pub fn fetcher() -> Fetcher {
    Arc::new(|url, max| fetch(url, max, TIMEOUT))
}

/// GET `url`, read at most `max` bytes, give up after `timeout`.
pub fn fetch(url: &str, max: usize, timeout: Duration) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .build()
        .into();
    let mut resp = agent
        .get(url)
        .header(
            "User-Agent",
            concat!("K4remote/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(c) => format!("the server returned HTTP {c}"),
            ureq::Error::Timeout(_) => format!("no reply within {} s", timeout.as_secs()),
            other => format!("could not reach the server: {other}"),
        })?;
    // With redirects off, a 3xx comes back as an ordinary response rather than an error: only a
    // 2xx is a reply.
    if !resp.status().is_success() {
        return Err(format!(
            "the server returned HTTP {}",
            resp.status().as_u16()
        ));
    }
    // Ask for one byte more than the cap and enforce the cap here, so "exactly at the cap is read,
    // one byte over is refused" does not depend on how the library treats its own limit. (ureq 3.3
    // errors when the body *reaches* its limit, so today it refuses one byte over by itself and the
    // length check below cannot fire; it is what keeps the cap right if that boundary ever moves.)
    let too_big = || format!("the reply is larger than {} KB", max / 1024);
    let body = resp
        .body_mut()
        .with_config()
        .limit((max as u64).saturating_add(1))
        .read_to_vec()
        .map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => too_big(),
            ureq::Error::Timeout(_) => format!("no reply within {} s", timeout.as_secs()),
            other => format!("could not read the reply: {other}"),
        })?;
    if body.len() > max {
        return Err(too_big());
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    /// Serve one connection with `reply` (raw bytes) after reading the request; return the port
    /// and a channel that yields the request text.
    fn serve(reply: Vec<u8>) -> (u16, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 4096];
            let mut req = Vec::new();
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => req.extend_from_slice(&buf[..n]),
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&req).into_owned());
            let _ = s.write_all(&reply);
            let _ = s.flush();
            // Let the client finish reading before the socket closes.
            thread::sleep(Duration::from_millis(100));
        });
        (port, rx)
    }

    fn ok(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    /// FR-SPOT-08: a good reply is returned whole, and the request is a plain GET that says who
    /// is asking and nothing about the operator or the radio.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_fetch_sends_an_anonymous_get() {
        let (port, req) = serve(ok(r#"[{"a":1}]"#));
        let body = fetch(
            &format!("http://127.0.0.1:{port}/spot/activator"),
            4096,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(body, br#"[{"a":1}]"#);
        let req = req.recv_timeout(Duration::from_secs(2)).unwrap();
        let lower = req.to_ascii_lowercase();
        assert!(req.starts_with("GET /spot/activator HTTP/1.1\r\n"), "{req}");
        assert!(
            lower.contains(&format!(
                "user-agent: k4remote/{}",
                env!("CARGO_PKG_VERSION")
            )),
            "{req}"
        );
        assert!(lower.contains("accept: application/json"), "{req}");
        for leak in ["cookie", "authorization", "x-", "referer"] {
            assert!(!lower.contains(leak), "the request carries {leak}: {req}");
        }
    }

    /// FR-SPOT-08: an error status, an over-size body, a redirect and a server that never answers
    /// are each an operator-readable error, and none of them is followed or waited on forever.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_fetch_failures_are_reported_and_bounded() {
        let go = |reply: Vec<u8>, max: usize, timeout: Duration| {
            let (port, _req) = serve(reply);
            fetch(&format!("http://127.0.0.1:{port}/x"), max, timeout)
        };
        let t = Duration::from_secs(5);

        let e = go(
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec(),
            4096,
            t,
        )
        .unwrap_err();
        assert_eq!(e, "the server returned HTTP 503");

        // Over the cap: refused, not truncated. Exactly at the cap is read.
        let big = "x".repeat(2000);
        let e = go(ok(&big), 1024, t).unwrap_err();
        assert!(e.contains("larger than 1 KB"), "{e}");
        assert_eq!(go(ok(&big), 2000, t).unwrap().len(), 2000);
        assert!(go(ok(&big), 1999, t).is_err(), "one byte over the cap");

        // A redirect is reported, never followed (the target here would be a live listener that
        // records if it was called).
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let tport = target.local_addr().unwrap().port();
        target.set_nonblocking(true).unwrap();
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:{tport}/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        let e = go(redirect.into_bytes(), 4096, t).unwrap_err();
        assert_eq!(e, "the server returned HTTP 302");
        thread::sleep(Duration::from_millis(150));
        assert!(
            target.accept().is_err(),
            "the redirect target was contacted"
        );

        // A server that accepts and then says nothing: the timeout ends it.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hold = thread::spawn(move || {
            let conn = listener.accept();
            thread::sleep(Duration::from_secs(3));
            drop(conn);
        });
        let started = Instant::now();
        let e = fetch(
            &format!("http://127.0.0.1:{port}/x"),
            4096,
            Duration::from_millis(400),
        )
        .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert!(e.contains("no reply within"), "{e}");
        drop(hold);

        // Nothing listening at all.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let cport = closed.local_addr().unwrap().port();
        drop(closed);
        let e = fetch(&format!("http://127.0.0.1:{cport}/x"), 4096, t).unwrap_err();
        assert!(e.starts_with("could not reach the server"), "{e}");
    }
}
