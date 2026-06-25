//! Authentication tests. trace: FR-AUTH-01
use k4_protocol::auth::auth_hash;

/// Known-answer test: lowercase hex of SHA-384("remotek4-test-pw").
/// Vector computed independently with Python `hashlib.sha384`.
///
/// trace: FR-AUTH-01
#[test]
fn fr_auth_01_sha384_hex_known_answer() {
    let got = auth_hash("remotek4-test-pw");
    assert_eq!(
        got,
        "10a702b979a1a56a83ebc4ae2dc4a42aa683e286f91b0f2e6d768e1b47a6ed8ea1e760ec7baf71ef73801d067695d670"
    );
}
