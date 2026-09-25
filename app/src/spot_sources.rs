//! The worker thread that runs the spot sources — the Reverse Beacon Network, a DX cluster (both
//! telnet), PSK Reporter (MQTT), POTA (a polled HTTP list) and FreeDV Reporter (a WebSocket) —
//! keeps the spot store fed, and
//! reports what each is doing (FR-SPOT-05/07/08/09).
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
use std::time::{Duration, Instant};

use k4_spot::freedv_source::{FreeDvConfig, FreeDvSource};
use k4_spot::mqtt_source::{CertInfo, MqttConfig, MqttSource};
use k4_spot::polled::{PolledConfig, PolledSource};
use k4_spot::pota;
use k4_spot::psk::{bands_overlapping, topic_for_band};
use k4_spot::telnet::{ConnState, Stats, TelnetConfig, TelnetSource};
use k4_spot::{Network, SourceError, Spot, SpotSource};

use crate::spots::SpotHandle;
use crate::tls;

/// What one source is doing.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: ConnState,
    /// The most recent failure, kept until the source is connected again.
    pub error: Option<String>,
    pub stats: Stats,
    /// A source that is asked now and then (POTA) rather than held open, so "connected" would be
    /// the wrong word for it.
    pub polled: bool,
    /// The server's certificate, when that is why it is not connected: the operator is asked
    /// whether to trust it (FR-SPOT-13).
    pub cert: Option<CertInfo>,
}

/// Each source's status; `None` = not enabled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Statuses {
    pub rbn: Option<Status>,
    pub dx_cluster: Option<Status>,
    pub psk_reporter: Option<Status>,
    pub pota: Option<Status>,
    pub freedv: Option<Status>,
}

pub type StatusHandle = Arc<Mutex<Statuses>>;

/// Instructions from the UI.
// `Configure` carries one config per network and the other commands carry almost nothing. A command is
// sent only when a setting changes — a few times a session — so the size gap costs nothing, and
// boxing it would only add a deref to every place a configuration is built.
#[allow(clippy::large_enum_variant)]
pub enum Cmd {
    /// Run these sources (`None` = off). A source is restarted only when its settings change.
    Configure {
        rbn: Option<TelnetConfig>,
        dx_cluster: Option<TelnetConfig>,
        psk_reporter: Option<MqttConfig>,
        pota: Option<PolledConfig>,
        freedv: Option<FreeDvConfig>,
    },
    /// Keep only spots inside `[lo, hi]` Hz, and (for PSK Reporter) subscribe to the bands that
    /// overlap it. An empty range (`lo > hi`) keeps none and subscribes to none; `None` keeps
    /// every telnet spot but subscribes to no PSK Reporter band.
    Window(Option<(u64, u64)>),
    /// Try again now instead of waiting out the backoff — sent after the operator approves a
    /// certificate.
    RetryNow,
}

#[derive(Debug, Clone, PartialEq)]
enum FeedConfig {
    Telnet(TelnetConfig),
    Mqtt(MqttConfig),
    Polled(PolledConfig),
    FreeDv(FreeDvConfig),
}

enum Feed {
    Telnet(TelnetSource),
    Mqtt(MqttSource),
    Polled(PolledSource),
    FreeDv(FreeDvSource),
}

impl Feed {
    /// `None` for a polled network no reply parser exists for — nothing is run rather than
    /// something that could only fail.
    fn new(cfg: &FeedConfig, pins: &tls::Pins) -> Option<Self> {
        Some(match cfg {
            FeedConfig::Telnet(c) => Feed::Telnet(TelnetSource::new(c.clone())),
            FeedConfig::Mqtt(c) => {
                let mut source = MqttSource::new(c.clone());
                source.set_tls_connector(tls::connector(std::sync::Arc::clone(pins)));
                Feed::Mqtt(source)
            }
            FeedConfig::Polled(c) => {
                let parse = match c.network {
                    Network::Pota => pota::parse_spots,
                    _ => return None,
                };
                Feed::Polled(PolledSource::new(
                    c.clone(),
                    crate::http_fetch::fetcher(),
                    parse,
                ))
            }
            FeedConfig::FreeDv(c) => Feed::FreeDv(FreeDvSource::new(c.clone())),
        })
    }

