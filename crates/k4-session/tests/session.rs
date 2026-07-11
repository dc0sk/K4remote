//! Session tests. trace: FR-SES-01, FR-SES-02, FR-SES-PING, FR-CAT-06, FR-CAT-07,
//! FR-TX-SAFE-01, FR-TX-SAFE-03, FR-TX-SAFE-04
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::time::{Duration, Instant};

use k4_protocol::cat::{decode_cat_text, encode_cat_payload};
use k4_protocol::cw::KeyElement::{Dah, Dit};
use k4_protocol::state::{connect_state_seed, Mode};
use k4_session::{Clock, Session, SessionConfig, SessionEvent};
use k4_transport::CatLink;

// --- shared-handle mock link ------------------------------------------------

#[derive(Debug, Default)]
struct LinkState {
    sent: Vec<String>,
    /// Each entry is one `poll_frames` batch of raw frame payloads.
    inbound: VecDeque<Vec<Vec<u8>>>,
    /// When set, `poll_frames` returns an I/O error (simulates a dead socket).
    fail: bool,
}

/// Cloneable handle: a clone given to the `Session`, a clone kept by the test.
#[derive(Debug, Clone, Default)]
struct MockLink(Rc<RefCell<LinkState>>);

impl MockLink {
    /// Queue a batch of CAT responses (wrapped as CAT payloads).
    fn queue(&self, msgs: &[&str]) {
        let batch = msgs.iter().map(|m| encode_cat_payload(m)).collect();
        self.0.borrow_mut().inbound.push_back(batch);
    }
    /// Queue a batch of arbitrary raw frame payloads.
    fn queue_payloads(&self, payloads: Vec<Vec<u8>>) {
        self.0.borrow_mut().inbound.push_back(payloads);
    }
    /// Make the next `poll_frames` return an I/O error.
    fn fail(&self) {
        self.0.borrow_mut().fail = true;
    }
    fn sent(&self) -> Vec<String> {
        self.0.borrow().sent.clone()
    }
    fn last_sent(&self) -> Option<String> {
        self.0.borrow().sent.last().cloned()
    }
}

impl CatLink for MockLink {
    fn send_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        // Record CAT commands as their text; other frames by a size marker.
        let entry =
            decode_cat_text(payload).unwrap_or_else(|| format!("<frame:{}>", payload.len()));
        self.0.borrow_mut().sent.push(entry);
        Ok(())
    }
    fn poll_frames(&mut self) -> io::Result<Vec<Vec<u8>>> {
        if self.0.borrow().fail {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "socket reset",
            ));
        }
        Ok(self.0.borrow_mut().inbound.pop_front().unwrap_or_default())
    }
}

// --- controllable clock -----------------------------------------------------

#[derive(Debug)]
struct FakeClock {
    instant: Cell<Instant>,
    secs: Cell<u64>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            instant: Cell::new(Instant::now()),
            secs: Cell::new(1_000),
        }
    }
    fn advance(&self, d: Duration) {
        self.instant.set(self.instant.get() + d);
        self.secs.set(self.secs.get() + d.as_secs());
    }
}

/// Local newtype so we can `impl Clock` (orphan rule) while sharing the clock
/// between the `Session` and the test driver.
#[derive(Debug, Clone)]
struct SharedClock(Rc<FakeClock>);

impl Clock for SharedClock {
    fn now(&self) -> Instant {
        self.0.instant.get()
    }
    fn unix_secs(&self) -> u64 {
        self.0.secs.get()
    }
}

fn build() -> (MockLink, Rc<FakeClock>, Session<MockLink, SharedClock>) {
    let link = MockLink::default();
    let inner = Rc::new(FakeClock::new());
    let session = Session::new(
        link.clone(),
        SharedClock(inner.clone()),
        SessionConfig::default(),
    );
    (link, inner, session)
}

// --- tests ------------------------------------------------------------------

/// The connect seed burst is sent verbatim (FR-CAT-07).
#[test]
fn fr_cat_07_seed_sends_get_burst() {
    let (link, _clock, mut s) = build();
    s.seed().unwrap();
    let expected: Vec<String> = connect_state_seed().iter().map(|s| s.to_string()).collect();
    assert_eq!(link.sent(), expected);
    assert_eq!(link.sent().first().map(String::as_str), Some("IF;"));
}

/// A hard I/O error on the link while transmitting reaches the safe state
/// immediately — without advancing the clock past the keep-alive timeout —
/// via `note_io_error` (what the worker calls when `pump()` errors).
///
/// trace: NFR-REL-FAILSAFE
#[test]
fn nfr_rel_failsafe_io_error_unkeys_immediately() {
    let (link, _clock, mut s) = build();
    s.arm_tx();
    assert!(s.begin_tx().unwrap());
    assert!(s.is_transmitting());

    link.fail(); // next poll_frames errors (dead socket)
    assert!(s.pump().is_err(), "pump must surface the I/O error");
    s.note_io_error(); // worker's response — no clock advance

    assert!(!s.is_connected(), "I/O error marks the link lost");
    assert!(!s.is_transmitting(), "fail-safe must unkey immediately");
    assert!(!s.is_tx_armed(), "fail-safe must disarm");
    assert_eq!(
        link.last_sent().as_deref(),
        Some("RX;"),
        "must send RX; to unkey"
    );
}

/// Keep-alive fires once per interval as `PING<secs>;` (FR-SES-01, FR-SES-PING).
#[test]
fn fr_ses_ping_keepalive_emits_timestamped_ping() {
    let (link, clock, mut s) = build();

    assert_eq!(s.tick().unwrap(), SessionEvent::Idle); // interval not yet elapsed

    clock.advance(Duration::from_millis(1000));
    assert_eq!(s.tick().unwrap(), SessionEvent::PingSent);

    let ping = link.last_sent().unwrap();
    assert!(ping.starts_with("PING") && ping.ends_with(';'), "{ping}");
    // carries the unix-seconds label
    assert_eq!(ping, "PING1001;");
}

