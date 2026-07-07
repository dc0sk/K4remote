//! K4 Remote — GUI application (ARC-08, iced).
//!
//! The view is a pure projection of [`UiSnapshot`] (ADR-04); all radio I/O runs
//! on a background [`worker`] thread, bridged by a command channel + a shared
//! snapshot polled on a timer (ADR-06, FR-UI-07). Layout and styling follow the
//! K4's native LCD and the reference client's visual language (R-EXT-02,
//! ADR-15): a dark layered theme, banded frame, grids of two-line state
//! buttons, and proportional S-meter bars (FR-UI-08..15).

mod spectrum;
mod ui;
mod worker;

use ui::ViewMode;

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use k4_config::{Config, Profile, SecretStore};

use std::cell::Cell;

use iced::widget::canvas::Canvas;
use iced::widget::{
    button, container, horizontal_space, mouse_area, pick_list, progress_bar, scrollable, slider,
    stack, vertical_slider,
};
use iced::widget::{Button, Column, Container, ProgressBar, Row, Text, TextInput};
use iced::{Alignment, Background, Border, Color, Element, Length, Subscription, Task, Theme};

thread_local! {
    /// The palette the free `shade`/`role_color` helpers resolve against
    /// (FR-UI-17). Set once per frame from the active [`ui::ThemeMode`]; the
    /// UI is single-threaded so this stays stable across a frame's build+draw.
    static ACTIVE_THEME: Cell<ui::EffectiveTheme> = const { Cell::new(ui::EffectiveTheme::Dark) };
}

fn set_active_theme(theme: ui::EffectiveTheme) {
    ACTIVE_THEME.with(|c| c.set(theme));
}

fn active_theme() -> ui::EffectiveTheme {
    ACTIVE_THEME.with(|c| c.get())
}

use worker::{ConnectTarget, UiSnapshot, WorkerCmd};

pub fn main() -> iced::Result {
    // App icon (taskbar / window bar), embedded from the packaging assets.
    let icon = iced::window::icon::from_file_data(
        include_bytes!("../../packaging/icons/k4remote-128.png"),
        None,
    )
    .ok();
    iced::application("K4 Remote", App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        // Start in landscape (wider than tall) — the layout is horizontal.
        .window(iced::window::Settings {
            size: iced::Size::new(ui::DEFAULT_WINDOW_SIZE.0, ui::DEFAULT_WINDOW_SIZE.1),
            icon,
            ..Default::default()
        })
        .run_with(App::new)
}

struct App {
    // connection form
    host: String,
    port: String,
    password: String,
    // tuning form
    freq_mhz: String,
    // use TLS-PSK (port 9204) instead of plaintext (9205)
    use_tls: bool,
    // serial (USB/RS232) transport instead of Ethernet
    serial_mode: bool,
    serial_path: String,
    serial_baud: String,
    // remember the password in the OS keychain (FR-CFG-03)
    remember: bool,
    // `Arc` so keychain I/O can move off the UI thread (it blocks on the Secret
    // Service / D-Bus and can hang on a locked keyring).
    secret_store: Arc<dyn SecretStore>,
    // raw CAT command entry (diagnostics console)
    cat_input: String,
    // where the config (profiles/prefs) is persisted
    config_path: Option<PathBuf>,
    // bridge to the worker
    cmd_tx: Sender<WorkerCmd>,
    snapshot: Arc<Mutex<UiSnapshot>>,
    // last snapshot read (what the view renders)
    ui: UiSnapshot,
    // switchable single-A / single-B / dual view (FR-UI-08, ARC-15)
    view_mode: ViewMode,
    // which primary's context row is open (FR-UI-13, ARC-15)
    context: ui::ContextRow,
    // current window width, for responsive band layout (FR-UI-12)
    window_w: f32,
    // selected UI theme (FR-UI-17)
    theme_mode: ui::ThemeMode,
    // detected OS dark preference, for the `System` theme (FR-UI-17)
    system_is_dark: bool,
    // whether the About box is showing (FR-UI-18)
    about_open: bool,
    // graphic-EQ band gains, −16..+16 dB (FR-EQ-01). RX EQ (`RE`) is one control
    // shared by the MAIN/SUB RX screens; TX EQ (`TE`) is separate.
    rx_eq: [i8; 8],
    tx_eq: [i8; 8],
    // panadapter/display parameters (FR-PAN-CTL-01, DISPLAY screen)
    display: DisplayState,
    // transmit config + which TX sub-panel is shown (Phase C)
    tx_cfg: TxConfig,
    tx_tab: TxTab,
    // RX config-row sub-screens (Phase D)
    rx_cfg: RxConfig,
    rx_tab: RxTab,
    // MENU screen search filter (FR-MENU-01)
    menu_filter: String,
    // TX text-message entry (FR-TX-MSG-01), Fn sub-panel + DX search
    tx_text: String,
    fn_tab: FnTab,
    dx_filter: String,
    // Whether the config screens have been seeded from the radio's read-back
    // values this connection (FR-UI-19 read-back). Reset on disconnect.
    seeded: bool,
    // Peer cache + settings dialog (FR-CFG-04, FR-UI-23).
    peers: k4_config::PeerCache,
    settings_open: bool,
    // Encrypt cached peer passwords under a master password (vs OS keychain).
    use_master: bool,
    master_password: String,
    master_key: Option<k4_config::MasterKey>,
    peer_status: String,
    // Cache the connected peer once per connection (FR-CFG-04).
    peer_cached: bool,
    // Audio device selection + local levels (FR-AUD-DEV-01/LVL-01).
    audio_outputs: Vec<String>,
    audio_inputs: Vec<String>,
    selected_output: Option<String>,
    selected_input: Option<String>,
    volume: f32,   // RX playback gain 0.0–2.0
    mic_gain: f32, // TX capture gain 0.0–3.0
    // Two-step guard for the remote power-off (FR-PWR-01).
    power_off_armed: bool,
    // Diagnostics log: show/hide + follow-newest (auto-scroll).
    show_log: bool,
    log_autoscroll: bool,
    log_id: scrollable::Id,
    log_len: usize,
    // Local RX filter bandwidth, Hz (seeded from the radio; cycled by the BW btn).
    bw_hz: u32,
    // Which VFO transmits (B under split); tracks the radio, set optimistically
    // when a spectrum frame is clicked so the highlight moves immediately.
    tx_vfo_b: bool,
    // Ticks to keep the optimistic tx_vfo_b before resuming the split read-back,
    // so the highlight doesn't flicker back before the radio's echo lands.
    tx_vfo_hold: u8,
    // Main-RX levels (seeded from the radio, driven by sliders): AF gain 0–60,
    // RF-gain attenuation 0–60 dB, squelch 0–40 (FR-RX-01, FR-RX-SQL-01).
    af_gain: u8,
    rf_gain: u8,
    squelch: u8,
    // TX levels: power (W, QRO) and speech compression 0–30 (FR-TX-02, FR-TX-CMP-01).
    tx_power: u16,
    compression: u8,
    // CW sidetone pitch Hz (FR-KEY-02); full-QSK + VOX/QSK delay 10-ms (FR-TX-DLY-01).
    cw_pitch: u16,
    qsk_full: bool,
    qsk_delay: u8,
    // Passband shift / AF center pitch, Hz (FR-FIL-01).
    shift_hz: u16,
}

/// Which graphic equalizer a screen edits (FR-EQ-01).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EqTarget {
    Rx,
    Tx,
}

/// Panadapter/display parameters shown on the DISPLAY screen (FR-PAN-CTL-01).
/// Held locally (the radio doesn't yet report them into the snapshot) and pushed
/// on change; reading them back is a follow-up.
#[derive(Debug, Clone, Copy)]
struct DisplayState {
    ref_db: i16,
    span_hz: u32,
    scale: u8,
    avg: u8,
    peak: bool,
    freeze: bool,
    wf_palette: u8,
    wf_height: u8,
}

impl Default for DisplayState {
    fn default() -> Self {
        Self {
            ref_db: -130,
            span_hz: 50_000,
            scale: 70,
            avg: 4,
            peak: false,
            freeze: false,
            wf_palette: 1,
            wf_height: 60,
        }
    }
}

/// A single DISPLAY-screen adjustment (FR-PAN-CTL-01).
#[derive(Debug, Clone, Copy)]
enum DispMsg {
    Mode(ViewMode),
    Ref(i16),
    Span(u32),
    Scale(u8),
    Avg(u8),
    Peak(bool),
    Freeze(bool),
    Palette(u8),
    Height(u8),
}

/// Transmit-configuration state shown on the TX screen (FR-KEY-01/FR-AUD-CFG-01).
/// Held locally and pushed on change (read-back is a follow-up).
#[derive(Debug, Clone, Copy)]
struct TxConfig {
    keyer_speed: u8,      // KS, 8–100 WPM
    weight: u16,          // KP weight, 90–125 (×0.01)
    paddle_reverse: bool, // KP
    iambic_b: bool,       // KP
    mic_input: u8,        // MI, 0–4
    mic_gain: u8,         // MG, 0–80
    front_preamp: u8,     // MS field a, 0–2
    front_bias: bool,     // MS field b
    line_level: u16,      // LI level (both channels)
    line_use_jack: bool,  // LI source
    antenna: u8,          // AN, TX antenna 1–3
    vox: bool,            // VX, voice VOX on/off
}

impl Default for TxConfig {
    fn default() -> Self {
        Self {
            keyer_speed: 20,
            weight: 110,
            paddle_reverse: false,
            iambic_b: false,
            mic_input: 0,
            mic_gain: 30,
            front_preamp: 0,
            front_bias: false,
            line_level: 20,
            line_use_jack: false,
            antenna: 1,
            vox: false,
        }
    }
}

/// Which sub-panel the TX screen shows (mirrors the K4's TX config row).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TxTab {
    Eq,
    Keyer,
    Mic,
    Line,
    Ant,
    Text,
}

/// Which sub-panel the Fn screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FnTab {
    Keys,
    Switches,
    Dx,
}

/// A single TX-config adjustment (FR-KEY-01/FR-AUD-CFG-01/FR-ANT-01).
#[derive(Debug, Clone, Copy)]
enum TxMsg {
    KeyerSpeed(u8),
    Weight(u16),
    PaddleReverse(bool),
    IambicB(bool),
    MicInput(u8),
    MicGain(u8),
    FrontPreamp(u8),
    FrontBias(bool),
    LineLevel(u16),
    LineUseJack(bool),
    Antenna(u8),
    Vox(bool),
}

/// RX config-row sub-screen state (FR-ANT-01/FR-AUD-CFG-01, Phase D). Held
/// locally and pushed on change.
#[derive(Debug, Clone, Copy)]
struct RxConfig {
    ant: u8,       // AR, main RX, 0–7
    ant_sub: u8,   // AR$, sub RX, 0–7
    lo_left: u8,   // LO left level (main), 0–40
    lo_right: u8,  // LO right level (sub), 0–40
    lo_gang: bool, // LO right-follows-left
}

impl Default for RxConfig {
    fn default() -> Self {
        Self {
            ant: 4,
            ant_sub: 4,
            lo_left: 10,
            lo_right: 10,
            lo_gang: true,
        }
    }
}

/// Which sub-panel the MAIN/SUB RX screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RxTab {
    Eq,
    Ant,
    LineOut,
    Filter,
}

/// A single RX config adjustment (FR-ANT-01/FR-AUD-CFG-01).
#[derive(Debug, Clone, Copy)]
enum RxMsg {
    Tab(RxTab),
    Ant(bool, u8), // (is_sub, AR value)
    LoLeft(u8),
    LoRight(u8),
    LoGang(bool),
}

