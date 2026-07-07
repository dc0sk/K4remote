//! K4 settings snapshot export/import (FR-CFG-06): a `.cfg` file holding the
//! radio's current settings as replayable CAT commands, guarded by a SHA-256
//! integrity hash over the command block.

use sha2::{Digest, Sha256};

/// A settings snapshot: the K4 serial, an export timestamp, and the CAT command
/// lines (each a `RESP`/`SET`-format string, terminated with `;`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub serial: String,
    pub timestamp: String,
    pub commands: Vec<String>,
}

/// Lowercase-hex SHA-256 of `s`.
pub fn sha256_hex(s: &str) -> String {
    Sha256::digest(s.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Canonical command block used for hashing: commands joined by newlines.
fn body(commands: &[String]) -> String {
    commands.join("\n")
}

/// Serialise a snapshot to `.cfg` text: a metadata header (including the
/// SHA-256 of the command block) followed by one command per line.
pub fn export(snap: &Snapshot) -> String {
    let body = body(&snap.commands);
    format!(
        "# K4 Remote settings export\nserial = {}\nexported = {}\nsha256 = {}\n\n{}\n",
        snap.serial,
        snap.timestamp,
        sha256_hex(&body),
        body,
    )
}

/// Parse a `.cfg` back into a snapshot, verifying the SHA-256 over the command
/// block. Returns an error on a hash mismatch (a corrupt/tampered file).
pub fn import(text: &str) -> Result<Snapshot, String> {
    let mut snap = Snapshot::default();
    let mut declared_sha = String::new();
    let mut lines = text.lines();
    for line in lines.by_ref() {
        let line = line.trim();
        if line.is_empty() {
            break; // header ends at the first blank line
        }
        if let Some(v) = line.strip_prefix("serial =") {
            snap.serial = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("exported =") {
            snap.timestamp = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("sha256 =") {
            declared_sha = v.trim().to_string();
        }
    }
    snap.commands = lines
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let actual = sha256_hex(&body(&snap.commands));
    if !declared_sha.is_empty() && actual != declared_sha {
        return Err(format!(
            "SHA-256 mismatch — file may be corrupt or edited (expected {declared_sha}, got {actual})"
        ));
    }
    Ok(snap)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// trace: FR-CFG-06
    #[test]
    fn export_import_round_trip_and_tamper_detection() {
        let snap = Snapshot {
            serial: "12345".into(),
            timestamp: "20260707T120000".into(),
            commands: vec!["FA00014074000;".into(), "MD3;".into()],
        };
        let text = export(&snap);
        let back = import(&text).expect("valid file imports");
        assert_eq!(back.commands, snap.commands);
        assert_eq!(back.serial, "12345");

        // Tampering with a command flips the hash → rejected.
        let tampered = text.replace("MD3;", "MD1;");
        assert!(import(&tampered).is_err());
    }
}
