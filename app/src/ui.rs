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
    /// **On air** — RF is leaving the antenna right now (red). Distinct from
    /// [`ColorRole::TxActive`], which is the amber of "armed and ready"
    /// (FR-UI-TX-01).
    OnAir,
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
            ColorRole::OnAir => (0xE0, 0x22, 0x18),    // red
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
    /// Every label [`ThemeMode::label`] can return, for width reservation
    /// (`FR-UI-STABLE-01`).
    pub const LABELS: [&'static str; 4] = ["Dark", "Light", "Contrast", "System"];

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
            ColorRole::OnAir => (0xC0, 0x14, 0x0C),
            ColorRole::Inactive => (0x7A, 0x80, 0x88),
        },
        EffectiveTheme::Contrast => match role {
            ColorRole::TxActive => (0xFF, 0xB0, 0x2E),
            ColorRole::VfoA => (0x4D, 0xB1, 0xFF),
            ColorRole::VfoB => (0x3D, 0xF0, 0x7A),
            ColorRole::RxValue => (0xFF, 0xFF, 0xFF),
            ColorRole::Caution => (0xFF, 0xEE, 0x00),
            ColorRole::OnAir => (0xFF, 0x3B, 0x30),
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
/// Every label [`connect_button`] can return, for width reservation
/// (`FR-UI-STABLE-01`).
///
/// Kept beside the function rather than at the call site so the two cannot
/// drift: a phase added without extending this list is caught by a test, not
/// by an operator noticing the row twitch.
pub const CONNECT_LABELS: [&str; 3] = ["Connect", "Cancel", "Disconnect"];

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
/// Every label [`conn_status`] can return, for width reservation.
pub const CONN_STATUS_LABELS: [&str; 3] = ["CONNECTED", "connecting...", "disconnected"];

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

/// Placeholder for a value the radio has not reported yet — one em dash, so a
/// popup and the chip it opened from say "unknown" the same way.
pub const UNKNOWN: &str = "—";

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

/// Width to reserve for a control whose label varies over a **known** set, so
/// it does not resize as the label changes (FR-UI-STABLE-01).
///
/// A button that reads `MUTE` and then `MUTED`, or a readout going from `7%`
/// to `100%`, changes width — and everything beside it shifts. Across a panel
/// of such controls the layout twitches continuously while the radio is being
/// operated, which is distracting and makes controls harder to hit.
///
/// Sizing to the widest member fixes it wherever the set is known at compile
/// time, which is the usual case: on/off labels, mode names, a percentage
/// 0-100. Not a substitute for a fixed width where the content is genuinely
/// unbounded (a callsign, a filter string).
///
/// The estimate is deliberately crude — longest label times a per-character
/// advance, plus padding. It only has to be a stable upper bound, not a
/// typographic measurement.
///
/// trace: FR-UI-STABLE-01
pub fn stable_label_width(labels: &[&str], text_size: f32, padding: f32) -> f32 {
    labels
        .iter()
        .map(|l| l.chars().map(char_em).sum::<f32>())
        .fold(0.0f32, f32::max)
        * text_size
        + padding
}

/// Approximate width of one character, in em.
///
/// A single factor for every character was wrong, and wrong in a way that took
/// a while to show: it was calibrated at 0.78 for **capitals**, because
/// `DISARM` wrapped inside its own reservation at 0.62. Applied to digits and
/// lower case — which is what the slider readouts are — that over-reserved by
/// about a quarter. Six readouts in the gain row overshot by ~108 px between
/// them, the row overflowed, and the `SHIFT` readout at the end was squeezed
/// to one character per line.
///
/// Every weight errs on the wide side of what the font actually renders:
/// under-reserving breaks the layout worse than the resizing this prevents.
fn char_em(c: char) -> f32 {
    match c {
        'A'..='Z' => 0.78,
        '0'..='9' => 0.62,
        'a'..='z' => 0.58,
        ' ' => 0.30,
        '.' | ',' | ':' | ';' | '\'' | '·' | '|' => 0.35,
        '%' | '+' | '-' | '/' | '(' | ')' => 0.55,
        // An em dash is an em wide by definition, and the placeholder uses one.
        '—' => 1.0,
        _ => 0.78,
    }
}

/// The `VT` tuning-step index for a frequency digit's **place value**.
///
/// The K4 numbers its tuning rates 0–5 for 1 Hz, 10 Hz, 100 Hz, 1 kHz, 10 kHz
/// and 100 kHz, so the index is simply the place's power of ten. Returns
/// `None` for a place the radio has no rate for — the MHz digits and above,
/// which select a band rather than a tuning rate.
///
/// trace: FR-VFO-STEP-01
pub fn tune_step_index(place: u64) -> Option<u8> {
    if place == 0 {
        return None;
    }
    let index = place.ilog10();
    // A place value is a power of ten; anything else is a caller passing a
    // frequency rather than a digit position.
    if 10u64.checked_pow(index) != Some(place) {
        return None;
    }
    (index <= 5).then_some(index as u8)
}

/// The **alternate** of a mode: its reverse or opposite-sideband partner, as
/// the K4's own mode-button group pairs them (D12 `MA`, "Mode Alternates":
/// CW normal/reverse, USB/LSB, DATA-A normal/reverse).
///
/// Returns the `MD` digit to send, or `None` for a mode that has no partner —
/// AM and FM stand alone, so tapping them again has nothing to switch to.
///
/// This is what "tap the mode you are already in" should do. An earlier
/// attempt used `MD/`, D12's toggle between the two *most recently used*
/// modes; that made a button labelled `CW` jump to whatever you happened to be
/// in before, and left CW-R reachable only from its own button.
///
/// trace: FR-UI-ALT-01
pub fn alternate_of(mode: &str) -> Option<u8> {
    match mode {
        "LSB" => Some(2),    // USB
        "USB" => Some(1),    // LSB
        "CW" => Some(7),     // CW-R
        "CW-R" => Some(3),   // CW
        "DATA" => Some(9),   // DATA-R
        "DATA-R" => Some(6), // DATA
        _ => None,           // AM, FM: no alternate
    }
}

/// Whether a key press should trigger the **emergency stop**.
///
/// Two routes, and the distinction between them matters:
///
/// * **`ESC` while on air.** The biggest, most isolated key on the board,
///   findable without looking, universally "get me out", and colliding with
///   nothing. Off air it keeps dismissing popups and dialogs. This is a gate
///   on *state*, not on *focus* — which is why it is sound where a
///   focus-dependent binding would not be: focus is unrelated to whether the
///   operator needs to stop, whereas "not on air" is precisely the condition
///   under which there is nothing to stop. The gate can never withhold the
///   function at the moment it is wanted.
///
/// * **`Ctrl+Shift+X`, unconditional.** The backstop for the one hole in the
///   above: if the app's idea of on-air is stale — radio keyed, snapshot not
///   yet caught up — `ESC` would not fire. This one does not consult state at
///   all.
///
/// `Ctrl+C` is deliberately *not* used: it would take copy away app-wide, and
/// an arbitrary chord is the wrong thing to have to recall under stress.
///
/// trace: FR-TX-SAFE-05
pub fn is_estop_press(
    key: &iced::keyboard::Key,
    mods: iced::keyboard::Modifiers,
    on_air: bool,
) -> bool {
    use iced::keyboard::{key::Named, Key};
    if matches!(key, Key::Named(Named::Escape)) && on_air {
        return true;
    }
    let is_x = match key {
        Key::Character(c) => c.as_str().eq_ignore_ascii_case("x"),
        _ => false,
    };
    is_x && (mods.control() || mods.command()) && mods.shift()
}

/// Whether the radio is **on air** by any route.
///
/// Not the same as the mic path being open. A tune emits a carrier without
/// `local_tx` being set — deliberately, so `send_tx_audio` stays closed and
/// the operator's microphone is not streamed over the tune (see
/// `Session::tune`). The indicator must nevertheless light, because what it
/// answers is "is RF going out", not "is my voice going out".
///
/// `radio_tx` is the **radio's own** report (the `IF` `t` flag), and it is the
/// only input that covers transmission this app did not start: a front-panel
/// PTT, VOX, the K-Pod, or a switch tap. The switch-row `TUNE`/`TUNE LP`
/// buttons are exactly that case — they send raw switch taps rather than
/// going through `Session::tune`, so `tuning` stays false while the radio is
/// keyed. Judging "on air" from local intent alone left the emergency stop
/// (`FR-TX-SAFE-05`) inert against every one of those routes, which was found
/// on a live radio: `TUNE` from the switch row, then `ESC`, did nothing.
///
/// `None` means the radio has not reported yet and is treated as *not* on air
/// — the local flags still cover anything this app initiated, so the stop is
/// never worse than before the radio answers.
///
/// trace: FR-UI-TX-01, FR-TX-SAFE-05
pub fn on_air(local_tx: bool, tuning: bool, radio_tx: Option<bool>) -> bool {
    local_tx || tuning || radio_tx == Some(true)
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

/// Attenuator ladder (`RA`), D14 p.1318: "Attenuation varies from 0 to 21 dB
/// in 3 dB steps." One source of truth for the hold ([`atten_hold`]) and the
/// popup slider, which must not offer a level the radio cannot hold.
pub const ATTEN_MAX_DB: u8 = 21;
/// Step of the attenuator ladder — see [`ATTEN_MAX_DB`].
pub const ATTEN_STEP_DB: u8 = 3;

/// Snap a free-running slider position onto the attenuator's 3 dB grid.
///
/// The popup's slider moves continuously under the mouse, but the radio only
/// has the eight documented levels. Snapping to the **nearest** step (rather
/// than truncating, as [`atten_hold`] does when leaving an off-grid state)
/// keeps the knob feeling like it follows the pointer instead of lagging it.
///
/// trace: FR-UI-POPUP-01
pub fn atten_snap(db: u8) -> u8 {
    let db = db.min(ATTEN_MAX_DB);
    let step = ATTEN_STEP_DB;
    let snapped = ((db + step / 2) / step) * step;
    snapped.min(ATTEN_MAX_DB)
}

/// Human label for an `AP` audio-peaking-filter bandwidth (D12 `AP`).
///
/// trace: FR-UI-POPUP-01
/// Every label [`apf_width_label`] can return, for width reservation
/// (`FR-UI-STABLE-01`). `150` is the widest; the unknown placeholder is the
/// narrowest, so a disconnected app must not size the control from it.
pub const APF_WIDTH_LABELS: [&str; 4] = ["30", "50", "150", UNKNOWN];

pub fn apf_width_label(width: Option<u8>) -> &'static str {
    match width {
        Some(0) => "30",
        Some(1) => "50",
        Some(2) => "150",
        _ => UNKNOWN,
    }
}

/// A receiver chip whose K4 counterpart opens a settings panel of its own.
///
/// D14 puts the *adjustment* of these behind the paired switch's hold — "hold
/// [LEVEL] to bring up the noise blanker controls (on/off, filtering mode, and
/// level)" (p.1368), "Hold [ATTN] to bring up the attenuator controls (on/off
/// and level)" (p.1318). The app draws one chip where the radio has two
/// switches, so the panel is what the chip's popup shows.
///
/// trace: FR-UI-POPUP-01
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RxPopup {
    Atten,
    Preamp,
    Agc,
    Nb,
    Nr,
    Notch,
    Apf,
}

/// Gap between the pointer and the popup's corner, and the minimum gap kept
/// between the popup and the window edge.
const POPUP_GAP: f32 = 6.0;

impl RxPopup {
    /// Approximate rendered size of this popup's card, in logical pixels.
    ///
    /// Used only to keep the card inside the window ([`popup_origin`]). iced
    /// lays the card out for real, so an estimate that is slightly too large
    /// costs a few pixels of margin, while one that is too small would let an
    /// edge run off-screen — so these round **up**.
    ///
    /// trace: FR-UI-POPUP-01
    pub fn size(self) -> (f32, f32) {
        match self {
            // on/off + three filter buttons, and a level slider beneath
            Self::Nb => (440.0, 120.0),
            // on/off + three width buttons on one row
            Self::Apf => (420.0, 90.0),
            // label + slider + reading + OUT
            Self::Atten => (400.0, 90.0),
            // on/off + label + slider + reading
            Self::Nr | Self::Notch => (400.0, 90.0),
            // a label and three or four buttons
            Self::Preamp | Self::Agc => (320.0, 90.0),
        }
    }

    /// Title shown on the popup card — the radio's own name for the panel,
    /// spelled out rather than abbreviated as on the chip.
    ///
    /// trace: FR-UI-POPUP-01
    pub fn title(self) -> &'static str {
        match self {
            Self::Atten => "ATTENUATOR",
            Self::Preamp => "PREAMP",
            Self::Agc => "AGC",
            Self::Nb => "NOISE BLANKER",
            Self::Nr => "NOISE REDUCTION",
            Self::Notch => "MANUAL NOTCH",
            Self::Apf => "AUDIO PEAKING FILTER",
        }
    }
}

