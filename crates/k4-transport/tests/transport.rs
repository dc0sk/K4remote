//! Transport abstraction tests. trace: FR-CONN-ABSTRACT
use k4_transport::{MockTransport, Transport};

/// A consumer written purely against the `Transport` trait works unchanged with
/// the mock — demonstrating transport-agnosticism (the same code will later
/// drive the real TCP and serial transports).
///
/// trace: FR-CONN-ABSTRACT
#[test]
fn fr_conn_abstract_trait_drives_a_mock_transport() {
    fn roundtrip(t: &mut dyn Transport) -> Vec<u8> {
        t.send(b"PING;").unwrap();
        t.recv().unwrap()
    }

    let mut t = MockTransport::new();
    t.push_inbound(b"PONG;");

    let reply = roundtrip(&mut t);

    assert_eq!(t.sent, b"PING;");
    assert_eq!(reply, b"PONG;");
}
