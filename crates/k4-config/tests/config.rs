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
    assert_eq!(store.get("host:9204").unwrap(), None);

    store.set("host:9204", "hunter2").unwrap();
    assert_eq!(store.get("host:9204").unwrap().as_deref(), Some("hunter2"));

    store.delete("host:9204").unwrap();
    assert_eq!(store.get("host:9204").unwrap(), None);
    store.delete("host:9204").unwrap(); // deleting absent is fine
}

/// `Ok(None)` (no secret saved) and `Err` (the store could not be read) are
/// different facts and must stay distinguishable end to end — collapsing them
/// is exactly what let a saved password silently read back as "none" (FR-CFG-08).
///
/// trace: FR-CFG-08
#[test]
fn fr_cfg_08_a_store_failure_is_not_a_clean_miss() {
    struct FailingStore;
    impl SecretStore for FailingStore {
        fn get(&self, _account: &str) -> Result<Option<String>, k4_config::SecretError> {
            Err(k4_config::SecretError("locked".into()))
        }
        fn set(&self, _account: &str, _secret: &str) -> Result<(), k4_config::SecretError> {
            Ok(())
        }
        fn delete(&self, _account: &str) -> Result<(), k4_config::SecretError> {
            Ok(())
        }
    }
    let err = FailingStore.get("host:9204").unwrap_err();
    assert_eq!(err.to_string(), "secret store error: locked");
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
        ..Default::default()
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
        // Empty the K-Pod macro table for this check: macro text is operator
        // content that can legitimately contain e.g. the "PSK" mode name, which
        // would trip the `psk` substring guard below — this test is about the
        // connection profile never carrying secret material, not user macros.
        prefs: Prefs {
            kpod_buttons: Vec::new(),
            ..Default::default()
        },
        ..Default::default()
    };
    // The spot-network table is named after PSK Reporter (FR-SPOT-04) and its
    // default broker host is mqtt.pskreporter.info (FR-SPOT-05): a spotting
    // service, not TLS-PSK key material. Exempt exactly those two tokens; any
    // other "psk" in the file still trips the guard.
    let toml = cfg
        .to_toml()
        .unwrap()
        .to_lowercase()
        .replace("psk_reporter", "")
        .replace("pskreporter", "");
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
        ..Default::default()
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

/// The K-Pod function-switch table has 16 slots seeded from the Elecraft sample
/// macros, and a customized table round-trips through the config file
/// (FR-KPOD-06).
///
/// trace: FR-KPOD-06
#[test]
fn fr_kpod_06_button_macros_seed_and_persist() {
    use k4_config::{default_kpod_buttons, KpodButton, KPOD_SLOT_COUNT};

    // Seed defaults are the full 16 slots, with the leading ones populated from
    // the Elecraft samples (non-empty CAT).
    let seed = default_kpod_buttons();
    assert_eq!(seed.len(), KPOD_SLOT_COUNT, "16 slots");
    assert!(
        seed.iter().any(|b| !b.cat.is_empty()),
        "seed must carry sample macros"
    );
    assert!(
        seed.iter()
            .all(|b| b.cat.is_empty() || b.cat.ends_with(';')),
        "every assigned macro is a `;`-terminated CAT string"
    );
    assert_eq!(
        Prefs::default().kpod_buttons,
        seed,
        "default Prefs uses the seed"
    );

    // A user edit to a slot survives a serialize/parse round-trip.
    let mut buttons = seed.clone();
    buttons[3] = KpodButton {
        label: "My CW".into(),
        cat: "MD3;BW0040;".into(),
    };
    let cfg = Config {
        prefs: Prefs {
            kpod_buttons: buttons.clone(),
            ..Default::default()
        },
        ..Default::default()
    };
    let back = Config::from_toml(&cfg.to_toml().unwrap()).unwrap();
    assert_eq!(
        back.prefs.kpod_buttons, buttons,
        "edited table must persist"
    );
}

/// The automatic update check is opt-out: default on, and it survives a
/// save/load round-trip so an operator's choice sticks.
/// trace: FR-UI-UPD-02
#[test]
fn fr_ui_upd_02_auto_update_check_defaults_on_and_persists() {
    assert!(
        Prefs::default().auto_update_check,
        "default opt-in: the check is on unless turned off"
    );

    // A round-trip through TOML preserves an explicit opt-out.
    let prefs = Prefs {
        auto_update_check: false,
        ..Default::default()
    };
    let toml = toml::to_string(&prefs).expect("serialize");
    let back: Prefs = toml::from_str(&toml).expect("deserialize");
    assert!(!back.auto_update_check, "opt-out is remembered");
}