/// No inbound for longer than the timeout → LinkLost, and a transmitting session
/// is forced back to RX and disarmed (FR-SES-02, FR-TX-SAFE-01).
#[test]
fn fr_ses_02_link_loss_triggers_failsafe_unkey() {
    let (link, clock, mut s) = build();
    s.arm_tx();
    assert!(s.begin_tx().unwrap());
    assert!(s.is_transmitting());

    clock.advance(Duration::from_secs(6)); // > 5 s link_timeout
    assert_eq!(s.tick().unwrap(), SessionEvent::LinkLost);

    assert!(!s.is_connected());
    assert!(!s.is_transmitting(), "fail-safe must unkey");
    assert!(!s.is_tx_armed(), "fail-safe must disarm");
    assert_eq!(link.last_sent().as_deref(), Some("RX;"));
}

/// Inbound traffic refreshes liveness so the link is NOT declared lost (FR-SES-02).
#[test]
fn fr_ses_02_inbound_keeps_link_alive() {
    let (link, clock, mut s) = build();
    link.queue(&["FA00014074000;"]);

    clock.advance(Duration::from_secs(4));
    s.pump().unwrap(); // refreshes last_rx at t=4s
    assert_eq!(s.state().vfo_a_hz, Some(14_074_000));

    clock.advance(Duration::from_secs(4)); // t=8s, only 4s since last rx
    assert_ne!(s.tick().unwrap(), SessionEvent::LinkLost);
    assert!(s.is_connected());
}

/// Transmit is impossible while disarmed (FR-TX-SAFE-03).
#[test]
fn fr_tx_safe_03_disarmed_transmit_is_inert() {
    let (link, _clock, mut s) = build();
    assert!(!s.begin_tx().unwrap(), "must refuse while disarmed");
    assert!(!s.is_transmitting());
    assert!(link.sent().is_empty(), "nothing sent while disarmed");
}

/// Emergency stop sends `RX;` and disarms regardless of state (FR-TX-SAFE-04).
#[test]
fn fr_tx_safe_04_emergency_stop_unkeys_and_disarms() {
    let (link, _clock, mut s) = build();
    s.arm_tx();
    s.begin_tx().unwrap();

    s.emergency_stop().unwrap();
    assert!(!s.is_transmitting());
    assert!(!s.is_tx_armed());
    assert_eq!(link.last_sent().as_deref(), Some("RX;"));
}

/// CW keying is gated by the TX arm: nothing is sent while disarmed; once armed,
/// the `KZ` stream is emitted (FR-TX-CW-01, FR-TX-SAFE-03).
#[test]
fn fr_tx_cw_01_keying_is_arm_gated() {
    let (link, _clock, mut s) = build();

    assert!(!s.send_cw(&[Dit, Dah]).unwrap(), "refused while disarmed");
    assert!(link.sent().is_empty());

    s.arm_tx();
    assert!(s.send_cw(&[Dit, Dah]).unwrap(), "sent while armed");
    assert_eq!(link.last_sent().as_deref(), Some("KZ.-;"));

    // CW is self-timed; it must not latch the sustained TX flag.
    assert!(!s.is_transmitting());
}

/// Pump applies sub-RX (`$`) responses to the correct field (FR-CAT-06).
#[test]
fn fr_cat_06_pump_applies_responses() {
    let (link, _clock, mut s) = build();
    link.queue(&["MD3;", "MD$2;"]);
    s.pump().unwrap();
    assert_eq!(s.state().mode_a, Some(Mode::Cw));
    assert_eq!(s.state().mode_b, Some(Mode::Usb));
}

/// TX audio is only sent while actively transmitting (FR-AUD-TX-01).
#[test]
fn fr_aud_tx_01_tx_audio_requires_active_transmit() {
    let (link, _clock, mut s) = build();
    let payload = vec![0x01u8, 0x01, 0, 3, 0xF0, 0, 0, 9, 9]; // an audio payload

    assert!(
        !s.send_tx_audio(&payload).unwrap(),
        "refused while not transmitting"
    );
    assert!(link.sent().is_empty());

    s.arm_tx();
    s.begin_tx().unwrap(); // now transmitting (sends "TX;")
    assert!(s.send_tx_audio(&payload).unwrap());
    assert!(link.sent().iter().any(|c| c.starts_with("<frame:")));
}

/// Pump demultiplexes a mixed batch: CAT → state, audio/spectrum → `Inbound`.
///
/// trace: FR-STREAM-02
#[test]
fn fr_stream_02_pump_demuxes_cat_audio_and_spectrum() {
    use k4_stream::audio::{AudioPacket, EncodeMode};

    let (link, _clock, mut s) = build();
    // one CAT response + one audio frame + one PAN frame, in a single batch
    let mut pan = vec![0u8; 27];
    pan[0] = 0x02;
    link.queue_payloads(vec![
        encode_cat_payload("FA00014074000;"),
        AudioPacket::encode(7, EncodeMode::OpusFloat, 240, &[1, 2, 3]),
        pan,
    ]);

    let inbound = s.pump().unwrap();

    assert_eq!(s.state().vfo_a_hz, Some(14_074_000)); // CAT → state
    assert_eq!(inbound.cat, vec!["FA00014074000;"]); // CAT surfaced for diagnostics
    assert_eq!(inbound.audio.len(), 1); // audio routed out
    assert_eq!(inbound.spectrum.len(), 1); // spectrum routed out
}
