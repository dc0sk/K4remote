//! K4 Remote — GUI application (ARC-08, iced).
//!
//! The view is a pure projection of [`UiSnapshot`] (ADR-04); all radio I/O runs
//! on a background [`worker`] thread, bridged by a command channel + a shared
//! snapshot polled on a timer (ADR-06, FR-UI-07). This is the P1b skeleton:
//! connection management, live state display, basic tuning/mode, and the
//! mandatory TX arm / emergency-stop affordances (FR-UI-01/02/03/04/06).

mod spectrum;
mod worker;

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use k4_config::{Config, Profile, SecretStore};

use iced::widget::canvas::Canvas;
use iced::widget::{Button, Column, Container, Row, Text, TextInput};
use iced::{Element, Length, Subscription, Task};

use worker::{ConnectTarget, UiSnapshot, WorkerCmd};

pub fn main() -> iced::Result {
    iced::application("K4 Remote", App::update, App::view)
        .subscription(App::subscription)
        .run_with(App::new)
}

struct App {
    // connection form
    host: String,
    port: String,
    password: String,
    // tuning form
    freq_mhz: String,
    // use TLS-PSK (port 9204) instead of plaintext (9205)
    use_tls: bool,
    // serial (USB/RS232) transport instead of Ethernet
    serial_mode: bool,
    serial_path: String,
    serial_baud: String,
    // remember the password in the OS keychain (FR-CFG-03)
    remember: bool,
    secret_store: Box<dyn SecretStore>,
    // raw CAT command entry (diagnostics console)
    cat_input: String,
    // where the config (profiles/prefs) is persisted
    config_path: Option<PathBuf>,
    // bridge to the worker
    cmd_tx: Sender<WorkerCmd>,
    snapshot: Arc<Mutex<UiSnapshot>>,
    // last snapshot read (what the view renders)
    ui: UiSnapshot,
}

#[derive(Debug, Clone)]
enum Message {
    HostChanged(String),
    PortChanged(String),
    PasswordChanged(String),
    FreqChanged(String),
    Connect,
    Disconnect,
    ToggleTls,
    ToggleRemember,
    ToggleSerialMode,
    SerialPathChanged(String),
    SerialBaudChanged(String),
    SetFreq,
    SetMode(u8),
    Band(bool),
    ToggleAtten,
    ToggleSplit,
    CycleAgc,
    ToggleNb,
    ToggleNr,
    TogglePreamp,
    ToggleRit,
    ToggleXit,
    ClearRitXit,
    ToggleArm,
    ToggleKey,
    EmergencyStop,
    CatInputChanged(String),
    SendCat,
    Tick,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let snapshot = Arc::new(Mutex::new(UiSnapshot::default()));
        worker::spawn(cmd_rx, Arc::clone(&snapshot));

        // Load persisted config and prefill the last-used connection (FR-CFG-01).
        let config_path = k4_config::default_config_path();
        let config = config_path.as_deref().map(Config::load).unwrap_or_default();
        let last = config.last.unwrap_or(Profile {
            host: "192.168.1.100".into(),
            port: 9205,
            use_tls: false,
            remember: false,
        });

        // Choose a secret store; load the saved password if "remember" is set.
        #[cfg(feature = "keychain")]
        let secret_store: Box<dyn SecretStore> = Box::new(k4_config::KeyringStore::new("k4remote"));
        #[cfg(not(feature = "keychain"))]
        let secret_store: Box<dyn SecretStore> = Box::new(k4_config::MemoryStore::new());

        let password = if last.remember {
            secret_store
                .get(&account_key(&last.host, last.port))
                .unwrap_or_default()
        } else {
            String::new()
        };

