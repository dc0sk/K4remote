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

/// Human-readable, distinguishable reason for a connection failure, keyed by the
/// I/O error kind (FR-CONN-04) — refused / timed-out / unreachable / auth. Pure
/// so the mapping is unit-testable; the worker uses it for the `Error` status.
/// trace: FR-CONN-04
pub fn connect_error_reason(kind: std::io::ErrorKind) -> &'static str {
    use std::io::ErrorKind;
    match kind {
        ErrorKind::ConnectionRefused => "connection refused — no server on that host/port",
        ErrorKind::TimedOut => "connection timed out — host unreachable or filtered",
        ErrorKind::PermissionDenied => "authentication rejected — wrong password",
        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => {
            "connection dropped by the radio"
        }
        ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => {
            "host unreachable — check the network"
        }
        ErrorKind::AddrNotAvailable | ErrorKind::NotFound => "host address not found",
        _ => "connection failed",
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

/// How long a press must last to count as a **hold** rather than a tap.
///
/// D14 p.359: "A tap is a brief press, while a hold is any press longer than
/// about 1/2 second." Matching the radio matters more than picking a nicer
/// number — an operator's muscle memory is calibrated on the front panel
/// (FR-UI-HOLD-01).
pub const HOLD_THRESHOLD: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether a press of `elapsed` duration is a hold.
///
/// A press with no recorded start is a **tap**, not a hold: losing the press
/// time must never silently promote a tap into the destructive-er of the two
/// actions.
///
/// trace: FR-UI-HOLD-01
pub fn is_hold(elapsed: Option<std::time::Duration>) -> bool {
    matches!(elapsed, Some(d) if d >= HOLD_THRESHOLD)
}

/// Noise-blanker button: on/off plus the active filter mode, so the mode the
/// hold cycles is visible without opening anything.
///
/// trace: FR-UI-11, FR-UI-HOLD-01
pub fn nb_button(on: Option<bool>, filter: Option<u8>) -> ButtonState {
    let value = match on {
        Some(true) => format!("On · {}", nb_filter_label(filter)),
        Some(false) => "Off".to_string(),
        None => UNKNOWN.to_string(),
    };
    ButtonState::new("NB", value)
}

/// Next noise-blanker filter mode for a **hold** (`NB`): NONE → NARROW →
/// WIDE → NONE.
///
/// On the radio this lives on the paired `[LEVEL]` switch, not on `[NB]`
/// itself: D14 p.767 lists the receiver row as pairs — `[NB]` and `[LEVEL]`,
/// `[NR]` and `[ADJ]`, `[NTCH]` and `[MANUAL]` — and p.1368 reads "Tap [NB] to
/// turn the noise blanker on/off, or hold [LEVEL] to bring up the noise
/// blanker controls (on/off, filtering mode, and level)". The app draws one
/// control where the radio has two switches, so the hold surfaces the paired
/// switch's function.
///
/// Filtering mode is the part of that pair the app could not otherwise
/// reach: the level already has a slider, and `nb_filter` was parsed into
/// state and passed straight back out on every set, never chosen. D14 p.1370
/// is explicit that it matters — "if you hear audio artifacts such as
/// 'pumping' … try changing the NB filter mode from NONE to NARROW or WIDE".
///
/// trace: FR-UI-HOLD-01
pub fn nb_filter_hold(cur: Option<u8>) -> u8 {
    (cur.unwrap_or(0).min(2) + 1) % 3
}

/// Human label for an `NB` filter mode.
///
/// trace: FR-UI-HOLD-01
pub fn nb_filter_label(mode: Option<u8>) -> &'static str {
    match mode {
        Some(1) => "NAR",
        Some(2) => "WIDE",
        Some(_) => "NONE",
        None => UNKNOWN,
    }
}

