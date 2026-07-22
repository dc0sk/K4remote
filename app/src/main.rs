//! K4 Remote — GUI application (ARC-08, iced).
//!
//! The view is a pure projection of [`UiSnapshot`] (ADR-04); all radio I/O runs
//! on a background [`worker`] thread, bridged by a command channel + a shared
//! snapshot polled on a timer (ADR-06, FR-UI-07). Layout and styling follow the
//! K4's native LCD and the reference client's visual language (R-EXT-02,
//! ADR-15): a dark layered theme, banded frame, grids of two-line state
//! buttons, and proportional S-meter bars (FR-UI-08..15).

mod meter;
mod spectrum;
mod tips;
mod ui;
mod update;
mod worker;

use ui::ViewMode;

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use k4_config::{Config, KpodButton, Profile, SecretStore};

use std::cell::Cell;

use iced::widget::canvas::Canvas;
use iced::widget::{
    button, container, horizontal_space, mouse_area, pick_list, progress_bar, scrollable, slider,
    stack, text_editor, vertical_slider,
};
use iced::widget::{
    tooltip, Button, Column, Container, MouseArea, ProgressBar, Row, Space, Text, TextInput,
    Tooltip,
};
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
    // Multi-window (daemon): the main window plus an optional detached
    // diagnostics window (FR-DIAG-04). Windows are opened in `App::new`.
    iced::daemon(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(|app, _id| app.theme())
        .run_with(App::new)
}

/// App icon (taskbar / window bar), embedded from the packaging assets.
fn app_icon() -> Option<iced::window::Icon> {
    iced::window::icon::from_file_data(
        include_bytes!("../../packaging/icons/k4remote-128.png"),
        None,
    )
    .ok()
}

/// Window settings for the detached diagnostics console.
fn diag_window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(560.0, 420.0),
        icon: app_icon(),
        ..Default::default()
    }
}

