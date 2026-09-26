//! Application configuration: connection profiles + preferences (FR-CFG-01/02).
//!
//! **Secrets are never persisted** (FR-CFG-03): [`Profile`] has no password field
//! by construction, so a serialized config cannot leak one. The password is
//! entered at connect time (a future enhancement may store it in the OS
//! keychain). [`redact`] masks secrets that must not appear in logs/status
//! (NFR-SEC-01).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub mod backup;
pub mod crypto;
pub mod peer;
pub mod secret;
pub use crypto::{CryptoError, MasterKey, Sealed};
pub use peer::{Peer, PeerCache, PeerSecret};
#[cfg(feature = "keychain")]
pub use secret::KeyringStore;
pub use secret::{MemoryStore, SecretError, SecretStore};

/// A saved connection profile — host, port, transport. **No password.**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Server host or IP.
    pub host: String,
    /// Server port (9205 plaintext / 9204 TLS-PSK).
    pub port: u16,
    /// Use TLS-PSK.
    #[serde(default)]
    pub use_tls: bool,
    /// Remember the password in the OS keychain (FR-CFG-03). Still never written
    /// to this config file.
    #[serde(default)]
    pub remember: bool,
}

/// One entry in the client-side frequency-memory bank (`FR-MEM-01`).
///
/// Client-side because the radio's own memory-channel command (`MC`) is
/// **[Pending] TBD** in the K4 Programmer's Reference rev. D12 — there is no
/// documented way to reach the K4's memories over CAT, so the app keeps its
/// own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    /// Operator's label. May be empty; the UI then shows the frequency.
    #[serde(default)]
    pub name: String,
    /// Frequency in Hz.
    pub hz: u64,
    /// Mode as the radio names it (`"CW"`, `"USB"`, …), if one was captured.
    #[serde(default)]
    pub mode: Option<String>,
}

