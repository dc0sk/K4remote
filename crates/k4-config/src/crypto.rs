//! Master-password encryption for cached peer passwords (FR-CFG-04, NFR-SEC-03).
//!
//! A key is derived from the user's master password and a per-store random salt
//! with **Argon2id** (memory-hard KDF); each secret is sealed with
//! **ChaCha20-Poly1305** (AEAD) under a fresh random nonce. A wrong master
//! password (or tampered ciphertext) fails the AEAD tag check rather than
//! yielding plaintext. Salt/nonce/ciphertext are stored hex-encoded in the TOML
//! config; the derived key lives only in memory.

use argon2::Argon2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};

/// Store-level KDF salt length (bytes).
pub const SALT_LEN: usize = 16;
/// ChaCha20-Poly1305 nonce length (bytes).
pub const NONCE_LEN: usize = 12;
/// A fixed marker sealed under the master key so a wrong password is detected
/// even with no peers stored.
const VERIFIER_PLAINTEXT: &[u8] = b"K4REMOTE-MASTER-OK";

/// Error from the peer-secret crypto.
#[derive(Debug, PartialEq, Eq)]
pub struct CryptoError(pub String);

impl std::fmt::Display for CryptoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "peer-secret crypto error: {}", self.0)
    }
}
impl std::error::Error for CryptoError {}

/// A sealed secret: random nonce + AEAD ciphertext (incl. tag), hex-encoded for
/// TOML storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sealed {
    /// Hex nonce (`NONCE_LEN` bytes).
    pub nonce: String,
    /// Hex ciphertext + 16-byte Poly1305 tag.
    pub ct: String,
}

/// Generate a fresh random store salt (hex).
pub fn new_salt() -> String {
    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    hex::encode(salt)
}

/// A 256-bit key derived from a master password + store salt. Zeroized on drop.
pub struct MasterKey([u8; 32]);

impl Drop for MasterKey {
    fn drop(&mut self) {
        // Volatile wipe — a plain `= 0` loop can be elided by the optimiser
        // since the bytes are never read again (audit #11).
        use zeroize::Zeroize;
        self.0.zeroize();
    }
}

impl MasterKey {
    /// Derive the key from `master` and a hex `salt` (Argon2id).
    pub fn derive(master: &str, salt_hex: &str) -> Result<Self, CryptoError> {
        let salt = hex::decode(salt_hex).map_err(|e| CryptoError(format!("bad salt: {e}")))?;
        let mut key = [0u8; 32];
        Argon2::default()
            .hash_password_into(master.as_bytes(), &salt, &mut key)
            .map_err(|e| CryptoError(e.to_string()))?;
        Ok(MasterKey(key))
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(Key::from_slice(&self.0))
    }

    /// Seal `plaintext` under a fresh random nonce.
    pub fn seal(&self, plaintext: &str) -> Result<Sealed, CryptoError> {
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);
        let ct = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
            .map_err(|e| CryptoError(e.to_string()))?;
        Ok(Sealed {
            nonce: hex::encode(nonce_bytes),
            ct: hex::encode(ct),
        })
    }

    /// Open a [`Sealed`] secret; wrong key / tampered data fails the tag check.
    pub fn open(&self, sealed: &Sealed) -> Result<String, CryptoError> {
        let nonce =
            hex::decode(&sealed.nonce).map_err(|e| CryptoError(format!("bad nonce: {e}")))?;
        let ct =
            hex::decode(&sealed.ct).map_err(|e| CryptoError(format!("bad ciphertext: {e}")))?;
        if nonce.len() != NONCE_LEN {
            return Err(CryptoError("bad nonce length".into()));
        }
        let pt = self
            .cipher()
            .decrypt(Nonce::from_slice(&nonce), ct.as_ref())
            .map_err(|_| CryptoError("wrong master password or corrupt data".into()))?;
        String::from_utf8(pt).map_err(|e| CryptoError(e.to_string()))
    }

    /// Seal the fixed verifier marker (stored once per store).
    pub fn make_verifier(&self) -> Result<Sealed, CryptoError> {
        self.seal(std::str::from_utf8(VERIFIER_PLAINTEXT).unwrap())
    }

    /// Check this key against a stored verifier (i.e. the master password is
    /// correct for the store).
    pub fn verify(&self, verifier: &Sealed) -> bool {
        matches!(self.open(verifier), Ok(s) if s.as_bytes() == VERIFIER_PLAINTEXT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// trace: FR-CFG-04, NFR-SEC-03
    #[test]
    fn seal_open_round_trips_and_wrong_password_fails() {
        let salt = new_salt();
        let key = MasterKey::derive("correct horse", &salt).unwrap();
        let sealed = key.seal("hunter2").unwrap();
        // Correct key round-trips.
        assert_eq!(key.open(&sealed).unwrap(), "hunter2");

        // A different master password derives a different key → tag check fails,
        // no plaintext leaks.
        let wrong = MasterKey::derive("wrong password", &salt).unwrap();
        assert!(wrong.open(&sealed).is_err());

        // Nonce is random per seal (ciphertext differs for the same plaintext).
        let sealed2 = key.seal("hunter2").unwrap();
        assert_ne!(sealed.nonce, sealed2.nonce);
    }

    /// trace: NFR-SEC-03
    #[test]
    fn tampered_ciphertext_is_rejected() {
        let salt = new_salt();
        let key = MasterKey::derive("pw", &salt).unwrap();
        let mut sealed = key.seal("secret").unwrap();
        // Flip a byte in the ciphertext hex.
        let mut bytes = hex::decode(&sealed.ct).unwrap();
        bytes[0] ^= 0xFF;
        sealed.ct = hex::encode(bytes);
        assert!(key.open(&sealed).is_err());
    }

    /// trace: FR-CFG-04, NFR-SEC-03
    #[test]
    fn verifier_detects_wrong_master_password() {
        let salt = new_salt();
        let key = MasterKey::derive("pw", &salt).unwrap();
        let verifier = key.make_verifier().unwrap();
        assert!(key.verify(&verifier));
        let wrong = MasterKey::derive("nope", &salt).unwrap();
        assert!(!wrong.verify(&verifier));
    }
}