#[derive(Debug, Clone)]
enum Message {
    HostChanged(String),
    PortChanged(String),
    PasswordChanged(String),
    FreqChanged(String),
    Connect,
    Disconnect,
    ToggleTls,
    ToggleRemember,
    ToggleSerialMode,
    SerialPathChanged(String),
    SerialBaudChanged(String),
    SetFreq,
    SetMode(u8),
    Band(bool),
    ToggleAtten,
    ToggleSplit,
    CycleAgc,
    CycleBandwidth,
    SelectTxVfo(bool),
    SetAfGain(u8),
    SetRfGain(u8),
    SetSquelch(u8),
    SetTxPower(u16),
    SetCompression(u8),
    SetCwPitch(u16),
    ToggleQskFull,
    SetQskDelay(u8),
    SetShift(u16),
    FilterPreset(u8),
    FilterNormalize,
    ToggleNb,
    ToggleNr,
    TogglePreamp,
    ToggleRit,
    ToggleXit,
    ClearRitXit,
    ToggleArm,
    ToggleKey,
    EmergencyStop,
    CatInputChanged(String),
    SendCat,
    SetViewMode(ViewMode),
    TapPrimary(ui::Primary),
    CycleTheme,
    ToggleAbout,
    /// Open a URL in the OS browser (About-box links / donate).
    OpenUrl(&'static str),
    // Settings dialog + peer cache (FR-UI-23, FR-CFG-04).
    ToggleSettings,
    UseMasterToggled(bool),
    MasterPasswordChanged(String),
    UnlockMaster,
    SelectPeer(usize),
    DeletePeer(usize),
    // Audio device + level controls (FR-AUD-DEV-01, FR-AUD-LVL-01).
    SelectOutputDevice(String),
    SelectInputDevice(String),
    VolumeChanged(f32),
    MicGainChanged(f32),
    SaveSettings,
    // Remote power control (FR-PWR-01).
    PowerRestart,
    PowerOffArm,
    PowerOffCancel,
    PowerOffConfirm,
    // Diagnostics log display options.
    ToggleShowLog,
    ToggleLogAutoscroll,
    // Graphic-EQ screens (FR-EQ-01).
    EqChanged(EqTarget, usize, i8),
    EqFlat(EqTarget),
    // DISPLAY screen (FR-PAN-CTL-01) and BAND screen (FR-VFO-04).
    Disp(DispMsg),
    SelectBand(u8),
    BandStack,
    // TX config (FR-KEY-01/FR-AUD-CFG-01), Fn VFO ops (FR-VFO-07), MENU (FR-MENU-01).
    SetTxTab(TxTab),
    Tx(TxMsg),
    VfoOp(u8),
    MenuOpen(u16),
    // Front-panel switch emulation (FR-SW-01): quick memories, PF keys.
    Switch(u16),
    // MENU screen search (FR-MENU-01).
    MenuFilter(String),
    // BAND transverter select (FR-VFO-04), TX text (FR-TX-MSG-01), Fn tabs / DX.
    SelectXvtr(u8),
    TxText(String),
    SendTxText,
    SetFnTab(FnTab),
    DxFilter(String),
    // RX config sub-screens (FR-ANT-01/FR-AUD-CFG-01, Phase D).
    Rx(RxMsg),
    Resized(f32),
    Tick,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        // `--demo` seeds the shared snapshot with sample state so the UI can be
        // inspected offline (coloured freqs, S-meter, chips). The worker only
        // writes diagnostics while disconnected, so the seed persists.
        let demo = std::env::args().any(|a| a == "--demo");
        let initial = if demo {
            worker::demo_snapshot()
        } else {
            UiSnapshot::default()
        };
        let snapshot = Arc::new(Mutex::new(initial.clone()));
        worker::spawn(cmd_rx, Arc::clone(&snapshot));

        // Load persisted config: prefill the last-used connection (FR-CFG-01/05)
        // and restore the peer cache (FR-CFG-04).
        let config_path = k4_config::default_config_path();
        let mut config = config_path.as_deref().map(Config::load).unwrap_or_default();
        let peers = std::mem::take(&mut config.peers);
        let prefs = std::mem::take(&mut config.prefs);
        let last = config.last.take().unwrap_or(Profile {
            host: "192.168.1.100".into(),
            port: 9205,
            use_tls: false,
            remember: false,
        });

        // Audio device lists + restored levels (FR-AUD-DEV-01/LVL-01, FR-CFG-05).
        let audio_outputs = k4_audio::output_device_names();
        let audio_inputs = k4_audio::input_device_names();
        let selected_output = prefs.audio_output.clone();
        let selected_input = prefs.audio_input.clone();
        let volume = prefs.volume_pct as f32 / 100.0;
        let mic_gain = prefs.mic_gain_pct as f32 / 100.0;
        // Seed the worker with the restored audio settings before any connect.
        let _ = cmd_tx.send(WorkerCmd::SetOutputDevice(selected_output.clone()));
        let _ = cmd_tx.send(WorkerCmd::SetInputDevice(selected_input.clone()));
        let _ = cmd_tx.send(WorkerCmd::SetVolume(volume));
        let _ = cmd_tx.send(WorkerCmd::SetMicGain(mic_gain));
        let theme_mode = theme_from_prefs(prefs.theme.as_deref());

        // Choose a secret store; load the saved password if "remember" is set.
        #[cfg(feature = "keychain")]
        let secret_store: Arc<dyn SecretStore> = Arc::new(k4_config::KeyringStore::new("k4remote"));
        #[cfg(not(feature = "keychain"))]
        let secret_store: Arc<dyn SecretStore> = Arc::new(k4_config::MemoryStore::new());

        // Keychain reads block on the Secret Service and can hang on a locked
        // keyring; bound startup with a timeout so it can never freeze the app.
        let password = if last.remember {
            let store = Arc::clone(&secret_store);
            let acct = account_key(&last.host, last.port);
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(store.get(&acct));
            });
            rx.recv_timeout(Duration::from_secs(2))
                .ok()
                .flatten()
                .unwrap_or_default()
        } else {
            String::new()
        };

        let app = App {
            host: last.host,
            port: last.port.to_string(),
            password,
            freq_mhz: "14.074".into(),
            use_tls: last.use_tls,
            serial_mode: false,
            serial_path: "/dev/ttyUSB0".into(),
            serial_baud: "38400".into(),
            remember: last.remember,
            secret_store,
            cat_input: String::new(),
            config_path,
            cmd_tx,
            snapshot,
            ui: initial,
            view_mode: ViewMode::default(),
            context: ui::ContextRow::opened(ui::Primary::Band),
            window_w: 1280.0,
            theme_mode,
            system_is_dark: detect_system_dark(),
            about_open: false,
            rx_eq: [0; 8],
            tx_eq: [0; 8],
            display: DisplayState::default(),
            tx_cfg: TxConfig::default(),
            tx_tab: TxTab::Keyer,
            rx_cfg: RxConfig::default(),
            rx_tab: RxTab::Eq,
            menu_filter: String::new(),
            tx_text: String::new(),
            fn_tab: FnTab::Keys,
            dx_filter: String::new(),
            seeded: false,
            peers,
            settings_open: false,
            use_master: false,
            master_password: String::new(),
            master_key: None,
            peer_status: String::new(),
            peer_cached: false,
            audio_outputs,
            audio_inputs,
            selected_output,
            selected_input,
            volume,
            mic_gain,
            power_off_armed: false,
            show_log: true,
            log_autoscroll: true,
            log_id: scrollable::Id::new("diag-log"),
            log_len: 0,
            bw_hz: 2800,
            tx_vfo_b: false,
            tx_vfo_hold: 0,
            af_gain: 30,
            rf_gain: 0,
            squelch: 0,
            tx_power: 10,
            compression: 0,
            cw_pitch: 600,
            qsk_full: false,
            qsk_delay: 30,
            shift_hz: 1500,
        };
        (app, Task::none())
    }

    /// Persist the current connection profile (no password) and store/clear the
    /// password in the keychain per the "remember" choice (FR-CFG-01/03).
    fn save_profile(&self) {
        let Ok(port) = self.port.parse::<u16>() else {
            return;
        };
        let account = account_key(&self.host, port);
        // Keychain I/O blocks on the Secret Service (D-Bus) and can hang on a
        // locked keyring — never run it on the UI thread (that froze the app on
        // Connect). Fire-and-forget on a worker thread.
        let store = Arc::clone(&self.secret_store);
        let remember = self.remember;
        let password = self.password.clone();
        std::thread::spawn(move || {
            let _ = if remember {
                store.set(&account, &password)
            } else {
                store.delete(&account)
            };
        });
        self.save_config();
    }

    /// Write the full config (last session + peer cache) to disk (FR-CFG-01/04/05).
    /// The peer cache is included so a save never wipes it.
    fn save_config(&self) {
        let Ok(port) = self.port.parse::<u16>() else {
            return;
        };
        if let Some(path) = self.config_path.as_deref() {
            let cfg = Config {
                last: Some(Profile {
                    host: self.host.clone(),
                    port,
                    use_tls: self.use_tls,
                    remember: self.remember,
                }),
                peers: self.peers.clone(),
                prefs: k4_config::Prefs {
                    audio_output: self.selected_output.clone(),
                    audio_input: self.selected_input.clone(),
                    volume_pct: (self.volume * 100.0).round() as u16,
                    mic_gain_pct: (self.mic_gain * 100.0).round() as u16,
                    theme: Some(theme_to_str(self.theme_mode).to_string()),
                    ..Default::default()
                },
            };
            let _ = cfg.save(path);
        }
    }

    /// Read a secret from the store with a bounded timeout (the keychain can
    /// block on a locked Secret Service; never freeze the UI on a click).
    fn secret_get_timed(&self, account: &str) -> String {
        let store = Arc::clone(&self.secret_store);
        let acct = account.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(store.get(&acct));
        });
        rx.recv_timeout(Duration::from_secs(2))
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Set or unlock the store master password for encrypted peer secrets
    /// (FR-CFG-04). First use establishes salt + verifier; later use verifies.
    fn unlock_master(&mut self) {
        if self.master_password.is_empty() {
            self.peer_status = "enter a master password".into();
            return;
        }
        let existed = self.peers.has_master();
        let res = if existed {
            self.peers.unlock(&self.master_password)
        } else {
            self.peers.init_master(&self.master_password)
        };
        match res {
            Ok(key) => {
                self.master_key = Some(key);
                self.use_master = true;
                self.peer_status = if existed {
                    "unlocked".into()
                } else {
                    "master password set".into()
                };
                if !existed {
                    self.save_config(); // persist the new salt + verifier
                }
            }
            Err(e) => {
                self.master_key = None;
                self.peer_status = e.to_string();
            }
        }
    }

    /// Populate the connection form from a cached peer, decrypting/retrieving its
    /// password per its storage mode (FR-CFG-04).
    fn select_peer(&mut self, i: usize) {
        let Some(peer) = self.peers.peers.get(i).cloned() else {
            return;
        };
        self.serial_mode = false;
        self.host = peer.host.clone();
        self.port = peer.port.to_string();
        self.use_tls = peer.use_tls;
        match &peer.secret {
            k4_config::PeerSecret::None => {
                self.password.clear();
                self.remember = false;
                self.peer_status = format!("selected {}", peer.name);
            }
            k4_config::PeerSecret::Keyring => {
                self.password = self.secret_get_timed(&account_key(&peer.host, peer.port));
                self.remember = true;
                self.use_master = false;
                self.peer_status = format!("selected {} (keychain)", peer.name);
            }
            k4_config::PeerSecret::Encrypted(sealed) => {
                self.use_master = true;
                self.remember = true;
                match &self.master_key {
                    Some(key) => match key.open(sealed) {
                        Ok(pw) => {
                            self.password = pw;
                            self.peer_status = format!("selected {} (decrypted)", peer.name);
                        }
                        Err(e) => {
                            self.password.clear();
                            self.peer_status = format!("decrypt failed: {e}");
                        }
                    },
                    None => {
                        self.password.clear();
                        self.peer_status = "unlock with the master password first".into();
                    }
                }
            }
        }
    }

    /// Delete a cached peer and its stored secret (FR-CFG-04).
    fn delete_peer(&mut self, i: usize) {
        let Some(peer) = self.peers.peers.get(i).cloned() else {
            return;
        };
        if matches!(peer.secret, k4_config::PeerSecret::Keyring) {
            let store = Arc::clone(&self.secret_store);
            let acct = account_key(&peer.host, peer.port);
            std::thread::spawn(move || {
                let _ = store.delete(&acct);
            });
        }
        self.peers.remove(&peer.host, peer.port);
        self.peer_status = format!("deleted {}", peer.name);
        self.save_config();
    }

    /// Cache the just-connected peer (FR-CFG-04): store its password per the
    /// chosen mode (encrypted under the master key / OS keychain / none).
    fn cache_current_peer(&mut self) {
        if self.serial_mode {
            return;
        }
        let Ok(port) = self.port.parse::<u16>() else {
            return;
        };
        let secret = if self.use_master {
            match self
                .master_key
                .as_ref()
                .and_then(|k| k.seal(&self.password).ok())
            {
                Some(s) => k4_config::PeerSecret::Encrypted(s),
                None => k4_config::PeerSecret::None,
            }
        } else if self.remember {
            k4_config::PeerSecret::Keyring // save_profile already stored it
        } else {
            k4_config::PeerSecret::None
        };
        self.peers.upsert(k4_config::Peer {
            name: self.host.clone(),
            host: self.host.clone(),
            port,
            use_tls: self.use_tls,
            secret,
        });
        self.save_config();
    }

    fn send(&self, cmd: WorkerCmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::HostChanged(v) => self.host = v,
            Message::PortChanged(v) => self.port = v,
            Message::PasswordChanged(v) => self.password = v,
            Message::FreqChanged(v) => self.freq_mhz = v,
            Message::Connect => {
                if self.serial_mode {
                    self.send(WorkerCmd::Connect(ConnectTarget::Serial {
                        path: self.serial_path.clone(),
                        baud: self.serial_baud.trim().parse().unwrap_or(38400),
                    }));
                } else if let Ok(port) = self.port.parse::<u16>() {
                    self.save_profile(); // remember host/port/tls for next launch
                    self.send(WorkerCmd::Connect(ConnectTarget::Tcp {
                        host: self.host.clone(),
                        port,
                        password: self.password.clone(),
                        use_tls: self.use_tls,
                    }));
                }
            }
            Message::Disconnect => {
                self.context.close(); // collapse any open context row
                self.send(WorkerCmd::Disconnect);
            }
            Message::ToggleTls => {
                self.use_tls = !self.use_tls;
                // Convenience: default each scheme to its standard port.
                self.port = if self.use_tls { "9204" } else { "9205" }.into();
            }
            Message::ToggleRemember => self.remember = !self.remember,
            Message::ToggleSerialMode => self.serial_mode = !self.serial_mode,
            Message::SerialPathChanged(v) => self.serial_path = v,
            Message::SerialBaudChanged(v) => self.serial_baud = v,
            Message::SetFreq => {
                if let Some(hz) = parse_mhz(&self.freq_mhz) {
                    self.send(WorkerCmd::SetFreqA(hz));
                }
            }
            Message::SetMode(digit) => self.send(WorkerCmd::SetMode(digit)),
            Message::Band(up) => self.send(WorkerCmd::Band(up)),
            Message::ToggleAtten => self.send(WorkerCmd::ToggleAtten),
            Message::ToggleSplit => self.send(WorkerCmd::ToggleSplit),
            Message::CycleAgc => self.send(WorkerCmd::CycleAgc),
            Message::CycleBandwidth => {
                // Step through common RX filter bandwidths (wraps at the top);
                // update locally for immediate feedback, then push to the radio.
                const BW: [u32; 8] = [500, 1000, 1500, 1800, 2400, 2700, 2800, 3200];
                self.bw_hz = BW
                    .iter()
                    .copied()
                    .find(|&b| b > self.bw_hz)
                    .unwrap_or(BW[0]);
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_bandwidth_hz(
                    self.bw_hz,
                )));
            }
            Message::SelectTxVfo(is_b) => {
                // TX VFO = B under split, A otherwise (FT / split). FR-UI-12.
                // Move the highlight immediately and hold it ~1.5 s while the
                // radio's split echo makes its way back (else it flickers back).
                self.tx_vfo_b = is_b;
                self.tx_vfo_hold = 10;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_split(is_b)));
            }
            Message::SetAfGain(v) => {
                self.af_gain = v;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_af_gain(v)));
            }
            Message::SetRfGain(v) => {
                self.rf_gain = v;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_rf_gain(v)));
            }
            Message::SetSquelch(v) => {
                self.squelch = v;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_squelch(v)));
            }
            Message::SetTxPower(v) => {
                self.tx_power = v;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_tx_power(v)));
            }
            Message::SetCompression(v) => {
                self.compression = v;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_compression(v)));
            }
            Message::SetCwPitch(hz) => {
                self.cw_pitch = hz;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_cw_pitch(hz)));
            }
            Message::ToggleQskFull => {
                self.qsk_full = !self.qsk_full;
                let mc = self.tx_mode_class();
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_qsk_delay(
                    self.qsk_full,
                    mc,
                    self.qsk_delay,
                )));
            }
            Message::SetQskDelay(v) => {
                self.qsk_delay = v;
                let mc = self.tx_mode_class();
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_qsk_delay(
                    self.qsk_full,
                    mc,
                    v,
                )));
            }
            Message::SetShift(hz) => {
                self.shift_hz = hz;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_shift_hz(hz)));
            }
            Message::FilterPreset(n) => {
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_filter_preset(n)))
            }
            Message::FilterNormalize => self.send(WorkerCmd::Cat(
                k4_protocol::cat::filter_normalize().to_string(),
            )),
            Message::ToggleNb => self.send(WorkerCmd::ToggleNb),
            Message::ToggleNr => self.send(WorkerCmd::ToggleNr),
            Message::TogglePreamp => self.send(WorkerCmd::TogglePreamp),
            Message::ToggleRit => self.send(WorkerCmd::ToggleRit),
            Message::ToggleXit => self.send(WorkerCmd::ToggleXit),
            Message::ClearRitXit => self.send(WorkerCmd::ClearRitXit),
            Message::ToggleArm => self.send(WorkerCmd::ArmTx(!self.ui.tx_armed)),
            Message::ToggleKey => self.send(WorkerCmd::Key(!self.ui.transmitting)),
            Message::EmergencyStop => self.send(WorkerCmd::EmergencyStop),
            Message::CatInputChanged(v) => self.cat_input = v,
            Message::SendCat => {
                let cmd = self.cat_input.trim();
                if !cmd.is_empty() {
                    self.send(WorkerCmd::SendRawCat(cmd.to_string()));
                    self.cat_input.clear();
                }
            }
            Message::SetViewMode(m) => self.view_mode = m,
            Message::TapPrimary(p) => {
                self.power_off_armed = false; // navigating away disarms power-off
                self.context.tap(p);
            }
            Message::CycleTheme => {
                self.theme_mode = self.theme_mode.next();
                // Re-detect the OS preference when entering System.
                if self.theme_mode == ui::ThemeMode::System {
                    self.system_is_dark = detect_system_dark();
                }
                self.save_config(); // remember the theme (FR-CFG-05)
            }
            Message::ToggleAbout => self.about_open = !self.about_open,
            Message::OpenUrl(url) => open_url(url),
            Message::ToggleSettings => self.settings_open = !self.settings_open,
            Message::UseMasterToggled(v) => self.use_master = v,
            Message::MasterPasswordChanged(v) => self.master_password = v,
            Message::UnlockMaster => self.unlock_master(),
            Message::SelectPeer(i) => self.select_peer(i),
            Message::DeletePeer(i) => self.delete_peer(i),
            Message::SelectOutputDevice(name) => {
                self.selected_output = (name != DEVICE_DEFAULT).then_some(name);
                self.send(WorkerCmd::SetOutputDevice(self.selected_output.clone()));
                self.save_config();
            }
            Message::SelectInputDevice(name) => {
                self.selected_input = (name != DEVICE_DEFAULT).then_some(name);
                self.send(WorkerCmd::SetInputDevice(self.selected_input.clone()));
                self.save_config();
            }
            Message::VolumeChanged(v) => {
                self.volume = v;
                self.send(WorkerCmd::SetVolume(v));
            }
            Message::MicGainChanged(g) => {
                self.mic_gain = g;
                self.send(WorkerCmd::SetMicGain(g));
            }
            Message::SaveSettings => self.save_config(),
            Message::PowerRestart => self.send(WorkerCmd::Cat(k4_protocol::cat::set_power(8))),
            Message::PowerOffArm => self.power_off_armed = true,
            Message::PowerOffCancel => self.power_off_armed = false,
            Message::PowerOffConfirm => {
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_power(0)));
                self.power_off_armed = false;
            }
            Message::ToggleShowLog => self.show_log = !self.show_log,
            Message::ToggleLogAutoscroll => {
                self.log_autoscroll = !self.log_autoscroll;
                if self.log_autoscroll {
                    // Resume following the newest line.
                    return scrollable::snap_to(
                        self.log_id.clone(),
                        scrollable::RelativeOffset::END,
                    );
                }
                // Freeze: convert the sticky "bottom" (Relative(1.0)) into a
                // concrete absolute offset near the current end so new lines no
                // longer drag the view. ~15 px per line at text size 11.
                return scrollable::scroll_to(
                    self.log_id.clone(),
                    scrollable::AbsoluteOffset {
                        x: 0.0,
                        y: self.log_len as f32 * 15.0,
                    },
                );
            }
            Message::EqChanged(target, band, value) => {
                let v = value.clamp(-ui::EQ_DB_RANGE, ui::EQ_DB_RANGE);
                match target {
                    EqTarget::Rx => {
                        self.rx_eq[band] = v;
                        self.send(WorkerCmd::SetRxEq(self.rx_eq));
                    }
                    EqTarget::Tx => {
                        self.tx_eq[band] = v;
                        self.send(WorkerCmd::SetTxEq(self.tx_eq));
                    }
                }
            }
            Message::EqFlat(target) => match target {
                EqTarget::Rx => {
                    self.rx_eq = [0; 8];
                    self.send(WorkerCmd::RxEqFlat);
                }
                EqTarget::Tx => {
                    self.tx_eq = [0; 8];
                    self.send(WorkerCmd::SetTxEq(self.tx_eq));
                }
            },
            Message::Disp(d) => self.apply_disp(d),
            Message::SelectBand(bn) => self.send(WorkerCmd::Cat(k4_protocol::cat::set_band(bn))),
            Message::BandStack => self.send(WorkerCmd::Cat(
                k4_protocol::cat::band_stack_next().to_string(),
            )),
            Message::SetTxTab(t) => self.tx_tab = t,
            Message::Tx(t) => self.apply_tx(t),
            Message::VfoOp(op) => self.send(WorkerCmd::Cat(k4_protocol::cat::vfo_copy_swap(op))),
            Message::MenuOpen(id) => self.send(WorkerCmd::Cat(k4_protocol::cat::menu_open(id))),
            Message::Switch(code) => self.send(WorkerCmd::Cat(k4_protocol::cat::switch(code))),
            Message::MenuFilter(q) => self.menu_filter = q,
            Message::SelectXvtr(n) => {
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_transverter_band(n)))
            }
            Message::TxText(s) => self.tx_text = s,
            Message::SendTxText => {
                let t = self.tx_text.trim();
                if !t.is_empty() {
                    self.send(WorkerCmd::Cat(k4_protocol::cat::send_text(t)));
                    self.tx_text.clear();
                }
            }
            Message::SetFnTab(t) => self.fn_tab = t,
            Message::DxFilter(q) => self.dx_filter = q,
            Message::Rx(m) => self.apply_rx(m),
            Message::Resized(w) => self.window_w = w,
            Message::Tick => {
                if let Ok(snap) = self.snapshot.lock() {
                    self.ui = snap.clone();
                }
                // Seed the config screens from the radio's reported values once
                // per connection, as the connect GET burst lands (FR-UI-19).
                if self.ui.phase == ui::ConnPhase::Connected {
                    if !self.seeded {
                        self.seed_from_radio();
                    }
                    if !self.peer_cached {
                        self.cache_current_peer(); // FR-CFG-04
                        self.peer_cached = true;
                    }
                } else {
                    self.seeded = false;
                    self.peer_cached = false;
                    self.power_off_armed = false;
                }
                // Track the radio's transmit VFO (split) when it reports one, but
                // not during the post-click hold window (avoids a flicker-back
                // before the radio's echo lands).
                if self.tx_vfo_hold > 0 {
                    self.tx_vfo_hold -= 1;
                } else if let Some(s) = self.ui.split {
                    self.tx_vfo_b = s;
                }
                // Follow the newest log line while auto-scroll is on (only when
                // the log actually grew, so manual scrolling isn't fought).
                let n = self.ui.diag_lines.len();
                let grew = n != self.log_len;
                self.log_len = n;
                if self.show_log && self.log_autoscroll && grew {
                    return scrollable::snap_to(
                        self.log_id.clone(),
                        scrollable::RelativeOffset::END,
                    );
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        // Poll the shared snapshot ~6×/s; the UI thread never blocks on I/O.
        let tick = iced::time::every(Duration::from_millis(150)).map(|_| Message::Tick);
        // Track window width for the responsive band layout (FR-UI-12).
        let resize = iced::window::resize_events().map(|(_id, size)| Message::Resized(size.width));
        Subscription::batch([tick, resize])
    }

    /// The effective (resolved) theme for this frame (FR-UI-17).
    fn effective_theme(&self) -> ui::EffectiveTheme {
        self.theme_mode.effective(self.system_is_dark)
    }

    /// Layered theme (FR-UI-15/17): background/panel/control surfaces from the
    /// active `ui::Shade` palette, semantic accents from `ui::ColorRole`.
    /// Publishes the active palette so the free `shade`/`role_color` helpers
    /// (and every `.style` closure that calls them) resolve to this theme.
    fn theme(&self) -> Theme {
        set_active_theme(self.effective_theme());
        Theme::custom(
            format!("K4 {}", self.theme_mode.label()),
            iced::theme::Palette {
                background: shade(ui::Shade::Bg),
                text: role_color(ui::ColorRole::RxValue),
                primary: role_color(ui::ColorRole::VfoA),
                success: role_color(ui::ColorRole::VfoB),
                danger: DANGER,
            },
        )
    }

    /// One VFO panel for the header band (FR-UI-12): receiver badge, big
    /// dot-grouped frequency, mode, and a proportional S-meter bar — the
    /// reference client's per-VFO block (FR-UI-09/10/15).
    fn vfo_panel(&self, pane: ui::Pane) -> Element<'_, Message> {
        let is_b = pane.is_b();
        let hz = if is_b {
            self.ui.vfo_b_hz
        } else {
            self.ui.vfo_a_hz
        };
        let mode = if is_b { self.ui.mode_b } else { self.ui.mode_a };
        let dbm = if is_b {
            self.ui.s_meter_dbm_sub
        } else {
            self.ui.s_meter_dbm
        };
        // The TX VFO is B under split, else A (FR-UI-12).
        let is_tx_vfo = is_b == (self.ui.split == Some(true));
        let role = ui::vfo_role(is_b, is_tx_vfo, self.ui.transmitting);
        // Strong signal (≥ S9, −73 dBm) reads "caution" yellow (FR-UI-10).
        let strong = matches!(dbm, Some(d) if d >= -73);
        let meter_role = if strong {
            ui::ColorRole::Caution
        } else {
            ui::ColorRole::RxValue
        };
        // Big readout is white like the K4/reference; amber while this VFO
        // transmits so the operating VFO is unmistakable (FR-UI-06/10).
        let freq_role = if self.ui.transmitting && is_tx_vfo {
            ui::ColorRole::TxActive
        } else {
            ui::ColorRole::RxValue
        };

        let head = Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(badge(pane.label(), role))
            .push(
                Text::new(ui::format_freq_opt(hz))
                    .size(38)
                    .color(role_color(freq_role)),
            )
            .push(horizontal_space())
            .push(
                Text::new(mode.unwrap_or("—"))
                    .size(15)
                    .color(role_color(role)),
            );

        // Proportional S-meter on the K4's S1..S9+60 face (FR-UI-15).
        let frac = dbm.map(ui::s_meter_fraction).unwrap_or(0.0);
        let meter = Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(
                ProgressBar::new(0.0..=1.0, frac)
                    .height(Length::Fixed(10.0))
                    .style(meter_style(strong)),
            )
            .push(
                Text::new(fmt_dbm(dbm))
                    .size(12)
                    .color(role_color(meter_role)),
            );

        // Fixed height matching the centre box (Fill is not allowed inside the
        // scrollable body), content vertically centred.
        Container::new(Column::new().spacing(8).push(head).push(meter))
            .style(panel_style)
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fixed(VFO_BAND_H))
            .align_y(Alignment::Center)
            .into()
    }

    /// The shared TX / SPLIT / RIT-XIT box that sits between the VFOs (FR-UI-12),
    /// so transmit routing is always visible. TX lights amber while transmitting.
    fn center_box(&self) -> Element<'_, Message> {
        let txing = self.ui.transmitting;
        let tx_ind: Element<Message> = Container::new(
            Text::new(if txing { "● TX" } else { "TX" })
                .size(13)
                .color(if txing {
                    Color::BLACK
                } else {
                    role_color(ui::ColorRole::Inactive)
                }),
        )
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(if txing {
                role_color(ui::ColorRole::TxActive)
            } else {
                shade(ui::Shade::Track)
            })),
            border: Border {
                color: shade(ui::Shade::Edge),
                width: 1.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        })
        .padding([3, 12])
        .into();

        // 2×2 grid of equally-sized two-line buttons (SPLIT/RIT over XIT/CLR),
        // keeping the box the same height as the VFO panels.
        let clr_state = ui::ButtonState {
            label: "CLR",
            value: "RIT/XIT".to_string(),
        };
        let col = Column::new()
            .spacing(6)
            .align_x(Alignment::Center)
            .push(tx_ind)
            .push(
                Row::new()
                    .spacing(6)
                    .push(two_line_btn(
                        ui::toggle_button("SPLIT", self.ui.split),
                        self.ui.split,
                        Some(Message::ToggleSplit),
                    ))
                    .push(two_line_btn(
                        ui::toggle_button("RIT", self.ui.rit_on),
                        self.ui.rit_on,
                        Some(Message::ToggleRit),
                    )),
            )
            .push(
                Row::new()
                    .spacing(6)
                    .push(two_line_btn(
                        ui::toggle_button("XIT", self.ui.xit_on),
                        self.ui.xit_on,
                        Some(Message::ToggleXit),
                    ))
                    .push(two_line_btn(clr_state, None, Some(Message::ClearRitXit))),
            );
        Container::new(col)
            .style(panel_style)
            .padding(10)
            .height(Length::Fixed(VFO_BAND_H))
            .align_y(Alignment::Center)
            .into()
    }

    /// Apply a DISPLAY-screen adjustment: update local state and push the `#`
    /// display command (FR-PAN-CTL-01).
    fn apply_disp(&mut self, d: DispMsg) {
        use k4_protocol::cat;
        let cmd = match d {
            DispMsg::Mode(m) => {
                self.view_mode = m;
                cat::set_pan_mode(pan_mode_code(m))
            }
            DispMsg::Ref(v) => {
                self.display.ref_db = v.clamp(-200, 60);
                cat::set_pan_ref(self.display.ref_db)
            }
            DispMsg::Span(v) => {
                self.display.span_hz = v.clamp(6000, 368_000);
                cat::set_pan_span_hz(self.display.span_hz)
            }
            DispMsg::Scale(v) => {
                self.display.scale = v.clamp(10, 150);
                cat::set_pan_scale(self.display.scale)
            }
            DispMsg::Avg(v) => {
                self.display.avg = v.clamp(1, 20);
                cat::set_pan_average(self.display.avg)
            }
            DispMsg::Peak(on) => {
                self.display.peak = on;
                cat::set_pan_peak(on)
            }
            DispMsg::Freeze(on) => {
                self.display.freeze = on;
                cat::set_pan_freeze(on)
            }
            DispMsg::Palette(p) => {
                self.display.wf_palette = p.min(4);
                cat::set_waterfall_palette(self.display.wf_palette)
            }
            DispMsg::Height(h) => {
                self.display.wf_height = h.min(100);
                cat::set_waterfall_height(self.display.wf_height)
            }
        };
        self.send(WorkerCmd::Cat(cmd));
    }

    /// Apply a TX-config adjustment: update local state and push the command
    /// (FR-KEY-01/FR-AUD-CFG-01). Keyer weight/paddle/iambic all live in `KP`.
    fn apply_tx(&mut self, t: TxMsg) {
        use k4_protocol::cat;
        let cmd = {
            let c = &mut self.tx_cfg;
            match t {
                TxMsg::KeyerSpeed(v) => {
                    c.keyer_speed = v.clamp(8, 100);
                    cat::set_keyer_speed(c.keyer_speed)
                }
                TxMsg::Weight(v) => {
                    c.weight = v.clamp(90, 125);
                    cat::set_keyer(c.iambic_b, c.paddle_reverse, c.weight)
                }
                TxMsg::PaddleReverse(b) => {
                    c.paddle_reverse = b;
                    cat::set_keyer(c.iambic_b, c.paddle_reverse, c.weight)
                }
                TxMsg::IambicB(b) => {
                    c.iambic_b = b;
                    cat::set_keyer(c.iambic_b, c.paddle_reverse, c.weight)
                }
                TxMsg::MicInput(n) => {
                    c.mic_input = n.min(4);
                    cat::set_mic_input(c.mic_input)
                }
                TxMsg::MicGain(v) => {
                    c.mic_gain = v.min(80);
                    cat::set_mic_gain(c.mic_gain)
                }
                TxMsg::FrontPreamp(v) => {
                    c.front_preamp = v.min(2);
                    cat::set_mic_setup(c.front_preamp, c.front_bias, false, 0, false)
                }
                TxMsg::FrontBias(b) => {
                    c.front_bias = b;
                    cat::set_mic_setup(c.front_preamp, c.front_bias, false, 0, false)
                }
                TxMsg::LineLevel(v) => {
                    c.line_level = v.min(250);
                    cat::set_line_in(c.line_level, c.line_level, c.line_use_jack)
                }
                TxMsg::LineUseJack(b) => {
                    c.line_use_jack = b;
                    cat::set_line_in(c.line_level, c.line_level, c.line_use_jack)
                }
                TxMsg::Antenna(n) => {
                    c.antenna = n.clamp(1, 3);
                    cat::set_tx_antenna(c.antenna)
                }
                TxMsg::Vox(on) => {
                    c.vox = on;
                    cat::set_vox('V', on)
                }
            }
        };
        self.send(WorkerCmd::Cat(cmd));
    }

    /// Apply an RX config adjustment: update local state and push the command
    /// (FR-ANT-01/FR-AUD-CFG-01, Phase D).
    fn apply_rx(&mut self, m: RxMsg) {
        use k4_protocol::cat;
        if let RxMsg::Tab(t) = m {
            self.rx_tab = t;
            return;
        }
        let cmd = {
            let c = &mut self.rx_cfg;
            match m {
                RxMsg::Ant(true, n) => {
                    c.ant_sub = n.min(7);
                    cat::set_rx_antenna_sub(c.ant_sub)
                }
                RxMsg::Ant(false, n) => {
                    c.ant = n.min(7);
                    cat::set_rx_antenna(c.ant)
                }
                RxMsg::LoLeft(v) => {
                    c.lo_left = v.min(40);
                    cat::set_line_out(c.lo_left, c.lo_right, c.lo_gang)
                }
                RxMsg::LoRight(v) => {
                    c.lo_right = v.min(40);
                    cat::set_line_out(c.lo_left, c.lo_right, c.lo_gang)
                }
                RxMsg::LoGang(b) => {
                    c.lo_gang = b;
                    cat::set_line_out(c.lo_left, c.lo_right, c.lo_gang)
                }
                RxMsg::Tab(_) => unreachable!(),
            }
        };
        self.send(WorkerCmd::Cat(cmd));
    }

    /// MAIN/SUB RX screen (FR-EQ-01/FR-ANT-01/FR-AUD-CFG-01): a tab row
    /// (EQ / ANT / LINE OUT) mirroring the K4's RX config row. LINE OUT is
    /// main-RX only (its right channel is the sub receiver).
    fn rx_screen(&self, sub: bool) -> Element<'_, Message> {
        let tab_btn = |tab: RxTab, label: &'static str| -> Element<Message> {
            let active = self.rx_tab == tab;
            Button::new(Text::new(label).size(12))
                .style(btn_style(if active {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([5, 12])
                .on_press(Message::Rx(RxMsg::Tab(tab)))
                .into()
        };
        let mut tabs = Row::new()
            .spacing(6)
            .push(tab_btn(RxTab::Eq, "EQ"))
            .push(tab_btn(RxTab::Ant, "ANT"));
        if !sub {
            tabs = tabs
                .push(tab_btn(RxTab::Filter, "FILTER"))
                .push(tab_btn(RxTab::LineOut, "LINE OUT"));
        }
        let content: Element<Message> = match self.rx_tab {
            RxTab::Ant => self.rx_ant_panel(sub),
            RxTab::Filter if !sub => self.rx_filter_panel(),
            RxTab::LineOut if !sub => self.line_out_panel(),
            // EQ (and LINE OUT falls back to EQ on the sub receiver).
            _ if sub => self.eq_screen(
                EqTarget::Rx,
                "Sub-RX equalizer",
                Some("Shares the RX EQ command (RE); independent sub-RX targeting pending radio verification."),
            ),
            _ => self.eq_screen(EqTarget::Rx, "RX equalizer", None),
        };
        Column::new().spacing(12).push(tabs).push(content).into()
    }

    /// RX → FILTER sub-panel: per-mode filter presets (`FP`, FR-MODE-03),
    /// passband shift / AF center pitch (`IS`, FR-FIL-01), and a BW shortcut.
    fn rx_filter_panel(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        let preset = |label: &'static str, n: u8| {
            small_btn_string(label.to_string(), Message::FilterPreset(n))
        };
        Column::new()
            .spacing(12)
            .push(Text::new("Filter presets (saved per mode)").size(12).color(rxv))
            .push(
                Row::new()
                    .spacing(6)
                    .push(preset("FL1", 1))
                    .push(preset("FL2", 2))
                    .push(preset("FL3", 3))
                    .push(small_btn("NORMALIZE", Message::FilterNormalize)),
            )
            .push(
                Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(Text::new("SHIFT").size(11).color(dim))
                    .push(
                        slider(200..=3000u16, self.shift_hz, Message::SetShift)
                            .step(10u16)
                            .width(Length::Fixed(180.0)),
                    )
                    .push(Text::new(format!("{} Hz", self.shift_hz)).size(11).color(rxv)),
            )
            .push(
                Row::new().spacing(6).align_y(Alignment::Center).push(small_btn_string(
                    format!("WIDTH: {:.2} kHz  (cycle)", self.bw_hz as f32 / 1000.0),
                    Message::CycleBandwidth,
                )),
            )
            .push(
                Text::new("Shift = IS AF center pitch; presets FP1–3 are per-mode. Sub-RX filter pending.")
                    .size(10)
                    .color(dim),
            )
            .into()
    }

    /// RX → ANT sub-panel (`AR`/`AR$`): cycle the RX antenna for this receiver.
    fn rx_ant_panel(&self, sub: bool) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let cur = if sub {
            self.rx_cfg.ant_sub
        } else {
            self.rx_cfg.ant
        };
        let name = ui::rx_antenna_names()[(cur as usize).min(7)];
        Column::new()
            .spacing(10)
            .push(
                Text::new(if sub {
                    "Sub-RX antenna"
                } else {
                    "Main-RX antenna"
                })
                .size(12)
                .color(role_color(ui::ColorRole::RxValue)),
            )
            .push(small_btn_string(
                format!("ANT: {name}"),
                Message::Rx(RxMsg::Ant(sub, (cur + 1) % 8)),
            ))
            .push(
                Text::new("0 Off · 1 RX2 · 2 =TX · 3 XVTR · 4 RX1 · 5–7 ATU RX (KAT4).")
                    .size(10)
                    .color(dim),
            )
            .into()
    }

    /// RX → LINE OUT sub-panel (`LO`): left (main) / right (sub) levels + gang.
    fn line_out_panel(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let c = self.rx_cfg;
        let mut row = Row::new()
            .spacing(18)
            .align_y(Alignment::Center)
            .push(disp_stepper(
                "LEFT",
                c.lo_left.to_string(),
                Message::Rx(RxMsg::LoLeft(c.lo_left.saturating_sub(1))),
                Message::Rx(RxMsg::LoLeft(c.lo_left + 1)),
            ));
        if !c.lo_gang {
            row = row.push(disp_stepper(
                "RIGHT",
                c.lo_right.to_string(),
                Message::Rx(RxMsg::LoRight(c.lo_right.saturating_sub(1))),
                Message::Rx(RxMsg::LoRight(c.lo_right + 1)),
            ));
        }
        row = row.push(small_btn_string(
            format!("R=L: {}", if c.lo_gang { "On" } else { "Off" }),
            Message::Rx(RxMsg::LoGang(!c.lo_gang)),
        ));
        Column::new()
            .spacing(10)
            .push(
                Text::new("Line-out levels (left = main RX, right = sub RX)")
                    .size(12)
                    .color(dim),
            )
            .push(row)
            .into()
    }

    /// TX configuration screen (FR-KEY-01/FR-AUD-CFG-01, SCR-*): a tab row
    /// (EQ / KEYER / MIC / LINE) mirroring the K4's TX config row, with the
    /// selected sub-panel below.
    fn tx_screen(&self) -> Element<'_, Message> {
        let tab_btn = |tab: TxTab, label: &'static str| -> Element<Message> {
            let active = self.tx_tab == tab;
            Button::new(Text::new(label).size(12))
                .style(btn_style(if active {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([5, 12])
                .on_press(Message::SetTxTab(tab))
                .into()
        };
        let tabs = Row::new()
            .spacing(6)
            .push(tab_btn(TxTab::Eq, "EQ"))
            .push(tab_btn(TxTab::Keyer, "KEYER"))
            .push(tab_btn(TxTab::Mic, "MIC"))
            .push(tab_btn(TxTab::Line, "LINE"))
            .push(tab_btn(TxTab::Ant, "ANT"))
            .push(tab_btn(TxTab::Text, "TEXT"));
        let content: Element<Message> = match self.tx_tab {
            TxTab::Eq => self.eq_screen(EqTarget::Tx, "TX equalizer", None),
            TxTab::Keyer => self.tx_keyer(),
            TxTab::Mic => self.tx_mic(),
            TxTab::Line => self.tx_line(),
            TxTab::Ant => self.tx_ant_panel(),
            TxTab::Text => self.tx_text_panel(),
        };
        Column::new().spacing(12).push(tabs).push(content).into()
    }

    /// The K4's transmit/antenna dual-function switches (tap left / hold right,
    /// `SW` emulation) as a compact grid for the TRANSMIT panel (FR-SW-01).
    fn tx_switch_grid(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let cell = |label: &str, code: u16| -> Element<Message> {
            Button::new(Text::new(label.to_string()).size(11))
                .style(btn_style(BtnKind::Plain))
                .padding([4, 6])
                .width(Length::Fixed(88.0))
                .on_press(Message::Switch(code))
                .into()
        };
        let mut grid = Column::new().spacing(4);
        for pair in ui::tx_function_switches().chunks(2) {
            let mut row = Row::new().spacing(10);
            for (tl, tap, hl, hold) in pair {
                row = row.push(
                    Row::new()
                        .spacing(3)
                        .push(cell(tl, *tap))
                        .push(cell(hl, *hold)),
                );
            }
            grid = grid.push(row);
        }
        Column::new()
            .spacing(4)
            .push(Text::new("Switches (tap · hold)").size(10).color(dim))
            .push(grid)
            .into()
    }

    /// TX → TEXT sub-panel (`KY`): type a CW/DATA message and send it.
    fn tx_text_panel(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        Column::new()
            .spacing(10)
            .push(
                Text::new("Send CW / DATA text (transmits the message)")
                    .size(12)
                    .color(role_color(ui::ColorRole::RxValue)),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(
                        TextInput::new("message text (≤60 chars)", &self.tx_text)
                            .on_input(Message::TxText)
                            .on_submit(Message::SendTxText)
                            .size(13)
                            .width(Length::Fixed(360.0)),
                    )
                    .push(small_btn("SEND", Message::SendTxText)),
            )
            .push(
                Text::new(
                    "Sent via KY; requires TX to be armed. Prosigns: ( = KN, + = AR, = = BT.",
                )
                .size(10)
                .color(dim),
            )
            .into()
    }

    /// TX → ANT sub-panel (`AN`): transmit antenna ANT1–3.
    fn tx_ant_panel(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let mut row = Row::new().spacing(6);
        for n in 1u8..=3 {
            let active = self.tx_cfg.antenna == n;
            row = row.push(
                Button::new(Text::new(format!("ANT {n}")).size(13))
                    .style(btn_style(if active {
                        BtnKind::Active
                    } else {
                        BtnKind::Plain
                    }))
                    .padding([8, 14])
                    .on_press(Message::Tx(TxMsg::Antenna(n))),
            );
        }
        Column::new()
            .spacing(10)
            .push(
                Text::new("Transmit antenna")
                    .size(12)
                    .color(role_color(ui::ColorRole::RxValue)),
            )
            .push(row)
            .push(
                Text::new(
                    "ANT2/ANT3 require the KAT4 ATU; RX-antenna config is on the RX screens.",
                )
                .size(10)
                .color(dim),
            )
            .into()
    }

    /// TX → KEYER sub-panel (`KS`/`KP`): speed, weight, paddle, iambic mode.
    fn tx_keyer(&self) -> Element<'_, Message> {
        let c = self.tx_cfg;
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        Column::new()
            .spacing(10)
            .push(
                Row::new()
                    .spacing(18)
                    .push(disp_stepper(
                        "SPEED",
                        format!("{} WPM", c.keyer_speed),
                        Message::Tx(TxMsg::KeyerSpeed(c.keyer_speed.saturating_sub(1))),
                        Message::Tx(TxMsg::KeyerSpeed(c.keyer_speed + 1)),
                    ))
                    .push(disp_stepper(
                        "WEIGHT",
                        format!("{:.2}", f64::from(c.weight) / 100.0),
                        Message::Tx(TxMsg::Weight(c.weight.saturating_sub(5))),
                        Message::Tx(TxMsg::Weight(c.weight + 5)),
                    )),
            )
            .push(
                Row::new()
                    .spacing(6)
                    .push(small_btn_string(
                        format!("PADDLE: {}", if c.paddle_reverse { "REV" } else { "NOR" }),
                        Message::Tx(TxMsg::PaddleReverse(!c.paddle_reverse)),
                    ))
                    .push(small_btn_string(
                        format!("IAMBIC: {}", if c.iambic_b { "B" } else { "A" }),
                        Message::Tx(TxMsg::IambicB(!c.iambic_b)),
                    )),
            )
            .push(
                // CW sidetone pitch (FR-KEY-02).
                Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(Text::new("PITCH").size(11).color(dim))
                    .push(
                        slider(250..=950u16, self.cw_pitch, Message::SetCwPitch)
                            .step(10u16)
                            .width(Length::Fixed(140.0)),
                    )
                    .push(
                        Text::new(format!("{} Hz", self.cw_pitch))
                            .size(11)
                            .color(rxv),
                    ),
            )
            .push(
                // Full break-in QSK + VOX/QSK delay for the current mode (FR-TX-DLY-01).
                Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(small_btn_string(
                        format!("QSK: {}", if self.qsk_full { "FULL" } else { "DELAY" }),
                        Message::ToggleQskFull,
                    ))
                    .push(Text::new("DLY").size(11).color(dim))
                    .push(
                        slider(0..=255u8, self.qsk_delay, Message::SetQskDelay)
                            .width(Length::Fixed(120.0)),
                    )
                    .push(
                        Text::new(format!("{} ms", u16::from(self.qsk_delay) * 10))
                            .size(11)
                            .color(rxv),
                    ),
            )
            .into()
    }

    /// Mode class for the `SD` delay command, derived from the main mode (`C`=CW
    /// & direct data, `D`=AF data, `V`=voice) — FR-TX-DLY-01.
    fn tx_mode_class(&self) -> char {
        match self.ui.mode_a {
            Some("CW") | Some("CW-R") => 'C',
            Some("DATA") | Some("DATA-R") | Some("FSK") | Some("FSK-D") => 'D',
            _ => 'V',
        }
    }

    /// TX → MIC sub-panel (`MI`/`MG`/`MS`): input, gain, front preamp, bias.
    fn tx_mic(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let c = self.tx_cfg;
        let input = ui::mic_inputs()[(c.mic_input as usize).min(4)];
        let preamp = ["0 dB", "10 dB", "20 dB"][(c.front_preamp as usize).min(2)];
        Column::new()
            .spacing(10)
            .push(
                Row::new()
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .push(small_btn_string(
                        format!("INPUT: {input}"),
                        Message::Tx(TxMsg::MicInput((c.mic_input + 1) % 5)),
                    ))
                    .push(disp_stepper(
                        "GAIN",
                        c.mic_gain.to_string(),
                        Message::Tx(TxMsg::MicGain(c.mic_gain.saturating_sub(5))),
                        Message::Tx(TxMsg::MicGain(c.mic_gain + 5)),
                    )),
            )
            .push(
                Row::new()
                    .spacing(6)
                    .push(small_btn_string(
                        format!("PREAMP: {preamp}"),
                        Message::Tx(TxMsg::FrontPreamp((c.front_preamp + 1) % 3)),
                    ))
                    .push(small_btn_string(
                        format!("BIAS: {}", if c.front_bias { "On" } else { "Off" }),
                        Message::Tx(TxMsg::FrontBias(!c.front_bias)),
                    ))
                    .push(two_line_btn(
                        ui::toggle_button("VOX", Some(c.vox)),
                        Some(c.vox),
                        Some(Message::Tx(TxMsg::Vox(!c.vox))),
                    )),
            )
            .push(
                // Speech compression (FR-TX-CMP-01), SSB modes.
                Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(Text::new("CMP").size(11).color(dim))
                    .push(
                        slider(0..=30u8, self.compression, Message::SetCompression)
                            .width(Length::Fixed(150.0)),
                    )
                    .push(
                        Text::new(self.compression.to_string())
                            .size(11)
                            .color(role_color(ui::ColorRole::RxValue)),
                    )
                    .push(Text::new("(SSB)").size(10).color(dim)),
            )
            .push(
                Text::new("Front mic shown; rear-mic + button config deferred (§1.6).")
                    .size(10)
                    .color(dim),
            )
            .into()
    }

    /// TX → LINE sub-panel (`LI`): source and level.
    fn tx_line(&self) -> Element<'_, Message> {
        let c = self.tx_cfg;
        Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(small_btn_string(
                format!(
                    "SOURCE: {}",
                    if c.line_use_jack { "LINE JACK" } else { "USB" }
                ),
                Message::Tx(TxMsg::LineUseJack(!c.line_use_jack)),
            ))
            .push(disp_stepper(
                "LEVEL",
                c.line_level.to_string(),
                Message::Tx(TxMsg::LineLevel(c.line_level.saturating_sub(5))),
                Message::Tx(TxMsg::LineLevel(c.line_level + 5)),
            ))
            .into()
    }

    /// Fn screen (FR-VFO-07/FR-SW-01/FR-UI-19, SCR-FN-*): a tab row
    /// (KEYS / SWITCHES / DX) — VFO ops + quick memories + PF keys, a
    /// front-panel switch panel, and the DX prefix list.
    fn fn_screen(&self) -> Element<'_, Message> {
        let tab_btn = |tab: FnTab, label: &'static str| -> Element<Message> {
            let active = self.fn_tab == tab;
            Button::new(Text::new(label).size(12))
                .style(btn_style(if active {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([5, 12])
                .on_press(Message::SetFnTab(tab))
                .into()
        };
        let tabs = Row::new()
            .spacing(6)
            .push(tab_btn(FnTab::Keys, "KEYS"))
            .push(tab_btn(FnTab::Switches, "SWITCHES"))
            .push(tab_btn(FnTab::Dx, "DX LIST"));
        let content: Element<Message> = match self.fn_tab {
            FnTab::Keys => self.fn_keys(),
            FnTab::Switches => self.fn_switches(),
            FnTab::Dx => self.fn_dx(),
        };
        Column::new().spacing(12).push(tabs).push(content).into()
    }

    /// Fn -> KEYS: VFO copy/swap (`AB`) + quick memories + PF keys (`SW`).
    fn fn_keys(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        let mut recall = Row::new().spacing(6);
        let mut store = Row::new().spacing(6);
        for (label, tap, hold) in ui::quick_mem_keys() {
            recall = recall.push(small_btn_string(
                format!("RCL {label}"),
                Message::Switch(*tap),
            ));
            store = store.push(small_btn_string(
                format!("STO {label}"),
                Message::Switch(*hold),
            ));
        }
        let mut pf = Row::new().spacing(6);
        for (label, code) in ui::pf_keys() {
            pf = pf.push(small_btn_string(
                (*label).to_string(),
                Message::Switch(*code),
            ));
        }
        Column::new()
            .spacing(8)
            .push(Text::new("VFO operations").size(12).color(rxv))
            .push(
                Row::new()
                    .spacing(6)
                    .push(small_btn("A > B", Message::VfoOp(0)))
                    .push(small_btn("B > A", Message::VfoOp(1)))
                    .push(small_btn("SWAP", Message::VfoOp(2)))
                    .push(small_btn("A > B (all)", Message::VfoOp(3)))
                    .push(small_btn("SWAP (all)", Message::VfoOp(5))),
            )
            .push(Text::new("Quick memories (M1-M4)").size(12).color(rxv))
            .push(recall)
            .push(store)
            .push(Text::new("Programmable keys").size(12).color(rxv))
            .push(pf)
            .push(
                Text::new("RCL taps the key (recall / play); STO holds it (store).")
                    .size(10)
                    .color(dim),
            )
            .into()
    }

    /// Fn -> SWITCHES: emulate useful front-panel switches (`SW`).
    fn fn_switches(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        let mut row = Row::new().spacing(6);
        for (label, code) in ui::radio_switches() {
            row = row.push(small_btn_string(
                (*label).to_string(),
                Message::Switch(*code),
            ));
        }
        // Remote power (FR-PWR-01): restart + a two-step-guarded power off. The K4
        // cannot be powered ON via CAT, so there is no "on" control.
        let danger = |label: &str, msg: Message| {
            Button::new(Text::new(label.to_string()).size(12))
                .style(btn_style(BtnKind::Danger))
                .padding([5, 10])
                .on_press(msg)
        };
        let power_row = if self.power_off_armed {
            Row::new()
                .spacing(6)
                .align_y(Alignment::Center)
                .push(small_btn("RESTART", Message::PowerRestart))
                .push(small_btn("CANCEL", Message::PowerOffCancel))
                .push(danger("CONFIRM POWER OFF", Message::PowerOffConfirm))
                .push(
                    Text::new("radio will power down")
                        .size(10)
                        .color(role_color(ui::ColorRole::Caution)),
                )
        } else {
            Row::new()
                .spacing(6)
                .push(small_btn("RESTART", Message::PowerRestart))
                .push(danger("POWER OFF", Message::PowerOffArm))
        };
        Column::new()
            .spacing(10)
            .push(Text::new("Front-panel switches").size(12).color(rxv))
            .push(row)
            .push(
                Text::new("Emulates a switch tap (SPOT/TUNE/ATU/DIV/LOCK/MON).")
                    .size(10)
                    .color(dim),
            )
            .push(Text::new("Radio power").size(12).color(rxv))
            .push(power_row)
            .push(
                Text::new("Restart = PS8; Power off = PS0. The K4 cannot be turned on via CAT.")
                    .size(10)
                    .color(dim),
            )
            .into()
    }

    /// Fn -> DX LIST: a searchable DXCC prefix/country reference (client-side).
    fn fn_dx(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let all = ui::dx_prefixes();
        let matches = ui::dx_search(&self.dx_filter);
        let mut list = Column::new().spacing(2);
        for i in &matches {
            let (pfx, country) = all[*i];
            list = list.push(
                Row::new()
                    .spacing(10)
                    .push(
                        Text::new(pfx)
                            .size(12)
                            .width(Length::Fixed(90.0))
                            .color(role_color(ui::ColorRole::VfoA)),
                    )
                    .push(Text::new(country).size(12)),
            );
        }
        Column::new()
            .spacing(8)
            .push(
                Row::new()
                    .spacing(10)
                    .align_y(Alignment::Center)
                    .push(
                        TextInput::new("search prefix / country...", &self.dx_filter)
                            .on_input(Message::DxFilter)
                            .size(13)
                            .width(Length::Fixed(240.0)),
                    )
                    .push(
                        Text::new(format!("{} / {}", matches.len(), all.len()))
                            .size(11)
                            .color(dim),
                    ),
            )
            .push(scrollable(list).height(Length::Fixed(200.0)))
            .push(
                Text::new("DXCC prefix reference (starter set; expandable).")
                    .size(10)
                    .color(dim),
            )
            .into()
    }

    /// Copy the radio's read-back values (from the snapshot's `RadioState`) into
    /// the local config-screen state, so EQ / DISPLAY / TX / RX show the radio's
    /// current settings on connect. Idempotent; runs until the connect GET burst
    /// has clearly landed, then latches `seeded`.
    ///
    /// trace: FR-UI-20
    fn seed_from_radio(&mut self) {
        let r = self.ui.radio.clone();
        if let Some(v) = r.bandwidth_hz {
            self.bw_hz = v;
        }
        if let Some(v) = r.af_gain {
            self.af_gain = v;
        }
        if let Some(v) = r.rf_gain_db {
            self.rf_gain = v;
        }
        if let Some(v) = r.squelch {
            self.squelch = v;
        }
        if let Some(v) = r.tx_power {
            self.tx_power = v;
        }
        if let Some(v) = r.compression {
            self.compression = v;
        }
        if let Some(v) = r.cw_pitch {
            self.cw_pitch = v;
        }
        if let Some(v) = r.qsk_full {
            self.qsk_full = v;
        }
        if let Some(v) = r.qsk_delay {
            self.qsk_delay = v;
        }
        if let Some(v) = r.shift_hz {
            self.shift_hz = v;
        }
        if let Some(v) = r.rx_eq {
            self.rx_eq = v;
        }
        if let Some(v) = r.tx_eq {
            self.tx_eq = v;
        }
        if let Some(v) = r.pan_ref {
            self.display.ref_db = v;
        }
        if let Some(v) = r.pan_span_hz {
            self.display.span_hz = v;
        }
        if let Some(v) = r.pan_scale {
            self.display.scale = v.min(u8::MAX as u16) as u8;
        }
        if let Some(v) = r.wf_palette {
            self.display.wf_palette = v;
        }
        if let Some(v) = r.wf_height {
            self.display.wf_height = v;
        }
        if let Some(v) = r.keyer_speed {
            self.tx_cfg.keyer_speed = v;
        }
        if let Some(v) = r.keyer_weight {
            self.tx_cfg.weight = v;
        }
        if let Some(v) = r.keyer_paddle_rev {
            self.tx_cfg.paddle_reverse = v;
        }
        if let Some(v) = r.keyer_iambic_b {
            self.tx_cfg.iambic_b = v;
        }
        if let Some(v) = r.mic_input {
            self.tx_cfg.mic_input = v;
        }
        if let Some(v) = r.mic_gain {
            self.tx_cfg.mic_gain = v;
        }
        if let Some(v) = r.tx_antenna {
            self.tx_cfg.antenna = v;
        }
        if let Some(v) = r.vox_voice {
            self.tx_cfg.vox = v;
        }
        if let Some(v) = r.rx_antenna {
            self.rx_cfg.ant = v;
        }
        if let Some(v) = r.rx_antenna_sub {
            self.rx_cfg.ant_sub = v;
        }
        if let Some(v) = r.line_out_left {
            self.rx_cfg.lo_left = v.min(u8::MAX as u16) as u8;
        }
        if let Some(v) = r.line_out_right {
            self.rx_cfg.lo_right = v.min(u8::MAX as u16) as u8;
        }
        if let Some(v) = r.line_out_gang {
            self.rx_cfg.lo_gang = v;
        }
        // Latch once a config value has arrived (the burst is landing), so later
        // user edits are not overwritten by re-seeding.
        if r.rx_eq.is_some() || r.band.is_some() || r.keyer_speed.is_some() {
            self.seeded = true;
        }
    }

    /// MENU screen (FR-MENU-01, SCR-MENU-*): the full K4 configuration-menu list
    /// (from D12), searchable; tapping an item opens it on the radio (`MO`).
    /// In-app value edit/lock/NORM (`MEDF`/`ME` read-back) is a follow-up.
    fn menu_config_screen(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        let items = ui::menu_items();
        let matches = ui::menu_search(&self.menu_filter);

        let mut list = Column::new().spacing(3);
        for i in &matches {
            let (id, name) = items[*i];
            list = list.push(
                Button::new(
                    Row::new()
                        .spacing(10)
                        .align_y(Alignment::Center)
                        .push(Text::new(name).size(12).width(Length::Fixed(320.0)))
                        .push(Text::new(format!("#{id:04}")).size(11).color(dim)),
                )
                .style(btn_style(BtnKind::Plain))
                .width(Length::Fill)
                .padding([4, 8])
                .on_press(Message::MenuOpen(id)),
            );
        }

        let header = Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(Text::new("MENU").size(12).color(rxv))
            .push(
                TextInput::new("search menu…", &self.menu_filter)
                    .on_input(Message::MenuFilter)
                    .size(13)
                    .width(Length::Fixed(240.0)),
            )
            .push(
                Text::new(format!("{} / {} items", matches.len(), items.len()))
                    .size(11)
                    .color(dim),
            );
        Column::new()
            .spacing(10)
            .push(header)
            .push(scrollable(list).height(Length::Fixed(200.0)))
            .push(
                Text::new(
                    "Tap an item to open it on the radio (MO). In-app value \
                     edit / lock / NORM needs MEDF/ME read-back — follow-up.",
                )
                .size(10)
                .color(dim),
            )
            .into()
    }

    /// DISPLAY panadapter-setup screen (FR-PAN-CTL-01, SCR-DSP-*): pan mode
    /// (also our `ViewMode`), reference, span, scale, averaging, waterfall
    /// height/palette, and peak/freeze — each pushing its `#` command live.
    fn display_screen(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let d = self.display;
        let mut modes = Row::new().spacing(6);
        for m in [ViewMode::SingleA, ViewMode::SingleB, ViewMode::Dual] {
            let active = self.view_mode == m;
            modes = modes.push(
                Button::new(Text::new(m.label()).size(12))
                    .style(btn_style(if active {
                        BtnKind::Active
                    } else {
                        BtnKind::Plain
                    }))
                    .padding([6, 12])
                    .on_press(Message::Disp(DispMsg::Mode(m))),
            );
        }
        let view_row = Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(Text::new("PAN").size(11).color(dim))
            .push(modes);
        let pal = ui::waterfall_palettes()[(d.wf_palette as usize).min(4)];
        let peak = two_line_btn(
            ui::toggle_button("PEAK", Some(d.peak)),
            Some(d.peak),
            Some(Message::Disp(DispMsg::Peak(!d.peak))),
        );
        let freeze = two_line_btn(
            ui::toggle_button("FREEZE", Some(d.freeze)),
            Some(d.freeze),
            Some(Message::Disp(DispMsg::Freeze(!d.freeze))),
        );
        // Steppers laid out on a fixed 3-column grid so labels, −/+ buttons and
        // values align across rows and columns.
        let grid_row = || Row::new().spacing(12);
        Column::new()
            .spacing(10)
            .push(view_row)
            .push(
                grid_row()
                    .push(disp_stepper(
                        "REF",
                        format!("{} dBm", d.ref_db),
                        Message::Disp(DispMsg::Ref(d.ref_db - 5)),
                        Message::Disp(DispMsg::Ref(d.ref_db + 5)),
                    ))
                    .push(disp_stepper(
                        "SPAN",
                        format!("{:.0} kHz", f64::from(d.span_hz) / 1000.0),
                        Message::Disp(DispMsg::Span(d.span_hz / 2)),
                        Message::Disp(DispMsg::Span(d.span_hz.saturating_mul(2))),
                    ))
                    .push(disp_stepper(
                        "SCALE",
                        d.scale.to_string(),
                        Message::Disp(DispMsg::Scale(d.scale.saturating_sub(5))),
                        Message::Disp(DispMsg::Scale(d.scale + 5)),
                    )),
            )
            .push(
                grid_row()
                    .push(disp_stepper(
                        "AVG",
                        d.avg.to_string(),
                        Message::Disp(DispMsg::Avg(d.avg.saturating_sub(1))),
                        Message::Disp(DispMsg::Avg(d.avg + 1)),
                    ))
                    .push(disp_stepper(
                        "WF HT",
                        format!("{}%", d.wf_height),
                        Message::Disp(DispMsg::Height(d.wf_height.saturating_sub(10))),
                        Message::Disp(DispMsg::Height(d.wf_height + 10)),
                    )),
            )
            .push(
                Row::new()
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .push(peak)
                    .push(freeze)
                    .push(small_btn_string(
                        format!("WF: {pal}"),
                        Message::Disp(DispMsg::Palette((d.wf_palette + 1) % 5)),
                    )),
            )
            .push(
                Text::new("Center / cursors / monitor-target: pending (k4-screens §1.3).")
                    .size(10)
                    .color(dim),
            )
            .into()
    }

    /// BAND screen (FR-VFO-04, SCR-BAND-*): direct band grid (`BN`), band-stack
    /// recall (`BN^`), and band up/down. GEN/memories/transverter deferred.
    fn band_screen(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let mut grid = Row::new().spacing(6);
        for (label, bn) in ui::band_buttons() {
            grid = grid.push(
                Button::new(Text::new(*label).size(13))
                    .style(btn_style(BtnKind::Plain))
                    .padding([8, 12])
                    .on_press(Message::SelectBand(*bn)),
            );
        }
        let ops = Row::new()
            .spacing(6)
            .align_y(Alignment::Center)
            .push(small_btn("BAND −", Message::Band(false)))
            .push(small_btn("BAND +", Message::Band(true)))
            .push(small_btn("BAND STACK", Message::BandStack));
        // Transverter bands XVTR1–12 (`XV`).
        let mut xvtr = Row::new().spacing(4);
        for n in 1u8..=12 {
            xvtr = xvtr.push(
                Button::new(Text::new(format!("XV{n}")).size(12))
                    .style(btn_style(BtnKind::Plain))
                    .padding([6, 8])
                    .on_press(Message::SelectXvtr(n)),
            );
        }
        Column::new()
            .spacing(10)
            .push(
                Text::new("Select a band (direct BN select)")
                    .size(12)
                    .color(dim),
            )
            .push(grid)
            .push(ops)
            .push(Text::new("Transverter bands (XV)").size(12).color(dim))
            .push(xvtr)
            .push(
                Text::new("GEN / memories on the Fn screen; XVTR band setup via MENU.")
                    .size(10)
                    .color(dim),
            )
            .into()
    }

    /// A menu screen shown *in place of the spectrum frame* when a primary
    /// softkey is active (FR-UI-19). The rest of the UI (VFO band, mode/filter
    /// controls, softkey row, and the lower panels) stays untouched. Content is
    /// the K4's additional configuration screens (per the manual), not a copy of
    /// controls already present elsewhere in the UI. Filled in per screen.
    fn menu_screen(&self, p: ui::Primary) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let heading = Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(
                Text::new(p.label())
                    .size(15)
                    .color(role_color(ui::ColorRole::VfoB)),
            )
            .push(
                Text::new("screen — tap the lit softkey to return to the spectrum")
                    .size(11)
                    .color(dim),
            );
        // Built screens replace the synopsis (FR-EQ-01): MAIN/SUB RX → RX EQ,
        // TX → TX EQ. The rest still show their one-line synopsis (Phase B/C).
        let body: Element<Message> = match ui::screen_kind(p) {
            ui::ScreenKind::RxEq => self.rx_screen(p == ui::Primary::SubRx),
            ui::ScreenKind::TxConfig => self.tx_screen(),
            ui::ScreenKind::Display => self.display_screen(),
            ui::ScreenKind::Band => self.band_screen(),
            ui::ScreenKind::Fn => self.fn_screen(),
            ui::ScreenKind::Menu => self.menu_config_screen(),
        };
        Container::new(Column::new().spacing(12).push(heading).push(body))
            .style(panel_style)
            .padding(14)
            .width(Length::Fill)
            .height(Length::Fixed(SCREEN_H))
            .into()
    }

    /// The K4 8-band graphic-equalizer screen (FR-EQ-01, SCR-EQ-*): a vertical
    /// slider + dB readout + `−/+` steppers per band (100–3200 Hz), a FLAT reset,
    /// and an optional caveat note. Adjusting a band sends `RE`/`TE` live.
    fn eq_screen(
        &self,
        target: EqTarget,
        title: &'static str,
        note: Option<&'static str>,
    ) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let values = match target {
            EqTarget::Rx => self.rx_eq,
            EqTarget::Tx => self.tx_eq,
        };
        let range = i32::from(ui::EQ_DB_RANGE);
        let mut bands = Row::new().spacing(10).align_y(Alignment::Center);
        for (i, label) in ui::eq_bands().iter().enumerate() {
            let v = values[i];
            let col = Column::new()
                .spacing(4)
                .align_x(Alignment::Center)
                .push(
                    Text::new(format!("{v:+}"))
                        .size(11)
                        .color(role_color(ui::ColorRole::RxValue)),
                )
                .push(small_btn("+", Message::EqChanged(target, i, v + 1)))
                .push(
                    vertical_slider(-range..=range, i32::from(v), move |nv| {
                        Message::EqChanged(target, i, nv as i8)
                    })
                    .height(Length::Fixed(95.0))
                    .step(1),
                )
                .push(small_btn("−", Message::EqChanged(target, i, v - 1)))
                .push(Text::new(*label).size(10).color(dim));
            bands = bands.push(col);
        }
        let controls = Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(
                Text::new(title)
                    .size(12)
                    .color(role_color(ui::ColorRole::RxValue)),
            )
            .push(small_btn("FLAT", Message::EqFlat(target)))
            .push(Text::new("Hz · ±16 dB").size(11).color(dim));
        let mut col = Column::new().spacing(10).push(controls).push(bands);
        if let Some(n) = note {
            col = col.push(Text::new(n).size(10).color(dim));
        }
        col.into()
    }

    fn view(&self) -> Element<'_, Message> {
        // Resolve colours against the active theme for the whole tree (FR-UI-17).
        set_active_theme(self.effective_theme());
        let dim = role_color(ui::ColorRole::Inactive);

        // Header band: title, link state, status line, the A / B / A+B view
        // selector (FR-UI-08, the reference client's segmented control), and a
        // quick connect/disconnect.
        let seg = |mode: ViewMode| -> Element<'_, Message> {
            let kind = if self.view_mode == mode {
                BtnKind::Active
            } else {
                BtnKind::Plain
            };
            Button::new(Text::new(mode.label()).size(12))
                .style(btn_style(kind))
                .padding([4, 10])
                .on_press(Message::SetViewMode(mode))
                .into()
        };
        let seg_row = Row::new()
            .spacing(4)
            .push(seg(ViewMode::SingleA))
            .push(seg(ViewMode::SingleB))
            .push(seg(ViewMode::Dual));
        // Connect / Cancel / Disconnect from the connection phase (FR-UI-16):
        // while an attempt is in flight the control shows "Cancel" and aborts it.
        let (conn_label, conn_action) = ui::connect_button(self.ui.phase);
        let conn_btn = Button::new(Text::new(conn_label).size(12))
            .style(btn_style(connect_kind(conn_action)))
            .padding([5, 10])
            .on_press(connect_msg(conn_action));
        // Theme selector (FR-UI-17) and About (FR-UI-18), top-right; About is
        // rightmost with the theme toggle to its left.
        let theme_btn =
            Button::new(Text::new(format!("Theme: {}", self.theme_mode.label())).size(12))
                .style(btn_style(BtnKind::Plain))
                .padding([5, 10])
                .on_press(Message::CycleTheme);
        let about_btn = Button::new(Text::new("About").size(12))
            .style(btn_style(if self.about_open {
                BtnKind::Active
            } else {
                BtnKind::Plain
            }))
            .padding([5, 10])
            .on_press(Message::ToggleAbout);
        // Settings dialog (FR-UI-23) — houses the connection form + peer cache.
        let settings_btn = Button::new(Text::new("Settings").size(12))
            .style(btn_style(if self.settings_open {
                BtnKind::Active
            } else {
                BtnKind::Plain
            }))
            .padding([5, 10])
            .on_press(Message::ToggleSettings);
        // Phase-aware connection indicator: a coloured dot + label (green =
        // connected, amber = connecting, grey = disconnected) — FR-UI-22.
        let (status_text, status_role) = ui::conn_status(self.ui.phase);
        let status_dot = Container::new(Text::new(" ").size(1))
            .width(Length::Fixed(9.0))
            .height(Length::Fixed(9.0))
            .style(move |_t: &Theme| container::Style {
                background: Some(Background::Color(role_color(status_role))),
                border: Border {
                    radius: 4.5.into(),
                    ..Default::default()
                },
                ..container::Style::default()
            });
        let status_ind = Row::new()
            .spacing(6)
            .align_y(Alignment::Center)
            .push(status_dot)
            .push(
                Text::new(status_text)
                    .size(12)
                    .color(role_color(status_role)),
            );
        let header = Row::new()
            .spacing(12)
            .align_y(Alignment::Center)
            .push(Text::new("K4 REMOTE").size(20))
            .push(status_ind)
            .push(
                Text::new(self.ui.status.clone())
                    .size(12)
                    .color(dim)
                    .width(Length::Fill),
            )
            .push(seg_row)
            .push(conn_btn)
            .push(settings_btn)
            .push(theme_btn)
            .push(about_btn);

        // VFO header band (FR-UI-12): A/B-symmetric panels with the shared
        // TX/SPLIT/RIT box between them, arranged per the responsive layout.
        let bl = ui::band_layout(self.window_w, self.view_mode);
        let vfo_band: Element<Message> = if bl.split_center {
            // dual, wide: A | centre | B
            Row::new()
                .spacing(10)
                .push(self.vfo_panel(bl.panes[0]))
                .push(self.center_box())
                .push(self.vfo_panel(bl.panes[1]))
                .into()
        } else if bl.stacked {
            let mut col = Column::new().spacing(10).push(self.center_box());
            for &p in &bl.panes {
                col = col.push(self.vfo_panel(p));
            }
            col.into()
        } else {
            // single, wide: pane + centre box
            Row::new()
                .spacing(10)
                .push(self.vfo_panel(bl.panes[0]))
                .push(self.center_box())
                .into()
        };

        // Main-RX controls strip: two-line state buttons (FR-UI-11) in the
        // reference client's grid style, plus mode / band / direct tuning.
        // Labelled MAIN RX because these commands act on the main receiver.
        let chips = Row::new()
            .spacing(6)
            .push(two_line_btn(
                ui::bandwidth_button(Some(self.bw_hz)),
                None,
                Some(Message::CycleBandwidth),
            ))
            .push(two_line_btn(
                ui::atten_button(self.ui.atten_on, self.ui.atten_db),
                self.ui.atten_on,
                Some(Message::ToggleAtten),
            ))
            .push(two_line_btn(
                ui::toggle_button("PRE", self.ui.preamp_on),
                self.ui.preamp_on,
                Some(Message::TogglePreamp),
            ))
            .push(two_line_btn(
                ui::toggle_button("NB", self.ui.nb_on),
                self.ui.nb_on,
                Some(Message::ToggleNb),
            ))
            .push(two_line_btn(
                ui::toggle_button("NR", self.ui.nr_on),
                self.ui.nr_on,
                Some(Message::ToggleNr),
            ))
            .push(two_line_btn(
                ui::agc_button(self.ui.agc_mode),
                None,
                Some(Message::CycleAgc),
            ));
        let mode_btn = |label: &'static str, digit: u8| -> Element<'_, Message> {
            let active = self.ui.mode_a == Some(label);
            Button::new(Text::new(label).size(12))
                .style(btn_style(if active {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([6, 10])
                .on_press(Message::SetMode(digit))
                .into()
        };
        let tune_row = Row::new()
            .spacing(6)
            .align_y(Alignment::Center)
            .push(mode_btn("LSB", 1))
            .push(mode_btn("USB", 2))
            .push(mode_btn("CW", 3))
            .push(mode_btn("DATA", 6))
            .push(small_btn("BAND −", Message::Band(false)))
            .push(small_btn("BAND +", Message::Band(true)))
            .push(horizontal_space())
            .push(Text::new("VFO A MHz").size(12).color(dim))
            .push(
                TextInput::new("14.074", &self.freq_mhz)
                    .on_input(Message::FreqChanged)
                    .on_submit(Message::SetFreq)
                    .size(13)
                    .width(Length::Fixed(110.0)),
            )
            .push(small_btn("SET", Message::SetFreq));
        // AF/RF gain + squelch sliders for the main receiver (FR-RX-01,
        // FR-RX-SQL-01) — the K4's RF/SQL knob, plus radio-side AF.
        let rxv = role_color(ui::ColorRole::RxValue);
        let gain = |label: &'static str,
                    val: u8,
                    max: u8,
                    msg: fn(u8) -> Message,
                    unit: &'static str|
         -> Element<Message> {
            Row::new()
                .spacing(6)
                .align_y(Alignment::Center)
                .push(Text::new(label).size(11).color(dim))
                .push(slider(0..=max, val, msg).width(Length::Fixed(110.0)))
                .push(Text::new(format!("{val}{unit}")).size(11).color(rxv))
                .into()
        };
        let gain_row = Row::new()
            .spacing(18)
            .align_y(Alignment::Center)
            .push(gain("AF", self.af_gain, 60, Message::SetAfGain, ""))
            .push(gain("RF", self.rf_gain, 60, Message::SetRfGain, " dB"))
            .push(gain("SQL", self.squelch, 40, Message::SetSquelch, ""));
        let controls = Container::new(
            Column::new()
                .spacing(8)
                .push(
                    Row::new()
                        .spacing(10)
                        .align_y(Alignment::Center)
                        .push(Text::new("MAIN RX").size(11).color(dim))
                        .push(chips),
                )
                .push(tune_row)
                .push(gain_row),
        )
        .style(panel_style)
        .padding(12)
        .width(Length::Fill);

        // Spectrum band (FR-PAN-02/03, FR-UI-05/12): one badged pane per VFO in
        // the active layout. The snapshot currently carries a single trace, so
        // both panes show it for now. Every pane keeps the same height whether
        // one or two are shown (dual A+B panes match single-A/B height).
        let mut spectrum_panes: Vec<Element<Message>> = Vec::new();
        let dual = bl.panes.len() > 1;
        for &p in &bl.panes {
            let role = if p.is_b() {
                ui::ColorRole::VfoB
            } else {
                ui::ColorRole::VfoA
            };
            let plot: Element<Message> = if self.ui.spectrum_latest.is_empty() {
                Container::new(
                    Text::new("spectrum + waterfall — waiting for data")
                        .size(12)
                        .color(dim),
                )
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(8)
                .into()
            } else {
                Canvas::new(spectrum::Spectrum {
                    latest: &self.ui.spectrum_latest,
                    waterfall: &self.ui.waterfall,
                    top_dbm: -30.0,
                    range_db: 100.0,
                })
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            };
            // Only in dual (A+B) view does the TX-VFO choice matter: the pane
            // matching the transmit VFO (B under split, else A) gets an accent
            // frame + TX tag, and clicking a pane makes it the TX VFO via FT
            // (split). In single-VFO view there's nothing to choose, so no
            // highlight and no click. FR-UI-12.
            let selected = dual && p.is_b() == self.tx_vfo_b;
            let mut header = Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(badge(p.label(), role));
            if selected {
                header = header.push(
                    Text::new("TX")
                        .size(11)
                        .color(role_color(ui::ColorRole::TxActive)),
                );
            }
            let pane = Container::new(Column::new().spacing(6).push(header).push(plot))
                .style(pane_style(selected))
                .padding(8)
                .width(Length::Fill)
                // Match the menu-screen slot exactly so the frame doesn't resize
                // when swapping the spectrum for a config screen (FR-UI-19).
                .height(Length::Fixed(SCREEN_H));
            spectrum_panes.push(if dual {
                mouse_area(pane)
                    .on_press(Message::SelectTxVfo(p.is_b()))
                    .into()
            } else {
                pane.into()
            });
        }
        let spectrum_band: Element<Message> = if bl.stacked {
            let mut col = Column::new().spacing(10);
            for e in spectrum_panes {
                col = col.push(e);
            }
            col.into()
        } else {
            let mut row = Row::new().spacing(10);
            for e in spectrum_panes {
                row = row.push(e);
            }
            row.into()
        };

        // The spectrum frame's slot shows a menu screen when a primary softkey
        // is active, and the spectrum otherwise (FR-UI-19). Only this slot
        // changes — the controls box above and the panels below are untouched.
        let panadapter_slot: Element<Message> = match self.context.active() {
            Some(active) => self.menu_screen(active),
            None => spectrum_band,
        };

        // Primary softkey row (FR-UI-13), pinned like the K4's on-screen button
        // band. The active primary's screen shows in the spectrum slot above
        // (FR-UI-19) — the old stub context sub-row is retired.
        let mut primaries = Row::new().spacing(6);
        for p in ui::Primary::all() {
            let kind = if self.context.is_open(p) {
                BtnKind::Active
            } else {
                BtnKind::Plain
            };
            primaries = primaries.push(
                Button::new(
                    Text::new(p.label())
                        .size(13)
                        .width(Length::Fill)
                        .align_x(Alignment::Center),
                )
                .style(btn_style(kind))
                .width(Length::Fill)
                .padding([8, 4])
                .on_press(Message::TapPrimary(p)),
            );
        }

        // TX safety affordances (FR-UI-04/06, FR-TX-SAFE-*): red-edged arm/PTT
        // that fill amber while engaged; a red emergency stop.
        let arm = Button::new(
            Text::new(if self.ui.tx_armed {
                "TX ARMED — DISARM"
            } else {
                "ARM TX"
            })
            .size(13),
        )
        .style(btn_style(if self.ui.tx_armed {
            BtnKind::Amber
        } else {
            BtnKind::Ptt
        }))
        .padding([6, 10])
        .on_press(Message::ToggleArm);
        let key =
            Button::new(Text::new(if self.ui.transmitting { "UNKEY" } else { "PTT" }).size(13))
                .style(btn_style(if self.ui.transmitting {
                    BtnKind::Amber
                } else {
                    BtnKind::Ptt
                }))
                .padding([6, 10])
                .on_press(Message::ToggleKey);
        let estop = Button::new(Text::new("EMERGENCY STOP").size(13))
            .style(btn_style(BtnKind::Danger))
            .padding([6, 10])
            .on_press(Message::EmergencyStop);
        // Transmit power slider (FR-TX-02) inline with PTT/e-stop — no extra
        // height. Seeded from the radio, QRO watts.
        let ptt_row = Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(key)
            .push(estop)
            .push(horizontal_space())
            .push(Text::new("PWR").size(11).color(dim))
            .push(
                slider(0..=110u16, self.tx_power, Message::SetTxPower).width(Length::Fixed(130.0)),
            )
            .push(
                Text::new(format!("{} W", self.tx_power))
                    .size(11)
                    .color(role_color(ui::ColorRole::RxValue)),
            );
        let tx_panel = Container::new(
            Column::new()
                .spacing(8)
                .push(Text::new("TRANSMIT").size(11).color(dim))
                .push(arm)
                .push(ptt_row)
                .push(self.tx_switch_grid()),
        )
        .style(panel_style)
        .padding(12)
        .width(Length::Fill)
        .height(Length::Fixed(BOTTOM_PANEL_H));

        // Connection panel (FR-UI-01) — Ethernet or serial fields.
        let fields: Column<Message> = if self.serial_mode {
            Column::new()
                .spacing(6)
                .push(labeled(
                    "Serial port",
                    &self.serial_path,
                    Message::SerialPathChanged,
                ))
                .push(labeled(
                    "Baud",
                    &self.serial_baud,
                    Message::SerialBaudChanged,
                ))
        } else {
            Column::new()
                .spacing(6)
                .push(labeled("Host", &self.host, Message::HostChanged))
                .push(labeled("Port", &self.port, Message::PortChanged))
                .push(secret("Password", &self.password, Message::PasswordChanged))
        };
        // Options and actions on separate rows so the buttons never get
        // squeezed in the third-width panel (glyph-wrapped labels).
        let mut conn_options = Row::new().spacing(6).push(small_btn(
            if self.serial_mode {
                "ETHERNET"
            } else {
                "SERIAL"
            },
            Message::ToggleSerialMode,
        ));
        if !self.serial_mode {
            conn_options = conn_options
                .push(
                    Button::new(Text::new("TLS").size(12))
                        .style(btn_style(if self.use_tls {
                            BtnKind::Active
                        } else {
                            BtnKind::Plain
                        }))
                        .padding([4, 10])
                        .on_press(Message::ToggleTls),
                )
                .push(
                    Button::new(Text::new("REMEMBER").size(12))
                        .style(btn_style(if self.remember {
                            BtnKind::Active
                        } else {
                            BtnKind::Plain
                        }))
                        .padding([4, 10])
                        .on_press(Message::ToggleRemember),
                );
        }
        // Both buttons stay in place (FR-UI-16). The CONNECT button switches to
        // CANCEL while an attempt is in flight, and cancels it; DISCONNECT stays
        // put and also aborts a pending attempt (worker Disconnect = cancel).
        let connecting = self.ui.phase == ui::ConnPhase::Connecting;
        let (connect_label, connect_press, connect_btn_kind) = if connecting {
            ("CANCEL", Message::Disconnect, BtnKind::Plain)
        } else {
            ("CONNECT", Message::Connect, BtnKind::Active)
        };
        let conn_actions = Row::new()
            .spacing(6)
            .push(
                Button::new(Text::new(connect_label).size(12))
                    .style(btn_style(connect_btn_kind))
                    .padding([5, 10])
                    .on_press(connect_press),
            )
            .push(small_btn("DISCONNECT", Message::Disconnect));
        // The connection form now lives in the Settings dialog (FR-UI-23), along
        // with the peer cache and master-password controls.
        let settings_inner = Column::new()
            .spacing(10)
            .push(Text::new("Settings").size(18))
            .push(Text::new("Connection").size(12).color(dim))
            .push(fields)
            .push(conn_options)
            .push(conn_actions)
            .push(Text::new("Saved peers").size(12).color(dim))
            .push(self.peer_list_view())
            .push(Text::new("Peer-password storage").size(12).color(dim))
            .push(self.master_section_view())
            .push(Text::new("Audio").size(12).color(dim))
            .push(self.audio_section_view())
            .push(
                Container::new(
                    Row::new()
                        .push(horizontal_space())
                        .push(small_btn("Close", Message::ToggleSettings)),
                )
                .width(Length::Fill)
                .padding([10, 0]),
            );
        let settings_card: Element<Message> = modal_scrim(
            Container::new(scrollable(settings_inner))
                .style(panel_style)
                .padding(18)
                .width(Length::Fixed(500.0))
                .max_height(720.0)
                .into(),
        );

        // Diagnostics console (FR-DIAG-01/02). Log oldest→newest so auto-scroll
        // follows the bottom.
        let mut log_col = Column::new().spacing(1);
        for line in &self.ui.diag_lines {
            log_col = log_col.push(Text::new(line.clone()).size(11).color(dim));
        }
        let opt = |label: &'static str, on: bool, msg: Message| {
            Button::new(Text::new(label).size(11))
                .style(btn_style(if on { BtnKind::Active } else { BtnKind::Plain }))
                .padding([3, 8])
                .on_press(msg)
        };
        let diag_header = Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(Text::new("DIAGNOSTICS").size(11).color(dim))
            .push(horizontal_space())
            .push(opt("LOG", self.show_log, Message::ToggleShowLog))
            .push(opt(
                "AUTOSCROLL",
                self.log_autoscroll,
                Message::ToggleLogAutoscroll,
            ));
        let mut diag_col = Column::new()
            .spacing(8)
            .push(diag_header)
            .push(
                Row::new()
                    .spacing(8)
                    .push(
                        TextInput::new("raw CAT, e.g. IF;", &self.cat_input)
                            .on_input(Message::CatInputChanged)
                            .on_submit(Message::SendCat)
                            .size(13)
                            .width(Length::Fixed(200.0)),
                    )
                    .push(small_btn("SEND", Message::SendCat)),
            )
            .push(
                Text::new(format!(
                    "RX audio frames: {}   spectrum: {} bins",
                    self.ui.audio_frames, self.ui.spectrum_bins
                ))
                .size(11)
                .color(dim),
            );
        if self.show_log {
            // Fixed always-present scrollbar on the right (no jumping). Auto-scroll
            // is driven from the tick (snap to newest); with it off the view holds
            // its position so you can read back through the log.
            let log = scrollable(log_col)
                .id(self.log_id.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .direction(scrollable::Direction::Vertical(
                    scrollable::Scrollbar::new().width(6.0).scroller_width(6.0),
                ));
            diag_col = diag_col.push(log);
        }
        let diagnostics = Container::new(diag_col)
            .style(panel_style)
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fixed(BOTTOM_PANEL_H));

        let bottom: Element<Message> = if bl.stacked {
            Column::new()
                .spacing(10)
                .push(tx_panel)
                .push(diagnostics)
                .into()
        } else {
            Row::new()
                .spacing(10)
                .push(tx_panel)
                .push(diagnostics)
                .into()
        };

        // K4-faithful band order (FR-UI-12/13/19): header, VFO band, main-RX
        // controls, then the spectrum slot (spectrum or the active primary's
        // screen), the primary softkey row, and the panels at the bottom.
        let body = Column::new()
            .spacing(10)
            .padding(14)
            .push(header)
            .push(vfo_band)
            .push(controls)
            .push(panadapter_slot)
            .push(primaries)
            .push(bottom);

        let content = Container::new(scrollable(body)).width(Length::Fill);

        // Modal dialogs over a dimming scrim: Settings (FR-UI-23) / About.
        if self.settings_open {
            stack![content, settings_card].into()
        } else if self.about_open {
            stack![content, self.about_overlay()].into()
        } else {
            content.into()
        }
    }

    /// Settings → saved-peers list (FR-CFG-04): each peer with Use / Delete.
    fn peer_list_view(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        if self.peers.peers.is_empty() {
            return Text::new("No saved peers yet — connect to add one.")
                .size(11)
                .color(dim)
                .into();
        }
        let mut list = Column::new().spacing(4);
        for (i, p) in self.peers.peers.iter().enumerate() {
            let store = match &p.secret {
                k4_config::PeerSecret::None => "no pw",
                k4_config::PeerSecret::Keyring => "keychain",
                k4_config::PeerSecret::Encrypted(_) => "encrypted",
            };
            list = list.push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(
                        Text::new(format!("{}:{}", p.host, p.port))
                            .size(12)
                            .color(rxv)
                            .width(Length::Fixed(180.0)),
                    )
                    .push(
                        Text::new(if p.use_tls { "TLS" } else { "tcp" })
                            .size(10)
                            .color(dim)
                            .width(Length::Fixed(32.0)),
                    )
                    .push(
                        Text::new(store)
                            .size(10)
                            .color(dim)
                            .width(Length::Fixed(72.0)),
                    )
                    .push(small_btn("Use", Message::SelectPeer(i)))
                    .push(small_btn("Del", Message::DeletePeer(i))),
            );
        }
        scrollable(list).height(Length::Fixed(150.0)).into()
    }

    /// Settings → peer-password storage (FR-CFG-04): OS keychain vs a
    /// master-password-encrypted config, and set/unlock the master password.
    fn master_section_view(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let mode = Button::new(
            Text::new(if self.use_master {
                "Encrypt in config (master password)"
            } else {
                "Store in OS keychain"
            })
            .size(12),
        )
        .style(btn_style(if self.use_master {
            BtnKind::Active
        } else {
            BtnKind::Plain
        }))
        .padding([4, 10])
        .on_press(Message::UseMasterToggled(!self.use_master));
        let mut col = Column::new().spacing(6).push(mode);
        if self.use_master {
            let set = self.peers.has_master();
            col = col.push(
                Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(
                        TextInput::new(
                            if set {
                                "master password (unlock)"
                            } else {
                                "set a master password"
                            },
                            &self.master_password,
                        )
                        .secure(true)
                        .on_input(Message::MasterPasswordChanged)
                        .on_submit(Message::UnlockMaster)
                        .size(13)
                        .width(Length::Fixed(220.0)),
                    )
                    .push(small_btn(
                        if set { "UNLOCK" } else { "SET" },
                        Message::UnlockMaster,
                    )),
            );
        }
        if !self.peer_status.is_empty() {
            col = col.push(Text::new(self.peer_status.clone()).size(11).color(dim));
        }
        col.into()
    }

    /// Settings → audio: RX/TX device selection (FR-AUD-DEV-01) and local
    /// volume / mic-gain sliders (FR-AUD-LVL-01).
    fn audio_section_view(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let opts = |names: &[String]| -> Vec<String> {
            std::iter::once(DEVICE_DEFAULT.to_string())
                .chain(names.iter().cloned())
                .collect()
        };
        let out_sel = self
            .selected_output
            .clone()
            .unwrap_or_else(|| DEVICE_DEFAULT.to_string());
        let in_sel = self
            .selected_input
            .clone()
            .unwrap_or_else(|| DEVICE_DEFAULT.to_string());
        let label = |t: &'static str| Text::new(t).size(11).color(dim).width(Length::Fixed(64.0));
        Column::new()
            .spacing(8)
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(label("Speaker"))
                    .push(
                        pick_list(
                            opts(&self.audio_outputs),
                            Some(out_sel),
                            Message::SelectOutputDevice,
                        )
                        .text_size(12)
                        .width(Length::Fixed(320.0)),
                    ),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(label("Mic"))
                    .push(
                        pick_list(
                            opts(&self.audio_inputs),
                            Some(in_sel),
                            Message::SelectInputDevice,
                        )
                        .text_size(12)
                        .width(Length::Fixed(320.0)),
                    ),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(label("Volume"))
                    .push(
                        slider(0.0..=2.0, self.volume, Message::VolumeChanged)
                            .step(0.05f32)
                            .on_release(Message::SaveSettings)
                            .width(Length::Fixed(240.0)),
                    )
                    .push(
                        Text::new(format!("{:.0}%", self.volume * 100.0))
                            .size(11)
                            .color(dim),
                    ),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(label("Mic gain"))
                    .push(
                        slider(0.0..=3.0, self.mic_gain, Message::MicGainChanged)
                            .step(0.05f32)
                            .on_release(Message::SaveSettings)
                            .width(Length::Fixed(240.0)),
                    )
                    .push(
                        Text::new(format!("{:.0}%", self.mic_gain * 100.0))
                            .size(11)
                            .color(dim),
                    ),
            )
            .into()
    }

    /// The About overlay (FR-UI-18): author, license, and project URL, each on
    /// its own line, over a dimming scrim; a Close button dismisses it.
    fn about_overlay(&self) -> Element<'_, Message> {
        let accent = role_color(ui::ColorRole::VfoA);
        let dim = role_color(ui::ColorRole::Inactive);
        // A borderless, accent-coloured "link" that opens a URL in the browser.
        let link = move |label: String, url: &'static str| -> Element<Message> {
            Button::new(Text::new(label).size(13).color(accent))
                .style(move |_t: &Theme, status: button::Status| button::Style {
                    background: None,
                    text_color: match status {
                        button::Status::Hovered | button::Status::Pressed => Color::WHITE,
                        _ => accent,
                    },
                    ..button::Style::default()
                })
                .padding(0)
                .on_press(Message::OpenUrl(url))
                .into()
        };
        let card = Column::new()
            .spacing(8)
            .push(Text::new("About K4 Remote").size(18))
            .push(Text::new(ui::ABOUT_AUTHOR).size(13))
            .push(
                Text::new(format!("Version {}", ui::app_version()))
                    .size(12)
                    .color(dim),
            )
            .push(link(ui::ABOUT_LICENSE.to_string(), ui::ABOUT_LICENSE_URL))
            .push(link(ui::ABOUT_URL.to_string(), ui::ABOUT_URL))
            .push(
                Button::new(Text::new("Donate via PayPal").size(13))
                    .style(btn_style(BtnKind::Active))
                    .padding([7, 14])
                    .on_press(Message::OpenUrl(ui::ABOUT_DONATE_URL)),
            )
            .push(Container::new(small_btn("Close", Message::ToggleAbout)).padding([10, 0]));
        let card = Container::new(card).style(panel_style).padding(18);
        // Dimming scrim behind the card.
        Container::new(card)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(Color {
                    a: 0.6,
                    ..Color::BLACK
                })),
                ..container::Style::default()
            })
            .into()
    }
}

