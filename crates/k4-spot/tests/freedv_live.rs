//! A manual probe of the real FreeDV Reporter, never run by the suite:
//!
//! `cargo test -p k4-spot --test freedv_live -- --ignored --nocapture`
//!
//! It joins `qso.freedv.org` in the **read-only `view` role** for at most ten seconds and hangs up.
//! What it sends is the WebSocket upgrade (with a `User-Agent`), the `view` connect and the pongs
//! the protocol requires — nothing identifying. It prints **counts only**, never a callsign: how
//! far the session got, how many stations the roster holds, how many events were malformed. That
//! is what decides whether the format built from another client's source matches the service.

use std::time::{Duration, Instant};

use k4_spot::freedv_source::{FreeDvConfig, FreeDvSource};
use k4_spot::telnet::ConnState;
use k4_spot::SpotSource;

#[test]
#[ignore = "contacts the real FreeDV Reporter"]
fn live_freedv_reporter() {
    let mut src = FreeDvSource::new(FreeDvConfig {
        host: "qso.freedv.org".into(),
        port: 80,
        user_agent: "K4remote-live-probe".into(),
    });
    let t0 = Instant::now();
    let (mut spots, mut errors, mut joined_at) = (0usize, Vec::new(), None);
    let mut freqs: Vec<u64> = Vec::new();
    while t0.elapsed() < Duration::from_secs(10) {
        if let Err(e) = src.poll(&mut |s| {
            spots += 1;
            freqs.push(s.freq_hz);
        }) {
            errors.push(e.to_string());
            break;
        }
        if joined_at.is_none() && src.state() == ConnState::Connected {
            joined_at = Some(t0.elapsed());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let st = src.stats();
    freqs.sort_unstable();
    freqs.dedup();
    println!(
        "PROBE: state={:?} joined_after={:?}",
        src.state(),
        joined_at
    );
    println!(
        "PROBE: stations={} spots_delivered={} distinct_frequencies={} rejected={} connects={}",
        src.stations(),
        spots,
        freqs.len(),
        st.rejected,
        st.connects
    );
    if let (Some(lo), Some(hi)) = (freqs.first(), freqs.last()) {
        println!("PROBE: frequency range {lo}..{hi} Hz");
    }
    println!("PROBE: errors={errors:?}");
    for (event, (count, shape)) in src.rejected_shapes() {
        println!("PROBE: rejected {count} x {event}: {shape}");
    }
}
