//! K4 protocol simulator (ARC-14): scriptable mock K4/0 server and wire-byte
//! builders for hardware-free integration tests (NFR-TEST-02).
//!
//! [`SimServer`] is a real loopback TCP server that speaks the connect handshake
//! (auth + init), records the commands it receives (so a test can assert the
//! init order, FR-AUTH-03), and answers a small set of GETs. [`server_cat_frame`]
//! builds the raw bytes a server puts on the wire for a single CAT response.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use k4_protocol::auth::auth_hash;
use k4_protocol::cat::{decode_cat_text, encode_cat_payload, set_vfo_a_hz};
use k4_protocol::frame::{encode_frame, FrameDecoder};

/// Bytes a K4 server would put on the wire for a CAT response `text`
/// (e.g. `"FA00014074000;"`): the CAT payload wrapped in the binary frame.
///
/// trace: FR-STREAM-01
pub fn server_cat_frame(text: &str) -> Vec<u8> {
    encode_frame(&encode_cat_payload(text))
}

/// A loopback mock K4/0 server on an ephemeral localhost port.
///
/// It validates the SHA-384 auth, acknowledges with a frame (so the client sees
/// "auth success"), records every CAT command it receives, answers `FA;` with a
/// canned frequency, and replies to `PING…;` with `PONG;`. Other commands
/// (`RDY;`, `K41;`, …) are recorded and acknowledged silently.
pub struct SimServer {
    addr: SocketAddr,
    received: Arc<Mutex<Vec<String>>>,
}

impl SimServer {
    /// Start the server. Requires `password`; serves `vfo_a_hz` for `FA;`.
    pub fn start(password: &str, vfo_a_hz: u64) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let received = Arc::new(Mutex::new(Vec::new()));

        let pw = password.to_string();
        let recv_handle = Arc::clone(&received);
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle_client(stream, &pw, vfo_a_hz, &recv_handle);
            }
        });

        Ok(Self { addr, received })
    }

    /// The bound address to connect to.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Snapshot of the CAT commands received so far, in order.
    pub fn received(&self) -> Vec<String> {
        self.received.lock().expect("sim mutex").clone()
    }
}

fn handle_client(
    mut stream: TcpStream,
    password: &str,
    vfo_a_hz: u64,
    received: &Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    // 1. Read the raw auth hash (lowercase hex of SHA-384 = 96 chars).
    let expected = auth_hash(password);
    let mut auth = vec![0u8; expected.len()];
    stream.read_exact(&mut auth)?;
    if auth != expected.as_bytes() {
        return Ok(()); // reject: drop the connection
    }

    // 2. Acknowledge — the client treats the first inbound frame as auth success.
    stream.write_all(&server_cat_frame("ID0;"))?;

    // 3. Serve framed CAT requests until the client disconnects.
    let mut decoder = FrameDecoder::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break; // EOF
        }
        for payload in decoder.push(&buf[..n]) {
            if let Some(text) = decode_cat_text(&payload) {
                received.lock().expect("sim mutex").push(text.clone());
                if let Some(reply) = response_for(&text, vfo_a_hz) {
                    stream.write_all(&server_cat_frame(&reply))?;
                }
            }
        }
    }
    Ok(())
}

fn response_for(command: &str, vfo_a_hz: u64) -> Option<String> {
    let cmd = command.strip_suffix(';').unwrap_or(command);
    if cmd == "FA" {
        Some(set_vfo_a_hz(vfo_a_hz))
    } else if cmd.starts_with("PING") {
        Some("PONG;".to_string())
    } else {
        None // RDY/K41/ER1/EM/SL and unknown: recorded, acknowledged silently
    }
}
