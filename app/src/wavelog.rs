//! QSO logging to Wavelog (FR-LOG; design `docs/concept/wavelog-logging-plan.md` v0.3): the HTTP
//! transport and the logging worker.
//!
//! The worker owns the queue (`k4_log::queue`) and sends one entry at a time, in queue order, while
//! it holds the queue's sender lock; the UI only sends it commands and reads its [`Status`]. Every
//! reply is classified by `k4_log::reply` — this file decides nothing about what a reply means.
//!
//! The token: read (`Secret::expose`) only here — by [`Transport::call`] for the `Authorization`
//! header and by [`save_token`] for the secret store; a structural test holds it to this file.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use k4_config::WavelogStation;
use k4_log::qso::Qso;
use k4_log::queue::{Entry, Queue, SenderLock, State};
use k4_log::reply::{self, Attempt, Failure, LastAttempt, Outcome, Pause};
use k4_log::secret::Secret;
use k4_log::server::Server;
use serde_json::Value;

/// The most a reply body may be (bytes). Wavelog's replies here are a few KB.
const MAX_BODY: u64 = 1024 * 1024;

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// The HTTP transport: bounded per phase, no redirects, the token only in its header.
#[derive(Clone)]
pub struct Transport {
    agent: ureq::Agent,
}

impl Transport {
    /// The transport with the app's timeouts.
    pub fn new() -> Self {
        Self::with_timeout(Duration::from_secs(15))
    }

    /// Each phase bounded by `t`. Per-phase timeouts say *where* a call stopped, which decides
    /// whether it may have reached the server (`reply::may_have_reached`).
    pub fn with_timeout(t: Duration) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_resolve(Some(t))
            .timeout_connect(Some(t))
            .timeout_send_request(Some(t))
            .timeout_send_body(Some(t))
            .timeout_recv_response(Some(t))
            .timeout_recv_body(Some(t))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .build()
            .into();
        Self { agent }
    }

    /// One request. `token`, when given, goes only into the `Authorization` header.
    pub fn call(
        &self,
        method: Method,
        url: &str,
        token: Option<&Secret>,
        body: Option<&str>,
    ) -> Attempt {
        let ua = concat!("K4remote/", env!("CARGO_PKG_VERSION"));
        let auth = token.map(|t| format!("Bearer {}", t.expose()));
        let result = match method {
            Method::Get => {
                let mut r = self
                    .agent
                    .get(url)
                    .header("User-Agent", ua)
                    .header("Accept", "application/json");
                if let Some(a) = &auth {
                    r = r.header("Authorization", a);
                }
                r.call()
            }
            Method::Post => {
                let mut r = self
                    .agent
                    .post(url)
                    .header("User-Agent", ua)
                    .header("Accept", "application/json")
                    .header("Content-Type", "application/json");
                if let Some(a) = &auth {
                    r = r.header("Authorization", a);
                }
                r.send(body.unwrap_or(""))
            }
        };
        match result {
            Ok(mut resp) => {
                let status = resp.status().as_u16();
                let json = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| v.to_ascii_lowercase().contains("json"));
                let retry_after = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse().ok());
                let redirect_to = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .map(host_of);
                match resp.body_mut().with_config().limit(MAX_BODY).read_to_vec() {
                    Ok(body) => Attempt::Response {
                        status,
                        json,
                        body,
                        retry_after,
                        redirect_to,
                    },
                    // The status line came back, so the request was processed.
                    Err(e) => Attempt::Failed(Failure::AfterSend(format!(
                        "could not read the reply: {e}"
                    ))),
                }
            }
            Err(e) => Attempt::Failed(failure(&e, url)),
        }
    }
}

impl Default for Transport {
    fn default() -> Self {
        Self::new()
    }
}

/// The host of a URL (for a redirect message; never followed).
fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split(['/', '?', '#']).next().unwrap_or("").to_string()
}

/// Whether a TLS failure is the certificate being refused (permanent until fixed).
fn is_certificate(e: &rustls::Error) -> bool {
    matches!(
        e,
        rustls::Error::InvalidCertificate(_) | rustls::Error::NoCertificatesPresented
    )
}

/// What a transport error means: nothing reached the server, the certificate was refused, or the
/// request may have been processed.
fn failure(e: &ureq::Error, url: &str) -> Failure {
    use ureq::Timeout as T;
    let host = host_of(url);
    match e {
        ureq::Error::HostNotFound => Failure::NotConnected(format!("{host} not found")),
        ureq::Error::ConnectionFailed => {
            Failure::NotConnected(format!("could not connect to {host}"))
        }
        // Only a timeout while resolving or connecting proves nothing reached the server. ureq 3.3
        // reports `SendRequest` also while waiting for the reply to a request the server has
        // read in full (seen in `fr_log_04_a_resend_after_a_lost_reply_logs_once`), so the send
        // phases count as "may have reached".
        ureq::Error::Timeout(T::Resolve | T::Connect) => {
            Failure::NotConnected(format!("{host} did not answer"))
        }
        ureq::Error::Timeout(_) => Failure::AfterSend(format!("no reply from {host} in time")),
        ureq::Error::Rustls(r) if is_certificate(r) => Failure::Certificate(host),
        ureq::Error::Io(io) => {
            if io
                .get_ref()
                .and_then(|i| i.downcast_ref::<rustls::Error>())
                .is_some_and(is_certificate)
            {
                return Failure::Certificate(host);
            }
            match io.kind() {
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::AddrNotAvailable => {
                    Failure::NotConnected(format!("could not connect to {host}"))
                }
                _ => Failure::AfterSend(format!("connection to {host} broke: {}", io.kind())),
            }
        }
        ureq::Error::Rustls(_) | ureq::Error::Tls(_) => {
            Failure::NotConnected(format!("no secure connection to {host}"))
        }
        other => Failure::AfterSend(format!("{host}: {other}")),
    }
}

