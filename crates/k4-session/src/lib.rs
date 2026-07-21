//! Session manager (ARC-07).
//!
//! Sits above a [`CatLink`] and owns the live [`RadioState`]. Responsibilities:
//! seed the state on connect (FR-CAT-07), send a 1 Hz keep-alive `PING<secs>;`
//! and detect link loss (FR-SES-01/02/PING), and enforce the transmit fail-safe
//! — on link loss, immediately drop keying and require re-arming (FR-TX-SAFE-01),
//! plus an explicit arm gate (FR-TX-SAFE-03) and emergency stop (FR-TX-SAFE-04).
//!
//! Time is injected via [`Clock`] so the keep-alive/timeout logic is fully
//! deterministic under test (no real sleeping).

use std::io;
use std::time::{Duration, Instant};

use k4_protocol::cat::{self, decode_cat_text, TuneAction};
use k4_protocol::cw::{encode_kz, encode_kzf, encode_kzl, KeyElement};
use k4_protocol::frame::PayloadType;
use k4_protocol::state::{connect_state_seed, RadioState};
use k4_transport::CatLink;

/// Bounded exponential backoff for reconnect scheduling (FR-SES-RECONNECT).
///
/// Each [`next_delay`](Backoff::next_delay) doubles from `base`, capped at `max`.
/// Pure and clock-free so the policy is unit-testable; the worker applies the
/// returned delays against real time.
#[derive(Debug, Clone)]
pub struct Backoff {
    base: Duration,
    max: Duration,
    attempt: u32,
}

impl Backoff {
    /// New backoff between `base` and `max`.
    pub fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            attempt: 0,
        }
    }

    /// The delay before the next attempt, doubling each call (capped at `max`).
    pub fn next_delay(&mut self) -> Duration {
        let shift = self.attempt.min(16);
        let delay = self.base.saturating_mul(1u32 << shift).min(self.max);
        self.attempt = self.attempt.saturating_add(1);
        delay
    }

    /// Reset the attempt counter (e.g. after a successful connect).
    pub fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Number of attempts scheduled so far.
    pub fn attempts(&self) -> u32 {
        self.attempt
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new(Duration::from_secs(1), Duration::from_secs(30))
    }
}

/// Non-CAT payloads demultiplexed out of [`Session::pump`] for the caller to
/// decode (audio via `k4-stream`/`k4-audio`, spectrum via `k4-stream`). CAT
/// payloads are applied to [`RadioState`] internally and not returned here.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Inbound {
    /// CAT response texts seen this pump (also applied to state); for diagnostics.
    pub cat: Vec<String>,
    /// Raw audio payloads (`0x01`).
    pub audio: Vec<Vec<u8>>,
    /// Raw panadapter/spectrum payloads (`0x02` / `0x03`).
    pub spectrum: Vec<Vec<u8>>,
}

/// Injected time source: a monotonic instant (for intervals) and unix seconds
/// (for the `PING<secs>;` label). Mocked in tests, system-backed in production.
pub trait Clock {
    /// Monotonic now.
    fn now(&self) -> Instant;
    /// Wall-clock seconds since the unix epoch.
    fn unix_secs(&self) -> u64;
}

/// Production clock backed by the OS.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn unix_secs(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Tuning for keep-alive and link-loss timing.
#[derive(Debug, Clone, Copy)]
pub struct SessionConfig {
    /// Keep-alive period (default 1 s; CON-05).
    pub ping_interval: Duration,
    /// Declare the link lost after this long with no inbound data (default 5 s,
    /// comfortably before the server's 10 s drop; FR-SES-02).
    pub link_timeout: Duration,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ping_interval: Duration::from_secs(1),
            link_timeout: Duration::from_secs(5),
        }
    }
}

/// Outcome of a [`Session::tick`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// Nothing was due this tick.
    Idle,
    /// A keep-alive `PING` was sent.
    PingSent,
    /// The link was declared lost (fail-safe applied).
    LinkLost,
}

/// Session over a CAT link with an injected clock.
#[derive(Debug)]
pub struct Session<L: CatLink, C: Clock> {
    link: L,
    clock: C,
    cfg: SessionConfig,
    state: RadioState,
    connected: bool,
    tx_armed: bool,
    transmitting: bool,
    /// A tune carrier is believed to be on air. Kept separate from
    /// `transmitting`, which gates the mic path — see [`Session::tune`].
    tuning: bool,
    last_rx: Instant,
    last_ping: Instant,
}

impl<L: CatLink, C: Clock> Session<L, C> {
    /// Create a connected session. `last_rx`/`last_ping` start "now".
    pub fn new(link: L, clock: C, cfg: SessionConfig) -> Self {
        let t0 = clock.now();
        Self {
            link,
            clock,
            cfg,
            state: RadioState::new(),
            connected: true,
            tx_armed: false,
            transmitting: false,
            tuning: false,
            last_rx: t0,
            last_ping: t0,
        }
    }

