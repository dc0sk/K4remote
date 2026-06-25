//! Config tests. trace: FR-CFG-01, FR-CFG-02, FR-CFG-03, NFR-SEC-01
use k4_config::{redact, Config, MemoryStore, Prefs, Profile, SecretStore};

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
        prefs: Prefs { tune_step_hz: 50 },
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
        prefs: Prefs { tune_step_hz: 10 },
    };
    cfg.save(&path).unwrap();
    assert_eq!(Config::load(&path), cfg);
    let _ = std::fs::remove_file(&path);
}