struct App {
    // connection form
    host: String,
    port: String,
    password: String,
    // tuning form
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
    // current window height, to keep an anchored popup on screen (FR-UI-POPUP-01)
    window_h: f32,
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
    // MENU value editing (FR-MENU-01): the selected item id and the pending new
    // value typed into its editor; the current value comes from the radio's
    // `menu_values` read-back.
    menu_selected: Option<u16>,
    menu_edit: String,
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
    // The RX chip whose settings popup is open, if any (FR-UI-POPUP-01).
    rx_popup: Option<ui::RxPopup>,
    // Live pointer position, and where it was when the popup was opened — the
    // popup is anchored at the control rather than centred on the window.
    cursor: (f32, f32),
    rx_popup_at: (f32, f32),
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
    // Volume control positions, 0–100 % (FR-AUD-LVL-01). The gain they map to
    // is `k4_audio::gain_from_level` — a perceptual curve, so the control can
    // read 0–100 % while still reaching +24 dB for a quiet stream.
    volume: u8,
    // Per-receiver trim [main, sub], 0–100 % (FR-RX-VOL-01). Balances the two
    // receivers against each other; overall loudness is the master's job, so
    // these only attenuate. Local to this app — the radio's own AF gain is
    // untouched, so this cannot disturb the front panel or another client.
    rx_volume: [u8; 2],
    // Per-receiver mute [main, sub] (FR-RX-VOL-01). Deliberately **not**
    // persisted: an app that starts muted looks broken, and the operator has
    // no reason to suspect a setting from a previous session.
    rx_muted: [bool; 2],
    mic_gain: f32, // TX capture gain 0.0–3.0
    // Two-step guard for the remote power-off (FR-PWR-01).
    power_off_armed: bool,
    // Multi-window: the main window id and the optional detached diagnostics
    // window (FR-DIAG-04).
    main_window: iced::window::Id,
    diag_window: Option<iced::window::Id>,
    diag_enabled: bool,
    // Diagnostics log: show/hide + follow-newest (auto-scroll).
    show_log: bool,
    log_autoscroll: bool,
    // Divider for the log-editor rebuild, which is far too expensive to run at
    // the 10 Hz tick rate (it re-shapes the whole visible buffer).
    log_refresh_div: u8,
    // The log is rendered in a read-only `text_editor` so lines can be selected
    // and copied natively (Ctrl+C). `log_content` holds the editor buffer;
    // `log_text` is the text it was built from, so it's only rebuilt when the
    // visible text actually changes (preserving selection while frozen).
    log_content: text_editor::Content,
    log_text: String,
    // Frozen log snapshot rendered while auto-scroll is off, so the churning
    // live traffic doesn't make the console unreadable.
    log_frozen: Vec<String>,
    // Case-insensitive substring filter for the log console (empty = show all).
    log_filter: String,
    // Local RX filter bandwidth, Hz (seeded from the radio; cycled by the BW btn).
    bw_hz: u32,
    // Which VFO transmits (B under split); tracks the radio, set optimistically
    // when a spectrum frame is clicked so the highlight moves immediately.
    tx_vfo_b: bool,
    // Last split value seen from the radio; tx_vfo_b is re-synced only when this
    // actually changes (a genuine transition), so a static/absent read-back never
    // snaps the optimistic highlight back.
    last_split: Option<bool>,
    // Last K4 panadapter mode seen (#DPM), so the A/B/A+B view follows it on a
    // genuine change without fighting a manual selection.
    last_pan_mode: Option<u8>,
    // Last per-VFO modes seen, so the per-mode tuning step (VT/VT$) is re-queried
    // when a VFO's mode is first known / changes (the K4 stores it per mode).
    last_mode_a: Option<k4_protocol::state::Mode>,
    last_mode_b: Option<k4_protocol::state::Mode>,
    // Optimistic VFO frequency after a digit click, so the readout updates
    // instantly instead of waiting for the radio's echo (FR-VFO-08). Cleared
    // once the radio confirms, or after a short staleness timeout (see ui::OptVfo).
    opt_vfo: ui::OptVfo,
    // Same optimistic pattern for the TX power range (H/L/X): adopt the radio's
    // range only on a genuine change, so a lagging read-back doesn't revert a
    // just-clicked range button.
    last_pwr_range: Option<char>,
    // Main-RX levels (seeded from the radio, driven by sliders): AF gain 0–60,
    // RF-gain attenuation 0–60 dB, squelch 0–40 (FR-RX-01, FR-RX-SQL-01).
    af_gain: u8,
    rf_gain: u8,
    squelch: u8,
    // TX levels: power (W, QRO) and speech compression 0–30 (FR-TX-02, FR-TX-CMP-01).
    tx_power: u16,
    tx_pwr_range: char,
    compression: u8,
    // RX DSP + VOX level sliders (local mirrors for smooth dragging).
    nb_level: u8,
    nr_level: u8,
    // Attenuator level (dB). A local mirror for the same reason the levels
    // above have one: the popup's slider must follow the drag, not the
    // read-back it is racing (FR-UI-POPUP-01).
    atten_db: u8,
    // Optimistic override for the mirror above. A read-back already in flight
    // when the level was set reports the *previous* one, and the resync would
    // copy it over the operator's choice — the intermittent "jumps back to the
    // old value". Held until the radio confirms, expired if it never does.
    opt_atten: ui::OptLevel,
    vox_gain: u8,
    anti_vox: u8,
    // CW sidetone pitch Hz (FR-KEY-02); full-QSK + VOX/QSK delay 10-ms (FR-TX-DLY-01).
    cw_pitch: u16,
    qsk_full: bool,
    qsk_delay: u8,
    // Passband shift / AF center pitch, Hz (FR-FIL-01).
    shift_hz: u16,
    // Manual-notch pitch Hz (FR-RX-NOTCH-01); on/off + auto-notch + APF read live
    // from the radio state.
    notch_pitch: u16,
    // Filter slider view: false = SHIFT (BW+shift), true = LO/HI edges (FR-FIL-02).
    filter_edge_view: bool,
    // DISPLAY-screen pan target: false = main (A), true = sub (B) (FR-PAN-CTL-01).
    pan_target_b: bool,
    // Mute the radio's TX monitor (ML=0) once per connect (remote-friendly).
    mute_mon: bool,
    mon_muted: bool,
    // PTT keyboard hotkey (push-to-talk): the configured combo, capture mode,
    // and press/keyed tracking. `arm_flash` blinks the ARM button when the
    // hotkey is pressed while disarmed.
    ptt_hotkey: String,
    ptt_toggle: bool,
    // Mode-adaptive UI: per-mode control emphasis (docs/concept/mode-aware-ui.md).
    mode_aware_ui: bool,
    // Elecraft K-Pod USB control surface enabled (runtime opt-in, FR-KPOD-04).
    kpod_enabled: bool,
    // K-Pod function-switch macro table: 16 slots (F1–F8 × tap/hold), each a CAT
    // string sent to the K4 on press. Edited in Settings, seeded from Elecraft
    // samples (FR-KPOD-06).
    kpod_buttons: Vec<KpodButton>,
    capturing_hotkey: bool,
    hotkey_down: bool,
    hotkey_keyed: bool,
    arm_flash: u8,
    /// Which control the pointer is resting on, and since when. A tip appears
    /// only once it has been there for `tips::TOOLTIP_DELAY` — iced 0.13's
    /// tooltip has no delay of its own, so the 100 ms UI tick drives it
    /// (FR-UI-TIP-01).
    hover: Option<(&'static str, std::time::Instant)>,
    /// When the current press began, for tap-vs-hold (FR-UI-HOLD-01).
    press_at: Option<std::time::Instant>,
    /// Whether tooltips are shown at all (persisted preference).
    tooltips: bool,
    /// Result of the last About-box update check (FR-UI-UPD-01).
    update_status: update::UpdateStatus,
    // Text decode (FR-TXT-01): on/off + a poll-rate divider for the TB reads.
    decode_on: bool,
    decode_tick: u8,
    // Periodic-resync divider: pulls radio→slider values and re-queries settings
    // so changes made at the K4 sync back to the app (FR-CAT-07).
    resync_tick: u32,
    // Which RX (VFO A/B) the frame controls target. Set by the header A/B and by
    // clicking a spectrum pane (needed in the A+B view where both are shown).
    active_rx_b: bool,
    // Brief tap feedback for momentary switch buttons: the last SW code tapped
    // and a countdown of ticks to keep it highlighted.
    switch_flash: Option<u16>,
    switch_flash_ticks: u8,
    // TUNE / TUNE LP have no read-back, so track their on/off locally (toggled
    // on tap, cleared when transmit ends).
    tune_on: bool,
    tune_lp_on: bool,
    // K4 settings export/import (FR-CFG-06): a path to load and the commands
    // parsed from it, plus a status/feedback line.
    import_path: String,
    loaded_commands: Vec<String>,
    backup_status: String,
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
    /// Panadapter noise blanker (`#NB`): 0 off, 1 on, 2 auto. Cleans the
    /// display without touching receive audio.
    pan_nb: u8,
    /// Panadapter NB level (`#NBL`), 0–14.
    pan_nb_level: u8,
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
            pan_nb: 0,
            pan_nb_level: 5,
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
    /// Panadapter fixed-tune (`#FXT`): pan stays put vs tracks the VFO.
    Fixed(bool),
    /// Panadapter noise blanker: 0 off, 1 on, 2 auto (`#NB`).
    PanNb(u8),
    /// Panadapter noise-blanker level, 0–14 (`#NBL`).
    PanNbLevel(u8),
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
    Tune(bool, bool),
    Connect,
    Disconnect,
    ToggleTls,
    ToggleRemember,
    ToggleSerialMode,
    SerialPathChanged(String),
    SerialBaudChanged(String),
    SetMode(u8),
    CycleMode(bool),
    /// Tune one frequency digit: (is_b, place value 10^k, up). Rolls within the
    /// clicked digit only (no carry to neighbours).
    FreqDigit(bool, u64, bool),
    Band(bool),
    ToggleAtten,
    ToggleSplit,
    CycleAgc,
    CycleBandwidth,
    SelectTxVfo(bool),
    PaneQsy(bool, u64),
    PaneWheel(bool, i32),
    SetAfGain(u8),
    SetRfGain(u8),
    SetSquelch(u8),
    SetDataSubmode(u8),
    SetTxPower(u16),
    SetCompression(u8),
    SetCwPitch(u16),
    ToggleQskFull,
    SetQskDelay(u8),
    SetShift(u16),
    ToggleFilterEdgeView,
    SetLoCut(u16),
    SetHiCut(u16),
    SetPanTarget(bool),
    ToggleMiniPan,
    ToggleMuteMon,
    FilterPreset(u8),
    FilterNormalize,
    ToggleSubRx,
    ToggleDiversity,
    ToggleManualNotch,
    SetNotchPitch(u16),
    ToggleAutoNotch,
    ToggleApf,
    CycleApfWidth,
    ToggleNb,
    ToggleNr,
    TogglePreamp,
    ToggleRit,
    ToggleXit,
    ClearRitXit,
    AdjustRitOffset(i16),
    SetMonitor(u8),
    Autospot,
    SetRepeaterMode(char),
    AdjustPlTone(i8),
    TogglePl,
    DvrPlay(u8),
    SetTxPowerRange(char),
    SetNbLevel(u8),
    SetNrLevel(u8),
    SetVoxGain(u8),
    SetAntiVox(u8),
    ToggleArm,
    TogglePttMode,
    ToggleModeAwareUi,
    ToggleKpod,
    /// Edit a K-Pod slot's free-form CAT macro string (slot index, text).
    KpodButtonCatChanged(usize, String),
    /// Apply a preset (by label) to a K-Pod slot, filling its label + CAT.
    KpodButtonPreset(usize, String),
    /// Reset the whole K-Pod macro table to the Elecraft sample seed.
    KpodButtonsReset,
    KeyPressed(
        iced::keyboard::Key,
        iced::keyboard::Modifiers,
        iced::window::Id,
    ),
    KeyReleased(iced::keyboard::Key, iced::keyboard::Modifiers),
    StartCaptureHotkey,
    ToggleKey,
    EmergencyStop,
    /// Deliberately does nothing: swallows a click so it cannot reach a
    /// click-away handler underneath (the RX popup card).
    Noop,
    /// Right-click on an RX chip: open that control's settings popup
    /// (FR-UI-POPUP-01).
    OpenRxPopup(ui::RxPopup),
    /// Dismiss the open RX settings popup.
    CloseRxPopup,
    /// Popup: set the attenuator to an absolute level, in dB (`RA`).
    SetAttenDb(u8),
    /// Popup: set the preamp to an absolute level, 0 (off) to 3 (`PA`).
    SetPreampLevel(u8),
    /// Popup: set AGC to an absolute mode — 0 off, 1 slow, 2 fast (`GT`).
    SetAgcMode(u8),
    /// Popup: set the APF bandwidth — 0 = 30, 1 = 50, 2 = 150 Hz (`AP`).
    SetApfWidth(u8),
    /// Popup: set the NB filter mode absolutely — 0 none, 1 narrow, 2 wide
    /// (`NB`). The chip's hold cycles the same setting.
    SetNbFilter(u8),
    /// Popup: the attenuator slider was released — reconcile the chip with
    /// the radio now the drag is over, rather than mid-drag.
    QueryAtten,
    /// A press began on a tap/hold control (FR-UI-HOLD-01).
    PressDown,
    /// A press ended: run the first message if it was a tap, the second if it
    /// was held past [`ui::HOLD_THRESHOLD`].
    TapOrHold(Box<Message>, Box<Message>),
    /// Pointer entered a control (tooltip hover tracking, FR-UI-TIP-01).
    HoverEnter(&'static str),
    /// Pointer left the control it was resting on.
    HoverExit,
    /// Enable/disable control tooltips.
    SetTooltips(bool),
    /// Run/stop a transmit tune (`TU`, FR-TX-TUNE-01). Distinct from
    /// [`Message::Tune`], which steps a VFO.
    TxTune(k4_protocol::cat::TuneAction),
    /// Toggle the ATU in/bypass (`AT/`, FR-ATU-01).
    AtuToggle,
    CatInputChanged(String),
    SendCat,
    SetViewMode(ViewMode),
    TapPrimary(ui::Primary),
    CycleTheme,
    ToggleAbout,
    /// Open a URL in the OS browser (About-box links / donate).
    OpenUrl(&'static str),
    /// Open a URL discovered at runtime (the release page from an update check).
    OpenUrlOwned(String),
    /// Ask GitHub whether a newer release exists (FR-UI-UPD-01).
    CheckUpdate,
    /// The outcome of that check.
    UpdateChecked(update::UpdateStatus),
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
    VolumeChanged(u8),
    /// Set one receiver's **local** playback volume (is_b, 0.0–2.0), the
    /// per-pane control above the spectrum. FR-RX-VOL-01.
    RxVolumeChanged(bool, u8),
    /// Mute/unmute one receiver locally (is_b). FR-RX-VOL-01.
    ToggleRxMute(bool),
    MicGainChanged(f32),
    SaveSettings,
    ExportConfig,
    SweepMenu,
    ImportPathChanged(String),
    LoadConfig,
    PlaybackConfig,
    // Remote power control (FR-PWR-01).
    PowerRestart,
    PowerOffArm,
    PowerOffCancel,
    PowerOffConfirm,
    // Diagnostics log display options.
    ToggleShowLog,
    ToggleDiagWindow,
    WindowOpened,
    WindowClosed(iced::window::Id),
    ToggleLogAutoscroll,
    LogFilterChanged(String),
    /// A text-editor action from the read-only log view (selection/scroll; edits
    /// are ignored).
    LogEditorAction(text_editor::Action),
    /// Copy the currently-visible (filtered) log lines to the clipboard.
    CopyLog,
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
    // Front-panel switch emulation (FR-SW-01): quick memories, PF keys.
    Switch(u16),
    // MENU screen search (FR-MENU-01).
    MenuFilter(String),
    // MENU value editing (FR-MENU-01): select an item (queries its value), edit
    // the pending value, set it (`ME<id>.<value>`), or nudge a numeric value ±.
    MenuSelect(u16),
    MenuEditChanged(String),
    MenuSet,
    MenuNudge(i64),
    // BAND transverter select (FR-VFO-04), TX text (FR-TX-MSG-01), Fn tabs / DX.
    SelectXvtr(u8),
    TxText(String),
    SendTxText,
    ToggleDecode,
    SetFnTab(FnTab),
    DxFilter(String),
    // RX config sub-screens (FR-ANT-01/FR-AUD-CFG-01, Phase D).
    Rx(RxMsg),
    Resized(iced::window::Id, f32, f32),
    /// Pointer moved, in window coordinates — remembered so a popup can open
    /// at the control instead of the middle of the window (FR-UI-POPUP-01).
    CursorMoved(f32, f32),
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
        // Positions if this config has them; otherwise migrate the old raw
        // multipliers, so upgrading does not change how loud the radio is.
        let volume = prefs
            .volume_level
            .unwrap_or_else(|| k4_audio::level_from_gain(prefs.volume_pct as f32 / 100.0));
        let rx_volume = [
            prefs
                .rx_volume_main_level
                .unwrap_or_else(|| prefs.rx_volume_main_pct.min(100) as u8),
            prefs
                .rx_volume_sub_level
                .unwrap_or_else(|| prefs.rx_volume_sub_pct.min(100) as u8),
        ];
        let mic_gain = prefs.mic_gain_pct as f32 / 100.0;
        // Seed the worker with the restored audio settings before any connect.
        let _ = cmd_tx.send(WorkerCmd::SetOutputDevice(selected_output.clone()));
        let _ = cmd_tx.send(WorkerCmd::SetInputDevice(selected_input.clone()));
        let _ = cmd_tx.send(WorkerCmd::SetVolume(k4_audio::gain_from_level(volume)));
        let _ = cmd_tx.send(WorkerCmd::SetRxVolume(
            false,
            f32::from(rx_volume[0]) / 100.0,
        ));
        let _ = cmd_tx.send(WorkerCmd::SetRxVolume(
            true,
            f32::from(rx_volume[1]) / 100.0,
        ));
        let _ = cmd_tx.send(WorkerCmd::SetMicGain(mic_gain));
        let kpod_enabled = prefs.kpod_enabled;
        let _ = cmd_tx.send(WorkerCmd::SetKpodEnabled(kpod_enabled));
        let kpod_buttons = prefs.kpod_buttons.clone();
        let _ = cmd_tx.send(WorkerCmd::SetKpodButtons(
            kpod_buttons.iter().map(|b| b.cat.clone()).collect(),
        ));
        let theme_mode = theme_from_prefs(prefs.theme.as_deref());
        let mute_mon = prefs.mute_radio_mon;
        let ptt_hotkey = prefs.ptt_hotkey.clone();
        let ptt_toggle = prefs.ptt_toggle;
        let mode_aware_ui = prefs.mode_aware_ui;
        let diag_enabled = prefs.diagnostics_window;
        let tooltips = prefs.tooltips;

        // Open the main window; the daemon starts with none (FR-DIAG-04).
        let (main_window, open_main) = iced::window::open(iced::window::Settings {
            size: iced::Size::new(
                ui::DEFAULT_WINDOW_SIZE.0.max(1320.0),
                ui::DEFAULT_WINDOW_SIZE.1,
            ),
            min_size: Some(iced::Size::new(1320.0, 1000.0)),
            icon: app_icon(),
            ..Default::default()
        });
        let mut window_tasks = vec![open_main.map(|_| Message::WindowOpened)];
        // Restore the detached diagnostics window if it was enabled.
        let diag_window = if diag_enabled {
            let (id, open) = iced::window::open(diag_window_settings());
            window_tasks.push(open.map(|_| Message::WindowOpened));
            Some(id)
        } else {
            None
        };

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
            // Start on the spectrum/waterfall, not a pre-opened BAND screen.
            context: ui::ContextRow::default(),
            window_w: 1280.0,
            window_h: 964.0,
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
            menu_selected: None,
            menu_edit: String::new(),
            tx_text: String::new(),
            fn_tab: FnTab::Keys,
            dx_filter: String::new(),
            seeded: false,
            peers,
            settings_open: false,
            rx_popup: None,
            cursor: (0.0, 0.0),
            rx_popup_at: (0.0, 0.0),
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
            rx_volume,
            rx_muted: [false, false],
            mic_gain,
            power_off_armed: false,
            main_window,
            diag_window,
            diag_enabled,
            show_log: false,
            log_autoscroll: true,
            log_refresh_div: 0,
            log_content: text_editor::Content::new(),
            log_text: String::new(),
            log_frozen: Vec::new(),
            log_filter: String::new(),
            bw_hz: 2800,
            tx_vfo_b: false,
            last_split: None,
            last_pan_mode: None,
            last_mode_a: None,
            last_mode_b: None,
            opt_vfo: ui::OptVfo::default(),
            last_pwr_range: None,
            af_gain: 30,
            rf_gain: 0,
            squelch: 0,
            tx_power: 10,
            tx_pwr_range: 'H',
            nb_level: 5,
            nr_level: 5,
            atten_db: 0,
            opt_atten: ui::OptLevel::default(),
            vox_gain: 20,
            anti_vox: 0,
            compression: 0,
            cw_pitch: 600,
            qsk_full: false,
            qsk_delay: 30,
            shift_hz: 1500,
            notch_pitch: 1000,
            filter_edge_view: false,
            pan_target_b: false,
            mute_mon,
            mon_muted: false,
            ptt_hotkey,
            ptt_toggle,
            mode_aware_ui,
            kpod_enabled,
            kpod_buttons,
            capturing_hotkey: false,
            hotkey_down: false,
            hotkey_keyed: false,
            arm_flash: 0,
            hover: None,
            press_at: None,
            tooltips,
            update_status: update::UpdateStatus::Idle,
            decode_on: false,
            decode_tick: 0,
            resync_tick: 0,
            active_rx_b: false,
            switch_flash: None,
            switch_flash_ticks: 0,
            tune_on: false,
            tune_lp_on: false,
            import_path: String::new(),
            loaded_commands: Vec::new(),
            backup_status: String::new(),
        };
        (app, Task::batch(window_tasks))
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
    /// Generate replayable CAT commands for the K4 settings the app tracks, from
    /// the current radio state (FR-CFG-06). Main + sub RX variants included.
    fn export_commands(&self) -> Vec<String> {
        use k4_protocol::cat;
        let r = &self.ui.radio;
        let mut c: Vec<String> = Vec::new();
        if let Some(v) = r.vfo_a_hz {
            c.push(cat::set_vfo_a_hz(v));
        }
        if let Some(v) = r.vfo_b_hz {
            c.push(cat::set_vfo_b_hz(v));
        }
        if let Some(m) = r.mode_a {
            c.push(cat::set_mode(md_digit(m)));
        }
        if let Some(m) = r.mode_b {
            c.push(cat::set_mode_sub(md_digit(m)));
        }
        // RX levels + filter/notch, for main (false) then sub (true).
        for (sub, bw, ag, rg, sq, is, non, npi, an, ap, aw) in [
            (
                false,
                r.bandwidth_hz,
                r.af_gain,
                r.rf_gain_db,
                r.squelch,
                r.shift_hz,
                r.notch_on,
                r.notch_pitch,
                r.auto_notch,
                r.apf_on,
                r.apf_width,
            ),
            (
                true,
                r.sub_bandwidth_hz,
                r.sub_af_gain,
                r.sub_rf_gain_db,
                r.sub_squelch,
                r.sub_shift_hz,
                r.sub_notch_on,
                r.sub_notch_pitch,
                r.sub_auto_notch,
                r.sub_apf_on,
                r.sub_apf_width,
            ),
        ] {
            if let Some(v) = bw {
                c.push(target_rx(cat::set_bandwidth_hz(v), sub));
            }
            if let Some(v) = ag {
                c.push(target_rx(cat::set_af_gain(v), sub));
            }
            if let Some(v) = rg {
                c.push(target_rx(cat::set_rf_gain(v), sub));
            }
            if let Some(v) = sq {
                c.push(target_rx(cat::set_squelch(v), sub));
            }
            if let Some(v) = is {
                c.push(target_rx(cat::set_shift_hz(v), sub));
            }
            if let (Some(on), Some(p)) = (non, npi) {
                c.push(target_rx(cat::set_manual_notch(on, p), sub));
            }
            if let Some(on) = an {
                c.push(target_rx(cat::set_auto_notch(on), sub));
            }
            if let (Some(on), Some(w)) = (ap, aw) {
                c.push(target_rx(cat::set_apf(on, w), sub));
            }
        }
        if let Some(v) = r.agc_mode {
            c.push(cat::set_agc(v));
        }
        if let (Some(db), Some(on)) = (r.atten_db, r.atten_on) {
            c.push(cat::set_attenuator(db, on));
        }
        if let Some(v) = r.tx_power {
            c.push(cat::set_tx_power_range(v, r.tx_power_range.unwrap_or('H')));
        }
        if let Some(v) = r.compression {
            c.push(cat::set_compression(v));
        }
        if let Some(v) = r.cw_pitch {
            c.push(cat::set_cw_pitch(v));
        }
        if let (Some(ib), Some(pr), Some(w)) =
            (r.keyer_iambic_b, r.keyer_paddle_rev, r.keyer_weight)
        {
            c.push(cat::set_keyer(ib, pr, w));
        }
        if let Some(v) = r.keyer_speed {
            c.push(cat::set_keyer_speed(v));
        }
        if let Some(v) = r.mic_input {
            c.push(cat::set_mic_input(v));
        }
        if let Some(v) = r.mic_gain {
            c.push(cat::set_mic_gain(v));
        }
        if let Some(v) = r.tx_antenna {
            c.push(cat::set_tx_antenna(v));
        }
        if let Some(v) = r.rx_antenna {
            c.push(cat::set_rx_antenna(v));
        }
        if let Some(v) = r.rx_antenna_sub {
            c.push(cat::set_rx_antenna_sub(v));
        }
        if let Some(on) = r.sub_rx {
            c.push(cat::set_sub_rx(on));
        }
        if let Some(on) = r.diversity {
            c.push(cat::set_diversity(on));
        }
        if let Some(on) = r.vox_voice {
            c.push(cat::set_vox('V', on));
        }
        if let Some(eq) = r.rx_eq {
            c.push(cat::set_rx_eq(eq));
        }
        if let Some(eq) = r.tx_eq {
            c.push(cat::set_tx_eq(eq));
        }
        if let Some(v) = r.pan_ref {
            c.push(cat::set_pan_ref(v));
        }
        if let Some(v) = r.pan_span_hz {
            c.push(cat::set_pan_span_hz(v));
        }
        if let Some(v) = r.pan_scale {
            c.push(cat::set_pan_scale(v as u8));
        }
        if let Some(v) = r.pan_mode {
            c.push(cat::set_pan_mode(v));
        }
        if let Some(v) = r.wf_palette {
            c.push(cat::set_waterfall_palette(v));
        }
        if let Some(v) = r.wf_height {
            c.push(cat::set_waterfall_height(v));
        }
        // Full-menu backup: replay any captured menu values (FR-CFG-07). Populated
        // by "Sweep menu"; empty otherwise (settings-only export).
        for (id, val) in &r.menu_values {
            c.push(format!("ME{id:04}.{val};"));
        }
        c
    }

    /// Export the current K4 settings to `K4-<serial>-<timestamp>.cfg` beside the
    /// app config, with a SHA-256 integrity hash (FR-CFG-06).
    fn export_config(&mut self) {
        let serial = self
            .ui
            .radio
            .serial
            .clone()
            .unwrap_or_else(|| "unknown".into());
        let ts = timestamp_now();
        let snap = k4_config::backup::Snapshot {
            serial: serial.clone(),
            timestamp: ts.clone(),
            commands: self.export_commands(),
        };
        if snap.commands.is_empty() {
            self.backup_status = "Nothing to export — connect and let settings load first.".into();
            return;
        }
        let dir = self
            .config_path
            .as_deref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let file = dir.join(format!("K4-{serial}-{ts}.cfg"));
        match std::fs::write(&file, k4_config::backup::export(&snap)) {
            Ok(()) => {
                self.backup_status = format!(
                    "Exported {} settings → {}",
                    snap.commands.len(),
                    file.display()
                );
            }
            Err(e) => self.backup_status = format!("Export failed: {e}"),
        }
    }

    /// Load and verify a `.cfg` from `import_path`, staging its commands for
    /// playback (FR-CFG-06).
    fn load_config_file(&mut self) {
        let path = self.import_path.trim();
        if path.is_empty() {
            self.backup_status = "Enter a .cfg path to import.".into();
            return;
        }
        match std::fs::read_to_string(path) {
            Ok(text) => match k4_config::backup::import(&text) {
                Ok(snap) => {
                    self.loaded_commands = snap.commands;
                    self.backup_status = format!(
                        "Loaded {} settings (serial {}, {}). Press Play to send.",
                        self.loaded_commands.len(),
                        snap.serial,
                        snap.timestamp
                    );
                }
                Err(e) => self.backup_status = e,
            },
            Err(e) => self.backup_status = format!("Read failed: {e}"),
        }
    }

    /// Persist the K-Pod macro table and push the CAT strings to the worker
    /// (FR-KPOD-06), so edits take effect on the next switch press immediately.
    fn push_kpod_buttons(&mut self) {
        self.send(WorkerCmd::SetKpodButtons(
            self.kpod_buttons.iter().map(|b| b.cat.clone()).collect(),
        ));
        self.save_config();
    }

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
                    volume_level: Some(self.volume),
                    rx_volume_main_level: Some(self.rx_volume[0]),
                    rx_volume_sub_level: Some(self.rx_volume[1]),
                    mic_gain_pct: (self.mic_gain * 100.0).round() as u16,
                    theme: Some(theme_to_str(self.theme_mode).to_string()),
                    mute_radio_mon: self.mute_mon,
                    diagnostics_window: self.diag_enabled,
                    tooltips: self.tips_on(),
                    ptt_hotkey: self.ptt_hotkey.clone(),
                    ptt_toggle: self.ptt_toggle,
                    mode_aware_ui: self.mode_aware_ui,
                    kpod_enabled: self.kpod_enabled,
                    kpod_buttons: self.kpod_buttons.clone(),
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
            Message::Tune(is_b, up) => self.step_vfo(is_b, up),
            Message::SetMode(digit) => self.send(WorkerCmd::SetMode(digit)),
            // MD+ (MD$+ for the sub) steps the mode through the K4's enabled
            // set; the MODE switch tap (SW43) only opens a chooser on the LCD.
            Message::CycleMode(is_b) => self.send(WorkerCmd::Cat(target_rx(
                k4_protocol::cat::cycle_mode(),
                is_b,
            ))),
            Message::FreqDigit(is_b, place, up) => {
                // Base on the optimistic value so rapid clicks accumulate without
                // waiting for the radio's echo.
                let cur = if is_b {
                    self.opt_vfo.b_or(self.ui.vfo_b_hz)
                } else {
                    self.opt_vfo.a_or(self.ui.vfo_a_hz)
                };
                if let Some(freq) = cur {
                    // Roll the clicked digit 0–9 within its own place (no carry).
                    let d = (freq / place) % 10;
                    let new_d = if up { (d + 1) % 10 } else { (d + 9) % 10 };
                    let delta = new_d as i64 * place as i64 - d as i64 * place as i64;
                    let new_freq = (freq as i64 + delta).max(0) as u64;
                    if is_b {
                        self.opt_vfo.set_b(new_freq);
                        self.send(WorkerCmd::SetFreqB(new_freq));
                    } else {
                        self.opt_vfo.set_a(new_freq);
                        self.send(WorkerCmd::SetFreqA(new_freq));
                    }
                }
                // Make the clicked digit the tuning cursor: set the K4 tune rate
                // (VT) to its place value (1 Hz…100 kHz), keeping the K4, the
                // K-Pod step, and the underline indicator in sync (FR-VFO-03).
                if let Some(index) = ui::tune_step_index(place) {
                    let mode = if is_b {
                        self.ui.radio.mode_b
                    } else {
                        self.ui.radio.mode_a
                    };
                    if let Some(m) = mode.map(md_digit) {
                        self.send(WorkerCmd::CatLocal(k4_protocol::cat::set_tune_step(
                            is_b, index, m,
                        )));
                    }
                }
            }
            Message::Band(up) => {
                // Band up/down applies to the **transmit** VFO — VFO B under split,
                // else VFO A — not always VFO A. `tx_vfo_b` tracks the radio's split
                // state (synced at connect), so this is right immediately, before
                // any A/B click (FR-VFO-04).
                let cmd = if up {
                    k4_protocol::cat::band_up()
                } else {
                    k4_protocol::cat::band_down()
                };
                self.send(WorkerCmd::Cat(target_rx(cmd.to_string(), self.tx_vfo_b)));
            }
            Message::ToggleAtten => {
                self.send(WorkerCmd::Cat(target_rx("RA/;".into(), self.active_sub())))
            }
            Message::Noop => {}
            Message::OpenRxPopup(p) => {
                // Freeze the anchor at the click: the pointer keeps moving
                // while the popup is up, and a card that followed it would be
                // impossible to aim at.
                self.rx_popup_at = self.cursor;
                // Drop the dwell, or the chip's tooltip stays up and covers
                // the popup it just opened. `on_enter` only fires on entry, so
                // it will not come back until the pointer leaves and returns.
                self.hover = None;
                self.rx_popup = Some(p);
            }
            Message::CloseRxPopup => self.rx_popup = None,
            Message::SetAttenDb(db) => {
                // The slider is free-running; the radio's ladder is not. Snap
                // before sending, and treat 0 dB as "out" for the same reason
                // the hold does — "on at 0" reads as engaged while doing
                // nothing (D14 p.1318).
                let db = ui::atten_snap(db);
                // Write the local mirror first and do NOT query: a drag emits
                // one of these per pixel, and querying each time made the
                // slider snap back to the last read-back mid-drag. `RA;` comes
                // back on the periodic resync, as it does for every other
                // level slider.
                self.atten_db = db;
                self.opt_atten.set(db);
                let sub = self.active_sub();
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_attenuator(db, db > 0),
                    sub,
                )));
            }
            Message::SetPreampLevel(level) => {
                let sub = self.active_sub();
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_preamp(level, level != 0),
                    sub,
                )));
                self.send(WorkerCmd::Cat(target_rx("PA;".into(), sub)));
            }
            Message::SetAgcMode(mode) => {
                let sub = self.active_sub();
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_agc(mode),
                    sub,
                )));
                self.send(WorkerCmd::Cat(target_rx("GT;".into(), sub)));
            }
            Message::QueryAtten => {
                let sub = self.active_sub();
                self.send(WorkerCmd::Cat(target_rx("RA;".into(), sub)));
            }
            Message::SetNbFilter(f) => {
                let sub = self.active_sub();
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_nb_level(self.nb_level, self.rx_nb_on() == Some(true), f),
                    sub,
                )));
                self.send(WorkerCmd::Cat(target_rx("NB;".into(), sub)));
            }
            Message::SetApfWidth(w) => {
                let sub = self.active_sub();
                let on = self.rx_apf_on() == Some(true);
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_apf(on, w),
                    sub,
                )));
                self.send(WorkerCmd::Cat(target_rx("AP;".into(), sub)));
            }
            Message::ToggleSplit => self.send(WorkerCmd::ToggleSplit),
            Message::CycleAgc => {
                // Tap: slow ↔ fast only. Switching AGC off is the *hold*
                // (D14 p.909), so a tap can never land on it by accident.
                let sub = self.active_sub();
                let next = ui::agc_tap(self.rx_agc_mode());
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_agc(next),
                    sub,
                )));
                self.send(WorkerCmd::Cat(target_rx("GT;".into(), sub)));
            }
            Message::CycleBandwidth => {
                // Step through common RX filter bandwidths (wraps at the top);
                // update locally for immediate feedback, then push to the radio.
                const BW: [u32; 8] = [500, 1000, 1500, 1800, 2400, 2700, 2800, 3200];
                self.bw_hz = BW
                    .iter()
                    .copied()
                    .find(|&b| b > self.bw_hz)
                    .unwrap_or(BW[0]);
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_bandwidth_hz(self.bw_hz),
                    self.active_sub(),
                )));
            }
            Message::SelectTxVfo(is_b) => {
                // Clicking a spectrum pane focuses that VFO: it becomes the active
                // RX (controls + RX A/B label follow it) and the TX VFO (FT/split).
                self.active_rx_b = is_b;
                self.sync_locals();
                // Move the TX highlight immediately; it only re-syncs on a genuine
                // split *transition*, so a stale read-back can't revert it.
                self.tx_vfo_b = is_b;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_split(is_b)));
            }
            Message::PaneQsy(is_b, hz) => {
                // Click-to-QSY: place the passband so the edge this mode
                // anchors lands on the clicked frequency — USB the low edge,
                // LSB the high edge, CW/AM/FM the centre (FR-PAN-05). Without
                // this the VFO went to the raw click, which on USB/LSB put the
                // passband a full filter-width off the signal.
                let hz = self.pane_click_vfo(is_b, hz);
                // (In dual view a wrapping mouse_area also selects the TX VFO.)
                if is_b {
                    self.opt_vfo.set_b(hz);
                    self.send(WorkerCmd::SetFreqB(hz));
                } else {
                    self.opt_vfo.set_a(hz);
                    self.send(WorkerCmd::SetFreqA(hz));
                }
            }
            Message::PaneWheel(is_b, dir) => self.pan_wheel(is_b, dir > 0),
            Message::SetAfGain(v) => {
                self.af_gain = v;
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_af_gain(v),
                    self.active_sub(),
                )));
            }
            Message::SetRfGain(v) => {
                self.rf_gain = v;
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_rf_gain(v),
                    self.active_sub(),
                )));
            }
            Message::SetSquelch(v) => {
                self.squelch = v;
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_squelch(v),
                    self.active_sub(),
                )));
            }
            // DATA sub-mode selector (DT/DT$). trace: FR-DATA-01
            Message::SetDataSubmode(n) => {
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_data_submode(
                    self.active_sub(),
                    n,
                )));
            }
            Message::SetTxPower(v) => {
                self.tx_power = v;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_tx_power_range(
                    v,
                    self.tx_pwr_range,
                )));
            }
            Message::SetTxPowerRange(r) => {
                self.tx_pwr_range = r;
                let max = if r == 'H' { 110 } else { 100 };
                self.tx_power = self.tx_power.min(max);
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_tx_power_range(
                    self.tx_power,
                    r,
                )));
            }
            Message::SetNbLevel(v) => {
                self.nb_level = v;
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_nb_level(
                        v,
                        self.rx_nb_on() == Some(true),
                        self.ui.radio.nb_filter.unwrap_or(0),
                    ),
                    self.active_sub(),
                )));
            }
            Message::SetNrLevel(v) => {
                self.nr_level = v;
                let mode = u8::from(self.rx_nr_on() == Some(true));
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_nr(v, mode),
                    self.active_sub(),
                )));
            }
            Message::SetVoxGain(v) => {
                self.vox_gain = v;
                let m = if self.tx_mode_class() == 'D' {
                    'D'
                } else {
                    'V'
                };
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_vox_gain(m, v)));
            }
            Message::SetAntiVox(v) => {
                self.anti_vox = v;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_antivox(v)));
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
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_shift_hz(hz),
                    self.active_sub(),
                )));
            }
            Message::ToggleFilterEdgeView => self.filter_edge_view = !self.filter_edge_view,
            Message::SetPanTarget(b) => self.pan_target_b = b,
            Message::ToggleMiniPan => self.send(WorkerCmd::Cat("#MP$/;".into())),
            Message::ToggleMuteMon => {
                self.mute_mon = !self.mute_mon;
                self.save_config();
                // Apply immediately if connected: mute now, or leave as-is.
                if self.mute_mon && self.ui.phase == ui::ConnPhase::Connected {
                    for m in 0..=2u8 {
                        self.send(WorkerCmd::Cat(k4_protocol::cat::set_monitor(m, 0)));
                    }
                    self.mon_muted = true;
                }
            }
            Message::SetLoCut(lo) => self.set_passband_edge(Some(lo), None),
            Message::SetHiCut(hi) => self.set_passband_edge(None, Some(hi)),
            Message::FilterPreset(n) => self.send(WorkerCmd::Cat(target_rx(
                k4_protocol::cat::set_filter_preset(n),
                self.active_sub(),
            ))),
            Message::FilterNormalize => self.send(WorkerCmd::Cat(target_rx(
                k4_protocol::cat::filter_normalize().to_string(),
                self.active_sub(),
            ))),
            Message::ToggleSubRx => {
                let on = self.ui.radio.sub_rx != Some(true);
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_sub_rx(on)));
                // Read back the actual state so an idempotent set (sub already in
                // that state) can't leave the button stuck on a stale value.
                self.send(WorkerCmd::Cat("SB;".into()));
            }
            Message::ToggleDiversity => {
                let on = self.ui.radio.diversity != Some(true);
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_diversity(on)));
                // Diversity also changes the sub receiver, so re-read both.
                self.send(WorkerCmd::Cat("DV;".into()));
                self.send(WorkerCmd::Cat("SB;".into()));
            }
            Message::ToggleManualNotch => {
                let on = self.rx_notch_on() != Some(true);
                let sub = self.active_sub();
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_manual_notch(on, self.notch_pitch),
                    sub,
                )));
                self.send(WorkerCmd::Cat(target_rx("NM;".into(), sub)));
            }
            Message::SetNotchPitch(p) => {
                self.notch_pitch = p;
                let on = self.rx_notch_on() == Some(true);
                let sub = self.active_sub();
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_manual_notch(on, p),
                    sub,
                )));
            }
            Message::ToggleAutoNotch => {
                let on = self.rx_auto_notch() != Some(true);
                let sub = self.active_sub();
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_auto_notch(on),
                    sub,
                )));
                self.send(WorkerCmd::Cat(target_rx("NA;".into(), sub)));
            }
            Message::ToggleApf => {
                let on = self.rx_apf_on() != Some(true);
                let w = self.rx_apf_width().unwrap_or(1);
                let sub = self.active_sub();
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_apf(on, w),
                    sub,
                )));
                self.send(WorkerCmd::Cat(target_rx("AP;".into(), sub)));
            }
            Message::CycleApfWidth => {
                let w = (self.rx_apf_width().unwrap_or(0) + 1) % 3;
                let on = self.rx_apf_on() == Some(true);
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_apf(on, w),
                    self.active_sub(),
                )));
            }
            Message::ToggleNb => {
                self.send(WorkerCmd::Cat(target_rx("NB/;".into(), self.active_sub())))
            }
            Message::ToggleNr => {
                let sub = self.active_sub();
                let on = self.rx_nr_on() == Some(true);
                let level = if sub {
                    self.ui.radio.sub_nr_level
                } else {
                    self.ui.radio.nr_level
                }
                .unwrap_or(5);
                let mode = if on { 0 } else { 1 };
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_nr(level, mode),
                    sub,
                )));
                self.send(WorkerCmd::Cat(target_rx("NR;".into(), sub)));
            }
            Message::TogglePreamp => {
                // Rotate through the preamp levels 0(off)→1→2→3→off (FR-RX-02).
                let on = self.rx_preamp_on().unwrap_or(false);
                let cur = if on {
                    self.ui.radio.preamp_level.unwrap_or(0)
                } else {
                    0
                };
                let next = (cur + 1) % 4;
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_preamp(next, next != 0),
                    self.active_sub(),
                )));
            }
            Message::ToggleRit => self.send(WorkerCmd::ToggleRit),
            Message::ToggleXit => self.send(WorkerCmd::ToggleXit),
            Message::ClearRitXit => self.send(WorkerCmd::ClearRitXit),
            Message::AdjustRitOffset(d) => {
                let next = (self.ui.radio.rit_offset.unwrap_or(0) + d).clamp(-9999, 9999);
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_rit_offset(next)));
            }
            Message::SetMonitor(level) => {
                let m = match self.tx_mode_class() {
                    'C' => 0,
                    'D' => 1,
                    _ => 2,
                };
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_monitor(m, level)));
            }
            Message::Autospot => self.send(WorkerCmd::Cat(k4_protocol::cat::set_spot(3))),
            Message::DvrPlay(n) => self.send(WorkerCmd::Cat(k4_protocol::cat::set_dvr(n))),
            Message::SetRepeaterMode(m) => {
                let off = self.ui.radio.repeater_offset_khz.unwrap_or(600);
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_repeater(m, off)));
            }
            Message::AdjustPlTone(d) => {
                let cur = self.ui.radio.pl_index.unwrap_or(1) as i16;
                let idx = (cur + d as i16).clamp(1, 50) as u8;
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_pl_tone(
                    idx,
                    self.ui.radio.pl_on == Some(true),
                )));
            }
            Message::TogglePl => {
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_pl_tone(
                    self.ui.radio.pl_index.unwrap_or(1),
                    self.ui.radio.pl_on != Some(true),
                )));
            }
            Message::ToggleArm => self.send(WorkerCmd::ArmTx(!self.ui.tx_armed)),
            // PTT keyboard hotkey (push-to-talk). trace: FR-TX-PTT-01
            Message::StartCaptureHotkey => self.capturing_hotkey = true,
            Message::TogglePttMode => {
                self.ptt_toggle = !self.ptt_toggle;
                self.save_config();
            }
            Message::ToggleModeAwareUi => {
                self.mode_aware_ui = !self.mode_aware_ui;
                self.save_config();
            }
            Message::ToggleKpod => {
                self.kpod_enabled = !self.kpod_enabled;
                self.send(WorkerCmd::SetKpodEnabled(self.kpod_enabled));
                self.save_config();
            }
            Message::KpodButtonCatChanged(idx, cat) => {
                if let Some(slot) = self.kpod_buttons.get_mut(idx) {
                    // Keep the shown label honest: adopt a matching preset's name,
                    // else blank it — a hand-typed macro has no canonical label.
                    slot.label = k4_config::KPOD_PRESETS
                        .iter()
                        .find(|p| p.cat == cat)
                        .map(|p| p.label.to_string())
                        .unwrap_or_default();
                    slot.cat = cat;
                    self.push_kpod_buttons();
                }
            }
            Message::KpodButtonPreset(idx, label) => {
                if let (Some(slot), Some(preset)) = (
                    self.kpod_buttons.get_mut(idx),
                    k4_config::KPOD_PRESETS.iter().find(|p| p.label == label),
                ) {
                    slot.label = preset.label.to_string();
                    slot.cat = preset.cat.to_string();
                    self.push_kpod_buttons();
                }
            }
            Message::KpodButtonsReset => {
                self.kpod_buttons = k4_config::default_kpod_buttons();
                self.push_kpod_buttons();
            }
            Message::KeyPressed(key, mods, window) => {
                // Emergency stop (FR-TX-SAFE-05) is checked before *everything*
                // — modals, hotkey capture, text entry. An emergency control
                // that can be swallowed by whatever has focus is not one.
                // ESC stops while on air (and so must be tested before the ESC
                // dismiss chain below); Ctrl+Shift+X always stops.
                if ui::is_estop_press(
                    &key,
                    mods,
                    ui::on_air(
                        self.ui.transmitting,
                        self.ui.tuning,
                        self.ui.radio.transmitting,
                    ),
                ) {
                    self.send(WorkerCmd::EmergencyStop);
                    return Task::none();
                }
                // ESC in the diagnostics console closes *that* window. It used
                // to fall through to the main window's dismiss chain and close
                // the Settings dialog instead, which is not where the operator
                // was looking.
                let is_esc = matches!(
                    key,
                    iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)
                );
                if is_esc && self.diag_window == Some(window) {
                    if let Some(id) = self.diag_window.take() {
                        self.diag_enabled = false;
                        self.save_config();
                        return iced::window::close(id);
                    }
                }
                // ESC dismisses an open modal (Settings / About, FR-UI-23) or
                // cancels an in-progress hotkey capture, before other key handling.
                if matches!(
                    key,
                    iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape)
                ) {
                    if self.capturing_hotkey {
                        self.capturing_hotkey = false;
                        return Task::none();
                    }
                    // The RX popup sits above the modals, so it dismisses first
                    // (FR-UI-POPUP-01).
                    if self.rx_popup.is_some() {
                        self.rx_popup = None;
                        return Task::none();
                    }
                    if self.settings_open {
                        self.settings_open = false;
                        return Task::none();
                    }
                    if self.about_open {
                        self.about_open = false;
                        return Task::none();
                    }
                }
                if self.capturing_hotkey {
                    // Ignore bare modifier presses; the next real key sets the combo.
                    if !is_modifier_key(&key) {
                        self.ptt_hotkey = hotkey_string(&key, mods);
                        self.capturing_hotkey = false;
                        self.save_config();
                    }
                } else if hotkey_string(&key, mods) == self.ptt_hotkey && !self.hotkey_down {
                    self.hotkey_down = true; // guard against key-repeat
                    if !self.ui.tx_armed {
                        self.arm_flash = 18; // blink ARM ~3× (must arm first)
                    } else if self.ptt_toggle {
                        // Toggle mode: press flips TX (like the PTT button).
                        self.send(WorkerCmd::Key(!self.ui.transmitting));
                    } else {
                        // Hold-to-talk: key down now, key up on release.
                        self.hotkey_keyed = true;
                        self.send(WorkerCmd::Key(true));
                    }
                }
            }
            Message::KeyReleased(key, _mods) => {
                // Release matches the hotkey's main key (modifiers may lift first).
                let main = self.ptt_hotkey.rsplit('+').next().unwrap_or("");
                if self.hotkey_down && key_label(&key) == main {
                    self.hotkey_down = false;
                    if self.hotkey_keyed {
                        self.hotkey_keyed = false;
                        self.send(WorkerCmd::Key(false)); // push-to-talk: key up
                    }
                }
            }
            Message::ToggleKey => self.send(WorkerCmd::Key(!self.ui.transmitting)),
            Message::EmergencyStop => self.send(WorkerCmd::EmergencyStop),
            Message::PressDown => self.press_at = Some(std::time::Instant::now()),
            Message::TapOrHold(tap, hold) => {
                // The radio's own convention: tap and hold are different
                // actions on the same control (D14 p.359). A press whose start
                // we never saw counts as a tap — holds carry the more
                // surprising action, so an unknown duration must not become one.
                let held = ui::is_hold(self.press_at.map(|t| t.elapsed()));
                self.press_at = None;
                let next = if held { hold } else { tap };
                return self.update(*next);
            }
            Message::HoverEnter(id) => {
                // Restart the dwell timer only when moving to a *different*
                // control, so re-entering the same one mid-hover does not
                // reset it.
                if self.hover.map(|(prev, _)| prev) != Some(id) {
                    self.hover = Some((id, std::time::Instant::now()));
                }
            }
            Message::HoverExit => self.hover = None,
            Message::SetTooltips(on) => {
                self.tooltips = on;
                if !on {
                    self.hover = None;
                }
                self.save_config();
            }
            Message::TxTune(action) => {
                // A tune keys the transmitter, so it is arm-gated in the
                // session like any other TX path. Flash the ARM control when
                // refused, matching the PTT-while-disarmed feedback
                // (FR-TX-PTT-01, FR-TX-SAFE-03).
                if action.transmits() && !self.ui.tx_armed {
                    self.arm_flash = 18; // blink ARM ~3x (must arm first)
                } else {
                    self.send(WorkerCmd::Tune(action));
                }
            }
            Message::AtuToggle => self.send(WorkerCmd::AtuToggle),
            Message::CatInputChanged(v) => self.cat_input = v,
            Message::SendCat => {
                let cmd = self.cat_input.trim();
                if !cmd.is_empty() {
                    self.send(WorkerCmd::SendRawCat(cmd.to_string()));
                    self.cat_input.clear();
                }
            }
            Message::SetViewMode(m) => {
                self.view_mode = m;
                // Single-A/B also picks the active RX VFO; A+B leaves it to a
                // spectrum-pane click.
                match m {
                    ViewMode::SingleA => self.active_rx_b = false,
                    ViewMode::SingleB => self.active_rx_b = true,
                    ViewMode::Dual => {}
                }
                self.sync_locals(); // load the newly-active RX VFO's slider values
            }
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
            Message::OpenUrlOwned(url) => open_url(&url),
            Message::CheckUpdate => {
                // Operator-initiated only; never on a timer. The request is
                // blocking, so it runs off the UI thread (FR-UI-UPD-01).
                self.update_status = update::UpdateStatus::Checking;
                let current = ui::app_version().to_string();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || update::check_now(&current))
                            .await
                            .unwrap_or_else(|e| {
                                update::UpdateStatus::Failed(format!("check did not run: {e}"))
                            })
                    },
                    Message::UpdateChecked,
                );
            }
            Message::UpdateChecked(status) => self.update_status = status,
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
                self.send(WorkerCmd::SetVolume(k4_audio::gain_from_level(v)));
            }
            Message::RxVolumeChanged(is_b, v) => {
                let i = usize::from(is_b);
                self.rx_volume[i] = v;
                // Moving the slider while muted sets the level to return to,
                // without unmuting — silently restoring audio because a slider
                // moved would be its own surprise.
                if !self.rx_muted[i] {
                    self.send(WorkerCmd::SetRxVolume(is_b, f32::from(v) / 100.0));
                }
                self.save_config();
            }
            Message::ToggleRxMute(is_b) => {
                let i = usize::from(is_b);
                self.rx_muted[i] = !self.rx_muted[i];
                // Mute is a gain of zero on the wire; the slider keeps its
                // value so unmuting restores the operator's level.
                let gain = if self.rx_muted[i] {
                    0.0
                } else {
                    f32::from(self.rx_volume[i]) / 100.0
                };
                self.send(WorkerCmd::SetRxVolume(is_b, gain));
            }
            Message::MicGainChanged(g) => {
                self.mic_gain = g;
                self.send(WorkerCmd::SetMicGain(g));
            }
            Message::SaveSettings => self.save_config(),
            Message::ExportConfig => self.export_config(),
            Message::SweepMenu => {
                for (id, _) in ui::menu_items() {
                    self.send(WorkerCmd::Cat(k4_protocol::cat::menu_query(*id)));
                }
                self.backup_status = format!(
                    "Querying {} menu items — wait a moment, then Export.",
                    ui::menu_items().len()
                );
            }
            Message::ImportPathChanged(v) => self.import_path = v,
            Message::LoadConfig => self.load_config_file(),
            Message::PlaybackConfig => {
                for cmd in self.loaded_commands.clone() {
                    self.send(WorkerCmd::Cat(cmd));
                }
                if !self.loaded_commands.is_empty() {
                    self.backup_status =
                        format!("Sent {} settings to the K4.", self.loaded_commands.len());
                }
            }
            Message::PowerRestart => self.send(WorkerCmd::Cat(k4_protocol::cat::set_power(8))),
            Message::PowerOffArm => self.power_off_armed = true,
            Message::PowerOffCancel => self.power_off_armed = false,
            Message::PowerOffConfirm => {
                self.send(WorkerCmd::Cat(k4_protocol::cat::set_power(0)));
                self.power_off_armed = false;
            }
            Message::ToggleShowLog => self.show_log = !self.show_log,
            Message::ToggleDiagWindow => {
                if let Some(id) = self.diag_window.take() {
                    self.diag_enabled = false;
                    self.save_config();
                    return iced::window::close(id);
                }
                let (id, open) = iced::window::open(diag_window_settings());
                self.diag_window = Some(id);
                self.diag_enabled = true;
                self.save_config();
                return open.map(|_| Message::WindowOpened);
            }
            Message::WindowOpened => {} // ids are captured at open time
            Message::WindowClosed(id) => {
                if id == self.main_window {
                    return iced::exit(); // closing the main window quits the app
                }
                if Some(id) == self.diag_window {
                    self.diag_window = None;
                    self.diag_enabled = false;
                    self.save_config();
                }
            }
            Message::ToggleLogAutoscroll => {
                self.log_autoscroll = !self.log_autoscroll;
                if self.log_autoscroll {
                    self.log_frozen.clear();
                } else {
                    // Freeze the current lines so the console holds still to be
                    // read/selected (the live buffer keeps churning underneath).
                    self.log_frozen = self.ui.diag_lines.as_ref().clone();
                }
                self.refresh_log_content();
            }
            Message::LogFilterChanged(pat) => {
                self.log_filter = pat;
                self.refresh_log_content();
            }
            Message::LogEditorAction(action) => {
                // Read-only console: allow navigation, selection, and scrolling
                // (and the widget's built-in Ctrl+C copy), but ignore edits so the
                // buffer can't be typed into.
                if !action.is_edit() {
                    self.log_content.perform(action);
                }
            }
            Message::CopyLog => {
                // Copy the currently-visible (filtered, frozen-or-live) lines to
                // the clipboard so they can be pasted elsewhere.
                let text = self.visible_log_lines().join("\n");
                return iced::clipboard::write(text);
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
            // Band select + band-stack recall target the transmit VFO (VFO B under
            // split, else A) via `target_rx`, so a band change follows the VFO you
            // operate on — not always VFO A (FR-VFO-04).
            Message::SelectBand(bn) => self.send(WorkerCmd::Cat(target_rx(
                k4_protocol::cat::set_band(bn),
                self.tx_vfo_b,
            ))),
            Message::BandStack => self.send(WorkerCmd::Cat(target_rx(
                k4_protocol::cat::band_stack_next().to_string(),
                self.tx_vfo_b,
            ))),
            Message::SetTxTab(t) => self.tx_tab = t,
            Message::Tx(t) => self.apply_tx(t),
            Message::VfoOp(op) => self.send(WorkerCmd::Cat(k4_protocol::cat::vfo_copy_swap(op))),
            Message::MenuSelect(id) => {
                // Select an item to edit: remember it, clear the entry field, and
                // query its current value (`ME<id>`) so the editor can show it.
                self.menu_selected = Some(id);
                self.menu_edit.clear();
                self.send(WorkerCmd::Cat(k4_protocol::cat::menu_query(id)));
            }
            Message::MenuEditChanged(v) => self.menu_edit = v,
            Message::MenuSet => {
                if let Some(id) = self.menu_selected {
                    let v = self.menu_edit.trim();
                    if !v.is_empty() {
                        self.send(WorkerCmd::Cat(k4_protocol::cat::menu_set(id, v)));
                        self.menu_edit.clear();
                        // Read the value back so the "current" reflects the change.
                        self.send(WorkerCmd::Cat(k4_protocol::cat::menu_query(id)));
                    }
                }
            }
            Message::MenuNudge(delta) => {
                // Step a numeric menu value by ±delta from its current read-back.
                if let Some(id) = self.menu_selected {
                    if let Some(cur) = self
                        .ui
                        .radio
                        .menu_values
                        .get(&id)
                        .and_then(|s| s.trim().parse::<i64>().ok())
                    {
                        let next = (cur + delta).to_string();
                        self.send(WorkerCmd::Cat(k4_protocol::cat::menu_set(id, &next)));
                        self.send(WorkerCmd::Cat(k4_protocol::cat::menu_query(id)));
                    }
                }
            }
            Message::Switch(code) => {
                use k4_protocol::cat;
                self.switch_flash = Some(code);
                self.switch_flash_ticks = 4; // ~0.6 s tap highlight
                match code {
                    16 => {
                        self.tune_on = !self.tune_on; // TUNE (toggle, no read-back)
                        self.send(WorkerCmd::Cat(cat::switch(16)));
                    }
                    131 => {
                        self.tune_lp_on = !self.tune_lp_on; // TUNE LP
                        self.send(WorkerCmd::Cat(cat::switch(131)));
                    }
                    // RX ANT / SUB ANT: step through only the enabled antennas
                    // (ACM/ACS mask) via AR/AR$; fall back to the switch tap if
                    // the mask isn't known.
                    70 => {
                        match next_avail_ant(self.ui.radio.rx_ant_avail, self.ui.radio.rx_antenna) {
                            Some(n) => self.send(WorkerCmd::Cat(cat::set_rx_antenna(n))),
                            None => self.send(WorkerCmd::Cat(cat::switch(70))),
                        }
                    }
                    157 => {
                        match next_avail_ant(
                            self.ui.radio.sub_ant_avail,
                            self.ui.radio.rx_antenna_sub,
                        ) {
                            Some(n) => self.send(WorkerCmd::Cat(cat::set_rx_antenna_sub(n))),
                            None => self.send(WorkerCmd::Cat(cat::switch(157))),
                        }
                    }
                    _ => self.send(WorkerCmd::Cat(cat::switch(code))),
                }
            }
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
            Message::ToggleDecode => {
                self.decode_on = !self.decode_on;
                // Mode 2 = CW RX decode (a sensible default); 0 = off. Active VFO.
                let mode = if self.decode_on { 2 } else { 0 };
                self.send(WorkerCmd::Cat(target_rx(
                    k4_protocol::cat::set_text_decode(mode, 0, 3),
                    self.active_sub(),
                )));
            }
            Message::SetFnTab(t) => self.fn_tab = t,
            Message::DxFilter(q) => self.dx_filter = q,
            Message::Rx(m) => self.apply_rx(m),
            Message::Resized(id, w, h) => {
                if id == self.main_window {
                    self.window_w = w;
                    self.window_h = h;
                }
            }
            Message::CursorMoved(x, y) => self.cursor = (x, y),
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
                    // Mute the radio's TX monitor once, so a remote session never
                    // blares the shack speaker.
                    // trace: FR-AUD-MON-01
                    if self.mute_mon && !self.mon_muted {
                        for m in 0..=2u8 {
                            self.send(WorkerCmd::Cat(k4_protocol::cat::set_monitor(m, 0)));
                        }
                        self.mon_muted = true;
                    }
                    // Keep in step with changes made directly at the K4: pull the
                    // radio state into the slider values every ~3 s, and re-query
                    // all settings every ~8 s (in case a change isn't auto-pushed).
                    self.resync_tick = self.resync_tick.wrapping_add(1);
                    if self.resync_tick.is_multiple_of(20) {
                        self.sync_locals();
                    }
                    // Poll the radio clock every ~2 s for the status strip.
                    if self.resync_tick.is_multiple_of(13) {
                        self.send(WorkerCmd::Cat("UT;".into()));
                    }
                    // `IF;` every ~500 ms, because its `t` flag is how we learn
                    // the radio is transmitting for any reason we did not
                    // originate — front-panel PTT, VOX, the K-Pod. It was
                    // otherwise only re-read in the ~5.3 s settings burst, which
                    // is far too slow for the emergency stop to depend on
                    // (FR-TX-SAFE-05): a whole tune can begin and end inside one
                    // interval. One short command twice a second is nothing
                    // beside the spectrum stream.
                    if self.resync_tick.is_multiple_of(5) {
                        self.send(WorkerCmd::Cat("IF;".into()));
                    }
                    if self.resync_tick.is_multiple_of(53) {
                        for cmd in k4_protocol::state::connect_state_seed() {
                            // Skip one-shot enables (`TM1;`) — only re-GET settings.
                            if cmd.ends_with(";") && !cmd.starts_with("TM1") {
                                self.send(WorkerCmd::Cat((*cmd).to_string()));
                            }
                        }
                    }
                } else {
                    self.seeded = false;
                    self.peer_cached = false;
                    self.power_off_armed = false;
                    self.mon_muted = false;
                    self.last_pwr_range = None;
                    self.resync_tick = 0;
                }
                // Re-sync the TX-VFO highlight only on a genuine split *transition*
                // (incl. the radio's echo of our own change or an external one). A
                // static or absent read-back leaves the optimistic value alone, so
                // it never snaps back.
                if let Some(s) = ui::adopt_on_change(&mut self.last_split, self.ui.split) {
                    self.tx_vfo_b = s;
                }
                // Keep the A / B / A+B view in sync with the K4's panadapter mode
                // (#DPM) on a genuine change — including the initial read-back after
                // connect (which lands after the one-shot seed), so the app shows
                // the receiver(s) the radio is actually streaming.
                if let Some(pm) =
                    ui::adopt_on_change(&mut self.last_pan_mode, self.ui.radio.pan_mode)
                {
                    self.view_mode = match pm {
                        1 => ui::ViewMode::SingleB,
                        2 => ui::ViewMode::Dual,
                        _ => ui::ViewMode::SingleA,
                    };
                    if self.view_mode != ui::ViewMode::Dual {
                        self.active_rx_b = self.view_mode == ui::ViewMode::SingleB;
                    }
                }
                // The K4 stores the tuning step (VT) per mode *and* per VFO, so
                // re-query each VFO's step with its mode once known / on a mode
                // change — the bare VT$; GET doesn't reliably report VFO B's.
                if let Some(m) = ui::adopt_on_change(&mut self.last_mode_a, self.ui.radio.mode_a) {
                    self.send(WorkerCmd::Cat(k4_protocol::cat::query_tune_step(
                        false,
                        md_digit(m),
                    )));
                }
                if let Some(m) = ui::adopt_on_change(&mut self.last_mode_b, self.ui.radio.mode_b) {
                    self.send(WorkerCmd::Cat(k4_protocol::cat::query_tune_step(
                        true,
                        md_digit(m),
                    )));
                }
                // Adopt the radio's TX power range on a genuine change only, so it
                // reflects the real state (incl. changes made at the K4) without a
                // stale echo snapping a just-clicked range button back.
                if let Some(r) =
                    ui::adopt_on_change(&mut self.last_pwr_range, self.ui.radio.tx_power_range)
                {
                    self.tx_pwr_range = r;
                }
                // Reconcile the optimistic VFO freq: drop it once the radio
                // confirms our value, or after a staleness timeout (~2 s) so a
                // clamped/rejected set falls back to the radio's real value.
                self.opt_vfo.reconcile(self.ui.vfo_a_hz, self.ui.vfo_b_hz);
                // Same contract for the attenuator level the operator just set.
                let atten_snap = if self.active_sub() {
                    self.ui.radio.sub_atten_db
                } else {
                    self.ui.atten_db
                };
                self.opt_atten.reconcile(atten_snap);
                // Expire the momentary switch-tap highlight.
                if self.switch_flash_ticks > 0 {
                    self.switch_flash_ticks -= 1;
                    if self.switch_flash_ticks == 0 {
                        self.switch_flash = None;
                    }
                }
                // Blink the ARM button (PTT hotkey pressed while disarmed).
                self.arm_flash = self.arm_flash.saturating_sub(1);
                // TUNE ends when transmit stops.
                if !self.ui.transmitting {
                    self.tune_on = false;
                    self.tune_lp_on = false;
                }
                // Poll the decoded-text buffer only while decode is on (~2.5 Hz),
                // so it adds no CAT traffic otherwise (FR-TXT-01).
                if self.decode_on && self.ui.phase == ui::ConnPhase::Connected {
                    self.decode_tick = self.decode_tick.wrapping_add(1);
                    if self.decode_tick.is_multiple_of(4) {
                        self.send(WorkerCmd::Cat("TB$;".into()));
                    }
                }
                // Rebuild the log editor buffer when its visible text changed;
                // while auto-scrolling this follows the newest line (see
                // `refresh_log_content`).
                //
                // Gated on the console window actually being **open**, and
                // throttled well below the 10 Hz tick. The rebuild re-shapes
                // the whole visible buffer, and under heavy CAT traffic the
                // text differs on every tick, so this ran continuously — the
                // cheap parts of the pipeline measure ~0.7 % of a core, so the
                // shaping is what costs. Nothing here is worth doing at 10 Hz
                // for a scrolling log a human is reading.
                self.log_refresh_div = self.log_refresh_div.wrapping_add(1);
                if self.diag_window.is_some()
                    && self.show_log
                    && self.log_refresh_div.is_multiple_of(3)
                {
                    self.refresh_log_content();
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        // Poll the shared snapshot ~10×/s; the UI thread never blocks on I/O. A
        // brisk tick keeps radio-round-trip changes (band, mode, freq read-back)
        // and the spectrum/meters feeling responsive.
        let tick = iced::time::every(Duration::from_millis(100)).map(|_| Message::Tick);
        // Track window size for the responsive band layout (FR-UI-12) and for
        // keeping an anchored popup on screen (FR-UI-POPUP-01).
        let resize = iced::window::resize_events()
            .map(|(id, size)| Message::Resized(id, size.width, size.height));
        // Pointer position, for anchoring a popup where it was opened.
        //
        // Deliberately **not** gated on the pointer being over a chip. That
        // was tried, to save the `view()` rebuild each of these messages
        // costs (4% → 20% of a core while the mouse moves, on an idle app),
        // and it broke the feature: the subscription only starts *after* the
        // enter event, so a pointer that arrives and stops — exactly what a
        // **hold** is — never produces a `CursorMoved`, and the popup opened
        // at the window corner with a stale (0, 0) anchor. That shipped in
        // v0.4.0.
        //
        // The overload this was trying to help was the diagnostics log
        // pipeline, which is fixed at its own source. Idle cost here is nil;
        // the cost is only paid while the mouse is actually moving.
        let cursor = iced::event::listen_with(|event, _status, _id| match event {
            iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                Some(Message::CursorMoved(position.x, position.y))
            }
            _ => None,
        });
        // Window lifecycle: quit on main-window close; track the diag window.
        let closed = iced::window::close_events().map(Message::WindowClosed);
        // Keyboard: PTT hotkey (push-to-talk) + hotkey capture.
        // `on_key_press` does not say which window the key came from, so a key
        // typed in the diagnostics console was handled as though it had been
        // pressed in the main window — ESC there closed the Settings dialog
        // (reported by the operator). `listen_with` carries the window id.
        let key_down = iced::event::listen_with(|event, _status, id| match event {
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                Some(Message::KeyPressed(key, modifiers, id))
            }
            _ => None,
        });
        let key_up = iced::keyboard::on_key_release(|k, m| Some(Message::KeyReleased(k, m)));
        Subscription::batch([tick, resize, cursor, closed, key_down, key_up])
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
    /// Step a VFO up/down by the radio's tuning rate (`VT`). When the step is
    /// known, compute the new frequency and set it optimistically (FA/FB) so the
    /// readout moves instantly; otherwise fall back to the radio's `UP/DN`.
    /// Wheel-tune from a panadapter pane (FR-PAN-10).
    ///
    /// Steps by a fraction of the *displayed span* rather than the radio's
    /// `VT` knob rate. `VT` is a front-panel setting and can be 1 MHz, so
    /// applying it here made a single stray scroll leave the band and trigger
    /// band-stack recalls that silently changed mode and DSP settings
    /// (issue #130). The VFO-panel arrows still use `VT`, which is what an
    /// operator expects of an explicit tuning control.
    fn pan_wheel(&mut self, is_b: bool, up: bool) {
        let span = self.pane_span_hz(is_b);
        let step = u64::from(k4_stream::render::pan_wheel_step_hz(span));
        let cur = if is_b {
            self.opt_vfo.b_or(self.ui.vfo_b_hz)
        } else {
            self.opt_vfo.a_or(self.ui.vfo_a_hz)
        };
        let Some(cur) = cur else { return };
        let new = if up {
            cur + step
        } else {
            cur.saturating_sub(step)
        };
        if is_b {
            self.opt_vfo.set_b(new);
            self.send(WorkerCmd::SetFreqB(new));
        } else {
            self.opt_vfo.set_a(new);
            self.send(WorkerCmd::SetFreqA(new));
        }
    }

    /// The span a pane is currently displaying, from the stream if a frame has
    /// arrived, else the `#SPN` read-back. `0` = not yet known.
    fn pane_span_hz(&self, is_b: bool) -> u32 {
        let latest = if is_b {
            &self.ui.spectrum_sub
        } else {
            &self.ui.spectrum_latest
        };
        if latest.span_hz > 0 {
            latest.span_hz
        } else if is_b {
            self.ui
                .radio
                .sub_pan_span_hz
                .or(self.ui.radio.pan_span_hz)
                .unwrap_or(0)
        } else {
            self.ui.radio.pan_span_hz.unwrap_or(0)
        }
    }

    fn step_vfo(&mut self, is_b: bool, up: bool) {
        let step = if is_b {
            self.ui.radio.sub_tune_step_hz
        } else {
            self.ui.radio.tune_step_hz
        };
        let cur = if is_b {
            self.opt_vfo.b_or(self.ui.vfo_b_hz)
        } else {
            self.opt_vfo.a_or(self.ui.vfo_a_hz)
        };
        match (step, cur) {
            (Some(step), Some(cur)) => {
                let step = u64::from(step);
                let new = if up {
                    cur + step
                } else {
                    cur.saturating_sub(step)
                };
                if is_b {
                    self.opt_vfo.set_b(new);
                    self.send(WorkerCmd::SetFreqB(new));
                } else {
                    self.opt_vfo.set_a(new);
                    self.send(WorkerCmd::SetFreqA(new));
                }
            }
            _ => {
                let cmd = match (is_b, up) {
                    (false, true) => "UP;",
                    (false, false) => "DN;",
                    (true, true) => "UPB;",
                    (true, false) => "DNB;",
                };
                self.send(WorkerCmd::Cat(cmd.to_string()));
            }
        }
    }

    /// The frequency readout as individually clickable digits (FR-VFO-08):
    /// a digit's top half increments it, the bottom half decrements it, rolling
    /// 0–9 within that digit only (no carry to neighbours).
    fn freq_digits(&self, is_b: bool, hz: Option<u64>, color: Color) -> Element<'_, Message> {
        let Some(hz) = hz else {
            return Text::new(ui::format_freq_opt(None))
                .size(38)
                .color(color)
                .into();
        };
        // The active tuning digit = the K4's tune step (VT) for this VFO. Its
        // place value gets an underline cursor, kept in sync with the radio.
        let step = if is_b {
            self.ui.radio.sub_tune_step_hz
        } else {
            self.ui.radio.tune_step_hz
        };
        let digits = hz.to_string();
        let n = digits.len();
        let lead = n % 3;
        let mut row = Row::new().align_y(Alignment::Center);
        for (i, ch) in digits.char_indices() {
            if i != 0 && i >= lead && (i - lead).is_multiple_of(3) {
                row = row.push(Text::new(".").size(38).color(color));
            }
            let place = 10u64.pow((n - 1 - i) as u32);
            // Fixed-width cell so the underline (below) reliably spans the digit
            // without a Fill-in-shrink collapse; tuned to the size-38 glyph advance.
            const DIGIT_W: f32 = 21.0;
            // Underline the digit whose place value is the current tune step.
            let underline: Element<'_, Message> = if step.map(u64::from) == Some(place) {
                Container::new(Space::new(Length::Fill, Length::Fixed(3.0)))
                    .width(Length::Fixed(DIGIT_W - 4.0))
                    .style(move |_: &Theme| container::Style {
                        background: Some(Background::Color(color)),
                        border: iced::Border {
                            radius: 1.5.into(),
                            ..Default::default()
                        },
                        ..container::Style::default()
                    })
                    .into()
            } else {
                Space::new(Length::Fixed(1.0), Length::Fixed(3.0)).into()
            };
            let cell = stack![
                Column::new()
                    .width(Length::Fixed(DIGIT_W))
                    .align_x(Alignment::Center)
                    .push(Text::new(ch.to_string()).size(38).color(color))
                    .push(underline),
                Column::new()
                    .push(
                        mouse_area(Space::new(Length::Fill, Length::Fill))
                            .on_press(Message::FreqDigit(is_b, place, true)),
                    )
                    .push(
                        mouse_area(Space::new(Length::Fill, Length::Fill))
                            .on_press(Message::FreqDigit(is_b, place, false)),
                    ),
            ];
            row = row.push(cell);
        }
        row.into()
    }

    fn vfo_panel(&self, pane: ui::Pane) -> Element<'_, Message> {
        let is_b = pane.is_b();
        // Prefer the optimistic freq (instant digit-click feedback) over the
        // radio snapshot until the radio confirms it (FR-VFO-08).
        let hz = if is_b {
            self.opt_vfo.b_or(self.ui.vfo_b_hz)
        } else {
            self.opt_vfo.a_or(self.ui.vfo_a_hz)
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

        // Per-VFO step tuning (UP/DN for A, UPB/DNB for B) at the radio's step.
        let tune_btn = |lbl: &'static str, up: bool| {
            Button::new(Text::new(lbl).size(15))
                .style(btn_style(BtnKind::Plain))
                .padding([2, 8])
                .on_press(Message::Tune(is_b, up))
        };
        let head = Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(badge(pane.label(), role))
            .push(tune_btn("◄", false))
            .push(self.freq_digits(is_b, hz, role_color(freq_role)))
            .push(tune_btn("►", true))
            .push(horizontal_space())
            // Mode is clickable: tap to step through the K4's enabled modes.
            .push(tipped(
                self.tips_on(),
                self.hover,
                "mode.cycle",
                Button::new(
                    Text::new(mode.unwrap_or("—"))
                        .size(15)
                        .color(role_color(role)),
                )
                .style(btn_style(BtnKind::Plain))
                .padding([2, 8])
                .on_press(Message::CycleMode(is_b)),
            ));

        // The transmit VFO's panel shows the TX bar graphs (RF/ALC/SWR/CMP)
        // while transmitting (FR-MTR-03); otherwise the proportional S-meter on
        // the K4's S1..S9+60 face (FR-UI-15).
        let show_tx = self.ui.transmitting && is_b == self.tx_vfo_b;
        let meter: Element<Message> = if show_tx {
            Canvas::new(meter::Meter {
                tx: true,
                s_dbm: None,
                alc: self.ui.radio.tx_alc.unwrap_or(0),
                cmp: self.ui.radio.tx_cmp.unwrap_or(0),
                fwd_w: self.ui.radio.tx_fwd_w.unwrap_or(0),
                swr_x10: self.ui.radio.tx_swr_x10.unwrap_or(0),
                show_cmp: matches!(
                    self.ui.mode_a,
                    Some("LSB") | Some("USB") | Some("AM") | Some("FM")
                ),
            })
            .width(Length::Fill)
            .height(Length::Fixed(66.0))
            .into()
        } else {
            let frac = dbm.map(ui::s_meter_fraction).unwrap_or(0.0);
            Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(
                    ProgressBar::new(0.0..=1.0, frac)
                        .height(Length::Fixed(10.0))
                        .style(meter_style(strong)),
                )
                .push(
                    // The bar to the left of this takes `Length::Fill`, so
                    // every character the reading gains is a character the bar
                    // loses — the meter visibly rescales as the signal moves,
                    // which is the one place in the app where that is actively
                    // misleading (FR-UI-STABLE-01). Reserved for the widest
                    // reading `fmt_dbm` can produce.
                    Text::new(fmt_dbm(dbm))
                        .size(12)
                        .width(Length::Fixed(ui::stable_label_width(
                            &[S_METER_WIDEST],
                            12.0,
                            4.0,
                        )))
                        .color(role_color(meter_role)),
                )
                .into()
        };

        // Fixed height matching the centre box (Fill is not allowed inside the
        // scrollable body), content vertically centred. In dual view the panel
        // is clickable to select the TX VFO and shows the accent border when it
        // is the transmit VFO — matching the spectrum panes (FR-UI-12).
        let dual = self.view_mode == ViewMode::Dual;
        let selected = dual && is_b == self.tx_vfo_b;
        let panel = Container::new(Column::new().spacing(8).push(head).push(meter))
            .style(pane_style(selected))
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fixed(VFO_BAND_H))
            .align_y(Alignment::Center);
        if dual {
            mouse_area(panel)
                .on_press(Message::SelectTxVfo(is_b))
                .into()
        } else {
            panel.into()
        }
    }

    /// The shared TX / SPLIT / RIT-XIT box that sits between the VFOs (FR-UI-12),
    /// so transmit routing is always visible. TX lights amber while transmitting.
    fn center_box(&self) -> Element<'_, Message> {
        // On air by any route, not just an open mic: a tune emits a carrier
        // without setting `transmitting` (FR-UI-TX-01).
        let txing = ui::on_air(
            self.ui.transmitting,
            self.ui.tuning,
            self.ui.radio.transmitting,
        );
        let tx_ind: Element<Message> = Container::new(
            Text::new(if txing { "● TX" } else { "TX" })
                .size(13)
                .color(if txing {
                    Color::WHITE
                } else {
                    role_color(ui::ColorRole::Inactive)
                }),
        )
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(if txing {
                // Red, not the amber used for "armed": this says RF is
                // leaving the antenna right now.
                role_color(ui::ColorRole::OnAir)
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
                    .push(tipped(
                        self.tips_on(),
                        self.hover,
                        "vfo.split",
                        two_line_btn(
                            ui::toggle_button("SPLIT", self.ui.split),
                            self.ui.split,
                            Some(Message::ToggleSplit),
                        ),
                    ))
                    .push(tipped(
                        self.tips_on(),
                        self.hover,
                        "vfo.rit",
                        two_line_btn(
                            ui::toggle_button("RIT", self.ui.rit_on),
                            self.ui.rit_on,
                            Some(Message::ToggleRit),
                        ),
                    )),
            )
            .push(
                Row::new()
                    .spacing(6)
                    .push(tipped(
                        self.tips_on(),
                        self.hover,
                        "vfo.xit",
                        two_line_btn(
                            ui::toggle_button("XIT", self.ui.xit_on),
                            self.ui.xit_on,
                            Some(Message::ToggleXit),
                        ),
                    ))
                    .push(tipped(
                        self.tips_on(),
                        self.hover,
                        "vfo.clr",
                        two_line_btn(clr_state, None, Some(Message::ClearRitXit)),
                    )),
            )
            // RIT/XIT offset readout + fine adjust (FR-VFO-05).
            .push({
                let off = self.ui.radio.rit_offset.unwrap_or(0);
                let step_btn = |lbl: &'static str, d: i16| {
                    Button::new(Text::new(lbl).size(12))
                        .style(btn_style(BtnKind::Plain))
                        .padding([2, 8])
                        .on_press(Message::AdjustRitOffset(d))
                };
                Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(step_btn("−", -10))
                    .push(
                        // The `+` step button sits immediately right of this,
                        // so an offset gaining a digit moves the control the
                        // operator is repeatedly clicking (FR-UI-STABLE-01).
                        Text::new(format!("{off:+} Hz"))
                            .size(12)
                            .width(Length::Fixed(ui::stable_label_width(
                                &["+9990 Hz"],
                                12.0,
                                4.0,
                            )))
                            .align_x(Alignment::Center)
                            .color(role_color(ui::ColorRole::RxValue)),
                    )
                    .push(step_btn("+", 10))
            });
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
                match m {
                    ViewMode::SingleA => self.active_rx_b = false,
                    ViewMode::SingleB => self.active_rx_b = true,
                    ViewMode::Dual => {}
                }
                self.sync_locals(); // active RX VFO may have changed
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
            DispMsg::Fixed(on) => cat::set_pan_fixed(on),
            DispMsg::PanNb(n) => {
                self.display.pan_nb = n.min(2);
                cat::set_pan_nb(self.display.pan_nb)
            }
            DispMsg::PanNbLevel(n) => {
                self.display.pan_nb_level = n.min(14);
                cat::set_pan_nb_level(self.display.pan_nb_level)
            }
            DispMsg::Height(h) => {
                self.display.wf_height = h.min(100);
                cat::set_waterfall_height(self.display.wf_height)
            }
        };
        // Only the commands the radio actually accepts a `$` on are targeted.
        // Established on hardware: `#SPN` is per-pan (changing it with the
        // target on A left pan B on its old span, #141), but `#REF` and `#SCL`
        // are **global** — the bare form applies to both pans, and the `$`
        // form does nothing at all, so targeting them made every DISPLAY
        // control silently inert whenever TARGET was set to B.
        let cmd = if cat_is_per_pan(&cmd) {
            target_pan(cmd, self.pan_target_b)
        } else {
            cmd
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
                // Tab is handled by the early return above; this arm only exists
                // for exhaustiveness — no-op rather than panic (audit G9).
                RxMsg::Tab(_) => return,
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
            tabs = tabs.push(tab_btn(RxTab::LineOut, "LINE OUT"));
        }
        let content: Element<Message> = match self.rx_tab {
            RxTab::Ant => self.rx_ant_panel(sub),
            RxTab::LineOut if !sub => self.line_out_panel(),
            // EQ (and LINE OUT falls back to EQ on the sub receiver).
            _ if sub => self.eq_screen(
                EqTarget::Rx,
                "Sub-RX equalizer",
                Some("Shares the RX EQ command (RE); independent sub-RX targeting pending radio verification."),
            ),
            _ => self.eq_screen(EqTarget::Rx, "RX equalizer", None),
        };
        // SUB + DIVERSITY now live in the MAIN RX chip row; the sub screen is a
        // clean single column, identical in layout to MAIN RX.
        Column::new().spacing(12).push(tabs).push(content).into()
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
        // Mode-adaptive: dim the TX config tabs the transmit mode doesn't use
        // (KEYER↔CW, MIC↔voice, LINE↔data, TEXT↔CW/data); EQ/ANT always apply.
        let tx_class = ui::ModeClass::from_mode(self.tx_mode());
        let adaptive = self.mode_aware_ui;
        let relevant = move |tab: TxTab| match tab {
            TxTab::Keyer => tx_class == ui::ModeClass::Cw,
            TxTab::Mic => matches!(
                tx_class,
                ui::ModeClass::Voice | ui::ModeClass::Am | ui::ModeClass::Fm
            ),
            TxTab::Line => tx_class == ui::ModeClass::Data,
            TxTab::Text => matches!(tx_class, ui::ModeClass::Cw | ui::ModeClass::Data),
            TxTab::Eq | TxTab::Ant => true,
        };
        let tab_btn = |tab: TxTab, label: &'static str| -> Element<Message> {
            let active = self.tx_tab == tab;
            let kind = if active {
                BtnKind::Active
            } else if adaptive && !relevant(tab) {
                BtnKind::Dim
            } else {
                BtnKind::Plain
            };
            Button::new(Text::new(label).size(12))
                .style(btn_style(kind))
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

    /// State + display label for a transmit/antenna switch by its `SW` code:
    /// antenna/ATU selections show their current value; VOX/QSK/XMIT/TUNE act as
    /// toggles (lit when engaged); the rest flash briefly on tap.
    fn sw_state(&self, label: &str, code: u16) -> (bool, String) {
        let ant = |n: Option<u8>| n.map(|v| v.to_string()).unwrap_or_else(|| "?".into());
        match code {
            16 => (self.tune_on, label.into()),     // TUNE
            131 => (self.tune_lp_on, label.into()), // TUNE LP
            50 => (self.ui.radio.vox_voice == Some(true), label.into()), // VOX
            134 => (self.ui.radio.qsk_full == Some(true), label.into()), // QSK
            // XMIT is lit while transmitting, unless a TUNE is what put us on air.
            30 => (
                self.ui.transmitting && !self.tune_on && !self.tune_lp_on,
                label.into(),
            ),
            158 => {
                // ATU: 2 = auto (in line), 1 = bypass (out).
                let in_line = self.ui.radio.atu_mode == Some(2);
                (
                    in_line,
                    format!("ATU {}", if in_line { "IN" } else { "BYP" }),
                )
            }
            60 => (false, format!("ANT {}", ant(self.ui.radio.tx_antenna))),
            70 => (false, format!("RX A {}", ant(self.ui.radio.rx_antenna))),
            157 => (
                false,
                format!("SUB A {}", ant(self.ui.radio.rx_antenna_sub)),
            ),
            _ => (self.switch_flash == Some(code), label.into()), // momentary flash
        }
    }

    /// FM sub-panel (shown in FM mode): repeater offset mode (`RP`) + PL/CTCSS
    /// tone (`PL`). trace: FR-FM-01
    /// The MAIN RX mode strip: a fixed-height row (always present, so the frame
    /// never resizes) holding mode-specific extras — SPOT + text-decode in CW,
    /// decode in DATA, the repeater/PL panel in FM (FR-UI-24). Adaptive-only
    /// extras appear only when the mode-adaptive UI is on; the FM panel shows in
    /// both modes (it has nowhere else to live).
    fn rx_mode_extras(&self, class: ui::ModeClass) -> Element<'_, Message> {
        let decode = || {
            Button::new(
                Text::new(if self.decode_on {
                    "DECODE ON"
                } else {
                    "DECODE"
                })
                .size(12),
            )
            .style(btn_style(if self.decode_on {
                BtnKind::Active
            } else {
                BtnKind::Plain
            }))
            .padding([5, 10])
            .on_press(Message::ToggleDecode)
        };
        // APF (CW peak filter) as a single-line toggle + width cycle, sized to
        // fit the strip (moved out of the chips row in Phase 3).
        let apf = || {
            tipped(
                self.tips_on(),
                self.hover,
                "rx.apf",
                popup_click(
                    ui::RxPopup::Apf,
                    Button::new(Text::new("APF").size(12))
                        .style(btn_style(if self.rx_apf_on() == Some(true) {
                            BtnKind::Active
                        } else {
                            BtnKind::Plain
                        }))
                        .padding([5, 10])
                        .on_press(Message::ToggleApf),
                ),
            )
        };
        let apf_bw = || {
            // `APF 150` is three characters wider than `APF 30`, with SPOT and
            // the decode control to its right (FR-UI-STABLE-01). The label set
            // lives beside `apf_width_label`, so a new width cannot be added
            // without the reservation following it.
            let labels: Vec<String> = ui::APF_WIDTH_LABELS
                .iter()
                .map(|w| format!("APF {w}"))
                .collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            small_btn_stable(
                format!("APF {}", ui::apf_width_label(self.rx_apf_width())),
                &refs,
                Message::CycleApfWidth,
            )
        };
        let mut row = Row::new().spacing(10).align_y(Alignment::Center);
        if self.mode_aware_ui {
            match class {
                ui::ModeClass::Cw => {
                    row = row
                        .push(apf())
                        .push(apf_bw())
                        .push(tipped(
                            self.tips_on(),
                            self.hover,
                            "cw.spot",
                            small_btn("SPOT", Message::Switch(42)),
                        ))
                        .push(decode());
                }
                ui::ModeClass::Data => {
                    // DATA sub-mode selector (DT): DATA A / AFSK A / FSK D / PSK D.
                    let cur = self.rx_data_submode();
                    for (n, label) in [(0, "DATA A"), (1, "AFSK A"), (2, "FSK D"), (3, "PSK D")] {
                        row = row.push(
                            Button::new(Text::new(label).size(11))
                                .style(btn_style(if cur == Some(n) {
                                    BtnKind::Active
                                } else {
                                    BtnKind::Plain
                                }))
                                .padding([4, 8])
                                .on_press(Message::SetDataSubmode(n)),
                        );
                    }
                    row = row.push(decode());
                }
                _ => {}
            }
        }
        if class == ui::ModeClass::Fm {
            row = row.push(self.fm_panel());
        }
        row.into()
    }

    fn fm_panel(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        let r = &self.ui.radio;
        let mode_btn = |lbl: &'static str, m: char| {
            Button::new(Text::new(lbl).size(11))
                .style(btn_style(if r.repeater_mode == Some(m) {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([4, 8])
                .on_press(Message::SetRepeaterMode(m))
        };
        let step = |lbl: &'static str, d: i8| {
            Button::new(Text::new(lbl).size(12))
                .style(btn_style(BtnKind::Plain))
                .padding([2, 8])
                .on_press(Message::AdjustPlTone(d))
        };
        Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(Text::new("RPT").size(10).color(dim))
            .push(mode_btn("S", 'S'))
            .push(mode_btn("+", '+'))
            .push(mode_btn("−", '-'))
            .push(
                Text::new(format!("{} kHz", r.repeater_offset_khz.unwrap_or(0)))
                    .size(10)
                    .color(rxv),
            )
            // The PL group used to be pushed to the far right of its own row.
            // Now that this panel shares the mode row it must stay compact, or
            // the spacer would shove PL off the end of the frame.
            .push(Space::with_width(Length::Fixed(12.0)))
            .push(
                Button::new(
                    Text::new(if r.pl_on == Some(true) {
                        "PL On"
                    } else {
                        "PL Off"
                    })
                    .size(11),
                )
                .style(btn_style(if r.pl_on == Some(true) {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([4, 8])
                .on_press(Message::TogglePl),
            )
            .push(step("−", -1))
            .push(
                Text::new(format!("{:.1} Hz", ctcss_hz(r.pl_index.unwrap_or(1))))
                    .size(10)
                    .color(rxv),
            )
            .push(step("+", 1))
            .into()
    }

    /// The K4's transmit/antenna dual-function switches (tap left / hold right,
    /// `SW` emulation) as a compact grid for the TRANSMIT panel (FR-SW-01).
    /// Selection/toggle switches reflect the live radio state (FR-UI-22-style).
    fn tx_switch_grid(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        // Mode-adaptive: dim TX controls the transmit VFO's mode doesn't use.
        let tx_class = ui::ModeClass::from_mode(self.tx_mode());
        let adaptive = self.mode_aware_ui;
        let tx_dim = move |c: ui::TxCtl| adaptive && ui::tx_ctl_vis(c, tx_class) != ui::Vis::Show;
        let cell = |label: &str, code: u16| -> Element<Message> {
            let (active, text) = self.sw_state(label, code);
            let dimmed = match code {
                50 => tx_dim(ui::TxCtl::Vox),  // VOX
                134 => tx_dim(ui::TxCtl::Qsk), // QSK
                _ => false,
            };
            let kind = if dimmed {
                BtnKind::Dim
            } else if active {
                BtnKind::Active
            } else {
                BtnKind::Plain
            };
            Button::new(Text::new(text).size(11))
                .style(btn_style(kind))
                .padding([4, 6])
                .width(Length::Fixed(92.0))
                .on_press(Message::Switch(code))
                .into()
        };
        // AUTOSPOT (SP3) sits at the end of the switch row, right of SUB A.
        let autospot = Button::new(Text::new("AUTOSPOT").size(11))
            .style(btn_style(if tx_dim(ui::TxCtl::Autospot) {
                BtnKind::Dim
            } else {
                BtnKind::Plain
            }))
            .padding([4, 8])
            .on_press(Message::Autospot);
        // All six dual-function switch pairs on one wide row, then AUTOSPOT.
        let mut switch_row = Row::new().spacing(10).align_y(Alignment::Center);
        for (tl, tap, hl, hold) in ui::tx_function_switches() {
            switch_row = switch_row.push(
                Row::new()
                    .spacing(3)
                    .push(cell(tl, *tap))
                    .push(cell(hl, *hold)),
            );
        }
        switch_row = switch_row.push(autospot);
        // MON (ML) + VOX gain (VG) + anti-VOX (VI) on one row (FR-VOX-02).
        let rxv = role_color(ui::ColorRole::RxValue);
        let vox_slider = |label: &'static str, val: u8, msg: fn(u8) -> Message, d: bool| {
            Row::new()
                .spacing(6)
                .align_y(Alignment::Center)
                .push(Text::new(label).size(10).color(dim))
                .push(
                    slider(0..=60u8, val, msg)
                        .step(1u8)
                        .width(Length::Fixed(90.0)),
                )
                .push(
                    Text::new(format!("{val}"))
                        .size(10)
                        .color(if d { dim } else { rxv }),
                )
        };
        let mon = self.ui.radio.monitor_level.unwrap_or(0);
        let mon_base = Row::new()
            .spacing(14)
            .align_y(Alignment::Center)
            .push(Text::new("MON").size(10).color(dim))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "tx.mon",
                slider(0..=100u8, mon, Message::SetMonitor)
                    .step(1u8)
                    .width(Length::Fixed(120.0)),
            ))
            // Reserved for "100": the DVR buttons now sit immediately after
            // this, so an extra digit here would move them (FR-UI-STABLE-01).
            // While this readout was the last thing in the row it could vary
            // freely, which is why it was left alone in the earlier sweep.
            .push(
                Text::new(format!("{mon}"))
                    .size(10)
                    .width(Length::Fixed(ui::stable_label_width(&["100"], 10.0, 4.0)))
                    .color(rxv),
            );
        // Compact ± stepper for the TX mode strip.
        let step_ctl = |label: &'static str,
                        val: String,
                        dn: Message,
                        up: Message|
         -> Element<'static, Message> {
            Row::new()
                .spacing(4)
                .align_y(Alignment::Center)
                .push(Text::new(label).size(10).color(dim))
                .push(
                    Button::new(Text::new("−").size(12))
                        .style(btn_style(BtnKind::Plain))
                        .padding([1, 8])
                        .on_press(dn),
                )
                .push(Text::new(val).size(10).color(rxv))
                .push(
                    Button::new(Text::new("+").size(12))
                        .style(btn_style(BtnKind::Plain))
                        .padding([1, 8])
                        .on_press(up),
                )
                .into()
        };
        let lvl = |label: &'static str, val: u8, max: u8, msg: fn(u8) -> Message| {
            Row::new()
                .spacing(6)
                .align_y(Alignment::Center)
                .push(Text::new(label).size(10).color(dim))
                .push(
                    slider(0..=max, val, msg)
                        .step(1u8)
                        .width(Length::Fixed(90.0)),
                )
                .push(Text::new(format!("{val}")).size(10).color(rxv))
        };
        // DVR voice-message playback 1–8 + STOP (FR-DVR-01).
        let dvr_full = || -> Element<'static, Message> {
            let mut r = Row::new()
                .spacing(4)
                .align_y(Alignment::Center)
                .push(Text::new("DVR").size(10).color(dim));
            for n in 1..=8u8 {
                r = r.push(
                    Button::new(Text::new(n.to_string()).size(11))
                        .style(btn_style(BtnKind::Plain))
                        .padding([2, 7])
                        .on_press(Message::DvrPlay(n)),
                );
            }
            r.push(
                Button::new(Text::new("STOP").size(10))
                    .style(btn_style(BtnKind::Danger))
                    .padding([2, 7])
                    .on_press(Message::DvrPlay(0)),
            )
            .into()
        };
        let mic_step = |c: TxConfig| {
            tipped(
                self.tips_on(),
                self.hover,
                "tx.mic",
                step_ctl(
                    "MIC",
                    format!("{}", c.mic_gain),
                    Message::Tx(TxMsg::MicGain(c.mic_gain.saturating_sub(5))),
                    Message::Tx(TxMsg::MicGain((c.mic_gain + 5).min(80))),
                ),
            )
        };
        // Mode-aware TX strip (Phase 4): the MON row keeps universal MON, then
        // per-mode gain; the action row swaps DVR ↔ CW keyer timing. Two rows in
        // every mode (height-neutral). Classic UI keeps VOX G / A-VOX + DVR.
        let c = self.tx_cfg;
        let voice_dvr = || -> Element<'static, Message> {
            Row::new()
                .spacing(12)
                .align_y(Alignment::Center)
                .push(mic_step(c))
                .push(dvr_full())
                .into()
        };
        // `levels` is the *mode-specific* part only — MON is universal and is
        // emitted separately, so the action group can sit directly after it.
        let extras = || Row::new().spacing(14).align_y(Alignment::Center);
        let (levels, action_row): (Element<'_, Message>, Element<'_, Message>) = if adaptive {
            match tx_class {
                ui::ModeClass::Cw => (
                    extras()
                        .push(tipped(
                            self.tips_on(),
                            self.hover,
                            "cw.wpm",
                            step_ctl(
                                "WPM",
                                format!("{}", c.keyer_speed),
                                Message::Tx(TxMsg::KeyerSpeed(c.keyer_speed.saturating_sub(1))),
                                Message::Tx(TxMsg::KeyerSpeed((c.keyer_speed + 1).min(100))),
                            ),
                        ))
                        .push(step_ctl(
                            "PITCH",
                            format!("{} Hz", self.cw_pitch),
                            Message::SetCwPitch(self.cw_pitch.saturating_sub(10).max(250)),
                            Message::SetCwPitch((self.cw_pitch + 10).min(950)),
                        ))
                        .into(),
                    tipped(
                        self.tips_on(),
                        self.hover,
                        "cw.qsk",
                        step_ctl(
                            "QSK DLY",
                            format!("{}", self.qsk_delay),
                            Message::SetQskDelay(self.qsk_delay.saturating_sub(1)),
                            Message::SetQskDelay(self.qsk_delay.saturating_add(1)),
                        ),
                    ),
                ),
                ui::ModeClass::Voice | ui::ModeClass::Am => (
                    extras()
                        .push(vox_slider(
                            "VOX G",
                            self.vox_gain,
                            Message::SetVoxGain,
                            false,
                        ))
                        .push(vox_slider(
                            "A-VOX",
                            self.anti_vox,
                            Message::SetAntiVox,
                            false,
                        ))
                        .push(lvl("CMP", self.compression, 30, Message::SetCompression))
                        .into(),
                    voice_dvr(),
                ),
                ui::ModeClass::Data => (
                    extras()
                        .push(vox_slider(
                            "VOX G",
                            self.vox_gain,
                            Message::SetVoxGain,
                            false,
                        ))
                        .into(),
                    Row::new().height(Length::Fixed(26.0)).into(),
                ),
                ui::ModeClass::Fm => (extras().into(), voice_dvr()),
            }
        } else {
            (
                extras()
                    .push(vox_slider(
                        "VOX G",
                        self.vox_gain,
                        Message::SetVoxGain,
                        false,
                    ))
                    .push(vox_slider(
                        "A-VOX",
                        self.anti_vox,
                        Message::SetAntiVox,
                        false,
                    ))
                    .into(),
                dvr_full(),
            )
        };
        Column::new()
            .spacing(6)
            .push(Text::new("Switches (tap · hold)").size(10).color(dim))
            .push(switch_row)
            // The action controls (DVR, or the CW keyer timing) ride on the end
            // of the level row rather than taking a row of their own. Neither
            // row filled its width — the levels ended around half way and the
            // DVR strip is short — so the second row was buying vertical space
            // with nothing in it.
            .push(
                Row::new()
                    .spacing(16)
                    .align_y(Alignment::Center)
                    // MON, then the action group, then the mode-specific
                    // levels. The action group used to come last, so the DVR
                    // buttons sat at a different x in every mode — four level
                    // controls ahead of them in voice, one in DATA, none in FM
                    // — and moved again as MON's own readout gained a digit.
                    // MON is the one control present in every mode, so placing
                    // them right after it is the only position that holds
                    // still (FR-UI-STABLE-01).
                    .push(mon_base)
                    .push(action_row)
                    .push(levels),
            )
            .into()
    }

    /// TX → TEXT sub-panel (`KY`): type a CW/DATA message and send it.
    fn tx_text_panel(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        Column::new()
            .spacing(10)
            // Decoded RX text (FR-TXT-01) — polled from the K4's TB buffer.
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(Text::new("Decoded RX text").size(12).color(rxv))
                    .push(
                        Button::new(
                            Text::new(if self.decode_on { "DECODE ON" } else { "DECODE OFF" })
                                .size(12),
                        )
                        .style(btn_style(if self.decode_on {
                            BtnKind::Active
                        } else {
                            BtnKind::Plain
                        }))
                        .padding([5, 10])
                        .on_press(Message::ToggleDecode),
                    ),
            )
            .push(
                Container::new(
                    scrollable(Text::new(self.ui.radio.decode_text.clone()).size(13).color(rxv))
                        .anchor_y(scrollable::Anchor::End)
                        .width(Length::Fill)
                        .height(Length::Fill),
                )
                .style(panel_style)
                .padding(8)
                .width(Length::Fill)
                .height(Length::Fixed(120.0)),
            )
            .push(
                Text::new("Send CW / DATA text (transmits the message)")
                    .size(12)
                    .color(rxv),
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
                Text::new("Decode: CW mode via TB polling (active only while ON). Send via KY (TX armed).")
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
            .push(tipped(
                self.tips_on(),
                self.hover,
                "cw.pitch",
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
            ))
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
    /// Mode of the active RX VFO (drives the MAIN RX frame's mode-awareness).
    fn active_mode(&self) -> Option<&'static str> {
        if self.active_rx_b {
            self.ui.mode_b
        } else {
            self.ui.mode_a
        }
    }

    /// Mode of the transmit VFO (B under split, else A) — drives the TX frame.
    fn tx_mode(&self) -> Option<&'static str> {
        if self.tx_vfo_b {
            self.ui.mode_b
        } else {
            self.ui.mode_a
        }
    }

    fn tx_mode_class(&self) -> char {
        match self.tx_mode() {
            Some("CW") | Some("CW-R") => 'C',
            Some("DATA") | Some("DATA-R") | Some("FSK") | Some("FSK-D") => 'D',
            _ => 'V',
        }
    }

    /// Whether the active RX VFO is the sub receiver (VFO B) — the RX controls
    /// target it with the `$` modifier when so. Sub is active in the single-B
    /// view; otherwise VFO A (main) is the receiver being controlled.
    fn active_sub(&self) -> bool {
        self.active_rx_b
    }

    /// Whether tooltips should show right now (FR-UI-TIP-01).
    ///
    /// Suppressed while a settings popup is open: the popup's scrim only
    /// captures presses, so pointer motion still reaches the controls beneath
    /// it, and their tooltips were drawing on top of the popup.
    ///
    /// trace: FR-UI-POPUP-01, FR-UI-TIP-01
    fn tips_on(&self) -> bool {
        self.tooltips && self.rx_popup.is_none()
    }

    /// Short label for the active RX VFO: `A` (main) or `B` (sub).
    fn active_rx_label(&self) -> &'static str {
        if self.active_sub() {
            "B"
        } else {
            "A"
        }
    }

    // Notch/APF state for the *active* RX VFO (main or sub `$` read-back).
    fn rx_notch_on(&self) -> Option<bool> {
        if self.active_sub() {
            self.ui.radio.sub_notch_on
        } else {
            self.ui.radio.notch_on
        }
    }
    fn rx_auto_notch(&self) -> Option<bool> {
        if self.active_sub() {
            self.ui.radio.sub_auto_notch
        } else {
            self.ui.radio.auto_notch
        }
    }
    fn rx_apf_on(&self) -> Option<bool> {
        if self.active_sub() {
            self.ui.radio.sub_apf_on
        } else {
            self.ui.radio.apf_on
        }
    }
    fn rx_apf_width(&self) -> Option<u8> {
        if self.active_sub() {
            self.ui.radio.sub_apf_width
        } else {
            self.ui.radio.apf_width
        }
    }

    /// Active RX's DATA sub-mode (0=DATA A, 1=AFSK A, 2=FSK D, 3=PSK D).
    fn rx_data_submode(&self) -> Option<u8> {
        if self.active_sub() {
            self.ui.radio.sub_data_submode
        } else {
            self.ui.radio.data_submode
        }
    }

    // Chip-control state for the active RX VFO (main mirror on the snapshot, sub
    // on the radio state).
    fn rx_atten_on(&self) -> Option<bool> {
        if self.active_sub() {
            self.ui.radio.sub_atten_on
        } else {
            self.ui.atten_on
        }
    }
    fn rx_atten_db(&self) -> Option<u8> {
        if self.active_sub() {
            self.ui.radio.sub_atten_db
        } else {
            self.ui.atten_db
        }
    }
    fn rx_preamp_on(&self) -> Option<bool> {
        if self.active_sub() {
            self.ui.radio.sub_preamp_on
        } else {
            self.ui.preamp_on
        }
    }
    fn rx_nb_on(&self) -> Option<bool> {
        if self.active_sub() {
            self.ui.radio.sub_nb_on
        } else {
            self.ui.nb_on
        }
    }
    fn rx_nr_on(&self) -> Option<bool> {
        if self.active_sub() {
            self.ui.radio.sub_nr_on
        } else {
            self.ui.nr_on
        }
    }
    fn rx_agc_mode(&self) -> Option<u8> {
        if self.active_sub() {
            self.ui.radio.sub_agc_mode
        } else {
            self.ui.agc_mode
        }
    }

    /// Move one passband edge (lo/hi) — derived from BW+IS — and send the
    /// resulting BW/IS pair to the active RX VFO (FR-FIL-02). The unmoved edge
    /// holds; a 50 Hz minimum width is enforced.
    fn set_passband_edge(&mut self, lo: Option<u16>, hi: Option<u16>) {
        use k4_protocol::cat;
        let (cur_lo, cur_hi) = cat::passband_edges(self.bw_hz, self.shift_hz);
        let (lo, hi) = match (lo, hi) {
            (Some(l), None) => (l.min(cur_hi.saturating_sub(50)), cur_hi),
            (None, Some(h)) => (cur_lo, h.max(cur_lo + 50)),
            _ => (cur_lo, cur_hi),
        };
        self.bw_hz = u32::from(hi - lo);
        self.shift_hz = (((u32::from(lo) + u32::from(hi)) / 2 + 5) / 10 * 10) as u16;
        let (bw_cmd, is_cmd) = cat::set_passband_edges_hz(lo, hi);
        let sub = self.active_sub();
        self.send(WorkerCmd::Cat(target_rx(bw_cmd, sub)));
        self.send(WorkerCmd::Cat(target_rx(is_cmd, sub)));
    }

    /// The mode + filter geometry for a pane: `(mode, bandwidth, IF centre
    /// pitch, CW sidetone pitch)`. `None` until the radio has reported a mode.
    ///
    /// `IS` is the IF **centre pitch** (D12), so the audio passband is
    /// `IS ± BW/2`; the mode then decides how that maps onto RF.
    fn pane_filter_geometry(&self, is_b: bool) -> Option<(k4_protocol::Mode, u32, u16, u16)> {
        let r = &self.ui.radio;
        let (mode, bw, is) = if is_b {
            (r.mode_b, r.sub_bandwidth_hz, r.sub_shift_hz)
        } else {
            (r.mode_a, r.bandwidth_hz, r.shift_hz)
        };
        // Defaults keep the overlay sane before the first BW/IS/CW read-back.
        Some((
            mode?,
            bw.unwrap_or(2_700),
            is.unwrap_or(1_500),
            r.cw_pitch.unwrap_or(600),
        ))
    }

    /// RF passband edges (absolute Hz) to shade on a pane's panadapter.
    fn pane_passband_hz(&self, is_b: bool, vfo_hz: u64) -> Option<(u64, u64)> {
        let (mode, bw, is, pitch) = self.pane_filter_geometry(is_b)?;
        Some(k4_protocol::cat::rf_passband_hz(
            vfo_hz, mode, bw, is, pitch,
        ))
    }

    /// The VFO frequency a panadapter click at `clicked_hz` should tune to.
    /// Falls back to the raw click while the mode is still unknown.
    fn pane_click_vfo(&self, is_b: bool, clicked_hz: u64) -> u64 {
        match self.pane_filter_geometry(is_b) {
            Some((mode, bw, is, pitch)) => {
                k4_protocol::cat::vfo_for_click(clicked_hz, mode, bw, is, pitch)
            }
            None => clicked_hz,
        }
    }

    /// Load the local slider values (BW/AF/RF/SQL/shift/notch-pitch) from the
    /// active RX VFO's radio state, so they reflect A or B after a view switch.
    fn sync_locals(&mut self) {
        let r = self.ui.radio.clone();
        let sub = self.active_sub();
        if let Some(v) = if sub {
            r.sub_bandwidth_hz
        } else {
            r.bandwidth_hz
        } {
            self.bw_hz = v;
        }
        if let Some(v) = if sub { r.sub_af_gain } else { r.af_gain } {
            self.af_gain = v;
        }
        if let Some(v) = if sub { r.sub_rf_gain_db } else { r.rf_gain_db } {
            self.rf_gain = v;
        }
        if let Some(v) = if sub { r.sub_squelch } else { r.squelch } {
            self.squelch = v;
        }
        if let Some(v) = if sub { r.sub_shift_hz } else { r.shift_hz } {
            self.shift_hz = v;
        }
        if let Some(v) = if sub {
            r.sub_notch_pitch
        } else {
            r.notch_pitch
        } {
            self.notch_pitch = v;
        }
        // TX sliders are not per-RX-VFO — always follow the radio. The power
        // *range* (H/L/X) is a user selection, so it is NOT re-synced here — a
        // lagging read-back would otherwise snap the range buttons back.
        if let Some(v) = r.tx_power {
            self.tx_power = v;
        }
        if let Some(v) = r.compression {
            self.compression = v;
        }
        if let Some(v) = r.cw_pitch {
            self.cw_pitch = v;
        }
        if let Some(v) = if sub { r.sub_nb_level } else { r.nb_level } {
            self.nb_level = v;
        }
        // Skipped while an optimistic level is pending: the read-back this
        // would copy may predate the operator's change (FR-UI-POPUP-01).
        if !self.opt_atten.is_pending() {
            if let Some(v) = if sub {
                r.sub_atten_db
            } else {
                self.ui.atten_db
            } {
                self.atten_db = v;
            }
        }
        if let Some(v) = if sub { r.sub_nr_level } else { r.nr_level } {
            self.nr_level = v;
        }
        if let Some(v) = r.vox_gain {
            self.vox_gain = v;
        }
        if let Some(v) = r.anti_vox {
            self.anti_vox = v;
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
                    .push(tipped(
                        self.tips_on(),
                        self.hover,
                        "tx.vox",
                        two_line_btn(
                            ui::toggle_button("VOX", Some(c.vox)),
                            Some(c.vox),
                            Some(Message::Tx(TxMsg::Vox(!c.vox))),
                        ),
                    )),
            )
            .push(tipped(
                self.tips_on(),
                self.hover,
                "tx.comp",
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
            ))
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
            let kind = if self.switch_flash == Some(*code) {
                BtnKind::Active
            } else {
                BtnKind::Plain
            };
            row = row.push(
                Button::new(Text::new(*label).size(12))
                    .style(btn_style(kind))
                    .padding([5, 10])
                    .on_press(Message::Switch(*code)),
            );
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
        // Adopt the radio's display layout (#DPM: 0=A, 1=B, 2=dual) so the app
        // starts the way the K4 is set up (e.g. both RX + both waterfalls).
        if let Some(pm) = r.pan_mode {
            self.view_mode = match pm {
                1 => ViewMode::SingleB,
                2 => ViewMode::Dual,
                _ => ViewMode::SingleA,
            };
            self.active_rx_b = self.view_mode == ViewMode::SingleB;
        }
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
        if let Some(v) = r.notch_pitch {
            self.notch_pitch = v;
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
    /// (from D12), searchable. Tap an item to select it (queries its value via
    /// `ME`); the editor below shows the current value and sets a new one
    /// (`ME<id>.<value>`) — type a value and **Set**, or nudge a number with ±.
    fn menu_config_screen(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        let items = ui::menu_items();
        let matches = ui::menu_search(&self.menu_filter);

        let mut list = Column::new().spacing(3);
        for i in &matches {
            let (id, name) = items[*i];
            let selected = self.menu_selected == Some(id);
            let mut row = Row::new()
                .spacing(10)
                .align_y(Alignment::Center)
                .push(Text::new(name).size(12).width(Length::Fixed(300.0)))
                .push(Text::new(format!("#{id:04}")).size(11).color(dim));
            if let Some(v) = self.ui.radio.menu_values.get(&id) {
                row = row.push(Text::new(format!("= {v}")).size(11).color(rxv));
            }
            list = list.push(
                Button::new(row)
                    .style(btn_style(if selected {
                        BtnKind::Active
                    } else {
                        BtnKind::Plain
                    }))
                    .width(Length::Fill)
                    .padding([4, 8])
                    .on_press(Message::MenuSelect(id)),
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

        let mut col = Column::new()
            .spacing(10)
            .push(header)
            .push(scrollable(list).height(Length::Fixed(180.0)));

        // Value editor for the selected item.
        if let Some(id) = self.menu_selected {
            let name = items
                .iter()
                .find(|(iid, _)| *iid == id)
                .map(|(_, n)| *n)
                .unwrap_or("");
            let cur = self
                .ui
                .radio
                .menu_values
                .get(&id)
                .map(String::as_str)
                .unwrap_or("…");
            col = col.push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(
                        Text::new(format!("{name}  #{id:04}"))
                            .size(12)
                            .color(rxv)
                            .width(Length::Fixed(250.0)),
                    )
                    .push(
                        Text::new(format!("now: {cur}"))
                            .size(12)
                            .color(dim)
                            .width(Length::Fixed(110.0)),
                    )
                    .push(small_btn("−", Message::MenuNudge(-1)))
                    .push(small_btn("+", Message::MenuNudge(1)))
                    .push(
                        TextInput::new("new value", &self.menu_edit)
                            .on_input(Message::MenuEditChanged)
                            .on_submit(Message::MenuSet)
                            .size(13)
                            .width(Length::Fixed(110.0)),
                    )
                    .push(small_btn("Set", Message::MenuSet)),
            );
        } else {
            col = col.push(
                Text::new("Tap an item to read its value, then set it or nudge a number with ±.")
                    .size(10)
                    .color(dim),
            );
        }
        col.into()
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
            modes = modes.push(tipped(
                self.tips_on(),
                self.hover,
                "pan.mode",
                Button::new(Text::new(m.label()).size(12))
                    .style(btn_style(if active {
                        BtnKind::Active
                    } else {
                        BtnKind::Plain
                    }))
                    .padding([6, 12])
                    .on_press(Message::Disp(DispMsg::Mode(m))),
            ));
        }
        // Per-pan target for the attribute controls (REF/SPAN/SCALE/… apply to
        // A or B via the `$` modifier) — most useful in dual view.
        let tgt_btn = |lbl: &'static str, b: bool, cur: bool| {
            Button::new(Text::new(lbl).size(11))
                .style(btn_style(if cur == b {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([4, 10])
                .on_press(Message::SetPanTarget(b))
        };
        // TARGET sits next to the PAN selector because it qualifies it: every
        // control on this screen applies to the targeted pan only, and the two
        // pans hold independent settings (#141). Across the row it read as an
        // unrelated control.
        let view_row = Row::new()
            .spacing(10)
            .align_y(Alignment::Center)
            .push(Text::new("PAN").size(11).color(dim))
            .push(modes)
            .push(tipped(
                self.tips_on(),
                self.hover,
                "pan.target",
                Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(Text::new("TARGET").size(11).color(dim))
                    .push(tgt_btn("A", false, self.pan_target_b))
                    .push(tgt_btn("B", true, self.pan_target_b)),
            ))
            .push(horizontal_space());
        let pal = ui::waterfall_palettes()[(d.wf_palette as usize).min(4)];
        let tip = |id: &'static str, w: Element<'static, Message>| {
            tipped(self.tips_on(), self.hover, id, w)
        };
        let peak = tip(
            "pan.peak",
            two_line_btn(
                ui::toggle_button("PEAK", Some(d.peak)),
                Some(d.peak),
                Some(Message::Disp(DispMsg::Peak(!d.peak))),
            ),
        );
        let freeze = tip(
            "pan.freeze",
            two_line_btn(
                ui::toggle_button("FREEZE", Some(d.freeze)),
                Some(d.freeze),
                Some(Message::Disp(DispMsg::Freeze(!d.freeze))),
            ),
        );
        // The K4 reports `#MP$-1` when the mini-pan cannot be turned on with the
        // current settings (D12 `#MP$` NOTE). Surface that as `N/A` so a refusal
        // is not indistinguishable from "off" — but keep the button live.
        // Neither D12 nor D14 documents *which* setting blocks it, so the
        // operator must be able to change something and retry; a disabled
        // button would be a dead end. The raw `#MP$` reply is in the
        // diagnostics console (rx, Debug) for confirmation.
        let minipan = tip(
            "pan.minipan",
            two_line_btn(
                if self.ui.radio.mini_pan_available == Some(false) {
                    ui::unavailable_button("MINI-PAN")
                } else {
                    ui::toggle_button("MINI-PAN", self.ui.radio.mini_pan_on)
                },
                self.ui.radio.mini_pan_on,
                Some(Message::ToggleMiniPan),
            ),
        );
        // Steppers laid out on a fixed 3-column grid so labels, −/+ buttons and
        // values align across rows and columns.
        let grid_row = || Row::new().spacing(12);
        Column::new()
            .spacing(10)
            .push(view_row)
            .push(
                grid_row()
                    .push(tip(
                        "pan.ref",
                        disp_stepper(
                            "REF",
                            format!("{} dBm", d.ref_db),
                            Message::Disp(DispMsg::Ref(d.ref_db - 5)),
                            Message::Disp(DispMsg::Ref(d.ref_db + 5)),
                        ),
                    ))
                    .push(tip(
                        "pan.span",
                        disp_stepper(
                            "SPAN",
                            format!("{:.0} kHz", f64::from(d.span_hz) / 1000.0),
                            Message::Disp(DispMsg::Span(d.span_hz / 2)),
                            Message::Disp(DispMsg::Span(d.span_hz.saturating_mul(2))),
                        ),
                    ))
                    .push(tip(
                        "pan.scale",
                        disp_stepper(
                            "SCALE",
                            d.scale.to_string(),
                            Message::Disp(DispMsg::Scale(d.scale.saturating_sub(5))),
                            Message::Disp(DispMsg::Scale(d.scale + 5)),
                        ),
                    )),
            )
            .push(
                grid_row()
                    .push(tip(
                        "pan.avg",
                        disp_stepper(
                            "AVG",
                            d.avg.to_string(),
                            Message::Disp(DispMsg::Avg(d.avg.saturating_sub(1))),
                            Message::Disp(DispMsg::Avg(d.avg + 1)),
                        ),
                    ))
                    .push(tipped(
                        self.tips_on(),
                        self.hover,
                        "pan.wfheight",
                        disp_stepper(
                            "WF HT",
                            format!("{}%", d.wf_height),
                            Message::Disp(DispMsg::Height(d.wf_height.saturating_sub(10))),
                            Message::Disp(DispMsg::Height(d.wf_height + 10)),
                        ),
                    )),
            )
            .push(
                Row::new()
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .push(peak)
                    .push(freeze)
                    .push(minipan)
                    .push(tipped(
                        self.tips_on(),
                        self.hover,
                        "pan.wfpalette",
                        small_btn_string(
                            format!("WF: {pal}"),
                            Message::Disp(DispMsg::Palette((d.wf_palette + 1) % 5)),
                        ),
                    ))
                    // Panadapter noise blanker (#NB / #NBL) — cleans the
                    // display without touching receive audio. The encoders
                    // existed and were tested; nothing reached them (#127).
                    // Fixed-tune (#FXT): the pan stops re-centring on every
                    // QSY. Driven from the radio's read-back, like the other
                    // `#`-family controls (#133).
                    .push(tip(
                        "pan.fixed",
                        two_line_btn(
                            ui::toggle_button("FIXED TUNE", self.ui.radio.pan_fixed),
                            self.ui.radio.pan_fixed,
                            Some(Message::Disp(DispMsg::Fixed(
                                !self.ui.radio.pan_fixed.unwrap_or(false),
                            ))),
                        ),
                    ))
                    .push(tip(
                        "pan.nb",
                        two_line_btn(
                            ui::pan_nb_button(d.pan_nb),
                            Some(d.pan_nb > 0),
                            Some(Message::Disp(DispMsg::PanNb((d.pan_nb + 1) % 3))),
                        ),
                    ))
                    .push(tip(
                        "pan.nblevel",
                        disp_stepper(
                            "PAN NB LVL",
                            d.pan_nb_level.to_string(),
                            Message::Disp(DispMsg::PanNbLevel(d.pan_nb_level.saturating_sub(1))),
                            Message::Disp(DispMsg::PanNbLevel(d.pan_nb_level + 1)),
                        ),
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
            grid = grid.push(tipped(
                self.tips_on(),
                self.hover,
                "vfo.band.up",
                Button::new(Text::new(*label).size(13))
                    .style(btn_style(BtnKind::Plain))
                    .padding([8, 12])
                    .on_press(Message::SelectBand(*bn)),
            ));
        }
        let ops = Row::new()
            .spacing(6)
            .align_y(Alignment::Center)
            .push(tipped(
                self.tips_on(),
                self.hover,
                "vfo.band.down",
                small_btn("BAND −", Message::Band(false)),
            ))
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

    /// The log lines currently visible in the console: the frozen snapshot while
    /// auto-scroll is off (so it holds still to be read/copied), else the live
    /// buffer, narrowed by the filter (FR-DIAG-02). Shared by the renderer and
    /// the Copy action so both show exactly the same set.
    fn visible_log_lines(&self) -> Vec<String> {
        let lines = if self.log_autoscroll {
            &self.ui.diag_lines
        } else {
            &self.log_frozen
        };
        let filter = self.log_filter.to_lowercase();
        lines
            .iter()
            .filter(|l| filter.is_empty() || l.to_lowercase().contains(&filter))
            .cloned()
            .collect()
    }

    /// Rebuild the read-only log-editor buffer from the currently-visible lines,
    /// but only when the text actually changed — so a static (frozen) view keeps
    /// the user's selection intact. While auto-scrolling, jump to the end to
    /// follow the newest line. Called on each tick and on filter/freeze changes.
    fn refresh_log_content(&mut self) {
        // Bound the buffer so a full 4 000-line ring isn't re-parsed every tick;
        // Copy still grabs the complete visible set.
        let visible = self.visible_log_lines();
        let start = visible.len().saturating_sub(2000);
        let text = visible[start..].join("\n");
        if text == self.log_text {
            return;
        }
        self.log_text = text;
        self.log_content = text_editor::Content::with_text(&self.log_text);
        if self.log_autoscroll {
            self.log_content
                .perform(text_editor::Action::Move(text_editor::Motion::DocumentEnd));
        }
    }

    /// The detached diagnostics window's content (FR-DIAG-04): the console
    /// (raw-CAT entry, LOG/AUTOSCROLL toggles, and the log) filling the window.
    fn diag_window_view(&self) -> Element<'_, Message> {
        set_active_theme(self.effective_theme());
        let dim = role_color(ui::ColorRole::Inactive);
        let visible_count = self.visible_log_lines().len();
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
            .push(
                Text::new(format!("{visible_count} lines"))
                    .size(10)
                    .color(dim),
            )
            .push(horizontal_space())
            .push(opt("LOG", self.show_log, Message::ToggleShowLog))
            .push(opt(
                "AUTOSCROLL",
                self.log_autoscroll,
                Message::ToggleLogAutoscroll,
            ))
            .push(opt("COPY", false, Message::CopyLog));
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
                    .push(small_btn("SEND", Message::SendCat))
                    .push(horizontal_space())
                    .push(
                        TextInput::new("filter log…", &self.log_filter)
                            .on_input(Message::LogFilterChanged)
                            .size(13)
                            .width(Length::Fixed(160.0)),
                    ),
            )
            .push(
                Text::new(format!(
                    "RX audio: {} decoded / {} played   spectrum: {} bins",
                    self.ui.audio_frames, self.ui.audio_played, self.ui.spectrum_bins
                ))
                .size(11)
                .color(dim),
            );
        if self.show_log {
            // A read-only text_editor so lines can be selected and copied (Ctrl+C)
            // natively; edits are ignored in the update handler. Turn AUTOSCROLL
            // off to freeze the buffer for a stable selection.
            let log = text_editor(&self.log_content)
                .on_action(Message::LogEditorAction)
                .size(11)
                .height(Length::Fill);
            diag_col = diag_col.push(log);
        }
        Container::new(diag_col)
            .style(panel_style)
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    /// Per-window title (daemon).
    fn title(&self, window: iced::window::Id) -> String {
        if Some(window) == self.diag_window {
            "K4 Remote — Diagnostics".into()
        } else {
            "K4 Remote".into()
        }
    }

    fn view(&self, window: iced::window::Id) -> Element<'_, Message> {
        // Resolve colours against the active theme for the whole tree (FR-UI-17).
        set_active_theme(self.effective_theme());
        // The detached diagnostics window renders only the console (FR-DIAG-04).
        if Some(window) == self.diag_window {
            return self.diag_window_view();
        }
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
            // Settings, theme and About all sit to the right of this, and it
            // changes on every connect and disconnect (FR-UI-STABLE-01).
            .width(Length::Fixed(ui::stable_label_width(
                &ui::CONNECT_LABELS,
                12.0,
                20.0,
            )))
            .on_press(connect_msg(conn_action));
        // Theme selector (FR-UI-17) and About (FR-UI-18), top-right; About is
        // rightmost with the theme toggle to its left.
        let theme_names: Vec<String> = ui::ThemeMode::LABELS
            .iter()
            .map(|l| format!("Theme: {l}"))
            .collect();
        let theme_labels: Vec<&str> = theme_names.iter().map(String::as_str).collect();
        let theme_btn =
            Button::new(Text::new(format!("Theme: {}", self.theme_mode.label())).size(12))
                .style(btn_style(BtnKind::Plain))
                .padding([5, 10])
                // "Theme: Contrast" against "Theme: Dark", with About to the
                // right (FR-UI-STABLE-01). Reserved from the rendered strings
                // rather than the bare names, so the "Theme: " prefix is
                // counted once, here, instead of being estimated separately.
                .width(Length::Fixed(ui::stable_label_width(
                    &theme_labels,
                    12.0,
                    20.0,
                )))
                .on_press(Message::CycleTheme);
        let about_btn = tipped(
            self.tips_on(),
            self.hover,
            "app.about",
            Button::new(Text::new("About").size(12))
                .style(btn_style(if self.about_open {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([5, 10])
                .on_press(Message::ToggleAbout),
        );
        // Settings dialog (FR-UI-23) — houses the connection form + peer cache.
        let settings_btn = tipped(
            self.tips_on(),
            self.hover,
            "app.settings",
            Button::new(Text::new("Settings").size(12))
                .style(btn_style(if self.settings_open {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([5, 10])
                .on_press(Message::ToggleSettings),
        );
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
                // "connecting..." is four characters longer than "CONNECTED",
                // so without a reservation the status message beside it slides
                // every time the phase changes (FR-UI-STABLE-01).
                Text::new(status_text)
                    .size(12)
                    .width(Length::Fixed(ui::stable_label_width(
                        &ui::CONN_STATUS_LABELS,
                        12.0,
                        4.0,
                    )))
                    .color(role_color(status_role)),
            );
        // Status strip: radio UTC clock + remote client count (FR-UI-STATUS-01).
        let mut status_bits: Vec<String> = Vec::new();
        if let Some(t) = self.ui.radio.utc_unix {
            status_bits.push(format!("{} UTC", fmt_utc_hms(t)));
        }
        if let Some(n) = self.ui.radio.client_count {
            if n > 1 {
                status_bits.push(format!("{n} clients"));
            }
        }
        // Deliberately NOT width-reserved, unlike the rest of this sweep.
        //
        // It sits *after* the `Length::Fill` status text, so it does shift the
        // controls to its right when the clock appears or a second client
        // joins — a genuine instance of what FR-UI-STABLE-01 forbids. But its
        // widest form is 27 characters, and reserving that takes ~257 px from
        // the status message, which then wraps to three lines and grows the
        // whole header, pushing every panel below it down. Measured, not
        // assumed: the reservation was written, screenshotted, and reverted.
        //
        // Trading an occasional horizontal shift for a permanent vertical one
        // is a bad trade, so this stays as it is until the status message has
        // somewhere else to go.
        let status_strip = Text::new(status_bits.join("  ·  ")).size(12).color(dim);
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
            .push(status_strip)
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
        // Mode-adaptive: dim controls the active RX mode doesn't use (Phase 1).
        let rx_class = ui::ModeClass::from_mode(self.active_mode());
        let rx_dim =
            |ctl: ui::RxCtl| self.mode_aware_ui && ui::rx_ctl_vis(ctl, rx_class) != ui::Vis::Show;
        let chips = Row::new()
            .spacing(6)
            .push(tipped(
                self.tips_on(),
                self.hover,
                "filter.bw",
                two_line_btn_dim(
                    ui::bandwidth_button(Some(self.bw_hz)),
                    None,
                    Some(Message::CycleBandwidth),
                    rx_dim(ui::RxCtl::Bw),
                ),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.atten",
                // Tap = in/out; hold (or right-click) brings up the
                // attenuator controls, which is what the radio does —
                // "Hold [ATTN] to bring up the attenuator controls (on/off
                // and level)" (D14 p.1318).
                popup_click(
                    ui::RxPopup::Atten,
                    tap_hold(
                        Message::ToggleAtten,
                        Message::OpenRxPopup(ui::RxPopup::Atten),
                        two_line_btn_visual(
                            ui::atten_button(self.rx_atten_on(), self.rx_atten_db()),
                            self.rx_atten_on(),
                            false,
                        ),
                    ),
                ),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.preamp",
                popup_click(
                    ui::RxPopup::Preamp,
                    tap_hold(
                        Message::TogglePreamp,
                        Message::OpenRxPopup(ui::RxPopup::Preamp),
                        two_line_btn_visual(
                            ui::preamp_button(self.rx_preamp_on(), self.ui.radio.preamp_level),
                            self.rx_preamp_on(),
                            false,
                        ),
                    ),
                ),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.nb",
                // Tap = on/off; hold brings up the noise-blanker controls —
                // "hold [LEVEL] to bring up the noise blanker controls
                // (on/off, filtering mode, and level)" (D14 p.1368). The app
                // draws one control where the radio has two switches, so the
                // hold carries the paired switch's function.
                popup_click(
                    ui::RxPopup::Nb,
                    tap_hold(
                        Message::ToggleNb,
                        Message::OpenRxPopup(ui::RxPopup::Nb),
                        two_line_btn_visual(
                            ui::nb_button(self.rx_nb_on(), self.ui.radio.nb_filter),
                            self.rx_nb_on(),
                            false,
                        ),
                    ),
                ),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.nr",
                popup_click(
                    ui::RxPopup::Nr,
                    tap_hold(
                        Message::ToggleNr,
                        Message::OpenRxPopup(ui::RxPopup::Nr),
                        two_line_btn_visual(
                            ui::toggle_button("NR", self.rx_nr_on()),
                            self.rx_nr_on(),
                            false,
                        ),
                    ),
                ),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.agc",
                // Tap = slow/fast (D14 p.909); hold brings up the AGC panel,
                // where switching it **off** now lives — a tap still cannot
                // reach off by accident.
                popup_click(
                    ui::RxPopup::Agc,
                    tap_hold(
                        Message::CycleAgc,
                        Message::OpenRxPopup(ui::RxPopup::Agc),
                        two_line_btn_visual(
                            ui::agc_button(self.rx_agc_mode()),
                            None,
                            rx_dim(ui::RxCtl::Agc),
                        ),
                    ),
                ),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.subrx",
                two_line_btn(
                    ui::toggle_button("SUB", self.ui.radio.sub_rx),
                    self.ui.radio.sub_rx,
                    Some(Message::ToggleSubRx),
                ),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.diversity",
                two_line_btn(
                    ui::toggle_button("DIV", self.ui.radio.diversity),
                    self.ui.radio.diversity,
                    Some(Message::ToggleDiversity),
                ),
            ))
            // Notch / APF for the active RX VFO, right of DIV.
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.notch",
                popup_click(
                    ui::RxPopup::Notch,
                    tap_hold(
                        Message::ToggleManualNotch,
                        Message::OpenRxPopup(ui::RxPopup::Notch),
                        two_line_btn_visual(
                            ui::toggle_button("NOTCH", self.rx_notch_on()),
                            self.rx_notch_on(),
                            rx_dim(ui::RxCtl::ManualNotch),
                        ),
                    ),
                ),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.autonotch",
                two_line_btn_dim(
                    ui::toggle_button("AUTO NCH", self.rx_auto_notch()),
                    self.rx_auto_notch(),
                    Some(Message::ToggleAutoNotch),
                    rx_dim(ui::RxCtl::AutoNotch),
                ),
            ));
        // APF (CW audio peak filter) is CW-only — it lives in the CW mode strip
        // (see rx_mode_strip), not the always-visible chips row (Phase 3).
        let mode_btn = |label: &'static str, digit: u8| -> Element<'_, Message> {
            let active = self.ui.mode_a == Some(label);
            // Re-tapping the active mode switches to its alternate — the
            // reverse or opposite sideband, as the radio pairs them (CW ⇄
            // CW-R, LSB ⇄ USB, DATA ⇄ DATA-R). AM and FM have no partner, so
            // they keep re-selecting themselves, which is a harmless no-op.
            let press = match (active, ui::alternate_of(label)) {
                (true, Some(alt)) => Message::SetMode(alt),
                _ => Message::SetMode(digit),
            };
            tipped(
                self.tips_on(),
                self.hover,
                "mode.select",
                Button::new(Text::new(label).size(12))
                    .style(btn_style(if active {
                        BtnKind::Active
                    } else {
                        BtnKind::Plain
                    }))
                    .padding([6, 10])
                    .on_press(press),
            )
        };
        let tune_row = Row::new()
            .spacing(6)
            .align_y(Alignment::Center)
            .push(mode_btn("LSB", 1))
            .push(mode_btn("USB", 2))
            .push(mode_btn("CW", 3))
            .push(mode_btn("CW-R", 7))
            .push(mode_btn("DATA", 6))
            .push(mode_btn("DATA-R", 9))
            .push(mode_btn("AM", 5))
            .push(mode_btn("FM", 4))
            .push(small_btn("BAND −", Message::Band(false)))
            .push(small_btn("BAND +", Message::Band(true)))
            .push(
                // SCAN (SW149) — lit while a scan is in progress (IF `s` flag). FR-SCAN-01.
                Button::new(Text::new("SCAN").size(12))
                    .style(btn_style(if self.ui.radio.scanning == Some(true) {
                        BtnKind::Active
                    } else {
                        BtnKind::Plain
                    }))
                    .padding([6, 10])
                    .on_press(Message::Switch(149)),
            )
            // Filter presets for the active RX VFO, right of SCAN.
            .push(tipped(
                self.tips_on(),
                self.hover,
                "filter.preset",
                small_btn_dim(
                    "FL1".into(),
                    Message::FilterPreset(1),
                    rx_dim(ui::RxCtl::FilterPresets),
                ),
            ))
            .push(small_btn_dim(
                "FL2".into(),
                Message::FilterPreset(2),
                rx_dim(ui::RxCtl::FilterPresets),
            ))
            .push(small_btn_dim(
                "FL3".into(),
                Message::FilterPreset(3),
                rx_dim(ui::RxCtl::FilterPresets),
            ))
            .push(small_btn_dim(
                "NORMALIZE".into(),
                Message::FilterNormalize,
                rx_dim(ui::RxCtl::FilterPresets),
            ))
            // Mode-specific extras ride on the end of this row rather than
            // occupying a row of their own (FR-UI-24). The strip used to be a
            // reserved fixed-height row that stayed empty in SSB and AM —
            // vertical space paid for in every mode to avoid the frame
            // resizing in three. Sharing this row costs nothing when empty and
            // still cannot resize the frame, because the row is one line high
            // either way.
            .push(self.rx_mode_extras(rx_class));
        // AF/RF gain + squelch sliders for the main receiver (FR-RX-01,
        // FR-RX-SQL-01) — the K4's RF/SQL knob, plus radio-side AF.
        let rxv = role_color(ui::ColorRole::RxValue);
        // Slider value colour: bright normally, grey when de-emphasised (the
        // label is already dim, so greying the value is the mode-adaptive cue).
        let vcol = |d: bool| if d { dim } else { rxv };
        let gain = |label: &'static str,
                    val: u8,
                    max: u8,
                    msg: fn(u8) -> Message,
                    unit: &'static str,
                    d: bool|
         -> Element<Message> {
            Row::new()
                .spacing(6)
                .align_y(Alignment::Center)
                .push(Text::new(label).size(11).color(dim))
                // 96 px, not 110: reserving a fixed width for the readouts
                // (FR-UI-STABLE-01) costs ~156 px across this row, and at 110
                // the row overflowed — the trailing SHIFT readout was squeezed
                // to one character per line. The sliders give up what the
                // readouts need.
                .push(slider(0..=max, val, msg).width(Length::Fixed(96.0)))
                .push(
                    // Reserve the widest reading this slider can produce. Taken
                    // from the same `max` the slider is built from, so the two
                    // cannot drift apart (FR-UI-STABLE-01) — every one of these
                    // sits mid-row with more controls to its right, so a digit
                    // appearing shifts the rest of the strip.
                    Text::new(format!("{val}{unit}"))
                        .size(11)
                        .width(Length::Fixed(ui::stable_label_width(
                            &[&format!("{max}{unit}")],
                            11.0,
                            4.0,
                        )))
                        .color(vcol(d)),
                )
                .into()
        };
        // Third row: SHIFT, then AF / RF / SQL, then PITCH — all for the active VFO.
        let hz_slider =
            |label: &'static str, val: u16, lo: u16, hi: u16, msg: fn(u16) -> Message, d: bool| {
                Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(Text::new(label).size(11).color(dim))
                    .push(
                        slider(lo..=hi, val, msg)
                            .step(10u16)
                            .width(Length::Fixed(96.0)),
                    )
                    .push(
                        Text::new(format!("{val} Hz"))
                            .size(11)
                            .width(Length::Fixed(ui::stable_label_width(
                                &[&format!("{hi} Hz")],
                                11.0,
                                4.0,
                            )))
                            .color(vcol(d)),
                    )
            };
        let level_slider = |label: &'static str, val: u8, max: u8, msg: fn(u8) -> Message| {
            Row::new()
                .spacing(6)
                .align_y(Alignment::Center)
                .push(Text::new(label).size(11).color(dim))
                .push(
                    slider(0..=max, val, msg)
                        .step(1u8)
                        .width(Length::Fixed(84.0)),
                )
                .push(
                    Text::new(format!("{val}"))
                        .size(11)
                        .width(Length::Fixed(ui::stable_label_width(
                            &[&format!("{max}")],
                            11.0,
                            4.0,
                        )))
                        .color(rxv),
                )
        };
        // Filter view toggles the SHIFT slider for LO/HI-cut edges (FR-FIL-02);
        // it sits in the gain row, right of NR LVL.
        let (lo_edge, hi_edge) = k4_protocol::cat::passband_edges(self.bw_hz, self.shift_hz);
        let shift_dim = rx_dim(ui::RxCtl::ShiftHiLo);
        let shift_kind = if shift_dim {
            BtnKind::Dim
        } else if self.filter_edge_view {
            BtnKind::Active
        } else {
            BtnKind::Plain
        };
        let mut filter_ctl = Row::new().spacing(10).align_y(Alignment::Center).push(
            Button::new(
                Text::new(if self.filter_edge_view {
                    "HI/LO"
                } else {
                    "SHFT"
                })
                .size(11),
            )
            .style(btn_style(shift_kind))
            .padding([5, 8])
            // First item of the filter group: the label, slider and readout to
            // its right all move when this toggles (FR-UI-STABLE-01).
            .width(Length::Fixed(ui::stable_label_width(
                &["HI/LO", "SHFT"],
                11.0,
                16.0,
            )))
            .on_press(Message::ToggleFilterEdgeView),
        );
        filter_ctl = if self.filter_edge_view {
            filter_ctl
                .push(hz_slider(
                    "LO",
                    lo_edge,
                    0,
                    3000,
                    Message::SetLoCut,
                    shift_dim,
                ))
                .push(hz_slider(
                    "HI",
                    hi_edge,
                    100,
                    5000,
                    Message::SetHiCut,
                    shift_dim,
                ))
        } else {
            filter_ctl.push(tipped(
                self.tips_on(),
                self.hover,
                "filter.shift",
                hz_slider(
                    "SHIFT",
                    self.shift_hz,
                    200,
                    3000,
                    Message::SetShift,
                    shift_dim,
                ),
            ))
        };
        // Gain row: AF / RF / SQL / PITCH, the NB / NR level sliders, then SHIFT.
        let gain_row = Row::new()
            .spacing(14)
            .align_y(Alignment::Center)
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.afgain",
                gain("AF", self.af_gain, 60, Message::SetAfGain, "", false),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.rfgain",
                gain("RF", self.rf_gain, 60, Message::SetRfGain, " dB", false),
            ))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "rx.squelch",
                gain(
                    "SQL",
                    self.squelch,
                    40,
                    Message::SetSquelch,
                    "",
                    rx_dim(ui::RxCtl::Squelch),
                ),
            ))
            .push(hz_slider(
                "NOTCH",
                self.notch_pitch,
                150,
                5000,
                Message::SetNotchPitch,
                rx_dim(ui::RxCtl::ManualNotch),
            ))
            .push(level_slider(
                "NB LVL",
                self.nb_level,
                15,
                Message::SetNbLevel,
            ))
            .push(level_slider(
                "NR LVL",
                self.nr_level,
                10,
                Message::SetNrLevel,
            ))
            .push(filter_ctl);
        let controls = Container::new(
            Column::new()
                .spacing(8)
                .push(
                    Row::new()
                        .spacing(10)
                        .align_y(Alignment::Center)
                        // Frame reflects (and controls) the active RX VFO: RX A / RX B.
                        .push(
                            Text::new(format!("RX {}", self.active_rx_label()))
                                .size(11)
                                .color(dim),
                        )
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
            // Each pane draws its own receiver's trace/waterfall (FR-PAN-02).
            let (latest, waterfall) = if p.is_b() {
                (&self.ui.spectrum_sub, &self.ui.waterfall_sub)
            } else {
                (&self.ui.spectrum_latest, &self.ui.waterfall)
            };
            let pane_center_hz = if p.is_b() {
                self.ui.vfo_b_hz
            } else {
                self.ui.vfo_a_hz
            }
            .unwrap_or(0);
            // Pan geometry comes from the stream itself (FR-PAN-06), which is
            // authoritative and stays right under fixed-tune where the pan
            // centre and the VFO diverge. Fall back to the VFO + `#SPN` only
            // until the first frame arrives.
            let pan_center_hz = if latest.center_hz > 0 {
                latest.center_hz as u64
            } else {
                pane_center_hz
            };
            // `PanRow::span_hz` is the span its bins cover *after* cropping to
            // `#SPN`, so it is the display span, not the streamed tier
            // (FR-PAN-08). Fall back to `#SPN` until the first frame arrives.
            let pan_span_hz = if latest.span_hz > 0 {
                latest.span_hz
            } else if p.is_b() {
                self.ui
                    .radio
                    .sub_pan_span_hz
                    .or(self.ui.radio.pan_span_hz)
                    .unwrap_or(0)
            } else {
                self.ui.radio.pan_span_hz.unwrap_or(0)
            };
            // Vertical window follows the radio: `#REF` (dBm at the bottom of
            // the scale) + `#SCL` (dB shown). The K4's read-back is
            // authoritative; the local DISPLAY-screen values are only the
            // fallback until it reports (FR-PAN-07). Previously these were
            // hardcoded to a −130…−30 window and the tracked #REF/#SCL were
            // never consulted at all.
            let (top_dbm, range_db) = k4_stream::render::pan_window(
                self.ui.radio.pan_ref.unwrap_or(self.display.ref_db),
                self.ui
                    .radio
                    .pan_scale
                    .unwrap_or(u16::from(self.display.scale)),
            );
            let plot: Element<Message> = if latest.bins.is_empty() {
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
                    latest: &latest.bins,
                    waterfall,
                    top_dbm,
                    range_db,
                    is_b: p.is_b(),
                    center_hz: pan_center_hz,
                    span_hz: pan_span_hz,
                    vfo_hz: pane_center_hz,
                    passband_hz: self.pane_passband_hz(p.is_b(), pane_center_hz),
                    on_qsy: Message::PaneQsy,
                    on_wheel: Message::PaneWheel,
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
            // This receiver's **local** listening level, in the space beside
            // the badge (FR-RX-VOL-01). RX audio arrives as 12 kHz stereo with
            // main on the left channel and sub on the right (FR-AUD-04), so
            // the two can be balanced against each other here without touching
            // the radio's own AF gain — nothing done here reaches the front
            // panel or another connected client.
            let is_b = p.is_b();
            let vol = self.rx_volume[usize::from(is_b)];
            let muted = self.rx_muted[usize::from(is_b)];
            header = header.push(tipped(
                self.tips_on(),
                self.hover,
                "pan.rxvolume",
                Row::new()
                    .spacing(6)
                    .align_y(Alignment::Center)
                    .push(
                        Text::new("VOL")
                            .size(11)
                            .color(role_color(ui::ColorRole::Inactive)),
                    )
                    .push(
                        slider(0..=100u8, vol, move |v| Message::RxVolumeChanged(is_b, v))
                            .width(Length::Fixed(110.0)),
                    )
                    .push(
                        // Reserved for "100%" so the row does not shift as the
                        // level changes (FR-UI-STABLE-01).
                        Text::new(format!("{vol}%"))
                            .size(11)
                            .width(Length::Fixed(ui::stable_label_width(&["100%"], 11.0, 4.0)))
                            .color(role_color(ui::ColorRole::RxValue)),
                    )
                    .push(
                        // Mute keeps the level, so unmuting returns to where
                        // the operator had it rather than to some default.
                        tipped(
                            self.tips_on(),
                            self.hover,
                            "pan.rxmute",
                            Button::new(Text::new(if muted { "MUTED" } else { "MUTE" }).size(10))
                                .style(btn_style(if muted {
                                    BtnKind::Amber
                                } else {
                                    BtnKind::Plain
                                }))
                                .padding([3, 7])
                                .on_press(Message::ToggleRxMute(is_b)),
                        ),
                    ),
            ));
            // Meters live in the top VFO panels now, not in the panadapter.
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
        let band_inner: Element<Message> = if bl.stacked {
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
        // Mini-pan overview: a single full-width framed strip above the whole
        // band (FR-UI-14) — one wide-span overview, not per-VFO. Always present
        // (like the spectrum frames) so the layout never shifts; shows a
        // placeholder until the 0x03 stream is on.
        let mini_inner: Element<Message> = if self.ui.mini_pan.is_empty() {
            Container::new(
                Text::new("mini-pan — enable on the DISPLAY screen")
                    .size(11)
                    .color(dim),
            )
            .center_y(Length::Fill)
            .width(Length::Fill)
            .into()
        } else {
            Canvas::new(spectrum::MiniPan {
                latest: &self.ui.mini_pan,
                top_dbm: -30.0,
                range_db: 100.0,
            })
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
        };
        let mini_frame = Container::new(mini_inner)
            .style(pane_style(false))
            .padding(8)
            .width(Length::Fill)
            .height(Length::Fixed(MINI_PAN_H));
        // The slot below the mini-pan shows a menu screen when a primary
        // softkey is active, and the spectrum otherwise (FR-UI-19). The
        // mini-pan frame stays above it either way: it is a tuning aid, and
        // an operator adjusting a setting on a screen is exactly when they
        // want to keep watching the band (FR-UI-14).
        let below: Element<Message> = match self.context.active() {
            Some(active) => self.menu_screen(active),
            None => band_inner,
        };
        let panadapter_slot: Element<Message> = Column::new()
            .spacing(10)
            .push(mini_frame)
            .push(below)
            .into();

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
        // Blink the ARM button (~3×) when the PTT hotkey is pressed while
        // disarmed, to cue the operator to arm first.
        let arm_blink = self.arm_flash > 0 && (self.arm_flash / 3) % 2 == 1;
        let arm_kind = if arm_blink {
            BtnKind::Danger
        } else if self.ui.tx_armed {
            BtnKind::Amber
        } else {
            BtnKind::Ptt
        };
        // The safety row's worst offender: "ARM TX" against the old
        // "TX ARMED — DISARM" was 6 characters against 17, so arming shoved
        // PTT and EMERGENCY STOP sideways — the two controls you least want
        // moving under a cursor that may be reaching for them.
        //
        // Reserving the wider label's space was the first attempt, but that
        // rests on *estimating* rendered width from a character count, and
        // this label contained an em dash — about twice the width the estimate
        // assumes. So the reservation could be too small and the button would
        // grow regardless.
        //
        // The button keeps its informative armed label and is simply made wide
        // enough for it, with the short label centred in the same space. The
        // width comes from the longest member of the set, so it cannot be got
        // wrong by editing one label and forgetting the other.
        // (FR-UI-STABLE-01)
        const ARM_LABELS: [&str; 2] = ["ARM TX", "TX ARMED — DISARM"];
        let arm = Button::new(
            Text::new(if self.ui.tx_armed {
                ARM_LABELS[1]
            } else {
                ARM_LABELS[0]
            })
            .size(13)
            .center(),
        )
        .style(btn_style(arm_kind))
        .width(Length::Fixed(ui::stable_label_width(
            &ARM_LABELS,
            13.0,
            20.0,
        )))
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
                // The one control in this row still moving its neighbours:
                // EMERGENCY STOP is immediately to the right, and it must not
                // slide out from under a hand reaching for it (FR-UI-STABLE-01).
                .width(Length::Fixed(ui::stable_label_width(
                    &["UNKEY", "PTT"],
                    13.0,
                    20.0,
                )))
                .on_press(Message::ToggleKey);
        let estop = Button::new(Text::new("EMERGENCY STOP").size(13))
            .style(btn_style(BtnKind::Danger))
            .padding([6, 10])
            .on_press(Message::EmergencyStop);
        // ATU + TUNE (FR-ATU-01, FR-TX-TUNE-01). These sit with the transmit
        // controls because every TUNE action but exit puts the radio on air.
        // `AT` reports 0 = not installed, so the tuner controls only appear
        // when the radio says a KAT4 is fitted (D12 `AT` NOTE).
        let atu_fitted = !matches!(self.ui.radio.atu_mode, Some(0));
        let tuning = self.ui.tuning;
        let atu = Button::new(
            Text::new(match self.ui.radio.atu_mode {
                Some(2) => "ATU AUTO",
                Some(1) => "ATU BYP",
                _ => "ATU",
            })
            .size(13),
        )
        .style(btn_style(if self.ui.radio.atu_mode == Some(2) {
            BtnKind::Active
        } else {
            BtnKind::Plain
        }))
        // Sized for the widest of its three labels (FR-UI-STABLE-01).
        .width(Length::Fixed(ui::stable_label_width(
            &["ATU", "ATU BYP", "ATU AUTO"],
            13.0,
            20.0,
        )))
        .padding([6, 10])
        .on_press(Message::AtuToggle);
        // Tapping TUNE while a tune is running stops it — the same control
        // must always be able to take the radio off air.
        let tune_btn =
            Button::new(Text::new(if tuning { "STOP TUNE" } else { "ATU TUNE" }).size(13))
                .style(btn_style(if tuning {
                    BtnKind::Amber
                } else {
                    BtnKind::Plain
                }))
                .width(Length::Fixed(ui::stable_label_width(
                    &["ATU TUNE", "STOP TUNE"],
                    13.0,
                    20.0,
                )))
                .padding([6, 10])
                .on_press(Message::TxTune(if tuning {
                    k4_protocol::cat::TuneAction::Exit
                } else {
                    k4_protocol::cat::TuneAction::AtuTune
                }));
        // PA/PWR controls: power range H (QRO) / L (QRP) / X (mW) + PWR slider
        // (FR-TX-02). Seeded from the radio.
        let range_btn = |lbl: &'static str, r: char, cur: char| {
            Button::new(Text::new(lbl).size(10))
                .style(btn_style(if cur == r {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([2, 6])
                .on_press(Message::SetTxPowerRange(r))
        };
        let pmax = if self.tx_pwr_range == 'H' { 110 } else { 100 };
        let pval = match self.tx_pwr_range {
            'H' => format!("{} W", self.tx_power),
            'X' => format!("{:.1} mW", f32::from(self.tx_power) / 10.0),
            _ => format!("{:.1} W", f32::from(self.tx_power) / 10.0),
        };
        // Row 1: the transmit controls (ARM / PTT / E-STOP), then ATU + TUNE
        // when a tuner is fitted, then the PA/PWR controls to their right.
        let mut transmit_row = Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(tipped(self.tips_on(), self.hover, "tx.arm", arm))
            .push(tipped(self.tips_on(), self.hover, "tx.ptt", key))
            .push(tipped(self.tips_on(), self.hover, "tx.estop", estop));
        if atu_fitted {
            transmit_row = transmit_row
                .push(tipped(self.tips_on(), self.hover, "atu.mode", atu))
                .push(tipped(self.tips_on(), self.hover, "atu.tune", tune_btn));
        }
        let transmit_row = transmit_row
            .push(horizontal_space())
            .push(Text::new("PWR").size(11).color(dim))
            .push(range_btn("H", 'H', self.tx_pwr_range))
            .push(range_btn("L", 'L', self.tx_pwr_range))
            .push(range_btn("X", 'X', self.tx_pwr_range))
            .push(tipped(
                self.tips_on(),
                self.hover,
                "tx.power",
                slider(0..=pmax, self.tx_power, Message::SetTxPower).width(Length::Fixed(110.0)),
            ))
            .push(
                Text::new(pval)
                    .size(11)
                    .color(role_color(ui::ColorRole::RxValue)),
            );
        // Order: transmit + PA/PWR controls · switches (+ MON/VOX · DVR).
        let tx_panel = Container::new(
            Column::new()
                .spacing(8)
                .push(Text::new("TRANSMIT").size(11).color(dim))
                .push(transmit_row)
                .push(self.tx_switch_grid()),
        )
        .style(panel_style)
        .padding(12)
        .width(Length::Fill);

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
        let mut conn_options = Row::new().spacing(6).push(small_btn_pair(
            self.serial_mode,
            "ETHERNET",
            "SERIAL",
            Message::ToggleSerialMode,
        ));
        if !self.serial_mode {
            conn_options = conn_options
                .push(tipped(
                    self.tips_on(),
                    self.hover,
                    "conn.tls",
                    Button::new(Text::new("TLS").size(12))
                        .style(btn_style(if self.use_tls {
                            BtnKind::Active
                        } else {
                            BtnKind::Plain
                        }))
                        .padding([4, 10])
                        .on_press(Message::ToggleTls),
                ))
                .push(tipped(
                    self.tips_on(),
                    self.hover,
                    "conn.remember",
                    Button::new(Text::new("REMEMBER").size(12))
                        .style(btn_style(if self.remember {
                            BtnKind::Active
                        } else {
                            BtnKind::Plain
                        }))
                        .padding([4, 10])
                        .on_press(Message::ToggleRemember),
                ));
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
            .push(tipped(
                self.tips_on(),
                self.hover,
                "conn.connect",
                Button::new(Text::new(connect_label).size(12))
                    .style(btn_style(connect_btn_kind))
                    .padding([5, 10])
                    .on_press(connect_press),
            ))
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
            .push(Text::new("K4 settings backup").size(12).color(dim))
            .push(self.backup_section_view())
            .push(Text::new("K-Pod function switches").size(12).color(dim))
            .push(self.kpod_buttons_view())
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
            Container::new(scrollable(
                // Inset the content so the scrollbar doesn't overlap it.
                Container::new(settings_inner).padding(iced::Padding {
                    top: 0.0,
                    right: 16.0,
                    bottom: 0.0,
                    left: 0.0,
                }),
            ))
            .style(panel_style)
            .padding(18)
            .width(Length::Fixed(500.0))
            .max_height(720.0)
            .into(),
        );

        // The diagnostics console lives in its own window now (FR-DIAG-04), so
        // the transmit panel stretches across the whole bottom row.
        let bottom: Element<Message> = tx_panel.into();

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
        // The RX settings popup (FR-UI-POPUP-01) sits above both, matching the
        // order ESC dismisses them in.
        if let Some(p) = self.rx_popup {
            stack![content, self.rx_popup_overlay(p)].into()
        } else if self.settings_open {
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
                    .push(small_btn_pair(set, "UNLOCK", "SET", Message::UnlockMaster)),
            );
        }
        if !self.peer_status.is_empty() {
            col = col.push(Text::new(self.peer_status.clone()).size(11).color(dim));
        }
        col.into()
    }

    /// Settings → audio: RX/TX device selection (FR-AUD-DEV-01) and local
    /// volume / mic-gain sliders (FR-AUD-LVL-01).
    /// Settings → K4 backup: export the radio's settings to a hashed `.cfg`, and
    /// load + play one back (FR-CFG-06).
    fn backup_section_view(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let mut col = Column::new()
            .spacing(6)
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(small_btn("Export K4 settings", Message::ExportConfig))
                    .push(small_btn("Sweep menu", Message::SweepMenu))
                    .push(
                        Text::new("→ K4-<serial>-<time>.cfg, SHA-256 verified")
                            .size(10)
                            .color(dim),
                    ),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(
                        TextInput::new(".cfg path to import", &self.import_path)
                            .on_input(Message::ImportPathChanged)
                            .size(13)
                            .width(Length::Fixed(300.0)),
                    )
                    .push(small_btn("Load", Message::LoadConfig))
                    .push(small_btn("Play → K4", Message::PlaybackConfig)),
            );
        if !self.backup_status.is_empty() {
            col = col.push(
                Text::new(self.backup_status.clone())
                    .size(10)
                    .color(role_color(ui::ColorRole::RxValue)),
            );
        }
        col.into()
    }

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
                        slider(0..=100u8, self.volume, Message::VolumeChanged)
                            .on_release(Message::SaveSettings)
                            .width(Length::Fixed(240.0)),
                    )
                    .push(Text::new(format!("{}%", self.volume)).size(11).color(dim)),
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
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(small_btn_pair(
                        self.mute_mon,
                        "Mute radio MON on connect: ON",
                        "Mute radio MON on connect: OFF",
                        Message::ToggleMuteMon,
                    ))
                    .push(
                        Text::new("keeps the shack speaker quiet during remote TX")
                            .size(10)
                            .color(dim),
                    ),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(tipped(
                        self.tips_on(),
                        self.hover,
                        "app.diag",
                        small_btn_pair(
                            self.diag_window.is_some(),
                            "Diagnostics window: ON",
                            "Diagnostics window: OFF",
                            Message::ToggleDiagWindow,
                        ),
                    ))
                    .push(
                        Text::new("show the CAT/diagnostics console in a separate window")
                            .size(10)
                            .color(dim),
                    ),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(small_btn_pair(
                        self.capturing_hotkey,
                        "PTT hotkey: press keys…",
                        "Set PTT hotkey",
                        Message::StartCaptureHotkey,
                    ))
                    .push(small_btn_pair(
                        self.ptt_toggle,
                        "Mode: Toggle",
                        "Mode: Hold",
                        Message::TogglePttMode,
                    ))
                    .push(
                        Text::new(format!("push-to-talk: {}", self.ptt_hotkey))
                            .size(10)
                            .color(dim),
                    ),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(small_btn_pair(
                        self.mode_aware_ui,
                        "Mode-adaptive UI: ON",
                        "Mode-adaptive UI: OFF",
                        Message::ToggleModeAwareUi,
                    ))
                    .push(
                        Text::new("dim/hide controls that don't apply to the current mode")
                            .size(10)
                            .color(dim),
                    ),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(tipped(
                        self.tips_on(),
                        self.hover,
                        "app.tooltips",
                        small_btn(
                            if self.tooltips {
                                "Control tooltips: ON"
                            } else {
                                "Control tooltips: OFF"
                            },
                            Message::SetTooltips(!self.tooltips),
                        ),
                    ))
                    .push(
                        Text::new("explain each control, and name the CAT command behind it")
                            .size(10)
                            .color(dim),
                    ),
            )
            .push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(small_btn_pair(
                        self.kpod_enabled,
                        "K-Pod: ON",
                        "K-Pod: OFF",
                        Message::ToggleKpod,
                    ))
                    .push(
                        Text::new(
                            "Elecraft K-Pod USB tuning knob — rocker picks VFO A/B/RIT, \
                             knob tunes (safe if not connected)",
                        )
                        .size(10)
                        .color(dim),
                    ),
            )
            .into()
    }

    /// Settings → K-Pod function-switch macros (FR-KPOD-06): 16 rows (F1–F8 ×
    /// tap/hold), each a preset picker that fills the slot plus a free-form CAT
    /// field, with a reset-to-Elecraft-samples action. The app sends the slot's
    /// CAT string to the K4 when that switch is pressed.
    fn kpod_buttons_view(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let accent = role_color(ui::ColorRole::VfoA);
        let presets: Vec<k4_config::KpodPreset> = k4_config::KPOD_PRESETS.to_vec();
        let mut col = Column::new().spacing(4).push(
            Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(
                    Text::new("Tap/hold F1–F8 send a CAT macro to the K4")
                        .size(10)
                        .color(dim),
                )
                .push(horizontal_space())
                .push(small_btn("Reset to samples", Message::KpodButtonsReset)),
        );
        for (idx, slot) in self.kpod_buttons.iter().enumerate() {
            // The picker fills the slot from a preset (shown as "label — desc");
            // the CAT field is always free-form. The label tag names the current
            // assignment at a glance (blank for a hand-typed macro).
            let label_tag = if slot.label.is_empty() {
                Text::new("—").size(11).color(dim)
            } else {
                Text::new(slot.label.clone()).size(11).color(accent)
            };
            let row = Row::new()
                .spacing(6)
                .align_y(Alignment::Center)
                .push(
                    Text::new(k4_config::kpod_slot_name(idx))
                        .size(11)
                        .color(dim)
                        .width(Length::Fixed(52.0)),
                )
                .push(
                    pick_list(presets.clone(), None::<k4_config::KpodPreset>, move |p| {
                        Message::KpodButtonPreset(idx, p.label.to_string())
                    })
                    .placeholder("preset")
                    .text_size(11)
                    .width(Length::Fixed(84.0)),
                )
                .push(label_tag.width(Length::Fixed(56.0)))
                .push(
                    TextInput::new("CAT e.g. MD3;BW0040;", &slot.cat)
                        .on_input(move |t| Message::KpodButtonCatChanged(idx, t))
                        .size(11)
                        .width(Length::Fill),
                );
            col = col.push(row);
        }
        col.into()
    }

    /// The RX chip settings popup (FR-UI-POPUP-01).
    ///
    /// This is the panel the radio opens on a hold — "hold [LEVEL] to bring up
    /// the noise blanker controls (on/off, filtering mode, and level)" (D14
    /// p.1368) — reached here by right-clicking the chip. Each popup carries
    /// exactly the paired switch's settings, so nothing here is a new
    /// capability: it is the adjustment that previously had nowhere to live.
    fn rx_popup_overlay(&self, p: ui::RxPopup) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let rxv = role_color(ui::ColorRole::RxValue);
        // One of a set of mutually exclusive choices (filter mode, AGC, width).
        let choice = |label: String, active: bool, msg: Message| -> Element<Message> {
            Button::new(Text::new(label).size(12))
                .style(btn_style(if active {
                    BtnKind::Active
                } else {
                    BtnKind::Plain
                }))
                .padding([5, 10])
                .on_press(msg)
                .into()
        };
        // A labelled slider with its value, laid out like the RX gain rows.
        let level = |label: &'static str,
                     val: u8,
                     max: u8,
                     unit: &'static str,
                     msg: fn(u8) -> Message|
         -> Element<Message> {
            Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(Text::new(label).size(11).color(dim))
                .push(slider(0..=max, val, msg).width(Length::Fixed(160.0)))
                .push(Text::new(format!("{val}{unit}")).size(12).color(rxv))
                .into()
        };
        let on_off = |on: Option<bool>, msg: Message| -> Element<Message> {
            choice(
                match on {
                    Some(true) => "ON".into(),
                    Some(false) => "OFF".into(),
                    None => ui::UNKNOWN.to_string(),
                },
                on == Some(true),
                msg,
            )
        };
        let body: Element<Message> = match p {
            ui::RxPopup::Atten => {
                // The slider detents on the radio's own 3 dB ladder so it
                // cannot be dragged to a level the K4 would quantise away
                // under the operator. `SetAttenDb` snaps regardless — the
                // guard belongs on the send path, not on this widget.
                //
                // Reads the local mirror, not the radio state: the read-back
                // lags a drag, and following it made the slider jump back to
                // the level the radio last reported.
                let db = self.atten_db;
                Row::new()
                    .spacing(12)
                    .align_y(Alignment::Center)
                    .push(Text::new("LEVEL").size(11).color(dim))
                    .push(
                        slider(0..=ui::ATTEN_MAX_DB, db, Message::SetAttenDb)
                            .step(ui::ATTEN_STEP_DB)
                            // One query when the drag ends, not one per pixel:
                            // the chip follows the radio again promptly without
                            // the read-back fighting the drag.
                            .on_release(Message::QueryAtten)
                            .width(Length::Fixed(160.0)),
                    )
                    .push(Text::new(format!("{db} dB")).size(12).color(rxv))
                    .push(choice("OUT".into(), db == 0, Message::SetAttenDb(0)))
                    .into()
            }
            ui::RxPopup::Preamp => {
                let cur = if self.rx_preamp_on() == Some(true) {
                    self.ui.radio.preamp_level.unwrap_or(0)
                } else {
                    0
                };
                let mut row = Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(Text::new("GAIN").size(11).color(dim))
                    .push(choice("OFF".into(), cur == 0, Message::SetPreampLevel(0)));
                for n in 1..=3u8 {
                    row = row.push(choice(format!("{n}"), cur == n, Message::SetPreampLevel(n)));
                }
                row.into()
            }
            ui::RxPopup::Agc => {
                let cur = self.rx_agc_mode();
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(Text::new("MODE").size(11).color(dim))
                    .push(choice("OFF".into(), cur == Some(0), Message::SetAgcMode(0)))
                    .push(choice(
                        "SLOW".into(),
                        cur == Some(1),
                        Message::SetAgcMode(1),
                    ))
                    .push(choice(
                        "FAST".into(),
                        cur == Some(2),
                        Message::SetAgcMode(2),
                    ))
                    .into()
            }
            ui::RxPopup::Nb => {
                // On/off, filtering mode, and level — the three the manual
                // names for this panel (D14 p.1368).
                let f = self.ui.radio.nb_filter;
                let mut modes = Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(Text::new("FILTER").size(11).color(dim));
                for n in 0..=2u8 {
                    modes = modes.push(choice(
                        ui::nb_filter_label(Some(n)).to_string(),
                        f == Some(n),
                        Message::SetNbFilter(n),
                    ));
                }
                Column::new()
                    .spacing(10)
                    .push(
                        Row::new()
                            .spacing(12)
                            .align_y(Alignment::Center)
                            .push(on_off(self.rx_nb_on(), Message::ToggleNb))
                            .push(modes),
                    )
                    .push(level("LEVEL", self.nb_level, 15, "", Message::SetNbLevel))
                    .into()
            }
            ui::RxPopup::Nr => Row::new()
                .spacing(12)
                .align_y(Alignment::Center)
                .push(on_off(self.rx_nr_on(), Message::ToggleNr))
                .push(level("LEVEL", self.nr_level, 10, "", Message::SetNrLevel))
                .into(),
            ui::RxPopup::Notch => Row::new()
                .spacing(12)
                .align_y(Alignment::Center)
                .push(on_off(self.rx_notch_on(), Message::ToggleManualNotch))
                .push(
                    Row::new()
                        .spacing(8)
                        .align_y(Alignment::Center)
                        .push(Text::new("PITCH").size(11).color(dim))
                        .push(
                            slider(150..=5000u16, self.notch_pitch, Message::SetNotchPitch)
                                .step(10u16)
                                .width(Length::Fixed(160.0)),
                        )
                        .push(
                            Text::new(format!("{} Hz", self.notch_pitch))
                                .size(12)
                                .color(rxv),
                        ),
                )
                .into(),
            ui::RxPopup::Apf => {
                let w = self.rx_apf_width();
                let mut row = Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(on_off(self.rx_apf_on(), Message::ToggleApf))
                    .push(Text::new("BW").size(11).color(dim));
                for n in 0..=2u8 {
                    row = row.push(choice(
                        format!("{} Hz", ui::apf_width_label(Some(n))),
                        w == Some(n),
                        Message::SetApfWidth(n),
                    ));
                }
                row.into()
            }
        };
        // The K4 dismisses every popup with a curved-arrow button (Intro p.20);
        // ESC and a click outside do the same here.
        // A fixed gap, not `horizontal_space()`: a Fill-width spacer would make
        // the whole card stretch across the window instead of shrink-wrapping
        // its controls.
        let header = Row::new()
            .spacing(12)
            .align_y(Alignment::Center)
            .push(Text::new(p.title()).size(12).color(dim))
            .push(Space::with_width(Length::Fixed(24.0)))
            .push(
                // ASCII: the bundled font has no U+21A9 curved arrow, which
                // renders as a tofu box.
                Button::new(Text::new("X").size(13))
                    .style(btn_style(BtnKind::Plain))
                    .padding([3, 9])
                    .on_press(Message::CloseRxPopup),
            );
        let card = Container::new(Column::new().spacing(10).push(header).push(body))
            .style(panel_style)
            .padding(14);
        // Clicks on the card itself must not reach the click-away scrim.
        let card = MouseArea::new(card).on_press(Message::Noop);
        // Anchor at the pointer position captured when the popup was opened,
        // pulled back inside the window if it would overhang. Positioned with
        // leading spacers because iced 0.13 has no absolute placement outside
        // a custom overlay widget.
        let (ox, oy) = ui::popup_origin(self.rx_popup_at, p.size(), (self.window_w, self.window_h));
        let anchored = Column::new()
            .push(Space::with_height(Length::Fixed(oy)))
            .push(
                Row::new()
                    .push(Space::with_width(Length::Fixed(ox)))
                    .push(card),
            );
        MouseArea::new(
            Container::new(anchored)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_t: &Theme| container::Style {
                    // Lighter than the modal scrim: this is a transient
                    // adjustment panel, not a dialog that owns the window.
                    background: Some(Background::Color(Color {
                        a: 0.35,
                        ..Color::BLACK
                    })),
                    ..container::Style::default()
                }),
        )
        .on_press(Message::CloseRxPopup)
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
        // Update check (FR-UI-UPD-01): a button, and whatever the last check
        // found. A newer version is shown as its version number, linking
        // straight to that release's download page.
        let update_row: Element<Message> = match &self.update_status {
            update::UpdateStatus::Checking => Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(Text::new("Checking for updates…").size(12).color(dim))
                .into(),
            update::UpdateStatus::Available { version, url } => Row::new()
                .spacing(6)
                .align_y(Alignment::Center)
                .push(Text::new("Update available:").size(12).color(dim))
                .push(
                    Button::new(Text::new(version.clone()).size(13).color(accent))
                        .style(move |_t: &Theme, status: button::Status| button::Style {
                            background: None,
                            text_color: match status {
                                button::Status::Hovered | button::Status::Pressed => Color::WHITE,
                                _ => accent,
                            },
                            ..button::Style::default()
                        })
                        .padding(0)
                        .on_press(Message::OpenUrlOwned(url.clone())),
                )
                .into(),
            other => {
                let (label, note) = match other {
                    update::UpdateStatus::UpToDate => ("Check for updates", "Up to date"),
                    update::UpdateStatus::Failed(e) => ("Check for updates", e.as_str()),
                    _ => ("Check for updates", ""),
                };
                let mut r = Row::new().spacing(8).align_y(Alignment::Center).push(
                    Button::new(Text::new(label).size(12))
                        .style(btn_style(BtnKind::Plain))
                        .padding([5, 10])
                        .on_press(Message::CheckUpdate),
                );
                if !note.is_empty() {
                    r = r.push(Text::new(note.to_string()).size(12).color(dim));
                }
                r.into()
            }
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
            .push(update_row)
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

/// Height of the always-present mini-pan overview frame (FR-UI-14).
const MINI_PAN_H: f32 = 56.0;

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
    /// De-emphasised (mode-adaptive UI): recessed, dim text; still clickable.
    Dim,
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
            BtnKind::Dim => (
                shade(ui::Shade::Track),
                role_color(ui::ColorRole::Inactive),
                shade(ui::Shade::Panel),
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
/// Wrap a control so that resting the pointer on it for
/// [`tips::TOOLTIP_DELAY`] shows its tip (FR-UI-TIP-01).
///
/// A no-op when tooltips are switched off, or when no tip has been written for
/// `id` — an un-written tip must render as nothing, not an empty bubble.
///
/// iced 0.13's `Tooltip` has no delay of its own, so the dwell is tracked here:
/// a `mouse_area` reports enter/exit, and the tooltip is only built once the
/// pointer has been still long enough. The 100 ms UI tick re-renders, so the
/// observed delay is 500–600 ms.
fn tipped<'a>(
    enabled: bool,
    hover: Option<(&'static str, std::time::Instant)>,
    id: &'static str,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let content = content.into();
    if !enabled {
        return content;
    }
    let Some(text) = tips::tip(id) else {
        return content;
    };
    let dwelt =
        matches!(hover, Some((h, since)) if h == id && since.elapsed() >= tips::TOOLTIP_DELAY);
    let inner: Element<'a, Message> = if dwelt {
        Tooltip::new(
            content,
            Container::new(Text::new(text).size(11))
                .padding([4, 8])
                .style(|theme: &Theme| {
                    let p = theme.extended_palette();
                    container::Style {
                        background: Some(Background::Color(p.background.weak.color)),
                        border: iced::Border {
                            color: p.background.strong.color,
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        ..container::Style::default()
                    }
                }),
            tooltip::Position::Bottom,
        )
        .into()
    } else {
        content
    };
    MouseArea::new(inner)
        .on_enter(Message::HoverEnter(id))
        .on_exit(Message::HoverExit)
        .into()
}

/// A two-line button rendered as a **visual only**, for use inside
/// [`tap_hold`].
///
/// An iced `Button` with an `on_press` captures both press and release, and
/// `MouseArea` delegates to its content first and returns early when the
/// content captured — so a wrapped button swallows the very events the
/// tap/hold timing needs. The tap fired from the button's own handler and the
/// hold could never happen at all.
///
/// Dropping `on_press` stops the capture, but iced then reports
/// `Status::Disabled`, which our style greys out. The style here maps
/// `Disabled` back to `Active` so a fully interactive control does not look
/// dead. The trade-off is that iced no longer reports `Hovered` for these
/// three buttons, so they lose their hover tint; the tooltip hover still
/// works, since that is driven by `MouseArea`.
fn two_line_btn_visual(
    state: ui::ButtonState,
    on: Option<bool>,
    dim: bool,
) -> Element<'static, Message> {
    let engaged = on == Some(true);
    let kind = if dim {
        BtnKind::Dim
    } else if engaged {
        BtnKind::Active
    } else {
        BtnKind::Plain
    };
    let label_color = if engaged && !dim {
        Color::WHITE
    } else {
        role_color(ui::ColorRole::Inactive)
    };
    let value = Text::new(state.value).size(13);
    let value = if dim {
        value.color(role_color(ui::ColorRole::Inactive))
    } else {
        value
    };
    let content = Column::new()
        .align_x(Alignment::Center)
        .push(Text::new(state.label).size(10).color(label_color))
        .push(value);
    let inner = btn_style(kind);
    Button::new(content)
        .style(move |t: &Theme, status: button::Status| {
            let status = match status {
                button::Status::Disabled => button::Status::Active,
                other => other,
            };
            inner(t, status)
        })
        .width(Length::Fixed(66.0))
        .padding([4, 6])
        .into()
}

/// Give a control the radio's tap/hold pair (FR-UI-HOLD-01).
///
/// Every K4 switch carries a white tap function and a yellow hold function
/// (D14 p.16); this reproduces that on a control the app already draws. The
/// inner widget keeps its own `on_press` for the visual affordance, but the
/// action is decided here on release, from how long the button was down.
fn tap_hold<'a>(
    tap: Message,
    hold: Message,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    MouseArea::new(content)
        .on_press(Message::PressDown)
        .on_release(Message::TapOrHold(Box::new(tap), Box::new(hold)))
        .into()
}

/// Give a chip a **right-click** that opens its settings popup
/// (FR-UI-POPUP-01).
///
/// Safe to wrap around [`tap_hold`] or a plain `Button`: a `MouseArea` only
/// captures the events it has a handler for, and both of those handle the
/// left button alone, so the right press falls through to this wrapper.
///
/// trace: FR-UI-POPUP-01
fn popup_click<'a>(
    popup: ui::RxPopup,
    content: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    MouseArea::new(content)
        .on_right_press(Message::OpenRxPopup(popup))
        .into()
}

