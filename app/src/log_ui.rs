//! The log form and the LOGGING settings tab (FR-LOG-01, FR-LOG-02, FR-LOG-07; design
//! `docs/concept/wavelog-logging-plan.md` v0.3).
//!
//! The UI only holds what is typed and a copy of the logging worker's status; the worker
//! (`wavelog.rs`) queues, sends and tests. Nothing reaches Wavelog except by the Log button (or
//! Enter in the call field).

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::Duration;

use iced::widget::{pick_list, Column, Row, Text, TextInput};
use iced::{Alignment, Element, Length, Task};
use k4_config::{SecretStore, WavelogPrefs, WavelogStation};
use k4_log::qso::{adif_mode, default_rst, frequencies, power_watts, DigitalMode, Draft, Qso};
use k4_log::queue::Queue;
use k4_log::secret::Secret;
use k4_log::server::Server;
use k4_protocol::state::Mode;

use crate::wavelog::{self, Cmd, Setup, Status, StatusHandle, TestReport, TestTarget};
use crate::{load_secret_timed, role_color, small_btn, ui, App, Message};

/// A station location in the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationChoice(pub WavelogStation);

impl std::fmt::Display for StationChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = &self.0;
        write!(f, "{} — {} {}", s.name, s.callsign, s.gridsquare)?;
        if s.active {
            f.write_str(" (active in Wavelog)")?;
        }
        Ok(())
    }
}

/// A digital mode in the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigitalChoice(pub DigitalMode);

impl std::fmt::Display for DigitalChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.label())
    }
}

/// The logging UI's state.
pub struct LogUi {
    /// What the config keeps.
    pub prefs: WavelogPrefs,
    tx: Option<Sender<Cmd>>,
    status: StatusHandle,
    /// The worker's status as of the last tick.
    pub view: Status,
    url_input: String,
    url_error: Option<String>,
    token_input: String,
    token: Option<Secret>,
    token_note: Option<String>,
    /// A keychain write or removal running on its thread, and what to say when it ends.
    keychain: Option<(std::sync::mpsc::Receiver<Result<(), String>>, String)>,
    call: String,
    rst_sent: String,
    rst_rcvd: String,
    name: String,
    comment: String,
    /// When the call was first typed (Unix seconds).
    time_on: Option<u64>,
    digital: DigitalMode,
    /// Frequency typed when no radio is connected (MHz).
    manual_freq: String,
    form_error: Option<String>,
    /// The last Test report already taken into the config.
    seen_test: Option<TestReport>,
}

