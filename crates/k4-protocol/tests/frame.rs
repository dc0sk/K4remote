//! Frame-layer tests. trace: FR-STREAM-01, FR-STREAM-02, FR-STREAM-03
use k4_protocol::frame::{encode_frame, FrameDecoder, PayloadType, END_MARKER, START_MARKER};

/// trace: FR-STREAM-01
#[test]
fn fr_stream_01_encode_wraps_payload_with_markers_and_be_length() {
    let payload = [0x00u8, 0x00, 0x00, b'F', b'A', b';'];
    let frame = encode_frame(&payload);

    assert_eq!(&frame[0..4], &START_MARKER, "start marker");
    assert_eq!(
        &frame[4..8],
        &(payload.len() as u32).to_be_bytes(),
        "big-endian payload length"
    );
    assert_eq!(&frame[8..8 + payload.len()], &payload, "payload body");
    assert_eq!(&frame[frame.len() - 4..], &END_MARKER, "end marker");
}

/// A frame delivered one byte at a time must still be reassembled exactly once
/// (partial START markers across reads must not lose sync).
///
/// trace: FR-STREAM-01, FR-STREAM-03
#[test]
fn fr_stream_01_decoder_reassembles_payload_split_across_reads() {
    let payload = b"\x00\x00\x00IF;".to_vec();
    let frame = encode_frame(&payload);

    let mut dec = FrameDecoder::new();
    let mut out: Vec<Vec<u8>> = Vec::new();
    for b in &frame {
        out.extend(dec.push(&[*b]));
    }

    assert_eq!(out, vec![payload]);
}

/// trace: FR-STREAM-02
#[test]
fn fr_stream_02_payload_type_dispatch_including_unknown() {
    assert_eq!(PayloadType::from_byte(0x00), PayloadType::Cat);
    assert_eq!(PayloadType::from_byte(0x01), PayloadType::Audio);
    assert_eq!(PayloadType::from_byte(0x02), PayloadType::Pan);
    assert_eq!(PayloadType::from_byte(0x03), PayloadType::MiniPan);
    assert_eq!(PayloadType::from_byte(0x42), PayloadType::Unknown(0x42));
}

/// Two frames delivered in a single push must both be returned, in order.
///
/// trace: FR-STREAM-01
#[test]
fn fr_stream_01_decodes_multiple_frames_in_one_push() {
    let mut wire = encode_frame(b"\x00\x00\x00A;");
    wire.extend(encode_frame(b"\x00\x00\x00B;"));

    let mut dec = FrameDecoder::new();
    let out = dec.push(&wire);

    assert_eq!(
        out,
        vec![b"\x00\x00\x00A;".to_vec(), b"\x00\x00\x00B;".to_vec()]
    );
}

/// Leading garbage and a frame with a corrupted END marker must not desync the
/// stream: the decoder skips them and still recovers the following good frame.
///
/// trace: FR-STREAM-03
#[test]
fn fr_stream_03_resyncs_after_garbage_and_corrupted_frame() {
    let good = encode_frame(b"\x00\x00\x00IF;");
    let mut bad = encode_frame(b"\x00\x00\x00BAD;");
    let last = bad.len() - 1;
    bad[last] ^= 0xFF; // corrupt the END marker

    let mut wire = vec![0x11u8, 0x22, 0x33]; // leading garbage
    wire.extend_from_slice(&bad);
    wire.extend_from_slice(&good);

    let mut dec = FrameDecoder::new();
    let out = dec.push(&wire);

    assert_eq!(out, vec![b"\x00\x00\x00IF;".to_vec()]);
}
