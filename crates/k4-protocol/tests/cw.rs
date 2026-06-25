//! CW keying tests. trace: FR-TX-CW-01, FR-TX-CW-02, FR-TX-SAFE-02
use k4_protocol::cw::{encode_kz, encode_kzf, encode_kzl, KeyElement::*};

/// Paddle elements encode to dits/dahs (the letter C = dah-dit-dah-dit).
///
/// trace: FR-TX-CW-01
#[test]
fn fr_tx_cw_01_paddle_elements() {
    assert_eq!(encode_kz(&[Dah, Dit, Dah, Dit]), "KZ-.-.;");
}

/// Key up/down elements use the documented 4-digit ms form (doc example:
/// a single dit from a straight key).
///
/// trace: FR-TX-CW-01
#[test]
fn fr_tx_cw_01_key_elements_and_pause() {
    assert_eq!(encode_kz(&[Down(0), Up(90)]), "KZD0000U0090;");

    // "CQ" with an 80 ms pause: C = -.-. , Q = --.-
    let cq = [Dah, Dit, Dah, Dit, Pause(80), Dah, Dah, Dit, Dah];
    assert_eq!(encode_kz(&cq), "KZ-.-.P0080--.-;");
}

/// Timing values are clamped to the 2500 ms maximum.
///
/// trace: FR-TX-CW-01
#[test]
fn fr_tx_cw_01_clamps_timing() {
    assert_eq!(encode_kz(&[Down(9999)]), "KZD2500;");
    assert_eq!(encode_kz(&[Pause(2500)]), "KZP2500;");
}

/// KZL key-down initial delay (default 80 ms), clamped to 1000 ms.
///
/// trace: FR-TX-CW-02
#[test]
fn fr_tx_cw_02_kzl_delay() {
    assert_eq!(encode_kzl(80), "KZL0080;");
    assert_eq!(encode_kzl(5000), "KZL1000;");
}

/// KZF fail-safe timeout (minutes), clamped to 1–10.
///
/// trace: FR-TX-SAFE-02
#[test]
fn fr_tx_safe_02_kzf_failsafe() {
    assert_eq!(encode_kzf(3), "KZF03;");
    assert_eq!(encode_kzf(0), "KZF01;");
    assert_eq!(encode_kzf(99), "KZF10;");
}
