//! A minimal WebSocket client (RFC 6455) for the networks reached that way (FreeDV Reporter). Pure
//! and offline: it builds the upgrade request, checks the reply, and turns bytes into messages and
//! messages into bytes; the socket is somebody else's.
//!
//! It is written for a peer that is **not trusted**. The reply to the upgrade is checked in full
//! (status, `Upgrade`, `Connection`, and the `Sec-WebSocket-Accept` value the key implies — which
//! needs SHA-1 and base64, both here and tested against their published vectors); no extension or
//! sub-protocol is ever requested, so one that comes back is refused; and every size is bounded
//! *before* anything is buffered: the header block, a frame's declared length, and a fragmented
//! message's total.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Largest reply header block read, bytes.
pub const MAX_HEADERS: usize = 8 * 1024;
/// Largest message (a frame, or a fragmented message's total) read, bytes.
pub const MAX_MESSAGE: usize = 256 * 1024;
/// Most header lines read.
const MAX_HEADER_LINES: usize = 64;

/// The GUID RFC 6455 §1.3 appends to the client's key.
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

// ---------------------------------------------------------------------------------------------
// SHA-1 and base64, only for the accept key. SHA-1 is broken as a hash; RFC 6455 uses it as a
// handshake check against confused HTTP peers, not for security, and asks for nothing else.

/// SHA-1 (FIPS 180-4) of `data`.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [
        0x6745_2301,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    let (blocks, _) = msg.as_chunks::<64>();
    for block in blocks {
        let mut w = [0u32; 80];
        for (i, word) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(*word);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (hv, v) in h.iter_mut().zip([a, b, c, d, e]) {
            *hv = hv.wrapping_add(v);
        }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

/// Standard base64 (RFC 4648 §4) with padding.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The `Sec-WebSocket-Accept` value a server must answer to `key`.
pub fn accept_key(key: &str) -> String {
    base64(&sha1(format!("{key}{GUID}").as_bytes()))
}

// ---------------------------------------------------------------------------------------------
// Randomness for the client key and the frame masks: a small generator seeded from the clock, the
// process id and a counter. Masking exists to stop a confused intermediary reading client data as
// HTTP, not to hide anything, so this does not need to be a cryptographic source.

/// A tiny xorshift generator.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn seeded() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        let c = COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed);
        Self::from_seed(nanos ^ c ^ (u64::from(std::process::id()) << 32))
    }

    pub fn from_seed(seed: u64) -> Self {
        Self(if seed == 0 {
            0x2545_F491_4F6C_DD1D
        } else {
            seed
        })
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A fresh `Sec-WebSocket-Key`: 16 random bytes, base64.
    pub fn key(&mut self) -> String {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&self.next_u64().to_le_bytes());
        b[8..].copy_from_slice(&self.next_u64().to_le_bytes());
        base64(&b)
    }

    pub fn mask(&mut self) -> [u8; 4] {
        (self.next_u64() as u32).to_le_bytes()
    }
}

// ---------------------------------------------------------------------------------------------
// The upgrade.

fn header_safe(what: &str, v: &str) -> Result<(), String> {
    if v.is_empty() || v.bytes().any(|b| b < 0x20 || b == 0x7f) {
        return Err(format!("{what} is empty or has a control character"));
    }
    Ok(())
}

/// The client's upgrade request. The host, path and user agent are checked for control characters
/// (a CR or LF in one would let it inject headers).
pub fn request(
    host: &str,
    port: u16,
    path: &str,
    key: &str,
    user_agent: &str,
) -> Result<Vec<u8>, String> {
    header_safe("the host", host)?;
    header_safe("the path", path)?;
    header_safe("the user agent", user_agent)?;
    if host.contains(' ') || path.contains(' ') || !path.starts_with('/') {
        return Err("the host or path is not valid in a request".into());
    }
    let host_header = if port == 80 {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    Ok(format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: {user_agent}\r\n\r\n"
    )
    .into_bytes())
}