/// Where to put a popup opened at `cursor`, so it appears **at the control**
/// rather than in the middle of the window, while staying fully on screen.
///
/// Returns the card's top-left corner. The popup is offset down-right of the
/// pointer like a context menu; when that would push it past an edge it is
/// pulled back inside, and a popup larger than the window is pinned to the
/// top-left rather than being pushed off the opposite edge.
///
/// trace: FR-UI-POPUP-01
pub fn popup_origin(cursor: (f32, f32), size: (f32, f32), window: (f32, f32)) -> (f32, f32) {
    let place = |pos: f32, extent: f32, bound: f32| -> f32 {
        // The furthest left/top edge that still leaves the card fully inside.
        let limit = bound - extent - POPUP_GAP;
        // `max(GAP)` last: if the card cannot fit at all, keeping the top-left
        // corner visible beats centring the overflow across both edges.
        (pos + POPUP_GAP).min(limit).max(POPUP_GAP)
    };
    (
        place(cursor.0, size.0, window.0),
        place(cursor.1, size.1, window.1),
    )
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

/// Optimistic override for a **level** the operator just set — the attenuator
/// (FR-UI-POPUP-01), same confirm-or-expire contract as [`OptVfo`].
///
/// Without this, a read-back that was already in flight when the level was set
/// reports the *previous* level, and the next resync copies it over the
/// operator's choice: the slider jumps back, intermittently, depending purely
/// on whether a poll happened to be in the air. Holding until the radio
/// **confirms** fixes that; expiring on staleness means a level the radio
/// rejects still falls back to reality rather than sticking forever.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opt<T> {
    pending: Option<T>,
    age: u8,
}