/// Pick-list label meaning "use the system default audio device".
const DEVICE_DEFAULT: &str = "(system default)";

/// Map a persisted theme name to the theme mode (FR-CFG-05).
fn theme_from_prefs(s: Option<&str>) -> ui::ThemeMode {
    match s {
        Some("light") => ui::ThemeMode::Light,
        Some("contrast") => ui::ThemeMode::Contrast,
        Some("system") => ui::ThemeMode::System,
        _ => ui::ThemeMode::Dark,
    }
}

/// The persisted name for a theme mode (FR-CFG-05).
fn theme_to_str(m: ui::ThemeMode) -> &'static str {
    match m {
        ui::ThemeMode::Dark => "dark",
        ui::ThemeMode::Light => "light",
        ui::ThemeMode::Contrast => "contrast",
        ui::ThemeMode::System => "system",
    }
}

/// Center `card` over a dimming full-window scrim (shared by the modal dialogs).
fn modal_scrim(card: Element<'_, Message>) -> Element<'_, Message> {
    Container::new(card)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(|_t: &Theme| container::Style {
            background: Some(Background::Color(Color {
                a: 0.6,
                ..Color::BLACK
            })),
            ..container::Style::default()
        })
        .into()
}

/// Open `url` in the OS default browser (About-box links / donate). Fire-and-
/// forget on a detached child so the UI never blocks.
fn open_url(url: &str) {
    #[cfg(target_os = "linux")]
    let program = "xdg-open";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(target_os = "windows")]
    let (program, url) = ("cmd", format!("/C start {url}"));
    let _ = std::process::Command::new(program).arg(url).spawn();
}