/// Operating preferences (FR-CFG-02/05). Audio levels are stored as integer
/// percents (unity = 100) so the struct stays `Eq` and the TOML stays clean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    /// Tuning step, Hz.
    pub tune_step_hz: u32,
    /// Client-side frequency memories (FR-MEM-01).
    #[serde(default)]
    pub memories: Vec<Memory>,
    /// Selected RX playback device name (`None` = system default).
    #[serde(default)]
    pub audio_output: Option<String>,
    /// Selected TX capture device name (`None` = system default).
    #[serde(default)]
    pub audio_input: Option<String>,
    /// RX playback volume, percent (0–200; 100 = unity).
    #[serde(default = "default_pct")]
    pub volume_pct: u16,
    /// Volume-control **positions**, 0–100 (FR-AUD-LVL-01 / FR-RX-VOL-01).
    /// Absent in configs written before the controls became 0–100 % with a
    /// perceptual curve; the app then migrates from the `*_pct` fields below,
    /// which stored a raw gain multiplier as a percentage.
    #[serde(default)]
    pub volume_level: Option<u8>,
    #[serde(default)]
    pub rx_volume_main_level: Option<u8>,
    #[serde(default)]
    pub rx_volume_sub_level: Option<u8>,
    /// Legacy raw-multiplier percentages, kept only so an existing config can
    /// be migrated. Not written by current versions.
    #[serde(default = "pct_100")]
    pub rx_volume_main_pct: u16,
    #[serde(default = "pct_100")]
    pub rx_volume_sub_pct: u16,
    /// TX mic capture gain, percent (0–300; 100 = unity).
    #[serde(default = "default_pct")]
    pub mic_gain_pct: u16,
    /// Selected UI theme name (`dark`/`light`/`contrast`/`system`).
    #[serde(default)]
    pub theme: Option<String>,
    /// Mute the radio's TX monitor (`ML=0`) on connect, so a remote session
    /// doesn't blare the shack speaker. Default on.
    #[serde(default = "default_true")]
    pub mute_radio_mon: bool,
    /// Show the diagnostics console in a separate window. Default off.
    #[serde(default)]
    pub diagnostics_window: bool,
    /// Show explanatory tooltips on controls after a short hover. Default on —
    /// the panel mirrors a radio with a hundred controls, and the tips name the
    /// CAT command behind each one (FR-UI-TIP-01).
    #[serde(default = "default_true")]
    pub tooltips: bool,
    /// PTT push-to-talk keyboard hotkey (e.g. `Ctrl+Space`).
    #[serde(default = "default_ptt_hotkey")]
    pub ptt_hotkey: String,
    /// PTT hotkey mode: `true` = toggle (press on/off), `false` = hold-to-talk.
    /// Default toggle.
    #[serde(default = "default_true")]
    pub ptt_toggle: bool,
    /// Use the mode-adaptive UI (per-mode control emphasis). Default on.
    #[serde(default = "default_true")]
    pub mode_aware_ui: bool,
    /// Automatically check for a newer release at start-up (FR-UI-UPD-02).
    /// Default on; opt-out here. One check per launch, silent unless an update
    /// is found.
    #[serde(default = "default_true")]
    pub auto_update_check: bool,
    /// Enable KPA1500 linear-amplifier support (FR-AMP-01). Default off
    /// (opt-in). When on, the app talks to the amp over its **own** Ethernet
    /// CAT server — a second connection alongside the K4 link, not the K4's
    /// one-way `EC` passthrough. Host/port/poll are set in the separate
    /// KPA1500 configuration window.
    #[serde(default)]
    pub kpa1500_enabled: bool,
    /// KPA1500 remote-head host (IP or hostname). Empty until configured.
    #[serde(default)]
    pub kpa1500_host: String,
    /// KPA1500 remote-head TCP command-server port. Default 1500.
    #[serde(default = "default_kpa1500_port")]
    pub kpa1500_port: u16,
    /// KPA1500 telemetry poll interval, milliseconds. Default 500.
    #[serde(default = "default_kpa1500_poll_ms")]
    pub kpa1500_poll_ms: u16,
    /// Longest a spot may be before its nameplate is hidden, minutes
    /// (FR-SPOT-03). Default 15; read through [`Prefs::spot_max_age_min`], which
    /// clamps a hand-edited value back into range.
    #[serde(default = "default_spot_max_age_min")]
    pub spot_max_age_min: u32,
    /// Spectrum afterglow, milliseconds (FR-PAN-14): how long a peak lingers on the trace. `0` is
    /// off; the default is [`AFTERGLOW_DEFAULT_MS`]. Read through
    /// [`Prefs::spectrum_afterglow_ms`], which brings a hand-edited value back into range.
    #[serde(default = "default_afterglow_ms")]
    pub spectrum_afterglow_ms: u32,
    /// Which spotting networks feed the spectrum nameplates, and each one's
    /// settings (FR-SPOT-04). Every network defaults to off.
    #[serde(default)]
    pub spot_networks: SpotNetworks,
    /// Enable the Elecraft K-Pod USB control surface. Default off (opt-in); the
    /// app runs normally whether or not a K-Pod is attached.
    #[serde(default)]
    pub kpod_enabled: bool,
    /// K-Pod function-switch assignments: 16 slots, F1–F8 each with a tap and a
    /// hold action (index = `(button-1)*2 + hold`; see `k4_kpod::slot_index`).
    /// Each slot's `cat` is sent to the K4 on that switch press. Seeded from the
    /// built-in Elecraft sample macros (FR-KPOD-06).
    #[serde(default = "default_kpod_buttons")]
    pub kpod_buttons: Vec<KpodButton>,
    /// Stored DTMF sequences (FR-FM-03), app-side — the K4's own CMD1–6 are not reachable over
    /// CAT. Read through [`Prefs::dtmf_sequences`], which always gives six cleaned slots.
    #[serde(default)]
    pub dtmf_sequences: Vec<DtmfSequence>,
}

/// One stored DTMF sequence (FR-FM-03): a short name and its digits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DtmfSequence {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub digits: String,
}

/// How many DTMF sequences are stored, like the K4's own CMD1–6 (FR-FM-03).
pub const DTMF_SEQ_COUNT: usize = 6;
/// Longest sequence, digits; mirrors `k4_protocol::cat::DTMF_SEQ_MAX` (the app tests they agree).
pub const DTMF_SEQ_DIGITS_MAX: usize = 32;
/// Longest sequence name, characters.
pub const DTMF_SEQ_NAME_MAX: usize = 16;

/// Keep only DTMF characters (`0`–`9`, `A`–`D`, `*`, `#`), upper-cased, at most 32.
pub fn sanitise_dtmf_digits(input: &str) -> String {
    input
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .filter(|c| c.is_ascii_digit() || matches!(c, 'A'..='D' | '*' | '#'))
        .take(DTMF_SEQ_DIGITS_MAX)
        .collect()
}

/// Keep only printable characters of a sequence name, at most 16.
pub fn sanitise_dtmf_name(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_control())
        .take(DTMF_SEQ_NAME_MAX)
        .collect()
}

