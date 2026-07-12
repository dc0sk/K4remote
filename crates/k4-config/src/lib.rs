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
            mic_gain_pct: 100,
            theme: None,
            mute_radio_mon: true,
            diagnostics_window: false,
            ptt_hotkey: default_ptt_hotkey(),
            ptt_toggle: true,
            mode_aware_ui: true,
            kpod_enabled: false,
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
