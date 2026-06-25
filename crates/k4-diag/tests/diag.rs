//! Diagnostic-log tests. trace: FR-DIAG-01
use k4_diag::{DiagLog, Level};

/// Records at or above the minimum level are kept and formatted; lower ones drop.
///
/// trace: FR-DIAG-01
#[test]
fn fr_diag_01_level_filtering_and_format() {
    let mut log = DiagLog::new(10, Level::Info);
    log.log(Level::Debug, "rx", "noisy"); // below Info → dropped
    log.log(Level::Info, "net", "connected to host:9205");
    log.log(Level::Warn, "net", "link lost");

    let lines = log.recent(10);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "[INFO] net: connected to host:9205");
    assert_eq!(lines[1], "[WARN] net: link lost");
}

/// The oldest lines are evicted past capacity.
///
/// trace: FR-DIAG-01
#[test]
fn fr_diag_01_bounded_capacity() {
    let mut log = DiagLog::new(2, Level::Debug);
    log.log(Level::Info, "a", "1");
    log.log(Level::Info, "a", "2");
    log.log(Level::Info, "a", "3");

    let lines = log.recent(10);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "[INFO] a: 2");
    assert_eq!(lines[1], "[INFO] a: 3");
}

/// `recent(n)` returns the last n lines, oldest first.
///
/// trace: FR-DIAG-01
#[test]
fn fr_diag_01_recent_window() {
    let mut log = DiagLog::new(10, Level::Debug);
    for i in 0..5 {
        log.log(Level::Debug, "x", &i.to_string());
    }
    assert_eq!(log.recent(2), vec!["[DEBUG] x: 3", "[DEBUG] x: 4"]);
}
