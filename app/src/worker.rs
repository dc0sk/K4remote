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
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
    /// Set main-RX mode (raw `MD` digit, e.g. 3 = CW).
    SetMode(u8),
    /// Band up (`true`) / down (`false`).
    Band(bool),
    /// Toggle the RX attenuator (`RA/`).
    ToggleAtten,
    /// Toggle split (`FT/`).
    ToggleSplit,
    /// Cycle AGC off → slow → fast.
    CycleAgc,
    /// Toggle the noise blanker (`SW32`).
    ToggleNb,
    /// Toggle noise reduction (`SW62`).
    ToggleNr,
    /// Toggle the preamp (`PA/`).
    TogglePreamp,
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
}

/// Snapshot the UI renders from (FR-UI-02/03/06). Written by the worker.
#[derive(Debug, Clone, Default)]
pub struct UiSnapshot {
    pub connected: bool,
    pub transmitting: bool,
    pub tx_armed: bool,
    pub vfo_a_hz: Option<u64>,
    pub vfo_b_hz: Option<u64>,
    pub mode_a: Option<&'static str>,
    pub split: Option<bool>,
    /// Main-RX S-meter bar count (`SM`).
    pub s_meter_bars: Option<u8>,
    /// Main-RX high-resolution S-meter, dBm (`SMH`).
    pub s_meter_dbm: Option<i32>,
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
    /// Latest spectrum trace (downsampled dBm bins).
    pub spectrum_latest: Vec<f32>,
    /// Waterfall history, newest row first (downsampled dBm bins).
    pub waterfall: Vec<Vec<f32>>,
    /// Recent diagnostic log lines (FR-DIAG-01/02).
    pub diag_lines: Vec<String>,
    /// Human-readable status / last error.
    pub status: String,
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
    spectrum_latest: Vec<f32>,
    waterfall: VecDeque<Vec<f32>>,
    diag: DiagLog,
    // Reconnect (FR-SES-RECONNECT): retained target + backoff schedule.
    connect_params: Option<ConnectTarget>,
    backoff: Backoff,
    next_attempt: Option<Instant>,
}