        let app = App {
            host: last.host,
            port: last.port.to_string(),
            password,
            freq_mhz: "14.074".into(),
            use_tls: last.use_tls,
            serial_mode: false,
            serial_path: "/dev/ttyUSB0".into(),
            serial_baud: "38400".into(),
            remember: last.remember,
            secret_store,
            cat_input: String::new(),
            config_path,
            cmd_tx,
            snapshot,
            ui: UiSnapshot::default(),
        };
        (app, Task::none())
    }

    /// Persist the current connection profile (no password) and store/clear the
    /// password in the keychain per the "remember" choice (FR-CFG-01/03).
    fn save_profile(&self) {
        let Ok(port) = self.port.parse::<u16>() else {
            return;
        };
        let account = account_key(&self.host, port);
        if self.remember {
            let _ = self.secret_store.set(&account, &self.password);
        } else {
            let _ = self.secret_store.delete(&account);
        }
        if let Some(path) = self.config_path.as_deref() {
            let cfg = Config {
                last: Some(Profile {
                    host: self.host.clone(),
                    port,
                    use_tls: self.use_tls,
                    remember: self.remember,
                }),
                ..Default::default()
            };
            let _ = cfg.save(path);
        }
    }

    fn send(&self, cmd: WorkerCmd) {
        let _ = self.cmd_tx.send(cmd);
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::HostChanged(v) => self.host = v,
            Message::PortChanged(v) => self.port = v,
            Message::PasswordChanged(v) => self.password = v,
            Message::FreqChanged(v) => self.freq_mhz = v,
            Message::Connect => {
                if self.serial_mode {
                    self.send(WorkerCmd::Connect(ConnectTarget::Serial {
                        path: self.serial_path.clone(),
                        baud: self.serial_baud.trim().parse().unwrap_or(38400),
                    }));
                } else if let Ok(port) = self.port.parse::<u16>() {
                    self.save_profile(); // remember host/port/tls for next launch
                    self.send(WorkerCmd::Connect(ConnectTarget::Tcp {
                        host: self.host.clone(),
                        port,
                        password: self.password.clone(),
                        use_tls: self.use_tls,
                    }));
                }
            }
            Message::Disconnect => self.send(WorkerCmd::Disconnect),
            Message::ToggleTls => {
                self.use_tls = !self.use_tls;
                // Convenience: default each scheme to its standard port.
                self.port = if self.use_tls { "9204" } else { "9205" }.into();
            }
            Message::ToggleRemember => self.remember = !self.remember,
            Message::ToggleSerialMode => self.serial_mode = !self.serial_mode,
            Message::SerialPathChanged(v) => self.serial_path = v,
            Message::SerialBaudChanged(v) => self.serial_baud = v,
            Message::SetFreq => {
                if let Some(hz) = parse_mhz(&self.freq_mhz) {
                    self.send(WorkerCmd::SetFreqA(hz));
                }
            }
            Message::SetMode(digit) => self.send(WorkerCmd::SetMode(digit)),
            Message::Band(up) => self.send(WorkerCmd::Band(up)),
            Message::ToggleAtten => self.send(WorkerCmd::ToggleAtten),
            Message::ToggleSplit => self.send(WorkerCmd::ToggleSplit),
            Message::CycleAgc => self.send(WorkerCmd::CycleAgc),
            Message::ToggleNb => self.send(WorkerCmd::ToggleNb),
            Message::ToggleNr => self.send(WorkerCmd::ToggleNr),
            Message::TogglePreamp => self.send(WorkerCmd::TogglePreamp),
            Message::ToggleRit => self.send(WorkerCmd::ToggleRit),
            Message::ToggleXit => self.send(WorkerCmd::ToggleXit),
            Message::ClearRitXit => self.send(WorkerCmd::ClearRitXit),
            Message::ToggleArm => self.send(WorkerCmd::ArmTx(!self.ui.tx_armed)),
            Message::ToggleKey => self.send(WorkerCmd::Key(!self.ui.transmitting)),
            Message::EmergencyStop => self.send(WorkerCmd::EmergencyStop),
            Message::CatInputChanged(v) => self.cat_input = v,
            Message::SendCat => {
                let cmd = self.cat_input.trim();
                if !cmd.is_empty() {
                    self.send(WorkerCmd::SendRawCat(cmd.to_string()));
                    self.cat_input.clear();
                }
            }
            Message::Tick => {
                if let Ok(snap) = self.snapshot.lock() {
                    self.ui = snap.clone();
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        // Poll the shared snapshot ~6×/s; the UI thread never blocks on I/O.
        iced::time::every(Duration::from_millis(150)).map(|_| Message::Tick)
    }

    fn view(&self) -> Element<'_, Message> {
        let connected = self.ui.connected;

        let conn_status = if connected {
            "CONNECTED"
        } else {
            "disconnected"
        };
        let header = Row::new()
            .spacing(12)
            .push(Text::new("K4 Remote").size(24))
            .push(Text::new(conn_status).size(16));

        // Connection panel (FR-UI-01) — Ethernet or serial fields.
        let mode_label = if self.serial_mode {
            "Mode: Serial"
        } else {
            "Mode: Ethernet"
        };
        let fields: Column<Message> = if self.serial_mode {
            Column::new()
                .spacing(6)
                .push(labeled(
                    "Serial port",
                    &self.serial_path,
                    Message::SerialPathChanged,
                ))
                .push(labeled(
                    "Baud",
                    &self.serial_baud,
                    Message::SerialBaudChanged,
                ))
        } else {
            Column::new()
                .spacing(6)
                .push(labeled("Host", &self.host, Message::HostChanged))
                .push(labeled("Port", &self.port, Message::PortChanged))
                .push(secret("Password", &self.password, Message::PasswordChanged))
        };
        let mut actions = Row::new()
            .spacing(8)
            .push(Button::new(Text::new("Connect")).on_press(Message::Connect))
            .push(Button::new(Text::new("Disconnect")).on_press(Message::Disconnect));
        if !self.serial_mode {
            actions = actions
                .push(
                    Button::new(Text::new(if self.use_tls { "TLS: on" } else { "TLS: off" }))
                        .on_press(Message::ToggleTls),
                )
                .push(
                    Button::new(Text::new(if self.remember {
                        "Remember: on"
                    } else {
                        "Remember: off"
                    }))
                    .on_press(Message::ToggleRemember),
                );
        }
        let conn_panel = Column::new()
            .spacing(6)
            .push(Text::new("Connection").size(18))
            .push(Button::new(Text::new(mode_label)).on_press(Message::ToggleSerialMode))
            .push(fields)
            .push(actions);

        // Radio state (FR-UI-02/03).
        let state_panel = Column::new()
            .spacing(4)
            .push(Text::new("Radio state").size(18))
            .push(Text::new(format!("VFO A: {}", fmt_hz(self.ui.vfo_a_hz))))
            .push(Text::new(format!("VFO B: {}", fmt_hz(self.ui.vfo_b_hz))))
            .push(Text::new(format!(
                "Mode:  {}",
                self.ui.mode_a.unwrap_or("—")
            )))
            .push(Text::new(format!("Split: {}", fmt_bool(self.ui.split))))
            .push(Text::new(format!(
                "Bandwidth: {}   Atten: {}",
                fmt_bandwidth(self.ui.bandwidth_hz),
                fmt_atten(self.ui.atten_db, self.ui.atten_on),
            )))
            .push(Text::new(format!("S-meter: {}", fmt_smeter(&self.ui))))
            .push(Text::new(if self.ui.transmitting {
                "STATE: ** TRANSMIT **"
            } else {
                "STATE: receive"
            }))
            .push(Text::new(format!(
                "RX audio frames: {}   spectrum: {} bins",
                self.ui.audio_frames, self.ui.spectrum_bins
            )));

        // Tuning + mode (FR-UI-02).
        let tuning = Row::new()
            .spacing(8)
            .push(Text::new("VFO A MHz:"))
            .push(
                TextInput::new("14.074", &self.freq_mhz)
                    .on_input(Message::FreqChanged)
                    .width(Length::Fixed(120.0)),
            )
            .push(Button::new(Text::new("Set")).on_press(Message::SetFreq));

        let modes = Row::new()
            .spacing(6)
            .push(Button::new(Text::new("LSB")).on_press(Message::SetMode(1)))
            .push(Button::new(Text::new("USB")).on_press(Message::SetMode(2)))
            .push(Button::new(Text::new("CW")).on_press(Message::SetMode(3)))
            .push(Button::new(Text::new("DATA")).on_press(Message::SetMode(6)));

        // Band / RX controls (FR-VFO-04, FR-VFO-06, FR-RX-02).
        let rx_controls = Row::new()
            .spacing(6)
            .push(Button::new(Text::new("Band ▼")).on_press(Message::Band(false)))
            .push(Button::new(Text::new("Band ▲")).on_press(Message::Band(true)))
            .push(Button::new(Text::new("Split")).on_press(Message::ToggleSplit))
            .push(Button::new(Text::new("Atten")).on_press(Message::ToggleAtten));

        // DSP controls (FR-RX-03/04).
        let dsp = Row::new()
            .spacing(6)
            .push(
                Button::new(Text::new(format!("AGC: {}", fmt_agc(self.ui.agc_mode))))
                    .on_press(Message::CycleAgc),
            )
            .push(
                Button::new(Text::new(format!("NB {}", on_off(self.ui.nb_on))))
                    .on_press(Message::ToggleNb),
            )
            .push(
                Button::new(Text::new(format!("NR {}", on_off(self.ui.nr_on))))
                    .on_press(Message::ToggleNr),
            )
            .push(
                Button::new(Text::new(format!("Pre {}", on_off(self.ui.preamp_on))))
                    .on_press(Message::TogglePreamp),
            );

        // RIT / XIT controls (FR-VFO-05).
        let ritxit = Row::new()
            .spacing(6)
            .push(
                Button::new(Text::new(format!("RIT {}", on_off(self.ui.rit_on))))
                    .on_press(Message::ToggleRit),
            )
            .push(
                Button::new(Text::new(format!("XIT {}", on_off(self.ui.xit_on))))
                    .on_press(Message::ToggleXit),
            )
            .push(Button::new(Text::new("Clear")).on_press(Message::ClearRitXit));

        // TX safety affordances (FR-UI-04/06, FR-TX-SAFE-*).
        let arm_label = if self.ui.tx_armed {
            "TX ARMED — tap to DISARM"
        } else {
            "TX disarmed — tap to ARM"
        };
        let key_label = if self.ui.transmitting {
            "UNKEY"
        } else {
            "KEY (PTT)"
        };
        let tx_panel = Column::new()
            .spacing(6)
            .push(Text::new("Transmit").size(18))
            .push(Button::new(Text::new(arm_label)).on_press(Message::ToggleArm))
            .push(
                Row::new()
                    .spacing(8)
                    .push(Button::new(Text::new(key_label)).on_press(Message::ToggleKey))
                    .push(
                        Button::new(Text::new("EMERGENCY STOP")).on_press(Message::EmergencyStop),
                    ),
            );

        // Spectrum + waterfall (FR-PAN-02/03, FR-UI-05). Falls back to a label
        // until the first frame arrives.
        let spectrum_view: Element<Message> = if self.ui.spectrum_latest.is_empty() {
            Container::new(Text::new("[ spectrum + waterfall — waiting for data ]"))
                .width(Length::Fill)
                .height(Length::Fixed(260.0))
                .padding(8)
                .into()
        } else {
            Canvas::new(spectrum::Spectrum {
                latest: &self.ui.spectrum_latest,
                waterfall: &self.ui.waterfall,
                top_dbm: -30.0,
                range_db: 100.0,
            })
            .width(Length::Fill)
            .height(Length::Fixed(260.0))
            .into()
        };

        // Diagnostics console (FR-DIAG-01/02).
        let mut log_col = Column::new().spacing(1);
        for line in self.ui.diag_lines.iter().rev().take(12) {
            log_col = log_col.push(Text::new(line.clone()).size(12));
        }
        let diagnostics = Column::new()
            .spacing(6)
            .push(Text::new("Diagnostics").size(18))
            .push(
                Row::new()
                    .spacing(8)
                    .push(
                        TextInput::new("raw CAT, e.g. IF;", &self.cat_input)
                            .on_input(Message::CatInputChanged)
                            .on_submit(Message::SendCat)
                            .width(Length::Fixed(220.0)),
                    )
                    .push(Button::new(Text::new("Send")).on_press(Message::SendCat)),
            )
            .push(log_col);

        let body = Column::new()
            .spacing(16)
            .padding(20)
            .push(header)
            .push(Text::new(format!("Status: {}", self.ui.status)))
            .push(conn_panel)
            .push(state_panel)
            .push(tuning)
            .push(modes)
            .push(rx_controls)
            .push(dsp)
            .push(ritxit)
            .push(tx_panel)
            .push(spectrum_view)
            .push(diagnostics);

        Container::new(body).width(Length::Fill).into()
    }
}