/// KPA1500 support is opt-in (default off) with a sensible default port/poll,
/// and the enable flag plus the connection settings survive a save/load
/// round-trip so the operator configures the amp once.
/// trace: FR-AMP-01
#[test]
fn fr_amp_01_kpa1500_defaults_off_and_persists() {
    let def = Prefs::default();
    assert!(!def.kpa1500_enabled, "default opt-in: support is off");
    assert_eq!(def.kpa1500_port, 1500, "the amp's command-server port");
    assert_eq!(def.kpa1500_poll_ms, 500);
    assert!(def.kpa1500_host.is_empty());

    // A configured amp round-trips through TOML unchanged.
    let prefs = Prefs {
        kpa1500_enabled: true,
        kpa1500_host: "192.168.1.50".into(),
        kpa1500_port: 1500,
        kpa1500_poll_ms: 250,
        ..Default::default()
    };
    let toml = toml::to_string(&prefs).expect("serialize");
    let back: Prefs = toml::from_str(&toml).expect("deserialize");
    assert!(back.kpa1500_enabled);
    assert_eq!(back.kpa1500_host, "192.168.1.50");
    assert_eq!(back.kpa1500_port, 1500);
    assert_eq!(back.kpa1500_poll_ms, 250);

    // A config written before this feature (no KPA fields) loads with the
    // opt-in default off — never surprising an upgrader with an amp link.
    let legacy = "tune_step_hz = 100";
    let old: Prefs = toml::from_str(legacy).expect("legacy config");
    assert!(!old.kpa1500_enabled);
    assert_eq!(old.kpa1500_port, 1500);
}

/// FR-SPOT-03: the spot age limit defaults to 15 min, is bounded to 1 min – 24 h,
/// survives a TOML round-trip, and an unusable value — typed into Settings or
/// hand-edited into the file — resolves to the default instead of being kept.
/// trace: FR-SPOT-03
#[test]
fn fr_spot_03_max_age_default_bounds_persist() {
    use k4_config::{parse_spot_max_age_min, sanitise_spot_max_age_min};

    assert_eq!(Prefs::default().spot_max_age_min(), 15, "the default limit");

    // Bounds: both edges are legal, one step outside is not.
    assert_eq!(sanitise_spot_max_age_min(1), 1);
    assert_eq!(sanitise_spot_max_age_min(24 * 60), 24 * 60);
    assert_eq!(sanitise_spot_max_age_min(0), 15, "0 would hide every spot");
    assert_eq!(sanitise_spot_max_age_min(24 * 60 + 1), 15);

    // The Settings field: digits in range are taken, anything else is the default.
    assert_eq!(parse_spot_max_age_min("30"), 30);
    assert_eq!(parse_spot_max_age_min(" 45 "), 45);
    for bad in ["", "abc", "-5", "0", "99999", "12.5"] {
        assert_eq!(parse_spot_max_age_min(bad), 15, "input {bad:?}");
    }

    // A chosen limit round-trips through TOML.
    let prefs = Prefs {
        spot_max_age_min: 60,
        ..Default::default()
    };
    let back: Prefs = toml::from_str(&toml::to_string(&prefs).expect("serialize")).expect("parse");
    assert_eq!(back.spot_max_age_min(), 60);

    // A config from before this feature loads with the default, and a
    // hand-edited out-of-range value reads back as the default too.
    let old: Prefs = toml::from_str("tune_step_hz = 100").expect("legacy config");
    assert_eq!(old.spot_max_age_min(), 15);
    let edited: Prefs = toml::from_str("tune_step_hz = 100\nspot_max_age_min = 0").expect("edited");
    assert_eq!(edited.spot_max_age_min(), 15);
}