#[derive(Debug, Clone)]
pub enum LogMsg {
    Url(String),
    UrlSave,
    Token(String),
    TokenSave,
    TokenForget,
    Operator(String),
    Test,
    Station(StationChoice),
    SendNow,
    Discard(String),
    SaveAdif,
    Call(String),
    RstSent(String),
    RstRcvd(String),
    Name(String),
    Comment(String),
    Digital(DigitalChoice),
    ManualFreq(String),
    Log,
    Clear,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl LogUi {
    /// Load the token (off the UI thread, bounded), open the queue and start the worker.
    pub fn start(
        prefs: WavelogPrefs,
        store: &Arc<dyn SecretStore>,
        config_path: Option<PathBuf>,
    ) -> Self {
        let status: StatusHandle = Arc::default();
        let mut token_note = None;
        let token = if prefs.url.is_empty() {
            None
        } else {
            match load_secret_timed(store, wavelog::TOKEN_ACCOUNT, Duration::from_secs(2)) {
                Some(Ok(Some(t))) => Secret::new(&t).ok(),
                Some(Ok(None)) => None,
                Some(Err(e)) => {
                    token_note = Some(format!("the saved token could not be read: {e}"));
                    None
                }
                None => {
                    token_note = Some("the keychain did not answer in time".into());
                    None
                }
            }
        };
        // The queue lives beside the config file.
        let queue = config_path
            .as_deref()
            .and_then(|p| p.parent())
            .map(|d| d.join("wavelog-queue"))
            .and_then(|d| Queue::open(d).ok());
        let tx = queue.map(|q| {
            let (tx, rx) = mpsc::channel();
            wavelog::spawn(rx, q, Arc::clone(&status));
            tx
        });
        let digital = DigitalMode::ALL
            .into_iter()
            .find(|d| d.label() == prefs.digital_mode)
            .unwrap_or_default();
        let ui = Self {
            url_input: prefs.url.clone(),
            prefs,
            tx,
            status,
            view: Status::default(),
            url_error: None,
            token_input: String::new(),
            token,
            token_note,
            keychain: None,
            call: String::new(),
            rst_sent: String::new(),
            rst_rcvd: String::new(),
            name: String::new(),
            comment: String::new(),
            time_on: None,
            digital,
            manual_freq: String::new(),
            form_error: None,
            seen_test: None,
        };
        ui.reconfigure();
        ui
    }

    fn send(&self, cmd: Cmd) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(cmd);
        }
    }

    /// The setup the worker sends with, if everything is saved.
    fn setup(&self) -> Option<Setup> {
        let server = Server::parse(&self.prefs.url)
            .ok()?
            .with_index_php(self.prefs.index_php);
        Some(Setup {
            server,
            token: self.token.clone()?,
            station_id: self.prefs.station_id?,
        })
    }

    /// Hand the current setup (or none) to the worker.
    fn reconfigure(&self) {
        self.send(Cmd::Configure(self.setup()));
    }

    /// The log form is offered once URL, token and station location are saved.
    pub fn ready(&self) -> bool {
        self.setup().is_some()
    }

    /// The call field changed at `now`: `time_on` is taken when it first gets text and dropped
    /// when it is emptied.
    fn type_call(&mut self, s: String, now: u64) {
        if self.call.trim().is_empty() && !s.trim().is_empty() {
            self.time_on = Some(now);
        }
        if s.trim().is_empty() {
            self.time_on = None;
        }
        self.call = s.to_ascii_uppercase();
        self.form_error = None;
    }
}

/// The mode of the transmit VFO (B in split).
fn tx_mode(r: &k4_protocol::state::RadioState) -> Option<Mode> {
    if r.split == Some(true) {
        r.mode_b.or(r.mode_a)
    } else {
        r.mode_a
    }
}

/// The draft from the form and the radio at `now` — or from the typed frequency when no radio is
/// connected. Empty RST fields take the mode's default.
fn draft_from(
    l: &LogUi,
    r: &k4_protocol::state::RadioState,
    connected: bool,
    now: u64,
) -> Result<Draft, String> {
    let (tx_hz, rx_hz, mode) = match (connected, r.vfo_a_hz) {
        (true, Some(a)) => {
            let (tx, rx) = frequencies(a, r.vfo_b_hz, r.split == Some(true));
            (tx, rx, tx_mode(r).unwrap_or(Mode::Usb))
        }
        _ => {
            let mhz: f64 = l
                .manual_freq
                .trim()
                .replace(',', ".")
                .parse()
                .map_err(|_| "no radio: enter the frequency in MHz".to_string())?;
            ((mhz * 1e6).round() as u64, None, Mode::Usb)
        }
    };
    let (m, sub) = adif_mode(mode, l.digital);
    let rst = default_rst(m, sub);
    let pick = |typed: &str| {
        if typed.trim().is_empty() {
            rst.to_string()
        } else {
            typed.trim().to_string()
        }
    };
    Ok(Draft {
        call: l.call.clone(),
        rst_sent: pick(&l.rst_sent),
        rst_rcvd: pick(&l.rst_rcvd),
        name: l.name.clone(),
        comment: l.comment.clone(),
        time_on: l.time_on.unwrap_or(now).min(now),
        time_off: now,
        tx_hz,
        rx_hz,
        mode,
        digital: l.digital,
        tx_pwr: power_watts(r.tx_power, r.tx_power_range),
        operator: Some(l.prefs.operator.clone()).filter(|o| !o.is_empty()),
    })
}