/// Check the server's reply to the upgrade. `Ok(None)` means the header block is not complete yet;
/// `Ok(Some(n))` means it is valid and ends at byte `n` (what follows is the first frames); `Err`
/// is a refusal worded for the operator.
pub fn check_response(buf: &[u8], key: &str) -> Result<Option<usize>, String> {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
        if buf.len() > MAX_HEADERS {
            return Err("the server's reply has no end to its headers".into());
        }
        return Ok(None);
    };
    let end = end + 4;
    if end > MAX_HEADERS {
        return Err("the server's reply headers are too long".into());
    }
    let head = std::str::from_utf8(&buf[..end]).map_err(|_| "the server's reply is not text")?;
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or("");
    let mut parts = status.splitn(3, ' ');
    if parts.next() != Some("HTTP/1.1") {
        return Err("the server did not answer with HTTP/1.1".into());
    }
    let code: u16 = parts
        .next()
        .and_then(|c| c.parse().ok())
        .ok_or("the server's status line is malformed")?;
    if code != 101 {
        return Err(format!(
            "the server answered HTTP {code}, not a WebSocket upgrade"
        ));
    }
    let (mut upgrade, mut connection, mut accept) = (false, false, false);
    for (n, line) in lines.filter(|l| !l.is_empty()).enumerate() {
        if n >= MAX_HEADER_LINES {
            return Err("the server's reply has too many headers".into());
        }
        let (name, value) = line.split_once(':').ok_or("a header line has no colon")?;
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim());
        match name.as_str() {
            "upgrade" => upgrade = value.eq_ignore_ascii_case("websocket"),
            "connection" => {
                connection = value
                    .split(',')
                    .any(|t| t.trim().eq_ignore_ascii_case("upgrade"))
            }
            "sec-websocket-accept" => accept = value == accept_key(key),
            // None was asked for, so one that comes back is a server doing what it was not told to.
            "sec-websocket-extensions" | "sec-websocket-protocol" => {
                return Err("the server chose an extension or sub-protocol nobody asked for".into())
            }
            _ => {}
        }
    }
    if !upgrade {
        return Err("the reply lacks `Upgrade: websocket`".into());
    }
    if !connection {
        return Err("the reply lacks `Connection: Upgrade`".into());
    }
    if !accept {
        return Err("the server's Sec-WebSocket-Accept does not match the key sent".into());
    }
    Ok(Some(end))
}

// ---------------------------------------------------------------------------------------------
// Frames.

/// A message as it arrives from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Text(String),
    Binary(Vec<u8>),
    Ping(Vec<u8>),
    Pong(Vec<u8>),
    /// The close code, if the server gave one.
    Close(Option<u16>),
}

pub const OP_TEXT: u8 = 0x1;
pub const OP_BINARY: u8 = 0x2;
pub const OP_CLOSE: u8 = 0x8;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;

