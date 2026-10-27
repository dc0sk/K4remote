//! The Wavelog server's URL (FR-LOG-02, FR-LOG-03): normalised once, HTTPS only except to this
//! computer, and the two path forms Wavelog answers on.
//!
//! The operator may type `https://log.example.org`, `…/`, `…/index.php` or `…/api/v2` (the DARC
//! documents the last form): all are one server. The token is tied to the normalised base, and the
//! base is what the config keeps.

/// A normalised Wavelog server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    /// `scheme://host[:port][/path]`, scheme and host lower-case, without a trailing `/`,
    /// `/index.php` or `/api/v2`.
    base: String,
    /// The API answers only under `/index.php/api/v2` (decided by the `status` check).
    index_php: bool,
}

/// Why a typed URL is not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    /// Not `http://` or `https://`.
    Scheme,
    NoHost,
    /// Plain HTTP to another computer: the token would travel in clear text.
    PlainHttp(String),
    /// Characters a URL here does not have (spaces, a query, a fragment, credentials).
    Malformed,
}

impl std::fmt::Display for UrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("no URL entered"),
            Self::Scheme => f.write_str("the URL must start with https://"),
            Self::NoHost => f.write_str("the URL has no host"),
            Self::PlainHttp(h) => write!(
                f,
                "plain http:// would send the token unencrypted to {h}; use https:// \
                 (http:// is allowed only to this computer)"
            ),
            Self::Malformed => {
                f.write_str("the URL contains characters it should not (spaces, ?, #, user@)")
            }
        }
    }
}

/// Hosts that are this computer: the only ones allowed over plain HTTP (the local test instance).
fn is_loopback(host: &str) -> bool {
    let h = host.trim_end_matches('.');
    h == "localhost" || h == "127.0.0.1" || h == "[::1]"
}

impl Server {
    /// Normalise a typed URL.
    pub fn parse(typed: &str) -> Result<Self, UrlError> {
        let t = typed.trim();
        if t.is_empty() {
            return Err(UrlError::Empty);
        }
        if t.chars().any(|c| c.is_whitespace() || c == '?' || c == '#') {
            return Err(UrlError::Malformed);
        }
        let lower = t.to_ascii_lowercase();
        let (scheme, rest) = if let Some(r) = lower.strip_prefix("https://") {
            ("https", &t[t.len() - r.len()..])
        } else if let Some(r) = lower.strip_prefix("http://") {
            ("http", &t[t.len() - r.len()..])
        } else {
            return Err(UrlError::Scheme);
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        if authority.contains('@') {
            return Err(UrlError::Malformed);
        }
        let authority = authority.to_ascii_lowercase();
        let host = if authority.starts_with('[') {
            authority
                .split_inclusive(']')
                .next()
                .unwrap_or("")
                .to_string()
        } else {
            authority.split(':').next().unwrap_or("").to_string()
        };
        if host.is_empty() || host == "[]" {
            return Err(UrlError::NoHost);
        }
        if scheme == "http" && !is_loopback(&host) {
            return Err(UrlError::PlainHttp(host));
        }
        let mut path = path.trim_end_matches('/').to_string();
        for suffix in ["/api/v2", "/index.php"] {
            if path.to_ascii_lowercase().ends_with(suffix) {
                path.truncate(path.len() - suffix.len());
                path = path.trim_end_matches('/').to_string();
            }
        }
        Ok(Self {
            base: format!("{scheme}://{authority}{path}"),
            index_php: false,
        })
    }

    /// The normalised base, as the config keeps it.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// The same server, using the `/index.php/api/v2` form (or not).
    pub fn with_index_php(mut self, on: bool) -> Self {
        self.index_php = on;
        self
    }

    pub fn index_php(&self) -> bool {
        self.index_php
    }

    /// The URL of an API v2 resource, e.g. `qso` or `lookup?callsign=DL1ABC`.
    pub fn endpoint(&self, resource: &str) -> String {
        let prefix = if self.index_php { "/index.php" } else { "" };
        format!("{}{prefix}/api/v2/{resource}", self.base)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-LOG-02: the forms of one server normalise to one base — DC0SK's `…/api/v2` included —
    /// and the API paths are built from it in both forms.
    /// trace: FR-LOG-02
    #[test]
    fn fr_log_02_url_forms_normalise_to_one_server() {
        for typed in [
            "https://log.dclnext.darc.de/api/v2",
            "https://log.dclnext.darc.de/api/v2/",
            "https://log.dclnext.darc.de",
            "https://log.dclnext.darc.de/",
            "https://log.dclnext.darc.de/index.php",
            "https://log.dclnext.darc.de/index.php/api/v2",
            "  HTTPS://Log.DCLNext.DARC.de/API/V2  ",
        ] {
            assert_eq!(
                Server::parse(typed).unwrap().base(),
                "https://log.dclnext.darc.de",
                "{typed:?}"
            );
        }
        let s = Server::parse("https://log.example.org/wavelog/").unwrap();
        assert_eq!(
            s.base(),
            "https://log.example.org/wavelog",
            "a sub-path is kept"
        );
        assert_eq!(
            s.endpoint("qso"),
            "https://log.example.org/wavelog/api/v2/qso"
        );
        assert_eq!(
            s.clone().with_index_php(true).endpoint("status"),
            "https://log.example.org/wavelog/index.php/api/v2/status"
        );
        assert_eq!(
            Server::parse("https://log.example.org:8443")
                .unwrap()
                .endpoint("token"),
            "https://log.example.org:8443/api/v2/token"
        );
    }

    /// FR-LOG-03: plain HTTP only to this computer (the test instance); everything else is
    /// HTTPS. Bad input is refused with the reason.
    /// trace: FR-LOG-03
    #[test]
    fn fr_log_03_https_except_to_this_computer() {
        for ok in [
            "http://127.0.0.1:8086/",
            "http://localhost:8086",
            "http://localhost.:8086",
            "http://[::1]:8086/index.php",
        ] {
            assert!(Server::parse(ok).is_ok(), "{ok}");
        }
        assert_eq!(
            Server::parse("http://log.dclnext.darc.de/api/v2"),
            Err(UrlError::PlainHttp("log.dclnext.darc.de".into()))
        );
        assert_eq!(
            Server::parse("http://127.0.0.2/"),
            Err(UrlError::PlainHttp("127.0.0.2".into())),
            "only the loopback address itself"
        );
        assert_eq!(Server::parse(""), Err(UrlError::Empty));
        assert_eq!(Server::parse("log.example.org"), Err(UrlError::Scheme));
        assert_eq!(
            Server::parse("ftp://log.example.org"),
            Err(UrlError::Scheme)
        );
        assert_eq!(Server::parse("https:///api/v2"), Err(UrlError::NoHost));
        assert_eq!(
            Server::parse("https://u:p@log.example.org"),
            Err(UrlError::Malformed)
        );
        assert_eq!(
            Server::parse("https://log.example.org/?x=1"),
            Err(UrlError::Malformed)
        );
    }
}