fn two_line_btn(
    state: ui::ButtonState,
    on: Option<bool>,
    msg: Option<Message>,
) -> Element<'static, Message> {
    two_line_btn_dim(state, on, msg, false)
}

/// [`two_line_btn`] with an optional de-emphasis (mode-adaptive UI): when `dim`,
/// the chip is greyed but stays clickable.
fn two_line_btn_dim(
    state: ui::ButtonState,
    on: Option<bool>,
    msg: Option<Message>,
    dim: bool,
) -> Element<'static, Message> {
    // The pure layer decides engaged vs. inactive (FR-UI-10); the view maps
    // "engaged" to the reference client's blue fill.
    let engaged = on.map(ui::toggle_role) == Some(ui::ColorRole::VfoB);
    let kind = if dim {
        BtnKind::Dim
    } else if engaged {
        BtnKind::Active
    } else {
        BtnKind::Plain
    };
    let label_color = if engaged && !dim {
        Color::WHITE
    } else {
        role_color(ui::ColorRole::Inactive)
    };
    let value = Text::new(state.value).size(13);
    let value = if dim {
        value.color(role_color(ui::ColorRole::Inactive))
    } else {
        value
    };
    let content = Column::new()
        .align_x(Alignment::Center)
        .push(Text::new(state.label).size(10).color(label_color))
        .push(value);
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

