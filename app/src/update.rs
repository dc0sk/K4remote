//! Release-update check against the project's GitHub releases (FR-UI-UPD-01).
//!
//! Deliberately **operator-initiated only**: the check runs when the About box's
//! button is pressed and never on a timer or at start-up. A radio-control
//! application should not make unannounced outbound connections, and a remote
//! station may be on a metered or firewalled link.
//!
//! The version comparison is pure and unit-tested here; the HTTPS request
//! itself is demonstrated (L4).

/// Where the check asks, and where the operator is sent.
pub const RELEASES_API: &str = "https://api.github.com/repos/dc0sk/K4remote/releases/latest";
/// Fallback landing page if the API reply carries no URL.
pub const RELEASES_URL: &str = "https://github.com/dc0sk/K4remote/releases/latest";

/// Outcome of a check, as shown in the About box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    /// No check has been run in this session.
    Idle,
    /// A request is in flight.
    Checking,
    /// The running build is the newest release.
    UpToDate,
    /// A newer release exists: its version and the page to download it from.
    Available { version: String, url: String },
    /// The check could not be completed (offline, rate-limited, malformed).
    Failed(String),
}

/// Parse a release tag into `(major, minor, patch)`.
///
/// Accepts an optional `v` prefix (`v0.2.3` and `0.2.3` alike) and tolerates a
/// missing patch (`v1.2` → `1.2.0`). Any pre-release or build suffix
/// (`-rc1`, `+build`) is ignored for ordering — GitHub's "latest" release
/// excludes pre-releases, so they should not normally appear.
///
/// trace: FR-UI-UPD-01
pub fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let t = tag.trim();
    let t = t
        .strip_prefix('v')
        .or_else(|| t.strip_prefix('V'))
        .unwrap_or(t);
    // Drop any pre-release / build metadata.
    let core = t.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.trim().parse().ok()?;
    let minor = parts.next().unwrap_or("0").trim().parse().ok()?;
    let patch = parts.next().unwrap_or("0").trim().parse().ok()?;
    // Reject trailing junk like "1.2.3.4".
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether `latest` is a strictly newer release than `current`.
///
/// Compares numerically, component by component — a lexical comparison would
/// rank `0.9.0` above `0.10.0`. Returns `false` when either version cannot be
/// parsed, so an unrecognised tag never nags the operator to "upgrade".
///
/// trace: FR-UI-UPD-01
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(cur), Some(new)) => new > cur,
        _ => false,
    }
}

/// Pull `tag_name` and `html_url` out of a GitHub "latest release" reply.
///
/// A hand-rolled scan rather than a JSON dependency: the two fields are flat
/// strings in a known document, and this keeps the update check from adding a
/// serialisation stack to the app. Returns `None` if the tag is absent, which
/// is treated as a failed check rather than "up to date" — silently reporting
/// success on a malformed reply would be the wrong failure direction.
///
/// trace: FR-UI-UPD-01
pub fn parse_release(json: &str) -> Option<(String, String)> {
    let tag = json_string_field(json, "tag_name")?;
    let url = json_string_field(json, "html_url").unwrap_or_else(|| RELEASES_URL.to_string());
    Some((tag, url))
}

/// Extract `"<field>": "<value>"` from flat JSON, honouring backslash escapes
/// so an escaped quote inside a value cannot terminate it early.
fn json_string_field(json: &str, field: &str) -> Option<String> {
    let key = format!("\"{field}\"");
    let start = json.find(&key)? + key.len();
    let rest = json.get(start..)?;
    // Skip whitespace and the colon.
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;

    let mut out = String::new();
    let mut escaped = false;
    for c in rest.chars() {
        if escaped {
            // Only the escapes that can occur in a tag or URL.
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None // unterminated string
}

/// How long the whole check may take before it is reported as failed. A
/// remote station may be on a slow or half-open link, and the About box must
/// not sit on "Checking…" indefinitely.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Ask GitHub for the latest release: `(tag, download page URL)`.
///
/// Blocking — call it off the UI thread. Errors are returned as short operator-
/// facing strings; there is nothing actionable in a transport error beyond
/// "it did not work", and the diagnostics console carries the detail.
pub fn fetch_latest() -> Result<(String, String), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .build()
        .into();
    // GitHub rejects requests without a User-Agent.
    let mut resp = agent
        .get(RELEASES_API)
        .header(
            "User-Agent",
            concat!("K4remote/", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(403) => {
                "GitHub rate limit reached — try again later".to_string()
            }
            ureq::Error::StatusCode(c) => format!("GitHub returned HTTP {c}"),
            other => format!("could not reach GitHub: {other}"),
        })?;
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("could not read the reply: {e}"))?;
    parse_release(&body).ok_or_else(|| "unexpected reply from GitHub".to_string())
}