    /// Issue the connect-time GET burst to seed [`RadioState`] (FR-CAT-07).
    pub fn seed(&mut self) -> io::Result<()> {
        for cmd in connect_state_seed() {
            self.link.send_cat(cmd)?;
        }
        Ok(())
    }

    /// Read available frames and demultiplex by type (FR-STREAM-02): CAT
    /// responses fold into [`RadioState`] (FR-CAT-06/AI), `PONG` refreshes
    /// liveness, and audio/spectrum payloads are returned in [`Inbound`] for the
    /// caller to decode. Any inbound traffic refreshes the link-loss timer.
    pub fn pump(&mut self) -> io::Result<Inbound> {
        let frames = self.link.poll_frames()?;
        if !frames.is_empty() {
            self.last_rx = self.clock.now();
        }
        let mut inbound = Inbound::default();
        for payload in frames {
            match payload.first().copied().map(PayloadType::from_byte) {
                Some(PayloadType::Cat) => {
                    if let Some(text) = decode_cat_text(&payload) {
                        if !text.starts_with("PONG") {
                            self.state.apply_cat(&text);
                        }
                        inbound.cat.push(text);
                    }
                }
                Some(PayloadType::Audio) => inbound.audio.push(payload),
                Some(PayloadType::Pan | PayloadType::MiniPan) => inbound.spectrum.push(payload),
                Some(PayloadType::Unknown(_)) | None => {} // tolerate (FR-CAT-04)
            }
        }
        Ok(inbound)
    }

    /// Advance time-based behaviour: link-loss check first, then keep-alive.
    ///
    /// trace: FR-SES-01, FR-SES-02, FR-SES-PING, FR-TX-SAFE-01
    pub fn tick(&mut self) -> io::Result<SessionEvent> {
        if !self.connected {
            return Ok(SessionEvent::Idle);
        }
        let now = self.clock.now();

        if now.saturating_duration_since(self.last_rx) >= self.cfg.link_timeout {
            self.connected = false;
            self.fail_safe();
            return Ok(SessionEvent::LinkLost);
        }

        if now.saturating_duration_since(self.last_ping) >= self.cfg.ping_interval {
            self.last_ping = now;
            self.link
                .send_cat(&format!("PING{};", self.clock.unix_secs()))?;
            return Ok(SessionEvent::PingSent);
        }

        Ok(SessionEvent::Idle)
    }

    /// Treat an I/O error on the link as immediate link loss: mark the session
    /// disconnected and apply the TX fail-safe *now* (unkey + disarm), instead
    /// of waiting for the keep-alive timeout in [`Session::tick`]. The worker
    /// calls this when [`Session::pump`] returns an error, so a hard socket
    /// error mid-transmit reaches the safe state within one service loop rather
    /// than after the link-timeout window.
    ///
    /// trace: NFR-REL-FAILSAFE, FR-TX-SAFE-01
    pub fn note_io_error(&mut self) {
        self.connected = false;
        self.fail_safe();
    }

    /// Arm transmit. Transmit is impossible while disarmed (FR-TX-SAFE-03).
    pub fn arm_tx(&mut self) {
        self.tx_armed = true;
    }

    /// Disarm transmit.
    pub fn disarm_tx(&mut self) {
        self.tx_armed = false;
    }

    /// Begin transmit. Returns `false` (and sends nothing) unless armed and
    /// connected (FR-TX-01, FR-TX-SAFE-03).
    pub fn begin_tx(&mut self) -> io::Result<bool> {
        if !self.tx_armed || !self.connected {
            return Ok(false);
        }
        self.transmitting = true;
        self.link.send_cat("TX;")?;
        Ok(true)
    }

    /// Send a CW keying stream (`KZ`). Gated by the TX arm exactly like voice
    /// transmit: returns `false` and sends nothing unless armed and connected
    /// (FR-TX-CW-01, FR-TX-SAFE-03). CW is self-timed by the radio (and bounded
    /// by the `KZF` fail-safe), so it does not set the sustained TX flag.
    pub fn send_cw(&mut self, elements: &[KeyElement]) -> io::Result<bool> {
        if !self.tx_armed || !self.connected {
            return Ok(false);
        }
        self.link.send_cat(&encode_kz(elements))?;
        Ok(true)
    }

    /// Start or stop a tune (`TU`; FR-TX-TUNE-01).
    ///
    /// Every action but [`TuneAction::Exit`] keys the transmitter, so those are
    /// gated by the TX arm exactly like voice and CW: returns `false` and sends
    /// nothing unless armed and connected (FR-TX-SAFE-03). `Exit` is always
    /// allowed — stopping transmit must never be gated.
    ///
    /// Deliberately does **not** set the sustained `transmitting` flag. The K4
    /// generates the tune carrier itself; setting that flag would open
    /// [`Session::send_tx_audio`] and stream mic audio over the top of it.
    /// Emergency stop and the link-loss fail-safe still end a tune, because
    /// the K4 returns `TU0` whenever it drops transmit (D12 `TU`), and both
    /// paths also send `TU0` explicitly.
    pub fn tune(&mut self, action: TuneAction) -> io::Result<bool> {
        if action.transmits() {
            if !self.tx_armed || !self.connected {
                return Ok(false);
            }
            self.tuning = true;
        } else {
            self.tuning = false;
        }
        self.link.send_cat(&cat::tune(action))?;
        Ok(true)
    }