/// Best-effort detection of an OS preference for a dark colour scheme, for the
/// `System` theme (FR-UI-17). Dependency-free; queries the desktop where cheap
/// and defaults to dark when unknown.
fn detect_system_dark() -> bool {
    #[cfg(target_os = "linux")]
    {
        if let Ok(out) = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout).to_lowercase();
            if s.contains("dark") {
                return true;
            }
            if s.contains("light") {
                return false;
            }
        }
    }
    true
}

// --- view helpers -----------------------------------------------------------

/// Red for destructive / emergency affordances.
const DANGER: Color = Color::from_rgb(0.898, 0.282, 0.235); // #E5483C

/// Shared height of the VFO header band panels (Fill is not allowed inside the
/// scrollable body, so the panels agree on a fixed height instead).
const VFO_BAND_H: f32 = 160.0;

/// Height of a menu screen shown in place of the spectrum frame (FR-UI-19).
/// Matches the panadapter footprint so the layout doesn't jump.
const SCREEN_H: f32 = 300.0;

/// (raised to fit the TRANSMIT switch grid.)
/// Shared height of the bottom TRANSMIT / DIAGNOSTICS panels so they line up
/// (the scrollable body can't stretch them to match, so fix it). The diagnostics
/// log scrolls within this height.
const BOTTOM_PANEL_H: f32 = 220.0;

