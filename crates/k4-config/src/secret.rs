//! Secret storage abstraction for the connection password (FR-CFG-03).
//!
//! The password is never written to the TOML config; if the operator opts in,
//! it is kept in an OS keychain via [`KeyringStore`] (feature `keychain`). The
//! [`SecretStore`] trait keeps the app decoupled from the backend, and
//! [`MemoryStore`] provides a hardware-free, unit-testable implementation.

use std::collections::HashMap;
use std::sync::Mutex;

/// Error from a secret-store backend.
#[derive(Debug)]
pub struct SecretError(pub String);

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "secret store error: {}", self.0)
    }
}
impl std::error::Error for SecretError {}

/// A keyed secret store (account → secret).
pub trait SecretStore: Send + Sync {
    /// Retrieve the secret for `account`, or `None` if absent/unavailable.
    fn get(&self, account: &str) -> Option<String>;
    /// Store `secret` for `account`.
    fn set(&self, account: &str, secret: &str) -> Result<(), SecretError>;
    /// Remove the secret for `account` (no error if absent).
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

/// In-memory store: persists only for the process lifetime. Useful as a default
/// and for tests.
#[derive(Debug, Default)]
pub struct MemoryStore {
    map: Mutex<HashMap<String, String>>,
}

impl MemoryStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Option<String> {
        self.map.lock().ok()?.get(account).cloned()
    }
    fn set(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.map
            .lock()
            .map_err(|e| SecretError(e.to_string()))?
            .insert(account.to_string(), secret.to_string());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.map
            .lock()
            .map_err(|e| SecretError(e.to_string()))?
            .remove(account);
        Ok(())
    }
}

/// OS-keychain store (Secret Service / macOS Keychain / Windows Credential
/// Manager) via the `keyring` crate. Operations degrade gracefully when no
/// keychain service is available.
#[cfg(feature = "keychain")]
pub struct KeyringStore {
    service: String,
}

#[cfg(feature = "keychain")]
impl KeyringStore {
    /// Create a store scoped to `service` (e.g. `"k4remote"`).
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

#[cfg(feature = "keychain")]
impl SecretStore for KeyringStore {
    fn get(&self, account: &str) -> Option<String> {
        keyring::Entry::new(&self.service, account)
            .ok()
            .and_then(|e| e.get_password().ok())
    }
    fn set(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        keyring::Entry::new(&self.service, account)
            .and_then(|e| e.set_password(secret))
            .map_err(|e| SecretError(e.to_string()))
    }
    fn delete(&self, account: &str) -> Result<(), SecretError> {
        let entry =
            keyring::Entry::new(&self.service, account).map_err(|e| SecretError(e.to_string()))?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            // Absent entry is not an error.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(SecretError(e.to_string())),
        }
    }
}