/// FR-SPOT-04: every spotting network defaults to off — a fresh install, and a
/// config written before this feature, contact no third party — and a configured
/// set of networks round-trips through TOML unchanged.
/// trace: FR-SPOT-04
#[test]
fn fr_spot_04_networks_default_off_and_persist() {
    let def = Prefs::default().spot_networks;
    assert!(!def.any_enabled(), "no network is on by default");
    assert!(!def.psk_reporter.enabled && !def.rbn.enabled && !def.dx_cluster.enabled);
    assert_eq!(def.psk_reporter.host, "mqtt.pskreporter.info");
    assert_eq!(def.psk_reporter.port, 1883);
    assert_eq!(def.rbn.host, "telnet.reversebeacon.net");
    assert_eq!(def.rbn.port, 7000);
    assert!(
        def.dx_cluster.host.is_empty(),
        "a cluster needs a chosen host"
    );

    // A configured set round-trips: each network keeps its own enable + settings.
    let mut nets = def.clone();
    nets.psk_reporter.enabled = true;
    nets.psk_reporter.host = "broker.example.org".into();
    nets.psk_reporter.port = 8883;
    nets.rbn.enabled = true;
    nets.rbn.login = "DC0SK".into();
    nets.dx_cluster.host = "cluster.example.org".into();
    nets.dx_cluster.port = 7373;
    let prefs = Prefs {
        spot_networks: nets.clone(),
        ..Default::default()
    };
    let back: Prefs = toml::from_str(&toml::to_string(&prefs).expect("serialize")).expect("parse");
    assert_eq!(back.spot_networks, nets);
    assert!(back.spot_networks.any_enabled());
    assert!(
        !back.spot_networks.dx_cluster.enabled,
        "toggles are independent"
    );

    // The Settings port field: a usable number is taken, anything else is the
    // default rather than a saved zero.
    use k4_config::parse_spot_port;
    assert_eq!(parse_spot_port("7373", 7300), 7373);
    for bad in ["", "0", "abc", "70000", "-1"] {
        assert_eq!(parse_spot_port(bad, 7300), 7300, "port {bad:?}");
    }

    // POTA: off, asked once a minute by default; its interval and switch round-trip, and a stray
    // interval in a hand-edited file is brought back into bounds when read.
    assert!(!def.pota.enabled);
    assert_eq!(def.pota.poll_secs(), 60);
    let mut with_pota = def.clone();
    with_pota.pota.enabled = true;
    with_pota.pota.poll_secs = 120;
    let prefs = Prefs {
        spot_networks: with_pota.clone(),
        ..Default::default()
    };
    let back: Prefs = toml::from_str(&toml::to_string(&prefs).expect("serialize")).expect("parse");
    assert_eq!(back.spot_networks, with_pota);
    assert!(back.spot_networks.any_enabled(), "POTA alone counts as on");
    for (stray, want) in [
        (0, 60),
        (1, 60),
        (29, 60),
        (30, 30),
        (3600, 3600),
        (3601, 60),
    ] {
        let p: Prefs = toml::from_str(&format!(
            "tune_step_hz = 100\n[spot_networks.pota]\nenabled = true\npoll_secs = {stray}\n"
        ))
        .expect("a config with an interval");
        assert_eq!(p.spot_networks.pota.poll_secs(), want, "interval {stray}");
    }
    use k4_config::parse_spot_poll_secs;
    assert_eq!(parse_spot_poll_secs("120"), 120);
    assert_eq!(parse_spot_poll_secs(" 45 "), 45);
    for bad in ["", "0", "29", "3601", "abc", "-5", "1.5"] {
        assert_eq!(parse_spot_poll_secs(bad), 60, "interval {bad:?}");
    }

    // FreeDV Reporter: off, at the service's own host and plain-WebSocket port; its settings
    // round-trip, it counts as a network being on, and a config without it loads with the defaults.
    assert!(!def.freedv.enabled);
    assert_eq!(
        (def.freedv.host.as_str(), def.freedv.port),
        ("qso.freedv.org", 80)
    );
    let mut with_freedv = def.clone();
    with_freedv.freedv.enabled = true;
    with_freedv.freedv.host = "reporter.example.org".into();
    with_freedv.freedv.port = 8080;
    let prefs = Prefs {
        spot_networks: with_freedv.clone(),
        ..Default::default()
    };
    let back: Prefs = toml::from_str(&toml::to_string(&prefs).expect("serialize")).expect("parse");
    assert_eq!(back.spot_networks, with_freedv);
    assert!(
        back.spot_networks.any_enabled(),
        "FreeDV alone counts as on"
    );
    assert!(!back.spot_networks.pota.enabled, "toggles are independent");
    let partial: Prefs =
        toml::from_str("tune_step_hz = 100\n[spot_networks.freedv]\nenabled = true\n")
            .expect("a config naming only the switch");
    assert!(partial.spot_networks.freedv.enabled);
    assert_eq!(partial.spot_networks.freedv.host, "qso.freedv.org");
    assert_eq!(partial.spot_networks.freedv.port, 80); // A section that names the network but not the switch leaves it off: nothing is ever turned on
                                                       // by a file that does not say so.
    let silent: Prefs =
        toml::from_str("tune_step_hz = 100\n[spot_networks.freedv]\nhost = \"h.example\"\n")
            .expect("a config that omits the switch");
    assert!(!silent.spot_networks.freedv.enabled);
    assert!(!silent.spot_networks.any_enabled());
    assert_eq!(silent.spot_networks.freedv.host, "h.example");

    // TLS is off by default and persists; approved certificates start empty.
    assert!(!def.psk_reporter.tls);
    assert!(def.trusted().is_empty());
    let mut enc = def.clone();
    enc.psk_reporter.tls = true;
    enc.psk_reporter.port = 1884;
    let prefs = Prefs {
        spot_networks: enc.clone(),
        ..Default::default()
    };
    let back: Prefs = toml::from_str(&toml::to_string(&prefs).expect("serialize")).expect("parse");
    assert_eq!(back.spot_networks, enc);

    // A config from when PSK Reporter was polled still loads: the old interval is ignored.
    let legacy_psk: Prefs = toml::from_str(
        "tune_step_hz = 100\n[spot_networks.psk_reporter]\nenabled = true\npoll_secs = 600\n",
    )
    .expect("a config with the old poll interval");
    assert!(legacy_psk.spot_networks.psk_reporter.enabled);
    assert_eq!(
        legacy_psk.spot_networks.psk_reporter.host,
        "mqtt.pskreporter.info"
    );

    // A pre-feature config — and one naming only some networks — loads with the
    // rest off and at their defaults.
    let old: Prefs = toml::from_str("tune_step_hz = 100").expect("legacy config");
    assert!(!old.spot_networks.any_enabled());
    assert_eq!(old.spot_networks, def);
    let partial: Prefs =
        toml::from_str("tune_step_hz = 100\n[spot_networks.psk_reporter]\nenabled = true\n")
            .expect("partial config");
    assert!(partial.spot_networks.psk_reporter.enabled);
    assert_eq!(
        partial.spot_networks.psk_reporter.host,
        "mqtt.pskreporter.info"
    );
    assert_eq!(partial.spot_networks.psk_reporter.port, 1883);
    assert_eq!(
        partial.spot_networks.rbn, def.rbn,
        "unnamed network keeps its defaults"
    );
}