/// Next attenuator level for a **hold** (`RA`): +3 dB, wrapping 21 → 0.
///
/// D14 p.1318: "Hold [ATTN] to bring up the attenuator controls (on/off and
/// level). Attenuation varies from 0 to 21 dB in 3 dB steps." The radio opens
/// an adjustment panel; with a mouse, stepping the level directly is the same
/// idea in one gesture.
///
/// A level off the 3 dB grid (a radio in some other state, or a stale
/// read-back) snaps up to the next valid step rather than being preserved.
///
/// trace: FR-UI-HOLD-01
pub fn atten_hold(cur: Option<u8>) -> u8 {
    const MAX: u8 = 21;
    let cur = cur.unwrap_or(0).min(MAX);
    let next = (cur / 3) * 3 + 3;
    if next > MAX {
        0
    } else {
        next
    }
}

/// Next AGC mode for a **tap** (`GT`): slow (1) and fast (2) only.
///
/// D14 p.909: "Selects AGC slow (AGC-S) or fast (AGC-F) for the current
/// operating mode." A tap therefore never lands on off — turning AGC off is
/// the hold function, and reaching it by accident while cycling would leave
/// the receiver wide open with no indication of why.
///
/// trace: FR-UI-HOLD-01
pub fn agc_tap(cur: Option<u8>) -> u8 {
    if cur == Some(1) {
        2
    } else {
        1
    }
}

/// Next AGC mode for a **hold** (`GT`): on/off, restoring slow when switching
/// back on.
///
/// D14 p.909: "Holding this button turns AGC on or off (AGC-)." With AGC off
/// the K4 falls back to an audio limiter (D14 p.911), so this is a real
/// operating mode rather than a mistake — but it belongs behind the hold.
///
/// trace: FR-UI-HOLD-01
pub fn agc_hold(cur: Option<u8>) -> u8 {
    if cur == Some(0) {
        1
    } else {
        0
    }
}

/// Panadapter noise-blanker button (`#NB`): 0 off, 1 on, 2 auto.
///
/// `auto` makes the pan NB follow the radio NB on/off, with the levels still
/// independent (D12 `#NB` NOTE), so it is worth showing distinctly from a
/// plain "on".
///
/// trace: FR-PAN-CTL-01
pub fn pan_nb_button(mode: u8) -> ButtonState {
    ButtonState::new(
        "PAN NB",
        match mode {
            0 => "Off",
            1 => "On",
            _ => "Auto",
        },
    )
}

/// Generic on/off control button (NB/NR/preamp/RIT/XIT…).
/// trace: FR-UI-11
/// A toggle button the radio has reported it cannot honour right now, shown as
/// `N/A` rather than a live-looking On/Off. Used for the mini-pan, which the K4
/// refuses under some display settings (`#MP$-1`, D12).
///
/// trace: FR-UI-11, FR-UI-14
pub fn unavailable_button(label: &'static str) -> ButtonState {
    ButtonState::new(label, "N/A")
}

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

// --- Optimistic-UI reconciliation (pure, testable) --------------------------

/// Optimistic VFO frequency override (FR-VFO-03/08): a local value shown
/// instantly when the operator sets/steps a VFO, then dropped once the radio
/// confirms it — or after a staleness timeout, so a value the radio clamps or
/// rejects falls back to the real read-back instead of sticking forever.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OptVfo {
    a: Option<u64>,
    b: Option<u64>,
    age: u8,
}

impl OptVfo {
    /// Ticks to hold an unconfirmed optimistic value (~2 s at 150 ms/tick).
    pub const STALE_TICKS: u8 = 15;

    /// Set an optimistic value for VFO A / B, resetting the staleness clock.
    pub fn set_a(&mut self, hz: u64) {
        self.a = Some(hz);
        self.age = 0;
    }
    pub fn set_b(&mut self, hz: u64) {
        self.b = Some(hz);
        self.age = 0;
    }

    /// Displayed VFO A / B: the pending optimistic value, else the snapshot.
    pub fn a_or(&self, snapshot: Option<u64>) -> Option<u64> {
        self.a.or(snapshot)
    }
    pub fn b_or(&self, snapshot: Option<u64>) -> Option<u64> {
        self.b.or(snapshot)
    }

