//! UI view-model helpers (ARC-15): pure, iced-free presentation logic backing
//! the testable `FR-UI-*` requirements. The iced view (`main.rs`, ARC-08) is a
//! thin projection over these; keeping them here makes the decidable parts of
//! the UI unit-testable without a display (NFR-USE-01, NFR-MAINT-01).
//!
//! Design rationale and the adopt/diverge stance: `docs/concept/ui-design.md`,
//! `ADR-15`, sourced from the K4 native LCD (`R-EXT-02`).

/// Which VFO(s) the main window shows, mirroring the K4 `PAN=A / PAN=B /
/// PAN=A+B` selection (FR-UI-08, R-EXT-02). The operator cycles through these;
/// the layout reflows to the active mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    /// Single VFO A focus (one panadapter pane).
    #[default]
    SingleA,
    /// Single VFO B / sub-RX focus.
    SingleB,
    /// Both VFOs side-by-side (dual panadapter).
    Dual,
}

impl ViewMode {
    /// Short label matching the K4's `PAN=` control (`A`, `B`, `A+B`).
    /// trace: FR-UI-08
    pub fn label(self) -> &'static str {
        match self {
            ViewMode::SingleA => "A",
            ViewMode::SingleB => "B",
            ViewMode::Dual => "A+B",
        }
    }
}

/// Semantic colour role (FR-UI-10) — *our palette, the K4's meaning*. The iced
/// layer maps each role to a concrete `iced::Color`; keeping the role enum here
/// (with a plain RGB accessor) lets the mapping be tested without iced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorRole {
    /// Transmit state and transmit-side values (amber/orange).
    TxActive,
    /// VFO A / main receiver (blue).
    VfoA,
    /// VFO B / sub receiver, and "active/selected" (green).
    VfoB,
    /// Receive-side readouts (near-white).
    RxValue,
    /// Caution, e.g. high SWR (yellow).
    Caution,
    /// An off/available control (dim grey).
    Inactive,
}

impl ColorRole {
    /// Concrete sRGB triple for the role — our own monitor-tuned palette, not a
    /// pixel-match of the K4 or any third-party app (ADR-15).
    /// trace: FR-UI-10
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            ColorRole::TxActive => (0xFF, 0x9A, 0x1E), // amber
            ColorRole::VfoA => (0x3D, 0x9B, 0xFF),     // blue
            ColorRole::VfoB => (0x33, 0xCC, 0x66),     // green
            ColorRole::RxValue => (0xEC, 0xEF, 0xF2),  // near-white
            ColorRole::Caution => (0xFF, 0xD4, 0x33),  // yellow
            ColorRole::Inactive => (0x66, 0x6B, 0x72), // dim grey
        }
    }
}

/// Layered surface shades for the dark, reference-faithful theme (FR-UI-15):
/// the window background, grouping panels, and interactive controls step up in
/// brightness so depth reads without heavy chrome — the visual grammar of the
/// K4 LCD and the reference client (R-EXT-02). Strict luminance ordering is the
/// testable contract; the exact values are our own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shade {
    /// Window background (near-black, like the K4 LCD field).
    Bg,
    /// Grouping panel behind a band of related controls.
    Panel,
    /// Recessed well: meter tracks, waterfall margins.
    Track,
    /// Interactive control (button) at rest.
    Control,
    /// Control under the pointer.
    ControlHover,
    /// Hairline border / edge.
    Edge,
}

impl Shade {
    /// Concrete sRGB triple for the shade in the default (dark) theme.
    /// trace: FR-UI-15
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Shade::Bg => (0x0B, 0x0D, 0x10),
            Shade::Panel => (0x14, 0x17, 0x1B),
            Shade::Track => (0x1A, 0x1D, 0x22),
            Shade::Control => (0x24, 0x28, 0x2E),
            Shade::ControlHover => (0x2F, 0x34, 0x3B),
            Shade::Edge => (0x3A, 0x40, 0x48),
        }
    }
}

/// Selectable UI theme (FR-UI-17). `System` follows the OS light/dark
/// preference; the other three are explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    /// Reference-faithful dark theme (the default).
    #[default]
    Dark,
    /// Light theme for bright environments.
    Light,
    /// High-contrast theme (pure black/white, bright accents).
    Contrast,
    /// Follow the operating system's light/dark preference.
    System,
}

/// A concrete theme actually used to resolve colours — `System` is resolved to
/// one of these first (FR-UI-17).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectiveTheme {
    Dark,
    Light,
    Contrast,
}

impl ThemeMode {
    /// Cycle order for the toggle button: Dark → Light → Contrast → System → …
    /// trace: FR-UI-17
    pub fn next(self) -> Self {
        match self {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Contrast,
            ThemeMode::Contrast => ThemeMode::System,
            ThemeMode::System => ThemeMode::Dark,
        }
    }

    /// Button label for the current mode.
    /// trace: FR-UI-17
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Dark => "Dark",
            ThemeMode::Light => "Light",
            ThemeMode::Contrast => "Contrast",
            ThemeMode::System => "System",
        }
    }

    /// Resolve to a concrete palette; `System` uses the detected OS preference.
    /// trace: FR-UI-17
    pub fn effective(self, system_is_dark: bool) -> EffectiveTheme {
        match self {
            ThemeMode::Dark => EffectiveTheme::Dark,
            ThemeMode::Light => EffectiveTheme::Light,
            ThemeMode::Contrast => EffectiveTheme::Contrast,
            ThemeMode::System => {
                if system_is_dark {
                    EffectiveTheme::Dark
                } else {
                    EffectiveTheme::Light
                }
            }
        }
    }
}

/// Surface shade for a given theme (FR-UI-17). Dark reuses [`Shade::rgb`]; Light
/// and Contrast are their own palettes.
/// trace: FR-UI-17
pub fn shade_rgb(theme: EffectiveTheme, s: Shade) -> (u8, u8, u8) {
    match theme {
        EffectiveTheme::Dark => s.rgb(),
        EffectiveTheme::Light => match s {
            Shade::Bg => (0xEE, 0xF1, 0xF4),
            Shade::Panel => (0xFF, 0xFF, 0xFF),
            Shade::Track => (0xE4, 0xE8, 0xED),
            Shade::Control => (0xE8, 0xEC, 0xF1),
            Shade::ControlHover => (0xDA, 0xDF, 0xE6),
            Shade::Edge => (0xC6, 0xCD, 0xD5),
        },
        EffectiveTheme::Contrast => match s {
            Shade::Bg => (0x00, 0x00, 0x00),
            Shade::Panel => (0x0A, 0x0A, 0x0A),
            Shade::Track => (0x14, 0x14, 0x14),
            Shade::Control => (0x1E, 0x1E, 0x1E),
            Shade::ControlHover => (0x30, 0x30, 0x30),
            Shade::Edge => (0xFF, 0xFF, 0xFF),
        },
    }
}