/// Small plain action button that shows one of **two** known labels, sized to
/// the wider so the row does not reflow when it flips (`FR-UI-STABLE-01`).
///
/// The two-label case is common enough — every ON/OFF setting toggle — that
/// spelling out a reservation at each site invites someone to skip it.
fn small_btn_pair(
    on: bool,
    on_label: &'static str,
    off_label: &'static str,
    msg: Message,
) -> Element<'static, Message> {
    Button::new(Text::new(if on { on_label } else { off_label }).size(12))
        .style(btn_style(BtnKind::Plain))
        .padding([5, 10])
        .width(Length::Fixed(ui::stable_label_width(
            &[on_label, off_label],
            12.0,
            20.0,
        )))
        .on_press(msg)
        .into()
}

/// Small plain action button whose label varies over a **known set**, sized to
/// the widest member so the row does not reflow when it changes
/// (`FR-UI-STABLE-01`).
///
/// `labels` must list every string this button can show; passing only the
/// current one silently reintroduces the resizing. Where the set comes from a
/// function, keep the list beside that function (see `ui::CONNECT_LABELS`)
/// rather than writing it out here.
fn small_btn_stable(label: String, labels: &[&str], msg: Message) -> Element<'static, Message> {
    Button::new(Text::new(label).size(12))
        .style(btn_style(BtnKind::Plain))
        .padding([5, 10])
        .width(Length::Fixed(ui::stable_label_width(labels, 12.0, 20.0)))
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

