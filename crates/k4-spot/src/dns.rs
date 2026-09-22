//! Bounded DNS resolution (FR-SPOT-14).
//!
//! `ToSocketAddrs` blocks on the OS resolver with **no timeout of its own** — a network with no
//! reachable DNS server can make it hang far longer than any connect timeout the app applies
//! afterward, and that hang happens on the shared thread every spot source is polled from
//! (`app/src/spot_sources.rs`), so it stalls polling and status updates for **every** enabled
//! network, not just the one whose host cannot be resolved. [`resolve_bounded`] puts the
//! resolution on its own thread and gives up waiting at `timeout`, the same tradeoff the
//! OS-keychain read makes for the same reason (`app/src/main.rs`'s `load_secret_timed`).

use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

/// Resolve `host:port`, abandoning the wait at `timeout` if the OS resolver has not answered by
/// then. The spawned thread is not killed — Rust cannot do that — it is simply abandoned and its
/// eventual result discarded, exactly as the keychain read abandons a hung Secret Service call.
pub fn resolve_bounded(
    host: &str,
    port: u16,
    timeout: Duration,
) -> std::io::Result<Vec<SocketAddr>> {
    resolve_bounded_with(host, port, timeout, |h, p| {
        (h, p).to_socket_addrs().map(|a| a.collect())
    })
}

/// As [`resolve_bounded`], with the blocking resolver call injected, so a hung or slow resolver
/// can be exercised without depending on real DNS or real time.
fn resolve_bounded_with<F>(
    host: &str,
    port: u16,
    timeout: Duration,
    resolver: F,
) -> std::io::Result<Vec<SocketAddr>>
where
    F: FnOnce(&str, u16) -> std::io::Result<Vec<SocketAddr>> + Send + 'static,
{
    let host_for_thread = host.to_string();
    let host_for_error = host.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(resolver(&host_for_thread, port));
    });
    rx.recv_timeout(timeout).unwrap_or_else(|_| {
        Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("resolving {host_for_error} did not finish within {timeout:?}"),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};
    use std::time::Instant;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::from(([127, 0, 0, 1], port))
    }

    /// A resolver that answers immediately passes its result straight through, whichever it is.
    ///
    /// trace: FR-SPOT-14
    #[test]
    fn fr_spot_14_a_prompt_resolver_passes_through() {
        let ok = resolve_bounded_with("host", 80, Duration::from_secs(1), |_, p| Ok(vec![addr(p)]));
        assert_eq!(ok.unwrap(), vec![addr(80)]);

        let err = resolve_bounded_with("bad.invalid", 80, Duration::from_secs(1), |_, _| {
            Err(Error::new(ErrorKind::NotFound, "no such host"))
        });
        assert_eq!(err.unwrap_err().kind(), ErrorKind::NotFound);
    }

    /// The whole point: a resolver that never returns must not block the caller past `timeout`,
    /// and must report a `TimedOut` error naming the host, not silently hang or panic.
    ///
    /// trace: FR-SPOT-14
    #[test]
    fn fr_spot_14_a_hung_resolver_times_out_instead_of_blocking_forever() {
        let start = Instant::now();
        let result =
            resolve_bounded_with("stuck.example", 80, Duration::from_millis(50), |_, _| {
                std::thread::sleep(Duration::from_secs(3600));
                Ok(Vec::new())
            });
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "must return promptly at the bound, not wait for the hung thread: {:?}",
            start.elapsed()
        );
        let err = result.expect_err("a hung resolver must report as an error, not succeed emptily");
        assert_eq!(err.kind(), ErrorKind::TimedOut);
        assert!(
            err.to_string().contains("stuck.example"),
            "the error should name the host that hung: {err}"
        );
    }

    /// A resolver that is merely slow, but still faster than the bound, must not be penalised —
    /// bounding the wait is not the same as bounding it too tightly.
    ///
    /// trace: FR-SPOT-14
    #[test]
    fn fr_spot_14_a_slow_but_timely_resolver_still_succeeds() {
        let result =
            resolve_bounded_with("slow.example", 53, Duration::from_millis(500), |_, p| {
                std::thread::sleep(Duration::from_millis(50));
                Ok(vec![addr(p)])
            });
        assert_eq!(result.unwrap(), vec![addr(53)]);
    }
}
