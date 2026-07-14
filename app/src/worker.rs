//! Background I/O worker (ADR-06 bridge): owns the blocking transport + session
//! on its own thread and communicates with the iced UI via a command channel and
//! a shared snapshot. The UI never blocks on I/O (FR-UI-07).
//!
//! It also demultiplexes inbound frames: CAT updates `RadioState` inside the
//! session; audio payloads are decoded ([`AudioPacket`]) and fed to a
//! [`JitterBuffer`]; PAN payloads are decoded ([`PanFrame`]) for the spectrum.
//! Opus decode + audio device playback are the remaining L4 step — here the
//! reassembled RX frames are counted so the pipeline is observable.

use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::ui::ConnPhase;

use k4_audio::{AudioInput, AudioOutput, JitterBuffer, OpusDecoder, OpusEncoder};
use k4_diag::{DiagLog, Level};
use k4_protocol::state::{Mode, RadioState};
use k4_session::{Backoff, Session, SessionConfig, SessionEvent, SystemClock};
use k4_stream::{AudioPacket, EncodeMode, PanFrame};
use k4_transport::{CatLink, ConnectConfig, SerialPortTransport, TcpRemoteTransport};

/// 20 ms TX frame at 12 kHz.
const TX_FRAME_SAMPLES: usize = 240;
/// Display width (bins) for the spectrum trace + waterfall.
const SPECTRUM_WIDTH: usize = 192;
/// Waterfall history depth (rows).
const WATERFALL_ROWS: usize = 64;

/// Bucket-peak downsample of a bin array to `target` columns.
fn downsample(bins: &[f32], target: usize) -> Vec<f32> {
    if bins.is_empty() || target == 0 {
        return Vec::new();
    }
    if bins.len() <= target {
        return bins.to_vec();
    }
    let chunk = bins.len() as f32 / target as f32;
    (0..target)
        .map(|i| {
            let start = (i as f32 * chunk) as usize;
            let end = (((i + 1) as f32 * chunk) as usize).clamp(start + 1, bins.len());
            bins[start..end]
                .iter()
                .copied()
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect()
}

/// Where/how to connect (transport selection).
#[derive(Debug, Clone)]
pub enum ConnectTarget {
    /// Ethernet to a K4/0 server (plaintext 9205 or TLS-PSK 9204).
    Tcp {
        host: String,
        port: u16,
        password: String,
        use_tls: bool,
    },
    /// USB/serial CAT (raw, control-only).
    Serial { path: String, baud: u32 },
}

/// Commands the UI sends to the worker.
#[derive(Debug, Clone)]
pub enum WorkerCmd {
    Connect(ConnectTarget),
    Disconnect,
    /// Set VFO A frequency in Hz.
    SetFreqA(u64),
    /// Set VFO B frequency in Hz.
    SetFreqB(u64),
    /// Set main-RX mode (raw `MD` digit, e.g. 3 = CW).
    SetMode(u8),
    /// Toggle split (`FT/`).
    ToggleSplit,
    /// Toggle RIT (`RT/`).
    ToggleRit,
    /// Toggle XIT (`XT/`).
    ToggleXit,
    /// Clear RIT/XIT offset (`RC`).
    ClearRitXit,
    /// Arm (`true`) / disarm (`false`) transmit.
    ArmTx(bool),
    /// Key (`true`) / unkey (`false`) transmit.
    Key(bool),
    /// Emergency stop.
    EmergencyStop,
    /// Send an arbitrary raw CAT command (diagnostics console, FR-DIAG-02).
    SendRawCat(String),
    /// Set the 8-band RX graphic equalizer (`RE`, FR-EQ-01).
    SetRxEq([i8; 8]),
    /// Set the 8-band TX graphic equalizer (`TE`, FR-EQ-01).
    SetTxEq([i8; 8]),
    /// Flatten the RX graphic equalizer (`REF`, FR-EQ-01).
    RxEqFlat,
    /// Send a pre-encoded CAT command built by the UI via `k4_protocol::cat`
    /// (display/band screens, FR-PAN-CTL-01/FR-VFO-04). Distinct from
    /// `SendRawCat` (operator console) — logged at debug, not info.
    Cat(String),
    /// Like [`WorkerCmd::Cat`] but also folds the command into the local radio
    /// state (`Session::apply_local`) so a `SET` the K4 doesn't echo (e.g. the
    /// tuning step `VT`) is reflected in the UI at once.
    CatLocal(String),
    /// Select the RX playback device by name (`None` = default) — FR-AUD-DEV-01.
    SetOutputDevice(Option<String>),
    /// Select the TX capture device by name (`None` = default) — FR-AUD-DEV-01.
    SetInputDevice(Option<String>),
    /// RX playback volume gain (FR-AUD-LVL-01), 0.0–2.0.
    SetVolume(f32),
    /// TX mic capture gain (FR-AUD-LVL-01), 0.0–3.0.
    SetMicGain(f32),
    /// Enable/disable the K-Pod control surface at runtime (FR-KPOD-04).
    SetKpodEnabled(bool),
    /// Replace the K-Pod function-switch macro table (16 slots, CAT strings;
    /// FR-KPOD-06).
    SetKpodButtons(Vec<String>),
}

/// Snapshot the UI renders from (FR-UI-02/03/06). Written by the worker.
#[derive(Debug, Clone, Default)]
pub struct UiSnapshot {
    pub connected: bool,
    /// Connection lifecycle phase, driving the connect/cancel control (FR-UI-16).
    pub phase: ConnPhase,
    pub transmitting: bool,
    pub tx_armed: bool,
    pub vfo_a_hz: Option<u64>,
    pub vfo_b_hz: Option<u64>,
    pub mode_a: Option<&'static str>,
    /// Sub-RX / VFO B mode (for the dual view).
    pub mode_b: Option<&'static str>,
    pub split: Option<bool>,
    /// Main-RX S-meter bar count (`SM`).
    pub s_meter_bars: Option<u8>,
    /// Main-RX high-resolution S-meter, dBm (`SMH`).
    pub s_meter_dbm: Option<i32>,
    /// Sub-RX high-resolution S-meter, dBm (`SMH$`).
    pub s_meter_dbm_sub: Option<i32>,
    /// Receive bandwidth, Hz (`BW`).
    pub bandwidth_hz: Option<u32>,
    /// RX attenuator: dB and on/off (`RA`).
    pub atten_db: Option<u8>,
    pub atten_on: Option<bool>,
    /// AGC mode: 0 off / 1 slow / 2 fast.
    pub agc_mode: Option<u8>,
    pub nb_on: Option<bool>,
    pub nr_on: Option<bool>,
    pub preamp_on: Option<bool>,
    pub rit_on: Option<bool>,
    pub xit_on: Option<bool>,
    /// Count of RX audio frames reassembled (jitter-buffer output).
    pub audio_frames: u64,
    /// Bin count of the most recent spectrum frame.
    pub spectrum_bins: usize,
    /// Latest spectrum trace for the main RX / VFO A (downsampled dBm bins).
    pub spectrum_latest: Vec<f32>,
    /// Latest spectrum trace for the sub RX / VFO B.
    pub spectrum_sub: Vec<f32>,
    /// Waterfall history (main RX / VFO A), newest row first.
    pub waterfall: Vec<Vec<f32>>,
    /// Waterfall history for the sub RX / VFO B.
    pub waterfall_sub: Vec<Vec<f32>>,
    /// Latest mini-pan trace (0x03), empty if disabled.
    pub mini_pan: Vec<f32>,
    /// Recent diagnostic log lines (FR-DIAG-01/02).
    pub diag_lines: Vec<String>,
    /// Human-readable status / last error.
    pub status: String,
    /// Full radio state model, for the config screens to read back their current
    /// values on connect (FR-UI-19 read-back). `None` fields = not yet reported.
    pub radio: RadioState,
}

/// Sample snapshot for offline UI inspection (`--demo`). Lets the GUI show the
/// coloured frequency readouts, the strong-signal (yellow) S-meter, and the
/// two-line state chips with no radio or sim attached. Not used in normal runs.
pub fn demo_snapshot() -> UiSnapshot {
    UiSnapshot {
        connected: true,
        phase: ConnPhase::Connected,
        transmitting: false,
        tx_armed: false,
        vfo_a_hz: Some(14_074_000),
        vfo_b_hz: Some(14_061_100),
        mode_a: Some("USB"),
        mode_b: Some("CW"),
        split: Some(false),
        s_meter_bars: Some(7),
        s_meter_dbm: Some(-68), // ≥ −73 → "caution" yellow (FR-UI-10)
        s_meter_dbm_sub: Some(-110),
        bandwidth_hz: Some(2800),
        atten_db: Some(6),
        atten_on: Some(true),
        agc_mode: Some(1),
        nb_on: Some(false),
        nr_on: Some(true),
        preamp_on: Some(true),
        rit_on: Some(false),
        xit_on: Some(false),
        status: "DEMO MODE — sample state (no radio). Switch A / B / A+B to reflow.".into(),
        ..Default::default()
    }
}

/// Short label for a [`Mode`].
pub fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Lsb => "LSB",
        Mode::Usb => "USB",
        Mode::Cw => "CW",
        Mode::Fm => "FM",
        Mode::Am => "AM",
        Mode::Data => "DATA",
        Mode::CwRev => "CW-R",
        Mode::DataRev => "DATA-R",
    }
}