/// Byte-valued override — the original, and still the common case.
pub type OptLevel = Opt<u8>;

impl<T> Default for Opt<T> {
    fn default() -> Self {
        Self {
            pending: None,
            age: 0,
        }
    }
}

impl<T: Copy + PartialEq> Opt<T> {
    /// Ticks to hold an unconfirmed value. Longer than [`OptVfo::STALE_TICKS`]:
    /// the attenuator is reconciled by the ~3 s resync rather than by a
    /// per-tick read-back, so it needs to outlive one resync interval.
    pub const STALE_TICKS: u8 = 40;

    /// Record a locally-set level, restarting the staleness clock.
    pub fn set(&mut self, v: T) {
        self.pending = Some(v);
        self.age = 0;
    }

    /// Whether a local value is currently being held (so the resync must not
    /// overwrite the mirror).
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// The value to display: the pending local one if there is one, else what
    /// the radio reported. For controls read straight from the snapshot rather
    /// than kept in a local mirror.
    pub fn or(&self, reported: Option<T>) -> Option<T> {
        self.pending.or(reported)
    }

    /// Reconcile one tick against the radio's reported level: drop the
    /// override once the radio agrees, and expire it if it never does.
    pub fn reconcile(&mut self, snapshot: Option<T>) {
        if self.pending.is_some() && self.pending == snapshot {
            self.pending = None;
            self.age = 0;
            return;
        }
        if self.pending.is_some() {
            self.age = self.age.saturating_add(1);
            if self.age > Self::STALE_TICKS {
                self.pending = None;
                self.age = 0;
            }
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
mod opt_level_tests {
    use super::*;

    /// The reported fault: a read-back that predates the operator's change
    /// must not be adopted. Here the radio is still reporting 21 dB when the
    /// operator sets 6 — the override has to survive that tick.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_stale_readback_does_not_revert_the_operator() {
        let mut o = OptLevel::default();
        o.set(6);
        o.reconcile(Some(21)); // in-flight reply, from before the set
        assert!(o.is_pending(), "a stale 21 dB must not clear the override");
        o.reconcile(Some(21));
        assert!(o.is_pending(), "still held while the radio disagrees");
    }

    /// Once the radio confirms, the override is dropped so the radio is
    /// authoritative again — including when the operator changes it at the rig.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_confirmation_releases_the_override() {
        let mut o = OptLevel::default();
        o.set(6);
        o.reconcile(Some(6));
        assert!(!o.is_pending(), "confirmed → the radio drives again");

        // And a later radio-side change is not blocked by a stale override.
        o.reconcile(Some(12));
        assert!(!o.is_pending());
    }

    /// A level the radio never accepts must not stick forever — it expires,
    /// so the UI ends up showing what the radio actually has.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_unconfirmed_override_expires() {
        let mut o = OptLevel::default();
        o.set(9);
        for _ in 0..=OptLevel::STALE_TICKS {
            o.reconcile(Some(21)); // radio rejects it, keeps reporting 21
        }
        assert!(
            !o.is_pending(),
            "an override the radio never confirms must expire"
        );
    }

