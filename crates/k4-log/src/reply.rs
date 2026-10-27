//! What a send's result means for its queue entry (FR-LOG-04; design §3).
//!
//! Classified by Wavelog's `error.code`, not the HTTP status alone — a 403 is a missing scope or a
//! station location that is not the token owner's, and only the first is a reason to stop the
//! whole queue. A duplicate counts as "already logged" only when an earlier try **may have reached**
//! the server; otherwise it is a different QSO already in the log, held for the operator — Wavelog's
//! duplicate key is minute + call + band + main mode + location, so it matches QSOs logged by hand
//! or by other programs too.

use serde_json::Value;

/// Whether the entry's last try may have been processed by the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LastAttempt {
    /// Never sent, or the last try provably did not reach the server.
    #[default]
    NotSent,
    /// The server may have processed it (a timeout or reset after sending, a 5xx).
    Unknown,
}

/// A transport failure without an HTTP response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// Name resolution or connection failed: nothing reached the server.
    NotConnected(String),
    /// The server's certificate was refused (permanent until fixed; no click-to-approve).
    Certificate(String),
    /// Timed out or reset after the request was written: it may have been processed.
    AfterSend(String),
}

/// One send's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attempt {
    Response {
        status: u16,
        /// The reply's `Content-Type` is JSON.
        json: bool,
        body: Vec<u8>,
        /// The `Retry-After` header, seconds.
        retry_after: Option<u64>,
        /// The host a 3xx points to.
        redirect_to: Option<String>,
    },
    Failed(Failure),
}

/// Why the whole queue stops until the operator acts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pause {
    /// Token unknown, revoked, expired or missing: a new one is needed.
    Token(String),
    /// The token lacks a scope.
    Scope(String),
    /// No API v2 at this URL (or a wrong URL).
    NoApi,
    /// The URL redirects; redirects are never followed.
    Redirect(String),
    /// A proxy refuses the method.
    Blocked(String),
    Certificate(String),
}

/// What happens to the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Created; Wavelog's id.
    Logged(u64),
    /// A duplicate of this entry's own earlier, possibly successful try: done.
    AlreadyLogged,
    /// Kept aside for the operator, with the reason.
    Held(String),
    /// Kept; the queue stops.
    Paused(Pause),
    /// Kept; try again later (`after` seconds at the earliest, when the server said so).
    Retry { after: Option<u64> },
}

/// Remove markup and the entities Wavelog puts in its messages (`<br>`, `&#039;`).
pub fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&#039;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

/// Whether this result means the request may have been processed — the entry's next
/// [`LastAttempt`]. Any 4xx is a refusal (nothing stored); a 5xx can follow a committed insert.
pub fn may_have_reached(a: &Attempt) -> bool {
    match a {
        Attempt::Response { status, .. } => *status >= 500,
        Attempt::Failed(Failure::AfterSend(_)) => true,
        Attempt::Failed(_) => false,
    }
}

/// The longest a throttled entry waits, whatever the server asks (seconds).
pub const MAX_RETRY_AFTER: u64 = 3600;

/// Classify one send of an entry whose last try was `last`.
pub fn classify(a: &Attempt, last: LastAttempt) -> Outcome {
    let (status, json, body, retry_after, redirect_to) = match a {
        Attempt::Failed(Failure::Certificate(host)) => {
            return Outcome::Paused(Pause::Certificate(host.clone()))
        }
        Attempt::Failed(_) => return Outcome::Retry { after: None },
        Attempt::Response {
            status,
            json,
            body,
            retry_after,
            redirect_to,
        } => (*status, *json, body, *retry_after, redirect_to),
    };
    if (300..400).contains(&status) {
        return Outcome::Paused(Pause::Redirect(redirect_to.clone().unwrap_or_default()));
    }
    let v: Option<Value> = if json {
        serde_json::from_slice(body).ok()
    } else {
        None
    };
    if status == 201 {
        let id = v
            .as_ref()
            .and_then(|v| v["data"]["id"].as_u64())
            .unwrap_or(0);
        return Outcome::Logged(id);
    }
    let err = v.as_ref().map(|v| &v["error"]).filter(|e| e.is_object());
    let Some(err) = err else {
        // No JSON error: a page from a web server or proxy, not from the API.
        return match status {
            404 => Outcome::Paused(Pause::NoApi),
            405 => Outcome::Paused(Pause::Blocked(format!(
                "the server refused the request (HTTP {status})"
            ))),
            429 | 500..=599 => Outcome::Retry {
                after: capped(retry_after),
            },
            _ => Outcome::Held(format!("unexpected reply from the server (HTTP {status})")),
        };
    };
    let code = err["code"].as_str().unwrap_or("");
    let message = plain(err["message"].as_str().unwrap_or(""));
    let details = &err["details"];
    match code {
        "validation_error" => {
            let duplicate = details["duplicate"]
                .as_array()
                .is_some_and(|d| !d.is_empty());
            match (duplicate, last) {
                (true, LastAttempt::Unknown) => Outcome::AlreadyLogged,
                (true, LastAttempt::NotSent) => Outcome::Held(format!(
                    "Wavelog already has this QSO ({message}) — discard it, or change it and send again"
                )),
                (false, _) => Outcome::Held(if message.is_empty() {
                    "Wavelog refused the QSO".into()
                } else {
                    message
                }),
            }
        }
        "invalid_json" => Outcome::Held(format!("Wavelog could not read the request: {message}")),
        "forbidden" => Outcome::Held(format!(
            "the station location is not yours any more ({message}) — pick one and send again"
        )),
        "invalid_token" | "token_expired" | "unauthorized" => {
            Outcome::Paused(Pause::Token(message))
        }
        "insufficient_scope" => Outcome::Paused(Pause::Scope(
            details["required_scope"]
                .as_str()
                .unwrap_or("?")
                .to_string(),
        )),
        "not_found" => Outcome::Held(format!(
            "Wavelog does not know this request ({message}) — please report it"
        )),
        "method_not_allowed" => Outcome::Paused(Pause::Blocked(message)),
        "rate_limited" => Outcome::Retry {
            after: capped(details["retry_after"].as_u64().or(retry_after)),
        },
        _ if status >= 500 => Outcome::Retry {
            after: capped(retry_after),
        },
        _ => Outcome::Held(format!("Wavelog: {code} {message}").trim().to_string()),
    }
}