/// Either transport, dispatched as a [`CatLink`] so the session is transport-
/// agnostic (FR-CONN-ABSTRACT).
enum AnyLink {
    Tcp(TcpRemoteTransport),
    Serial(SerialPortTransport),
}

impl CatLink for AnyLink {
    fn send_frame(&mut self, payload: &[u8]) -> std::io::Result<()> {
        match self {
            AnyLink::Tcp(t) => CatLink::send_frame(t, payload),
            AnyLink::Serial(s) => CatLink::send_frame(s, payload),
        }
    }
    fn poll_frames(&mut self) -> std::io::Result<Vec<Vec<u8>>> {
        match self {
            AnyLink::Tcp(t) => CatLink::poll_frames(t),
            AnyLink::Serial(s) => CatLink::poll_frames(s),
        }
    }
}

type Link = Session<AnyLink, SystemClock>;

/// Open the TCP transport, choosing TLS-PSK when requested and compiled in.
fn open_tcp(
    host: &str,
    port: u16,
    cfg: &ConnectConfig,
    use_tls: bool,
) -> std::io::Result<TcpRemoteTransport> {
    #[cfg(feature = "tls")]
    if use_tls {
        return TcpRemoteTransport::connect_tls((host, port), cfg);
    }
    #[cfg(not(feature = "tls"))]
    let _ = use_tls;
    TcpRemoteTransport::connect((host, port), cfg)
}