    /// The hold must outlive one resync interval, or the very read-back it
    /// exists to survive would arrive after it expired.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_hold_outlives_a_resync_interval() {
        // The tick is 100 ms; the settings resync runs about every 3 s.
        let hold_ms = u32::from(OptLevel::STALE_TICKS) * 100;
        assert!(
            hold_ms > 3_000,
            "hold is {hold_ms} ms, shorter than the ~3 s resync it must survive"
        );
    }

    /// Setting again restarts the clock — a drag is many sets, and the last
    /// one deserves a full hold rather than inheriting the first one's age.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_setting_again_restarts_the_clock() {
        let mut o = OptLevel::default();
        o.set(3);
        for _ in 0..OptLevel::STALE_TICKS {
            o.reconcile(Some(21));
        }
        o.set(9); // the operator moves it again, just before expiry
        o.reconcile(Some(21));
        assert!(o.is_pending(), "the new value must get a full hold");
    }
}

#[cfg(test)]
mod stable_width_tests {
    use super::*;

    /// The reserved width follows the **longest** label, so the control does
    /// not resize when it switches to it.
    /// trace: FR-UI-STABLE-01
    #[test]
    fn fr_ui_stable_01_width_follows_the_longest_label() {
        assert!(
            stable_label_width(&["MUTE", "MUTED"], 10.0, 14.0)
                >= stable_label_width(&["MUTED"], 10.0, 14.0),
            "must fit the longest member"
        );
        // Order must not matter: the set is what counts, not which is current.
        assert_eq!(
            stable_label_width(&["MUTE", "MUTED"], 10.0, 14.0),
            stable_label_width(&["MUTED", "MUTE"], 10.0, 14.0)
        );
    }

    /// Adding a longer label widens the reservation; adding a shorter one does
    /// not narrow it. A control must never shrink below what it may display.
    /// trace: FR-UI-STABLE-01
    #[test]
    fn fr_ui_stable_01_width_is_monotonic_in_the_label_set() {
        let base = stable_label_width(&["ATU", "ATU BYP"], 13.0, 20.0);
        assert!(stable_label_width(&["ATU", "ATU BYP", "ATU AUTO"], 13.0, 20.0) > base);
        assert_eq!(
            stable_label_width(&["ATU", "ATU BYP", "ON"], 13.0, 20.0),
            base
        );
    }

    /// A percentage readout is the common case: 0-100 % spans one to four
    /// characters, and sizing for "100%" keeps the row still as it counts up.
    /// trace: FR-UI-STABLE-01
    #[test]
    fn fr_ui_stable_01_percentage_readout_is_sized_for_its_widest() {
        let widest = stable_label_width(&["100%"], 11.0, 0.0);
        for pct in [0u8, 7, 42, 99, 100] {
            let s = format!("{pct}%");
            assert!(
                stable_label_width(&[s.as_str()], 11.0, 0.0) <= widest,
                "{s} must fit inside the reservation for 100%"
            );
        }
    }

    /// An empty set is not a panic — a control with nothing defined yet just
    /// gets its padding.
    /// trace: FR-UI-STABLE-01
    #[test]
    fn fr_ui_stable_01_empty_label_set_is_safe() {
        assert_eq!(stable_label_width(&[], 12.0, 8.0), 8.0);
    }
}

#[cfg(test)]
mod tune_step_tests {
    use super::*;

    /// Each digit maps to the radio's rate for its place value.
    /// trace: FR-VFO-STEP-01
    #[test]
    fn fr_vfo_step_01_place_maps_to_the_radios_rate() {
        assert_eq!(tune_step_index(1), Some(0), "1 Hz");
        assert_eq!(tune_step_index(10), Some(1), "10 Hz");
        assert_eq!(tune_step_index(100), Some(2), "100 Hz");
        assert_eq!(tune_step_index(1_000), Some(3), "1 kHz");
        assert_eq!(tune_step_index(10_000), Some(4), "10 kHz");
        assert_eq!(tune_step_index(100_000), Some(5), "100 kHz");
    }

    /// The MHz digits and above have no tuning rate — they select a band, so
    /// tapping one must not try to set a rate the radio does not have.
    /// trace: FR-VFO-STEP-01
    #[test]
    fn fr_vfo_step_01_places_above_the_ladder_have_no_rate() {
        assert_eq!(tune_step_index(1_000_000), None, "1 MHz");
        assert_eq!(tune_step_index(10_000_000), None);
        assert_eq!(tune_step_index(100_000_000), None);
    }