/// One K-Pod function-switch assignment (FR-KPOD-06): a short display `label`
/// (K4 convention ≤ 7 chars) and the `cat` macro — a semicolon-separated K4 CAT
/// command string sent to the radio when the switch is pressed. An empty `cat`
/// means the slot is unassigned (no-op).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KpodButton {
    /// Short display label (≤ 7 chars by K4 convention).
    #[serde(default)]
    pub label: String,
    /// CAT macro string sent on press (empty = unassigned).
    #[serde(default)]
    pub cat: String,
}

impl KpodButton {
    /// An empty (unassigned) slot.
    pub fn empty() -> Self {
        Self {
            label: String::new(),
            cat: String::new(),
        }
    }
}

/// Number of assignable K-Pod slots: F1–F8 × {tap, hold}. The slot **order**
/// (index `(button-1)*2 + hold` — F1 tap, F1 hold, F2 tap, …) is defined by
/// `k4_kpod::slot_index`, which the worker uses to look up the pressed switch;
/// [`Prefs::kpod_buttons`] is stored in that same order.
pub const KPOD_SLOT_COUNT: usize = 16;

/// Human name for slot `index` (0–15), e.g. `"F1 tap"`, `"F8 hold"`.
pub fn kpod_slot_name(index: usize) -> String {
    let f = index / 2 + 1;
    let action = if index.is_multiple_of(2) {
        "tap"
    } else {
        "hold"
    };
    format!("F{f} {action}")
}

/// A selectable K-Pod macro preset for the config editor's pick-list: a short
/// `label`, the `cat` command string, and a one-line `desc`. Mix of confident
/// K4-native quick actions and the Elecraft Owner's-Manual sample macros
/// (K3-compatible `SWT`/`SWH` codes the K4 accepts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KpodPreset {
    pub label: &'static str,
    pub cat: &'static str,
    pub desc: &'static str,
}

impl std::fmt::Display for KpodPreset {
    /// Rendered in the editor's pick-list as `label — desc`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} — {}", self.label, self.desc)
    }
}

/// Built-in K-Pod macro presets shown in the config editor's pick-list
/// (FR-KPOD-06). K4-native quick actions first, then the Elecraft sample macros.
pub const KPOD_PRESETS: &[KpodPreset] = &[
    // --- K4-native quick actions (confident CAT) ---
    KpodPreset {
        label: "RIT Clr",
        cat: "RC;",
        desc: "Clear RIT/XIT offset",
    },
    KpodPreset {
        label: "Split+",
        cat: "FT1;",
        desc: "Split on (TX on VFO B)",
    },
    KpodPreset {
        label: "Split-",
        cat: "FT0;",
        desc: "Split off (TX on VFO A)",
    },
    KpodPreset {
        label: "RIT On",
        cat: "RT1;",
        desc: "RIT on",
    },
    KpodPreset {
        label: "RIT Off",
        cat: "RT0;",
        desc: "RIT off",
    },
    KpodPreset {
        label: "XIT On",
        cat: "XT1;",
        desc: "XIT on",
    },
    KpodPreset {
        label: "XIT Off",
        cat: "XT0;",
        desc: "XIT off",
    },
    KpodPreset {
        label: "CW",
        cat: "MD3;",
        desc: "Mode CW",
    },
    KpodPreset {
        label: "LSB",
        cat: "MD1;",
        desc: "Mode LSB",
    },
    KpodPreset {
        label: "USB",
        cat: "MD2;",
        desc: "Mode USB",
    },
    KpodPreset {
        label: "DATA",
        cat: "MD6;",
        desc: "Mode DATA A",
    },
    // --- Elecraft Owner's-Manual sample macros ---
    KpodPreset {
        label: "B>A",
        cat: "SWT11;SWT13;SWT11;",
        desc: "Copy VFO B to A",
    },
    KpodPreset {
        label: "SPLIT+2",
        cat: "SWT13;SWT13;FT1;UPB5;RT0;XT0;LK1;",
        desc: "Split, TX +2 kHz",
    },
    KpodPreset {
        label: "RUN",
        cat: "SWT13;SWT13;FT0;RT1;XT0;RC;SWH58;",
        desc: "Run: simplex, RIT on, clear",
    },
    KpodPreset {
        label: "CW UP2",
        cat: "MD3;SWT13;SWT13;FT1;UPB5;RT0;XT0;LK1;SWT58;",
        desc: "CW split TX +2 kHz",
    },
    KpodPreset {
        label: "CW UP5",
        cat: "MD3;SWT13;SWT13;FT1;UPB7;RT0;XT0;LK1;SWT58;",
        desc: "CW split TX +5 kHz",
    },
    KpodPreset {
        label: "SSB UP5",
        cat: "SWT13;SWT13;FT1;UPB7;RT0;XT0;LK1;SWT58;BW0210;",
        desc: "SSB split TX +5 kHz",
    },
    KpodPreset {
        label: "RX UP2",
        cat: "SWT13;SWT13;FT1;UP5;RT0;XT0;LK$1;",
        desc: "Sub-RX split +2 kHz",
    },
    KpodPreset {
        label: "RX UP5",
        cat: "SWT13;SWT13;FT1;UP7;RT0;XT0;LK$1;",
        desc: "Sub-RX split +5 kHz",
    },
    KpodPreset {
        label: "RTTY",
        cat: "MD6;DT1;SWH29;",
        desc: "DATA A, RTTY, dual passband",
    },
    KpodPreset {
        label: "PSK",
        cat: "MD6;DT3;IS 0600;BW0025;SWT49;RT1;",
        desc: "DATA A, PSK D, 250 Hz",
    },
    KpodPreset {
        label: "Cleanup",
        cat: "FT0;RT0;XT0;LN0;SQ000;SWT13;SWT13;SWH58;NB0;NB$0;SB0;",
        desc: "Reset TX/RX state",
    },
    KpodPreset {
        label: "Divrsty",
        cat: "FT0;LK0;LK$0;SB1;DV1;RC;",
        desc: "Diversity RX on",
    },
];