    /// Whether a tune is believed to be running. Distinct from
    /// [`Session::is_transmitting`], which gates the mic path.
    pub fn is_tuning(&self) -> bool {
        self.tuning
    }

    /// Configure the radio-side CW key-down initial delay (`KZL`; FR-TX-CW-02).
    pub fn set_cw_delay(&mut self, ms: u16) -> io::Result<()> {
        self.link.send_cat(&encode_kzl(ms))
    }

    /// Configure the radio-side CW fail-safe timeout (`KZF`; FR-TX-SAFE-02) so a
    /// stalled keying stream cannot hold the key down indefinitely.
    pub fn set_cw_failsafe(&mut self, minutes: u8) -> io::Result<()> {
        self.link.send_cat(&encode_kzf(minutes))
    }

    /// Send one transmit-audio frame payload (wrapped in a frame). Only while
    /// actively transmitting and connected; returns `false` otherwise
    /// (FR-AUD-TX-01, FR-TX-SAFE-03 — mic audio never leaks off-air).
    pub fn send_tx_audio(&mut self, payload: &[u8]) -> io::Result<bool> {
        if !self.transmitting || !self.connected {
            return Ok(false);
        }
        self.link.send_frame(payload)?;
        Ok(true)
    }

    /// End transmit (sends `RX;` if currently transmitting).
    pub fn end_tx(&mut self) -> io::Result<()> {
        if self.transmitting {
            self.transmitting = false;
            self.link.send_cat("RX;")?;
        }
        Ok(())
    }

    /// Emergency stop: unconditionally leave transmit and disarm (FR-TX-SAFE-04).
    pub fn emergency_stop(&mut self) -> io::Result<()> {
        self.transmitting = false;
        self.tx_armed = false;
        // A tune carrier is on air without `transmitting` being set, so stop it
        // explicitly rather than relying on the radio's automatic `TU0` on
        // dropping transmit (D12 `TU`). Best-effort, then the unconditional
        // `RX;` — the stop must reach the radio even if the first send fails.
        if self.tuning {
            self.tuning = false;
            let _ = self.link.send_cat(&cat::tune(TuneAction::Exit));
        }
        self.link.send_cat("RX;")
    }

    /// Fail-safe applied on link loss: cease keying locally and disarm so TX
    /// cannot resume without an explicit re-arm (FR-TX-SAFE-01). The `RX;` send
    /// is best-effort — the link may already be down.
    fn fail_safe(&mut self) {
        if self.transmitting {
            self.transmitting = false;
            let _ = self.link.send_cat("RX;");
        }
        if self.tuning {
            self.tuning = false;
            let _ = self.link.send_cat(&cat::tune(TuneAction::Exit));
            let _ = self.link.send_cat("RX;");
        }
        self.tx_armed = false;
    }

    /// Send a non-transmit control command (e.g. `"FA00014074000;"`, `"MD3;"`).
    /// TX commands must go through [`Session::begin_tx`]/[`Session::end_tx`].
    pub fn send(&mut self, command: &str) -> io::Result<()> {
        // The arm interlock lives here, at the seam every raw command passes
        // through — not only in `begin_tx`/`send_cw`/`tune`. Those three were
        // gated while this passthrough was not, so the switch-emulation
        // `TUNE`/`TUNE LP`/`ATU TUNE`/`XMIT`, DVR playback, and anything typed
        // into the diagnostics console could key the transmitter with the arm
        // off (FR-TX-SAFE-03, found on a live radio).
        if cat::keys_transmitter(command) && (!self.tx_armed || !self.connected) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "transmit is disarmed",
            ));
        }
        self.link.send_cat(command)
    }

    /// Disconnect cleanly by sending `RRN;` (FR-CONN-02).
    pub fn disconnect(&mut self) -> io::Result<()> {
        self.connected = false;
        self.link.send_cat("RRN;")
    }

    /// The current radio state (UI projection source).
    pub fn state(&self) -> &RadioState {
        &self.state
    }

    /// Fold a locally-generated CAT string into the state, as if the radio had
    /// echoed it — for optimistic display of commands we originate (e.g. K-Pod
    /// tuning) when the radio does not auto-echo `SET`s. Reconciled by the next
    /// real read-back.
    pub fn apply_local(&mut self, cat: &str) -> bool {
        self.state.apply_cat(cat)
    }

    /// Whether the link is considered up.
    pub fn is_connected(&self) -> bool {
        self.connected
    }
    /// Whether transmit is currently active.
    pub fn is_transmitting(&self) -> bool {
        self.transmitting
    }
    /// Whether transmit is armed.
    pub fn is_tx_armed(&self) -> bool {
        self.tx_armed
    }
}
