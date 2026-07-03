//! Peer cache: successfully-connected servers, with per-peer password storage in
//! either the OS credential manager or the local config **encrypted** under a
//! master password (FR-CFG-04). No plaintext password is ever serialized.

use serde::{Deserialize, Serialize};

use crate::crypto::{new_salt, CryptoError, MasterKey, Sealed};

/// Where a cached peer's password lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "store", rename_all = "lowercase")]
pub enum PeerSecret {
    /// No stored password — entered on each connect.
    None,
    /// Stored in the OS credential manager, keyed by the peer's [`Peer::key`].
    Keyring,
    /// Encrypted in the config file under the store master key.
    Encrypted(Sealed),
}

/// A cached peer (a server we have connected to). **No plaintext password.**
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    /// Display name (defaults to the host).
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub use_tls: bool,
    pub secret: PeerSecret,
}

impl Peer {
    /// Stable identity for de-duplication and the keychain account (`host:port`).
    pub fn key(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// The peer cache persisted in the config (FR-CFG-04). `salt`/`verifier` back the
/// encrypted-password mode; they are absent until a master password is set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerCache {
    /// Store-level Argon2 salt (hex) for encrypted-mode peers.
    #[serde(default)]
    pub salt: String,
    /// Marker sealed under the master key, to detect a wrong master password.
    #[serde(default)]
    pub verifier: Option<Sealed>,
    /// Cached peers, most-recently-connected first.
    #[serde(default)]
    pub peers: Vec<Peer>,
}

impl PeerCache {
    /// Index of the peer matching `host:port`, if cached.
    pub fn position(&self, host: &str, port: u16) -> Option<usize> {
        self.peers
            .iter()
            .position(|p| p.host == host && p.port == port)
    }

    /// The peer matching `host:port`, if cached.
    pub fn find(&self, host: &str, port: u16) -> Option<&Peer> {
        self.position(host, port).map(|i| &self.peers[i])
    }

    /// Insert `peer` (or replace an existing one with the same key), moving it to
    /// the front (most-recent).
    pub fn upsert(&mut self, peer: Peer) {
        if let Some(i) = self.position(&peer.host, peer.port) {
            self.peers.remove(i);
        }
        self.peers.insert(0, peer);
    }

    /// Remove the peer matching `host:port`, returning it if present.
    pub fn remove(&mut self, host: &str, port: u16) -> Option<Peer> {
        self.position(host, port).map(|i| self.peers.remove(i))
    }

    /// Whether a store master password has been established.
    pub fn has_master(&self) -> bool {
        !self.salt.is_empty() && self.verifier.is_some()
    }

    /// Establish the store master password for the first time: generate a salt,
    /// derive the key, and store a verifier. Returns the derived key for sealing.
    pub fn init_master(&mut self, master: &str) -> Result<MasterKey, CryptoError> {
        let salt = new_salt();
        let key = MasterKey::derive(master, &salt)?;
        self.verifier = Some(key.make_verifier()?);
        self.salt = salt;
        Ok(key)
    }

    /// Unlock the store with `master`: derive the key from the stored salt and
    /// verify it against the stored verifier. Errors on a wrong master password.
    pub fn unlock(&self, master: &str) -> Result<MasterKey, CryptoError> {
        let verifier = self
            .verifier
            .as_ref()
            .ok_or_else(|| CryptoError("no master password set".into()))?;
        let key = MasterKey::derive(master, &self.salt)?;
        if key.verify(verifier) {
            Ok(key)
        } else {
            Err(CryptoError("wrong master password".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(host: &str, port: u16, secret: PeerSecret) -> Peer {
        Peer {
            name: host.to_string(),
            host: host.to_string(),
            port,
            use_tls: true,
            secret,
        }
    }

    /// trace: FR-CFG-04
    #[test]
    fn upsert_find_remove() {
        let mut c = PeerCache::default();
        c.upsert(peer("a.example", 9204, PeerSecret::Keyring));
        c.upsert(peer("b.example", 9204, PeerSecret::None));
        // Re-connecting to a.example moves it to the front and does not duplicate.
        c.upsert(peer("a.example", 9204, PeerSecret::Keyring));
        assert_eq!(c.peers.len(), 2);
        assert_eq!(c.peers[0].host, "a.example");
        assert!(c.find("b.example", 9204).is_some());

        let removed = c.remove("a.example", 9204).unwrap();
        assert_eq!(removed.host, "a.example");
        assert!(c.find("a.example", 9204).is_none());
        assert_eq!(c.peers.len(), 1);
    }

    /// trace: FR-CFG-04, NFR-SEC-03
    #[test]
    fn master_password_seals_and_unlocks_peer() {
        let mut c = PeerCache::default();
        assert!(!c.has_master());
        // First-time setup, then store an encrypted peer.
        let key = c.init_master("s3cret-master").unwrap();
        assert!(c.has_master());
        let sealed = key.seal("radio-password").unwrap();
        c.upsert(peer("enc.example", 9204, PeerSecret::Encrypted(sealed)));

        // Unlock with the correct master password → decrypt the peer's password.
        let key2 = c.unlock("s3cret-master").unwrap();
        if let PeerSecret::Encrypted(s) = &c.find("enc.example", 9204).unwrap().secret {
            assert_eq!(key2.open(s).unwrap(), "radio-password");
        } else {
            panic!("expected encrypted secret");
        }

        // Wrong master password is rejected at unlock (no decryption attempted).
        assert!(c.unlock("wrong").is_err());
    }
}