    /// Reconcile one tick against the latest radio snapshot: drop a value the
    /// radio has confirmed (snapshot matches), and expire everything once the
    /// unconfirmed value has been held longer than [`OptVfo::STALE_TICKS`].
    ///
    /// trace: FR-VFO-08
    pub fn reconcile(&mut self, snap_a: Option<u64>, snap_b: Option<u64>) {
        if self.a == snap_a {
            self.a = None;
        }
        if self.b == snap_b {
            self.b = None;
        }
        if self.a.is_some() || self.b.is_some() {
            self.age = self.age.saturating_add(1);
            if self.age > Self::STALE_TICKS {
                self.a = None;
                self.b = None;
            }
        } else {
            self.age = 0;
        }
    }
}

/// "Adopt on genuine change" reconciler (the `last_split` / `last_pwr_range`
/// pattern): update `last` and return the value to adopt only when the snapshot
/// has genuinely changed. A static or absent read-back returns `None`, so a
/// just-clicked optimistic local value is never snapped back by a lagging echo.
pub fn adopt_on_change<T: Copy + PartialEq>(last: &mut Option<T>, current: Option<T>) -> Option<T> {
    if current != *last {
        *last = current;
        current
    } else {
        None
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

/// Mode-varying TX switch-row controls (VOX/QSK cells + AUTOSPOT). Other
/// per-mode TX controls (VOX gain, DVR, compression, mic gain, keyer timing)
/// are shown/hidden structurally by the TX mode strip, not dimmed via this map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxCtl {
    Qsk,
    Vox,
    Autospot,
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each connection phase reports a distinct status label to the UI, and each
    /// failure kind maps to a distinguishable human-readable reason.
    ///
    /// trace: FR-CONN-03, FR-CONN-04
    #[test]
    fn fr_conn_03_04_states_and_failure_reasons_are_distinct() {
        use std::io::ErrorKind;
        // FR-CONN-03: every phase yields its own status label.
        let labels: Vec<&str> = [
            ConnPhase::Disconnected,
            ConnPhase::Connecting,
            ConnPhase::Connected,
        ]
        .iter()
        .map(|p| conn_status(*p).0)
        .collect();
        let unique: std::collections::BTreeSet<&&str> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "each phase → distinct label");
        // FR-CONN-04: the named failure kinds each map to their own reason.
        let kinds = [
            ErrorKind::ConnectionRefused,
            ErrorKind::TimedOut,
            ErrorKind::PermissionDenied,
            ErrorKind::HostUnreachable,
        ];
        let reasons: std::collections::BTreeSet<&str> =
            kinds.iter().map(|k| connect_error_reason(*k)).collect();
        assert_eq!(
            reasons.len(),
            kinds.len(),
            "each failure kind → distinct reason"
        );
        assert!(connect_error_reason(ErrorKind::PermissionDenied).contains("auth"));
    }

    /// Optimistic VFO: shows the local value, drops it on confirm, and expires
    /// a never-confirmed value after the staleness window.
    ///
    /// trace: FR-VFO-08, FR-VFO-03
    #[test]
    fn fr_vfo_08_optimistic_reconcile() {
        let mut o = OptVfo::default();
        // No override → shows the snapshot.
        assert_eq!(o.a_or(Some(100)), Some(100));
        // Set → shows the optimistic value over a stale snapshot.
        o.set_a(14_000_000);
        assert_eq!(o.a_or(Some(100)), Some(14_000_000));
        // Radio confirms → optimistic value is dropped, age resets.
        o.reconcile(Some(14_000_000), None);
        assert_eq!(o.a_or(Some(14_000_000)), Some(14_000_000));
        assert_eq!(o, OptVfo::default());

        // A value the radio never confirms expires after STALE_TICKS.
        o.set_b(7_000_000);
        for _ in 0..=OptVfo::STALE_TICKS {
            o.reconcile(Some(0), Some(0)); // snapshot never matches 7 MHz
        }
        assert_eq!(
            o.b_or(Some(0)),
            Some(0),
            "stale optimistic value must expire"
        );
    }

    /// adopt_on_change adopts only on a genuine snapshot transition.
    #[test]
    fn adopt_on_change_only_on_transition() {
        let mut last = None;
        assert_eq!(adopt_on_change(&mut last, Some(true)), Some(true)); // first read
        assert_eq!(adopt_on_change(&mut last, Some(true)), None); // unchanged echo
        assert_eq!(adopt_on_change(&mut last, Some(false)), Some(false)); // genuine change
        assert_eq!(adopt_on_change(&mut last, None), None); // cleared read-back adopts nothing
        assert_eq!(last, None); // ...but `last` still tracks it
    }

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
        // QSK/AUTOSPOT CW-only; VOX everywhere but CW.
        assert_eq!(tx_ctl_vis(TxCtl::Qsk, ModeClass::Cw), Vis::Show);
        assert_eq!(tx_ctl_vis(TxCtl::Qsk, ModeClass::Voice), Vis::Hide);
        assert_eq!(tx_ctl_vis(TxCtl::Autospot, ModeClass::Cw), Vis::Show);
        assert_eq!(tx_ctl_vis(TxCtl::Vox, ModeClass::Cw), Vis::Hide);
        assert_eq!(tx_ctl_vis(TxCtl::Vox, ModeClass::Voice), Vis::Show);
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

#[cfg(test)]
mod taphold_tests {
    use super::*;
    use std::time::Duration;

    /// The threshold is the radio's own: about half a second (D14 p.359).
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_threshold_matches_the_radio() {
        assert_eq!(HOLD_THRESHOLD, Duration::from_millis(500));
    }

    /// A press at or past the threshold is a hold; anything shorter is a tap.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_classifies_press_duration() {
        assert!(!is_hold(Some(Duration::from_millis(0))));
        assert!(!is_hold(Some(Duration::from_millis(120))), "a normal click");
        assert!(!is_hold(Some(Duration::from_millis(499))));
        assert!(
            is_hold(Some(Duration::from_millis(500))),
            "boundary is a hold"
        );
        assert!(is_hold(Some(Duration::from_millis(900))));
        assert!(is_hold(Some(Duration::from_secs(5))), "a very long press");
    }

    /// A lost press time degrades to a tap. Holds carry the more surprising
    /// action (AGC off, ATU bypass), so an unknown duration must never be
    /// promoted into one.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_unknown_press_is_a_tap() {
        assert!(!is_hold(None));
    }
}