/// Visual kind of a styled button (FR-UI-10/15): rest-state control, engaged
/// (blue fill, like the reference client), transmit-critical (red edge),
/// destructive (red fill), or transmitting-now (amber fill).
#[derive(Clone, Copy, PartialEq)]
enum BtnKind {
    Plain,
    Active,
    Ptt,
    Danger,
    Amber,
}

/// Button style closure for a [`BtnKind`] over the layered dark palette.
fn btn_style(kind: BtnKind) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let (mut bg, mut fg, edge) = match kind {
            BtnKind::Plain => (
                shade(ui::Shade::Control),
                role_color(ui::ColorRole::RxValue),
                shade(ui::Shade::Edge),
            ),
            BtnKind::Active => (
                Color::from_rgb8(0x1E, 0x5F, 0xB8),
                Color::WHITE,
                Color::from_rgb8(0x2F, 0x77, 0xD0),
            ),
            BtnKind::Ptt => (
                shade(ui::Shade::Control),
                role_color(ui::ColorRole::RxValue),
                DANGER,
            ),
            BtnKind::Danger => (Color::from_rgb8(0x8E, 0x1F, 0x17), Color::WHITE, DANGER),
            BtnKind::Amber => (
                role_color(ui::ColorRole::TxActive),
                Color::BLACK,
                role_color(ui::ColorRole::TxActive),
            ),
        };
        match status {
            button::Status::Hovered | button::Status::Pressed => {
                if matches!(kind, BtnKind::Plain | BtnKind::Ptt) {
                    bg = shade(ui::Shade::ControlHover);
                }
            }
            button::Status::Disabled => {
                bg = shade(ui::Shade::Panel);
                fg = role_color(ui::ColorRole::Inactive);
            }
            button::Status::Active => {}
        }
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: fg,
            border: Border {
                color: edge,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..button::Style::default()
        }
    }
}