/// Semantic role colour for a given theme (FR-UI-17). Dark reuses
/// [`ColorRole::rgb`]; Light darkens accents for contrast on a light ground,
/// Contrast brightens them against pure black.
/// trace: FR-UI-17
pub fn role_rgb(theme: EffectiveTheme, role: ColorRole) -> (u8, u8, u8) {
    match theme {
        EffectiveTheme::Dark => role.rgb(),
        EffectiveTheme::Light => match role {
            ColorRole::TxActive => (0xC7, 0x6A, 0x00),
            ColorRole::VfoA => (0x1E, 0x66, 0xD0),
            ColorRole::VfoB => (0x1E, 0x8A, 0x44),
            ColorRole::RxValue => (0x1A, 0x1E, 0x24),
            ColorRole::Caution => (0xB8, 0x86, 0x00),
            ColorRole::Inactive => (0x7A, 0x80, 0x88),
        },
        EffectiveTheme::Contrast => match role {
            ColorRole::TxActive => (0xFF, 0xB0, 0x2E),
            ColorRole::VfoA => (0x4D, 0xB1, 0xFF),
            ColorRole::VfoB => (0x3D, 0xF0, 0x7A),
            ColorRole::RxValue => (0xFF, 0xFF, 0xFF),
            ColorRole::Caution => (0xFF, 0xEE, 0x00),
            ColorRole::Inactive => (0xB0, 0xB0, 0xB0),
        },
    }
}

/// About-box content (FR-UI-18): author, license, project URL — one per line.
pub const ABOUT_AUTHOR: &str = "by Simon Keimer (DC0SK)";
/// The project's license (see the repository `LICENSE`).
pub const ABOUT_LICENSE: &str = "License: GNU GPL v3.0";
/// The full text of the license (opened from the About box).
pub const ABOUT_LICENSE_URL: &str = "https://www.gnu.de/documents/gpl-3.0.en.html";
/// The project's home.
pub const ABOUT_URL: &str = "https://github.com/dc0sk/K4remote";
/// PayPal donation link (from the author's GitHub profile).
pub const ABOUT_DONATE_URL: &str = "https://www.paypal.com/donate/?hosted_button_id=WY9U4MQ3ZAQWC";

/// This build's version string (from the crate metadata), shown in the About box.
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// K4 S-meter face endpoints (FR-UI-15): S1 ≈ −121 dBm up to S9+60 dB ≈ −13 dBm,
/// with S9 = −73 dBm and 6 dB per S-unit — the scale printed on the radio's
/// meter and drawn as a proportional bar by the reference client.
pub const S_METER_FLOOR_DBM: i32 = -121;
/// Upper end of the meter face (S9 + 60 dB).
pub const S_METER_CEIL_DBM: i32 = -13;

/// Fraction of the S-meter bar to fill for a dBm reading, clamped to `[0, 1]`.
/// trace: FR-UI-15
pub fn s_meter_fraction(dbm: i32) -> f32 {
    let span = (S_METER_CEIL_DBM - S_METER_FLOOR_DBM) as f32;
    ((dbm - S_METER_FLOOR_DBM) as f32 / span).clamp(0.0, 1.0)
}

/// Connection lifecycle phase surfaced to the UI (FR-UI-16, a presentation
/// subset of the `FR-CONN-03` state set). The connect control's label and
/// action are a pure function of this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConnPhase {
    /// Idle: no session and no attempt in flight.
    #[default]
    Disconnected,
    /// An attempt is in flight (opening/handshaking) or waiting to retry.
    Connecting,
    /// A live session is up.
    Connected,
}

/// What the primary connect control does when tapped (FR-UI-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectAction {
    /// Start a connection attempt.
    Connect,
    /// Abort the in-flight attempt and return to disconnected.
    Cancel,
    /// Tear down the live session.
    Disconnect,
}

/// Label + action for the connect control given the connection phase (FR-UI-16):
/// idle shows **Connect**; while an attempt is in flight it shows **Cancel** and
/// tapping it aborts; once connected it shows **Disconnect**. Pure so the
/// label/action contract is unit-testable without iced.
/// trace: FR-UI-16
pub fn connect_button(phase: ConnPhase) -> (&'static str, ConnectAction) {
    match phase {
        ConnPhase::Disconnected => ("Connect", ConnectAction::Connect),
        ConnPhase::Connecting => ("Cancel", ConnectAction::Cancel),
        ConnPhase::Connected => ("Disconnect", ConnectAction::Disconnect),
    }
}

/// The header connection indicator's label and colour role for each phase
/// (FR-UI-22): green when connected, amber while connecting, grey when idle.
/// trace: FR-UI-22
pub fn conn_status(phase: ConnPhase) -> (&'static str, ColorRole) {
    match phase {
        ConnPhase::Connected => ("CONNECTED", ColorRole::VfoB),
        ConnPhase::Connecting => ("connecting...", ColorRole::TxActive),
        ConnPhase::Disconnected => ("disconnected", ColorRole::Inactive),
    }
}

/// Default window size at launch (FR-UI-21) — landscape (wider than tall). The
/// height fits the fixed-height content (VFO band + screen slot + panels) so the
/// window opens without a scrollbar.
/// trace: FR-UI-21
pub const DEFAULT_WINDOW_SIZE: (f32, f32) = (1280.0, 1000.0);

/// Colour role for a VFO's header given which receiver it is, whether it is the
/// transmit VFO (split-aware), and whether the radio is transmitting. Transmit
/// overrides so the operating VFO is unmistakable during TX (FR-UI-06).
/// trace: FR-UI-10
pub fn vfo_role(is_vfo_b: bool, is_tx_vfo: bool, transmitting: bool) -> ColorRole {
    if transmitting && is_tx_vfo {
        ColorRole::TxActive
    } else if is_vfo_b {
        ColorRole::VfoB
    } else {
        ColorRole::VfoA
    }
}

/// Colour role for an on/off control button: active controls read green
/// ("active/selected"), inactive ones dim grey.
/// trace: FR-UI-10
pub fn toggle_role(on: bool) -> ColorRole {
    if on {
        ColorRole::VfoB
    } else {
        ColorRole::Inactive
    }
}

/// Format a frequency in Hz with the K4's dot-grouped readout (FR-UI-09):
/// `14_070_000 → "14.070.000"`. Standard 3-digit grouping from the right with
/// `.` separators; values below 1 kHz are shown plain.
/// trace: FR-UI-09
pub fn format_freq_hz(hz: u64) -> String {
    let digits = hz.to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let lead = bytes.len() % 3;
    for (i, &b) in bytes.iter().enumerate() {
        if i != 0 && i >= lead && (i - lead).is_multiple_of(3) {
            out.push('.');
        }
        out.push(b as char);
    }
    out
}

/// Optional-Hz convenience: shows a placeholder when the value is unknown.
/// trace: FR-UI-09
pub fn format_freq_opt(hz: Option<u64>) -> String {
    match hz {
        Some(v) => format_freq_hz(v),
        None => "—.———.———".to_string(),
    }
}

/// A two-line state button: a fixed function `label` plus a live `value` derived
/// from the radio state (FR-UI-11). The button *is* the status readout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonState {
    pub label: &'static str,
    pub value: String,
}

