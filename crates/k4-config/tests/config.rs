//! Config tests. trace: FR-CFG-01, FR-CFG-02, FR-CFG-03, FR-CFG-04, FR-CFG-05,
//! NFR-SEC-01
use k4_config::{redact, Config, MemoryStore, Peer, PeerSecret, Prefs, Profile, SecretStore};

/// The `SecretStore` abstraction holds the password out of the config file
/// (FR-CFG-03): set/get/delete round-trip via the in-memory backend.
///
/// trace: FR-CFG-03
#[test]
fn fr_cfg_03_secret_store_roundtrip() {
    let store = MemoryStore::new();
    assert_eq!(store.get("host:9204"), None);

    store.set("host:9204", "hunter2").unwrap();
    assert_eq!(store.get("host:9204").as_deref(), Some("hunter2"));

    store.delete("host:9204").unwrap();
    assert_eq!(store.get("host:9204"), None);
    store.delete("host:9204").unwrap(); // deleting absent is fine
}

/// A config round-trips through TOML unchanged (profile + prefs).
///
/// trace: FR-CFG-01, FR-CFG-02
#[test]
fn fr_cfg_01_toml_roundtrip() {
    let cfg = Config {
        last: Some(Profile {
            host: "192.168.1.100".into(),
            port: 9204,
            use_tls: true,
            remember: false,
        }),
        prefs: Prefs {
            tune_step_hz: 50,
            ..Default::default()
        },
        peers: Default::default(),
    };
    let toml = cfg.to_toml().unwrap();
    assert_eq!(Config::from_toml(&toml).unwrap(), cfg);
}

/// The serialized config contains no password/secret field (secret-free by
/// construction).
///
/// trace: FR-CFG-03
#[test]
fn fr_cfg_03_no_secret_in_serialized_config() {
    let cfg = Config {
        last: Some(Profile {
            host: "host".into(),
            port: 9204,
            use_tls: true,
            remember: false,
        }),
        ..Default::default()
    };
    let toml = cfg.to_toml().unwrap().to_lowercase();
    assert!(!toml.contains("password"));
    assert!(!toml.contains("secret"));
    assert!(!toml.contains("psk"));
}

/// `redact` masks the secret and never leaks it (NFR-SEC-01).
///
/// trace: NFR-SEC-01
#[test]
fn nfr_sec_01_redact_masks_secret() {
    let masked = redact("connect failed for pw hunter2 on host", "hunter2");
    assert_eq!(masked, "connect failed for pw *** on host");
    assert!(!masked.contains("hunter2"));
    // Empty secret is a no-op.
    assert_eq!(redact("nothing to hide", ""), "nothing to hide");
}

/// Loading a missing file yields the default config (startup never fails).
///
/// trace: FR-CFG-01
#[test]
fn fr_cfg_01_load_missing_is_default() {
    let path = std::env::temp_dir().join("k4cfg-does-not-exist-xyz.toml");
    assert_eq!(Config::load(&path), Config::default());
}

/// Save then load round-trips through a real file.
///
/// trace: FR-CFG-01
#[test]
fn fr_cfg_01_save_load_file_roundtrip() {
    let path = std::env::temp_dir().join(format!("k4cfg-test-{}.toml", std::process::id()));
    let cfg = Config {
        last: Some(Profile {
            host: "10.0.0.5".into(),
            port: 9205,
            use_tls: false,
            remember: true,
        }),
        prefs: Prefs {
            tune_step_hz: 10,
            ..Default::default()
        },
        peers: Default::default(),
    };
    cfg.save(&path).unwrap();
    assert_eq!(Config::load(&path), cfg);
    let _ = std::fs::remove_file(&path);
}

/// The last session (connection profile) and peer cache persist across a
/// save/load cycle — the app remembers them on restart (FR-CFG-05, FR-CFG-04).
///
/// trace: FR-CFG-05, FR-CFG-04
#[test]
fn fr_cfg_05_remembers_last_session_and_peers() {
    let path = std::env::temp_dir().join(format!("k4cfg-peers-{}.toml", std::process::id()));
    let mut peers = k4_config::PeerCache::default();
    peers.upsert(Peer {
        name: "radio".into(),
        host: "radio.lan".into(),
        port: 9204,
        use_tls: true,
        secret: PeerSecret::Keyring,
    });
    let cfg = Config {
        last: Some(Profile {
            host: "radio.lan".into(),
            port: 9204,
            use_tls: true,
            remember: true,
        }),
        peers,
        ..Default::default()
    };
    cfg.save(&path).unwrap();
    let loaded = Config::load(&path);
    assert_eq!(loaded.last, cfg.last);
    assert_eq!(loaded.peers.peers.len(), 1);
    assert_eq!(loaded.peers.peers[0].host, "radio.lan");
    assert_eq!(loaded.peers.peers[0].secret, PeerSecret::Keyring);
    // No plaintext password anywhere in the serialized config.
    let toml = cfg.to_toml().unwrap().to_lowercase();
    assert!(!toml.contains("password"));
    let _ = std::fs::remove_file(&path);
}

/// Audio device selection + local levels + theme persist across save/load — the
/// app remembers them on restart (FR-AUD-DEV-01, FR-AUD-LVL-01, FR-CFG-05).
///
/// trace: FR-AUD-DEV-01, FR-AUD-LVL-01, FR-CFG-05
#[test]
fn fr_aud_dev_lvl_settings_persist() {
    let cfg = Config {
        prefs: Prefs {
            audio_output: Some("USB Audio".into()),
            audio_input: Some("Default Mic".into()),
            volume_pct: 150,
            mic_gain_pct: 80,
            theme: Some("contrast".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    let back = Config::from_toml(&cfg.to_toml().unwrap()).unwrap();
    assert_eq!(back.prefs.audio_output.as_deref(), Some("USB Audio"));
    assert_eq!(back.prefs.audio_input.as_deref(), Some("Default Mic"));
    assert_eq!(back.prefs.volume_pct, 150);
    assert_eq!(back.prefs.mic_gain_pct, 80);
    assert_eq!(back.prefs.theme.as_deref(), Some("contrast"));
}

/// The K-Pod control surface is an opt-in: it defaults off and round-trips
/// through the config file (FR-KPOD-05). The runtime behaviour when no K-Pod is
/// attached is demonstration-verified (the worker retries discovery and never
/// blocks/panics).
///
/// trace: FR-KPOD-05
#[test]
fn fr_kpod_05_enable_is_opt_in_and_persists() {
    assert!(
        !Prefs::default().kpod_enabled,
        "K-Pod must be off by default"
    );
    let cfg = Config {
        prefs: Prefs {
            kpod_enabled: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let back = Config::from_toml(&cfg.to_toml().unwrap()).unwrap();
    assert!(back.prefs.kpod_enabled, "enabling K-Pod must persist");
}
