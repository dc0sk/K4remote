//! Read-only K4 state probe (read-back session support). Connects on port 9205,
//! lets the `RDY;` state dump arrive, sends extra GETs for the screens we want to
//! read back, and prints every CAT response so we can see the exact RESP formats.
//!
//! GET-only — never transmits. Usage:
//!   K4PW=<password> cargo run -p k4remote --example probe -- <host>

use std::time::{Duration, Instant};

use k4_protocol::state::RadioState;
use k4_transport::{ConnectConfig, TcpRemoteTransport};

fn main() {
    let host = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "192.168.121.58".to_string());
    let password = std::env::var("K4PW").unwrap_or_default();
    if password.is_empty() {
        eprintln!("set K4PW=<password> in the environment");
        std::process::exit(2);
    }

    let cfg = ConnectConfig {
        password,
        read_timeout: Duration::from_millis(150),
        ..Default::default()
    };

    eprintln!("connecting to {host}:9204 (TLS-PSK) …");
    let mut t = match TcpRemoteTransport::connect_tls((host.as_str(), 9204u16), &cfg) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("connect failed: {e}");
            std::process::exit(1);
        }
    };
    eprintln!("connected; collecting the RDY state dump …");

    // Extra GETs for the read-back screens (all query forms; no SET/TX).
    let gets = [
        "RE;", "TE;", "KP;", "KS;", "MI;", "MG;", "MS;", "LI;", "LO;", "AN;", "AR;", "AR$;", "VX;",
        "VXV;", "BN;", "BN$;", "#REF;", "#REF$;", "#SPN;", "#SPN$;", "#SCL;", "#AVG;", "#PKM;",
        "#FRZ;", "#WFC;", "#WFC$;", "#WFH;", "#DPM;", "#NB;", "#NBL;", "PC;", "MG;",
    ];
    for g in gets {
        let _ = t.send_cat(g);
    }

    let mut seen: Vec<String> = Vec::new();
    let mut raw_frames = 0usize;
    let mut errors = 0usize;
    let mut radio = RadioState::new();
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        match t.poll_frames() {
            Ok(frames) => {
                for f in frames {
                    raw_frames += 1;
                    if let Some(text) = k4_protocol::cat::decode_cat_text(&f) {
                        for resp in text.split_inclusive(';') {
                            let resp = resp.trim();
                            if resp.is_empty() {
                                continue;
                            }
                            radio.apply_cat(resp); // feed the read-back parser
                            if !seen.iter().any(|s| s == resp) {
                                seen.push(resp.to_string());
                                println!("{resp}");
                            }
                        }
                    }
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => {
                errors += 1;
                if errors <= 3 {
                    eprintln!("read error: {} ({:?})", e, e.kind());
                }
            }
        }
    }
    let _ = t.disconnect();
    eprintln!(
        "done: {} raw frames, {} distinct CAT responses, {} errors",
        raw_frames,
        seen.len(),
        errors
    );
    eprintln!("\n=== read-back into RadioState (what the screens will show) ===");
    eprintln!(
        "VFO A / B      : {:?} / {:?} Hz",
        radio.vfo_a_hz, radio.vfo_b_hz
    );
    eprintln!("mode A / band  : {:?} / BN{:?}", radio.mode_a, radio.band);
    eprintln!("RX EQ (RE)     : {:?}", radio.rx_eq);
    eprintln!("TX EQ (TE)     : {:?}", radio.tx_eq);
    eprintln!(
        "keyer          : iambic_b={:?} paddle_rev={:?} weight={:?} speed={:?}",
        radio.keyer_iambic_b, radio.keyer_paddle_rev, radio.keyer_weight, radio.keyer_speed
    );
    eprintln!(
        "mic input/gain : {:?} / {:?}",
        radio.mic_input, radio.mic_gain
    );
    eprintln!(
        "line out L/R/gang: {:?}/{:?}/{:?}",
        radio.line_out_left, radio.line_out_right, radio.line_out_gang
    );
    eprintln!(
        "antennas TX/RX/sub: {:?}/{:?}/{:?}   VOX(voice)={:?}",
        radio.tx_antenna, radio.rx_antenna, radio.rx_antenna_sub, radio.vox_voice
    );
    eprintln!(
        "pan ref/span/scale/mode: {:?}/{:?}/{:?}/{:?}  wf palette/height: {:?}/{:?}",
        radio.pan_ref,
        radio.pan_span_hz,
        radio.pan_scale,
        radio.pan_mode,
        radio.wf_palette,
        radio.wf_height
    );
}
