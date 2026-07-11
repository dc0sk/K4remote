//! Transport abstraction (FR-CONN-ABSTRACT, ARC-02).
//!
//! A [`Transport`] is a bidirectional byte channel to a K4 server. The CAT and
//! streaming layers depend only on this trait, so a USB/serial transport (ADR-02)
//! can be added later without changing them. [`MockTransport`] is the in-memory
//! double used for hardware-free tests (NFR-TEST-02).

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use k4_protocol::auth::auth_hash;
use k4_protocol::cat::{decode_cat_text, encode_cat_payload};
use k4_protocol::frame::{encode_frame, FrameDecoder};

/// A bidirectional byte channel to a K4 server.
pub trait Transport {
    /// Send raw bytes (already frame-wrapped by a higher layer).
    fn send(&mut self, data: &[u8]) -> io::Result<()>;

    /// Receive the next chunk of currently-available bytes (may be empty).
    fn recv(&mut self) -> io::Result<Vec<u8>>;
}

/// A framed link to the radio. This is the abstraction the session layer
/// (`k4-session`, ARC-07) depends on, so it can be driven by a mock without real
/// sockets (NFR-TEST-02).
///
/// [`send_frame`](CatLink::send_frame) (wrap a payload in the binary envelope and
/// send it) and [`poll_frames`](CatLink::poll_frames) are the primitives; CAT
/// helpers are derived from them.
pub trait CatLink {
    /// Wrap `payload` (e.g. a CAT or audio payload) in a frame and send it.
    fn send_frame(&mut self, payload: &[u8]) -> io::Result<()>;

    /// Read once and return any complete frame payloads now available (each
    /// payload's first byte is its `PayloadType`). Implementations should map a
    /// read timeout to an empty vec, not an error.
    fn poll_frames(&mut self) -> io::Result<Vec<Vec<u8>>>;

    /// Frame and send a single CAT command (e.g. `"FA;"`).
    fn send_cat(&mut self, command: &str) -> io::Result<()> {
        self.send_frame(&encode_cat_payload(command))
    }

    /// Convenience: just the CAT response texts (default: filter [`poll_frames`]).
    fn poll_cat(&mut self) -> io::Result<Vec<String>> {
        Ok(self
            .poll_frames()?
            .iter()
            .filter_map(|p| decode_cat_text(p))
            .collect())
    }
}

/// A readable + writable byte stream (plain TCP or TLS).
trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

/// Settings for the K4/0 connect handshake (FR-AUTH-01/03).
#[derive(Debug, Clone)]
pub struct ConnectConfig {
    /// Server password. On port 9205 it is SHA-384-hashed; on TLS port 9204 it
    /// is the PSK key (FR-AUTH-02).
    pub password: String,
    /// TLS-PSK identity (optional; usually empty).
    pub identity: String,
    /// Audio encode mode for `EM` (0=RAW32, 1=RAW16, 2=Opus int, 3=Opus float).
    pub encode_mode: u8,
    /// Streaming-latency tier for `SL`.
    pub streaming_latency: u8,
    /// Optional startup macro sent before `RDY;`.
    pub startup_macro: Option<String>,
    /// Socket read timeout.
    pub read_timeout: Duration,
    /// TCP connect timeout — how long to wait for the socket to establish
    /// before failing, instead of the OS default (which can be minutes).
    pub connect_timeout: Duration,
}

/// Open a TCP connection with an explicit connect timeout (FR-CONN-05): resolve
/// the address and try each candidate with [`TcpStream::connect_timeout`], so a
/// dead or filtered host fails within the timeout rather than blocking on the
/// OS default.
///
/// trace: FR-CONN-05
fn connect_timeout<A: ToSocketAddrs>(addr: A, timeout: Duration) -> io::Result<TcpStream> {
    let mut last_err = None;
    for sa in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&sa, timeout) {
            Ok(s) => return Ok(s),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no address to connect to")))
}

impl Default for ConnectConfig {
    fn default() -> Self {
        Self {
            password: String::new(),
            identity: String::new(),
            encode_mode: 3,
            streaming_latency: 2,
            startup_macro: None,
            read_timeout: Duration::from_secs(2),
            connect_timeout: Duration::from_secs(10),
        }
    }
}