/// The default 16-slot K-Pod assignment table, seeded from the Elecraft sample
/// macros (FR-KPOD-06). The first slots get the manual's samples in order; the
/// rest start empty (editable in the config menu).
pub fn default_kpod_buttons() -> Vec<KpodButton> {
    // Seed from the Elecraft sample macros (skip the K4-native quick-actions,
    // which start at the head of KPOD_PRESETS) so the table matches the manual.
    let samples = KPOD_PRESETS.iter().skip(11);
    let mut slots: Vec<KpodButton> = samples
        .take(KPOD_SLOT_COUNT)
        .map(|p| KpodButton {
            label: p.label.to_string(),
            cat: p.cat.to_string(),
        })
        .collect();
    slots.resize(KPOD_SLOT_COUNT, KpodButton::empty());
    slots
}

fn default_ptt_hotkey() -> String {
    "Ctrl+Space".to_string()
}

fn default_pct() -> u16 {
    100
}

fn default_true() -> bool {
    true
}

/// The KPA1500's TCP command-server port (its remote-head interface).
fn default_kpa1500_port() -> u16 {
    1500
}

/// Default KPA1500 telemetry poll interval, milliseconds.
fn default_kpa1500_poll_ms() -> u16 {
    500
}

/// Default and bounds for the spot age limit, minutes (FR-SPOT-03).
pub const SPOT_MAX_AGE_DEFAULT_MIN: u32 = 15;
pub const SPOT_MAX_AGE_MIN_MIN: u32 = 1;
pub const SPOT_MAX_AGE_MAX_MIN: u32 = 24 * 60;

fn default_spot_max_age_min() -> u32 {
    SPOT_MAX_AGE_DEFAULT_MIN
}

/// Bring a spot age limit (minutes) into `1 min ..= 24 h`; anything outside the
/// range — a hand-edited config, a stray `0` — falls back to the default rather
/// than hiding every spot or keeping them forever.
pub fn sanitise_spot_max_age_min(min: u32) -> u32 {
    if (SPOT_MAX_AGE_MIN_MIN..=SPOT_MAX_AGE_MAX_MIN).contains(&min) {
        min
    } else {
        SPOT_MAX_AGE_DEFAULT_MIN
    }
}

/// Parse the Settings age field: digits only, in range, else the default
/// (FR-SPOT-03) — an empty or unusable entry never becomes a saved value.
pub fn parse_spot_max_age_min(input: &str) -> u32 {
    input
        .trim()
        .parse::<u32>()
        .map(sanitise_spot_max_age_min)
        .unwrap_or(SPOT_MAX_AGE_DEFAULT_MIN)
}

/// Bounds of the spectrum afterglow, milliseconds (FR-PAN-14). `0` is off; anything else is at
/// least [`AFTERGLOW_MIN_MS`] (a shorter trail is not visible at the display's row rate) and at most
/// [`AFTERGLOW_MAX_MS`].
pub const AFTERGLOW_MIN_MS: u32 = 50;
pub const AFTERGLOW_MAX_MS: u32 = 5000;
/// Default afterglow, milliseconds: on by default, at a gentle trail length.
pub const AFTERGLOW_DEFAULT_MS: u32 = 500;