/// Encode one final, masked client frame.
pub fn encode(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | (opcode & 0x0f));
    match payload.len() {
        n if n < 126 => out.push(0x80 | n as u8),
        n if n <= 0xffff => {
            out.push(0x80 | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(0x80 | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(&mask);
    out.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// Turns the server's bytes into messages, across reads split anywhere.
#[derive(Debug, Default)]
pub struct FrameReader {
    buf: Vec<u8>,
    /// A fragmented message in progress: its opcode and what has arrived.
    partial: Option<(u8, Vec<u8>)>,
}

impl FrameReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes and return every message they complete. An error ends the connection: the peer
    /// broke the protocol, and nothing after it can be trusted.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Message>, String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let b = &self.buf;
            if b.len() < 2 {
                break;
            }
            let (fin, rsv, op) = (b[0] & 0x80 != 0, b[0] & 0x70, b[0] & 0x0f);
            let (masked, len7) = (b[1] & 0x80 != 0, usize::from(b[1] & 0x7f));
            if rsv != 0 {
                return Err("a frame uses a reserved bit (no extension was negotiated)".into());
            }
            if masked {
                return Err("the server sent a masked frame".into());
            }
            if !matches!(op, 0x0 | 0x1 | 0x2 | 0x8 | 0x9 | 0xA) {
                return Err(format!("a frame has the reserved opcode {op:#x}"));
            }
            let (header, len) = match len7 {
                126 => {
                    if b.len() < 4 {
                        break;
                    }
                    (4, usize::from(u16::from_be_bytes([b[2], b[3]])))
                }
                127 => {
                    if b.len() < 10 {
                        break;
                    }
                    let n = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
                    if n >> 63 != 0 {
                        return Err("a frame declares a length with the top bit set".into());
                    }
                    if n > MAX_MESSAGE as u64 {
                        return Err(format!("a frame is larger than {} KB", MAX_MESSAGE / 1024));
                    }
                    (10, n as usize)
                }
                n => (2, n),
            };
            // (The 7- and 16-bit forms cannot exceed the cap; the 64-bit form was checked above,
            // before its length was used for anything.)
            let is_control = op & 0x8 != 0;
            if is_control && (!fin || len > 125) {
                return Err("a control frame is fragmented or longer than 125 bytes".into());
            }
            if b.len() < header + len {
                break;
            }
            let payload = b[header..header + len].to_vec();
            self.buf.drain(..header + len);

            match op {
                OP_PING => out.push(Message::Ping(payload)),
                OP_PONG => out.push(Message::Pong(payload)),
                OP_CLOSE => {
                    if payload.len() == 1 {
                        return Err("a close frame has a one-byte body".into());
                    }
                    let code =
                        (payload.len() >= 2).then(|| u16::from_be_bytes([payload[0], payload[1]]));
                    out.push(Message::Close(code));
                }
                0x0 => {
                    let Some((first, mut have)) = self.partial.take() else {
                        return Err("a continuation frame with nothing to continue".into());
                    };
                    if have.len() + payload.len() > MAX_MESSAGE {
                        return Err(format!(
                            "a fragmented message is larger than {} KB",
                            MAX_MESSAGE / 1024
                        ));
                    }
                    have.extend_from_slice(&payload);
                    if fin {
                        out.push(finish(first, have)?);
                    } else {
                        self.partial = Some((first, have));
                    }
                }
                _ => {
                    if self.partial.is_some() {
                        return Err("a new message began before the last one ended".into());
                    }
                    if fin {
                        out.push(finish(op, payload)?);
                    } else {
                        self.partial = Some((op, payload));
                    }
                }
            }
        }
        Ok(out)
    }
}

fn finish(op: u8, payload: Vec<u8>) -> Result<Message, String> {
    if op == OP_TEXT {
        String::from_utf8(payload)
            .map(Message::Text)
            .map_err(|_| "a text message is not valid UTF-8".to_string())
    } else {
        Ok(Message::Binary(payload))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// FR-SPOT-08: SHA-1, base64 and the accept key against published vectors — FIPS 180
    /// ("abc", the empty string, the two-block message, a million `a`), RFC 4648 §10, and the worked
    /// example in RFC 6455 §1.3.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_ws_hashes_match_published_vectors() {
        assert_eq!(
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            hex(&sha1(&vec![b'a'; 1_000_000])),
            "34aa973cd4c4daa4f61eeb2bdbad27316534016f"
        );
        // Around the padding boundaries (55, 56, 63, 64, 65 bytes) — where implementations slip.
        for (n, want) in [
            (55, "c1c8bbdc22796e28c0e15163d20899b65621d65a"),
            (56, "c2db330f6083854c99d4b5bfb6e8f29f201be699"),
            (63, "03f09f5b158a7a8cdad920bddc29b81c18a551f5"),
            (64, "0098ba824b5c16427bd7a1122a5a442a25ec644d"),
            (65, "11655326c708d70319be2610e8a57d9a5b959d3b"),
        ] {
            assert_eq!(hex(&sha1(&vec![b'a'; n])), want, "{n} bytes");
        }
        for (raw, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(raw.as_bytes()), want, "{raw:?}");
        }
        assert_eq!(
            base64(&[0xfb, 0xff, 0xbf]),
            "+/+/",
            "the two non-alphanumeric characters"
        );
        // RFC 6455 §1.3: this key gives this accept value.
        assert_eq!(
            accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    /// FR-SPOT-08: the client key is 16 bytes of base64, differs from one connection to the next,
    /// and the masks are not constant.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_ws_key_and_mask_vary() {
        let mut r = Rng::seeded();
        let (a, b) = (r.key(), r.key());
        assert_eq!(a.len(), 24);
        assert!(
            a.ends_with("=="),
            "16 bytes encode with two pad characters: {a}"
        );
        assert_ne!(a, b);
        assert_ne!(
            Rng::seeded().key(),
            Rng::seeded().key(),
            "separate generators differ"
        );
        let masks: std::collections::HashSet<[u8; 4]> = (0..50).map(|_| r.mask()).collect();
        assert!(masks.len() > 40);
        // A zero seed does not stick at zero.
        assert_ne!(Rng::from_seed(0).next_u64(), 0);
        assert_eq!(
            Rng::from_seed(7).next_u64(),
            Rng::from_seed(7).next_u64(),
            "deterministic"
        );
    }

    /// FR-SPOT-08: the request is what a server expects, and a value that could inject a header
    /// is refused.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_ws_request_is_well_formed_and_cannot_inject() {
        let req = request(
            "qso.freedv.org",
            80,
            "/socket.io/?EIO=4&transport=websocket",
            "KEY==",
            "K4remote/1",
        )
        .unwrap();
        let text = String::from_utf8(req).unwrap();
        assert_eq!(
            text,
            "GET /socket.io/?EIO=4&transport=websocket HTTP/1.1\r\nHost: qso.freedv.org\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: KEY==\r\nSec-WebSocket-Version: 13\r\nUser-Agent: K4remote/1\r\n\r\n"
        );
        let other = request("h.example", 8080, "/p", "K", "u").unwrap();
        assert!(String::from_utf8(other)
            .unwrap()
            .contains("Host: h.example:8080\r\n"));
        for (h, p, u) in [
            ("h\r\nX: y", "/p", "u"),
            ("h", "/p\r\nX: y", "u"),
            ("h", "/p", "u\r\nX: y"),
            ("h", "/p", "u\nX: y"),
            ("", "/p", "u"),
            ("h", "", "u"),
            ("h", "/p", ""),
            ("h", "p", "u"),
            ("h st", "/p", "u"),
            ("h", "/p q", "u"),
            ("h\0", "/p", "u"),
        ] {
            assert!(request(h, 80, p, "K", u).is_err(), "{h:?} {p:?} {u:?}");
        }
    }

    fn reply(extra: &str, key: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n{extra}\r\n",
            accept_key(key)
        )
        .into_bytes()
    }

    /// FR-SPOT-08: the upgrade reply is accepted only when every part is right, needs the whole
    /// header block, leaves the bytes after it alone, and is refused when it is wrong or hostile.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_ws_response_is_checked_in_full() {
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let good = reply("", key);
        assert_eq!(check_response(&good, key), Ok(Some(good.len())));
        // Bytes after the headers (the first frames) are not consumed.
        let mut more = good.clone();
        more.extend_from_slice(b"\x81\x02hi");
        assert_eq!(check_response(&more, key), Ok(Some(good.len())));
        // Not complete yet: every prefix asks for more, never accepts early.
        for n in 0..good.len() {
            assert_eq!(
                check_response(&good[..n], key),
                Ok(None),
                "prefix of {n} bytes"
            );
        }
        // Header names and the values that are case-insensitive, and a token list.
        for token in [
            "keep-alive, Upgrade",
            "keep-alive, upgrade",
            "UPGRADE",
            "  upgrade  ,keep-alive",
        ] {
            let loose = format!(
                "HTTP/1.1 101 Switching Protocols\r\nupgrade: WebSocket\r\nCONNECTION: {token}\r\nsec-websocket-accept: {}\r\nServer: x\r\n\r\n",
                accept_key(key)
            )
            .into_bytes();
            assert_eq!(
                check_response(&loose, key),
                Ok(Some(loose.len())),
                "{token:?}"
            );
        }

        let wrong = |what: &str, bytes: Vec<u8>| {
            let e = check_response(&bytes, key).expect_err(what);
            assert!(!e.is_empty());
            e
        };
        assert!(wrong("wrong accept", reply("", "another key")).contains("does not match"));
        assert!(wrong("status 200", b"HTTP/1.1 200 OK\r\n\r\n".to_vec()).contains("HTTP 200"));
        assert!(wrong(
            "status 301",
            b"HTTP/1.1 301 Moved\r\nLocation: http://x/\r\n\r\n".to_vec()
        )
        .contains("HTTP 301"));
        assert!(
            wrong("status 403", b"HTTP/1.1 403 Forbidden\r\n\r\n".to_vec()).contains("HTTP 403")
        );
        assert!(wrong("HTTP/1.0", b"HTTP/1.0 101 x\r\n\r\n".to_vec()).contains("HTTP/1.1"));
        assert!(wrong("garbage status", b"HTTP/1.1 abc\r\n\r\n".to_vec()).contains("malformed"));
        assert!(wrong("not http", b"SSH-2.0-x\r\n\r\n".to_vec()).contains("HTTP/1.1"));
        assert!(wrong(
            "no upgrade header",
            format!(
                "HTTP/1.1 101 x\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                accept_key(key)
            )
            .into_bytes()
        )
        .contains("Upgrade: websocket"));
        assert!(wrong("upgrade is not websocket", format!("HTTP/1.1 101 x\r\nUpgrade: h2c\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(key)).into_bytes()).contains("Upgrade: websocket"));
        assert!(wrong(
            "no connection header",
            format!(
                "HTTP/1.1 101 x\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                accept_key(key)
            )
            .into_bytes()
        )
        .contains("Connection: Upgrade"));
        assert!(wrong(
            "no accept",
            b"HTTP/1.1 101 x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n".to_vec()
        )
        .contains("does not match"));
        assert!(wrong(
            "extension nobody asked for",
            reply("Sec-WebSocket-Extensions: permessage-deflate\r\n", key)
        )
        .contains("extension"));
        assert!(wrong(
            "sub-protocol nobody asked for",
            reply("Sec-WebSocket-Protocol: chat\r\n", key)
        )
        .contains("sub-protocol"));
        assert!(
            wrong("header without a colon", reply("no colon here\r\n", key)).contains("no colon")
        );
        assert!(wrong(
            "not text",
            b"HTTP/1.1 101 x\r\nX: \xff\xfe\r\n\r\n".to_vec()
        )
        .contains("not text"));
        // Bounds: a header block that never ends, one that ends too late, and too many headers.
        assert!(wrong("endless", vec![b'a'; 8193]).contains("no end"));
        let mut late = b"HTTP/1.1 101 x\r\n".to_vec();
        late.extend(std::iter::repeat_n(b'a', 8192));
        late.extend_from_slice(b"\r\n\r\n");
        assert!(wrong("too long", late).contains("too long"));
        // A block that ends exactly at the cap is read; one byte later is not.
        let pad = |total: usize| {
            let mut v =
                b"HTTP/1.1 101 x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nX-Pad: ".to_vec();
            let tail = format!("\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(key));
            let fill = total - v.len() - tail.len();
            v.extend(std::iter::repeat_n(b'a', fill));
            v.extend_from_slice(tail.as_bytes());
            assert_eq!(v.len(), total);
            v
        };
        assert_eq!(check_response(&pad(8192), key), Ok(Some(8192)));
        assert!(wrong("one past the cap", pad(8193)).contains("too long"));
        let many: String = (0..70).map(|i| format!("X-{i}: v\r\n")).collect();
        assert!(wrong("too many", reply(&many, key)).contains("too many"));
        let sixty: String = (0..61).map(|i| format!("X-{i}: v\r\n")).collect();
        assert!(
            check_response(&reply(&sixty, key), key).is_ok(),
            "just under the limit is fine"
        );
        // 64 header lines (3 + 61) are read; a 65th is one too many.
        let sixty_five: String = (0..62).map(|i| format!("X-{i}: v\r\n")).collect();
        assert!(
            check_response(&reply(&sixty_five, key), key)
                .unwrap_err()
                .contains("too many"),
            "65 header lines must be refused"
        );
        // Nothing to decide yet is not an error, up to the cap.
        assert_eq!(check_response(&vec![b'a'; 8192], key), Ok(None));
    }

    fn feed(chunks: &[&[u8]]) -> Result<Vec<Message>, String> {
        let mut r = FrameReader::new();
        let mut all = Vec::new();
        for c in chunks {
            all.extend(r.push(c)?);
        }
        Ok(all)
    }

    /// FR-SPOT-08: frames against the worked examples in RFC 6455 §5.7 — the encoder produces the
    /// masked ones byte for byte, the reader takes the unmasked ones, however the bytes are split.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_ws_frames_match_the_rfc_examples() {
        // §5.7: a single-frame masked text "Hello" (what a client sends) and unmasked (what a
        // server sends), a masked pong, and a masked ping.
        let key = [0x37, 0xfa, 0x21, 0x3d];
        assert_eq!(
            encode(OP_TEXT, b"Hello", key),
            [0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]
        );
        assert_eq!(
            encode(OP_PONG, b"Hello", key),
            [0x8a, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58]
        );
        assert_eq!(
            feed(&[&[0x81, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f]]),
            Ok(vec![Message::Text("Hello".into())])
        );
        assert_eq!(
            feed(&[&[0x89, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f]]),
            Ok(vec![Message::Ping(b"Hello".to_vec())])
        );
        // A fragmented message: "Hel" then "lo".
        assert_eq!(
            feed(&[&[0x01, 0x03, 0x48, 0x65, 0x6c], &[0x80, 0x02, 0x6c, 0x6f]]),
            Ok(vec![Message::Text("Hello".into())])
        );
        // A control frame in the middle of a fragmented message is delivered, and the message goes on.
        assert_eq!(
            feed(&[&[0x01, 0x03, 0x48, 0x65, 0x6c, 0x89, 0x00, 0x80, 0x02, 0x6c, 0x6f]]),
            Ok(vec![
                Message::Ping(Vec::new()),
                Message::Text("Hello".into())
            ])
        );
        // 256 bytes of binary uses the 16-bit length; 64 KiB the 64-bit one.
        let mut f256 = vec![0x82, 0x7e, 0x01, 0x00];
        f256.extend(std::iter::repeat_n(7u8, 256));
        assert_eq!(feed(&[&f256]), Ok(vec![Message::Binary(vec![7; 256])]));
        let mut f64k = vec![0x82, 0x7f, 0, 0, 0, 0, 0, 1, 0, 0];
        f64k.extend(std::iter::repeat_n(9u8, 65_536));
        assert_eq!(feed(&[&f64k]), Ok(vec![Message::Binary(vec![9; 65_536])]));
        // The encoder's own length forms: 125, 126, 65535 and 65536 bytes.
        for (n, second) in [
            (125usize, 0x80 | 125u8),
            (126, 0x80 | 126),
            (65_535, 0x80 | 126),
            (65_536, 0x80 | 127),
        ] {
            let f = encode(OP_BINARY, &vec![1u8; n], [1, 2, 3, 4]);
            assert_eq!(f[1], second, "{n}");
            let header = match n {
                0..=125 => 2,
                126..=65_535 => 4,
                _ => 10,
            };
            assert_eq!(f.len(), header + 4 + n);
            // Masking is its own inverse: unmask by hand and the payload is what went in.
            let body: Vec<u8> = f[header + 4..]
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ [1, 2, 3, 4][i % 4])
                .collect();
            assert!(body.iter().all(|b| *b == 1));
        }
        // Split at every byte: the same messages come out.
        let stream: Vec<u8> = [
            &[0x81, 0x05, 0x48, 0x65, 0x6c, 0x6c, 0x6f][..],
            &[0x89, 0x02, 0x01, 0x02],
            &[0x82, 0x7e, 0x00, 0x80][..],
            &[5u8; 128],
            &[0x88, 0x02, 0x03, 0xe8],
        ]
        .concat();
        let whole = feed(&[&stream]).unwrap();
        assert_eq!(whole.len(), 4);
        assert_eq!(whole[3], Message::Close(Some(1000)));
        let bytewise: Vec<&[u8]> = stream.chunks(1).collect();
        assert_eq!(feed(&bytewise).unwrap(), whole);
        // Close with no body.
        assert_eq!(feed(&[&[0x88, 0x00]]), Ok(vec![Message::Close(None)]));
        // Empty text and empty binary.
        assert_eq!(
            feed(&[&[0x81, 0x00, 0x82, 0x00]]),
            Ok(vec![
                Message::Text(String::new()),
                Message::Binary(Vec::new())
            ])
        );
    }

    /// FR-SPOT-08: a peer that breaks the protocol ends the connection, and every bound is
    /// checked before anything large is buffered.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_ws_reader_refuses_hostile_frames() {
        let bad: Vec<(&str, Vec<u8>, &str)> = vec![
            (
                "masked from the server",
                vec![0x81, 0x85, 1, 2, 3, 4, 0, 0, 0, 0, 0],
                "masked",
            ),
            ("RSV1", vec![0xc1, 0x00], "reserved bit"),
            ("RSV2", vec![0xa1, 0x00], "reserved bit"),
            ("RSV3", vec![0x91, 0x00], "reserved bit"),
            ("reserved opcode 3", vec![0x83, 0x00], "reserved opcode"),
            ("reserved opcode 7", vec![0x87, 0x00], "reserved opcode"),
            ("reserved opcode B", vec![0x8b, 0x00], "reserved opcode"),
            ("reserved opcode F", vec![0x8f, 0x00], "reserved opcode"),
            ("fragmented ping", vec![0x09, 0x00], "control frame"),
            ("fragmented close", vec![0x08, 0x00], "control frame"),
            (
                "long ping",
                {
                    let mut v = vec![0x89, 126, 0, 126];
                    v.extend([0u8; 126]);
                    v
                },
                "control frame",
            ),
            ("one-byte close body", vec![0x88, 0x01, 0x03], "one-byte"),
            (
                "continuation with nothing",
                vec![0x80, 0x01, 0x41],
                "nothing to continue",
            ),
            (
                "new message inside a fragmented one",
                vec![0x01, 0x01, 0x41, 0x81, 0x01, 0x42],
                "began before",
            ),
            ("invalid utf-8", vec![0x81, 0x02, 0xff, 0xfe], "UTF-8"),
            (
                "invalid utf-8 across fragments",
                vec![0x01, 0x01, 0xc3, 0x80, 0x01, 0x41],
                "UTF-8",
            ),
            (
                "64-bit length, top bit set",
                vec![0x82, 0x7f, 0x80, 0, 0, 0, 0, 0, 0, 1],
                "top bit",
            ),
            (
                "64-bit length beyond the cap",
                vec![0x82, 0x7f, 0, 0, 0, 0, 0, 0x10, 0, 0],
                "larger than",
            ),
        ];
        for (name, bytes, why) in bad {
            let e = feed(&[&bytes]).expect_err(name);
            assert!(e.contains(why), "{name}: {e}");
        }
        // The size cap: a frame declaring MAX_MESSAGE is read, one more is refused *before* its
        // payload has arrived (nothing is buffered for it).
        let mut at_cap = vec![0x82, 0x7f, 0, 0, 0, 0, 0, 0x04, 0, 0];
        assert_eq!(MAX_MESSAGE, 0x40000);
        at_cap.extend(std::iter::repeat_n(0u8, MAX_MESSAGE));
        assert_eq!(feed(&[&at_cap]).map(|m| m.len()), Ok(1));
        let over = [0x82, 0x7f, 0, 0, 0, 0, 0, 0x04, 0, 1];
        let mut r = FrameReader::new();
        assert!(
            r.push(&over).unwrap_err().contains("larger than"),
            "refused on its header alone"
        );
        // A fragmented message's total is bounded too: two fragments of 200 KB.
        let frag = |first: bool| {
            let mut v = vec![
                if first { 0x02 } else { 0x80 },
                0x7f,
                0,
                0,
                0,
                0,
                0,
                0x03,
                0x0d,
                0x40,
            ];
            v.extend(std::iter::repeat_n(1u8, 200_000));
            v
        };
        let mut r = FrameReader::new();
        assert_eq!(r.push(&frag(true)), Ok(vec![]));
        assert!(r
            .push(&frag(false))
            .unwrap_err()
            .contains("fragmented message is larger"));
        // Many small fragments that add up are bounded as well.
        let mut r = FrameReader::new();
        assert_eq!(r.push(&[0x02, 0x7e, 0x80, 0x00]).map(|_| ()), Ok(()));
        // (a partial frame declaring 32 KB then stalling holds at most that much)
        assert!(r.buf.len() <= 4);
        // Errors leave the reader unusable in spirit, but a further push must not panic.
        let mut r = FrameReader::new();
        assert!(r.push(&[0xc1, 0x00]).is_err());
        let _ = r.push(&[0x81, 0x01, 0x41]);
    }
}