/// Small action button, de-emphasised (`BtnKind::Dim`) when `dim` — for the
/// mode-adaptive UI. Still clickable.
fn small_btn_dim(label: String, msg: Message, dim: bool) -> Element<'static, Message> {
    Button::new(Text::new(label).size(12))
        .style(btn_style(if dim { BtnKind::Dim } else { BtnKind::Plain }))
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
/// The widest reading [`fmt_dbm`] can produce, for width reservation.
///
/// `S9+60dB` is the top of the S-unit ladder and `-121 dBm` the bottom of the
/// range, so no single real reading is this wide — which is the point: the
/// reservation has to cover the widest *unit* and the widest *number*
/// independently, because which pairs actually occur is the radio's business.
const S_METER_WIDEST: &str = "S9+60dB (-121 dBm)";

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

/// CTCSS tone table (Hz) for PL index 1–50 (K4 Programmer's Ref D12).
const CTCSS_HZ: [f32; 50] = [
    67.0, 69.3, 71.9, 74.4, 77.0, 79.7, 82.5, 85.4, 88.5, 91.5, 94.8, 97.4, 100.0, 103.5, 107.2,
    110.9, 114.8, 118.8, 123.0, 127.3, 131.8, 136.5, 141.3, 146.2, 151.4, 156.7, 159.8, 162.2,
    165.5, 167.9, 171.3, 173.8, 177.3, 179.9, 183.5, 186.2, 189.9, 192.8, 196.6, 199.5, 203.5,
    206.5, 210.7, 218.1, 225.7, 229.1, 233.6, 241.8, 250.3, 254.1,
];