    /// A place that is not a power of ten is not a digit position at all —
    /// guard against a caller passing a frequency rather than a place value.
    /// trace: FR-VFO-STEP-01
    #[test]
    fn fr_vfo_step_01_non_place_values_are_rejected() {
        assert_eq!(tune_step_index(0), None);
        assert_eq!(tune_step_index(7), None);
        assert_eq!(
            tune_step_index(14_074_000),
            None,
            "a frequency, not a place"
        );
    }

    /// Every index the function yields is inside the radio's 0-5 range, so a
    /// `VT` built from it can never name a rate that does not exist.
    /// trace: FR-VFO-STEP-01
    #[test]
    fn fr_vfo_step_01_index_is_always_in_range() {
        for p in 0..=18u32 {
            if let Some(i) = tune_step_index(10u64.pow(p)) {
                assert!(i <= 5, "10^{p} yielded index {i}");
            }
        }
    }
}

#[cfg(test)]
mod alternate_mode_tests {
    use super::*;

    /// Each paired mode maps to its reverse / opposite sideband.
    /// trace: FR-UI-ALT-01
    #[test]
    fn fr_ui_alt_01_pairs_are_the_radios_own() {
        assert_eq!(alternate_of("LSB"), Some(2), "LSB -> USB");
        assert_eq!(alternate_of("USB"), Some(1), "USB -> LSB");
        assert_eq!(alternate_of("CW"), Some(7), "CW -> CW-R");
        assert_eq!(alternate_of("CW-R"), Some(3), "CW-R -> CW");
        assert_eq!(alternate_of("DATA"), Some(9), "DATA -> DATA-R");
        assert_eq!(alternate_of("DATA-R"), Some(6), "DATA-R -> DATA");
    }

    /// The pairing is symmetric: tapping twice returns where you started, so
    /// the gesture is always reversible by repeating it.
    /// trace: FR-UI-ALT-01
    #[test]
    fn fr_ui_alt_01_pairing_is_reversible() {
        let digit_of = |m: &str| match m {
            "LSB" => 1,
            "USB" => 2,
            "CW" => 3,
            "CW-R" => 7,
            "DATA" => 6,
            "DATA-R" => 9,
            _ => 0,
        };
        for (a, b) in [("LSB", "USB"), ("CW", "CW-R"), ("DATA", "DATA-R")] {
            assert_eq!(alternate_of(a), Some(digit_of(b)));
            assert_eq!(alternate_of(b), Some(digit_of(a)));
        }
    }

    /// AM and FM have no partner, so re-tapping them must do nothing rather
    /// than jump to some unrelated mode.
    /// trace: FR-UI-ALT-01
    #[test]
    fn fr_ui_alt_01_modes_without_a_partner_have_none() {
        assert_eq!(alternate_of("AM"), None);
        assert_eq!(alternate_of("FM"), None);
        assert_eq!(alternate_of(""), None);
        assert_eq!(alternate_of("NOT A MODE"), None);
    }

    /// An alternate is never the mode you are already in — that was the whole
    /// complaint about the first attempt, where re-tapping could land back on
    /// the same button or somewhere unrelated.
    /// trace: FR-UI-ALT-01
    #[test]
    fn fr_ui_alt_01_alternate_is_never_the_same_mode() {
        for (m, digit) in [
            ("LSB", 1u8),
            ("USB", 2),
            ("CW", 3),
            ("CW-R", 7),
            ("DATA", 6),
            ("DATA-R", 9),
        ] {
            assert_ne!(alternate_of(m), Some(digit), "{m} must not map to itself");
        }
    }
}

#[cfg(test)]
mod estop_hotkey_tests {
    use super::*;
    use iced::keyboard::{key::Named, Key, Modifiers};