/// Worker-owned state across connection lifetimes.
struct WorkerState {
    session: Option<Link>,
    rx_audio: JitterBuffer,
    rx_decoder: Option<OpusDecoder>,
    audio_out: Option<AudioOutput>,
    audio_in: Option<AudioInput>,
    tx_encoder: Option<OpusEncoder>,
    tx_seq: u8,
    audio_frames: u64,
    spectrum_bins: usize,
    // Per-receiver spectrum/waterfall, indexed by `PanFrame.receiver` (0=main/A,
    // 1=sub/B), so a dual-pan view shows each RX's own trace (FR-PAN-02).
    spectrum_latest: [Vec<f32>; 2],
    waterfall: [VecDeque<Vec<f32>>; 2],
    // Latest mini-pan (0x03) trace, if enabled (FR-UI-14).
    mini_pan: Vec<f32>,
    diag: DiagLog,
    // Reconnect (FR-SES-RECONNECT): retained target + backoff schedule.
    connect_params: Option<ConnectTarget>,
    backoff: Backoff,
    next_attempt: Option<Instant>,
    // In-flight connect attempt running on a short-lived thread, so the blocking
    // TCP/TLS handshake cannot freeze the worker and the attempt is cancellable
    // (FR-UI-16). `None` when no attempt is being awaited.
    pending_connect: Option<Receiver<ConnectOutcome>>,
    // Audio device selection + local levels (FR-AUD-DEV-01/LVL-01), applied when
    // the device streams are (re-)created per connection.
    out_device: Option<String>,
    in_device: Option<String>,
    volume: f32,
    mic_gain: f32,
    // Elecraft K-Pod USB control surface (FR-KPOD-*), when the feature is built.
    #[cfg(feature = "kpod")]
    kpod: kpod::KpodState,
}

/// Result handed back from the connect thread: the opened link + its session
/// profile, or a redacted error message.
type ConnectOutcome = Result<(AnyLink, SessionConfig), String>;

impl WorkerState {
    fn new() -> Self {
        Self {
            #[cfg(feature = "kpod")]
            kpod: kpod::KpodState::new(),
            session: None,
            rx_audio: JitterBuffer::new(8),
            rx_decoder: None,
            audio_out: None,
            audio_in: None,
            tx_encoder: None,
            tx_seq: 0,
            audio_frames: 0,
            spectrum_bins: 0,
            spectrum_latest: [Vec::new(), Vec::new()],
            waterfall: [VecDeque::new(), VecDeque::new()],
            mini_pan: Vec::new(),
            // Debug level so the raw CAT console shows traffic; bounded ring.
            // Sized to hold a few minutes of busy traffic so lines don't scroll
            // out from under the reader before they can be copied.
            diag: DiagLog::new(4000, Level::Debug),
            connect_params: None,
            backoff: Backoff::default(),
            next_attempt: None,
            pending_connect: None,
            out_device: None,
            in_device: None,
            volume: 1.0,
            mic_gain: 1.0,
        }
    }
    /// Reset per-connection state. The Opus codecs and device streams are
    /// re-created per connection (decoders are stateful; devices may change).
    fn reset(&mut self) {
        self.rx_audio = JitterBuffer::new(8);
        self.rx_decoder = OpusDecoder::rx().ok();
        self.audio_out = AudioOutput::with_device(self.out_device.as_deref()).ok();
        if let Some(out) = self.audio_out.as_mut() {
            out.set_volume(self.volume);
        }
        self.audio_in = AudioInput::with_device(self.in_device.as_deref()).ok();
        if let Some(inp) = self.audio_in.as_mut() {
            inp.set_mic_gain(self.mic_gain);
        }
        self.tx_encoder = OpusEncoder::mono().ok();
        self.tx_seq = 0;
        self.audio_frames = 0;
        self.spectrum_bins = 0;
        for rx in 0..2 {
            self.spectrum_latest[rx].clear();
            self.waterfall[rx].clear();
        }
        self.mini_pan.clear();
    }
}

/// Spawn the worker thread and return its handle.
pub fn spawn(rx: Receiver<WorkerCmd>, snapshot: Arc<Mutex<UiSnapshot>>) -> thread::JoinHandle<()> {
    thread::spawn(move || run(rx, snapshot))
}

fn set_status(snapshot: &Arc<Mutex<UiSnapshot>>, status: impl Into<String>) {
    if let Ok(mut s) = snapshot.lock() {
        s.status = status.into();
    }
}

/// Publish the connection phase, keeping the `connected` bool consistent
/// (FR-UI-16).
fn set_phase(snapshot: &Arc<Mutex<UiSnapshot>>, phase: ConnPhase) {
    if let Ok(mut s) = snapshot.lock() {
        s.phase = phase;
        s.connected = phase == ConnPhase::Connected;
    }
}

fn publish(snapshot: &Arc<Mutex<UiSnapshot>>, ws: &WorkerState) {
    let Some(session) = ws.session.as_ref() else {
        return;
    };
    let st: &RadioState = session.state();
    if let Ok(mut s) = snapshot.lock() {
        s.connected = session.is_connected();
        // A live session means we are connected; the disconnect/reconnect paths
        // move us out of this phase (FR-UI-16).
        if s.connected {
            s.phase = ConnPhase::Connected;
        }
        s.transmitting = session.is_transmitting();
        s.tx_armed = session.is_tx_armed();
        s.vfo_a_hz = st.vfo_a_hz;
        s.vfo_b_hz = st.vfo_b_hz;
        s.mode_a = st.mode_a.map(mode_label);
        s.mode_b = st.mode_b.map(mode_label);
        s.split = st.split;
        s.s_meter_bars = st.s_meter_bars;
        s.s_meter_dbm = st.s_meter_dbm;
        s.s_meter_dbm_sub = st.s_meter_dbm_sub;
        s.bandwidth_hz = st.bandwidth_hz;
        s.atten_db = st.atten_db;
        s.atten_on = st.atten_on;
        s.agc_mode = st.agc_mode;
        s.nb_on = st.nb_on;
        s.nr_on = st.nr_on;
        s.preamp_on = st.preamp_on;
        s.rit_on = st.rit_on;
        s.xit_on = st.xit_on;
        s.audio_frames = ws.audio_frames;
        s.spectrum_bins = ws.spectrum_bins;
        s.spectrum_latest = ws.spectrum_latest[0].clone();
        s.spectrum_sub = ws.spectrum_latest[1].clone();
        s.waterfall = ws.waterfall[0].iter().cloned().collect();
        s.waterfall_sub = ws.waterfall[1].iter().cloned().collect();
        s.mini_pan = ws.mini_pan.clone();
        s.diag_lines = ws.diag.recent(4000);
        s.radio = st.clone();
    }
}