fn default_afterglow_ms() -> u32 {
    AFTERGLOW_DEFAULT_MS
}

/// Bring an afterglow time into range: `0` stays off, a smaller non-zero value is raised to the
/// minimum and a larger one lowered to the maximum. Unlike the spot age limit this **clamps**
/// instead of falling back to the default, because `0` (off) is a deliberate, valid choice rather
/// than an out-of-range one — a typed `6000` should give the longest trail, not silently reset to
/// whatever the current default happens to be.
pub fn sanitise_afterglow_ms(ms: u32) -> u32 {
    if ms == 0 {
        0
    } else {
        ms.clamp(AFTERGLOW_MIN_MS, AFTERGLOW_MAX_MS)
    }
}

/// Parse the Settings afterglow field: digits only, else off (FR-PAN-14) — an empty or unusable
/// entry never becomes a saved trail.
pub fn parse_afterglow_ms(input: &str) -> u32 {
    input
        .trim()
        .parse::<u32>()
        .map(sanitise_afterglow_ms)
        .unwrap_or(0)
}

/// PSK Reporter's public MQTT feed (R-EXT-05): host and plain-TCP port, and the TLS port.
pub const SPOT_PSK_DEFAULT_HOST: &str = "mqtt.pskreporter.info";
pub const SPOT_PSK_DEFAULT_PORT: u16 = 1883;
pub const SPOT_PSK_TLS_PORT: u16 = 1884;

/// The most approved certificates kept. Far more than anyone needs; the cap is what stops a
/// corrupted or hostile file from growing the list without end.
pub const MAX_TRUSTED_CERTS: usize = 32;

/// A server certificate the operator approved by hand (FR-SPOT-13): one host, one port, one
/// certificate. It is public information, not a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedCert {
    pub host: String,
    pub port: u16,
    /// SHA-256 of the certificate, 64 lower-case hex digits.
    pub sha256: String,
}

impl TrustedCert {
    /// Whether the entry is usable: a plain host name, a non-zero port and a well-formed
    /// fingerprint. Anything else is dropped when read, never repaired.
    pub fn is_valid(&self) -> bool {
        let host_ok = !self.host.is_empty()
            && self.host.len() <= 253
            && self
                .host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'_'));
        let fp_ok = self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        host_ok && fp_ok && self.port != 0
    }
}

/// Parse a Settings port field: a non-zero `u16`, else `default` — an empty or
/// unusable entry never becomes a saved port (FR-SPOT-04).
pub fn parse_spot_port(input: &str, default: u16) -> u16 {
    input
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p != 0)
        .unwrap_or(default)
}

/// PSK Reporter as a spot source (FR-SPOT-04, FR-SPOT-05): a live MQTT feed, so there is no poll
/// interval. An older config's `poll_secs` is ignored on load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PskReporterPrefs {
    /// Off until the operator turns it on.
    #[serde(default)]
    pub enabled: bool,
    /// The MQTT broker.
    #[serde(default = "default_psk_host")]
    pub host: String,
    #[serde(default = "default_psk_port")]
    pub port: u16,
    /// Connect encrypted (TLS, normally port 1884) instead of plain (FR-SPOT-13).
    #[serde(default)]
    pub tls: bool,
}

fn default_psk_host() -> String {
    SPOT_PSK_DEFAULT_HOST.to_string()
}

fn default_psk_port() -> u16 {
    SPOT_PSK_DEFAULT_PORT
}

impl Default for PskReporterPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_psk_host(),
            port: SPOT_PSK_DEFAULT_PORT,
            tls: false,
        }
    }
}

/// Default and bounds for how often a polled spot network is asked, seconds (FR-SPOT-08). They
/// mirror `k4_spot::polled` (this crate does not depend on it); the app has a test that they agree.
pub const SPOT_POLL_DEFAULT_SECS: u64 = 60;
pub const SPOT_POLL_MIN_SECS: u64 = 30;
pub const SPOT_POLL_MAX_SECS: u64 = 3600;

/// Bring a poll interval into `30 s ..= 1 h`; anything outside — a hand-edited config, a stray `0`
/// — falls back to the default rather than hammering a network or never asking.
pub fn sanitise_spot_poll_secs(secs: u64) -> u64 {
    if (SPOT_POLL_MIN_SECS..=SPOT_POLL_MAX_SECS).contains(&secs) {
        secs
    } else {
        SPOT_POLL_DEFAULT_SECS
    }
}