    fn esc() -> Key {
        Key::Named(Named::Escape)
    }
    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }

    /// On air, ESC stops — whatever modifiers happen to be down.
    /// trace: FR-TX-SAFE-05
    #[test]
    fn fr_tx_safe_05_esc_stops_while_on_air() {
        assert!(is_estop_press(&esc(), Modifiers::empty(), true));
        assert!(
            is_estop_press(&esc(), Modifiers::SHIFT, true),
            "a stray modifier must not swallow the stop"
        );
    }

    /// Off air, ESC is *not* a stop — it stays the dismiss key, so closing a
    /// popup cannot be mistaken for an emergency action.
    /// trace: FR-TX-SAFE-05, FR-UI-POPUP-01
    #[test]
    fn fr_tx_safe_05_esc_off_air_is_not_a_stop() {
        assert!(!is_estop_press(&esc(), Modifiers::empty(), false));
    }

    /// The backstop stops regardless of what the app believes about on-air
    /// state — the case a stale snapshot would otherwise strand.
    /// trace: FR-TX-SAFE-05
    #[test]
    fn fr_tx_safe_05_backstop_ignores_on_air_state() {
        for on_air in [true, false] {
            assert!(
                is_estop_press(&ch("x"), Modifiers::CTRL | Modifiers::SHIFT, on_air),
                "Ctrl+Shift+X must stop with on_air={on_air}"
            );
            assert!(
                is_estop_press(&ch("X"), Modifiers::CTRL | Modifiers::SHIFT, on_air),
                "shifted X reports upper-case on some layouts"
            );
            assert!(
                is_estop_press(&ch("x"), Modifiers::COMMAND | Modifiers::SHIFT, on_air),
                "Cmd+Shift+X on macOS"
            );
        }
    }

    /// The backstop needs its full chord — partial presses are ordinary
    /// editing keys (Ctrl+X is cut) and must not stop the transmitter.
    /// trace: FR-TX-SAFE-05
    #[test]
    fn fr_tx_safe_05_partial_chords_do_not_stop() {
        for on_air in [true, false] {
            assert!(
                !is_estop_press(&ch("x"), Modifiers::CTRL, on_air),
                "Ctrl+X is cut"
            );
            assert!(!is_estop_press(&ch("x"), Modifiers::SHIFT, on_air));
            assert!(!is_estop_press(&ch("x"), Modifiers::empty(), on_air));
        }
    }

    /// Ordinary editing chords are left alone — in particular `Ctrl+C`, which
    /// an earlier revision of this requirement had taken over.
    /// trace: FR-TX-SAFE-05
    #[test]
    fn fr_tx_safe_05_editing_chords_are_untouched() {
        for c in ["c", "v", "a", "z", "s"] {
            for on_air in [true, false] {
                assert!(
                    !is_estop_press(&ch(c), Modifiers::CTRL, on_air),
                    "Ctrl+{c} must remain an editing shortcut"
                );
                assert!(
                    !is_estop_press(&ch(c), Modifiers::CTRL | Modifiers::SHIFT, on_air),
                    "Ctrl+Shift+{c} must not stop"
                );
            }
        }
    }

    /// Typing plain letters never stops the transmitter, even mid-transmission
    /// (a CW `KY` message is typed while on air).
    /// trace: FR-TX-SAFE-05
    #[test]
    fn fr_tx_safe_05_typing_never_stops() {
        for c in ["a", "x", "c", "e", "q"] {
            assert!(!is_estop_press(&ch(c), Modifiers::empty(), true));
            assert!(!is_estop_press(&ch(c), Modifiers::SHIFT, true));
        }
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
}

#[cfg(test)]
mod rx_popup_tests {
    use super::*;

    /// The popup's slider can only ever land on a level the radio has
    /// (D14 p.1318). A slider is free-running, so this is the guard that
    /// stops it sending, say, 14 dB.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_atten_snap_stays_on_the_radios_ladder() {
        for raw in 0u8..=255 {
            let v = atten_snap(raw);
            assert!(v <= ATTEN_MAX_DB, "raw={raw} → {v} exceeds the maximum");
            assert_eq!(v % ATTEN_STEP_DB, 0, "raw={raw} → {v} is off the grid");
        }
    }

    /// Snapping goes to the *nearest* step, so the slider tracks the pointer
    /// instead of trailing it by up to a whole step.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_atten_snap_rounds_to_nearest() {
        assert_eq!(atten_snap(0), 0);
        assert_eq!(atten_snap(1), 0, "below the midpoint rounds down");
        assert_eq!(atten_snap(2), 3, "at/above the midpoint rounds up");
        assert_eq!(atten_snap(3), 3, "an on-grid value is unchanged");
        assert_eq!(atten_snap(20), 21);
        assert_eq!(atten_snap(21), 21, "the top of the ladder is reachable");
    }

    /// Every documented level is reachable through the slider — a snap that
    /// skipped one would make it unsettable from the popup.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_atten_snap_reaches_every_level() {
        let reachable: std::collections::BTreeSet<u8> =
            (0u8..=ATTEN_MAX_DB).map(atten_snap).collect();
        let ladder: std::collections::BTreeSet<u8> = (0..=ATTEN_MAX_DB / ATTEN_STEP_DB)
            .map(|n| n * ATTEN_STEP_DB)
            .collect();
        assert_eq!(reachable, ladder, "every 3 dB step must be selectable");
    }

    /// The popup can set every level the radio's ladder defines, and nothing
    /// between them.
    ///
    /// This used to compare against `atten_hold`, the blind 3 dB stepper the
    /// chip's hold ran before the hold started opening this popup instead.
    /// The ladder is the same; it is now asserted directly rather than
    /// through a function that no longer exists.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_atten_snap_covers_the_whole_ladder() {
        let mut level = 0u8;
        while level <= ATTEN_MAX_DB {
            assert_eq!(
                atten_snap(level),
                level,
                "{level} dB is on the radio's ladder but the popup cannot represent it"
            );
            level += ATTEN_STEP_DB;
        }
    }

    /// APF bandwidths are the radio's three (D12 `AP`), and an unknown
    /// read-back shows as unknown rather than defaulting to a real width.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_apf_width_labels() {
        assert_eq!(apf_width_label(Some(0)), "30");
        assert_eq!(apf_width_label(Some(1)), "50");
        assert_eq!(apf_width_label(Some(2)), "150");
        // The reservation list must cover every label, or the control resizes
        // as the width changes (FR-UI-STABLE-01).
        for w in [Some(0), Some(1), Some(2), Some(9), None] {
            let label = apf_width_label(w);
            assert!(
                APF_WIDTH_LABELS.contains(&label),
                "{w:?} renders {label:?}, missing from APF_WIDTH_LABELS"
            );
        }
        assert_eq!(apf_width_label(None), UNKNOWN);
        assert_eq!(apf_width_label(Some(9)), UNKNOWN, "out of range is unknown");
    }

    /// The popup opens *at the pointer* — the whole point of anchoring it
    /// rather than centring it on the window.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_origin_follows_the_pointer() {
        let win = (1320.0, 1000.0);
        let size = (400.0, 90.0);
        let (x, y) = popup_origin((300.0, 250.0), size, win);
        assert!(
            (300.0..=320.0).contains(&x) && (250.0..=270.0).contains(&y),
            "expected the card next to the pointer, got ({x}, {y})"
        );
        // A pointer further right puts the card further right.
        let (x2, _) = popup_origin((600.0, 250.0), size, win);
        assert!(x2 > x, "the card must track the pointer");
    }

    /// Opening near an edge pulls the card back inside instead of letting it
    /// hang off where its controls could not be reached.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_origin_stays_inside_the_window() {
        let win = (1320.0, 1000.0);
        for p in [
            RxPopup::Atten,
            RxPopup::Preamp,
            RxPopup::Agc,
            RxPopup::Nb,
            RxPopup::Nr,
            RxPopup::Notch,
            RxPopup::Apf,
        ] {
            let (w, h) = p.size();
            // Every corner, including well outside the window.
            for cursor in [
                (0.0, 0.0),
                (1319.0, 999.0),
                (1319.0, 0.0),
                (0.0, 999.0),
                (5000.0, 5000.0),
            ] {
                let (x, y) = popup_origin(cursor, (w, h), win);
                assert!(x >= 0.0 && y >= 0.0, "{p:?} at {cursor:?} → ({x}, {y})");
                assert!(
                    x + w <= win.0 && y + h <= win.1,
                    "{p:?} at {cursor:?} → ({x}, {y}) runs off a {win:?} window"
                );
            }
        }
    }

    /// A window too small for the card keeps the card's **top-left** on
    /// screen — the title and first control — rather than splitting the
    /// overflow across both edges and losing both.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_origin_degrades_gracefully_when_too_small() {
        let (x, y) = popup_origin((100.0, 100.0), (400.0, 90.0), (200.0, 60.0));
        assert!(x >= 0.0 && y >= 0.0, "({x}, {y}) is off the top-left");
        assert!(x <= POPUP_GAP && y <= POPUP_GAP, "expected a pinned corner");
    }

    /// Each popup names its own panel, and no two share a title — the title
    /// is the only thing telling the operator which chip they opened.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_popup_01_titles_are_present_and_distinct() {
        let all = [
            RxPopup::Atten,
            RxPopup::Preamp,
            RxPopup::Agc,
            RxPopup::Nb,
            RxPopup::Nr,
            RxPopup::Notch,
            RxPopup::Apf,
        ];
        let titles: std::collections::BTreeSet<&str> = all.iter().map(|p| p.title()).collect();
        assert_eq!(titles.len(), all.len(), "two popups share a title");
        assert!(
            all.iter().all(|p| !p.title().is_empty()),
            "a popup has no title"
        );
    }
}

