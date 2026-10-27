//! The persisted queue (FR-LOG-04; design §4): a directory with one file per QSO.
//!
//! Every QSO is written here — and flushed to disk — before the form says "queued", and removed
//! only when Wavelog has it. One file per entry means no shared rewrite: two K4 Remote instances
//! adding at once, or a crash in the middle of a write, cannot lose another entry; a torn write
//! leaves only a `.tmp`, which is ignored and cleaned up. Only the holder of the sender lock sends.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::qso::Qso;
use crate::reply::LastAttempt;

/// The entry format's version.
pub const VERSION: u32 = 1;
/// The most entries (pending + held) the queue takes; past that, logging refuses (never drops).
pub const MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Waiting to be sent.
    Pending,
    /// Kept aside for the operator, with a reason.
    Held,
}

/// One queued QSO and where it goes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub v: u32,
    pub id: String,
    /// When it was logged (Unix seconds) — also its place in the queue.
    pub queued_at: u64,
    /// The server it was logged for (normalised base).
    pub url: String,
    pub station_profile_id: u32,
    pub qso: Qso,
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default)]
    pub last_attempt: LastAttempt,
}

impl Entry {
    fn file_name(&self) -> String {
        format!("{:010}-{}.json", self.queued_at, self.id)
    }
}

/// Why an entry could not be queued.
#[derive(Debug)]
pub enum QueueError {
    /// [`MAX_ENTRIES`] reached.
    Full,
    Io(io::Error),
}

impl std::fmt::Display for QueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(
                f,
                "the log queue holds {MAX_ENTRIES} QSOs — send or save the held ones as ADIF first"
            ),
            Self::Io(e) => write!(f, "could not write the log queue: {e}"),
        }
    }
}

impl From<io::Error> for QueueError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// The queue directory.
#[derive(Debug, Clone)]
pub struct Queue {
    dir: PathBuf,
}