/// The secret-store account of the Wavelog token (one account; the URL it belongs to is in the
/// config, and changing the URL deletes the token).
pub const TOKEN_ACCOUNT: &str = "wavelog-token";

/// Store the token — with the transport's header, the only other reader of it.
pub fn save_token(store: &dyn k4_config::SecretStore, token: &Secret) -> Result<(), String> {
    store
        .set(TOKEN_ACCOUNT, token.expose())
        .map_err(|e| e.to_string())
}

/// Remove the stored token.
pub fn forget_token(store: &dyn k4_config::SecretStore) -> Result<(), String> {
    store.delete(TOKEN_ACCOUNT).map_err(|e| e.to_string())
}

/// What the logging needs to send: where, with which token, to which station location.
#[derive(Debug, Clone)]
pub struct Setup {
    pub server: Server,
    pub token: Secret,
    pub station_id: u32,
}

/// Commands from the UI.
#[derive(Debug)]
pub enum Cmd {
    /// Logging configured (or not): a new setup also lifts a pause.
    Configure(Option<Setup>),
    /// Queue a QSO (for the configured server and location).
    Log(Box<Qso>),
    /// Retry now, also after a pause.
    SendNow,
    /// Check the server and token; report on the status.
    Test,
    /// Remove a held entry.
    Discard(String),
    /// Save the held entries as ADIF.
    SaveAdif(PathBuf),
}

/// A held entry as the UI lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldView {
    pub id: String,
    /// "DL1ABC 20m SSB 2026-10-27 0750".
    pub summary: String,
    pub reason: String,
}

/// The result of a Test (FR-LOG-02).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestReport {
    /// Everything needed to log works.
    pub ok: bool,
    /// What was found, in order, for the tab.
    pub lines: Vec<String>,
    /// The path form that answered (`Some(true)` = only under `/index.php`).
    pub index_php: Option<bool>,
    /// Wavelog's station locations, when the token may read them.
    pub locations: Option<Vec<WavelogStation>>,
}

/// What the UI shows (FR-LOG-07).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub configured: bool,
    pub pending: usize,
    pub held: Vec<HeldView>,
    /// Why the queue is paused.
    pub paused: Option<String>,
    /// The last thing that happened ("logged DL1ABC", an error).
    pub last: Option<String>,
    /// Another K4 Remote holds the sender lock.
    pub other_sender: bool,
    pub test: Option<TestReport>,
    /// A Test is running.
    pub testing: bool,
}

pub type StatusHandle = Arc<Mutex<Status>>;

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Back-off after "unreachable": 30 s, doubling, at most 30 min.
const BACKOFF_FIRST: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(30 * 60);

fn pause_text(p: &Pause) -> String {
    match p {
        Pause::Token(m) => format!("the token was refused ({m}) — enter a new one"),
        Pause::Scope(s) => format!("the token lacks the {s} permission — create one with it"),
        Pause::NoApi => {
            "no Wavelog API v2 at this URL (Wavelog 3.1.0 or later is needed) — check the URL"
                .into()
        }
        Pause::Redirect(h) => format!("the URL redirects to {h} — enter that URL instead"),
        Pause::Blocked(m) => format!("the server refused the request: {m}"),
        Pause::Certificate(h) => {
            format!("the certificate of {h} was refused — the server needs a valid certificate")
        }
    }
}

fn summary(q: &Qso) -> String {
    format!(
        "{} {} {} {} {}",
        q.call,
        q.band,
        q.submode.as_deref().unwrap_or(&q.mode),
        q.qso_date,
        &q.time_on[..q.time_on.len().min(4)]
    )
}

/// The logging worker's state, separate from its thread so tests can drive it step by step.
pub struct Worker {
    queue: Queue,
    transport: Transport,
    setup: Option<Setup>,
    status: StatusHandle,
    paused: Option<String>,
    next_try: Instant,
    backoff: Duration,
    lock: Option<SenderLock>,
}

impl Worker {
    pub fn new(queue: Queue, transport: Transport, status: StatusHandle) -> Self {
        Self {
            queue,
            transport,
            setup: None,
            status,
            paused: None,
            next_try: Instant::now(),
            backoff: BACKOFF_FIRST,
            lock: None,
        }
    }

    fn set_last(&self, s: impl Into<String>) {
        if let Ok(mut st) = self.status.lock() {
            st.last = Some(s.into());
        }
    }