#[cfg(test)]
mod nb_filter_tests {
    use super::*;

    /// Labels track the modes, and an unknown mode is not shown as NONE —
    /// "we have not heard from the radio" and "the filter is off" are
    /// different things. Used by the chip and by the NB popup's mode buttons.
    /// trace: FR-UI-POPUP-01
    #[test]
    fn fr_ui_hold_01_nb_filter_labels() {
        assert_eq!(nb_filter_label(Some(0)), "NONE");
        assert_eq!(nb_filter_label(Some(1)), "NAR");
        assert_eq!(nb_filter_label(Some(2)), "WIDE");
        assert_eq!(nb_filter_label(None), UNKNOWN);
        assert_ne!(nb_filter_label(None), nb_filter_label(Some(0)));
    }
}

#[cfg(test)]
mod on_air_tests {
    use super::on_air;

    /// Any route to air counts — including a tune, which deliberately does
    /// not set the transmit flag.
    /// trace: FR-UI-TX-01
    #[test]
    fn fr_ui_tx_01_any_route_to_air_counts() {
        assert!(!on_air(false, false, Some(false)), "idle");
        assert!(on_air(true, false, Some(false)), "PTT / voice");
        assert!(
            on_air(false, true, Some(false)),
            "a tune emits a carrier without setting the transmit flag — the \
             indicator must still light, or the operator sees a dark TX box \
             while RF is going out"
        );
        assert!(on_air(true, true, Some(true)));
    }

    /// Transmission this app did **not** initiate still counts, on the
    /// radio's own report alone.
    ///
    /// The case that found this: the switch-row `TUNE` sends a raw switch tap
    /// rather than going through `Session::tune`, so both local flags stay
    /// false while the radio is keyed. Judged on local intent alone, the
    /// emergency stop was inert — pressing ESC during such a tune did
    /// nothing, on a live radio. Front-panel PTT, VOX and the K-Pod are the
    /// same shape.
    /// trace: FR-UI-TX-01, FR-TX-SAFE-05
    #[test]
    fn fr_tx_safe_05_radio_reported_transmit_counts_on_its_own() {
        assert!(
            on_air(false, false, Some(true)),
            "the radio says it is transmitting; nothing local does — this must \
             still be on air, or the emergency stop cannot reach it"
        );
    }

    /// Before the radio has reported, the local flags still decide — an
    /// unknown state must not be read as "on air" (which would make ESC stop
    /// dismissing dialogs on a disconnected app) nor suppress a local route.
    /// trace: FR-UI-TX-01
    #[test]
    fn fr_ui_tx_01_unknown_radio_state_falls_back_to_local() {
        assert!(
            !on_air(false, false, None),
            "idle and unknown is not on air"
        );
        assert!(on_air(true, false, None), "a local transmit still counts");
        assert!(on_air(false, true, None), "a local tune still counts");
    }
}