/// Parse the Settings interval field: digits only, in range, else the default (FR-SPOT-08) — an
/// empty or unusable entry never becomes a saved value.
pub fn parse_spot_poll_secs(input: &str) -> u64 {
    input
        .trim()
        .parse::<u64>()
        .map(sanitise_spot_poll_secs)
        .unwrap_or(SPOT_POLL_DEFAULT_SECS)
}

fn default_spot_poll_secs() -> u64 {
    SPOT_POLL_DEFAULT_SECS
}

/// POTA as a spot source (FR-SPOT-08): a public list asked for now and then, so the only setting
/// besides the switch is how often.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PotaPrefs {
    /// Off until the operator turns it on.
    #[serde(default)]
    pub enabled: bool,
    /// Seconds between requests, always within `30 s ..= 1 h` once read through
    /// [`PotaPrefs::poll_secs`].
    #[serde(default = "default_spot_poll_secs")]
    pub poll_secs: u64,
}

impl Default for PotaPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            poll_secs: SPOT_POLL_DEFAULT_SECS,
        }
    }
}

impl PotaPrefs {
    /// The interval to use, in bounds whatever the file says.
    pub fn poll_secs(&self) -> u64 {
        sanitise_spot_poll_secs(self.poll_secs)
    }
}

/// FreeDV Reporter (R-EXT-05): the service's host and its plain-WebSocket port.
pub const SPOT_FREEDV_DEFAULT_HOST: &str = "qso.freedv.org";
pub const SPOT_FREEDV_DEFAULT_PORT: u16 = 80;
/// Its port for `wss` (TLS).
pub const SPOT_FREEDV_TLS_PORT: u16 = 443;

fn default_freedv_host() -> String {
    SPOT_FREEDV_DEFAULT_HOST.to_string()
}

fn default_freedv_port() -> u16 {
    SPOT_FREEDV_DEFAULT_PORT
}

/// Default and bounds for how often FreeDV Reporter re-stamps a station still on the air, seconds
/// (FR-SPOT-08; the default was 30 s until DC0SK found it too fast, 2026-09-24). They mirror
/// `k4_spot::freedv_source` (this crate does not depend on it); the app has a test that they agree.
pub const SPOT_FREEDV_REFRESH_DEFAULT_SECS: u64 = 60;
pub const SPOT_FREEDV_REFRESH_MIN_SECS: u64 = 30;
pub const SPOT_FREEDV_REFRESH_MAX_SECS: u64 = 300;

/// Bring a refresh into `30 s ..= 5 min`; anything outside falls back to the default.
pub fn sanitise_freedv_refresh_secs(secs: u64) -> u64 {
    if (SPOT_FREEDV_REFRESH_MIN_SECS..=SPOT_FREEDV_REFRESH_MAX_SECS).contains(&secs) {
        secs
    } else {
        SPOT_FREEDV_REFRESH_DEFAULT_SECS
    }
}

/// Parse the Settings refresh field: digits only, in range, else the default.
pub fn parse_freedv_refresh_secs(input: &str) -> u64 {
    input
        .trim()
        .parse::<u64>()
        .map(sanitise_freedv_refresh_secs)
        .unwrap_or(SPOT_FREEDV_REFRESH_DEFAULT_SECS)
}

fn default_freedv_refresh_secs() -> u64 {
    SPOT_FREEDV_REFRESH_DEFAULT_SECS
}

/// FreeDV Reporter as a spot source (FR-SPOT-08): a live WebSocket feed of the stations on the air,
/// joined read-only. Off until the operator turns it on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeDvPrefs {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_freedv_host")]
    pub host: String,
    #[serde(default = "default_freedv_port")]
    pub port: u16,
    /// Seconds between re-stamps of the stations still on the air, always within `30 s ..= 5 min`
    /// once read through [`FreeDvPrefs::refresh_secs`].
    #[serde(default = "default_freedv_refresh_secs")]
    pub refresh_secs: u64,
    /// Connect over `wss` (TLS, normally port 443) instead of plain `ws` (FR-SPOT-08, FR-SPOT-13).
    #[serde(default)]
    pub tls: bool,
}

impl Default for FreeDvPrefs {
    fn default() -> Self {
        Self {
            enabled: false,
            host: default_freedv_host(),
            port: default_freedv_port(),
            refresh_secs: SPOT_FREEDV_REFRESH_DEFAULT_SECS,
            tls: false,
        }
    }
}

impl FreeDvPrefs {
    /// The refresh to use, in bounds whatever the file says.
    pub fn refresh_secs(&self) -> u64 {
        sanitise_freedv_refresh_secs(self.refresh_secs)
    }
}