/// Direct TCP transport to a K4/0 server on the unencrypted port (ARC-02a).
///
/// `connect` performs the documented handshake: send `SHA-384(password)` as raw
/// hex (FR-AUTH-01), then the post-auth init sequence `RDY; K41; ER1; EM…; SL…;`
/// (FR-AUTH-03). TLS-PSK (port 9204, FR-AUTH-02) is a later addition.
pub struct TcpRemoteTransport {
    stream: Box<dyn Stream>,
    decoder: FrameDecoder,
}

impl TcpRemoteTransport {
    /// Connect to `addr` on the unencrypted port, authenticate with the SHA-384
    /// password hash, and run the post-auth init sequence.
    ///
    /// trace: FR-CONN-01, FR-AUTH-01, FR-AUTH-03
    pub fn connect<A: ToSocketAddrs>(addr: A, cfg: &ConnectConfig) -> io::Result<Self> {
        let stream = connect_timeout(addr, cfg.connect_timeout)?;
        stream.set_read_timeout(Some(cfg.read_timeout))?;
        let mut t = Self {
            stream: Box::new(stream),
            decoder: FrameDecoder::new(),
        };

        // 1. Raw auth hash (NOT frame-wrapped).
        t.stream.write_all(auth_hash(&cfg.password).as_bytes())?;
        t.stream.flush()?;

        // 2. Post-auth init sequence (FR-AUTH-03).
        t.run_init(cfg)?;
        Ok(t)
    }

    /// Connect over TLS-PSK (port 9204): the TLS handshake authenticates via the
    /// pre-shared key (the password), so no SHA-384 hash is sent — just the
    /// post-auth init sequence (FR-AUTH-02/03).
    ///
    /// trace: FR-AUTH-02
    #[cfg(feature = "tls")]
    pub fn connect_tls<A: ToSocketAddrs>(addr: A, cfg: &ConnectConfig) -> io::Result<Self> {
        use openssl::ssl::{Ssl, SslContext, SslMethod, SslStream, SslVerifyMode, SslVersion};

        // Do NOT set a short read timeout before the handshake — the multi-round
        // TLS-PSK handshake would time out mid-way. It is applied afterwards.
        let tcp = connect_timeout(addr, cfg.connect_timeout)?;

        let map = |e: openssl::error::ErrorStack| io::Error::other(e.to_string());
        let mut ctx = SslContext::builder(SslMethod::tls_client()).map_err(map)?;
        // Force TLS 1.2 — the K4 uses TLS1.2 PSK ciphers (R-EXT-01).
        ctx.set_min_proto_version(Some(SslVersion::TLS1_2))
            .map_err(map)?;
        ctx.set_max_proto_version(Some(SslVersion::TLS1_2))
            .map_err(map)?;
        ctx.set_verify(SslVerifyMode::NONE); // PSK, no certificates
                                             // The K4 negotiates PSK-AES256-CBC-SHA384; OpenSSL 3.x's default security
                                             // level 1 rejects that CBC PSK suite, so drop to level 0.
        ctx.set_security_level(0);
        ctx.set_cipher_list("PSK-AES256-CBC-SHA384:PSK-AES128-CBC-SHA256:PSK")
            .map_err(map)?;

        let identity = cfg.identity.clone();
        let password = cfg.password.clone();
        ctx.set_psk_client_callback(move |_ssl, _hint, id_out, psk_out| {
            let id = identity.as_bytes();
            let n = id.len().min(id_out.len().saturating_sub(1));
            id_out[..n].copy_from_slice(&id[..n]);
            id_out[n] = 0; // null-terminate the identity
            let pw = password.as_bytes();
            let m = pw.len().min(psk_out.len());
            psk_out[..m].copy_from_slice(&pw[..m]);
            Ok(m)
        });
        let ctx = ctx.build();

        let ssl = Ssl::new(&ctx).map_err(map)?;
        let mut stream = SslStream::new(ssl, tcp).map_err(map)?;
        stream
            .connect()
            .map_err(|e| io::Error::other(format!("TLS handshake: {e}")))?;
        // Now that the handshake is done, apply the polling read timeout.
        stream.get_ref().set_read_timeout(Some(cfg.read_timeout))?;

        let mut t = Self {
            stream: Box::new(stream),
            decoder: FrameDecoder::new(),
        };
        t.run_init(cfg)?;
        Ok(t)
    }