#[cfg(test)]
mod agc_taphold_tests {
    use super::*;

    /// A tap only ever selects slow or fast — never off (D14 p.909).
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_agc_tap_never_reaches_off() {
        assert_eq!(agc_tap(Some(1)), 2, "slow → fast");
        assert_eq!(agc_tap(Some(2)), 1, "fast → slow");
        assert_eq!(agc_tap(Some(0)), 1, "off → slow, i.e. tapping restores AGC");
        assert_eq!(agc_tap(None), 1, "unknown → slow");
        // The property that matters: no tap can turn AGC off.
        for cur in [None, Some(0), Some(1), Some(2), Some(9)] {
            assert_ne!(agc_tap(cur), 0, "cur={cur:?} must not tap into AGC off");
        }
    }

    /// A hold toggles AGC off and back on, restoring slow.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_agc_hold_toggles_off() {
        assert_eq!(agc_hold(Some(1)), 0, "slow → off");
        assert_eq!(agc_hold(Some(2)), 0, "fast → off");
        assert_eq!(agc_hold(Some(0)), 1, "off → slow");
        assert_eq!(agc_hold(None), 0);
    }

    /// Tap and hold are different actions whenever AGC is on — the point of
    /// the convention.
    ///
    /// They deliberately coincide from the **off** state: D14's tap "selects
    /// slow or fast" and its hold "turns AGC on", and both of those mean slow
    /// when starting from off. So an operator who cannot remember which
    /// gesture restores AGC gets it back either way, which is the forgiving
    /// direction.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_tap_and_hold_differ_while_agc_is_on() {
        for cur in [Some(1), Some(2)] {
            assert_ne!(agc_tap(cur), agc_hold(cur), "cur={cur:?}");
        }
        // From off, both restore AGC rather than one of them being inert.
        assert_eq!(agc_tap(Some(0)), agc_hold(Some(0)));
        assert_ne!(agc_tap(Some(0)), 0, "neither gesture leaves AGC off");
    }
}