/// Human-readable description of a connect target (no secrets).
fn describe(target: &ConnectTarget) -> String {
    match target {
        ConnectTarget::Tcp {
            host,
            port,
            use_tls,
            ..
        } => {
            let scheme = if *use_tls { "tls" } else { "tcp" };
            format!("{host}:{port} ({scheme})")
        }
        ConnectTarget::Serial { path, baud } => format!("{path}@{baud}"),
    }
}

/// Open the link and run the (blocking) handshake for `target`, returning the
/// ready link + its session profile or a redacted error. Pure w.r.t. worker
/// state so it can run on a short-lived connect thread (FR-UI-16): the blocking
/// TCP/TLS handshake never freezes the worker, and the attempt is cancellable.
fn open_link(target: ConnectTarget) -> ConnectOutcome {
    // Serial has no PING/PONG, so its keep-alive + link-loss are disabled.
    let timeout = Duration::from_millis(100);
    let desc = describe(&target);
    let (result, session_cfg, secret): (std::io::Result<AnyLink>, _, String) = match &target {
        ConnectTarget::Tcp {
            host,
            port,
            password,
            use_tls,
        } => {
            let cfg = ConnectConfig {
                password: password.clone(),
                read_timeout: timeout,
                ..Default::default()
            };
            (
                open_tcp(host, *port, &cfg, *use_tls).map(AnyLink::Tcp),
                SessionConfig::default(),
                password.clone(),
            )
        }
        ConnectTarget::Serial { path, baud } => (
            SerialPortTransport::open(path, *baud, timeout).map(AnyLink::Serial),
            SessionConfig {
                ping_interval: Duration::from_secs(3600),
                link_timeout: Duration::from_secs(86_400),
            },
            String::new(),
        ),
    };

    match result {
        Ok(link) => Ok((link, session_cfg)),
        // Distinguish the failure kind (FR-CONN-04) and redact any secret the
        // error might echo (NFR-SEC-01).
        Err(e) => {
            let reason = crate::ui::connect_error_reason(e.kind());
            Err(k4_config::redact(
                &format!("connect to {desc} failed: {reason}"),
                &secret,
            ))
        }
    }
}

/// Start a connection attempt on a background thread (FR-UI-16). Sets the phase
/// to `Connecting` immediately; the result is collected later by
/// [`poll_pending`]. A no-op if an attempt is already in flight.
fn begin_connect(ws: &mut WorkerState, snapshot: &Arc<Mutex<UiSnapshot>>) {
    if ws.pending_connect.is_some() {
        return;
    }
    let Some(target) = ws.connect_params.clone() else {
        return;
    };
    let desc = describe(&target);
    let (tx, rx) = mpsc::channel();
    // The connect thread owns the blocking handshake. If the attempt is
    // cancelled, the receiver is dropped and the eventual send is discarded
    // (dropping the freshly-opened link, which closes the socket).
    thread::spawn(move || {
        let _ = tx.send(open_link(target));
    });
    ws.pending_connect = Some(rx);
    ws.diag
        .log(Level::Info, "net", &format!("connecting to {desc}"));
    set_phase(snapshot, ConnPhase::Connecting);
    set_status(snapshot, format!("connecting to {desc}…"));
}

/// Collect the result of an in-flight connect attempt, if any has finished
/// (FR-UI-16). On success install the session; on failure schedule a backoff
/// retry while remaining in the `Connecting` phase (still cancellable).
fn poll_pending(ws: &mut WorkerState, snapshot: &Arc<Mutex<UiSnapshot>>) {
    let Some(rx) = ws.pending_connect.as_ref() else {
        return;
    };
    match rx.try_recv() {
        Ok(Ok((link, session_cfg))) => {
            ws.pending_connect = None;
            let mut s = Session::new(link, SystemClock, session_cfg);
            let _ = s.seed();
            ws.reset();
            ws.session = Some(s);
            ws.backoff.reset();
            ws.next_attempt = None;
            let msg = ws
                .connect_params
                .as_ref()
                .map(|t| format!("connected to {}", describe(t)))
                .unwrap_or_else(|| "connected".to_string());
            ws.diag.log(Level::Info, "net", &msg);
            set_phase(snapshot, ConnPhase::Connected);
            set_status(snapshot, msg);
        }
        Ok(Err(msg)) => {
            ws.pending_connect = None;
            if ws.connect_params.is_some() {
                // Retry with backoff; stay in Connecting so the button keeps
                // offering Cancel throughout the wait.
                let delay = ws.backoff.next_delay();
                ws.next_attempt = Some(Instant::now() + delay);
                let msg = format!(
                    "{msg} — retry in {:?} (attempt {})",
                    delay,
                    ws.backoff.attempts()
                );
                ws.diag.log(Level::Warn, "net", &msg);
                set_status(snapshot, msg);
            } else {
                // Cancelled meanwhile: go quiet.
                set_phase(snapshot, ConnPhase::Disconnected);
                set_status(snapshot, "disconnected");
            }
        }
        Err(TryRecvError::Empty) => {} // still connecting
        Err(TryRecvError::Disconnected) => {
            // Connect thread died without a result; drop and let retry logic run.
            ws.pending_connect = None;
        }
    }
}

