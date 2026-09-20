//! The worker thread that runs the spot sources — the Reverse Beacon Network, a DX cluster (both
//! telnet) and PSK Reporter (MQTT) — keeps the spot store fed, and reports what each is doing
//! (FR-SPOT-05/07/09).
//!
//! Each source is polled independently on this thread, off the UI and the radio-control paths, so
//! a slow, blocked or failing network can never delay the UI, CAT or audio, and one network's
//! failure does not stop the others (FR-SPOT-09). The UI sends what to run ([`Cmd`]) and reads the
//! outcome ([`Statuses`]) once per tick.
//!
//! The frequency window the UI sends does two jobs: it filters what the sources keep, and it picks
//! which PSK Reporter band topics to subscribe to, so the live feed is asked only for the bands the
//! VFOs are on.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use k4_spot::mqtt_source::{MqttConfig, MqttSource};
use k4_spot::psk::{bands_overlapping, topic_for_band};
use k4_spot::telnet::{ConnState, Stats, TelnetConfig, TelnetSource};
use k4_spot::{SourceError, Spot, SpotSource};

use crate::spots::SpotHandle;

/// What one source is doing.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: ConnState,
    /// The most recent failure, kept until the source is connected again.
    pub error: Option<String>,
    pub stats: Stats,
}

/// Each source's status; `None` = not enabled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Statuses {
    pub rbn: Option<Status>,
    pub dx_cluster: Option<Status>,
    pub psk_reporter: Option<Status>,
}

pub type StatusHandle = Arc<Mutex<Statuses>>;

/// Instructions from the UI.
pub enum Cmd {
    /// Run these sources (`None` = off). A source is restarted only when its settings change.
    Configure {
        rbn: Option<TelnetConfig>,
        dx_cluster: Option<TelnetConfig>,
        psk_reporter: Option<MqttConfig>,
    },
    /// Keep only spots inside `[lo, hi]` Hz, and (for PSK Reporter) subscribe to the bands that
    /// overlap it. An empty range (`lo > hi`) keeps none and subscribes to none; `None` keeps
    /// every telnet spot but subscribes to no PSK Reporter band.
    Window(Option<(u64, u64)>),
}

#[derive(Debug, Clone, PartialEq)]
enum FeedConfig {
    Telnet(TelnetConfig),
    Mqtt(MqttConfig),
}

enum Feed {
    Telnet(TelnetSource),
    Mqtt(MqttSource),
}

impl Feed {
    fn new(cfg: &FeedConfig) -> Self {
        match cfg {
            FeedConfig::Telnet(c) => Feed::Telnet(TelnetSource::new(c.clone())),
            FeedConfig::Mqtt(c) => Feed::Mqtt(MqttSource::new(c.clone())),
        }
    }

    fn poll(&mut self, sink: &mut dyn FnMut(Spot)) -> Result<(), SourceError> {
        match self {
            Feed::Telnet(s) => s.poll(sink),
            Feed::Mqtt(s) => s.poll(sink),
        }
    }

    fn set_window(&mut self, w: Option<(u64, u64)>) {
        match self {
            Feed::Telnet(s) => s.set_window(w),
            Feed::Mqtt(s) => s.set_window(w),
        }
    }

    /// Telnet feeds have no topics; an MQTT feed subscribes to exactly these.
    fn set_topics(&mut self, topics: Vec<String>) {
        if let Feed::Mqtt(s) = self {
            s.set_topics(topics);
        }
    }

    fn state(&self) -> ConnState {
        match self {
            Feed::Telnet(s) => s.state(),
            Feed::Mqtt(s) => s.state(),
        }
    }

    fn stats(&self) -> Stats {
        match self {
            Feed::Telnet(s) => s.stats(),
            Feed::Mqtt(s) => s.stats(),
        }
    }
}

#[derive(Default)]
struct Slot {
    cfg: Option<FeedConfig>,
    src: Option<Feed>,
    error: Option<String>,
}

impl Slot {
    fn status(&self) -> Option<Status> {
        let src = self.src.as_ref()?;
        Some(Status {
            state: src.state(),
            error: self.error.clone(),
            stats: src.stats(),
        })
    }
}

/// The band topics for a window: one per band the window overlaps, none for an empty window.
fn topics_for(window: Option<(u64, u64)>) -> Vec<String> {
    window
        .map(|(lo, hi)| {
            bands_overlapping(lo, hi)
                .into_iter()
                .map(topic_for_band)
                .collect()
        })
        .unwrap_or_default()
}