/// FR-SPOT-13: approved certificates round-trip, adding and withdrawing works on exactly the entry
/// named, and a malformed, duplicated or excess entry in a hand-edited file is dropped when read.
/// trace: FR-SPOT-13
#[test]
fn fr_spot_13_trusted_certificates_persist_and_are_validated() {
    use k4_config::{TrustedCert, MAX_TRUSTED_CERTS};
    let fp = |c: char| c.to_string().repeat(64);
    let cert = |host: &str, port: u16, c: char| TrustedCert {
        host: host.into(),
        port,
        sha256: fp(c),
    };
    let mut nets = Prefs::default().spot_networks;
    assert!(nets.trust(cert("mqtt.example.org", 1884, 'a')));
    assert!(
        nets.trust(cert("mqtt.example.org", 1884, 'b')),
        "a second certificate for one host"
    );
    assert!(
        nets.trust(cert("mqtt.example.org", 8883, 'a')),
        "the same certificate on another port"
    );
    assert!(
        !nets.trust(cert("mqtt.example.org", 1884, 'a')),
        "a duplicate"
    );
    assert_eq!(nets.trusted().len(), 3);

    // Round trip.
    let prefs = Prefs {
        spot_networks: nets.clone(),
        ..Default::default()
    };
    let text = toml::to_string(&prefs).expect("serialize");
    let back: Prefs = toml::from_str(&text).expect("parse");
    assert_eq!(back.spot_networks.trusted(), nets.trusted());

    // Withdrawing removes exactly the one named.
    assert!(nets.forget(1));
    assert_eq!(
        nets.trusted(),
        vec![
            cert("mqtt.example.org", 1884, 'a'),
            cert("mqtt.example.org", 8883, 'a')
        ]
    );
    assert!(!nets.forget(2), "no such entry");
    assert!(!nets.forget(usize::MAX));

    // Malformed entries are refused when adding...
    for bad in [
        cert("", 1884, 'a'),
        cert("host name", 1884, 'a'),
        cert("host\n", 1884, 'a'),
        cert("h\u{e9}st", 1884, 'a'),
        cert("mqtt.example.org", 0, 'a'),
        // Printable, but not a host name: none of these may reach a connection or a file.
        cert("a/b", 1884, 'a'),
        cert("h@st", 1884, 'a'),
        cert("h'st", 1884, 'a'),
        cert("h;st", 1884, 'a'),
        cert("h\"st", 1884, 'a'),
        TrustedCert {
            host: "h.example".into(),
            port: 1,
            sha256: format!("{}a", fp('a')),
        },
        TrustedCert {
            host: "h.example".into(),
            port: 1,
            sha256: fp('a')[..63].into(),
        },
        TrustedCert {
            host: "h.example".into(),
            port: 1,
            sha256: fp('A'),
        },
        TrustedCert {
            host: "h.example".into(),
            port: 1,
            sha256: fp('g'),
        },
        TrustedCert {
            host: "h.example".into(),
            port: 1,
            sha256: String::new(),
        },
        cert(&"h".repeat(254), 1, 'a'),
    ] {
        assert!(!bad.is_valid(), "{bad:?}");
        assert!(
            !Prefs::default().spot_networks.trust(bad.clone()),
            "{bad:?}"
        );
    }
    assert!(cert(&"h".repeat(253), 1, 'a').is_valid());

    // ...and dropped when a file already holds them, along with duplicates and the excess.
    let mut file = Prefs::default().spot_networks;
    file.trusted_certs = vec![
        cert("ok.example", 1884, 'a'),
        cert("", 1, 'a'),
        cert("ok.example", 1884, 'a'),
        cert("ok.example", 0, 'b'),
        cert("also.example", 1884, 'c'),
    ];
    assert_eq!(
        file.trusted(),
        vec![
            cert("ok.example", 1884, 'a'),
            cert("also.example", 1884, 'c')
        ]
    );
    let mut full = Prefs::default().spot_networks;
    for i in 0..MAX_TRUSTED_CERTS + 10 {
        full.trusted_certs
            .push(cert(&format!("h{i}.example"), 1884, 'a'));
    }
    assert_eq!(full.trusted().len(), MAX_TRUSTED_CERTS);
    assert_eq!(
        full.trusted()[0].host,
        "h0.example",
        "the first ones are kept"
    );
    assert!(
        !full.trust(cert("new.example", 1884, 'b')),
        "the list is full"
    );

    // A config from before this feature loads with none.
    let old: Prefs = toml::from_str("tune_step_hz = 100").expect("legacy config");
    assert!(old.spot_networks.trusted().is_empty());
}