impl App {
    /// Handle a logging message. The worker is reconfigured only when the setup changes (URL,
    /// token, station location): a `Configure` lifts a pause and resets the back-off, which a
    /// keystroke in the call field must not do.
    pub(crate) fn log_update(&mut self, m: LogMsg) -> Task<Message> {
        let mut save = false;
        let before = self.log.setup().map(|s| (s.server, s.token, s.station_id));
        let l = &mut self.log;
        match m {
            LogMsg::Url(s) => {
                l.url_input = s;
                l.url_error = None;
            }
            LogMsg::UrlSave => match Server::parse(&l.url_input) {
                Ok(server) => {
                    if server.base() != l.prefs.url {
                        // The token belongs to the old URL: it is deleted, never sent elsewhere.
                        if l.token.take().is_some() || !l.prefs.url.is_empty() {
                            l.keychain = Some((
                                wavelog::forget_token_in_background(Arc::clone(&self.secret_store)),
                                "old token removed".into(),
                            ));
                        }
                        l.prefs.url = server.base().to_string();
                        l.prefs.index_php = false;
                        l.prefs.station_id = None;
                        l.prefs.locations.clear();
                        l.token_note = Some("URL saved — enter the token for it".into());
                    }
                    l.url_input = l.prefs.url.clone();
                    save = true;
                }
                Err(e) => l.url_error = Some(e.to_string()),
            },
            LogMsg::Token(s) => l.token_input = s,
            LogMsg::TokenSave => match Secret::new(&l.token_input) {
                Ok(t) if !l.prefs.url.is_empty() => {
                    l.token_note = Some("saving the token in the keychain…".into());
                    l.keychain = Some((
                        wavelog::save_token_in_background(
                            Arc::clone(&self.secret_store),
                            t.clone(),
                        ),
                        format!("token {} saved", t.hint()),
                    ));
                    // Usable at once; the keychain only decides whether it survives a restart.
                    l.token = Some(t);
                    l.token_input.clear();
                }
                Ok(_) => l.token_note = Some("save the URL first".into()),
                Err(e) => l.token_note = Some(e.to_string()),
            },
            LogMsg::TokenForget => {
                l.keychain = Some((
                    wavelog::forget_token_in_background(Arc::clone(&self.secret_store)),
                    "token removed".into(),
                ));
                l.token = None;
                l.token_note = Some("removing the token…".into());
            }
            LogMsg::Operator(s) => {
                l.prefs.operator = s.trim().to_ascii_uppercase();
                save = true;
            }
            LogMsg::Test => match (Server::parse(&l.prefs.url), l.token.clone()) {
                (Ok(server), Some(token)) => l.send(Cmd::Test(Box::new(TestTarget {
                    server: server.with_index_php(l.prefs.index_php),
                    token,
                }))),
                _ => l.token_note = Some("save the URL and the token first".into()),
            },
            LogMsg::Station(c) => {
                l.prefs.station_id = Some(c.0.id);
                save = true;
            }
            LogMsg::SendNow => l.send(Cmd::SendNow),
            LogMsg::Discard(id) => l.send(Cmd::Discard(id)),
            LogMsg::SaveAdif => {
                let path = k4_config::default_config_path().as_deref().and_then(|p| {
                    p.parent()
                        .map(|d| d.join(format!("wavelog-held-{}.adi", unix_now())))
                });
                if let Some(p) = path {
                    l.send(Cmd::SaveAdif(p));
                }
            }
            LogMsg::Call(s) => l.type_call(s, unix_now()),
            LogMsg::RstSent(s) => l.rst_sent = s,
            LogMsg::RstRcvd(s) => l.rst_rcvd = s,
            LogMsg::Name(s) => l.name = s,
            LogMsg::Comment(s) => l.comment = s,
            LogMsg::Digital(d) => {
                l.digital = d.0;
                l.prefs.digital_mode = d.0.label().to_string();
                save = true;
            }
            LogMsg::ManualFreq(s) => l.manual_freq = s,
            LogMsg::Clear => self.log_clear(),
            LogMsg::Log => match self
                .log_draft()
                .and_then(|d| Qso::from_draft(&d).map_err(|e| e.to_string()))
            {
                Ok(q) => {
                    self.log.send(Cmd::Log(Box::new(q)));
                    self.log_clear();
                }
                Err(e) => self.log.form_error = Some(e),
            },
        }
        if self.log.setup().map(|s| (s.server, s.token, s.station_id)) != before {
            self.log.reconfigure();
        }
        if save {
            self.save_config();
        }
        Task::none()
    }

