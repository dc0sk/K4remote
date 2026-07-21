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

/// Operating preferences (FR-CFG-02/05). Audio levels are stored as integer
/// percents (unity = 100) so the struct stays `Eq` and the TOML stays clean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prefs {
    /// Tuning step, Hz.
    pub tune_step_hz: u32,
    /// Selected RX playback device name (`None` = system default).
    #[serde(default)]
    pub audio_output: Option<String>,
    /// Selected TX capture device name (`None` = system default).
    #[serde(default)]
    pub audio_input: Option<String>,
    /// RX playback volume, percent (0–200; 100 = unity).
    #[serde(default = "default_pct")]
    pub volume_pct: u16,
    /// Per-receiver local playback gain, percent (FR-RX-VOL-01). Independent of
    /// `volume_pct`, which is the master over both.
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

impl Default for Prefs {
    fn default() -> Self {
        Self {
            tune_step_hz: 100,
            audio_output: None,
            audio_input: None,
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
            kpod_enabled: false,
            kpod_buttons: default_kpod_buttons(),
        }
    }
}

/// Persisted application config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Load from `path`, returning the default config on any error (missing file,
    /// parse failure) so startup never fails.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| Self::from_toml(&text).ok())
            .unwrap_or_default()
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