/// Format a Unix timestamp as a UTC `HH:MM:SS` time-of-day.
fn fmt_utc_hms(unix: u64) -> String {
    let s = unix % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

/// PL/CTCSS tone frequency (Hz) for a 1–50 table index.
fn ctcss_hz(index: u8) -> f32 {
    CTCSS_HZ
        .get(usize::from(index.max(1) - 1))
        .copied()
        .unwrap_or(0.0)
}

/// Display label for a keyboard key (`Space`, `A`, `F1`, …).
fn key_label(key: &iced::keyboard::Key) -> String {
    match key {
        iced::keyboard::Key::Named(n) => format!("{n:?}"),
        iced::keyboard::Key::Character(c) => c.to_uppercase(),
        iced::keyboard::Key::Unidentified => "?".to_string(),
    }
}

/// Whether a key is a bare modifier (ignored when capturing a hotkey).
fn is_modifier_key(key: &iced::keyboard::Key) -> bool {
    matches!(
        key_label(key).as_str(),
        "Control" | "Shift" | "Alt" | "Super" | "Meta" | "Hyper"
    )
}

/// Canonical hotkey string, e.g. `Ctrl+Space` / `Ctrl+Shift+A`.
fn hotkey_string(key: &iced::keyboard::Key, mods: iced::keyboard::Modifiers) -> String {
    let mut s = String::new();
    if mods.control() {
        s.push_str("Ctrl+");
    }
    if mods.alt() {
        s.push_str("Alt+");
    }
    if mods.shift() {
        s.push_str("Shift+");
    }
    if mods.logo() {
        s.push_str("Super+");
    }
    s.push_str(&key_label(key));
    s
}

/// `MD`/`MD$` digit for a mode.
fn md_digit(m: k4_protocol::state::Mode) -> u8 {
    use k4_protocol::state::Mode::*;
    match m {
        Lsb => 1,
        Usb => 2,
        Cw => 3,
        Fm => 4,
        Am => 5,
        Data => 6,
        CwRev => 7,
        DataRev => 9,
    }
}

/// UTC timestamp `YYYYMMDDHHMMSS` for config-export filenames (civil-from-days).
fn timestamp_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (h, mi, s) = (secs / 3600 % 24, secs / 60 % 60, secs % 60);
    let z = (secs / 86400) as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}{h:02}{mi:02}{s:02}")
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