/// Held while sending; dropped, it lets another instance send.
#[derive(Debug)]
pub struct SenderLock(#[allow(dead_code)] File);

impl Queue {
    /// Open (create) the queue in `dir`, removing `.tmp` files a crash left behind.
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        for e in fs::read_dir(&dir)?.flatten() {
            if e.path().extension().is_some_and(|x| x == "tmp") {
                let _ = fs::remove_file(e.path());
            }
        }
        Ok(Self { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn entry_files(&self) -> io::Result<Vec<PathBuf>> {
        let mut files: Vec<PathBuf> = fs::read_dir(&self.dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        Ok(files)
    }

    /// Write `entry` to its file atomically: a `.tmp`, flushed, renamed, and the directory flushed.
    fn write(&self, entry: &Entry) -> io::Result<()> {
        let path = self.dir.join(entry.file_name());
        let tmp = path.with_extension("tmp");
        {
            let mut f = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&tmp)?;
            f.write_all(&serde_json::to_vec_pretty(entry).map_err(io::Error::other)?)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        // The rename is durable only once the directory is.
        if let Ok(d) = File::open(&self.dir) {
            let _ = d.sync_all();
        }
        Ok(())
    }

    /// Queue a QSO for `url` and station location `station_profile_id`; returns the entry once
    /// it is on disk.
    pub fn add(
        &self,
        url: &str,
        station_profile_id: u32,
        qso: Qso,
        now: u64,
    ) -> Result<Entry, QueueError> {
        if self.entry_files()?.len() >= MAX_ENTRIES {
            return Err(QueueError::Full);
        }
        let id: String = (0..16)
            .map(|_| format!("{:x}", rand::thread_rng().gen_range(0..16u8)))
            .collect();
        let entry = Entry {
            v: VERSION,
            id,
            queued_at: now,
            url: url.to_string(),
            station_profile_id,
            qso,
            state: State::Pending,
            reason: None,
            attempts: 0,
            last_attempt: LastAttempt::NotSent,
        };
        self.write(&entry)?;
        Ok(entry)
    }

    /// Every readable entry in queue order. A file that does not parse, or has another version,
    /// is renamed `….unreadable` and reported, never deleted.
    pub fn entries(&self) -> io::Result<(Vec<Entry>, Vec<PathBuf>)> {
        let mut good = Vec::new();
        let mut set_aside = Vec::new();
        for path in self.entry_files()? {
            let parsed = fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Entry>(&b).ok())
                .filter(|e| e.v == VERSION);
            match parsed {
                Some(e) => good.push(e),
                None => {
                    let aside = path.with_extension("json.unreadable");
                    fs::rename(&path, &aside)?;
                    set_aside.push(aside);
                }
            }
        }
        Ok((good, set_aside))
    }

    /// Store a changed entry (state, reason, attempts) in place, atomically.
    pub fn update(&self, entry: &Entry) -> io::Result<()> {
        self.write(entry)
    }

    /// Remove an entry Wavelog has (or the operator discarded).
    pub fn remove(&self, entry: &Entry) -> io::Result<()> {
        match fs::remove_file(self.dir.join(entry.file_name())) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// The sender lock, if no other instance holds it.
    pub fn try_lock_sender(&self) -> io::Result<Option<SenderLock>> {
        let f = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join(".lock"))?;
        match f.try_lock() {
            Ok(()) => Ok(Some(SenderLock(f))),
            Err(fs::TryLockError::WouldBlock) => Ok(None),
            Err(fs::TryLockError::Error(e)) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qso::{DigitalMode, Draft};
    use k4_protocol::state::Mode;

    fn qso(call: &str) -> Qso {
        Qso::from_draft(&Draft {
            call: call.into(),
            rst_sent: "59".into(),
            rst_rcvd: "59".into(),
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

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("k4-log-queue-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    /// FR-LOG-04: entries are added on disk, listed in queue order, updated in place and removed;
    /// a reopened queue sees the same entries (a restart loses nothing).
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_entries_survive_and_keep_their_order() {
        let dir = scratch("order");
        let q = Queue::open(&dir).unwrap();
        let a = q.add("https://log.example", 1, qso("DL1AA"), 100).unwrap();
        let b = q.add("https://log.example", 1, qso("DL2BB"), 101).unwrap();
        let mut held = b.clone();
        held.state = State::Held;
        held.reason = Some("why".into());
        held.last_attempt = LastAttempt::Unknown;
        q.update(&held).unwrap();
        let (list, aside) = Queue::open(&dir).unwrap().entries().unwrap();
        assert!(aside.is_empty());
        assert_eq!(
            list,
            vec![a.clone(), held.clone()],
            "queue order, and the update kept"
        );
        q.remove(&a).unwrap();
        q.remove(&a).unwrap(); // removing twice is not an error
        assert_eq!(q.entries().unwrap().0, vec![held]);
        let _ = fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: a torn write (a `.tmp` left by a crash) loses nothing confirmed and is cleaned
    /// up; an unreadable or future-version file is set aside on its own, not deleted, and the
    /// others still load.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_torn_and_unreadable_files() {
        let dir = scratch("torn");
        let q = Queue::open(&dir).unwrap();
        let kept = q.add("https://log.example", 1, qso("DL1AA"), 100).unwrap();
        fs::write(dir.join("0000000101-deadbeef.tmp"), b"{\"v\":1,\"id\":").unwrap();
        fs::write(dir.join("0000000102-garbage.json"), b"not json").unwrap();
        let mut future = serde_json::to_value(&kept).unwrap();
        future["v"] = 2.into();
        future["id"] = "future".into();
        fs::write(dir.join("0000000103-future.json"), future.to_string()).unwrap();
        let q = Queue::open(&dir).unwrap();
        assert!(
            !dir.join("0000000101-deadbeef.tmp").exists(),
            "the torn .tmp is cleaned up"
        );
        let (list, aside) = q.entries().unwrap();
        assert_eq!(list, vec![kept]);
        assert_eq!(aside.len(), 2);
        assert!(aside
            .iter()
            .all(|p| p.exists() && p.to_string_lossy().ends_with(".json.unreadable")));
        let _ = fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: two writers at once lose nothing (one file per entry); only one holds the
    /// sender lock, and it passes on when dropped.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_two_writers_one_sender() {
        let dir = scratch("writers");
        let (q1, q2) = (Queue::open(&dir).unwrap(), Queue::open(&dir).unwrap());
        let t1 = std::thread::spawn({
            let q = q1.clone();
            move || (0..100).for_each(|i| drop(q.add("u", 1, qso("DL1AA"), 1_000 + i).unwrap()))
        });
        let t2 = std::thread::spawn({
            let q = q2.clone();
            move || (0..100).for_each(|i| drop(q.add("u", 1, qso("DL2BB"), 1_000 + i).unwrap()))
        });
        t1.join().unwrap();
        t2.join().unwrap();
        assert_eq!(q1.entries().unwrap().0.len(), 200, "no entry lost");
        let lock = q1.try_lock_sender().unwrap();
        assert!(lock.is_some());
        assert!(
            q2.try_lock_sender().unwrap().is_none(),
            "one sender at a time"
        );
        drop(lock);
        assert!(
            q2.try_lock_sender().unwrap().is_some(),
            "the lock passes on"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// FR-LOG-04: at the bound, adding refuses (never drops an entry), with the way out named.
    /// trace: FR-LOG-04
    #[test]
    fn fr_log_04_the_queue_is_bounded() {
        assert_eq!(MAX_ENTRIES, 10_000);
        let dir = scratch("bound");
        let q = Queue::open(&dir).unwrap();
        // Fill with placeholder files (cheaper than 10 000 real entries): the bound counts files.
        for i in 0..MAX_ENTRIES {
            fs::write(dir.join(format!("{i:010}-x.json")), b"{}").unwrap();
        }
        let err = q.add("u", 1, qso("DL1AA"), 1).unwrap_err();
        assert!(matches!(err, QueueError::Full));
        assert!(err.to_string().contains("ADIF"), "{err}");
        fs::remove_file(dir.join(format!("{:010}-x.json", 0))).unwrap();
        assert!(
            q.add("u", 1, qso("DL1AA"), 1).is_ok(),
            "one below the bound is accepted"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
