//! Serial (CAT-only) transport tests, using an in-memory port (no hardware).
//! trace: FR-CONN-ABSTRACT, FR-CAT-02
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::rc::Rc;

use k4_transport::{CatLink, SerialTransport};

#[derive(Default)]
struct PortState {
    rx: VecDeque<u8>,
    tx: Vec<u8>,
}

/// Cloneable in-memory byte stream: one clone goes into the transport, one stays
/// with the test to preload reads and inspect writes.
#[derive(Clone, Default)]
struct MockPort(Rc<RefCell<PortState>>);

impl MockPort {
    fn preload(&self, data: &[u8]) {
        self.0.borrow_mut().rx.extend(data.iter().copied());
    }
    fn written(&self) -> Vec<u8> {
        self.0.borrow().tx.clone()
    }
}

impl Read for MockPort {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut state = self.0.borrow_mut();
        if state.rx.is_empty() {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "no data"));
        }
        let n = buf.len().min(state.rx.len());
        for slot in buf.iter_mut().take(n) {
            *slot = state.rx.pop_front().unwrap();
        }
        Ok(n)
    }
}

impl Write for MockPort {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().tx.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The serial transport drives the `CatLink` interface over **raw** CAT: inbound
/// bytes split into commands, outbound writes are plain ASCII with no binary
/// frame envelope (unlike the Ethernet transport).
///
/// trace: FR-CONN-ABSTRACT, FR-CAT-02
#[test]
fn fr_conn_abstract_serial_speaks_raw_cat() {
    let port = MockPort::default();
    port.preload(b"FA00014074000;MD3;"); // raw CAT, no framing
    let mut t = SerialTransport::from_port(port.clone());

    // Inbound: raw CAT split into discrete commands.
    assert_eq!(t.poll_cat().unwrap(), vec!["FA00014074000;", "MD3;"]);

    // Outbound: plain ASCII, NOT a binary frame.
    t.send_cat("MD3;").unwrap();
    assert_eq!(port.written(), b"MD3;");
}

/// A read timeout surfaces as "no data this tick", not an error.
///
/// trace: FR-CONN-ABSTRACT
#[test]
fn fr_conn_abstract_serial_timeout_is_empty() {
    let mut t = SerialTransport::from_port(MockPort::default()); // nothing to read
    assert!(t.poll_frames().unwrap().is_empty());
}
