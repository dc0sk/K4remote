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

/// FR-CAT-02: input with no `;` for more than the 64 KiB bound is discarded **up to the next
/// `;`** — the decoder then resynchronises, so the command after the junk arrives whole, however
/// the junk was split across reads (one read or many small ones).
/// trace: FR-CAT-02
#[test]
fn fr_cat_02_resynchronises_after_oversized_junk() {
    for chunk in [70_000usize, 4096, 1000] {
        let mut d = LineDecoder::new();
        let mut got = Vec::new();
        let junk = vec![b'X'; 70_000];
        for piece in junk.chunks(chunk) {
            got.extend(d.push(piece));
        }
        got.extend(d.push(b"tail;FA;ID;"));
        assert_eq!(
            got,
            vec!["FA;".to_string(), "ID;".to_string()],
            "chunk {chunk}"
        );
    }
    // Below the bound nothing is discarded.
    let mut d = LineDecoder::new();
    let mut long = vec![b'Y'; 1000];
    long.push(b';');
    assert_eq!(d.push(&long).len(), 1);
}