impl ButtonState {
    fn new(label: &'static str, value: impl Into<String>) -> Self {
        ButtonState {
            label,
            value: value.into(),
        }
    }
}

const UNKNOWN: &str = "—";

/// AGC button state from the `AG`/`GT` mode code (0 off / 1 slow / 2 fast).
/// trace: FR-UI-11
pub fn agc_button(mode: Option<u8>) -> ButtonState {
    let value = match mode {
        Some(0) => "Off",
        Some(1) => "Slow",
        Some(2) => "Fast",
        Some(_) => "On",
        None => UNKNOWN,
    };
    ButtonState::new("AGC", value)
}

/// Attenuator button state from on/off + dB (`RA`).
/// trace: FR-UI-11
pub fn atten_button(on: Option<bool>, db: Option<u8>) -> ButtonState {
    let value = match (on, db) {
        (Some(false), _) | (None, _) => "Off".to_string(),
        (Some(true), Some(d)) => format!("{d} dB"),
        (Some(true), None) => "On".to_string(),
    };
    ButtonState::new("ATT", value)
}

/// Receive-bandwidth button state in kHz with 2 decimals (`BW`): 2800 → "2.80".
/// trace: FR-UI-11
pub fn bandwidth_button(hz: Option<u32>) -> ButtonState {
    let value = match hz {
        Some(v) => format!("{:.2}", v as f64 / 1000.0),
        None => UNKNOWN.to_string(),
    };
    ButtonState::new("BW", value)
}

/// Generic on/off control button (NB/NR/preamp/RIT/XIT…).
/// trace: FR-UI-11
pub fn toggle_button(label: &'static str, on: Option<bool>) -> ButtonState {
    let value = match on {
        Some(true) => "On",
        Some(false) => "Off",
        None => UNKNOWN,
    };
    ButtonState::new(label, value)
}

/// Preamp chip showing the current level (`PA`): "Off" or the gain step 1–3.
/// trace: FR-RX-02
pub fn preamp_button(on: Option<bool>, level: Option<u8>) -> ButtonState {
    let value = match (on, level) {
        (Some(true), Some(n)) if n > 0 => format!("Lvl {n}"),
        (Some(true), _) => "On".to_string(),
        (Some(false), _) => "Off".to_string(),
        (None, _) => UNKNOWN.to_string(),
    };
    ButtonState::new("PRE", value)
}

/// The K4's seven fixed primary buttons (FR-UI-13). Tapping one opens its
/// context sub-row of controls; the row sits just above the primaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Primary {
    Menu,
    Fn,
    Display,
    Band,
    MainRx,
    SubRx,
    Tx,
}

impl Primary {
    /// All primaries in their on-screen left-to-right order.
    /// trace: FR-UI-13
    pub fn all() -> [Primary; 7] {
        [
            Primary::Menu,
            Primary::Fn,
            Primary::Display,
            Primary::Band,
            Primary::MainRx,
            Primary::SubRx,
            Primary::Tx,
        ]
    }

    /// On-screen label, matching the K4.
    /// trace: FR-UI-13
    pub fn label(self) -> &'static str {
        match self {
            Primary::Menu => "MENU",
            Primary::Fn => "Fn",
            Primary::Display => "DISPLAY",
            Primary::Band => "BAND",
            Primary::MainRx => "MAIN RX",
            Primary::SubRx => "SUB RX",
            Primary::Tx => "TX",
        }
    }
}

/// Tracks which primary's context row is open (FR-UI-13). At most one row is
/// open; tapping the open primary again closes it (the K4's toggle behaviour).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextRow {
    open: Option<Primary>,
}

impl ContextRow {
    /// Tap a primary: open its row, or close it if it was already the open one.
    /// trace: FR-UI-13
    pub fn tap(&mut self, p: Primary) {
        self.open = if self.open == Some(p) { None } else { Some(p) };
    }

    /// The primary whose context row is currently open, if any.
    /// trace: FR-UI-13
    pub fn active(self) -> Option<Primary> {
        self.open
    }

    /// Whether `p`'s context row is the open one (for highlighting the primary).
    /// trace: FR-UI-13
    pub fn is_open(self, p: Primary) -> bool {
        self.open == Some(p)
    }

    /// Close any open context row.
    /// trace: FR-UI-13
    pub fn close(&mut self) {
        self.open = None;
    }
}

/// The kind of screen a primary opens in the spectrum-frame slot (FR-UI-19).
/// The main/sub distinction (both `RxEq`) is handled by the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    RxEq,
    TxConfig,
    Display,
    Band,
    Fn,
    Menu,
}

/// Which screen each primary softkey opens (FR-UI-19). Pure so the primary →
/// screen mapping is unit-testable; the iced view renders the chosen screen.
/// trace: FR-UI-19
pub fn screen_kind(p: Primary) -> ScreenKind {
    match p {
        Primary::MainRx | Primary::SubRx => ScreenKind::RxEq,
        Primary::Tx => ScreenKind::TxConfig,
        Primary::Display => ScreenKind::Display,
        Primary::Band => ScreenKind::Band,
        Primary::Fn => ScreenKind::Fn,
        Primary::Menu => ScreenKind::Menu,
    }
}

/// The K4's direct-select bands with their `BN` numbers (FR-VFO-04): 00 = 160 m
/// … 10 = 6 m (Programmer's Reference `BN`, confirmed vs QK4). Used by the BAND
/// screen's band grid.
/// trace: FR-VFO-04
pub fn band_buttons() -> &'static [(&'static str, u8)] {
    &[
        ("160", 0),
        ("80", 1),
        ("60", 2),
        ("40", 3),
        ("30", 4),
        ("20", 5),
        ("17", 6),
        ("15", 7),
        ("12", 8),
        ("10", 9),
        ("6", 10),
    ]
}

/// Waterfall-palette names for the `#WFC` display command (FR-PAN-CTL-01).
/// trace: FR-PAN-CTL-01
pub fn waterfall_palettes() -> &'static [&'static str; 5] {
    &["Gray", "Color", "Teal", "Blue", "Sepia"]
}

/// The four quick-memory keys with their `SW` switch codes (FR-SW-01):
/// `(label, tap-code, hold-code)`. Tap recalls/plays the memory, hold stores
/// it (per K4 rev. D12 switch-emulation table).
/// trace: FR-SW-01
pub fn quick_mem_keys() -> &'static [(&'static str, u16, u16); 4] {
    &[
        ("M1", 17, 162),
        ("M2", 51, 163),
        ("M3", 18, 164),
        ("M4", 52, 165),
    ]
}

/// The four programmable function keys with their `SW` codes (FR-SW-01).
/// trace: FR-SW-01
pub fn pf_keys() -> &'static [(&'static str, u16); 4] {
    &[("PF1", 153), ("PF2", 154), ("PF3", 155), ("PF4", 156)]
}

/// RX-antenna short names for the `AR` command (FR-ANT-01), index = the `AR`
/// value 0–7 (0 disconnected … 5–7 = ATU RX antennas).
/// trace: FR-ANT-01
pub fn rx_antenna_names() -> &'static [&'static str; 8] {
    &["Off", "RX2", "=TX", "XVTR", "RX1", "ATU1", "ATU2", "ATU3"]
}