    fn poll(&mut self, sink: &mut dyn FnMut(Spot)) -> Result<(), SourceError> {
        match self {
            Feed::Telnet(s) => s.poll(sink),
            Feed::Mqtt(s) => s.poll(sink),
            Feed::Polled(s) => s.poll(sink),
            Feed::FreeDv(s) => s.poll(sink),
        }
    }

    fn set_window(&mut self, w: Option<(u64, u64)>) {
        match self {
            Feed::Telnet(s) => s.set_window(w),
            Feed::Mqtt(s) => s.set_window(w),
            Feed::Polled(s) => s.set_window(w),
            Feed::FreeDv(s) => s.set_window(w),
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
            Feed::Polled(s) => s.state(),
            Feed::FreeDv(s) => s.state(),
        }
    }

    /// The certificate the last attempt was refused for, if that is why it is down.
    fn pending_cert(&self) -> Option<CertInfo> {
        match self {
            Feed::Mqtt(s) => s.pending_cert().cloned(),
            _ => None,
        }
    }

    fn retry_now(&mut self) {
        if let Feed::Mqtt(s) = self {
            s.retry_now();
        }
    }

    fn stats(&self) -> Stats {
        match self {
            Feed::Telnet(s) => s.stats(),
            Feed::Mqtt(s) => s.stats(),
            Feed::Polled(s) => s.stats(),
            Feed::FreeDv(s) => s.stats(),
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
            polled: matches!(src, Feed::Polled(_)),
            cert: src.pending_cert(),
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
pub fn spawn(rx: Receiver<Cmd>, store: SpotHandle, status: StatusHandle, pins: tls::Pins) {
    let _ = thread::Builder::new()
        .name("spot-sources".into())
        .spawn(move || run(&rx, &store, &status, &pins));
}

fn run(rx: &Receiver<Cmd>, store: &SpotHandle, status: &StatusHandle, pins: &tls::Pins) {
    let mut slots = [
        Slot::default(),
        Slot::default(),
        Slot::default(),
        Slot::default(),
        Slot::default(),
    ];
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
                    pota,
                    freedv,
                }) => {
                    let wanted = [
                        rbn.map(FeedConfig::Telnet),
                        dx_cluster.map(FeedConfig::Telnet),
                        psk_reporter.map(FeedConfig::Mqtt),
                        pota.map(FeedConfig::Polled),
                        freedv.map(FeedConfig::FreeDv),
                    ];
                    for (slot, want) in slots.iter_mut().zip(wanted) {
                        if slot.cfg != want {
                            slot.error = None;
                            slot.src =
                                want.as_ref().and_then(|c| Feed::new(c, pins)).map(|mut s| {
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
                Ok(Cmd::RetryNow) => {
                    for src in slots.iter_mut().filter_map(|s| s.src.as_mut()) {
                        src.retry_now();
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }

        let started = Instant::now();
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
            pota: slots[3].status(),
            freedv: slots[4].status(),
        };
        if now != published {
            if let Ok(mut g) = status.lock() {
                *g = now.clone();
            }
            published = now;
        }
        thread::sleep(pace(active, started.elapsed()));
    }
}

/// How long the worker rests at the end of a pass. With nothing to run, a long rest. With sources
/// running, only enough to make a pass last [`MIN_PASS`]: a telnet or MQTT source already spends
/// its 20 ms socket read inside `poll`, but a polled source returns at once, and without a rest a
/// worker running only POTA would spin a whole core.
fn pace(active: bool, spent: Duration) -> Duration {
    if active {
        MIN_PASS.saturating_sub(spent)
    } else {
        IDLE_REST
    }
}

/// The least one pass over the running sources takes.
const MIN_PASS: Duration = Duration::from_millis(10);
/// The rest when no source is running.
const IDLE_REST: Duration = Duration::from_millis(100);

/// One line describing a source, and whether it is a problem worth colouring.
pub fn describe(status: &Status) -> (String, bool) {
    match (status.state, &status.error) {
        (ConnState::Connected, _) if status.polled => (
            format!("last request succeeded — {} spots", status.stats.spots),
            false,
        ),
        (ConnState::Disconnected, None) if status.polled => {
            ("waiting for the first reply…".into(), false)
        }
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
    /// FR-SPOT-14: no hostname is resolved unbounded anywhere a spot source connects from — every
    /// k4-spot module, the TLS connector, the HTTP fetch and this worker. `to_socket_addrs` has no
    /// timeout of its own, and all these run on (or block) the one thread every network is polled
    /// from, so one hung resolver would stall them all. Resolution goes through
    /// `k4_spot::dns::resolve_bounded`, the one place a bare call is allowed. Structural and
    /// **banning the construct**, so a new connector cannot quietly add one back: the TLS connector
    /// did exactly that after the fix for the plain ones. This module is sliced off this file so
    /// the needles below do not find themselves.
    /// trace: FR-SPOT-14
    #[test]
    fn fr_spot_14_no_spot_connection_resolves_unbounded() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        // Without a leading dot, so the fully qualified form (`ToSocketAddrs::to_socket_addrs(&a)`)
        // is caught as well as the method call.
        let needles = [concat!("to_socket", "_addrs("), concat!("lookup", "_host(")];
        let mut scanned: Vec<(String, String)> = Vec::new();
        let spot_src = root.join("crates/k4-spot/src");
        for entry in std::fs::read_dir(&spot_src).expect("k4-spot sources") {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                scanned.push((name, std::fs::read_to_string(&path).unwrap()));
            }
        }
        for rel in ["app/src/tls/mod.rs", "app/src/http_fetch.rs"] {
            scanned.push((rel.into(), std::fs::read_to_string(root.join(rel)).unwrap()));
        }
        let this = include_str!("spot_sources.rs");
        let this = &this[..this
            .find(concat!("mod tests", " {"))
            .expect("this test module")];
        scanned.push(("app/src/spot_sources.rs".into(), this.to_string()));

        // The scan reaches what it claims: every connector's file is in it, and the needle does
        // find the one allowed call — so a miss below is a real absence, not a broken search.
        for must in [
            "dns.rs",
            "telnet.rs",
            "mqtt_source.rs",
            "freedv_source.rs",
            "app/src/tls/mod.rs",
        ] {
            assert!(
                scanned.iter().any(|(n, _)| n == must),
                "{must} was not scanned"
            );
        }
        let dns = &scanned.iter().find(|(n, _)| n == "dns.rs").unwrap().1;
        assert!(
            dns.contains(needles[0]),
            "the needle does not match the allowed call"
        );

        for (name, text) in scanned.iter().filter(|(n, _)| n != "dns.rs") {
            for needle in needles {
                assert!(
                    !text.contains(needle),
                    "{name} resolves a hostname unbounded (`{needle}`); use \
                     k4_spot::dns::resolve_bounded"
                );
            }
        }
    }

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
            tls: false,
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
        spawn(rx, Arc::clone(&store), Arc::clone(&status), Arc::default());

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
            pota: None,
            freedv: None,
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
            pota: None,
            freedv: None,
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
        spawn(rx, Arc::clone(&store), Arc::clone(&status), Arc::default());

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
            pota: None,
            freedv: None,
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
            polled: false,
            cert: None,
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

    /// A mock HTTP server: answers every request with `status` and `body`, then closes.
    fn mock_http(status: &'static str, body: String) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut s) = conn else { return };
                let mut buf = [0u8; 2048];
                let mut req = Vec::new();
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let reply = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = s.write_all(reply.as_bytes());
                let _ = s.flush();
                thread::sleep(Duration::from_millis(50));
            }
        });
        port
    }

    fn pota_cfg(port: u16) -> PolledConfig {
        PolledConfig {
            network: Network::Pota,
            url: format!("http://127.0.0.1:{port}/spot/activator"),
            interval: Duration::from_secs(60),
            max_body: pota::MAX_BODY,
        }
    }

    fn pota_body() -> String {
        let rec = |call: &str, khz: &str| {
            format!(
                r#"{{"activator":"{call}","frequency":"{khz}","mode":"FT8","reference":"CA-0040","spotTime":"2026-09-21T05:07:00","spotter":"BB2BBB","comments":"CQ POTA","invalid":null}}"#
            )
        };
        format!(
            "[{},{},{}]",
            rec("aa1aaa", "14074.0"), // inside the window
            rec("cc3ccc", "7030.0"),  // outside it
            rec("CQ", "14075.0")      // not a callsign
        )
    }

    /// FR-SPOT-08/09: the worker runs POTA from an HTTP list into the store (window-filtered, bad
    /// records counted), a failing POTA is reported against its own network while another source
    /// keeps delivering, and switching POTA off removes its status.
    /// trace: FR-SPOT-08, FR-SPOT-09
    #[test]
    fn fr_spot_08_worker_runs_pota_and_isolates_its_failure() {
        let store: SpotHandle = Arc::default();
        let status: StatusHandle = Arc::default();
        let (tx, rx) = mpsc::channel();
        spawn(rx, Arc::clone(&store), Arc::clone(&status), Arc::default());
        tx.send(Cmd::Window(Some((14_000_000, 14_100_000))))
            .unwrap();

        // A healthy POTA feed.
        let good = mock_http("200 OK", pota_body());
        tx.send(Cmd::Configure {
            rbn: None,
            dx_cluster: None,
            psk_reporter: None,
            pota: Some(pota_cfg(good)),
            freedv: None,
        })
        .unwrap();
        wait("the POTA spot reaches the store", || {
            store.lock().unwrap().spots().len() == 1
        });
        {
            let st = store.lock().unwrap();
            let spot = &st.spots()[0];
            assert_eq!((spot.call.as_str(), spot.freq_hz), ("AA1AAA", 14_074_000));
            assert_eq!(spot.network, Network::Pota);
            assert_eq!(spot.comment.as_deref(), Some("CA-0040 CQ POTA"));
        }
        wait("POTA reports success", || {
            status
                .lock()
                .unwrap()
                .pota
                .as_ref()
                .is_some_and(|p| p.state == ConnState::Connected && p.stats.connects == 1)
        });
        let p = status.lock().unwrap().pota.clone().unwrap();
        assert!(p.polled);
        assert_eq!(p.error, None);
        assert_eq!(p.stats.spots, 1);
        assert_eq!(p.stats.outside_window, 1);
        assert_eq!(p.stats.rejected, 1);
        assert_eq!(
            describe(&p),
            ("last request succeeded — 1 spots".into(), false)
        );

        // POTA fails while RBN is healthy: the failure is POTA's alone.
        let bad = mock_http("503 Service Unavailable", String::new());
        let cluster = mock_cluster("DX de K1TTT-#: 14074.0 W1AW CW 30 dB 20 WPM CQ 1200Z\r\n");
        tx.send(Cmd::Configure {
            rbn: Some(cfg(Network::Rbn, cluster)),
            dx_cluster: None,
            psk_reporter: None,
            pota: Some(pota_cfg(bad)),
            freedv: None,
        })
        .unwrap();
        wait("POTA's failure is reported", || {
            status
                .lock()
                .unwrap()
                .pota
                .as_ref()
                .is_some_and(|p| p.error.is_some())
        });
        wait("RBN delivers meanwhile", || {
            status
                .lock()
                .unwrap()
                .rbn
                .as_ref()
                .is_some_and(|r| r.state == ConnState::Connected && r.stats.spots >= 1)
        });
        let s = status.lock().unwrap().clone();
        let p = s.pota.unwrap();
        assert_eq!(p.error.as_deref(), Some("the server returned HTTP 503"));
        assert_eq!(p.state, ConnState::Disconnected);
        assert_eq!(describe(&p), ("the server returned HTTP 503".into(), true));
        assert_eq!(s.rbn.unwrap().error, None, "RBN is unaffected");

        // Off means gone.
        tx.send(Cmd::Configure {
            rbn: None,
            dx_cluster: None,
            psk_reporter: None,
            pota: None,
            freedv: None,
        })
        .unwrap();
        wait("every source is stopped", || {
            let s = status.lock().unwrap();
            s.pota.is_none() && s.rbn.is_none()
        });
    }

    /// FR-SPOT-08: the worker rests between passes when only a source that returns at once is
    /// running (otherwise it would spin a core), and rests longer when nothing runs.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_worker_pacing() {
        let ms = Duration::from_millis;
        assert_eq!(pace(false, ms(0)), ms(100), "nothing running: a long rest");
        assert_eq!(pace(false, ms(500)), ms(100));
        assert_eq!(pace(true, ms(0)), ms(10), "an instant pass still rests");
        assert_eq!(pace(true, ms(4)), ms(6), "a pass tops up to the minimum");
        assert_eq!(pace(true, ms(10)), ms(0));
        assert_eq!(pace(true, ms(25)), ms(0), "a slow pass adds no rest");
    }

