//! CW keying via the K4 remote paddle/key data stream.
//!
//! Source: K4 Programmer's Reference — `KZ` (key data stream), `KZL` (key-down
//! initial delay), `KZF` (key fail-safe timeout). A `KZ` stream is `KZ` followed
//! by a sequence of elements and a `;` terminator:
//!   `.` dit · `-` dah · `Pnnnn` pause · `Unnnn` key-up · `Dnnnn` key-down
//! where `nnnn` is 0–2500 ms.

use std::fmt::Write as _;

/// Maximum ms value for a paddle/key timing element.
const MAX_ELEMENT_MS: u16 = 2500;

/// One element of a `KZ` keying stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyElement {
    /// Paddle dit (`.`).
    Dit,
    /// Paddle dah (`-`).
    Dah,
    /// Paddle element pause, ms (`Pnnnn`).
    Pause(u16),
    /// Key element up, ms (`Unnnn`).
    Up(u16),
    /// Key element down, ms (`Dnnnn`).
    Down(u16),
}

impl KeyElement {
    fn append_to(self, out: &mut String) {
        match self {
            KeyElement::Dit => out.push('.'),
            KeyElement::Dah => out.push('-'),
            KeyElement::Pause(ms) => {
                let _ = write!(out, "P{:04}", ms.min(MAX_ELEMENT_MS));
            }
            KeyElement::Up(ms) => {
                let _ = write!(out, "U{:04}", ms.min(MAX_ELEMENT_MS));
            }
            KeyElement::Down(ms) => {
                let _ = write!(out, "D{:04}", ms.min(MAX_ELEMENT_MS));
            }
        }
    }
}

/// Encode a `KZ` keying stream: `KZ<elements>;`.
///
/// Example: `[Dah, Dit, Dah, Dit]` → `"KZ-.-.;"` (the letter C).
///
/// trace: FR-TX-CW-01
pub fn encode_kz(elements: &[KeyElement]) -> String {
    let mut s = String::from("KZ");
    for e in elements {
        e.append_to(&mut s);
    }
    s.push(';');
    s
}

/// Encode `KZL` — key-down initial delay, 0–1000 ms (default 80).
///
/// trace: FR-TX-CW-02
pub fn encode_kzl(ms: u16) -> String {
    format!("KZL{:04};", ms.min(1000))
}

/// Encode `KZF` — key fail-safe timeout, 1–10 minutes (default 3).
///
/// trace: FR-TX-SAFE-02
pub fn encode_kzf(minutes: u8) -> String {
    format!("KZF{:02};", minutes.clamp(1, 10))
}
