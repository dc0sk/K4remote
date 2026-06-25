//! L2 integration: simulated server frames → fragmented wire → client decode.
//! Exercises the frame layer (encode on the server side, reassemble + classify
//! + CAT-decode on the client side) with no hardware (NFR-TEST-02).

use k4_protocol::cat::decode_cat_text;
use k4_protocol::frame::{FrameDecoder, PayloadType};
use k4_sim::server_cat_frame;

/// Two back-to-back CAT responses from the simulated server are reassembled and
/// decoded by the client frame layer even when the TCP stream is fragmented at
/// awkward boundaries.
///
/// trace: FR-STREAM-01, FR-STREAM-02, FR-CAT-01
#[test]
fn l2_server_cat_frames_roundtrip_through_client_decoder() {
    let wire: Vec<u8> = [server_cat_frame("FA00014074000;"), server_cat_frame("MD3;")].concat();

    let mut decoder = FrameDecoder::new();
    let mut payloads: Vec<Vec<u8>> = Vec::new();
    // 5-byte chunks straddle markers, length fields, and payload bodies.
    for chunk in wire.chunks(5) {
        payloads.extend(decoder.push(chunk));
    }

    assert_eq!(payloads.len(), 2, "both frames recovered");
    for payload in &payloads {
        assert_eq!(PayloadType::from_byte(payload[0]), PayloadType::Cat);
    }
    assert_eq!(
        decode_cat_text(&payloads[0]).as_deref(),
        Some("FA00014074000;")
    );
    assert_eq!(decode_cat_text(&payloads[1]).as_deref(), Some("MD3;"));
}