/// Run a check and classify the result against the running build.
///
/// Blocking; see [`fetch_latest`].
pub fn check_now(current: &str) -> UpdateStatus {
    match fetch_latest() {
        Ok((tag, url)) => {
            if is_newer(current, &tag) {
                UpdateStatus::Available { version: tag, url }
            } else {
                UpdateStatus::UpToDate
            }
        }
        Err(e) => UpdateStatus::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tags parse with or without the `v`, tolerate a missing patch, and reject
    /// junk rather than guessing.
    /// trace: FR-UI-UPD-01
    #[test]
    fn fr_ui_upd_01_parse_version() {
        assert_eq!(parse_version("v0.2.3"), Some((0, 2, 3)));
        assert_eq!(parse_version("0.2.3"), Some((0, 2, 3)));
        assert_eq!(parse_version("V1.0.0"), Some((1, 0, 0)));
        assert_eq!(parse_version(" v2.10.7 "), Some((2, 10, 7)));
        assert_eq!(parse_version("v1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("v3"), Some((3, 0, 0)));
        // Pre-release / build metadata is ignored for ordering.
        assert_eq!(parse_version("v0.3.0-rc1"), Some((0, 3, 0)));
        assert_eq!(parse_version("0.3.0+build7"), Some((0, 3, 0)));
        // Junk yields nothing.
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("nightly"), None);
        assert_eq!(parse_version("v1.2.3.4"), None);
        assert_eq!(parse_version("v1.x.3"), None);
    }

    /// Newer-ness is numeric, not lexical: `0.10.0` is newer than `0.9.0`,
    /// which a string comparison gets backwards.
    /// trace: FR-UI-UPD-01
    #[test]
    fn fr_ui_upd_01_is_newer_compares_numerically() {
        assert!(is_newer("0.9.0", "0.10.0"), "0.10.0 > 0.9.0");
        assert!(!is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("v0.2.3", "v0.2.4"));
        assert!(is_newer("v0.2.3", "v0.3.0"));
        assert!(is_newer("v0.2.3", "v1.0.0"));
        // Equal is not newer — no nagging an operator already up to date.
        assert!(!is_newer("v0.2.3", "v0.2.3"));
        assert!(
            !is_newer("0.2.3", "v0.2.3"),
            "the `v` prefix is not a change"
        );
        // Older upstream (a yanked release, a rollback) is not an update.
        assert!(!is_newer("v0.2.4", "v0.2.3"));
    }

    /// An unparseable version never reports an update: the failure direction
    /// must be "say nothing", not "tell the operator to upgrade".
    /// trace: FR-UI-UPD-01
    #[test]
    fn fr_ui_upd_01_unparseable_never_claims_an_update() {
        assert!(!is_newer("v0.2.3", "not-a-version"));
        assert!(!is_newer("not-a-version", "v9.9.9"));
        assert!(!is_newer("", ""));
    }

    /// The release reply yields the tag and the page to send the operator to.
    /// trace: FR-UI-UPD-01
    #[test]
    fn fr_ui_upd_01_parse_release() {
        let json = r#"{"url":"https://api.github.com/x","html_url":"https://github.com/dc0sk/K4remote/releases/tag/v0.3.0","id":42,"tag_name":"v0.3.0","name":"0.3.0"}"#;
        let (tag, url) = parse_release(json).expect("parses");
        assert_eq!(tag, "v0.3.0");
        assert_eq!(url, "https://github.com/dc0sk/K4remote/releases/tag/v0.3.0");
    }

    /// A reply without a tag is a *failed* check, not "up to date".
    /// trace: FR-UI-UPD-01
    #[test]
    fn fr_ui_upd_01_missing_tag_is_not_success() {
        assert_eq!(parse_release(r#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_release(""), None);
        assert_eq!(parse_release("{}"), None);
        // Rate-limit replies carry no tag either.
        assert_eq!(
            parse_release(
                r#"{"message":"API rate limit exceeded","documentation_url":"https://docs.github.com"}"#
            ),
            None
        );
    }

    /// Escapes inside a value must not terminate it early, and a missing
    /// `html_url` falls back to the releases page rather than an empty link.
    /// trace: FR-UI-UPD-01
    #[test]
    fn fr_ui_upd_01_parse_release_edge_cases() {
        // Escaped quote inside the URL value.
        let json = r#"{"tag_name":"v1.0.0","html_url":"https://example.com/a\"b"}"#;
        let (tag, url) = parse_release(json).unwrap();
        assert_eq!(tag, "v1.0.0");
        assert_eq!(url, "https://example.com/a\"b");

        // No html_url → fall back to the releases page.
        let (_, url) = parse_release(r#"{"tag_name":"v1.0.0"}"#).unwrap();
        assert_eq!(url, RELEASES_URL);

        // Unterminated string is a failure, not a truncated value.
        assert_eq!(parse_release(r#"{"tag_name":"v1.0.0"#), None);

        // Whitespace around the colon is tolerated.
        let (tag, _) = parse_release("{\"tag_name\"  :  \"v2.0.0\"}").unwrap();
        assert_eq!(tag, "v2.0.0");
    }
}