fn capped(s: Option<u64>) -> Option<u64> {
    s.map(|s| s.min(MAX_RETRY_AFTER))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(status: u16, body: &str) -> Attempt {
        Attempt::Response {
            status,
            json: true,
            body: body.as_bytes().to_vec(),
            retry_after: None,
            redirect_to: None,
        }
    }

    // Replies recorded from a real Wavelog (local 3.2.x and the DARC server), 2026-10-27.
    const CREATED: &str = r#"{"data":{"id":1,"station_id":1,"qso_date":"2026-10-27 07:50:00","mode":"SSB","submode":"USB","freq":"14250000","call":"DL1ABC","band":"20m"},"meta":{"timestamp":"2026-10-27T07:47:03+00:00","resource":"qso","method":"POST"}}"#;
    const DUPLICATE: &str = r#"{"error":{"code":"validation_error","message":"Date\/Time: 2026-10-27 07:50:00 Callsign: DL1ABC Band: 20m Duplicate for N0CALL","details":{"critical":[],"validation":[],"duplicate":["Date\/Time: 2026-10-27 07:50:00 Callsign: DL1ABC Band: 20m Duplicate for N0CALL<br>"]}}}"#;
    const MISSING: &str = r#"{"error":{"code":"validation_error","message":"Missing required field(s): time_on","details":{"missing":["time_on"]}}}"#;
    const BAD_BAND: &str = r#"{"error":{"code":"validation_error","message":"QSO on 2026-10-27 07:51:00: Band &#039;21m&#039; is not in the band configuration. This QSO wasn&#039;t imported.","details":{"critical":[],"validation":["QSO on 2026-10-27 07:51:00: Band &#039;21m&#039; is not in the band configuration. This QSO wasn&#039;t imported.<br>"],"duplicate":[]}}}"#;
    const SCOPE: &str = r#"{"error":{"code":"insufficient_scope","message":"Token is missing the required scope: qso:write","details":{"required_scope":"qso:write"}}}"#;
    const FORBIDDEN: &str = r#"{"error":{"code":"forbidden","message":"station_profile_id does not belong to the API key owner"}}"#;
    const BAD_TOKEN: &str =
        r#"{"error":{"code":"invalid_token","message":"Invalid or revoked API token"}}"#;
    const NO_TOKEN: &str = r#"{"error":{"code":"unauthorized","message":"Missing API token"}}"#;

    /// FR-LOG-04: every reply class of the design's table, from recorded replies — including a
    /// `validation_error` without `details`, and a duplicate that is held unless an earlier try
    /// may have got through.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_every_reply_class() {
        use LastAttempt::*;
        let c = |a: &Attempt| classify(a, NotSent);
        assert_eq!(c(&resp(201, CREATED)), Outcome::Logged(1));
        assert_eq!(
            classify(&resp(400, DUPLICATE), Unknown),
            Outcome::AlreadyLogged
        );
        match classify(&resp(400, DUPLICATE), NotSent) {
            Outcome::Held(r) => {
                assert!(
                    r.contains("already has this QSO") && r.contains("DL1ABC"),
                    "{r}"
                );
                assert!(!r.contains("<br>"), "markup removed: {r}");
            }
            o => panic!("a first-try duplicate is held, not {o:?}"),
        }
        assert_eq!(
            c(&resp(400, MISSING)),
            Outcome::Held("Missing required field(s): time_on".into())
        );
        match c(&resp(400, BAD_BAND)) {
            Outcome::Held(r) => assert!(r.contains("Band '21m'") && !r.contains("&#039;"), "{r}"),
            o => panic!("{o:?}"),
        }
        let no_details = r#"{"error":{"code":"validation_error","message":"Nothing to import"}}"#;
        assert_eq!(
            c(&resp(400, no_details)),
            Outcome::Held("Nothing to import".into())
        );
        let bare = r#"{"error":{"code":"validation_error","details":null}}"#;
        assert_eq!(
            c(&resp(400, bare)),
            Outcome::Held("Wavelog refused the QSO".into())
        );
        assert_eq!(
            c(&resp(403, SCOPE)),
            Outcome::Paused(Pause::Scope("qso:write".into()))
        );
        match c(&resp(403, FORBIDDEN)) {
            Outcome::Held(r) => assert!(
                r.contains("station location"),
                "403 forbidden holds one entry: {r}"
            ),
            o => panic!("{o:?}"),
        }
        assert_eq!(
            c(&resp(401, BAD_TOKEN)),
            Outcome::Paused(Pause::Token("Invalid or revoked API token".into()))
        );
        assert_eq!(
            c(&resp(401, NO_TOKEN)),
            Outcome::Paused(Pause::Token("Missing API token".into()))
        );
        let expired = r#"{"error":{"code":"token_expired","message":"Token expired"}}"#;
        assert_eq!(
            c(&resp(401, expired)),
            Outcome::Paused(Pause::Token("Token expired".into()))
        );
        let not_found = r#"{"error":{"code":"not_found","message":"Unknown resource"}}"#;
        assert!(matches!(c(&resp(404, not_found)), Outcome::Held(r) if r.contains("report")));
        // A Wavelog without API v2: an HTML 404.
        let html = Attempt::Response {
            status: 404,
            json: false,
            body: b"<html><h1>Page Not Found</h1></html>".to_vec(),
            retry_after: None,
            redirect_to: None,
        };
        assert_eq!(c(&html), Outcome::Paused(Pause::NoApi));
        let redirect = Attempt::Response {
            status: 301,
            json: false,
            body: vec![],
            retry_after: None,
            redirect_to: Some("log.elsewhere.example".into()),
        };
        assert_eq!(
            c(&redirect),
            Outcome::Paused(Pause::Redirect("log.elsewhere.example".into()))
        );
        let blocked = r#"{"error":{"code":"method_not_allowed","message":"POST not allowed"}}"#;
        assert_eq!(
            c(&resp(405, blocked)),
            Outcome::Paused(Pause::Blocked("POST not allowed".into()))
        );
        let limited = r#"{"error":{"code":"rate_limited","message":"slow down","details":{"retry_after":120}}}"#;
        assert_eq!(c(&resp(429, limited)), Outcome::Retry { after: Some(120) });
        let greedy =
            r#"{"error":{"code":"rate_limited","message":"x","details":{"retry_after":999999}}}"#;
        assert_eq!(
            c(&resp(429, greedy)),
            Outcome::Retry {
                after: Some(MAX_RETRY_AFTER)
            }
        );
        let internal = r#"{"error":{"code":"internal_error","message":"boom"}}"#;
        assert_eq!(c(&resp(500, internal)), Outcome::Retry { after: None });
        let proxy = Attempt::Response {
            status: 502,
            json: false,
            body: b"Bad Gateway".to_vec(),
            retry_after: Some(30),
            redirect_to: None,
        };
        assert_eq!(c(&proxy), Outcome::Retry { after: Some(30) });
        assert_eq!(
            c(&Attempt::Failed(Failure::Certificate("log.example".into()))),
            Outcome::Paused(Pause::Certificate("log.example".into()))
        );
        assert_eq!(
            c(&Attempt::Failed(Failure::NotConnected("dns".into()))),
            Outcome::Retry { after: None }
        );
        assert_eq!(
            c(&Attempt::Failed(Failure::AfterSend("timeout".into()))),
            Outcome::Retry { after: None }
        );
    }

    /// FR-LOG-04: which results may have reached the server — the basis of the duplicate rule.
    /// A 4xx refusal stores nothing; a 5xx can follow a committed insert; a timeout after sending
    /// may have been processed; a failed connection was not.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_what_may_have_reached_the_server() {
        assert!(!may_have_reached(&resp(400, DUPLICATE)));
        assert!(!may_have_reached(&resp(401, BAD_TOKEN)));
        assert!(may_have_reached(&resp(500, "{}")));
        assert!(may_have_reached(&Attempt::Failed(Failure::AfterSend(
            "reset".into()
        ))));
        assert!(!may_have_reached(&Attempt::Failed(Failure::NotConnected(
            "refused".into()
        ))));
        assert!(!may_have_reached(&Attempt::Failed(Failure::Certificate(
            "x".into()
        ))));
    }

    /// FR-LOG-04: Wavelog's markup and entities do not reach the operator.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_messages_are_plain_text() {
        assert_eq!(
            plain("Band &#039;21m&#039; is bad.<br>"),
            "Band '21m' is bad."
        );
        assert_eq!(plain("a &amp; b <b>c</b>"), "a & b c");
    }
}