impl WorkerState {
    fn new() -> Self {
        Self {
            session: None,
            rx_audio: JitterBuffer::new(8),
            rx_decoder: None,
            audio_out: None,
            audio_in: None,
            tx_encoder: None,
            tx_seq: 0,
            audio_frames: 0,
            spectrum_bins: 0,
            spectrum_latest: Vec::new(),
            waterfall: VecDeque::new(),
            // Debug level so the raw CAT console shows traffic; bounded ring.
            diag: DiagLog::new(300, Level::Debug),
            connect_params: None,
            backoff: Backoff::default(),
            next_attempt: None,
        }
    }
    /// Reset per-connection state. The Opus codecs and device streams are
    /// re-created per connection (decoders are stateful; devices may change).
    fn reset(&mut self) {
        self.rx_audio = JitterBuffer::new(8);
        self.rx_decoder = OpusDecoder::rx().ok();
        self.audio_out = AudioOutput::new().ok();
        self.audio_in = AudioInput::new().ok();
        self.tx_encoder = OpusEncoder::mono().ok();
        self.tx_seq = 0;
        self.audio_frames = 0;
        self.spectrum_bins = 0;
        self.spectrum_latest.clear();
        self.waterfall.clear();
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

fn publish(snapshot: &Arc<Mutex<UiSnapshot>>, ws: &WorkerState) {
    let Some(session) = ws.session.as_ref() else {
        return;
    };
    let st: &RadioState = session.state();
    if let Ok(mut s) = snapshot.lock() {
        s.connected = session.is_connected();
        s.transmitting = session.is_transmitting();
        s.tx_armed = session.is_tx_armed();
        s.vfo_a_hz = st.vfo_a_hz;
        s.vfo_b_hz = st.vfo_b_hz;
        s.mode_a = st.mode_a.map(mode_label);
        s.split = st.split;
        s.s_meter_bars = st.s_meter_bars;
        s.s_meter_dbm = st.s_meter_dbm;
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
        s.spectrum_latest = ws.spectrum_latest.clone();
        s.waterfall = ws.waterfall.iter().cloned().collect();
        s.diag_lines = ws.diag.recent(30);
    }
}

/// Attempt to (re)connect using the retained params. On success, set up the
/// session + audio and clear the backoff; on failure, schedule the next retry.
fn attempt_connect(ws: &mut WorkerState, snapshot: &Arc<Mutex<UiSnapshot>>) {
    let Some(target) = ws.connect_params.clone() else {
        return;
    };

    // Open the link and pick a session profile. Serial has no PING/PONG, so its
    // keep-alive + link-loss are effectively disabled.
    let timeout = Duration::from_millis(100);
    let (result, session_cfg, desc, secret): (std::io::Result<AnyLink>, _, _, String) =
        match &target {
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
                let scheme = if *use_tls { "tls" } else { "tcp" };
                (
                    open_tcp(host, *port, &cfg, *use_tls).map(AnyLink::Tcp),
                    SessionConfig::default(),
                    format!("{host}:{port} ({scheme})"),
                    password.clone(),
                )
            }
            ConnectTarget::Serial { path, baud } => (
                SerialPortTransport::open(path, *baud, timeout).map(AnyLink::Serial),
                SessionConfig {
                    ping_interval: Duration::from_secs(3600),
                    link_timeout: Duration::from_secs(86_400),
                },
                format!("{path}@{baud}"),
                String::new(),
            ),
        };

    match result {
        Ok(link) => {
            let mut s = Session::new(link, SystemClock, session_cfg);
            let _ = s.seed();
            ws.reset();
            ws.session = Some(s);
            ws.backoff.reset();
            ws.next_attempt = None;
            let msg = format!("connected to {desc}");
            ws.diag.log(Level::Info, "net", &msg);
            set_status(snapshot, msg);
        }
        Err(e) => {
            let delay = ws.backoff.next_delay();
            ws.next_attempt = Some(Instant::now() + delay);
            // Defensively redact any secret in case an error echoes it (NFR-SEC-01).
            let msg = k4_config::redact(
                &format!(
                    "connect to {desc} failed: {e} — retry in {:?} (attempt {})",
                    delay,
                    ws.backoff.attempts()
                ),
                &secret,
            );
            ws.diag.log(Level::Warn, "net", &msg);
            set_status(snapshot, msg);
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

        // 2. Service the link, attempt a scheduled reconnect, or idle.
        if ws.session.is_some() {
            service(&mut ws, &snapshot);
        } else {
            if let Some(at) = ws.next_attempt {
                if Instant::now() >= at {
                    ws.next_attempt = None;
                    attempt_connect(&mut ws, &snapshot);
                }
            }
            // Publish diagnostics even while disconnected (connect errors, etc.).
            if let Ok(mut s) = snapshot.lock() {
                s.diag_lines = ws.diag.recent(30);
            }
            thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Pump the session once, demux inbound frames, and publish the snapshot.
fn service(ws: &mut WorkerState, snapshot: &Arc<Mutex<UiSnapshot>>) {
    let Some(session) = ws.session.as_mut() else {
        return;
    };

    if let Ok(inbound) = session.pump() {
        // RX audio → jitter buffer (reorder) → Opus decode to PCM → speaker.
        for payload in &inbound.audio {
            if let Some(pkt) = AudioPacket::decode(payload) {
                ws.rx_audio.push(pkt.sequence, pkt.data.to_vec());
            }
        }
        while let Some(opus_frame) = ws.rx_audio.pop() {
            match ws.rx_decoder.as_mut() {
                Some(dec) => {
                    if let Ok(pcm) = dec.decode_float(&opus_frame) {
                        if let Some(out) = ws.audio_out.as_mut() {
                            out.submit_stereo_12k(&pcm);
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
                ws.spectrum_bins = frame.bins_dbm.len();
                let row = downsample(&frame.bins_dbm, SPECTRUM_WIDTH);
                ws.spectrum_latest = row.clone();
                ws.waterfall.push_front(row);
                while ws.waterfall.len() > WATERFALL_ROWS {
                    ws.waterfall.pop_back();
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
            set_status(
                snapshot,
                format!(
                    "link lost — reconnecting in {:?} (attempt {})",
                    delay,
                    ws.backoff.attempts()
                ),
            );
        }
    }
}

fn handle_cmd(cmd: WorkerCmd, ws: &mut WorkerState, snapshot: &Arc<Mutex<UiSnapshot>>) {
    match cmd {
        WorkerCmd::Connect(target) => {
            ws.connect_params = Some(target);
            ws.backoff.reset();
            ws.next_attempt = None;
            attempt_connect(ws, snapshot);
        }
        WorkerCmd::Disconnect => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.disconnect();
            }
            ws.session = None;
            ws.connect_params = None; // stop auto-reconnect
            ws.next_attempt = None;
            ws.backoff.reset();
            ws.diag.log(Level::Info, "net", "disconnected");
            set_status(snapshot, "disconnected");
        }
        WorkerCmd::SetFreqA(hz) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&k4_protocol::cat::set_vfo_a_hz(hz));
            }
        }
        WorkerCmd::SetMode(digit) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(&k4_protocol::cat::set_mode(digit));
            }
        }
        WorkerCmd::Band(up) => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send(if up {
                    k4_protocol::cat::band_up()
                } else {
                    k4_protocol::cat::band_down()
                });
            }
        }
        WorkerCmd::ToggleAtten => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send("RA/;"); // toggle form
            }
        }
        WorkerCmd::ToggleSplit => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send("FT/;"); // toggle form
            }
        }
        WorkerCmd::CycleAgc => {
            if let Some(s) = ws.session.as_mut() {
                let next = (s.state().agc_mode.unwrap_or(0) + 1) % 3;
                let _ = s.send(&k4_protocol::cat::set_agc(next));
            }
        }
        WorkerCmd::ToggleNb => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send("SW32;"); // tap NB
            }
        }
        WorkerCmd::ToggleNr => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send("SW62;"); // tap NR
            }
        }
        WorkerCmd::TogglePreamp => {
            if let Some(s) = ws.session.as_mut() {
                let _ = s.send("PA/;");
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
    }
}