// --- view helpers -----------------------------------------------------------

fn labeled<'a>(
    label: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    Row::new()
        .spacing(8)
        .push(Text::new(label).width(Length::Fixed(80.0)))
        .push(TextInput::new("", value).on_input(on_input))
        .into()
}

fn secret<'a>(
    label: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    Row::new()
        .spacing(8)
        .push(Text::new(label).width(Length::Fixed(80.0)))
        .push(TextInput::new("", value).secure(true).on_input(on_input))
        .into()
}

fn fmt_hz(hz: Option<u64>) -> String {
    match hz {
        Some(hz) => format!("{:.6} MHz", hz as f64 / 1_000_000.0),
        None => "—".to_string(),
    }
}

fn fmt_smeter(ui: &UiSnapshot) -> String {
    match (ui.s_meter_dbm, ui.s_meter_bars) {
        (Some(dbm), _) => format!("{} ({dbm} dBm)", k4_protocol::s_unit_label(dbm)),
        (None, Some(bars)) => format!("{bars} bars"),
        _ => "—".to_string(),
    }
}

fn fmt_bandwidth(hz: Option<u32>) -> String {
    match hz {
        Some(hz) => format!("{hz} Hz"),
        None => "—".to_string(),
    }
}

fn fmt_atten(db: Option<u8>, on: Option<bool>) -> String {
    match (db, on) {
        (Some(db), Some(true)) => format!("{db} dB"),
        (_, Some(false)) => "off".to_string(),
        _ => "—".to_string(),
    }
}

fn fmt_agc(mode: Option<u8>) -> &'static str {
    match mode {
        Some(0) => "OFF",
        Some(1) => "SLOW",
        Some(2) => "FAST",
        _ => "—",
    }
}

fn on_off(b: Option<bool>) -> &'static str {
    match b {
        Some(true) => "ON",
        Some(false) => "off",
        None => "—",
    }
}

fn fmt_bool(b: Option<bool>) -> &'static str {
    match b {
        Some(true) => "on",
        Some(false) => "off",
        None => "—",
    }
}

/// Keychain account key for a connection (`host:port`).
fn account_key(host: &str, port: u16) -> String {
    format!("{host}:{port}")
}

/// Parse a MHz string (e.g. "14.074") into Hz.
fn parse_mhz(s: &str) -> Option<u64> {
    let mhz: f64 = s.trim().parse().ok()?;
    if mhz <= 0.0 {
        return None;
    }
    Some((mhz * 1_000_000.0).round() as u64)
}