/// FR-PAN-14: the spectrum afterglow is off by default, persists, and a hand-edited or typed value
/// is brought into range — zero stays off, small non-zero values rise to the minimum, large ones
/// fall to the maximum, and an unusable entry means off.
/// trace: FR-PAN-14
#[test]
fn fr_pan_14_afterglow_setting_persists_and_is_bounded() {
    use k4_config::{
        parse_afterglow_ms, sanitise_afterglow_ms, AFTERGLOW_DEFAULT_MS, AFTERGLOW_MAX_MS,
        AFTERGLOW_MIN_MS,
    };
    // A user-requested, documented default: pinned as a literal so it can't drift by one
    // with nothing failing (mutate this by hand to check — cargo-mutants does not mutate
    // `const` items).
    assert_eq!(AFTERGLOW_DEFAULT_MS, 500);
    assert_eq!(
        Prefs::default().spectrum_afterglow_ms(),
        AFTERGLOW_DEFAULT_MS,
        "on by default, at a gentle trail length"
    );

    let prefs = Prefs {
        spectrum_afterglow_ms: 750,
        ..Default::default()
    };
    let back: Prefs = toml::from_str(&toml::to_string(&prefs).expect("serialize")).expect("parse");
    assert_eq!(back.spectrum_afterglow_ms(), 750);

    // A config from before this feature existed has no opinion on it, so it picks up
    // today's default rather than being silently switched off.
    let old: Prefs = toml::from_str("tune_step_hz = 100").expect("legacy config");
    assert_eq!(old.spectrum_afterglow_ms(), AFTERGLOW_DEFAULT_MS);

    // A config that explicitly saved `0` (e.g. from before the default changed, or an
    // operator choosing it off) keeps that choice — the loop below covers `0` alongside
    // every other hand-edited value, so it is not repeated here.

    // A hand-edited value is brought into range when read.
    for (stray, want) in [
        (0, 0),
        (1, AFTERGLOW_MIN_MS),
        (AFTERGLOW_MIN_MS - 1, AFTERGLOW_MIN_MS),
        (AFTERGLOW_MIN_MS, AFTERGLOW_MIN_MS),
        (2500, 2500),
        (AFTERGLOW_MAX_MS, AFTERGLOW_MAX_MS),
        (AFTERGLOW_MAX_MS + 1, AFTERGLOW_MAX_MS),
        (u32::MAX, AFTERGLOW_MAX_MS),
    ] {
        assert_eq!(sanitise_afterglow_ms(stray), want, "value {stray}");
        let p: Prefs = toml::from_str(&format!(
            "tune_step_hz = 100\nspectrum_afterglow_ms = {stray}\n"
        ))
        .expect("a config with an afterglow");
        assert_eq!(p.spectrum_afterglow_ms(), want, "file value {stray}");
    }

    // The Settings field: digits in range are taken, out of range is clamped, anything else is off.
    assert_eq!(parse_afterglow_ms("500"), 500);
    assert_eq!(parse_afterglow_ms(" 500 "), 500);
    assert_eq!(parse_afterglow_ms("0"), 0);
    assert_eq!(parse_afterglow_ms("10"), AFTERGLOW_MIN_MS);
    assert_eq!(parse_afterglow_ms("99999"), AFTERGLOW_MAX_MS);
    for bad in ["", "abc", "-5", "1.5", "5 0", "٣٠٠", "4294967296"] {
        assert_eq!(parse_afterglow_ms(bad), 0, "entry {bad:?}");
    }
}