    /// Post-auth init sequence: optional startup macro, then `RDY/K41/ER1/EM/SL`.
    fn run_init(&mut self, cfg: &ConnectConfig) -> io::Result<()> {
        if let Some(macro_text) = &cfg.startup_macro {
            self.send_cat(macro_text)?;
        }
        self.send_cat("RDY;")?;
        self.send_cat("K41;")?;
        self.send_cat("ER1;")?;
        self.send_cat(&format!("EM{};", cfg.encode_mode))?;
        self.send_cat(&format!("SL{};", cfg.streaming_latency))?;
        Ok(())
    }

    /// Wrap `payload` in a frame envelope and send it.
    pub fn send_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        self.stream.write_all(&encode_frame(payload))
    }

    /// Frame and send a single CAT command (e.g. `"FA;"`).
    pub fn send_cat(&mut self, command: &str) -> io::Result<()> {
        self.send_frame(&encode_cat_payload(command))
    }

    /// Disconnect cleanly by sending `RRN;` (FR-CONN-02).
    pub fn disconnect(&mut self) -> io::Result<()> {
        self.send_cat("RRN;")
    }

    /// Read once and return any complete frame payloads now available.
    pub fn poll_frames(&mut self) -> io::Result<Vec<Vec<u8>>> {
        let mut buf = [0u8; 4096];
        let n = self.stream.read(&mut buf)?;
        Ok(self.decoder.push(&buf[..n]))
    }

    /// Read once and return any complete CAT response texts (ignores audio/PAN).
    pub fn poll_cat(&mut self) -> io::Result<Vec<String>> {
        Ok(self
            .poll_frames()?
            .iter()
            .filter_map(|p| decode_cat_text(p))
            .collect())
    }
}

impl Transport for TcpRemoteTransport {
    fn send(&mut self, data: &[u8]) -> io::Result<()> {
        self.stream.write_all(data)
    }

    fn recv(&mut self) -> io::Result<Vec<u8>> {
        let mut buf = [0u8; 4096];
        let n = self.stream.read(&mut buf)?;
        Ok(buf[..n].to_vec())
    }
}

impl CatLink for TcpRemoteTransport {
    fn send_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        // Explicit path to the inherent method (not a recursive trait call).
        TcpRemoteTransport::send_frame(self, payload)
    }

    fn poll_frames(&mut self) -> io::Result<Vec<Vec<u8>>> {
        match TcpRemoteTransport::poll_frames(self) {
            Ok(v) => Ok(v),
            // A socket read timeout just means "no data this tick".
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(Vec::new())
            }
            Err(e) => Err(e),
        }
    }
}

/// In-memory [`Transport`] double for tests: records everything sent and
/// replays queued inbound chunks.
#[derive(Debug, Default)]
pub struct MockTransport {
    /// All bytes passed to [`Transport::send`], concatenated.
    pub sent: Vec<u8>,
    inbox: VecDeque<Vec<u8>>,
}

impl MockTransport {
    /// Create an empty mock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a chunk that a later [`Transport::recv`] call will return.
    pub fn push_inbound(&mut self, data: &[u8]) {
        self.inbox.push_back(data.to_vec());
    }
}

impl Transport for MockTransport {
    fn send(&mut self, data: &[u8]) -> io::Result<()> {
        self.sent.extend_from_slice(data);
        Ok(())
    }

    fn recv(&mut self) -> io::Result<Vec<u8>> {
        Ok(self.inbox.pop_front().unwrap_or_default())
    }
}

/// Test-only TLS-PSK loopback server (enabled with the `tls` feature) used to
/// validate [`TcpRemoteTransport::connect_tls`] without a real radio.
#[cfg(feature = "tls")]
pub mod tls_support {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::thread;

    use k4_protocol::cat::{decode_cat_text, encode_cat_payload};
    use k4_protocol::frame::{encode_frame, FrameDecoder};
    use openssl::ssl::{Ssl, SslContext, SslMethod, SslStream, SslVersion};