fn run(rx: Receiver<WorkerCmd>, snapshot: Arc<Mutex<UiSnapshot>>) {
    let mut ws = WorkerState::new();

    loop {
        // 1. Drain pending UI commands.
        loop {
            match rx.try_recv() {
                Err(TryRecvError::Disconnected) => return, // UI gone → stop
                Ok(cmd) => handle_cmd(cmd, &mut ws, &snapshot),
                Err(TryRecvError::Empty) => break,
            }
        }

        // 2. Collect any finished connect attempt (FR-UI-16).
        poll_pending(&mut ws, &snapshot);

        // 3. Service the link, start a scheduled (re)connect, or idle.
        if ws.session.is_some() {
            service(&mut ws, &snapshot);
        } else {
            // Start the next attempt when due and none is already in flight.
            if ws.pending_connect.is_none() {
                if let Some(at) = ws.next_attempt {
                    if Instant::now() >= at {
                        ws.next_attempt = None;
                        begin_connect(&mut ws, &snapshot);
                    }
                }
            }
            // Publish diagnostics even while disconnected (connect errors, etc.).
            if let Ok(mut s) = snapshot.lock() {
                s.diag_lines = ws.diag.recent(4000);
            }
            thread::sleep(Duration::from_millis(50));
        }

        // Drain K-Pod events every loop — recognition is independent of the radio
        // link; tuning applies only when a session is connected.
        #[cfg(feature = "kpod")]
        ws.kpod.service(ws.session.as_mut(), &mut ws.diag);
    }
}

/// Pump the session once, demux inbound frames, and publish the snapshot.
fn service(ws: &mut WorkerState, snapshot: &Arc<Mutex<UiSnapshot>>) {
    let Some(session) = ws.session.as_mut() else {
        return;
    };

    let pumped = session.pump();
    if let Err(e) = &pumped {
        // A hard socket error must reach the safe state immediately — not after
        // the keep-alive timeout — so an error mid-TX unkeys now (NFR-REL-FAILSAFE).
        ws.diag
            .log(Level::Warn, "net", &format!("link I/O error: {e}"));
        session.note_io_error();
        set_status(snapshot, "link lost");
    }
    if let Ok(inbound) = pumped {
        // RX audio → jitter buffer (reorder) → Opus decode to PCM → speaker.
        for payload in &inbound.audio {
            if let Some(pkt) = AudioPacket::decode(payload) {
                ws.rx_audio.push(pkt.sequence, pkt.data.to_vec());
            }
        }
        // Half-duplex: while transmitting, the K4 streams its own TX monitor over
        // the RX audio channel. Playing it out the PC speakers, with an open mic,
        // forms a monitor→speaker→mic→TX feedback loop (FR-AUD-TX-01). Keep
        // decoding to hold the Opus state in sync, but suppress playback on TX.
        let txing = session.is_transmitting();
        while let Some(opus_frame) = ws.rx_audio.pop() {
            match ws.rx_decoder.as_mut() {
                Some(dec) => {
                    if let Ok(pcm) = dec.decode_float(&opus_frame) {
                        if !txing {
                            if let Some(out) = ws.audio_out.as_mut() {
                                out.submit_stereo_12k(&pcm);
                            }
                        }
                        ws.audio_frames += 1;
                    }
                }
                None => ws.audio_frames += 1,
            }
        }
        // Log received CAT for the diagnostics console (skip PONG noise).
        for text in &inbound.cat {
            if !text.starts_with("PONG") {
                ws.diag.log(Level::Debug, "rx", text);
            }
        }
        // Spectrum → downsampled trace + waterfall history (FR-PAN-02/03).
        for payload in &inbound.spectrum {
            if let Some(frame) = PanFrame::decode(payload) {
                let row = downsample(&frame.bins_dbm, SPECTRUM_WIDTH);
                if frame.mini {
                    ws.mini_pan = row; // mini-pan overview (FR-UI-14)
                    continue;
                }
                let rx = usize::from(frame.receiver.min(1)); // 0=main/A, 1=sub/B
                ws.spectrum_bins = frame.bins_dbm.len();
                ws.spectrum_latest[rx] = row.clone();
                ws.waterfall[rx].push_front(row);
                while ws.waterfall[rx].len() > WATERFALL_ROWS {
                    ws.waterfall[rx].pop_back();
                }
            }
        }
    }

    // TX audio: while keyed, pull mic frames → Opus encode → send (FR-AUD-TX-01).
    if session.is_transmitting() {
        if let (Some(input), Some(encoder)) = (ws.audio_in.as_ref(), ws.tx_encoder.as_mut()) {
            while let Some(frame) = input.take_frame(TX_FRAME_SAMPLES) {
                if let Ok(opus) = encoder.encode_float(&frame) {
                    let payload = AudioPacket::encode(
                        ws.tx_seq,
                        EncodeMode::OpusFloat,
                        TX_FRAME_SAMPLES as u16,
                        &opus,
                    );
                    ws.tx_seq = ws.tx_seq.wrapping_add(1);
                    let _ = session.send_tx_audio(&payload);
                }
            }
        }
    }

    if let Ok(SessionEvent::LinkLost) = session.tick() {
        ws.diag.log(Level::Warn, "net", "link lost");
        set_status(snapshot, "link lost");
    }

    let connected = session.is_connected();
    publish(snapshot, ws);
    if !connected {
        ws.session = None;
        // Auto-reconnect unless the user explicitly disconnected (params cleared).
        if ws.connect_params.is_some() {
            let delay = ws.backoff.next_delay();
            ws.next_attempt = Some(Instant::now() + delay);
            // Back to Connecting so the control offers Cancel during the wait.
            set_phase(snapshot, ConnPhase::Connecting);
            set_status(
                snapshot,
                format!(
                    "link lost — reconnecting in {:?} (attempt {})",
                    delay,
                    ws.backoff.attempts()
                ),
            );
        } else {
            set_phase(snapshot, ConnPhase::Disconnected);
        }
    }
}