#[cfg(test)]
mod stable_label_set_tests {
    use super::{conn_status, connect_button, ConnPhase, CONNECT_LABELS, CONN_STATUS_LABELS};

    /// The reserved-width label sets must list *every* label their function
    /// can return. A width is reserved from these lists, so a phase added
    /// without extending them silently reintroduces the resizing the
    /// requirement exists to prevent — and it would show up as a twitching
    /// header, which is exactly the symptom nobody files a bug about.
    /// trace: FR-UI-STABLE-01
    #[test]
    fn fr_ui_stable_01_label_sets_cover_every_phase() {
        // Listed explicitly rather than iterated: adding a variant should
        // break this line and make the author look at the lists.
        let phases = [
            ConnPhase::Disconnected,
            ConnPhase::Connecting,
            ConnPhase::Connected,
        ];
        for p in phases {
            let (label, _) = connect_button(p);
            assert!(
                CONNECT_LABELS.contains(&label),
                "{p:?} renders {label:?}, missing from CONNECT_LABELS"
            );
            let (status, _) = conn_status(p);
            assert!(
                CONN_STATUS_LABELS.contains(&status),
                "{p:?} renders {status:?}, missing from CONN_STATUS_LABELS"
            );
        }
        assert_eq!(CONNECT_LABELS.len(), phases.len(), "no unused reservations");
        assert_eq!(CONN_STATUS_LABELS.len(), phases.len());
    }
}

#[cfg(test)]
mod char_width_tests {
    use super::stable_label_width;

    /// Capitals must keep the width they had, and digits must reserve less.
    ///
    /// The regression this guards: a single 0.78 em factor was calibrated for
    /// capitals (`DISARM` wrapped at 0.62), then applied to numeric readouts,
    /// which are far narrower. The gain row's six readouts over-reserved by
    /// ~108 px between them, overflowed the row, and squeezed the trailing
    /// `SHIFT` readout to one character per line.
    /// trace: FR-UI-STABLE-01
    #[test]
    fn fr_ui_stable_01_digits_reserve_less_than_capitals() {
        // Same character count, very different real width.
        let caps = stable_label_width(&["ABCDEFG"], 11.0, 0.0);
        let digits = stable_label_width(&["1234567"], 11.0, 0.0);
        assert!(
            digits < caps,
            "digits ({digits:.1}) must reserve less than capitals ({caps:.1})"
        );

        // The all-capitals case that set the 0.78 factor is unchanged, so this
        // cannot reintroduce the wrapped DISARM.
        assert_eq!(
            stable_label_width(&["TX ARMED — DISARM"], 13.0, 20.0),
            "TX ARMED — DISARM".chars().map(super::char_em).sum::<f32>() * 13.0 + 20.0
        );

        // A readout must still fit: reserve for "5000 Hz" and check the whole
        // range renders no wider.
        let widest = stable_label_width(&["5000 Hz"], 11.0, 4.0);
        for hz in [0u32, 150, 1000, 4999, 5000] {
            let s = format!("{hz} Hz");
            assert!(
                stable_label_width(&[s.as_str()], 11.0, 4.0) <= widest,
                "{s} must fit the reservation for 5000 Hz"
            );
        }
    }
}

#[cfg(test)]
mod opt_override_tests {
    use super::{Opt, OptLevel};

    /// A read-back from *earlier in the same drag* must not overwrite the value
    /// the operator is currently setting.
    ///
    /// The reported symptom: "the sliders are laggy and the values jump back
    /// and forth". The resync overwrote each local value unconditionally, so
    /// mid-drag it wrote back an earlier step of that same drag and the slider
    /// snapped backwards under the finger.
    /// trace: FR-UI-STABLE-01, FR-RX-01
    #[test]
    fn fr_rx_01_a_stale_read_back_does_not_overwrite_a_live_drag() {
        let mut o = OptLevel::default();
        o.set(40); // operator has dragged to 40
        assert!(o.is_pending(), "the resync must stand off while pending");

        // The radio is still reporting 10 from earlier in the drag.
        o.reconcile(Some(10));
        assert!(o.is_pending(), "a stale value must not clear the override");
        assert_eq!(o.or(Some(10)), Some(40), "the operator's value is shown");

        // Once the radio catches up the override retires and the radio leads.
        o.reconcile(Some(40));
        assert!(!o.is_pending());
        assert_eq!(o.or(Some(40)), Some(40));
    }

    /// An override that is never confirmed expires, so a dropped command
    /// cannot leave the UI permanently asserting a value the radio never took.
    /// trace: FR-RX-01
    #[test]
    fn fr_rx_01_an_unconfirmed_override_expires() {
        let mut o = OptLevel::default();
        o.set(30);
        for _ in 0..=OptLevel::STALE_TICKS {
            o.reconcile(Some(5)); // radio never agrees
        }
        assert!(!o.is_pending(), "the override must not be held forever");
        assert_eq!(o.or(Some(5)), Some(5), "the radio wins once it expires");
    }

    /// The override is generic now, because notch pitch and passband shift are
    /// 16-bit and were fighting the read-back in exactly the same way.
    /// trace: FR-RX-01
    #[test]
    fn fr_rx_01_override_works_for_wider_values() {
        let mut o: Opt<u16> = Opt::default();
        o.set(2_400);
        o.reconcile(Some(1_000));
        assert_eq!(o.or(Some(1_000)), Some(2_400));
        o.reconcile(Some(2_400));
        assert_eq!(o.or(Some(2_400)), Some(2_400));
        assert!(!o.is_pending());
    }
}