    /// FR-SPOT-08: a polled source is described as requests, not as a connection.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_polled_status_wording() {
        let mk = |state, error: Option<&str>, spots| Status {
            state,
            error: error.map(str::to_string),
            stats: Stats {
                spots,
                ..Stats::default()
            },
            polled: true,
            cert: None,
        };
        assert_eq!(
            describe(&mk(ConnState::Disconnected, None, 0)),
            ("waiting for the first reply…".into(), false)
        );
        assert_eq!(
            describe(&mk(ConnState::Connected, None, 12)).0,
            "last request succeeded — 12 spots"
        );
        assert_eq!(
            describe(&mk(
                ConnState::Disconnected,
                Some("no reply within 30 s"),
                0
            )),
            ("no reply within 30 s".into(), true)
        );
        // The interval bounds here and in the settings file are the same.
        assert_eq!(
            k4_config::SPOT_POLL_MIN_SECS,
            k4_spot::polled::MIN_INTERVAL_SECS
        );
        assert_eq!(
            k4_config::SPOT_POLL_MAX_SECS,
            k4_spot::polled::MAX_INTERVAL_SECS
        );
        assert_eq!(
            k4_config::SPOT_POLL_DEFAULT_SECS,
            k4_spot::polled::DEFAULT_INTERVAL_SECS
        );
        // And so are FreeDV Reporter's refresh bounds (FR-SPOT-08).
        assert_eq!(
            (
                k4_config::SPOT_FREEDV_REFRESH_MIN_SECS,
                k4_config::SPOT_FREEDV_REFRESH_DEFAULT_SECS,
                k4_config::SPOT_FREEDV_REFRESH_MAX_SECS
            ),
            (
                k4_spot::freedv_source::MIN_REFRESH_SECS,
                k4_spot::freedv_source::DEFAULT_REFRESH_SECS,
                k4_spot::freedv_source::MAX_REFRESH_SECS
            )
        );
    }

    /// FR-SPOT-13: over TLS the worker refuses a certificate no authority signed, reports it with
    /// its fingerprint, and sends **nothing** to the server; once the operator approves it and the
    /// worker is told to retry, it connects and the pending certificate is gone.
    /// trace: FR-SPOT-13
    #[test]
    fn fr_spot_13_worker_asks_before_trusting_and_connects_once_approved() {
        use crate::tls::test_certs::{A_CERT, A_KEY, A_SHA256};
        use crate::tls::{testkit, Pin};
        use std::sync::atomic::{AtomicUsize, Ordering};

        // A TLS broker: answer the first packet (CONNECT) with a CONNACK and hold the line.
        let connects = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&connects);
        let port = testkit::serve_tls(
            A_CERT,
            A_KEY,
            rustls::DEFAULT_VERSIONS,
            4,
            Arc::new(move |mut tls| {
                let mut buf = [0u8; 256];
                if tls.read(&mut buf).is_ok_and(|n| n > 0) {
                    seen.fetch_add(1, Ordering::SeqCst);
                    let _ = tls.write_all(&[0x20, 0x02, 0x00, 0x00]);
                    let _ = tls.flush();
                }
                thread::sleep(Duration::from_secs(3));
            }),
        );

        let store: SpotHandle = Arc::default();
        let status: StatusHandle = Arc::default();
        let pins: tls::Pins = Arc::default();
        let (tx, rx) = mpsc::channel();
        spawn(
            rx,
            Arc::clone(&store),
            Arc::clone(&status),
            Arc::clone(&pins),
        );
        tx.send(Cmd::Window(Some((14_000_000, 14_100_000))))
            .unwrap();
        tx.send(Cmd::Configure {
            rbn: None,
            dx_cluster: None,
            psk_reporter: Some(MqttConfig {
                tls: true,
                ..mqtt_cfg(port)
            }),
            pota: None,
            freedv: None,
        })
        .unwrap();

        wait("the untrusted certificate is reported", || {
            status
                .lock()
                .unwrap()
                .psk_reporter
                .as_ref()
                .is_some_and(|p| p.cert.is_some())
        });
        let p = status.lock().unwrap().psk_reporter.clone().unwrap();
        let cert = p.cert.clone().expect("a pending certificate");
        assert_eq!(
            cert.sha256, A_SHA256,
            "the fingerprint shown is the server's"
        );
        assert_eq!((cert.host.as_str(), cert.port), ("127.0.0.1", port));
        assert!(!cert.changed);
        assert_eq!(p.state, ConnState::Disconnected);
        assert!(
            p.error
                .as_deref()
                .is_some_and(|e| e.contains("not trusted")),
            "{:?}",
            p.error
        );
        assert!(describe(&p).1, "shown as a problem");
        assert_eq!(
            connects.load(Ordering::SeqCst),
            0,
            "the broker was sent an MQTT packet before the certificate was approved"
        );

        // The operator approves that certificate; the worker is told to try again.
        pins.lock().unwrap().push(Pin {
            host: cert.host.clone(),
            port: cert.port,
            sha256: cert.sha256.clone(),
        });
        tx.send(Cmd::RetryNow).unwrap();
        // Well inside the 1 s the source would otherwise wait before trying again: the retry is
        // what makes an approval take effect at once.
        let t0 = Instant::now();
        while status
            .lock()
            .unwrap()
            .psk_reporter
            .as_ref()
            .is_none_or(|p| p.state != ConnState::Connected)
        {
            assert!(
                t0.elapsed() < Duration::from_millis(700),
                "not connected within 700 ms of the approval"
            );
            thread::sleep(Duration::from_millis(10));
        }
        let p = status.lock().unwrap().psk_reporter.clone().unwrap();
        assert!(p.cert.is_none(), "the pending certificate is cleared");
        assert_eq!(p.error, None);
        assert_eq!(connects.load(Ordering::SeqCst), 1);
    }

    /// A mock FreeDV Reporter: upgrade, Engine.IO open, read the client's connect frame, Socket.IO
    /// acknowledgement, then the given events, then hold the line. Frames are built by hand.
    fn mock_freedv(events: Vec<String>) -> u16 {
        fn frame(payload: &[u8]) -> Vec<u8> {
            let mut f = vec![0x81];
            match payload.len() {
                n if n < 126 => f.push(n as u8),
                n => {
                    f.push(126);
                    f.extend_from_slice(&(n as u16).to_be_bytes());
                }
            }
            f.extend_from_slice(payload);
            f
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let mut req = Vec::new();
            let mut b = [0u8; 1];
            while !req.ends_with(b"\r\n\r\n") {
                if s.read_exact(&mut b).is_err() {
                    return;
                }
                req.push(b[0]);
            }
            let req = String::from_utf8_lossy(&req).into_owned();
            let key = req
                .lines()
                .find_map(|l| l.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap_or("")
                .trim()
                .to_string();
            let _ = write!(
                s,
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                k4_spot::ws::accept_key(&key)
            );
            let _ = s.write_all(&frame(
                br#"0{"sid":"s","pingInterval":25000,"pingTimeout":20000}"#,
            ));
            // The client's connect frame: 2 header bytes, 4 mask bytes, the payload.
            let mut h = [0u8; 2];
            if s.read_exact(&mut h).is_err() {
                return;
            }
            let mut rest = vec![0u8; 4 + usize::from(h[1] & 0x7f)];
            if s.read_exact(&mut rest).is_err() {
                return;
            }
            let _ = s.write_all(&frame(br#"40{"sid":"me"}"#));
            for e in events {
                let _ = s.write_all(&frame(e.as_bytes()));
            }
            thread::sleep(Duration::from_secs(4));
        });
        port
    }

    fn freedv_cfg(port: u16) -> FreeDvConfig {
        FreeDvConfig {
            host: "127.0.0.1".into(),
            port,
            user_agent: "K4remote/test".into(),
            refresh_secs: k4_spot::freedv_source::DEFAULT_REFRESH_SECS,
        }
    }

    /// FR-SPOT-08/09: the worker runs FreeDV Reporter into the store (window-filtered), a failing
    /// FreeDV Reporter is reported against its own network while RBN keeps delivering, and
    /// switching it off removes its status.
    /// trace: FR-SPOT-08, FR-SPOT-09
    #[test]
    fn fr_spot_08_worker_runs_freedv_and_isolates_its_failure() {
        let store: SpotHandle = Arc::default();
        let status: StatusHandle = Arc::default();
        let (tx, rx) = mpsc::channel();
        spawn(rx, Arc::clone(&store), Arc::clone(&status), Arc::default());
        tx.send(Cmd::Window(Some((14_000_000, 14_100_000))))
            .unwrap();

        let port = mock_freedv(vec![
            r#"42["freq_change",{"sid":"a","freq":14074000,"callsign":"aa1aaa"}]"#.into(),
            r#"42["freq_change",{"sid":"b","freq":7177000,"callsign":"bb2bbb"}]"#.into(),
        ]);
        tx.send(Cmd::Configure {
            rbn: None,
            dx_cluster: None,
            psk_reporter: None,
            pota: None,
            freedv: Some(freedv_cfg(port)),
        })
        .unwrap();
        wait("the FreeDV spot reaches the store", || {
            store.lock().unwrap().spots().len() == 1
        });
        {
            let st = store.lock().unwrap();
            let spot = &st.spots()[0];
            assert_eq!((spot.call.as_str(), spot.freq_hz), ("AA1AAA", 14_074_000));
            assert_eq!(spot.network, Network::FreeDvReporter);
        }
        wait("FreeDV reports it is connected", || {
            status
                .lock()
                .unwrap()
                .freedv
                .as_ref()
                .is_some_and(|f| f.state == ConnState::Connected && f.stats.outside_window == 1)
        });
        let f = status.lock().unwrap().freedv.clone().unwrap();
        assert!(!f.polled, "a live feed, not a polled one");
        assert_eq!(f.error, None);
        assert_eq!(f.stats.spots, 1);
        assert_eq!(describe(&f), ("connected — 1 spots".into(), false));

        // FreeDV fails (nothing listening) while RBN is healthy: the failure is FreeDV's alone.
        let closed = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let cluster = mock_cluster("DX de K1TTT-#: 14074.0 W1AW CW 30 dB 20 WPM CQ 1200Z\r\n");
        tx.send(Cmd::Configure {
            rbn: Some(cfg(Network::Rbn, cluster)),
            dx_cluster: None,
            psk_reporter: None,
            pota: None,
            freedv: Some(freedv_cfg(closed)),
        })
        .unwrap();
        wait("FreeDV's failure is reported", || {
            status
                .lock()
                .unwrap()
                .freedv
                .as_ref()
                .is_some_and(|f| f.error.is_some())
        });
        wait("RBN delivers meanwhile", || {
            status
                .lock()
                .unwrap()
                .rbn
                .as_ref()
                .is_some_and(|r| r.state == ConnState::Connected && r.stats.spots >= 1)
        });
        let s = status.lock().unwrap().clone();
        let f = s.freedv.unwrap();
        assert_eq!(f.state, ConnState::Disconnected);
        assert!(
            f.error
                .as_deref()
                .is_some_and(|e| e.contains("connect to 127.0.0.1")),
            "{:?}",
            f.error
        );
        assert!(describe(&f).1, "shown as a problem");
        assert_eq!(s.rbn.unwrap().error, None, "RBN is unaffected");

        // Off means gone.
        tx.send(Cmd::Configure {
            rbn: None,
            dx_cluster: None,
            psk_reporter: None,
            pota: None,
            freedv: None,
        })
        .unwrap();
        wait("every source is stopped", || {
            let s = status.lock().unwrap();
            s.freedv.is_none() && s.rbn.is_none()
        });
    }
}