    fn log_clear(&mut self) {
        let l = &mut self.log;
        l.call.clear();
        l.rst_sent.clear();
        l.rst_rcvd.clear();
        l.name.clear();
        l.comment.clear();
        l.time_on = None;
        l.form_error = None;
    }

    fn log_mode(&self) -> Option<Mode> {
        tx_mode(&self.ui.radio)
    }

    fn log_draft(&self) -> Result<Draft, String> {
        draft_from(&self.log, &self.ui.radio, self.ui.connected, unix_now())
    }

    /// Copy the worker's status; take a new Test report's path form and station list into the
    /// config.
    pub(crate) fn log_tick(&mut self) {
        if let Some((rx, done)) = &self.log.keychain {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    self.log.token_note = Some(done.clone());
                    self.log.keychain = None;
                }
                Ok(Err(e)) => {
                    self.log.token_note = Some(format!(
                        "the keychain refused it ({e}) — the token is kept for this session only"
                    ));
                    self.log.keychain = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.log.keychain = None,
            }
        }
        let snap = self
            .log
            .status
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        let new_test = snap
            .test
            .clone()
            .filter(|t| Some(t) != self.log.seen_test.as_ref());
        self.log.view = snap;
        let Some(t) = new_test else { return };
        self.log.seen_test = Some(t.clone());
        let l = &mut self.log;
        if let Some(ix) = t.index_php {
            l.prefs.index_php = ix;
        }
        if let Some(list) = t.locations {
            if l.prefs
                .station_id
                .is_none_or(|id| !list.iter().any(|s| s.id == id))
            {
                l.prefs.station_id = list
                    .iter()
                    .find(|s| s.active)
                    .or(list.first())
                    .map(|s| s.id);
            }
            l.prefs.locations = list;
        }
        self.log.reconfigure();
        self.save_config();
    }

    /// The log strip under the AF recorder (FR-LOG-01, FR-LOG-07), when logging is set up.
    pub(crate) fn log_strip(&self) -> Option<Element<'_, Message>> {
        let l = &self.log;
        if !l.ready() {
            return None;
        }
        let dim = role_color(ui::ColorRole::Inactive);
        let caution = role_color(ui::ColorRole::Caution);
        let r = &self.ui.radio;
        let field = |hint: &'static str, value: &str, w: f32, f: fn(String) -> LogMsg| {
            TextInput::new(hint, value)
                .on_input(move |s| Message::Log(f(s)))
                .on_submit(Message::Log(LogMsg::Log))
                .size(13)
                .width(Length::Fixed(w))
        };
        let mut row = Row::new()
            .spacing(6)
            .align_y(Alignment::Center)
            .push(Text::new("LOG").size(11).color(dim))
            .push(field("call", &l.call, 110.0, LogMsg::Call))
            .push(field("RST s", &l.rst_sent, 52.0, LogMsg::RstSent))
            .push(field("RST r", &l.rst_rcvd, 52.0, LogMsg::RstRcvd))
            .push(field("name", &l.name, 120.0, LogMsg::Name))
            .push(field("comment", &l.comment, 200.0, LogMsg::Comment));
        let connected = self.ui.connected && r.vfo_a_hz.is_some();
        if !connected {
            row = row.push(field("MHz", &l.manual_freq, 90.0, LogMsg::ManualFreq));
        }
        if matches!(self.log_mode(), Some(Mode::Data | Mode::DataRev)) {
            row = row.push(
                pick_list(
                    DigitalMode::ALL.map(DigitalChoice).to_vec(),
                    Some(DigitalChoice(l.digital)),
                    |d| Message::Log(LogMsg::Digital(d)),
                )
                .text_size(12),
            );
        }
        row = row
            .push(small_btn("Log", Message::Log(LogMsg::Log)))
            .push(small_btn("Clear", Message::Log(LogMsg::Clear)));
        // What will be logged from the radio, and the queue's state.
        // What will be logged from the radio — shown before a call is typed, so the operator
        // sees the frequency, band, mode and power the QSO will carry.
        let what = match self.log_draft() {
            Ok(d) => {
                let (m, sub) = adif_mode(d.mode, d.digital);
                format!(
                    "{:.4} MHz {} {}{}",
                    d.tx_hz as f64 / 1e6,
                    k4_log::band::band_for(d.tx_hz).unwrap_or("(no band)"),
                    sub.unwrap_or(m),
                    d.tx_pwr.map(|p| format!(" {p} W")).unwrap_or_default()
                )
            }
            Err(_) => String::new(),
        };
        let v = &l.view;
        let mut state = format!("queue: {} waiting, {} held", v.pending, v.held.len());
        if let Some(p) = &v.paused {
            state = format!("{state} — paused: {p}");
        } else if v.other_sender {
            state = format!("{state} — sent by another K4 Remote");
        }
        let mut col = Column::new().spacing(4).push(row).push(
            Row::new()
                .spacing(12)
                .push(Text::new(what).size(11).color(dim))
                .push(Text::new(state).size(11).color(if v.paused.is_some() {
                    caution
                } else {
                    dim
                }))
                .push(
                    Text::new(v.last.clone().unwrap_or_default())
                        .size(11)
                        .color(dim),
                ),
        );
        if let Some(e) = &l.form_error {
            col = col.push(Text::new(e.clone()).size(11).color(caution));
        }
        Some(col.into())
    }

    /// The LOGGING settings tab (FR-LOG-02, FR-LOG-07).
    pub(crate) fn logging_settings_view(&self) -> Element<'_, Message> {
        let dim = role_color(ui::ColorRole::Inactive);
        let caution = role_color(ui::ColorRole::Caution);
        let l = &self.log;
        let label = |s: &'static str| Text::new(s).size(12).width(Length::Fixed(110.0));
        let mut col = Column::new().spacing(8).push(
            Text::new(
                "Log QSOs to a Wavelog logbook (API v2, Wavelog 3.1.0 or later). Create a token in \
                 Wavelog under API with qso:write, station:read and lookup:read.",
            )
            .size(11)
            .color(dim),
        );
        col = col.push(
            Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(label("Wavelog URL"))
                .push(
                    TextInput::new("https://log.example.org", &l.url_input)
                        .on_input(|s| Message::Log(LogMsg::Url(s)))
                        .on_submit(Message::Log(LogMsg::UrlSave))
                        .size(13)
                        .width(Length::Fixed(360.0)),
                )
                .push(small_btn("Save", Message::Log(LogMsg::UrlSave))),
        );
        if let Some(e) = &l.url_error {
            col = col.push(Text::new(e.clone()).size(11).color(caution));
        }
        let saved = l
            .token
            .as_ref()
            .map_or("none saved".to_string(), |t| format!("saved: {}", t.hint()));
        col = col.push(
            Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(label("Token"))
                .push(
                    TextInput::new("wl2_…", &l.token_input)
                        .secure(true)
                        .on_input(|s| Message::Log(LogMsg::Token(s)))
                        .on_submit(Message::Log(LogMsg::TokenSave))
                        .size(13)
                        .width(Length::Fixed(360.0)),
                )
                .push(small_btn("Save", Message::Log(LogMsg::TokenSave)))
                .push(small_btn("Forget", Message::Log(LogMsg::TokenForget)))
                .push(Text::new(saved).size(11).color(dim)),
        );
        if let Some(n) = &l.token_note {
            col = col.push(Text::new(n.clone()).size(11).color(dim));
        }
        col = col.push(
            Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(label("Operator"))
                .push(
                    TextInput::new("optional (club logbooks)", &l.prefs.operator)
                        .on_input(|s| Message::Log(LogMsg::Operator(s)))
                        .size(13)
                        .width(Length::Fixed(160.0)),
                ),
        );
        let choices: Vec<StationChoice> = l
            .prefs
            .locations
            .iter()
            .cloned()
            .map(StationChoice)
            .collect();
        let chosen = l
            .prefs
            .station_id
            .and_then(|id| l.prefs.locations.iter().find(|s| s.id == id))
            .cloned()
            .map(StationChoice);
        col = col.push(
            Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(label("Station location"))
                .push(
                    pick_list(choices, chosen, |c| Message::Log(LogMsg::Station(c)))
                        .placeholder("press Test to read the list")
                        .text_size(12),
                ),
        );
        col = col.push(
            Row::new()
                .spacing(8)
                .align_y(Alignment::Center)
                .push(small_btn("Test", Message::Log(LogMsg::Test)))
                .push(
                    Text::new(if l.view.testing { "testing…" } else { "" })
                        .size(11)
                        .color(dim),
                ),
        );
        if let Some(t) = &l.view.test {
            for line in &t.lines {
                col = col.push(Text::new(line.clone()).size(11).color(if t.ok {
                    dim
                } else {
                    caution
                }));
            }
        }
        // The queue.
        let v = &l.view;
        col = col.push(Text::new("Queue").size(12).color(dim)).push(
            Text::new(format!(
                "{} waiting, {} held{}",
                v.pending,
                v.held.len(),
                v.paused
                    .as_ref()
                    .map(|p| format!(" — paused: {p}"))
                    .unwrap_or_default()
            ))
            .size(11)
            .color(if v.paused.is_some() { caution } else { dim }),
        );
        for h in &v.held {
            col = col.push(
                Row::new()
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .push(
                        Text::new(format!("{} — {}", h.summary, h.reason))
                            .size(11)
                            .color(caution),
                    )
                    .push(small_btn(
                        "Discard",
                        Message::Log(LogMsg::Discard(h.id.clone())),
                    )),
            );
        }
        col = col.push(
            Row::new()
                .spacing(8)
                .push(small_btn("Send now", Message::Log(LogMsg::SendNow)))
                .push(small_btn(
                    "Save held as ADIF",
                    Message::Log(LogMsg::SaveAdif),
                )),
        );
        if let Some(last) = &v.last {
            col = col.push(Text::new(last.clone()).size(11).color(dim));
        }
        col.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k4_protocol::state::RadioState;

    fn ui() -> LogUi {
        LogUi {
            prefs: WavelogPrefs::default(),
            tx: None,
            status: Arc::default(),
            view: Status::default(),
            url_input: String::new(),
            url_error: None,
            token_input: String::new(),
            token: None,
            token_note: None,
            keychain: None,
            call: String::new(),
            rst_sent: String::new(),
            rst_rcvd: String::new(),
            name: String::new(),
            comment: String::new(),
            time_on: None,
            digital: DigitalMode::Ft8,
            manual_freq: String::new(),
            form_error: None,
            seen_test: None,
        }
    }

    fn radio(a: u64, b: u64, split: bool, mode_a: Mode, mode_b: Mode) -> RadioState {
        RadioState {
            vfo_a_hz: Some(a),
            vfo_b_hz: Some(b),
            split: Some(split),
            mode_a: Some(mode_a),
            mode_b: Some(mode_b),
            tx_power: Some(50),
            tx_power_range: Some('H'),
            ..RadioState::default()
        }
    }

    /// FR-LOG-01: `time_on` is when the call was first typed — not moved by further typing,
    /// dropped when the field is emptied, taken again when it is typed anew.
    /// trace: FR-LOG-01
    #[test]
    fn fr_log_01_time_on_is_when_the_call_is_first_typed() {
        let mut l = ui();
        l.type_call("d".into(), 100);
        assert_eq!((l.call.as_str(), l.time_on), ("D", Some(100)));
        l.type_call("DL1".into(), 105);
        assert_eq!(l.time_on, Some(100), "further typing keeps it");
        l.type_call(String::new(), 110);
        assert_eq!(l.time_on, None);
        l.type_call("DK2".into(), 200);
        assert_eq!(l.time_on, Some(200));
    }

    /// FR-LOG-01, FR-LOG-08: the draft takes frequency, mode and power from the radio — in split
    /// the transmit VFO B and its mode, with VFO A as the receive frequency — the times from the
    /// form and the clock, and the mode's RST when none is typed.
    /// trace: FR-LOG-01, FR-LOG-08
    #[test]
    fn fr_log_01_the_draft_comes_from_the_radio() {
        let mut l = ui();
        l.type_call("dl1abc".into(), 1_000);
        let plain = radio(14_250_000, 7_000_000, false, Mode::Usb, Mode::Cw);
        let d = draft_from(&l, &plain, true, 1_300).unwrap();
        assert_eq!((d.tx_hz, d.rx_hz, d.mode), (14_250_000, None, Mode::Usb));
        assert_eq!((d.time_on, d.time_off), (1_000, 1_300));
        assert_eq!((d.rst_sent.as_str(), d.rst_rcvd.as_str()), ("59", "59"));
        assert_eq!(d.tx_pwr, Some(50.0));
        let split = radio(14_000_000, 14_005_000, true, Mode::Usb, Mode::Cw);
        let d = draft_from(&l, &split, true, 1_300).unwrap();
        assert_eq!(
            (d.tx_hz, d.rx_hz, d.mode),
            (14_005_000, Some(14_000_000), Mode::Cw)
        );
        assert_eq!(d.rst_sent, "599", "CW's default");
        l.rst_rcvd = " 579 ".into();
        assert_eq!(
            draft_from(&l, &split, true, 1_300).unwrap().rst_rcvd,
            "579",
            "a typed RST is kept"
        );
    }

    /// FR-LOG-01: with no radio connected the typed frequency is used (a comma as the decimal
    /// mark too), and without one the form says what is missing.
    /// trace: FR-LOG-01
    #[test]
    fn fr_log_01_without_a_radio_the_typed_frequency() {
        let mut l = ui();
        l.type_call("DL1ABC".into(), 1_000);
        let r = RadioState::default();
        assert!(draft_from(&l, &r, false, 1_100)
            .unwrap_err()
            .contains("MHz"));
        l.manual_freq = "7,074".into();
        let d = draft_from(&l, &r, false, 1_100).unwrap();
        assert_eq!((d.tx_hz, d.rx_hz), (7_074_000, None));
        // Connected but no frequency yet counts as no radio.
        assert_eq!(draft_from(&l, &r, true, 1_100).unwrap().tx_hz, 7_074_000);
    }

    /// FR-LOG-03, FR-CFG-08: no keychain I/O on the UI thread — this file stores and removes the
    /// token only through the background helpers. (A save that ran here froze the window for the
    /// D-Bus timeout when no keyring answered, seen 2026-10-27 on a private bus.)
    /// trace: FR-LOG-03
    #[test]
    fn fr_log_03_no_keychain_io_on_the_ui_thread() {
        let text = include_str!("log_ui.rs");
        let prod = &text[..text.find(concat!("mod ", "tests {")).expect("test module")];
        for blocking in [
            concat!("wavelog::save_", "token("),
            concat!("wavelog::forget_", "token("),
        ] {
            assert!(!prod.contains(blocking), "{blocking} runs on the UI thread");
        }
        for stray in [".set(", ".delete(", ".get("] {
            assert!(
                !prod.contains(&format!("secret_store{stray}")),
                "secret_store{stray} on the UI thread"
            );
        }
        assert!(
            prod.contains("save_token_in_background(")
                && prod.contains("forget_token_in_background(")
        );
    }

    /// FR-LOG-01: nothing reaches Wavelog except by the Log action — the one place that queues a
    /// QSO is the `LogMsg::Log` arm, and the worker fills its queue from that command only.
    /// Structural, over the production code of this file and the worker's.
    /// trace: FR-LOG-01
    #[test]
    fn fr_log_01_nothing_is_sent_without_the_log_action() {
        let text = include_str!("log_ui.rs");
        let prod = &text[..text.find(concat!("mod ", "tests {")).expect("test module")];
        let needle = concat!("Cmd::", "Log(");
        let sites: Vec<usize> = prod.match_indices(needle).map(|(i, _)| i).collect();
        assert_eq!(sites.len(), 1, "one place queues a QSO");
        let arm = prod[..sites[0]]
            .rfind("LogMsg::")
            .map(|i| &prod[i..sites[0]])
            .unwrap_or("");
        assert!(
            arm.starts_with("LogMsg::Log =>"),
            "it is the Log action's arm: {arm:.60}"
        );
        let worker = include_str!("wavelog.rs");
        let worker = &worker[..worker
            .find(concat!("mod ", "tests {"))
            .expect("test module")];
        assert_eq!(
            worker.matches(".add(").count(),
            1,
            "the queue is filled from Cmd::Log only"
        );
    }
}