/// Grouping panel: a slightly raised rounded surface (FR-UI-15).
fn panel_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(shade(ui::Shade::Panel))),
        border: Border {
            color: shade(ui::Shade::Edge),
            width: 1.0,
            radius: 10.0.into(),
        },
        ..container::Style::default()
    }
}

/// Spectrum-pane style: like [`panel_style`] but with an accent border + faint
/// tint when this pane is the transmit VFO (FR-UI-12), so the selected frame
/// reads as active.
fn pane_style(selected: bool) -> impl Fn(&Theme) -> container::Style {
    move |_theme| {
        let (r, g, b) = ui::ColorRole::TxActive.rgb();
        let accent = Color::from_rgb8(r, g, b);
        container::Style {
            background: Some(Background::Color(shade(ui::Shade::Panel))),
            border: Border {
                color: if selected {
                    accent
                } else {
                    shade(ui::Shade::Edge)
                },
                width: if selected { 2.0 } else { 1.0 },
                radius: 10.0.into(),
            },
            ..container::Style::default()
        }
    }
}

/// S-meter bar style: recessed track, green fill turning caution-yellow on
/// strong signals (FR-UI-10/15).
fn meter_style(strong: bool) -> impl Fn(&Theme) -> progress_bar::Style {
    move |_theme| progress_bar::Style {
        background: Background::Color(shade(ui::Shade::Track)),
        bar: Background::Color(role_color(if strong {
            ui::ColorRole::Caution
        } else {
            ui::ColorRole::VfoB
        })),
        border: Border {
            color: shade(ui::Shade::Edge),
            width: 1.0,
            radius: 4.0.into(),
        },
    }
}