    /// Spawn a TLS-PSK server on an ephemeral localhost port that accepts
    /// `password` as the PSK and replies to `FA;` with `fa_response`.
    pub fn psk_loopback(password: &str, fa_response: &str) -> std::io::Result<SocketAddr> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let pw = password.to_string();
        let fa = fa_response.to_string();
        thread::spawn(move || {
            let _ = serve(listener, &pw, &fa);
        });
        Ok(addr)
    }

    fn serve(listener: TcpListener, password: &str, fa: &str) -> std::io::Result<()> {
        let map = |e: openssl::error::ErrorStack| std::io::Error::other(e.to_string());
        let mut ctx = SslContext::builder(SslMethod::tls_server()).map_err(map)?;
        ctx.set_min_proto_version(Some(SslVersion::TLS1_2))
            .map_err(map)?;
        ctx.set_max_proto_version(Some(SslVersion::TLS1_2))
            .map_err(map)?;
        ctx.set_cipher_list("PSK").map_err(map)?;
        let pw = password.to_string();
        ctx.set_psk_server_callback(move |_ssl, _identity, psk_out| {
            let b = pw.as_bytes();
            let n = b.len().min(psk_out.len());
            psk_out[..n].copy_from_slice(&b[..n]);
            Ok(n)
        });
        let ctx = ctx.build();

        let (tcp, _) = listener.accept()?;
        let ssl = Ssl::new(&ctx).map_err(map)?;
        let mut stream = SslStream::new(ssl, tcp).map_err(map)?;
        stream
            .accept()
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        let mut decoder = FrameDecoder::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf)?;
            if n == 0 {
                break;
            }
            for payload in decoder.push(&buf[..n]) {
                if decode_cat_text(&payload).as_deref() == Some("FA;") {
                    stream.write_all(&encode_frame(&encode_cat_payload(fa)))?;
                }
            }
        }
        Ok(())
    }
}

/// CAT-only transport over a byte stream (USB virtual COM / RS232; ARC-02b).
///
/// Unlike the Ethernet transport, the serial path carries **raw `;`-terminated
/// CAT** — no binary frame envelope, no audio/spectrum. This type adapts that
/// raw stream to the [`CatLink`] interface: outbound CAT payloads are written as
/// plain ASCII, inbound bytes are split into commands and re-wrapped as CAT
/// payloads so the session/demux layers are unchanged. Generic over the byte
/// stream so the adapter is testable without a real port.
pub struct SerialTransport<P> {
    port: P,
    decoder: k4_protocol::cat::LineDecoder,
}

impl<P: Read + Write> SerialTransport<P> {
    /// Wrap an already-open byte stream (e.g. a serial port).
    pub fn from_port(port: P) -> Self {
        Self {
            port,
            decoder: k4_protocol::cat::LineDecoder::new(),
        }
    }
}

/// Serial transport over a real OS serial port (requires the `serial` feature).
#[cfg(feature = "serial")]
pub type SerialPortTransport = SerialTransport<Box<dyn serialport::SerialPort>>;

#[cfg(feature = "serial")]
impl SerialTransport<Box<dyn serialport::SerialPort>> {
    /// Open a serial port (e.g. `"/dev/ttyUSB0"`, 38400 baud) for K4 CAT.
    pub fn open(path: &str, baud: u32, timeout: Duration) -> io::Result<Self> {
        let port = serialport::new(path, baud)
            .timeout(timeout)
            .open()
            .map_err(|e| io::Error::other(e.to_string()))?;
        Ok(Self::from_port(port))
    }
}

/// List available serial port names (e.g. for a UI picker).
#[cfg(feature = "serial")]
pub fn available_serial_ports() -> Vec<String> {
    serialport::available_ports()
        .map(|ports| ports.into_iter().map(|p| p.port_name).collect())
        .unwrap_or_default()
}

impl<P: Read + Write> CatLink for SerialTransport<P> {
    fn send_frame(&mut self, payload: &[u8]) -> io::Result<()> {
        // Serial carries CAT only: write the ASCII of CAT payloads, drop others
        // (audio/spectrum cannot traverse the serial control link).
        if let Some(text) = decode_cat_text(payload) {
            self.port.write_all(text.as_bytes())?;
        }
        Ok(())
    }

    fn poll_frames(&mut self) -> io::Result<Vec<Vec<u8>>> {
        let mut buf = [0u8; 4096];
        match self.port.read(&mut buf) {
            Ok(0) => Ok(Vec::new()),
            Ok(n) => Ok(self
                .decoder
                .push(&buf[..n])
                .iter()
                .map(|cmd| encode_cat_payload(cmd))
                .collect()),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(Vec::new())
            }
            Err(e) => Err(e),
        }
    }
}