fn handle_cmd(cmd: WorkerCmd, ws: &mut WorkerState, snapshot: &Arc<Mutex<UiSnapshot>>) {
    match cmd {
        WorkerCmd::Connect(target) => {
            ws.connect_params = Some(target);
            ws.backoff.reset();
            ws.next_attempt = None;
            begin_connect(ws, snapshot);
        }
        WorkerCmd::Disconnect => {
            // Also serves as "cancel" while an attempt is in flight (FR-UI-16):
            // drop the pending receiver so the connect thread's result (and its
            // freshly-opened link) is discarded.
            let was_connecting =
                ws.session.is_none() && (ws.pending_connect.is_some() || ws.next_attempt.is_some());
            if let Some(s) = ws.session.as_mut() {
                let _ = s.disconnect();
            }
            ws.session = None;
            ws.pending_connect = None;
            ws.connect_params = None; // stop auto-reconnect / retry
            ws.next_attempt = None;
            ws.backoff.reset();
            let msg = if was_connecting {
                "connection attempt cancelled"
            } else {
                "disconnected"
            };
            ws.diag.log(Level::Info, "net", msg);
            set_phase(snapshot, ConnPhase::Disconnected);
            set_status(snapshot, msg);
        }
        WorkerCmd::SetFreqA(hz) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&k4_protocol::cat::set_vfo_a_hz(hz));
            }
        }
        WorkerCmd::SetFreqB(hz) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&k4_protocol::cat::set_vfo_b_hz(hz));
            }
        }
        WorkerCmd::SetMode(digit) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&k4_protocol::cat::set_mode(digit));
            }
        }
        WorkerCmd::ToggleSplit => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send("FT/;"); // toggle form
            }
        }
        WorkerCmd::ToggleRit => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send("RT/;");
            }
        }
        WorkerCmd::ToggleXit => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send("XT/;");
            }
        }
        WorkerCmd::ClearRitXit => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(k4_protocol::cat::clear_rit_xit());
            }
        }
        WorkerCmd::ArmTx(arm) => {
            if let Some(s) = ws.session.as_mut() {
                if arm {
                    s.arm_tx();
                } else {
                    s.disarm_tx();
                }
            }
        }
        WorkerCmd::Key(key) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = if key {
                    s.begin_tx().map(|_| ())
                } else {
                    s.end_tx()
                };
            }
        }
        WorkerCmd::EmergencyStop => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.emergency_stop();
            }
        }
        WorkerCmd::SendRawCat(cmd) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&cmd);
                ws.diag.log(Level::Info, "tx", &cmd);
            }
        }
        WorkerCmd::SetRxEq(bands) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&k4_protocol::cat::set_rx_eq(bands));
            }
        }
        WorkerCmd::SetTxEq(bands) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&k4_protocol::cat::set_tx_eq(bands));
            }
        }
        WorkerCmd::RxEqFlat => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(k4_protocol::cat::rx_eq_flat());
            }
        }
        WorkerCmd::Cat(cmd) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&cmd);
                ws.diag.log(Level::Debug, "tx", &cmd);
            }
        }
        WorkerCmd::CatLocal(cmd) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&cmd);
                s.apply_local(&cmd);
                ws.diag.log(Level::Debug, "tx", &cmd);
            }
        }
        // Audio device / level control (FR-AUD-DEV-01 / FR-AUD-LVL-01). Device
        // changes recreate the stream (only while a session's streams exist).
        WorkerCmd::SetOutputDevice(name) => {
            ws.out_device = name;
            if ws.audio_out.is_some() {
                ws.audio_out = AudioOutput::with_device(ws.out_device.as_deref()).ok();
                if let Some(out) = ws.audio_out.as_mut() {
                    out.set_volume(ws.volume);
                }
            }
        }
        WorkerCmd::SetInputDevice(name) => {
            ws.in_device = name;
            if ws.audio_in.is_some() {
                ws.audio_in = AudioInput::with_device(ws.in_device.as_deref()).ok();
                if let Some(inp) = ws.audio_in.as_mut() {
                    inp.set_mic_gain(ws.mic_gain);
                }
            }
        }
        WorkerCmd::SetVolume(v) => {
            ws.volume = v;
            if let Some(out) = ws.audio_out.as_mut() {
                out.set_volume(v);
            }
        }
        WorkerCmd::SetMicGain(g) => {
            ws.mic_gain = g;
            if let Some(inp) = ws.audio_in.as_mut() {
                inp.set_mic_gain(g);
            }
        }
        WorkerCmd::SetKpodEnabled(on) => {
            #[cfg(feature = "kpod")]
            ws.kpod.set_enabled(on, &mut ws.diag);
            #[cfg(not(feature = "kpod"))]
            let _ = on;
        }
        WorkerCmd::SetKpodButtons(cats) => {
            #[cfg(feature = "kpod")]
            ws.kpod.set_buttons(cats);
            #[cfg(not(feature = "kpod"))]
            let _ = cats;
        }
    }
}