/// A telnet spot source — the Reverse Beacon Network or a DX cluster
/// (FR-SPOT-04, connected by FR-SPOT-07).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterPrefs {
    /// Off until the operator turns it on.
    #[serde(default)]
    pub enabled: bool,
    /// Host to connect to. Empty until configured (for a DX cluster).
    #[serde(default)]
    pub host: String,
    /// TCP port.
    #[serde(default)]
    pub port: u16,
    /// Callsign to log in with; empty means the operator's own.
    #[serde(default)]
    pub login: String,
}

impl ClusterPrefs {
    /// The Reverse Beacon Network's telnet feed. The host and port are
    /// prefilled defaults, still to be confirmed against RBN's own documentation
    /// (`OP-7`); the network stays off until enabled.
    pub fn rbn() -> Self {
        Self {
            enabled: false,
            host: "telnet.reversebeacon.net".into(),
            port: 7000,
            login: String::new(),
        }
    }

    /// A DX cluster: no host until the operator picks one.
    pub fn dx_cluster() -> Self {
        Self {
            enabled: false,
            host: String::new(),
            port: 7300,
            login: String::new(),
        }
    }
}

/// The spotting networks and their settings (FR-SPOT-04). All off by default,
/// so a fresh install — or a config from before this feature — contacts no third
/// party.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotNetworks {
    #[serde(default)]
    pub psk_reporter: PskReporterPrefs,
    #[serde(default = "ClusterPrefs::rbn")]
    pub rbn: ClusterPrefs,
    #[serde(default = "ClusterPrefs::dx_cluster")]
    pub dx_cluster: ClusterPrefs,
    #[serde(default)]
    pub pota: PotaPrefs,
    #[serde(default)]
    pub freedv: FreeDvPrefs,
    /// Certificates approved by hand for encrypted connections. Read through
    /// [`SpotNetworks::trusted`], which drops anything malformed.
    #[serde(default)]
    pub trusted_certs: Vec<TrustedCert>,
}

impl Default for SpotNetworks {
    fn default() -> Self {
        Self {
            psk_reporter: PskReporterPrefs::default(),
            rbn: ClusterPrefs::rbn(),
            dx_cluster: ClusterPrefs::dx_cluster(),
            pota: PotaPrefs::default(),
            freedv: FreeDvPrefs::default(),
            trusted_certs: Vec::new(),
        }
    }
}

impl SpotNetworks {
    /// The approved certificates that are usable: malformed entries and duplicates dropped, at most
    /// [`MAX_TRUSTED_CERTS`] kept (the first ones).
    pub fn trusted(&self) -> Vec<TrustedCert> {
        let mut out: Vec<TrustedCert> = Vec::new();
        for c in &self.trusted_certs {
            if c.is_valid() && !out.contains(c) && out.len() < MAX_TRUSTED_CERTS {
                out.push(c.clone());
            }
        }
        out
    }

    /// Approve a certificate. `false` (and nothing changes) if it is malformed, already approved,
    /// or the list is full.
    pub fn trust(&mut self, cert: TrustedCert) -> bool {
        let mut now = self.trusted();
        if !cert.is_valid() || now.contains(&cert) || now.len() >= MAX_TRUSTED_CERTS {
            return false;
        }
        now.push(cert);
        self.trusted_certs = now;
        true
    }

    /// Withdraw the approval at `index` of [`SpotNetworks::trusted`]. `false` if there is none.
    pub fn forget(&mut self, index: usize) -> bool {
        let mut now = self.trusted();
        if index >= now.len() {
            return false;
        }
        now.remove(index);
        self.trusted_certs = now;
        true
    }

    /// Whether any network is switched on.
    pub fn any_enabled(&self) -> bool {
        self.psk_reporter.enabled
            || self.rbn.enabled
            || self.dx_cluster.enabled
            || self.pota.enabled
            || self.freedv.enabled
    }
}

impl Prefs {
    /// The stored DTMF sequences (FR-FM-03): always exactly [`DTMF_SEQ_COUNT`] slots — the first
    /// six in the file, padded with empty ones — each cleaned, whatever the file says.
    pub fn dtmf_sequences(&self) -> Vec<DtmfSequence> {
        let mut out: Vec<DtmfSequence> = self
            .dtmf_sequences
            .iter()
            .take(DTMF_SEQ_COUNT)
            .map(|s| DtmfSequence {
                name: sanitise_dtmf_name(&s.name),
                digits: sanitise_dtmf_digits(&s.digits),
            })
            .collect();
        out.resize(DTMF_SEQ_COUNT, DtmfSequence::default());
        out
    }