/// Receiver badge: a filled tag in the VFO's semantic colour with dark text,
/// like the K4's corner `A`/`B` markers (FR-UI-10).
fn badge(label: &'static str, role: ui::ColorRole) -> Element<'static, Message> {
    Container::new(Text::new(label).size(15).color(Color::BLACK))
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(role_color(role))),
            border: Border {
                radius: 4.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .padding([2, 8])
        .into()
}

/// A two-line state button (FR-UI-11): small function label over the live
/// value, blue-filled when the toggle is on (FR-UI-10). `msg: None` renders it
/// as a read-only indicator (disabled).
fn two_line_btn(
    state: ui::ButtonState,
    on: Option<bool>,
    msg: Option<Message>,
) -> Element<'static, Message> {
    // The pure layer decides engaged vs. inactive (FR-UI-10); the view maps
    // "engaged" to the reference client's blue fill.
    let engaged = on.map(ui::toggle_role) == Some(ui::ColorRole::VfoB);
    let kind = if engaged {
        BtnKind::Active
    } else {
        BtnKind::Plain
    };
    let label_color = if engaged {
        Color::WHITE
    } else {
        role_color(ui::ColorRole::Inactive)
    };
    let content = Column::new()
        .align_x(Alignment::Center)
        .push(Text::new(state.label).size(10).color(label_color))
        .push(Text::new(state.value).size(13));
    let mut b = Button::new(content)
        .style(btn_style(kind))
        .width(Length::Fixed(66.0))
        .padding([4, 6]);
    if let Some(m) = msg {
        b = b.on_press(m);
    }
    b.into()
}

