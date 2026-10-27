//! The Wavelog API token (FR-LOG-03): a type that cannot be printed by accident.
//!
//! `Debug` shows only `wl2_…` and the last four characters; there is no `Display`, `Serialize` or
//! `Clone` into a plain string. The one accessor, [`Secret::expose`], is meant for one place — the
//! HTTP transport setting the `Authorization` header — and the app's structural test holds it there.

/// A Wavelog API v2 token (`wl2_` + 40 hex characters as Wavelog makes them).
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

/// Why a typed token was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    Empty,
    /// An API v1 key or something else: v2 tokens start with `wl2_`.
    NotV2,
    /// Spaces or control characters inside — usually a bad paste.
    Malformed,
}

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Empty => "no token entered",
            Self::NotV2 => {
                "not an API v2 token (they start with wl2_; create one in Wavelog under API)"
            }
            Self::Malformed => {
                "the token contains spaces or other characters a token does not have"
            }
        })
    }
}

impl Secret {
    /// Accept a typed or pasted token, trimmed.
    pub fn new(typed: &str) -> Result<Self, SecretError> {
        let t = typed.trim();
        if t.is_empty() {
            return Err(SecretError::Empty);
        }
        if !t.starts_with("wl2_") || t.len() < 8 {
            return Err(SecretError::NotV2);
        }
        if !t.chars().all(|c| c.is_ascii_graphic()) {
            return Err(SecretError::Malformed);
        }
        Ok(Self(t.to_string()))
    }

    /// The token itself — for the transport's `Authorization` header and the secret store only.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// What may be shown: `wl2_…` and the last four characters.
    pub fn hint(&self) -> String {
        let tail: String = self
            .0
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("wl2_…{tail}")
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret({})", self.hint())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "wl2_0123456789abcdef0123456789abcdef01234567";

    /// FR-LOG-03: a token prints as its hint only — `Debug`, and anything built with it.
    /// trace: FR-LOG-03
    #[test]
    fn fr_log_03_the_token_never_prints() {
        let s = Secret::new(TOKEN).unwrap();
        let debug = format!("{s:?} {:?} {:#?}", Some(&s), vec![s.clone()]);
        assert!(
            !debug.contains(TOKEN) && !debug.contains("0123456789abcdef"),
            "{debug}"
        );
        assert!(debug.contains("wl2_…4567"), "{debug}");
        assert_eq!(s.hint(), "wl2_…4567");
        assert_eq!(s.expose(), TOKEN);
    }

    /// FR-LOG-03: a token is trimmed; a v1 key, an empty field or a bad paste are refused with
    /// the reason.
    /// trace: FR-LOG-03
    #[test]
    fn fr_log_03_typed_tokens() {
        assert_eq!(
            Secret::new(&format!("  {TOKEN}\n")).unwrap().expose(),
            TOKEN
        );
        assert_eq!(Secret::new("   "), Err(SecretError::Empty));
        assert_eq!(Secret::new("cl12345abcdef"), Err(SecretError::NotV2));
        assert_eq!(Secret::new("wl2_ab cd ef"), Err(SecretError::Malformed));
    }
}