/// Transmit-audio input source names for the `MI` command (FR-AUD-CFG-01),
/// index = the `MI` value (0 front … 4 rear+line).
/// trace: FR-AUD-CFG-01
pub fn mic_inputs() -> &'static [&'static str; 5] {
    &["Front", "Rear", "Line", "Front+Line", "Rear+Line"]
}

/// The K4's configuration-menu items with their menu ids (FR-MENU-01), from the
/// *Programmer's Reference D12* menu table (sorted by id). Each item maps to
/// `MEDF<id>` (name/definition), `ME<id>.<value>` (set), and `MO<id>` (open on
/// the radio screen).
/// trace: FR-MENU-01
pub fn menu_items() -> &'static [(u16, &'static str)] {
    &[
        (1, "Speaker, Internal"),
        (2, "TX ALC"),
        (3, "Fan Speed Min"),
        (4, "KAT4 ATU Option"),
        (5, "KRX4 2ND RX Option"),
        (6, "KPA4 PA Option"),
        (7, "AGC Hold Time"),
        (8, "AGC Decay, Slow"),
        (9, "AGC Decay, Fast"),
        (10, "AGC Threshold"),
        (11, "AGC Attack"),
        (12, "AGC Slope"),
        (13, "AGC Noise Pulse Reject"),
        (14, "TX 2-Tone Generator"),
        (15, "TX Gain Cal via TUNE"),
        (27, "Wattmeter Cal"),
        (28, "TX Gain Cal"),
        (30, "Spectrum Trace Fill"),
        (33, "Radio Serial Number"),
        (34, "Radio Type"),
        (36, "LCD Brightness"),
        (37, "LED Brightness"),
        (38, "VFO Counts per Turn"),
        (39, "AF Limiter (AGC off)"),
        (40, "Reference Freq"),
        (41, "VFO B Different Band"),
        (42, "RIT CLR 2nd Tap Restore"),
        (43, "IP Address"),
        (44, "RIT Knob Alt. Function"),
        (45, "VFO Coarse Tuning"),
        (46, "Per-Band Power"),
        (48, "AutoRef Averaging"),
        (49, "AutoRef Debounce"),
        (50, "AutoRef Offset"),
        (52, "TX DLY, Key Out to RF Out"),
        (53, "TX Inhibit Mode"),
        (54, "Serial RS232: DTR"),
        (55, "Serial RS232: RTS"),
        (57, "Serial RS232: Baud Rate"),
        (58, "Serial USB-PC1: DTR"),
        (59, "Serial USB-PC1: RTS"),
        (60, "Serial USB-PC1: Baud Rate"),
        (61, "Serial USB-PC2: DTR"),
        (62, "Serial USB-PC2: RTS"),
        (63, "Serial USB-PC2: Baud Rate"),
        (64, "FSK Dual-Tone RX Filter"),
        (65, "Serial RS232: Auto Info"),
        (66, "Serial USB-PC1: Auto Info"),
        (67, "Serial USB-PC2: Auto Info"),
        (69, "TUNE LP (Low power TUNE)"),
        (70, "Ext. Monitor Function"),
        (71, "Ext. Monitor Location"),
        (72, "Speakers + Phones"),
        (73, "RX Auto Attenuation"),
        (74, "Mouse L/R Button QSY"),
        (75, "XVTR OUT Test"),
        (76, "XVTR Band <n> Mode"),
        (77, "XVTR Band <n> R.F."),
        (78, "XVTR Band <n> I.F."),
        (79, "XVTR Band <n> Offset"),
        (80, "Screen Cap File"),
        (83, "Speakers, External"),
        (84, "FSK Polarity"),
        (85, "FSK Mark-Tone"),
        (86, "XVTR Band # Select"),
        (87, "Message Repeat Interval"),
        (88, "FM Deviation, Voice"),
        (89, "FM Deviation, Tone"),
        (90, "Spectrum Freq. Marks"),
        (91, "RX 1.5 MHz High-Pass Fil."),
        (92, "Spectrum Amplitude Units"),
        (93, "Preamp 3 (12/10/6 m)"),
        (97, "RX Dyn. Range Optimization"),
        (98, "XVTR Band <n> Power Out"),
        (100, "DIGOUT1 (ACC jack, pin 11)"),
        (101, "TX Monitor Level, Line Out"),
        (102, "RX CW IIR Filters (50-200 Hz)"),
        (103, "TX Noise Gate Threshold"),
        (104, "CW TX in SSB Mode"),
        (105, "RX Audio Mix with Sub On"),
        (106, "RX All-Mode Squelch"),
        (107, "Mouse Pointer Size, LCD"),
        (108, "TX Audio LF Cutoff, SSB"),
        (109, "Mouse Pointer Size, Ext. Mon."),
        (110, "TX DLY, Unkey to Receive"),
        (111, "TX QSK Method"),
        (112, "TX Monitor Method, Voice"),
        (113, "RX Audio Gain Boost"),
        (114, "TX Monitor Level, Remote"),
    ]
}

/// Indices of [`menu_items`] whose name contains `query` (case-insensitive);
/// an empty query matches everything (FR-MENU-01). Backs the MENU screen search.
/// trace: FR-MENU-01
pub fn menu_search(query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    menu_items()
        .iter()
        .enumerate()
        .filter(|(_, (_, name))| q.is_empty() || name.to_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// Handy front-panel switches reachable by `SW` emulation (FR-SW-01), as
/// `(label, code)` — codes from the D12 switch-emulation table.
/// trace: FR-SW-01
pub fn radio_switches() -> &'static [(&'static str, u16); 7] {
    &[
        ("SPOT", 42),
        ("TUNE", 16),
        ("ATU TUNE", 40),
        ("DIV", 152),
        ("LOCK A", 63),
        ("LOCK B", 151),
        ("MON", 128),
    ]
}

/// The K4's transmit-area dual-function switches (`SW` emulation, FR-SW-01), as
/// `(tap-label, tap-code, hold-label, hold-code)` — codes from the D12
/// switch-emulation table (Intro rev. C5 labels).
/// trace: FR-SW-01
pub fn tx_function_switches() -> &'static [(&'static str, u16, &'static str, u16); 6] {
    &[
        ("TUNE", 16, "TUNE LP", 131),
        ("ATU TUNE", 40, "ATU", 158),
        ("ANT", 60, "REM ANT", 135),
        ("XMIT", 30, "TEST", 132),
        ("VOX", 50, "QSK", 134),
        ("RX ANT", 70, "SUB ANT", 157),
    ]
}