/// Elecraft K-Pod USB control-surface integration (FR-KPOD-04). Feature-gated
/// (`kpod`): a dedicated thread owns the HID device (its blocking USB reads must
/// not stall the audio/CAT service loop) — it polls the K-Pod, drives the
/// indicator LEDs, and forwards events over a channel. The worker drains that
/// channel each loop and applies tuning to the connected session: the rocker
/// selects VFO A / VFO B / RIT-XIT, the encoder tunes it. The pure protocol +
/// mapping (`k4_kpod`) is unit-tested; the HID I/O here is L4 hardware.
#[cfg(feature = "kpod")]
mod kpod {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::Link;
    use k4_diag::{DiagLog, Level};
    use k4_kpod::device::Kpod;
    use k4_kpod::{action_for, led_aux_packet, selection_leds, Action, Report, Rocker, Tuner};

    /// Message from the K-Pod thread to the worker.
    enum KpodMsg {
        Connected,
        Lost,
        Event(Report),
    }

    pub(super) struct KpodState {
        /// Runtime opt-in (config toggle), shared with the poll thread.
        enabled: Arc<AtomicBool>,
        rx: Receiver<KpodMsg>,
        tuner: [Tuner; 2], // [VFO A, VFO B] running tune targets
        /// Last TX-VFO we commanded from the rocker (via split): Some(true)=B.
        tx_vfo: Option<bool>,
        /// Per-slot CAT macro strings (F1–F8 × tap/hold; see `k4_kpod::slot_index`)
        /// sent to the K4 on a switch press (FR-KPOD-06). Empty = unassigned.
        buttons: Vec<String>,
        /// Last macro we fired and when, to collapse a burst of identical reports
        /// into one press. The K-Pod emits **one** resolved `'u'` event per gesture
        /// (tap = hold-bit 0, hold = 1; confirmed live — a 1–2 s hold yields a
        /// single report), so each gesture fires once; a short window guards
        /// against any repeated report without dropping deliberate repeats.
        last_fire: Option<((u8, bool), Instant)>,
        _handle: thread::JoinHandle<()>,
    }

    impl KpodState {
        /// Spawn the (idle) poll thread; it does nothing until `set_enabled(true)`.
        pub(super) fn new() -> Self {
            let enabled = Arc::new(AtomicBool::new(false));
            let (tx, rx) = mpsc::channel();
            let flag = Arc::clone(&enabled);
            let handle = thread::Builder::new()
                .name("kpod".into())
                .spawn(move || poll_loop(&flag, &tx))
                .expect("spawn kpod thread");
            Self {
                enabled,
                rx,
                tuner: [Tuner::default(); 2],
                tx_vfo: None,
                buttons: vec![String::new(); k4_kpod::SLOT_COUNT],
                last_fire: None,
                _handle: handle,
            }
        }

        /// Replace the function-switch macro table (from the config editor,
        /// FR-KPOD-06). Length is normalized to `SLOT_COUNT`.
        pub(super) fn set_buttons(&mut self, mut cats: Vec<String>) {
            cats.resize(k4_kpod::SLOT_COUNT, String::new());
            self.buttons = cats;
        }

        /// Enable/disable at runtime (config toggle); the poll thread reacts.
        pub(super) fn set_enabled(&mut self, on: bool, diag: &mut DiagLog) {
            self.enabled.store(on, Ordering::Relaxed);
            diag.log(
                Level::Info,
                "kpod",
                if on {
                    "K-Pod enabled — searching for device"
                } else {
                    "K-Pod disabled"
                },
            );
        }

        /// Drain pending K-Pod messages (non-blocking) and apply tuning if a
        /// session is connected. Recognition (connect/disconnect) is logged
        /// regardless of the radio link.
        ///
        /// trace: FR-KPOD-04
        pub(super) fn service(&mut self, mut session: Option<&mut Link>, diag: &mut DiagLog) {
            while let Ok(msg) = self.rx.try_recv() {
                match msg {
                    KpodMsg::Connected => diag.log(Level::Info, "kpod", "K-Pod connected"),
                    KpodMsg::Lost => diag.log(Level::Info, "kpod", "K-Pod disconnected"),
                    KpodMsg::Event(report) => {
                        if let Some(s) = session.as_deref_mut() {
                            // Rocker A/B → switch the K4 TX VFO via split: VFO A =
                            // split off (TX on A), VFO B = split on (TX on B). Only
                            // on a genuine change; RIT/XIT/idle leave TX alone. The
                            // app reflects the TX VFO from the split state.
                            let want_tx_b = match report.rocker {
                                Rocker::VfoA => Some(false),
                                Rocker::VfoB => Some(true),
                                _ => None,
                            };
                            if let Some(tx_b) = want_tx_b {
                                if self.tx_vfo != Some(tx_b) {
                                    self.tx_vfo = Some(tx_b);
                                    let cmd = if tx_b { "FT1;" } else { "FT0;" };
                                    let _ = s.send(cmd);
                                    s.apply_local(cmd);
                                }
                            }
                            self.fire_button(&report, s, diag);
                            self.apply(report, s);
                        }
                        // No session → ignore (nothing to tune/switch yet).
                    }
                }
            }
        }