#[cfg(test)]
mod atten_hold_tests {
    use super::*;

    /// The hold walks the documented 0–21 dB ladder in 3 dB steps and wraps.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_atten_hold_steps_3db() {
        let ladder = [0u8, 3, 6, 9, 12, 15, 18, 21];
        for pair in ladder.windows(2) {
            assert_eq!(
                atten_hold(Some(pair[0])),
                pair[1],
                "{} → {}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(atten_hold(Some(21)), 0, "wraps at the top");
        assert_eq!(atten_hold(None), 3, "unknown starts the ladder");
    }

    /// Every reachable level is on the documented grid and within range —
    /// including from a level the radio should never report.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_atten_hold_stays_on_the_grid() {
        for cur in 0u8..=255 {
            let n = atten_hold(Some(cur));
            assert!(n <= 21, "cur={cur} → {n} exceeds the 21 dB maximum");
            assert_eq!(n % 3, 0, "cur={cur} → {n} is off the 3 dB grid");
        }
    }

    /// Off-grid input snaps up to the next valid step rather than being
    /// carried forward.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_atten_hold_snaps_off_grid_values() {
        assert_eq!(atten_hold(Some(1)), 3);
        assert_eq!(atten_hold(Some(4)), 6);
        assert_eq!(atten_hold(Some(20)), 21);
    }

    /// Repeated holds visit the whole ladder and return to the start — no
    /// level is unreachable and none is a dead end.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_atten_hold_cycles_every_level() {
        let mut seen = Vec::new();
        let mut cur = 0u8;
        for _ in 0..8 {
            cur = atten_hold(Some(cur));
            seen.push(cur);
        }
        assert_eq!(seen, vec![3, 6, 9, 12, 15, 18, 21, 0]);
    }
}

#[cfg(test)]
mod nb_filter_tests {
    use super::*;

    /// The hold cycles the three documented filter modes and returns to NONE.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_nb_filter_cycles() {
        assert_eq!(nb_filter_hold(Some(0)), 1, "NONE → NARROW");
        assert_eq!(nb_filter_hold(Some(1)), 2, "NARROW → WIDE");
        assert_eq!(nb_filter_hold(Some(2)), 0, "WIDE → NONE");
        assert_eq!(nb_filter_hold(None), 1, "unknown starts the cycle");
    }

    /// Only the three modes the `NB` command defines are ever produced, even
    /// from a value the radio should never report.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_nb_filter_stays_in_range() {
        for cur in 0u8..=255 {
            assert!(nb_filter_hold(Some(cur)) <= 2, "cur={cur}");
        }
    }

    /// Three holds return to where they started — no mode is unreachable and
    /// none is a dead end.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_nb_filter_round_trips() {
        let mut cur = 0u8;
        let mut seen = Vec::new();
        for _ in 0..3 {
            cur = nb_filter_hold(Some(cur));
            seen.push(cur);
        }
        assert_eq!(seen, vec![1, 2, 0]);
    }

    /// Labels track the modes, and an unknown mode is not shown as NONE —
    /// "we have not heard from the radio" and "the filter is off" are
    /// different things.
    /// trace: FR-UI-HOLD-01
    #[test]
    fn fr_ui_hold_01_nb_filter_labels() {
        assert_eq!(nb_filter_label(Some(0)), "NONE");
        assert_eq!(nb_filter_label(Some(1)), "NAR");
        assert_eq!(nb_filter_label(Some(2)), "WIDE");
        assert_eq!(nb_filter_label(None), UNKNOWN);
        assert_ne!(nb_filter_label(None), nb_filter_label(Some(0)));
    }
}