/// Next enabled RX-antenna value after `cur`, cycling within an `AR$`-value
/// availability bitmask (bit `v` = value `v` enabled). `None` if no mask / none
/// enabled — the caller then falls back to the raw switch tap.
fn next_avail_ant(avail: Option<u8>, cur: Option<u8>) -> Option<u8> {
    let mask = avail?;
    if mask == 0 {
        return None;
    }
    let cur = cur.unwrap_or(0);
    (1..=8u8)
        .map(|step| (cur + step) % 8)
        .find(|&v| mask & (1 << v) != 0)
}

/// Retarget a `#`-panadapter command at the sub pan (VFO B) by inserting the
/// `$` modifier after the 4-char `#XXX` mnemonic (e.g. `#REF-020;` →
/// `#REF$-020;`). Unknown-to-a-pan commands are ignored by the K4, so this is
/// safe for attributes that lack a per-pan variant.
/// Whether a `#`-family command accepts the `$` sub-pan modifier.
///
/// Deliberately an allow-list of what has been **observed to work**, not a
/// deny-list of what is known to break. D12's naming is not a reliable guide
/// here — it documents the reference level as `#REF$`, yet on a real radio the
/// bare `#REF` is what takes effect (for both pans) and the `$` form is inert.
///
/// | Command | Behaviour | Evidence |
/// |---|---|---|
/// | `#SPN` | per pan | changing it with TARGET on A left pan B unchanged (#141) |
/// | `#REF`, `#SCL` | global, `$` inert | only take effect with TARGET on A, and then apply to both pans |
/// | everything else | unverified — treated as global | — |
///
/// Anything unverified stays off the list: sending a bare command that turns
/// out to be per-pan sets the wrong pan, which is visible and recoverable;
/// sending a `$` form the radio ignores makes the control silently do nothing,
/// which is what this fixes.
///
/// trace: FR-PAN-CTL-01
fn cat_is_per_pan(cmd: &str) -> bool {
    cmd.starts_with("#SPN")
}