        /// Run a function-switch macro when a switch is pressed (FR-KPOD-06):
        /// look up the (button, tap/hold) slot's CAT string and send it to the
        /// K4. The K-Pod emits one resolved `'u'` event per gesture (tap → hold=0,
        /// hold → hold=1; confirmed live), so we fire once per *pure* button event
        /// (`ticks == 0`) — collapsing only a burst of the identical `(button,
        /// hold)` within a short window. Encoder-tick reports carry the held-button
        /// state too, so they're excluded here (else a held switch re-fires on
        /// every tick while tuning); they still tune via [`Self::apply`].
        fn fire_button(&mut self, report: &Report, session: &mut Link, diag: &mut DiagLog) {
            // Only a *pure* button event (no encoder motion) is a switch press. The
            // K-Pod carries the held-button state on encoder-tick reports too, so
            // without this a held switch re-fires its macro on every tick while the
            // knob turns — flooding the K4. `ticks != 0` reports still tune (via
            // `apply`); they just don't re-fire the macro.
            if report.button == 0 || report.ticks != 0 {
                return;
            }
            let press = (report.button, report.hold);
            // The K-Pod emits one resolved event per gesture (tap → hold=0, hold →
            // hold=1), with no release/idle event to reset between presses. Fire on
            // every button event so repeated identical gestures (tap, tap) each
            // count — collapsing only a burst of the *same* (button, hold) within a
            // short window, in case a report is ever repeated.
            let now = Instant::now();
            if let Some((last, at)) = self.last_fire {
                if last == press && now.duration_since(at) < Duration::from_millis(150) {
                    return;
                }
            }
            self.last_fire = Some((press, now));
            let Some(idx) = k4_kpod::slot_index(report.button, report.hold) else {
                return;
            };
            let cat = match self.buttons.get(idx) {
                Some(c) if !c.is_empty() => c.clone(),
                _ => return,
            };
            diag.log(
                Level::Info,
                "kpod",
                &format!(
                    "F{} {} → {cat}",
                    report.button,
                    if report.hold { "hold" } else { "tap" }
                ),
            );
            // The K4 parses concatenated `;`-terminated commands, so send the
            // whole macro in one shot; fold each command into local state
            // (apply_cat handles one at a time) for optimistic display.
            let _ = session.send(&cat);
            for part in cat.split_inclusive(';') {
                let cmd = part.trim();
                if !cmd.is_empty() {
                    session.apply_local(cmd);
                }
            }
        }

        /// Apply one decoded event to the connected session (tuning).
        fn apply(&mut self, report: Report, session: &mut Link) {
            let step = session.state().tune_step_hz.unwrap_or(10);
            // RIT/XIT is a fine ±9999 Hz offset — tune it at 10 Hz/tick regardless
            // of the (possibly coarse) VFO rate, so it doesn't slam to the clamp.
            const RIT_STEP_HZ: u32 = 10;
            match action_for(&report, step, RIT_STEP_HZ) {
                Action::Tune { vfo_b, delta_hz } => {
                    let idx = usize::from(vfo_b);
                    let radio = if vfo_b {
                        session.state().vfo_b_hz
                    } else {
                        session.state().vfo_a_hz
                    };
                    if let Some(new) = self.tuner[idx].tune(radio, delta_hz) {
                        let cmd = if vfo_b {
                            k4_protocol::cat::set_vfo_b_hz(new)
                        } else {
                            k4_protocol::cat::set_vfo_a_hz(new)
                        };
                        let _ = session.send(&cmd);
                        // Reflect it locally now so the app's readout follows the
                        // knob immediately (the K4 does not auto-echo SETs); the
                        // ~8 s resync reconciles.
                        session.apply_local(&cmd);
                    }
                }
                Action::RitXit { delta_hz } => {
                    // The offset only shifts the frequency with RIT (or XIT) on,
                    // so enable RIT if neither is — otherwise turning the knob
                    // silently changes a stored value with no audible effect,
                    // which reads as "out of sync" against the radio.
                    if session.state().rit_on != Some(true) && session.state().xit_on != Some(true)
                    {
                        let _ = session.send("RT1;");
                        session.apply_local("RT1;");
                    }
                    let cur = i64::from(session.state().rit_offset.unwrap_or(0));
                    let off = (cur + delta_hz).clamp(-9999, 9999) as i16;
                    let cmd = k4_protocol::cat::set_rit_offset(off);
                    let _ = session.send(&cmd);
                    session.apply_local(&cmd);
                }
                Action::None => {}
            }
            // Hand back to the radio's real value once it confirms our target.
            self.tuner[0].sync(session.state().vfo_a_hz);
            self.tuner[1].sync(session.state().vfo_b_hz);
        }
    }

    /// The K-Pod poll thread: opens the device when enabled, retries discovery,
    /// drives the selection LEDs, and forwards events. It owns all the blocking
    /// USB I/O so the worker's service loop never stalls. Exits when the worker
    /// (and thus the channel receiver) is dropped.
    fn poll_loop(enabled: &Arc<AtomicBool>, tx: &Sender<KpodMsg>) {
        let mut dev: Option<Kpod> = None;
        let mut last_rocker: Option<Rocker> = None;
        loop {
            if !enabled.load(Ordering::Relaxed) {
                if dev.take().is_some() {
                    last_rocker = None;
                    if tx.send(KpodMsg::Lost).is_err() {
                        return;
                    }
                }
                thread::sleep(Duration::from_millis(150));
                continue;
            }
            if dev.is_none() {
                match Kpod::open() {
                    Ok(d) => {
                        dev = Some(d);
                        if tx.send(KpodMsg::Connected).is_err() {
                            return;
                        }
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(1000)); // retry discovery
                        continue;
                    }
                }
            }
            let d = dev.as_ref().expect("device present");
            match d.poll() {
                Ok(report) => {
                    if report.is_event() {
                        if last_rocker != Some(report.rocker) {
                            last_rocker = Some(report.rocker);
                            let _ = d.command(led_aux_packet(selection_leds(report.rocker)));
                        }
                        if tx.send(KpodMsg::Event(report)).is_err() {
                            return; // worker gone
                        }
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => {
                    dev = None;
                    last_rocker = None;
                    if tx.send(KpodMsg::Lost).is_err() {
                        return;
                    }
                    thread::sleep(Duration::from_millis(1000));
                }
            }
        }
    }
}
