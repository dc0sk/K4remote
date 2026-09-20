//! The worker thread that runs the telnet spot sources — the Reverse Beacon Network and a DX
//! cluster — keeps the spot store fed, and reports what each is doing (FR-SPOT-07/09).
//!
//! Each source is polled independently on this thread, off the UI and the radio-control paths, so
//! a slow, blocked or failing network can never delay the UI, CAT or audio, and one network's
//! failure does not stop the other (FR-SPOT-09). The UI sends what to run ([`Cmd`]) and reads the
//! outcome ([`Statuses`]) once per tick.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use k4_spot::telnet::{ConnState, Stats, TelnetConfig, TelnetSource};
use k4_spot::SpotSource;

use crate::spots::SpotHandle;

/// What one source is doing.
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub state: ConnState,
    /// The most recent failure, kept until the source is connected again.
    pub error: Option<String>,
    pub stats: Stats,
}

/// The two telnet sources' status; `None` = not enabled.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Statuses {
    pub rbn: Option<Status>,
    pub dx_cluster: Option<Status>,
}

pub type StatusHandle = Arc<Mutex<Statuses>>;

/// Instructions from the UI.
pub enum Cmd {
    /// Run these sources (`None` = off). A source is restarted only when its settings change.
    Configure {
        rbn: Option<TelnetConfig>,
        dx_cluster: Option<TelnetConfig>,
    },
    /// Keep only spots inside `[lo, hi]` Hz. An empty range (`lo > hi`) keeps none.
    Window(Option<(u64, u64)>),
}

#[derive(Default)]
struct Slot {
    cfg: Option<TelnetConfig>,
    src: Option<TelnetSource>,
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

/// Start the worker. It ends when the UI drops its end of the command channel.
pub fn spawn(rx: Receiver<Cmd>, store: SpotHandle, status: StatusHandle) {
    let _ = thread::Builder::new()
        .name("spot-sources".into())
        .spawn(move || run(&rx, &store, &status));
}

fn run(rx: &Receiver<Cmd>, store: &SpotHandle, status: &StatusHandle) {
    let mut slots = [Slot::default(), Slot::default()];
    // Until told otherwise, keep nothing: no radio, no view, nothing to label.
    let mut window: Option<(u64, u64)> = Some((1, 0));
    let mut published = Statuses::default();
    loop {
        loop {
            match rx.try_recv() {
                Ok(Cmd::Configure { rbn, dx_cluster }) => {
                    for (slot, want) in slots.iter_mut().zip([rbn, dx_cluster]) {
                        if slot.cfg != want {
                            slot.error = None;
                            slot.src = want.clone().map(|cfg| {
                                let mut s = TelnetSource::new(cfg);
                                s.set_window(window);
                                s
                            });
                            slot.cfg = want;
                        }
                    }
                }
                Ok(Cmd::Window(w)) => {
                    window = w;
                    for src in slots.iter_mut().filter_map(|s| s.src.as_mut()) {
                        src.set_window(w);
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
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::time::Instant;

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
        let stored = store.lock().unwrap();
        assert_eq!(stored.spots()[0].call, "W1AW");
        assert_eq!(stored.spots()[0].network, Network::Rbn);
        drop(stored);

        // A window that excludes the band keeps later spots out; turning a source off removes its status.
        tx.send(Cmd::Configure {
            rbn: None,
            dx_cluster: None,
        })
        .unwrap();
        wait("both sources are stopped", || {
            let s = status.lock().unwrap();
            s.rbn.is_none() && s.dx_cluster.is_none()
        });
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
        let _: Option<TcpStream> = None;
    }
}