    /// The spectrum afterglow in milliseconds, `0` (off) or within
    /// [`AFTERGLOW_MIN_MS`]`..=`[`AFTERGLOW_MAX_MS`] (FR-PAN-14).
    pub fn spectrum_afterglow_ms(&self) -> u32 {
        sanitise_afterglow_ms(self.spectrum_afterglow_ms)
    }

    /// The spot age limit in minutes, always within `1 min ..= 24 h`
    /// (FR-SPOT-03).
    pub fn spot_max_age_min(&self) -> u32 {
        sanitise_spot_max_age_min(self.spot_max_age_min)
    }
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            tune_step_hz: 100,
            memories: Vec::new(),
            audio_output: None,
            audio_input: None,
            volume_level: None,
            rx_volume_main_level: None,
            rx_volume_sub_level: None,
            volume_pct: 100,
            rx_volume_main_pct: 100,
            rx_volume_sub_pct: 100,
            mic_gain_pct: 100,
            theme: None,
            mute_radio_mon: true,
            diagnostics_window: false,
            tooltips: true,
            ptt_hotkey: default_ptt_hotkey(),
            ptt_toggle: true,
            mode_aware_ui: true,
            auto_update_check: true,
            kpa1500_enabled: false,
            kpa1500_host: String::new(),
            kpa1500_port: 1500,
            kpa1500_poll_ms: 500,
            spot_max_age_min: SPOT_MAX_AGE_DEFAULT_MIN,
            spectrum_afterglow_ms: AFTERGLOW_DEFAULT_MS,
            spot_networks: SpotNetworks::default(),
            kpod_enabled: false,
            kpod_buttons: default_kpod_buttons(),
            dtmf_sequences: Vec::new(),
        }
    }
}

/// Persisted application config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// The most recently used connection (prefilled on next launch).
    #[serde(default)]
    pub last: Option<Profile>,
    /// Operating preferences.
    #[serde(default)]
    pub prefs: Prefs,
    /// Cache of successfully-connected peers (FR-CFG-04).
    #[serde(default)]
    pub peers: PeerCache,
    /// Whether the one-time afterglow-default migration (FR-CFG-09) has already been applied
    /// to this config. `false` only for a file saved before this field existed; a config built
    /// fresh in code (nothing to migrate) is `true` by construction — see [`Config::default`].
    /// Bare `#[serde(default)]` cannot express this on its own: it supplies a value for a field
    /// the file never had, not for a value (`spectrum_afterglow_ms = 0`) the file has
    /// explicitly, left over from before that field's default changed.
    #[serde(default)]
    pub afterglow_default_migrated: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            last: None,
            prefs: Prefs::default(),
            peers: PeerCache::default(),
            afterglow_default_migrated: true,
        }
    }
}

impl Config {
    /// Serialize to pretty TOML.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Parse from TOML.
    pub fn from_toml(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    /// One-time migrations for a config loaded from disk (FR-CFG-09): a default that changed
    /// after some configs were already saved with the *old* default explicitly present cannot
    /// be handled by `#[serde(default = "...")]` alone, since that only supplies a value for a
    /// field the file never had — not for a value the file has explicitly, from before the
    /// default changed. Runs at most once per config: an operator who sets the field back to
    /// `0` afterwards is respected, not repeatedly overridden.
    fn migrate(&mut self) {
        if !self.afterglow_default_migrated {
            if self.prefs.spectrum_afterglow_ms == 0 {
                self.prefs.spectrum_afterglow_ms = AFTERGLOW_DEFAULT_MS;
            }
            self.afterglow_default_migrated = true;
        }
    }

    /// Load from `path`, returning the default config on any error (missing file,
    /// parse failure) so startup never fails.
    pub fn load(path: &Path) -> Self {
        let mut cfg = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| Self::from_toml(&text).ok())
            .unwrap_or_default();
        cfg.migrate();
        cfg
    }

    /// Save to `path`, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let toml = self
            .to_toml()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        std::fs::write(path, toml)
    }
}

/// The default config-file path (`$XDG_CONFIG_HOME` / `$HOME/.config` / `%APPDATA%`).
pub fn default_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .or_else(|| std::env::var_os("APPDATA").map(PathBuf::from))?;
    Some(base.join("k4remote").join("config.toml"))
}

/// Replace every occurrence of `secret` in `text` with `***`, so a secret cannot
/// leak into logs or status messages (NFR-SEC-01). An empty secret is a no-op.
pub fn redact(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "***")
    }
}

/// Serde default for the per-receiver volume percentages.
fn pct_100() -> u16 {
    100
}
