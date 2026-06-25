//! Serial CAT line-decoder tests. trace: FR-CAT-02
use k4_protocol::cat::LineDecoder;

/// Multiple `;`-terminated commands in one chunk split correctly (terminator kept).
///
/// trace: FR-CAT-02
#[test]
fn fr_cat_02_splits_multiple_commands() {
    let mut d = LineDecoder::new();
    assert_eq!(
        d.push(b"FA00014074000;MD3;"),
        vec!["FA00014074000;", "MD3;"]
    );
}

/// A command split across reads is reassembled; the partial tail is retained.
///
/// trace: FR-CAT-02
#[test]
fn fr_cat_02_reassembles_across_reads() {
    let mut d = LineDecoder::new();
    assert!(d.push(b"FA0001").is_empty()); // no terminator yet
    assert_eq!(d.push(b"4074000;"), vec!["FA00014074000;"]);
}

/// A trailing partial command stays buffered until its terminator arrives.
///
/// trace: FR-CAT-02
#[test]
fn fr_cat_02_keeps_partial_tail() {
    let mut d = LineDecoder::new();
    assert_eq!(d.push(b"MD3;FA"), vec!["MD3;"]);
    assert_eq!(d.push(b"7;"), vec!["FA7;"]);
}