    /// Publish counts, held entries and the pause.
    fn publish(&self) {
        let (entries, _) = self.queue.entries().unwrap_or_default();
        let held = entries
            .iter()
            .filter(|e| e.state == State::Held)
            .map(|e| HeldView {
                id: e.id.clone(),
                summary: summary(&e.qso),
                reason: e.reason.clone().unwrap_or_default(),
            })
            .collect();
        if let Ok(mut st) = self.status.lock() {
            st.configured = self.setup.is_some();
            st.pending = entries.iter().filter(|e| e.state == State::Pending).count();
            st.held = held;
            st.paused = self.paused.clone();
            st.other_sender = self.setup.is_some() && self.lock.is_none();
        }
    }

    /// Handle one command.
    pub fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Configure(setup) => {
                self.setup = setup;
                self.paused = None;
                self.next_try = Instant::now();
            }
            Cmd::Log(qso) => match &self.setup {
                Some(s) => match self
                    .queue
                    .add(s.server.base(), s.station_id, *qso, unix_now())
                {
                    Ok(e) => {
                        self.set_last(format!("queued {}", summary(&e.qso)));
                        self.next_try = Instant::now();
                    }
                    Err(e) => self.set_last(e.to_string()),
                },
                None => self.set_last("logging is not set up (Settings → LOGGING)"),
            },
            Cmd::SendNow => {
                self.paused = None;
                self.next_try = Instant::now();
                self.backoff = BACKOFF_FIRST;
            }
            Cmd::Test => self.test(),
            Cmd::Discard(id) => {
                if let Ok((entries, _)) = self.queue.entries() {
                    for e in entries.iter().filter(|e| e.id == id) {
                        let _ = self.queue.remove(e);
                        self.set_last(format!("discarded {}", summary(&e.qso)));
                    }
                }
            }
            Cmd::SaveAdif(path) => {
                let held: Vec<Entry> = self
                    .queue
                    .entries()
                    .map(|(e, _)| e.into_iter().filter(|e| e.state == State::Held).collect())
                    .unwrap_or_default();
                let text = k4_log::adif::to_adif(&held, env!("CARGO_PKG_VERSION"));
                match std::fs::write(&path, text) {
                    Ok(()) => self.set_last(format!(
                        "saved {} held QSOs to {}",
                        held.len(),
                        path.display()
                    )),
                    Err(e) => self.set_last(format!("could not save {}: {e}", path.display())),
                }
            }
        }
        self.publish();
    }

    /// Send the next pending entry if one is due. Returns whether a request was made.
    pub fn step(&mut self) -> bool {
        let Some(setup) = self.setup.clone() else {
            return false;
        };
        if self.lock.is_none() {
            self.lock = self.queue.try_lock_sender().ok().flatten();
        }
        if self.lock.is_none() || self.paused.is_some() || Instant::now() < self.next_try {
            self.publish();
            return false;
        }
        let Ok((entries, _)) = self.queue.entries() else {
            return false;
        };
        let Some(mut entry) = entries.into_iter().find(|e| e.state == State::Pending) else {
            self.publish();
            return false;
        };
        if entry.url != setup.server.base() {
            entry.state = State::Held;
            entry.reason = Some(format!(
                "logged for {}, but logging now goes to {} — save it as ADIF, then discard it",
                entry.url,
                setup.server.base()
            ));
            let _ = self.queue.update(&entry);
            self.publish();
            return false;
        }
        let attempt = self.transport.call(
            Method::Post,
            &setup.server.endpoint("qso"),
            Some(&setup.token),
            Some(&entry.qso.body(entry.station_profile_id)),
        );
        let outcome = reply::classify(&attempt, entry.last_attempt);
        // Once a try may have reached the server, the entry stays "may have reached" until edited.
        if reply::may_have_reached(&attempt) {
            entry.last_attempt = LastAttempt::Unknown;
        }
        entry.attempts += 1;
        let what = summary(&entry.qso);
        match outcome {
            Outcome::Logged(id) => {
                let _ = self.queue.remove(&entry);
                self.set_last(format!("logged {what} (Wavelog #{id})"));
                self.backoff = BACKOFF_FIRST;
            }
            Outcome::AlreadyLogged => {
                let _ = self.queue.remove(&entry);
                self.set_last(format!("{what} was already in the log"));
                self.backoff = BACKOFF_FIRST;
            }
            Outcome::Held(reason) => {
                entry.state = State::Held;
                entry.reason = Some(reason);
                let _ = self.queue.update(&entry);
                self.set_last(format!("{what} held"));
            }
            Outcome::Paused(p) => {
                let _ = self.queue.update(&entry);
                self.paused = Some(pause_text(&p));
            }
            Outcome::Retry { after } => {
                let _ = self.queue.update(&entry);
                let wait = after
                    .map(Duration::from_secs)
                    .unwrap_or(Duration::ZERO)
                    .max(self.backoff);
                self.next_try = Instant::now() + wait;
                self.backoff = (self.backoff * 2).min(BACKOFF_MAX);
                let why = match &attempt {
                    Attempt::Failed(Failure::NotConnected(m) | Failure::AfterSend(m)) => m.clone(),
                    Attempt::Response { status, .. } => format!("HTTP {status}"),
                    Attempt::Failed(Failure::Certificate(h)) => h.clone(),
                };
                self.set_last(format!(
                    "Wavelog not reachable ({why}); next try in {} s",
                    wait.as_secs()
                ));
            }
        }
        self.publish();
        true
    }

    /// The Test (FR-LOG-02): API v2 present (by the public `status`, in either path form), the
    /// token valid with its scopes, owner and expiry, and the station locations.
    fn test(&mut self) {
        let mut r = TestReport::default();
        let Some(setup) = self.setup.clone() else {
            r.lines.push("enter the URL and the token first".into());
            self.finish_test(r);
            return;
        };
        if let Ok(mut st) = self.status.lock() {
            st.testing = true;
        }
        let ok_status = |a: &Attempt| match a {
            Attempt::Response {
                status: 200,
                json: true,
                body,
                ..
            } => serde_json::from_slice::<Value>(body).is_ok_and(|v| v["data"]["status"] == "ok"),
            _ => false,
        };
        let first = self
            .transport
            .call(Method::Get, &setup.server.endpoint("status"), None, None);
        let server = if ok_status(&first) {
            r.index_php = Some(setup.server.index_php());
            setup.server.clone()
        } else {
            let other = setup
                .server
                .clone()
                .with_index_php(!setup.server.index_php());
            let second = self
                .transport
                .call(Method::Get, &other.endpoint("status"), None, None);
            if ok_status(&second) {
                r.index_php = Some(other.index_php());
                other
            } else {
                r.lines
                    .push(match reply::classify(&first, LastAttempt::NotSent) {
                        Outcome::Paused(p) => pause_text(&p),
                        _ => match &first {
                            Attempt::Failed(Failure::NotConnected(m) | Failure::AfterSend(m)) => {
                                format!("not reachable: {m}")
                            }
                            _ => pause_text(&Pause::NoApi),
                        },
                    });
                self.finish_test(r);
                return;
            }
        };
        r.lines
            .push(format!("API v2 answers at {}", server.endpoint("")));
        let tok = self.transport.call(
            Method::Get,
            &server.endpoint("token"),
            Some(&setup.token),
            None,
        );
        let scopes: Vec<String> = match &tok {
            Attempt::Response {
                status: 200, body, ..
            } => {
                let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
                let d = &v["data"];
                let scopes: Vec<String> = d["scopes"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| s.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                r.lines.push(format!(
                    "token \"{}\" of {}, may: {}; expires: {}",
                    d["name"].as_str().unwrap_or("?"),
                    d["owner"].as_str().unwrap_or("?"),
                    scopes.join(", "),
                    d["expires_at"].as_str().unwrap_or("never")
                ));
                scopes
            }
            other => {
                r.lines
                    .push(match reply::classify(other, LastAttempt::NotSent) {
                        Outcome::Paused(p) => pause_text(&p),
                        o => format!("the token check failed: {o:?}"),
                    });
                self.finish_test(r);
                return;
            }
        };
        let can = |s: &str| scopes.iter().any(|x| x == s);
        if !can("qso:write") {
            r.lines
                .push("the token lacks qso:write — it cannot log QSOs".into());
        }
        if !can("lookup:read") {
            r.lines
                .push("no lookup:read — the callsign lookup stays off".into());
        }
        if can("station:read") {
            let st = self.transport.call(
                Method::Get,
                &server.endpoint("station"),
                Some(&setup.token),
                None,
            );
            if let Attempt::Response {
                status: 200, body, ..
            } = &st
            {
                let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
                let list: Vec<WavelogStation> = v["data"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|s| {
                                Some(WavelogStation {
                                    id: u32::try_from(s["id"].as_u64()?).ok()?,
                                    name: s["name"].as_str().unwrap_or("").to_string(),
                                    callsign: s["callsign"].as_str().unwrap_or("").to_string(),
                                    gridsquare: s["gridsquare"].as_str().unwrap_or("").to_string(),
                                    active: s["active"].as_bool().unwrap_or(false),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                r.lines.push(format!("{} station location(s)", list.len()));
                r.locations = Some(list);
            } else {
                r.lines.push("could not read the station locations".into());
            }
        } else {
            r.lines
                .push("no station:read — type the station location id".into());
        }
        r.ok = can("qso:write");
        self.finish_test(r);
    }

    fn finish_test(&mut self, r: TestReport) {
        if let Ok(mut st) = self.status.lock() {
            st.testing = false;
            st.test = Some(r);
        }
    }
}

/// Start the logging worker on its own thread.
pub fn spawn(rx: Receiver<Cmd>, queue: Queue, status: StatusHandle) {
    let _ = thread::Builder::new()
        .name("wavelog".into())
        .spawn(move || {
            let mut w = Worker::new(queue, Transport::new(), status);
            w.publish();
            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(cmd) => w.command(cmd),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                while w.step() {}
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use k4_log::qso::{DigitalMode, Draft};
    use k4_protocol::state::Mode;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    const TOKEN: &str = "wl2_0123456789abcdef0123456789abcdef01234567";
    const CREATED: &str = r#"{"data":{"id":42,"call":"DL1ABC"},"meta":{}}"#;
    const DUPLICATE: &str = r#"{"error":{"code":"validation_error","message":"Date\/Time: 2026-10-27 07:50:00 Callsign: DL1ABC Band: 20m Duplicate for N0CALL","details":{"critical":[],"validation":[],"duplicate":["... Duplicate for N0CALL<br>"]}}}"#;
    const FORBIDDEN: &str = r#"{"error":{"code":"forbidden","message":"station_profile_id does not belong to the API key owner"}}"#;
    const BAD_TOKEN: &str =
        r#"{"error":{"code":"invalid_token","message":"Invalid or revoked API token"}}"#;

    /// What the mock server does with one connection.
    enum Step {
        /// Status, extra headers, JSON body.
        Reply(u16, &'static str, String),
        /// Read the request and never answer (a lost reply).
        Hang,
    }

    /// One request the mock server got.
    #[derive(Debug, Clone)]
    struct Seen {
        line: String,
        auth: Option<String>,
        body: String,
    }

    /// A mock HTTP server answering connections in order from `script`.
    fn mock(script: Vec<Step>) -> (u16, Arc<Mutex<Vec<Seen>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        thread::spawn(move || {
            for step in script {
                let Ok((mut s, _)) = l.accept() else { return };
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let (mut auth, mut len) = (None, 0usize);
                loop {
                    let mut h = String::new();
                    r.read_line(&mut h).unwrap();
                    let t = h.trim_end();
                    if t.is_empty() {
                        break;
                    }
                    let lower = t.to_ascii_lowercase();
                    if lower.starts_with("authorization:") {
                        auth = Some(t["authorization:".len()..].trim().to_string());
                    }
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; len];
                r.read_exact(&mut body).unwrap();
                log.lock().unwrap().push(Seen {
                    line: line.trim_end().to_string(),
                    auth,
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
                match step {
                    Step::Reply(code, extra, json) => {
                        let _ = write!(
                            s,
                            "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{json}",
                            json.len()
                        );
                    }
                    Step::Hang => {
                        // Keep the connection open, unanswered, while the next one is served.
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(1500));
                            drop(s);
                        });
                    }
                }
            }
        });
        (port, seen)
    }

    fn qso(call: &str) -> Qso {
        Qso::from_draft(&Draft {
            call: call.into(),
            rst_sent: "59".into(),
            rst_rcvd: "57".into(),
            name: String::new(),
            comment: String::new(),
            time_on: 1_793_087_400,
            time_off: 1_793_087_460,
            tx_hz: 14_250_000,
            rx_hz: None,
            mode: Mode::Usb,
            digital: DigitalMode::Ft8,
            tx_pwr: None,
            operator: None,
        })
        .unwrap()
    }

    /// A worker for the mock server on `port`, its queue in a scratch directory.
    fn worker(tag: &str, port: u16) -> (Worker, StatusHandle, PathBuf) {
        let dir = std::env::temp_dir().join(format!("k4-wavelog-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let status: StatusHandle = Arc::default();
        let mut w = Worker::new(
            Queue::open(&dir).unwrap(),
            Transport::with_timeout(Duration::from_millis(400)),
            Arc::clone(&status),
        );
        w.command(Cmd::Configure(Some(Setup {
            server: Server::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
            token: Secret::new(TOKEN).unwrap(),
            station_id: 7,
        })));
        (w, status, dir)
    }

    fn snapshot(s: &StatusHandle) -> Status {
        s.lock().unwrap().clone()
    }

    /// FR-LOG-04, FR-LOG-03: a QSO is queued, sent with the token in the Authorization header and
    /// the station location in the body, and removed once Wavelog has it.
    /// trace: FR-LOG-04, FR-LOG-03
    #[test]
    fn fr_log_04_a_qso_is_logged_and_removed() {
        let (port, seen) = mock(vec![Step::Reply(201, "", CREATED.into())]);
        let (mut w, status, dir) = worker("logged", port);
        w.command(Cmd::Log(Box::new(qso("DL1ABC"))));
        assert_eq!(snapshot(&status).pending, 1, "on disk before it is sent");
        assert!(w.step());
        let st = snapshot(&status);
        assert_eq!((st.pending, st.held.len()), (0, 0));
        assert!(
            st.last
                .as_deref()
                .is_some_and(|l| l.contains("logged DL1ABC") && l.contains("#42")),
            "{st:?}"
        );
        let req = &seen.lock().unwrap()[0];
        assert_eq!(req.line, "POST /api/v2/qso HTTP/1.1");
        assert_eq!(
            req.auth.as_deref(),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        let body: Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!(
            (body["call"].as_str(), body["station_profile_id"].as_u64()),
            (Some("DL1ABC"), Some(7))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: a reply lost after sending, then the resend answered "duplicate", is logged
    /// once — the entry is removed as already logged, not held.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_a_resend_after_a_lost_reply_logs_once() {
        let (port, seen) = mock(vec![Step::Hang, Step::Reply(400, "", DUPLICATE.into())]);
        let (mut w, status, dir) = worker("lost", port);
        w.command(Cmd::Log(Box::new(qso("DL1ABC"))));
        assert!(w.step());
        let st = snapshot(&status);
        assert_eq!(st.pending, 1, "kept after the lost reply: {st:?}");
        let (entries, _) = w.queue.entries().unwrap();
        assert_eq!(
            entries[0].last_attempt,
            LastAttempt::Unknown,
            "it may have reached the server"
        );
        w.command(Cmd::SendNow);
        assert!(w.step());
        let st = snapshot(&status);
        assert_eq!((st.pending, st.held.len()), (0, 0), "{st:?}");
        assert!(
            st.last
                .as_deref()
                .is_some_and(|l| l.contains("already in the log")),
            "{st:?}"
        );
        assert_eq!(seen.lock().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: a duplicate on the first try is another QSO already in the log — held for the
    /// operator, never removed as logged.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_a_first_try_duplicate_is_held() {
        let (port, _) = mock(vec![Step::Reply(400, "", DUPLICATE.into())]);
        let (mut w, status, dir) = worker("dup", port);
        w.command(Cmd::Log(Box::new(qso("DL1ABC"))));
        assert!(w.step());
        let st = snapshot(&status);
        assert_eq!((st.pending, st.held.len()), (0, 1), "{st:?}");
        assert!(st.held[0].reason.contains("already has this QSO"), "{st:?}");
        assert!(
            st.held[0]
                .summary
                .starts_with("DL1ABC 20m USB 2026-10-27 0750"),
            "{st:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: a foreign station location holds only that entry; a refused token pauses the
    /// queue — nothing more is sent — until a new setup is saved.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_forbidden_holds_and_a_bad_token_pauses() {
        let (port, seen) = mock(vec![
            Step::Reply(403, "", FORBIDDEN.into()),
            Step::Reply(401, "", BAD_TOKEN.into()),
            Step::Reply(201, "", CREATED.into()),
        ]);
        let (mut w, status, dir) = worker("pause", port);
        w.command(Cmd::Log(Box::new(qso("DL1ABC"))));
        w.command(Cmd::Log(Box::new(qso("DL2BBB"))));
        assert!(w.step());
        assert_eq!(snapshot(&status).held.len(), 1);
        assert!(w.step());
        let st = snapshot(&status);
        assert!(
            st.paused
                .as_deref()
                .is_some_and(|p| p.contains("token was refused")),
            "{st:?}"
        );
        assert!(!w.step(), "paused: nothing is sent");
        assert_eq!(seen.lock().unwrap().len(), 2);
        w.command(Cmd::Configure(Some(Setup {
            server: Server::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
            token: Secret::new(TOKEN).unwrap(),
            station_id: 7,
        })));
        assert!(w.step(), "a new setup lifts the pause");
        assert_eq!(snapshot(&status).pending, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: when nothing answers, the entry is kept, marked as not sent, and the next try
    /// waits for the back-off.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_unreachable_is_retried_later() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let (mut w, status, dir) = worker("down", port);
        w.command(Cmd::Log(Box::new(qso("DL1ABC"))));
        assert!(w.step());
        let st = snapshot(&status);
        assert_eq!(st.pending, 1);
        assert!(
            st.last
                .as_deref()
                .is_some_and(|l| l.contains("not reachable") && l.contains("30 s")),
            "{st:?}"
        );
        assert_eq!(
            w.queue.entries().unwrap().0[0].last_attempt,
            LastAttempt::NotSent
        );
        assert!(!w.step(), "the back-off holds the next try");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: a redirect is never followed; the queue pauses naming where it points.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_a_redirect_pauses() {
        let (port, seen) = mock(vec![Step::Reply(
            302,
            "Location: https://elsewhere.example/api/v2/qso\r\n",
            String::new(),
        )]);
        let (mut w, status, dir) = worker("redirect", port);
        w.command(Cmd::Log(Box::new(qso("DL1ABC"))));
        assert!(w.step());
        let st = snapshot(&status);
        assert!(
            st.paused
                .as_deref()
                .is_some_and(|p| p.contains("elsewhere.example")),
            "{st:?}"
        );
        assert_eq!(seen.lock().unwrap().len(), 1, "not followed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-02: the Test finds API v2 (falling back to the /index.php form), reads the token's
    /// scopes and owner, and the station locations.
    /// trace: FR-LOG-02
    #[test]
    fn fr_log_02_the_test_reports_api_token_and_stations() {
        let token = r#"{"data":{"id":1,"name":"k4","owner":"DC0SK","scopes":["qso:write","station:read","lookup:read"],"expires_at":null}}"#;
        let listing = r#"{"data":[{"id":3,"name":"Home","callsign":"DC0SK","gridsquare":"JO31","active":true},{"id":4,"name":"Portable","callsign":"DC0SK/P","gridsquare":"JO30","active":false}]}"#;
        let (port, seen) = mock(vec![
            Step::Reply(
                404,
                "",
                r#"{"error":{"code":"not_found","message":"x"}}"#.into(),
            ),
            Step::Reply(
                200,
                "",
                r#"{"data":{"name":"Wavelog API","status":"ok"}}"#.into(),
            ),
            Step::Reply(200, "", token.into()),
            Step::Reply(200, "", listing.into()),
        ]);
        let (mut w, status, dir) = worker("test", port);
        w.command(Cmd::Test);
        let r = snapshot(&status).test.expect("a report");
        assert!(r.ok, "{r:?}");
        assert_eq!(r.index_php, Some(true), "only the /index.php form answered");
        assert!(
            r.lines
                .iter()
                .any(|l| l.contains("of DC0SK") && l.contains("qso:write")),
            "{r:?}"
        );
        let st = r.locations.unwrap();
        assert_eq!((st.len(), st[0].id, st[0].active), (2, 3, true));
        let lines: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.line.clone())
            .collect();
        assert_eq!(
            lines,
            [
                "GET /api/v2/status HTTP/1.1",
                "GET /index.php/api/v2/status HTTP/1.1",
                "GET /index.php/api/v2/token HTTP/1.1",
                "GET /index.php/api/v2/station HTTP/1.1"
            ]
        );
        assert!(
            seen.lock().unwrap()[0].auth.is_none(),
            "status is asked without the token"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: a QSO queued for one server is never sent to another — after the URL changes it
    /// is held (to be saved as ADIF), and nothing reaches the new server for it.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_an_entry_for_another_server_is_held() {
        let (port, seen) = mock(vec![Step::Reply(201, "", CREATED.into())]);
        let (mut w, status, dir) = worker("moved", port);
        w.command(Cmd::Log(Box::new(qso("DL1ABC"))));
        // Logging now goes to another server (its own token): the queued entry must not follow.
        w.command(Cmd::Configure(Some(Setup {
            server: Server::parse(&format!("http://localhost:{port}")).unwrap(),
            token: Secret::new(TOKEN).unwrap(),
            station_id: 7,
        })));
        assert!(!w.step());
        let st = snapshot(&status);
        assert_eq!((st.pending, st.held.len()), (0, 1), "{st:?}");
        assert!(
            st.held[0].reason.contains("logged for http://127.0.0.1"),
            "{st:?}"
        );
        assert!(seen.lock().unwrap().is_empty(), "nothing was sent");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: only one K4 Remote sends from a queue; the other says so and only adds.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_one_sender_per_queue() {
        let (port, _) = mock(vec![Step::Reply(201, "", CREATED.into())]);
        let (mut first, _, dir) = worker("owner", port);
        first.command(Cmd::Log(Box::new(qso("DL1ABC"))));
        let status2: StatusHandle = Arc::default();
        let mut second = Worker::new(
            Queue::open(&dir).unwrap(),
            Transport::new(),
            Arc::clone(&status2),
        );
        // The first takes the lock with its first step.
        assert!(first.step());
        second.command(Cmd::Configure(Some(Setup {
            server: Server::parse(&format!("http://127.0.0.1:{port}")).unwrap(),
            token: Secret::new(TOKEN).unwrap(),
            station_id: 7,
        })));
        second.command(Cmd::Log(Box::new(qso("DL2BBB"))));
        assert!(!second.step());
        assert!(
            snapshot(&status2).other_sender,
            "the second instance says another one sends"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-03: after a session of every kind of reply, the token appears in no status text,
    /// held reason or message — only in the Authorization header the server received.
    /// trace: FR-LOG-03
    #[test]
    fn fr_log_03_the_token_reaches_only_the_header() {
        let (port, seen) = mock(vec![
            Step::Reply(201, "", CREATED.into()),
            Step::Reply(400, "", DUPLICATE.into()),
            Step::Reply(403, "", FORBIDDEN.into()),
            Step::Reply(401, "", BAD_TOKEN.into()),
        ]);
        let (mut w, status, dir) = worker("leak", port);
        for c in ["DL1AAA", "DL1BBB", "DL1CCC", "DL1DDD"] {
            w.command(Cmd::Log(Box::new(qso(c))));
        }
        while w.step() {}
        w.command(Cmd::Test);
        let text = format!("{:?} {:#?}", snapshot(&status), w.setup);
        assert!(
            !text.contains(TOKEN) && !text.contains("0123456789abcdef"),
            "{text}"
        );
        let (entries, _) = w.queue.entries().unwrap();
        let files: String = entries
            .iter()
            .map(|e| serde_json::to_string(e).unwrap())
            .collect();
        assert!(!files.contains("wl2_"), "no token in the queue files");
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .all(|s| s.auth.as_deref() == Some(format!("Bearer {TOKEN}").as_str())));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// FR-LOG-03: the token is read in one file only — for the transport's Authorization header
    /// and the secret store. Structural, over the app and crate sources' production code.
    /// trace: FR-LOG-03
    #[test]
    fn fr_log_03_the_token_is_read_only_by_the_transport() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let needle = concat!(".exp", "ose()");
        let mut found = Vec::new();
        let mut dirs = vec![root.join("app/src"), root.join("crates")];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                if p.is_dir() {
                    if name != "target" {
                        dirs.push(p);
                    }
                } else if name.ends_with(".rs") {
                    let text = std::fs::read_to_string(&p).unwrap();
                    // Production code only: everything before the file's test module.
                    let prod = text
                        .find("#[cfg(test)]")
                        .map_or(text.as_str(), |i| &text[..i]);
                    for (i, _) in prod.match_indices(needle) {
                        let line = prod[..i].lines().count();
                        found.push(format!(
                            "{}:{line}",
                            p.strip_prefix(&root).unwrap().display()
                        ));
                    }
                }
            }
        }
        // Two readers, both here: the transport's Authorization header and the secret store.
        assert_eq!(found.len(), 2, "expose() elsewhere: {found:?}");
        assert!(
            found.iter().all(|f| f.starts_with("app/src/wavelog.rs:")),
            "{found:?}"
        );
    }
}

/// Against a real local Wavelog (`tools/wavelog-test/setup.sh`): run with
/// `K4_WAVELOG_URL=http://127.0.0.1:8086 K4_WAVELOG_TOKEN=wl2_… cargo test -p k4remote --bin
/// k4remote -- --ignored local_wavelog`. Refuses any host but this computer, so it can never write
/// to a real logbook.
#[cfg(test)]
mod local_wavelog {
    use super::*;
    use k4_log::qso::{DigitalMode, Draft};
    use k4_protocol::state::Mode;

    fn setup() -> (Server, Secret) {
        let url = std::env::var("K4_WAVELOG_URL").expect("K4_WAVELOG_URL");
        let token = std::env::var("K4_WAVELOG_TOKEN").expect("K4_WAVELOG_TOKEN");
        let server = Server::parse(&url).expect("a URL");
        let host = server.base().split("://").nth(1).unwrap_or("");
        assert!(
            ["127.0.0.1", "localhost", "[::1]"]
                .iter()
                .any(|h| host.starts_with(h)),
            "refusing to write to {host}: only a local test instance"
        );
        (server, Secret::new(&token).expect("a v2 token"))
    }

    /// FR-LOG-01, FR-LOG-02, FR-LOG-04, FR-LOG-08 against a real Wavelog: the Test finds the
    /// station; a QSO is logged through the worker and reads back with Wavelog's own mode/submode,
    /// Hz, seconds and power; the same QSO again is a first-try duplicate, held.
    /// trace: FR-LOG-01, FR-LOG-02, FR-LOG-04, FR-LOG-08
    #[test]
    #[ignore = "needs a local Wavelog: tools/wavelog-test/setup.sh"]
    fn local_wavelog_logs_reads_back_and_holds_a_duplicate() {
        let (server, token) = setup();
        let dir = std::env::temp_dir().join(format!("k4-wavelog-local-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let status: StatusHandle = Arc::default();
        let mut w = Worker::new(
            Queue::open(&dir).unwrap(),
            Transport::new(),
            Arc::clone(&status),
        );
        w.command(Cmd::Configure(Some(Setup {
            server: server.clone(),
            token: token.clone(),
            station_id: 1,
        })));
        w.command(Cmd::Test);
        let report = status.lock().unwrap().test.clone().unwrap();
        assert!(report.ok, "{report:?}");
        assert!(
            report
                .locations
                .as_ref()
                .is_some_and(|l| l.iter().any(|s| s.id == 1)),
            "{report:?}"
        );

        // A call no earlier run used, and a start time with 17 seconds — so "seconds are kept"
        // cannot pass on a whole minute by chance.
        let now = unix_now();
        let call = format!("DL{}ZZ", now % 9 + 1);
        let time_on = now - 120 - now % 60 + 17;
        let q = Qso::from_draft(&Draft {
            call: call.clone(),
            rst_sent: String::new(),
            rst_rcvd: String::new(),
            name: "K4 Remote test".into(),
            comment: format!("local test {now}"),
            time_on,
            time_off: time_on + 30,
            tx_hz: 14_081_500,
            rx_hz: None,
            mode: Mode::Data,
            digital: DigitalMode::Ft4,
            tx_pwr: Some(5.0),
            operator: None,
        })
        .unwrap();
        w.command(Cmd::Log(Box::new(q.clone())));
        while w.step() {}
        let st = status.lock().unwrap().clone();
        assert!(
            st.last.as_deref().is_some_and(|l| l.contains("logged")),
            "{st:?}"
        );
        assert_eq!((st.pending, st.held.len()), (0, 0), "{st:?}");

        // Read it back as Wavelog stored it — the ADIF export carries every field (the JSON list
        // leaves out tx_pwr).
        let back = Transport::new().call(
            Method::Get,
            &server.endpoint(&format!("qso?callsign={call}&format=adif")),
            Some(&token),
            None,
        );
        let Attempt::Response {
            status: 200, body, ..
        } = back
        else {
            panic!("could not read the QSO back: {back:?}")
        };
        let v: Value = serde_json::from_slice(&body).unwrap();
        let adif = v["data"]["adif"]
            .as_str()
            .unwrap_or("")
            .to_ascii_uppercase();
        let (_, hhmmss) = k4_log::qso::utc_date_time(time_on);
        assert!(hhmmss.ends_with("17"), "{hhmmss}");
        for want in [
            "<MODE:4>MFSK".to_string(),
            "<SUBMODE:3>FT4".to_string(),
            "<BAND:3>20M".to_string(),
            "<TX_PWR:1>5".to_string(),
            format!("<TIME_ON:6>{hhmmss}"),
        ] {
            assert!(adif.contains(&want), "{want} not in {adif}");
        }
        assert!(
            adif.contains("<FREQ:") && adif.contains(">14.0815"),
            "{adif}"
        );

        // The same QSO again, first try: another QSO already in the log — held.
        w.command(Cmd::Log(Box::new(q)));
        while w.step() {}
        let st = status.lock().unwrap().clone();
        assert_eq!((st.pending, st.held.len()), (0, 1), "{st:?}");
        assert!(st.held[0].reason.contains("already has this QSO"), "{st:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