/// A curated set of DXCC prefix → country mappings for the DX-list screen
/// (SCR-DX, FR-UI-19). A starter set of common entities — expandable to the full
/// DXCC list (a data task, not blocking).
/// trace: FR-UI-19
pub fn dx_prefixes() -> &'static [(&'static str, &'static str)] {
    &[
        ("K/W/N", "United States"),
        ("KH6", "Hawaii"),
        ("KL7", "Alaska"),
        ("KP4", "Puerto Rico"),
        ("VE", "Canada"),
        ("XE", "Mexico"),
        ("CM", "Cuba"),
        ("HI", "Dominican Rep."),
        ("6Y", "Jamaica"),
        ("ZF", "Cayman Is."),
        ("PY", "Brazil"),
        ("LU", "Argentina"),
        ("CE", "Chile"),
        ("HK", "Colombia"),
        ("YV", "Venezuela"),
        ("OA", "Peru"),
        ("CX", "Uruguay"),
        ("CP", "Bolivia"),
        ("HC", "Ecuador"),
        ("ZP", "Paraguay"),
        ("PZ", "Suriname"),
        ("G", "England"),
        ("GM", "Scotland"),
        ("GW", "Wales"),
        ("GI", "Northern Ireland"),
        ("EI", "Ireland"),
        ("F", "France"),
        ("DL", "Germany"),
        ("I", "Italy"),
        ("EA", "Spain"),
        ("CT", "Portugal"),
        ("PA", "Netherlands"),
        ("ON", "Belgium"),
        ("LX", "Luxembourg"),
        ("HB", "Switzerland"),
        ("OE", "Austria"),
        ("OK", "Czech Republic"),
        ("OM", "Slovakia"),
        ("SP", "Poland"),
        ("HA", "Hungary"),
        ("YO", "Romania"),
        ("LZ", "Bulgaria"),
        ("YU", "Serbia"),
        ("9A", "Croatia"),
        ("S5", "Slovenia"),
        ("SV", "Greece"),
        ("TA", "Turkey"),
        ("OH", "Finland"),
        ("SM", "Sweden"),
        ("LA", "Norway"),
        ("OZ", "Denmark"),
        ("TF", "Iceland"),
        ("ES", "Estonia"),
        ("YL", "Latvia"),
        ("LY", "Lithuania"),
        ("UA", "Russia (European)"),
        ("UR", "Ukraine"),
        ("EW", "Belarus"),
        ("4X", "Israel"),
        ("A6", "United Arab Emirates"),
        ("HZ", "Saudi Arabia"),
        ("JA", "Japan"),
        ("BY", "China"),
        ("BV", "Taiwan"),
        ("HL", "South Korea"),
        ("VU", "India"),
        ("YB", "Indonesia"),
        ("HS", "Thailand"),
        ("9M", "Malaysia"),
        ("DU", "Philippines"),
        ("XV", "Vietnam"),
        ("VK", "Australia"),
        ("ZL", "New Zealand"),
        ("P2", "Papua New Guinea"),
        ("ZS", "South Africa"),
        ("5Z", "Kenya"),
        ("SU", "Egypt"),
        ("CN", "Morocco"),
        ("EA8", "Canary Islands"),
        ("CT3", "Madeira"),
        ("D4", "Cape Verde"),
    ]
}

/// Indices of [`dx_prefixes`] whose prefix or country contains `query`
/// (case-insensitive); empty query matches all (SCR-DX search).
/// trace: FR-UI-19
pub fn dx_search(query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    dx_prefixes()
        .iter()
        .enumerate()
        .filter(|(_, (pfx, country))| {
            q.is_empty() || pfx.to_lowercase().contains(&q) || country.to_lowercase().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

/// The K4's 8 graphic-equalizer band centre frequencies (FR-EQ-01), shared by
/// the RX and TX EQ screens (*Intro to the K4*, `R-EXT-02`; confirmed vs the
/// Programmer's Reference `RE`/`TE`). Each band is adjustable ±16 dB.
/// trace: FR-EQ-01
pub fn eq_bands() -> &'static [&'static str; 8] {
    &["100", "200", "400", "800", "1200", "1600", "2400", "3200"]
}

/// The ±dB travel of each graphic-EQ band (FR-EQ-01).
pub const EQ_DB_RANGE: i8 = 16;

/// A panadapter/VFO pane, tagged with its receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    A,
    B,
}

impl Pane {
    /// Whether this pane is VFO B / sub-RX (drives semantic colour, FR-UI-10).
    pub fn is_b(self) -> bool {
        self == Pane::B
    }

    /// Pane label as shown in the K4's corner badge.
    pub fn label(self) -> &'static str {
        match self {
            Pane::A => "A",
            Pane::B => "B",
        }
    }
}

/// Below this window width the bands stack vertically instead of placing the
/// two VFO columns side-by-side (FR-UI-12 responsive reflow).
pub const NARROW_BREAKPOINT: f32 = 900.0;

/// How the VFO header + panadapter band are arranged for a given window width
/// and view mode (FR-UI-12). Pure so the layout decision is unit-testable; the
/// iced view consumes `panes`/`split_center`/`stacked` to build the frame.
#[derive(Debug, Clone, PartialEq)]
pub struct BandLayout {
    /// The pane(s) to show, left-to-right (or top-to-bottom when stacked).
    pub panes: Vec<Pane>,
    /// Whether the shared TX/SPLIT/RIT-XIT box sits *between* two VFO columns
    /// (only in dual on a wide enough window).
    pub split_center: bool,
    /// Narrow window: stack the panes vertically rather than side-by-side.
    pub stacked: bool,
}

/// Compute the band arrangement (FR-UI-12).
/// trace: FR-UI-12
pub fn band_layout(window_w: f32, mode: ViewMode) -> BandLayout {
    let panes = match mode {
        ViewMode::SingleA => vec![Pane::A],
        ViewMode::SingleB => vec![Pane::B],
        ViewMode::Dual => vec![Pane::A, Pane::B],
    };
    let stacked = window_w < NARROW_BREAKPOINT;
    BandLayout {
        split_center: mode == ViewMode::Dual && !stacked,
        stacked,
        panes,
    }
}

// --- Mode-adaptive UI (docs/concept/mode-aware-ui.md) -----------------------

/// Presentation class derived from the operating mode. Drives which controls
/// are shown/dimmed/hidden in the mode-aware UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeClass {
    Cw,
    Voice,
    Data,
    Am,
    Fm,
}

impl ModeClass {
    /// Classify a K4 mode string; unknown/absent → `Voice` (safe default).
    /// trace: FR-UI-24
    pub fn from_mode(m: Option<&str>) -> ModeClass {
        match m {
            Some("CW") | Some("CW-R") => ModeClass::Cw,
            Some("DATA") | Some("DATA-R") | Some("FSK") | Some("FSK-D") => ModeClass::Data,
            Some("AM") => ModeClass::Am,
            Some("FM") => ModeClass::Fm,
            _ => ModeClass::Voice,
        }
    }
}

/// How a control should appear for the current mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vis {
    /// Full emphasis.
    Show,
    /// Visible but de-emphasised (still usable).
    Dim,
    /// Not shown (lives only in the mode strip).
    Hide,
}

/// Mode-varying RX-frame controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxCtl {
    Bw,
    FilterPresets,
    ShiftHiLo,
    Agc,
    ManualNotch,
    AutoNotch,
    Squelch,
}

/// Mode-varying TX-frame controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// `Cmp`/`MicGain` have no main-panel control yet — wired in Phase 4 (mode strip).
#[allow(dead_code)]
pub enum TxCtl {
    Qsk,
    Vox,
    AntiVox,
    Dvr,
    Autospot,
    Cmp,
    MicGain,
}

