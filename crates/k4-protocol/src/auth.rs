//! Connection authentication (FR-AUTH-01).
//!
//! On the unencrypted port (default 9205) the client proves knowledge of the
//! password by sending `SHA-384(password)` as a lowercase hex string, raw (not
//! frame-wrapped), immediately after the TCP connect. Source: R-EXT-01.

use sha2::{Digest, Sha384};
use std::fmt::Write as _;

/// Compute the K4 auth token: lowercase hex of `SHA-384(password)`.
///
/// trace: FR-AUTH-01
pub fn auth_hash(password: &str) -> String {
    let digest = Sha384::digest(password.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        // infallible: writing to a String never errors
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}