/// FR-CFG-09: the one-time afterglow-default migration, at the `Config` level (not `Prefs`
/// alone — the flag that distinguishes "never migrated" from "deliberately off" lives on
/// `Config`, so a bare `Prefs` deserialize, as above, does not exercise it).
///
/// trace: FR-CFG-09
#[test]
fn fr_cfg_09_afterglow_migrates_once_and_respects_a_later_choice() {
    use k4_config::AFTERGLOW_DEFAULT_MS;

    // A file saved by a build before this migration existed: an explicit `0` (the old
    // default, never a deliberate choice) and no migration flag at all. It must come back
    // as today's default, and the file must now say so was migrated.
    let path = std::env::temp_dir().join(format!("k4cfg-migrate-old-{}.toml", std::process::id()));
    std::fs::write(
        &path,
        "[prefs]\ntune_step_hz = 100\nspectrum_afterglow_ms = 0\n",
    )
    .unwrap();
    let loaded = Config::load(&path);
    assert_eq!(
        loaded.prefs.spectrum_afterglow_ms(),
        AFTERGLOW_DEFAULT_MS,
        "an old file's implicit 0 must be promoted to today's default"
    );
    assert!(
        loaded.afterglow_default_migrated,
        "a loaded config must record that migration has now run"
    );
    let _ = std::fs::remove_file(&path);

    // A file already marked migrated, with an explicit `0`: the operator chose it after
    // upgrading, and it must NOT be silently promoted back to the default.
    let path2 = std::env::temp_dir().join(format!("k4cfg-migrate-new-{}.toml", std::process::id()));
    std::fs::write(
        &path2,
        "afterglow_default_migrated = true\n[prefs]\ntune_step_hz = 100\nspectrum_afterglow_ms = 0\n",
    )
    .unwrap();
    let loaded2 = Config::load(&path2);
    assert_eq!(
        loaded2.prefs.spectrum_afterglow_ms(),
        0,
        "a deliberate 0, chosen after migration, must be respected, not overridden"
    );
    let _ = std::fs::remove_file(&path2);

    // A brand new, never-saved config has nothing to migrate and is marked as such by
    // construction, not by having actually run the migration.
    assert!(Config::default().afterglow_default_migrated);
}