/// Message for a connect-control action (FR-UI-16). Cancel and Disconnect both
/// tear down via the worker's `Disconnect` command (which also aborts a pending
/// attempt).
fn connect_msg(action: ui::ConnectAction) -> Message {
    match action {
        ui::ConnectAction::Connect => Message::Connect,
        ui::ConnectAction::Cancel | ui::ConnectAction::Disconnect => Message::Disconnect,
    }
}

/// Button style for a connect-control action: the idle "Connect" is the primary
/// (blue) action; Cancel/Disconnect are plain.
fn connect_kind(action: ui::ConnectAction) -> BtnKind {
    match action {
        ui::ConnectAction::Connect => BtnKind::Active,
        ui::ConnectAction::Cancel | ui::ConnectAction::Disconnect => BtnKind::Plain,
    }
}

/// Small plain action button.
fn small_btn(label: &'static str, msg: Message) -> Element<'static, Message> {
    Button::new(Text::new(label).size(12))
        .style(btn_style(BtnKind::Plain))
        .padding([5, 10])
        .on_press(msg)
        .into()
}

/// Small plain action button with an owned (dynamic) label.
fn small_btn_string(label: String, msg: Message) -> Element<'static, Message> {
    Button::new(Text::new(label).size(12))
        .style(btn_style(BtnKind::Plain))
        .padding([5, 10])
        .on_press(msg)
        .into()
}

/// A `label · − · value · +` stepper row for the DISPLAY screen (FR-PAN-CTL-01).
/// One DISPLAY-grid cell: `label [−] value [+]`, a fixed total width so the
/// label / buttons / value line up in columns across every row (FR-PAN-CTL-01).
fn disp_stepper(
    label: &'static str,
    value: String,
    dn: Message,
    up: Message,
) -> Element<'static, Message> {
    let cell = Row::new()
        .spacing(6)
        .align_y(Alignment::Center)
        .push(
            Text::new(label)
                .size(11)
                .width(Length::Fixed(46.0))
                .color(role_color(ui::ColorRole::Inactive)),
        )
        .push(small_btn("−", dn))
        .push(
            Text::new(value)
                .size(13)
                .width(Length::Fixed(76.0))
                .align_x(Alignment::Center),
        )
        .push(small_btn("+", up));
    Container::new(cell).width(Length::Fixed(200.0)).into()
}

/// K4 `#DPM` display-mode code for a [`ViewMode`] (FR-PAN-CTL-01).
fn pan_mode_code(m: ViewMode) -> u8 {
    match m {
        ViewMode::SingleA => 0,
        ViewMode::SingleB => 1,
        ViewMode::Dual => 2,
    }
}

fn labeled<'a>(
    label: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    Row::new()
        .spacing(8)
        .align_y(Alignment::Center)
        .push(Text::new(label).size(12).width(Length::Fixed(80.0)))
        .push(TextInput::new("", value).size(13).on_input(on_input))
        .into()
}

fn secret<'a>(
    label: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    Row::new()
        .spacing(8)
        .align_y(Alignment::Center)
        .push(Text::new(label).size(12).width(Length::Fixed(80.0)))
        .push(
            TextInput::new("", value)
                .size(13)
                .secure(true)
                .on_input(on_input),
        )
        .into()
}

/// S-meter label from a dBm value alone (per-VFO, FR-UI-12).
fn fmt_dbm(dbm: Option<i32>) -> String {
    match dbm {
        Some(dbm) => format!("{} ({dbm} dBm)", k4_protocol::s_unit_label(dbm)),
        None => "—".to_string(),
    }
}

/// Map a semantic colour role (ARC-15) to an iced colour for the active theme
/// (FR-UI-10/17).
fn role_color(role: ui::ColorRole) -> Color {
    let (r, g, b) = ui::role_rgb(active_theme(), role);
    Color::from_rgb8(r, g, b)
}

/// Map a surface shade (ARC-15) to an iced colour for the active theme
/// (FR-UI-15/17).
fn shade(s: ui::Shade) -> Color {
    let (r, g, b) = ui::shade_rgb(active_theme(), s);
    Color::from_rgb8(r, g, b)
}

/// Keychain account key for a connection (`host:port`).
fn account_key(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// Parse a MHz string (e.g. "14.074") into Hz.
fn parse_mhz(s: &str) -> Option<u64> {
    let mhz: f64 = s.trim().parse().ok()?;
    if mhz <= 0.0 {
        return None;
    }
    Some((mhz * 1_000_000.0).round() as u64)
}