/// Visibility of an RX control for a mode class (concept §1).
/// trace: FR-UI-24
pub fn rx_ctl_vis(c: RxCtl, m: ModeClass) -> Vis {
    use ModeClass::*;
    use Vis::*;
    match (c, m) {
        (RxCtl::Bw | RxCtl::FilterPresets | RxCtl::Agc, Fm) => Dim,
        (RxCtl::Bw | RxCtl::FilterPresets | RxCtl::Agc, _) => Show,
        (RxCtl::ShiftHiLo, Fm) => Hide,
        (RxCtl::ShiftHiLo, Am) => Dim,
        (RxCtl::ShiftHiLo, _) => Show,
        (RxCtl::ManualNotch, Cw) => Dim,
        (RxCtl::ManualNotch, Fm) => Hide,
        (RxCtl::ManualNotch, _) => Show,
        (RxCtl::AutoNotch, Cw | Fm) => Hide,
        (RxCtl::AutoNotch, Data) => Dim,
        (RxCtl::AutoNotch, _) => Show,
        (RxCtl::Squelch, Fm) => Show,
        (RxCtl::Squelch, _) => Dim,
    }
}

/// Visibility of a TX control for a mode class (concept §1).
/// trace: FR-UI-24
pub fn tx_ctl_vis(c: TxCtl, m: ModeClass) -> Vis {
    use ModeClass::*;
    use Vis::*;
    match (c, m) {
        (TxCtl::Qsk | TxCtl::Autospot, Cw) => Show,
        (TxCtl::Qsk | TxCtl::Autospot, _) => Hide,
        (TxCtl::Vox, Cw) => Hide,
        (TxCtl::Vox, _) => Show,
        (TxCtl::AntiVox, Cw) => Hide,
        (TxCtl::AntiVox, Data) => Dim,
        (TxCtl::AntiVox, _) => Show,
        (TxCtl::Dvr, Cw | Data) => Hide,
        (TxCtl::Dvr, _) => Show,
        (TxCtl::Cmp, Voice | Am) => Show,
        (TxCtl::Cmp, Fm) => Dim,
        (TxCtl::Cmp, _) => Hide,
        (TxCtl::MicGain, Voice | Am | Fm) => Show,
        (TxCtl::MicGain, _) => Hide,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// trace: FR-UI-24
    #[test]
    fn fr_ui_24_mode_class_and_visibility() {
        assert_eq!(ModeClass::from_mode(Some("CW-R")), ModeClass::Cw);
        assert_eq!(ModeClass::from_mode(Some("USB")), ModeClass::Voice);
        assert_eq!(ModeClass::from_mode(Some("FM")), ModeClass::Fm);
        assert_eq!(ModeClass::from_mode(None), ModeClass::Voice);
        // Squelch promoted in FM, dimmed elsewhere.
        assert_eq!(rx_ctl_vis(RxCtl::Squelch, ModeClass::Fm), Vis::Show);
        assert_eq!(rx_ctl_vis(RxCtl::Squelch, ModeClass::Cw), Vis::Dim);
        assert_eq!(rx_ctl_vis(RxCtl::ShiftHiLo, ModeClass::Fm), Vis::Hide);
        // QSK/autospot CW-only; VOX not in CW; CMP voice; DVR not CW/DATA.
        assert_eq!(tx_ctl_vis(TxCtl::Qsk, ModeClass::Cw), Vis::Show);
        assert_eq!(tx_ctl_vis(TxCtl::Qsk, ModeClass::Voice), Vis::Hide);
        assert_eq!(tx_ctl_vis(TxCtl::Vox, ModeClass::Cw), Vis::Hide);
        assert_eq!(tx_ctl_vis(TxCtl::Cmp, ModeClass::Voice), Vis::Show);
        assert_eq!(tx_ctl_vis(TxCtl::Cmp, ModeClass::Cw), Vis::Hide);
        assert_eq!(tx_ctl_vis(TxCtl::Dvr, ModeClass::Data), Vis::Hide);
    }

    // trace: FR-UI-08 — default view mode is single-A.
    #[test]
    fn fr_ui_08_view_mode_default() {
        assert_eq!(ViewMode::default(), ViewMode::SingleA);
    }

    // trace: FR-EQ-01 — the graphic EQ exposes the K4's 8 bands with ±16 dB.
    #[test]
    fn fr_eq_01_eq_bands() {
        assert_eq!(eq_bands().len(), 8);
        assert_eq!(eq_bands()[0], "100");
        assert_eq!(eq_bands()[7], "3200");
        assert_eq!(EQ_DB_RANGE, 16);
    }

    // trace: FR-VFO-04 — the BAND grid maps labels to `BN` numbers (160=00…6=10).
    #[test]
    fn fr_vfo_04_band_buttons() {
        let b = band_buttons();
        assert_eq!(b.len(), 11);
        assert_eq!(b[0], ("160", 0));
        assert_eq!(b[10], ("6", 10));
        // Monotonic band numbers 0..=10.
        for (i, (_, bn)) in b.iter().enumerate() {
            assert_eq!(*bn as usize, i);
        }
        assert_eq!(waterfall_palettes().len(), 5);
    }

    // trace: FR-AUD-CFG-01 — mic-input names index by the `MI` value (0–4).
    #[test]
    fn fr_aud_cfg_01_mic_inputs() {
        assert_eq!(mic_inputs().len(), 5);
        assert_eq!(mic_inputs()[0], "Front");
        assert_eq!(mic_inputs()[2], "Line");
    }

    // trace: FR-ANT-01 — RX-antenna names index by the `AR` value (0–7).
    #[test]
    fn fr_ant_01_rx_antenna_names() {
        assert_eq!(rx_antenna_names().len(), 8);
        assert_eq!(rx_antenna_names()[0], "Off");
        assert_eq!(rx_antenna_names()[4], "RX1");
    }

    // trace: FR-MENU-01 — the menu table is non-trivial, id-sorted, searchable.
    #[test]
    fn fr_menu_01_menu_items_and_search() {
        let items = menu_items();
        assert!(items.len() >= 80);
        // Ids are strictly increasing (sorted, unique).
        for pair in items.windows(2) {
            assert!(pair[0].0 < pair[1].0, "menu ids not sorted/unique");
        }
        // A couple of known ids from D12.
        assert!(items.contains(&(36, "LCD Brightness")));
        assert!(items.contains(&(73, "RX Auto Attenuation")));
        // Empty query matches all; a term filters case-insensitively.
        assert_eq!(menu_search("").len(), items.len());
        let agc = menu_search("agc");
        assert!(!agc.is_empty() && agc.len() < items.len());
        assert!(agc
            .iter()
            .all(|&i| items[i].1.to_lowercase().contains("agc")));
        assert!(menu_search("zzzznomatch").is_empty());
    }

    // trace: FR-SW-01 — quick-memory, PF, and radio-switch codes are correct.
    #[test]
    fn fr_sw_01_switch_key_codes() {
        let m = quick_mem_keys();
        assert_eq!(m[0], ("M1", 17, 162));
        assert_eq!(m[3], ("M4", 52, 165));
        assert_eq!(pf_keys()[0], ("PF1", 153));
        assert_eq!(pf_keys().len(), 4);
        assert!(radio_switches().contains(&("SPOT", 42)));
        assert!(radio_switches().contains(&("MON", 128)));
        // TX dual-function switches: tap + hold codes present.
        let tx = tx_function_switches();
        assert_eq!(tx[0], ("TUNE", 16, "TUNE LP", 131));
        assert_eq!(tx[1], ("ATU TUNE", 40, "ATU", 158)); // hold = ATU in/out
        assert_eq!(tx[4], ("VOX", 50, "QSK", 134));
        assert_eq!(tx.len(), 6);
    }

    // trace: FR-UI-19 — the DX prefix list is searchable by prefix or country.
    #[test]
    fn fr_ui_19_dx_search() {
        let all = dx_prefixes();
        assert!(all.len() >= 50);
        assert_eq!(dx_search("").len(), all.len());
        // Search by country substring.
        let germany = dx_search("germany");
        assert_eq!(germany.len(), 1);
        assert_eq!(all[germany[0]], ("DL", "Germany"));
        // Search by prefix.
        assert!(!dx_search("VE").is_empty());
        assert!(dx_search("zzz").is_empty());
    }

    // trace: FR-UI-08 — each mode reports its PAN= label.
    #[test]
    fn fr_ui_08_view_mode_label() {
        assert_eq!(ViewMode::SingleA.label(), "A");
        assert_eq!(ViewMode::SingleB.label(), "B");
        assert_eq!(ViewMode::Dual.label(), "A+B");
    }

    // trace: FR-UI-09 — dot-grouped frequency formatting.
    #[test]
    fn fr_ui_09_freq_dot_grouping() {
        assert_eq!(format_freq_hz(14_070_000), "14.070.000");
        assert_eq!(format_freq_hz(7_045_000), "7.045.000");
        assert_eq!(format_freq_hz(145_230_000), "145.230.000");
        assert_eq!(format_freq_hz(1_000), "1.000");
        assert_eq!(format_freq_hz(500), "500");
        assert_eq!(format_freq_hz(0), "0");
        assert_eq!(format_freq_opt(None), "—.———.———");
        assert_eq!(format_freq_opt(Some(14_061_100)), "14.061.100");
    }

    // trace: FR-UI-10 — semantic colour roles; transmit overrides; A/B distinct.
    #[test]
    fn fr_ui_10_semantic_colour_roles() {
        // RX, not transmitting: A=blue, B=green.
        assert_eq!(vfo_role(false, false, false), ColorRole::VfoA);
        assert_eq!(vfo_role(true, false, false), ColorRole::VfoB);
        // Transmitting on the TX VFO → amber, overriding A/B.
        assert_eq!(vfo_role(false, true, true), ColorRole::TxActive);
        assert_eq!(vfo_role(true, true, true), ColorRole::TxActive);
        // Transmitting but not this VFO (split) keeps its A/B colour.
        assert_eq!(vfo_role(true, false, true), ColorRole::VfoB);

        assert_eq!(toggle_role(true), ColorRole::VfoB);
        assert_eq!(toggle_role(false), ColorRole::Inactive);

        // Every role yields a distinct colour (no two roles collide).
        let roles = [
            ColorRole::TxActive,
            ColorRole::VfoA,
            ColorRole::VfoB,
            ColorRole::RxValue,
            ColorRole::Caution,
            ColorRole::Inactive,
        ];
        for (i, a) in roles.iter().enumerate() {
            for b in &roles[i + 1..] {
                assert_ne!(a.rgb(), b.rgb(), "{a:?} and {b:?} share a colour");
            }
        }
    }

    // trace: FR-UI-11 — two-line state buttons derive (label, value) from state.
    #[test]
    fn fr_ui_11_two_line_state_buttons() {
        assert_eq!(agc_button(Some(1)), ButtonState::new("AGC", "Slow"));
        assert_eq!(agc_button(Some(0)), ButtonState::new("AGC", "Off"));
        assert_eq!(agc_button(None), ButtonState::new("AGC", "—"));

        assert_eq!(
            atten_button(Some(true), Some(6)),
            ButtonState::new("ATT", "6 dB")
        );
        assert_eq!(
            atten_button(Some(false), Some(6)),
            ButtonState::new("ATT", "Off")
        );

        assert_eq!(bandwidth_button(Some(2800)), ButtonState::new("BW", "2.80"));
        assert_eq!(bandwidth_button(Some(500)), ButtonState::new("BW", "0.50"));

        assert_eq!(
            toggle_button("NB", Some(true)),
            ButtonState::new("NB", "On")
        );
        assert_eq!(
            toggle_button("NR", Some(false)),
            ButtonState::new("NR", "Off")
        );
    }

    // trace: FR-UI-13 — context row opens, toggles closed, and is exclusive.
    #[test]
    fn fr_ui_13_context_row_toggle_is_exclusive() {
        let mut ctx = ContextRow::default();
        assert!(ctx.active().is_none());
        assert_eq!(ctx.active(), None);

        // Tapping a fresh row opens the primary.
        let mut pre = ContextRow::default();
        pre.tap(Primary::Band);
        assert_eq!(pre.active(), Some(Primary::Band));
        assert!(pre.is_open(Primary::Band));

        // Tapping a primary opens its row.
        ctx.tap(Primary::Band);
        assert_eq!(ctx.active(), Some(Primary::Band));
        assert!(ctx.is_open(Primary::Band));
        assert!(!ctx.is_open(Primary::Tx));
        assert!(ctx.active().is_some());

        // Tapping a different primary switches (only one open at a time).
        ctx.tap(Primary::Tx);
        assert_eq!(ctx.active(), Some(Primary::Tx));
        assert!(!ctx.is_open(Primary::Band));

        // Tapping the open primary again closes it.
        ctx.tap(Primary::Tx);
        assert!(ctx.active().is_none());

        // Explicit close is idempotent.
        ctx.tap(Primary::Menu);
        ctx.close();
        assert_eq!(ctx.active(), None);
    }

    // trace: FR-UI-19 — every primary maps to a screen; MAIN/SUB RX share RX EQ.
    #[test]
    fn fr_ui_19_screen_kind_per_primary() {
        assert_eq!(screen_kind(Primary::MainRx), ScreenKind::RxEq);
        assert_eq!(screen_kind(Primary::SubRx), ScreenKind::RxEq);
        assert_eq!(screen_kind(Primary::Tx), ScreenKind::TxConfig);
        assert_eq!(screen_kind(Primary::Display), ScreenKind::Display);
        assert_eq!(screen_kind(Primary::Band), ScreenKind::Band);
        assert_eq!(screen_kind(Primary::Fn), ScreenKind::Fn);
        assert_eq!(screen_kind(Primary::Menu), ScreenKind::Menu);
    }

    // trace: FR-UI-13 — seven primaries in K4 order.
    #[test]
    fn fr_ui_13_primaries_order() {
        let labels: Vec<_> = Primary::all().iter().map(|p| p.label()).collect();
        assert_eq!(
            labels,
            ["MENU", "Fn", "DISPLAY", "BAND", "MAIN RX", "SUB RX", "TX"]
        );
    }

    // trace: FR-UI-17 — theme cycles through all four modes and resolves.
    #[test]
    fn fr_ui_17_theme_mode_cycles_and_resolves() {
        // Default is dark; cycle visits all four then wraps.
        assert_eq!(ThemeMode::default(), ThemeMode::Dark);
        let seq = [
            ThemeMode::Dark,
            ThemeMode::Light,
            ThemeMode::Contrast,
            ThemeMode::System,
        ];
        let mut m = ThemeMode::Dark;
        for expected in seq.iter().skip(1).chain(std::iter::once(&ThemeMode::Dark)) {
            m = m.next();
            assert_eq!(&m, expected);
        }
        // Each mode has a distinct label.
        let labels: Vec<_> = seq.iter().map(|m| m.label()).collect();
        assert_eq!(labels, ["Dark", "Light", "Contrast", "System"]);

        // System follows the detected OS preference; the rest are fixed.
        assert_eq!(ThemeMode::System.effective(true), EffectiveTheme::Dark);
        assert_eq!(ThemeMode::System.effective(false), EffectiveTheme::Light);
        assert_eq!(ThemeMode::Light.effective(true), EffectiveTheme::Light);

        // Dark palette matches the base `rgb()` accessors; themes differ.
        assert_eq!(shade_rgb(EffectiveTheme::Dark, Shade::Bg), Shade::Bg.rgb());
        assert_eq!(
            role_rgb(EffectiveTheme::Dark, ColorRole::VfoA),
            ColorRole::VfoA.rgb()
        );
        assert_ne!(
            shade_rgb(EffectiveTheme::Light, Shade::Bg),
            shade_rgb(EffectiveTheme::Dark, Shade::Bg)
        );
        // Light theme reads dark text on a light ground (RxValue is dark).
        let (r, g, b) = role_rgb(EffectiveTheme::Light, ColorRole::RxValue);
        assert!(u32::from(r) + u32::from(g) + u32::from(b) < 300);
    }

    // trace: FR-UI-18 — About shows author, version, a license link, the project
    // URL, and the donate link.
    #[test]
    fn fr_ui_18_about_content() {
        assert!(ABOUT_AUTHOR.contains("DC0SK"));
        assert!(ABOUT_LICENSE.contains("GPL"));
        assert!(ABOUT_LICENSE_URL.starts_with("https://") && ABOUT_LICENSE_URL.contains("gpl-3.0"));
        assert!(ABOUT_URL.contains("github.com/dc0sk/K4remote"));
        assert!(ABOUT_DONATE_URL.contains("paypal.com"));
        // Version comes from the crate metadata and is non-empty.
        assert!(!app_version().is_empty());
    }

    // trace: FR-UI-21 — the app launches in landscape (wider than tall).
    #[test]
    fn fr_ui_21_default_window_is_landscape() {
        let (w, h) = DEFAULT_WINDOW_SIZE;
        assert!(w > h, "default window must be landscape");
    }

    // trace: FR-UI-22 — the connection indicator maps each phase to label+colour.
    #[test]
    fn fr_ui_22_conn_status_by_phase() {
        assert_eq!(conn_status(ConnPhase::Connected).1, ColorRole::VfoB);
        assert_eq!(conn_status(ConnPhase::Connecting).1, ColorRole::TxActive);
        assert_eq!(conn_status(ConnPhase::Disconnected).1, ColorRole::Inactive);
        assert!(conn_status(ConnPhase::Connected).0.contains("CONNECTED"));
    }

    // trace: FR-UI-16 — connect control maps phase → (label, action); the
    // in-flight phase shows a cancel affordance.
    #[test]
    fn fr_ui_16_connect_button_label_and_action() {
        assert_eq!(
            connect_button(ConnPhase::Disconnected),
            ("Connect", ConnectAction::Connect)
        );
        assert_eq!(
            connect_button(ConnPhase::Connecting),
            ("Cancel", ConnectAction::Cancel)
        );
        assert_eq!(
            connect_button(ConnPhase::Connected),
            ("Disconnect", ConnectAction::Disconnect)
        );
        // Default phase is disconnected.
        assert_eq!(ConnPhase::default(), ConnPhase::Disconnected);
    }

    // trace: FR-UI-15 — surface shades are strictly luminance-ordered (depth
    // reads from brightness layering, not chrome).
    #[test]
    fn fr_ui_15_surface_shades_are_layered() {
        let order = [
            Shade::Bg,
            Shade::Panel,
            Shade::Track,
            Shade::Control,
            Shade::ControlHover,
            Shade::Edge,
        ];
        let lum = |s: Shade| {
            let (r, g, b) = s.rgb();
            u32::from(r) + u32::from(g) + u32::from(b)
        };
        for pair in order.windows(2) {
            assert!(
                lum(pair[0]) < lum(pair[1]),
                "{:?} is not darker than {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    // trace: FR-UI-15 — S-meter bar spans S1..S9+60 dB with S9 at −73 dBm.
    #[test]
    fn fr_ui_15_s_meter_fraction_scale() {
        assert_eq!(s_meter_fraction(S_METER_FLOOR_DBM), 0.0);
        assert_eq!(s_meter_fraction(S_METER_CEIL_DBM), 1.0);
        // S9 (−73 dBm) sits 48 dB up a 108 dB face.
        assert!((s_meter_fraction(-73) - 48.0 / 108.0).abs() < 1e-6);
        // Readings off the face are clamped, not overdrawn.
        assert_eq!(s_meter_fraction(-140), 0.0);
        assert_eq!(s_meter_fraction(0), 1.0);
    }

    // trace: FR-UI-12 — banded layout: panes per mode, shared centre box, reflow.
    #[test]
    fn fr_ui_12_band_layout_panes_and_reflow() {
        let wide = 1280.0;

        // Single modes → one pane, no centre box.
        let a = band_layout(wide, ViewMode::SingleA);
        assert_eq!(a.panes, vec![Pane::A]);
        assert!(!a.split_center && !a.stacked);

        let b = band_layout(wide, ViewMode::SingleB);
        assert_eq!(b.panes, vec![Pane::B]);

        // Dual on a wide window → two panes side-by-side with the centre box.
        let d = band_layout(wide, ViewMode::Dual);
        assert_eq!(d.panes, vec![Pane::A, Pane::B]);
        assert!(d.split_center && !d.stacked);
        assert!(!d.panes[0].is_b() && d.panes[1].is_b());

        // Narrow window → stacked, no side-by-side centre box even in dual.
        let narrow = band_layout(NARROW_BREAKPOINT - 1.0, ViewMode::Dual);
        assert!(narrow.stacked && !narrow.split_center);
        assert_eq!(narrow.panes, vec![Pane::A, Pane::B]);

        assert_eq!(Pane::A.label(), "A");
        assert_eq!(Pane::B.label(), "B");
    }
}