fn target_pan(cmd: String, sub: bool) -> String {
    if sub && cmd.starts_with('#') && cmd.len() >= 4 && !cmd[4..].starts_with('$') {
        format!("{}${}", &cmd[..4], &cmd[4..])
    } else {
        cmd
    }
}

/// Retarget a 2-letter RX command at the sub receiver by inserting the `$`
/// modifier after the mnemonic (e.g. `BW0270;` → `BW$0270;`) when `sub` is set.
/// Commands already carrying `$`, or shorter than 2 chars, pass through.
fn target_rx(cmd: String, sub: bool) -> String {
    if sub && cmd.len() >= 2 && &cmd[..1] != "#" && !cmd[2..].starts_with('$') {
        format!("{}${}", &cmd[..2], &cmd[2..])
    } else {
        cmd
    }
}

#[cfg(test)]
mod band_target_tests {
    use super::target_rx;

    /// Band select / band up-down / band-stack recall target the **transmit** VFO
    /// (VFO B under split, else A): the bare mnemonic for the main VFO, and the
    /// `$` form (inserted after the 2-char mnemonic) for the sub / VFO B — so a
    /// band change follows the VFO you operate on instead of always VFO A.
    ///
    /// trace: FR-VFO-04
    #[test]
    fn band_commands_follow_active_vfo() {
        for cmd in ["BN05;", "BN+;", "BN-;", "BN^;"] {
            assert_eq!(
                target_rx(cmd.to_string(), false),
                cmd,
                "main VFO stays bare"
            );
        }
        assert_eq!(target_rx("BN05;".to_string(), true), "BN$05;");
        assert_eq!(target_rx("BN+;".to_string(), true), "BN$+;");
        assert_eq!(target_rx("BN-;".to_string(), true), "BN$-;");
        assert_eq!(target_rx("BN^;".to_string(), true), "BN$^;");
    }
}

#[cfg(test)]
mod estop_wiring_tests {
    /// The emergency-stop hotkey must be dispatched **first** in
    /// `Message::KeyPressed`, before ESC/modal handling, hotkey capture, and
    /// text entry.
    ///
    /// Structural, like the tap/hold guard below, and for the same reason: the
    /// pure predicate (`ui::is_estop_hotkey`) is thoroughly unit-tested, but
    /// nothing in the suite proves it is *wired*. Verified by sabotage —
    /// disabling the dispatch left all 266 tests green, which for a safety
    /// control is not an acceptable place to leave it. Anything inserted above
    /// this check could swallow the stop when focus is in a text field, which
    /// is exactly the case the requirement exists to cover.
    ///
    /// trace: FR-TX-SAFE-05
    #[test]
    fn fr_tx_safe_05_estop_is_dispatched_before_all_other_key_handling() {
        let src = include_str!("main.rs");
        let handler = src
            .find("Message::KeyPressed(key, mods) => {")
            .expect("the KeyPressed handler must exist");
        let estop = src[handler..]
            .find("ui::is_estop_press(")
            .expect("KeyPressed must dispatch the emergency stop (FR-TX-SAFE-05)");
        let body = &src[handler..handler + estop];

        // Nothing that consumes a key press may precede it.
        for barrier in [
            "self.capturing_hotkey",
            "self.ptt_hotkey",
            "self.settings_open",
            "self.about_open",
            "self.rx_popup",
            "Named::Escape",
        ] {
            assert!(
                !body.contains(barrier),
                "`{barrier}` is handled before the emergency stop — it could \
                 swallow the stop and leave the radio keyed:\n{body}"
            );
        }

        // And it must actually send the stop, not merely test for the key.
        // Scoped to the rest of this match arm rather than a fixed byte
        // window — rustfmt reflows the call across lines as its arguments
        // grow, and a magic-number window made this guard fail on formatting
        // alone.
        let arm_end = src[handler..]
            .find("\n            Message::")
            .map_or(src.len(), |n| handler + n);
        let after = &src[handler + estop..arm_end];
        assert!(
            after.contains("WorkerCmd::EmergencyStop"),
            "the hotkey must dispatch EmergencyStop:\n{after}"
        );
    }
}

#[cfg(test)]
mod tap_hold_wiring_tests {
    /// Every `tap_hold` call must wrap a **non-interactive** visual.
    ///
    /// Structural, not logical: an iced `Button` carrying its own `on_press`
    /// captures both press and release, and `MouseArea` delegates to its
    /// content first and returns early when the content captured. A wrapped
    /// interactive button therefore swallows exactly the events the hold
    /// timing needs — the tap still fires from the button's own handler, so
    /// the control looks fine and the hold silently never happens. That
    /// shipped, and only a hardware check caught it.
    ///
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_tap_hold_wraps_a_non_interactive_visual() {
        let src = include_str!("main.rs");
        let mut checked = 0;
        for (i, _) in src.match_indices("tap_hold(") {
            // Skip the definition and this test's own prose.
            let line_start = src[..i].rfind('\n').map_or(0, |n| n + 1);
            let line = &src[line_start..i];
            if line.contains("fn ") || line.contains("///") {
                continue;
            }
            let window = &src[i..(i + 500).min(src.len())];
            assert!(
                window.contains("two_line_btn_visual"),
                "tap_hold call #{checked} must wrap two_line_btn_visual, got:\n{window}"
            );
            assert!(
                !window.contains("Some(Message::"),
                "tap_hold call #{checked} wraps an interactive button — its \
                 on_press will capture the events the hold needs:\n{window}"
            );
            checked += 1;
        }
        assert!(
            checked >= 6,
            "expected all six popup chips (ATT/PRE/AGC/NB/NR/NOTCH), found {checked}"
        );
    }
}

#[cfg(test)]
mod pan_target_tests {
    use super::{cat_is_per_pan, target_pan};

    /// Only `#SPN` is targeted. `#REF`/`#SCL` are global on a real radio: the
    /// bare form applies to both pans and the `$` form is inert, so targeting
    /// them made every DISPLAY control silently do nothing when TARGET was B.
    /// trace: FR-PAN-CTL-01
    #[test]
    fn fr_pan_ctl_01_only_span_is_targeted() {
        assert!(cat_is_per_pan("#SPN50000;"));
        for cmd in [
            "#REF-130;",
            "#SCL70;",
            "#AVG04;",
            "#PKM1;",
            "#FRZ0;",
            "#WFC1;",
            "#WFH060;",
            "#DPM2;",
        ] {
            assert!(!cat_is_per_pan(cmd), "{cmd} must not be targeted");
        }
    }

    /// Targeting adds `$` for pan B and leaves pan A bare; an untargeted
    /// command is unchanged either way.
    /// trace: FR-PAN-CTL-01
    #[test]
    fn fr_pan_ctl_01_target_pan_rewrites_only_for_b() {
        assert_eq!(target_pan("#SPN50000;".into(), false), "#SPN50000;");
        assert_eq!(target_pan("#SPN50000;".into(), true), "#SPN$50000;");
        // Already carrying `$` is left alone rather than doubled.
        assert_eq!(target_pan("#SPN$50000;".into(), true), "#SPN$50000;");
    }

    /// The regression this guards: a global command must reach the radio in
    /// its bare form for **both** target settings, or it does nothing on B.
    /// trace: FR-PAN-CTL-01
    #[test]
    fn fr_pan_ctl_01_global_commands_are_never_dollared() {
        // Mirrors the call site so the composition is exercised, not just the
        // predicate: only a per-pan command is rewritten.
        let on_wire = |cmd: &str, target_b: bool| -> String {
            if cat_is_per_pan(cmd) {
                target_pan(cmd.to_string(), target_b)
            } else {
                cmd.to_string()
            }
        };
        for cmd in ["#REF-130;", "#SCL70;", "#AVG04;", "#WFH060;"] {
            assert_eq!(on_wire(cmd, false), cmd, "{cmd} with TARGET A");
            assert_eq!(
                on_wire(cmd, true),
                cmd,
                "{cmd} with TARGET B — a `$` here is inert on the radio, so the \
                 control would silently do nothing"
            );
        }
        // Span is still targeted, so the two panes stay independently settable.
        assert_eq!(on_wire("#SPN50000;", false), "#SPN50000;");
        assert_eq!(on_wire("#SPN50000;", true), "#SPN$50000;");
    }
}

#[cfg(test)]
mod stable_width_tests {
    use super::{fmt_dbm, S_METER_WIDEST};

    /// No reading the S-meter can produce may exceed the width reserved for
    /// it. The reservation sits beside a `Length::Fill` bar, so an unexpectedly
    /// long reading does not merely shift a neighbour — it steals width from
    /// the meter, and the bar reads short for a strong signal.
    ///
    /// Swept across the whole plausible dBm range rather than spot-checked:
    /// the S-unit label changes shape partway up (`S0`..`S9`, then `S9+nndB`),
    /// so the widest string is not at either end of the range.
    /// trace: FR-UI-STABLE-01
    #[test]
    fn fr_ui_stable_01_s_meter_reservation_covers_every_reading() {
        let reserved = S_METER_WIDEST.chars().count();
        let mut widest = String::new();
        for dbm in -140..=10 {
            let s = fmt_dbm(Some(dbm));
            if s.chars().count() > widest.chars().count() {
                widest = s.clone();
            }
            assert!(
                s.chars().count() <= reserved,
                "{dbm} dBm renders {s:?} ({} chars), over the {reserved} reserved",
                s.chars().count()
            );
        }
        // The placeholder must fit too — it is what a disconnected app shows.
        assert!(fmt_dbm(None).chars().count() <= reserved);
        // Not an exact-fit assertion: the constant deliberately combines the
        // widest unit with the widest number, which need not co-occur.
        assert!(
            widest.chars().count() <= reserved,
            "widest real reading was {widest:?}"
        );
    }
}