/// Start the worker. It ends when the UI drops its end of the command channel.
pub fn spawn(rx: Receiver<Cmd>, store: SpotHandle, status: StatusHandle) {
    let _ = thread::Builder::new()
        .name("spot-sources".into())
        .spawn(move || run(&rx, &store, &status));
}

fn run(rx: &Receiver<Cmd>, store: &SpotHandle, status: &StatusHandle) {
    let mut slots = [Slot::default(), Slot::default(), Slot::default()];
    // Until told otherwise, keep nothing and subscribe to nothing: no radio, no view, nothing to
    // label.
    let mut window: Option<(u64, u64)> = Some((1, 0));
    let mut published = Statuses::default();
    loop {
        loop {
            match rx.try_recv() {
                Ok(Cmd::Configure {
                    rbn,
                    dx_cluster,
                    psk_reporter,
                }) => {
                    let wanted = [
                        rbn.map(FeedConfig::Telnet),
                        dx_cluster.map(FeedConfig::Telnet),
                        psk_reporter.map(FeedConfig::Mqtt),
                    ];
                    for (slot, want) in slots.iter_mut().zip(wanted) {
                        if slot.cfg != want {
                            slot.error = None;
                            slot.src = want.as_ref().map(|cfg| {
                                let mut s = Feed::new(cfg);
                                s.set_window(window);
                                s.set_topics(topics_for(window));
                                s
                            });
                            slot.cfg = want;
                        }
                    }
                }
                Ok(Cmd::Window(w)) => {
                    window = w;
                    let topics = topics_for(w);
                    for src in slots.iter_mut().filter_map(|s| s.src.as_mut()) {
                        src.set_window(w);
                        src.set_topics(topics.clone());
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let mut active = false;
        for slot in &mut slots {
            let Some(src) = slot.src.as_mut() else {
                continue;
            };
            active = true;
            let mut got = Vec::new();
            match src.poll(&mut |s| got.push(s)) {
                Err(e) => slot.error = Some(e.0),
                Ok(()) if src.state() == ConnState::Connected => slot.error = None,
                Ok(()) => {}
            }
            if !got.is_empty() {
                if let Ok(mut st) = store.lock() {
                    for s in got {
                        st.insert(s);
                    }
                }
            }
        }

        let now = Statuses {
            rbn: slots[0].status(),
            dx_cluster: slots[1].status(),
            psk_reporter: slots[2].status(),
        };
        if now != published {
            if let Ok(mut g) = status.lock() {
                *g = now.clone();
            }
            published = now;
        }
        if !active {
            thread::sleep(Duration::from_millis(100));
        }
    }
}

/// One line describing a source, and whether it is a problem worth colouring.
pub fn describe(status: &Status) -> (String, bool) {
    match (status.state, &status.error) {
        (ConnState::Connected, _) => {
            let st = &status.stats;
            let shed = if st.shed > 0 {
                format!(", {} shed", st.shed)
            } else {
                String::new()
            };
            (format!("connected — {} spots{shed}", st.spots), false)
        }
        (ConnState::AwaitingLogin, _) => ("connected, logging in…".into(), false),
        (ConnState::Disconnected, Some(e)) => (e.clone(), true),
        (ConnState::Disconnected, None) => ("connecting…".into(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use k4_spot::Network;

    /// A mock cluster: prompt, read the login, then send one spot line.
    fn mock_cluster(spot_line: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            s.write_all(b"Please enter your call: ").unwrap();
            let mut login = String::new();
            BufReader::new(s.try_clone().unwrap())
                .read_line(&mut login)
                .unwrap();
            for _ in 0..3 {
                let _ = s.write_all(spot_line.as_bytes());
                thread::sleep(Duration::from_millis(30));
            }
            thread::sleep(Duration::from_secs(3));
        });
        port
    }

    fn cfg(network: Network, port: u16) -> TelnetConfig {
        TelnetConfig {
            network,
            host: "127.0.0.1".into(),
            port,
            login: "dc0sk".into(),
        }
    }

    fn mqtt_cfg(port: u16) -> MqttConfig {
        MqttConfig {
            network: Network::PskReporter,
            host: "127.0.0.1".into(),
            port,
        }
    }

    fn wait(what: &str, mut ok: impl FnMut() -> bool) {
        let t0 = Instant::now();
        while !ok() {
            assert!(
                t0.elapsed() < Duration::from_secs(5),
                "timed out waiting for: {what}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// A packet from the client: type byte and body.
    fn read_packet(s: &mut std::net::TcpStream, wait: Duration) -> Option<(u8, Vec<u8>)> {
        s.set_read_timeout(Some(wait)).ok()?;
        let mut first = [0u8; 1];
        s.read_exact(&mut first).ok()?;
        let (mut len, mut mult) = (0usize, 1usize);
        loop {
            let mut b = [0u8; 1];
            s.read_exact(&mut b).ok()?;
            len += usize::from(b[0] & 0x7F) * mult;
            mult *= 128;
            if b[0] & 0x80 == 0 {
                break;
            }
        }
        let mut body = vec![0u8; len];
        s.read_exact(&mut body).ok()?;
        Some((first[0], body))
    }

    fn topics_in(body: &[u8], qos_byte: bool) -> Vec<String> {
        let mut out = Vec::new();
        let mut i = 2;
        while i + 2 <= body.len() {
            let n = usize::from(u16::from_be_bytes([body[i], body[i + 1]]));
            out.push(String::from_utf8_lossy(&body[i + 2..i + 2 + n]).into_owned());
            i += 2 + n + usize::from(qos_byte);
        }
        out
    }

    /// FR-SPOT-07/09: the worker runs a healthy source into the store while another source fails —
    /// the failure is reported against its own network with its reason, and does not stop the
    /// healthy one; and spots outside the window are not stored.
    /// trace: FR-SPOT-07, FR-SPOT-09
    #[test]
    fn fr_spot_09_failing_source_does_not_stop_the_other() {
        let store: SpotHandle = Arc::default();
        let status: StatusHandle = Arc::default();
        let (tx, rx) = mpsc::channel();
        spawn(rx, Arc::clone(&store), Arc::clone(&status));

        let good = mock_cluster("DX de K1TTT-#: 14074.0 W1AW CW 30 dB 20 WPM CQ 1200Z\r\n");
        let dead = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port(); // refused
        tx.send(Cmd::Window(Some((14_000_000, 14_100_000))))
            .unwrap();
        tx.send(Cmd::Configure {
            rbn: Some(cfg(Network::Rbn, good)),
            dx_cluster: Some(cfg(Network::DxCluster, dead)),
            psk_reporter: None,
        })
        .unwrap();

        wait("the spot reaches the store", || {
            !store.lock().unwrap().is_empty()
        });
        wait("the failure is reported", || {
            status
                .lock()
                .unwrap()
                .dx_cluster
                .as_ref()
                .is_some_and(|s| s.error.is_some())
        });
        let s = status.lock().unwrap().clone();
        let rbn = s.rbn.expect("rbn is running");
        let dx = s.dx_cluster.expect("dx cluster is running");
        assert_eq!(rbn.state, ConnState::Connected);
        assert_eq!(rbn.error, None);
        assert!(rbn.stats.spots >= 1);
        assert_eq!(dx.state, ConnState::Disconnected);
        assert!(
            dx.error.as_deref().is_some_and(|e| e.contains("connect")),
            "{:?}",
            dx.error
        );
        assert!(s.psk_reporter.is_none(), "PSK Reporter was not asked for");
        let stored = store.lock().unwrap();
        assert_eq!(stored.spots()[0].call, "W1AW");
        assert_eq!(stored.spots()[0].network, Network::Rbn);
        drop(stored);

        // Turning every source off removes its status.
        tx.send(Cmd::Configure {
            rbn: None,
            dx_cluster: None,
            psk_reporter: None,
        })
        .unwrap();
        wait("all sources are stopped", || {
            let s = status.lock().unwrap();
            s.rbn.is_none() && s.dx_cluster.is_none() && s.psk_reporter.is_none()
        });
    }

    /// FR-SPOT-05: the worker subscribes PSK Reporter to exactly the bands the window overlaps,
    /// moves the subscription when the window moves, and stores what arrives.
    /// trace: FR-SPOT-05
    #[test]
    fn fr_spot_05_worker_follows_the_window_with_band_subscriptions() {
        let store: SpotHandle = Arc::default();
        let status: StatusHandle = Arc::default();
        let (tx, rx) = mpsc::channel();
        spawn(rx, Arc::clone(&store), Arc::clone(&status));

        let (seen_tx, seen_rx) = mpsc::channel();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let _ = read_packet(&mut s, Duration::from_secs(3)).unwrap(); // CONNECT
            s.write_all(&[0x20, 0x02, 0x00, 0x00]).unwrap();
            let mut announced = false;
            while let Some((t, body)) = read_packet(&mut s, Duration::from_millis(1500)) {
                match t {
                    0x82 => {
                        seen_tx.send(("subscribe", topics_in(&body, true))).unwrap();
                        s.write_all(&[0x90, 0x03, body[0], body[1], 0x00]).unwrap();
                        if !announced {
                            announced = true;
                            let now = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap()
                                .as_secs();
                            let payload = format!(
                                r#"{{"f":14074742,"md":"FT8","rp":-9,"t":{now},"sc":"AA1AAA","rc":"BB2BBB","b":"20m"}}"#
                            );
                            let mut m = vec![0x30, (2 + 1 + payload.len()) as u8, 0, 1, b't'];
                            m.extend_from_slice(payload.as_bytes());
                            s.write_all(&m).unwrap();
                        }
                    }
                    0xA2 => seen_tx
                        .send(("unsubscribe", topics_in(&body, false)))
                        .unwrap(),
                    _ => {}
                }
            }
        });

        tx.send(Cmd::Window(Some((
            14_074_000 - 300_000,
            14_074_000 + 300_000,
        ))))
        .unwrap();
        tx.send(Cmd::Configure {
            rbn: None,
            dx_cluster: None,
            psk_reporter: Some(mqtt_cfg(port)),
        })
        .unwrap();
        assert_eq!(
            seen_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            ("subscribe", vec!["pskr/filter/v2/20m/#".to_string()]),
            "exactly the band the VFO is on"
        );
        wait("the spot reaches the store", || {
            !store.lock().unwrap().is_empty()
        });
        assert_eq!(
            store.lock().unwrap().spots()[0].network,
            Network::PskReporter
        );
        assert_eq!(store.lock().unwrap().spots()[0].call, "AA1AAA");

        // Retune to 40 m: the 20 m topic goes, the 40 m topic comes.
        tx.send(Cmd::Window(Some((
            7_040_000 - 300_000,
            7_040_000 + 300_000,
        ))))
        .unwrap();
        let mut got = vec![seen_rx.recv_timeout(Duration::from_secs(5)).unwrap()];
        got.push(seen_rx.recv_timeout(Duration::from_secs(5)).unwrap());
        assert!(
            got.contains(&("unsubscribe", vec!["pskr/filter/v2/20m/#".to_string()])),
            "{got:?}"
        );
        assert!(
            got.contains(&("subscribe", vec!["pskr/filter/v2/40m/#".to_string()])),
            "{got:?}"
        );

        // No radio (an empty window): everything is dropped, so nothing is subscribed to.
        tx.send(Cmd::Window(Some((1, 0)))).unwrap();
        let last = seen_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            last,
            ("unsubscribe", vec!["pskr/filter/v2/40m/#".to_string()])
        );
        assert!(
            seen_rx.recv_timeout(Duration::from_millis(600)).is_err(),
            "and no subscription is made for an empty window"
        );
    }

    /// FR-SPOT-05: which topics a window asks for.
    /// trace: FR-SPOT-05
    #[test]
    fn fr_spot_05_topics_follow_the_window() {
        assert_eq!(
            topics_for(Some((13_774_000, 14_374_000))),
            ["pskr/filter/v2/20m/#"]
        );
        assert_eq!(
            topics_for(Some((9_900_000, 14_500_000))),
            ["pskr/filter/v2/30m/#", "pskr/filter/v2/20m/#"]
        );
        assert!(
            topics_for(Some((1, 0))).is_empty(),
            "an empty window needs no band"
        );
        assert!(
            topics_for(None).is_empty(),
            "no window means no band, never the whole feed"
        );
        assert!(topics_for(Some((30_000_000, 40_000_000))).is_empty());
    }

    /// FR-SPOT-09: the wording shown for each state.
    /// trace: FR-SPOT-09
    #[test]
    fn fr_spot_09_status_wording() {
        let mk = |state, error: Option<&str>, spots, shed| Status {
            state,
            error: error.map(str::to_string),
            stats: Stats {
                spots,
                shed,
                ..Stats::default()
            },
        };
        assert_eq!(
            describe(&mk(ConnState::Connected, None, 42, 0)),
            ("connected — 42 spots".into(), false)
        );
        assert_eq!(
            describe(&mk(ConnState::Connected, None, 42, 7)).0,
            "connected — 42 spots, 7 shed"
        );
        assert_eq!(
            describe(&mk(ConnState::AwaitingLogin, None, 0, 0)),
            ("connected, logging in…".into(), false)
        );
        assert_eq!(
            describe(&mk(ConnState::Disconnected, None, 0, 0)),
            ("connecting…".into(), false)
        );
        // An error is shown as a problem, with its reason.
        assert_eq!(
            describe(&mk(
                ConnState::Disconnected,
                Some("a login callsign is required"),
                0,
                0
            )),
            ("a login callsign is required".into(), true)
        );
        // A stale error does not hide a working connection.
        assert!(!describe(&mk(ConnState::Connected, Some("old"), 1, 0)).1);
    }
}
