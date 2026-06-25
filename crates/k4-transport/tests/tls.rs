//! TLS-PSK transport test (enabled with the `tls` feature).
//! trace: FR-AUTH-02
#![cfg(feature = "tls")]

use k4_transport::tls_support::psk_loopback;
use k4_transport::{ConnectConfig, TcpRemoteTransport};

/// A TLS-PSK handshake with the correct pre-shared key succeeds and a CAT
/// GET/RESP round-trips over the encrypted channel.
///
/// trace: FR-AUTH-02
#[test]
fn fr_auth_02_tls_psk_connect_and_roundtrip() {
    let addr = psk_loopback("secretpsk", "FA00014074000;").unwrap();
    let cfg = ConnectConfig {
        password: "secretpsk".into(),
        ..Default::default()
    };

    let mut t = TcpRemoteTransport::connect_tls(addr, &cfg).expect("TLS-PSK connect");
    t.send_cat("FA;").unwrap();

    let mut got = None;
    for _ in 0..20 {
        if let Ok(messages) = t.poll_cat() {
            if let Some(m) = messages.into_iter().find(|m| m.starts_with("FA")) {
                got = Some(m);
                break;
            }
        }
    }
    assert_eq!(got.as_deref(), Some("FA00014074000;"));
}

/// A wrong pre-shared key fails the handshake (connect errors).
///
/// trace: FR-AUTH-02
#[test]
fn fr_auth_02_tls_psk_wrong_key_fails() {
    let addr = psk_loopback("rightkey", "FA00014074000;").unwrap();
    let cfg = ConnectConfig {
        password: "wrongkey".into(),
        ..Default::default()
    };
    assert!(TcpRemoteTransport::connect_tls(addr, &cfg).is_err());
}
