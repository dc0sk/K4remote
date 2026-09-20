//! A minimal MQTT 3.1.1 codec, enough to subscribe to a public feed (FR-SPOT-05): pure and
//! offline, no dependencies.
//!
//! Only what a read-only subscriber needs: encode CONNECT, SUBSCRIBE, UNSUBSCRIBE, PINGREQ and
//! DISCONNECT, and decode what a broker sends back. Everything decoded is **untrusted and bounded**
//! ([`PacketReader`]): a packet declaring more than [`MAX_PACKET`] bytes, a malformed length, or an
//! impossible flag is an error, never a huge allocation, and the buffer never holds more than one
//! packet's worth however much arrives.
//!
//! References: the OASIS MQTT 3.1.1 specification. The encoders are held to hand-computed byte
//! vectors from it, not to this file's own decoder.

/// Largest packet accepted, bytes. A spot message is about 200 bytes; a packet declaring more than
/// this is not from a spot feed.
pub const MAX_PACKET: usize = 16 * 1024;

/// A protocol error in what the broker sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttError(pub String);

impl std::fmt::Display for MqttError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for MqttError {}

fn put_len(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let mut b = (n % 128) as u8;
        n /= 128;
        if n > 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            break;
        }
    }
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn packet(first: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![first];
    put_len(&mut out, body.len());
    out.extend_from_slice(body);
    out
}

/// CONNECT with a clean session and no credentials.
pub fn connect(client_id: &str, keepalive_secs: u16) -> Vec<u8> {
    let mut b = Vec::new();
    put_str(&mut b, "MQTT");
    b.push(4); // protocol level 3.1.1
    b.push(0x02); // clean session
    b.extend_from_slice(&keepalive_secs.to_be_bytes());
    put_str(&mut b, client_id);
    packet(0x10, &b)
}

/// SUBSCRIBE to `topics`, each at QoS 0.
pub fn subscribe(packet_id: u16, topics: &[&str]) -> Vec<u8> {
    let mut b = packet_id.to_be_bytes().to_vec();
    for t in topics {
        put_str(&mut b, t);
        b.push(0); // requested QoS 0
    }
    packet(0x82, &b)
}

/// UNSUBSCRIBE from `topics`.
pub fn unsubscribe(packet_id: u16, topics: &[&str]) -> Vec<u8> {
    let mut b = packet_id.to_be_bytes().to_vec();
    for t in topics {
        put_str(&mut b, t);
    }
    packet(0xA2, &b)
}

pub fn pingreq() -> Vec<u8> {
    vec![0xC0, 0x00]
}

pub fn disconnect() -> Vec<u8> {
    vec![0xE0, 0x00]
}

/// What a broker can send that a subscriber cares about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    ConnAck {
        session_present: bool,
        code: u8,
    },
    SubAck {
        packet_id: u16,
        codes: Vec<u8>,
    },
    UnsubAck {
        packet_id: u16,
    },
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: u8,
    },
    PingResp,
    /// A well-formed packet of a type we do not use.
    Other(u8),
}

/// Turns the bytes a broker sends into packets, under hard bounds.
#[derive(Debug, Default)]
pub struct PacketReader {
    buf: Vec<u8>,
}

impl PacketReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes held waiting for the rest of a packet. Never more than one packet's worth.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// Feed bytes; returns every packet completed. An error means the stream is not a valid MQTT
    /// stream and the connection should be dropped.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Packet>, MqttError> {
        let mut out = Vec::new();
        // Chunked, so a huge input never sits in the buffer all at once.
        for chunk in bytes.chunks(4096) {
            self.buf.extend_from_slice(chunk);
            self.drain(&mut out)?;
        }
        Ok(out)
    }

    fn drain(&mut self, out: &mut Vec<Packet>) -> Result<(), MqttError> {
        loop {
            if self.buf.len() < 2 {
                return Ok(());
            }
            // Remaining length: 1–4 bytes, seven bits each.
            let mut rem = 0usize;
            let mut mult = 1usize;
            let mut i = 1;
            loop {
                let Some(&b) = self.buf.get(i) else {
                    return Ok(()); // the length itself is not all here yet
                };
                rem += usize::from(b & 0x7F) * mult;
                i += 1;
                if b & 0x80 == 0 {
                    break;
                }
                if i > 4 {
                    return Err(MqttError("malformed remaining length".into()));
                }
                mult *= 128;
            }
            if rem > MAX_PACKET {
                return Err(MqttError(format!(
                    "a {rem}-byte packet is larger than any spot feed sends"
                )));
            }
            let total = i + rem;
            if self.buf.len() < total {
                return Ok(());
            }
            let first = self.buf[0];
            let packet = decode(first, &self.buf[i..total])?;
            out.push(packet);
            self.buf.drain(..total);
        }
    }
}

fn decode(first: u8, body: &[u8]) -> Result<Packet, MqttError> {
    let bad = |what: &str| MqttError(format!("malformed {what} packet"));
    match first >> 4 {
        2 => {
            if body.len() != 2 {
                return Err(bad("CONNACK"));
            }
            Ok(Packet::ConnAck {
                session_present: body[0] & 1 == 1,
                code: body[1],
            })
        }
        3 => {
            let qos = (first >> 1) & 3;
            if qos == 3 {
                return Err(bad("PUBLISH (QoS 3)"));
            }
            if body.len() < 2 {
                return Err(bad("PUBLISH"));
            }
            let tlen = usize::from(u16::from_be_bytes([body[0], body[1]]));
            let mut at = 2 + tlen;
            if body.len() < at {
                return Err(bad("PUBLISH"));
            }
            let topic = String::from_utf8_lossy(&body[2..at]).into_owned();
            if qos > 0 {
                at += 2; // packet identifier
                if body.len() < at {
                    return Err(bad("PUBLISH"));
                }
            }
            Ok(Packet::Publish {
                topic,
                payload: body[at..].to_vec(),
                qos,
            })
        }
        9 => {
            if body.len() < 2 {
                return Err(bad("SUBACK"));
            }
            Ok(Packet::SubAck {
                packet_id: u16::from_be_bytes([body[0], body[1]]),
                codes: body[2..].to_vec(),
            })
        }
        11 => {
            if body.len() != 2 {
                return Err(bad("UNSUBACK"));
            }
            Ok(Packet::UnsubAck {
                packet_id: u16::from_be_bytes([body[0], body[1]]),
            })
        }
        13 => {
            if !body.is_empty() {
                return Err(bad("PINGRESP"));
            }
            Ok(Packet::PingResp)
        }
        _ => Ok(Packet::Other(first >> 4)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-SPOT-05: the encoders produce the bytes the MQTT 3.1.1 specification defines — compared
    /// with vectors worked out by hand from it, not with this file's own decoder.
    /// trace: FR-SPOT-05
    #[test]
    fn fr_spot_05_mqtt_encoders_match_the_specification() {
        // CONNECT: fixed header 0x10, remaining length 15 = 10 (protocol name "MQTT", level 4,
        // clean-session flag, keepalive) + 5 (client id "k4r" with its 2-byte length).
        assert_eq!(
            connect("k4r", 30),
            [
                0x10, 0x0F, 0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x1E, 0x00, 0x03,
                b'k', b'4', b'r'
            ]
        );
        // SUBSCRIBE: 0x82 (the reserved flags must be 0010), remaining length 8 = packet id 2 +
        // topic (2 + 3) + requested QoS 1.
        assert_eq!(
            subscribe(1, &["a/b"]),
            [0x82, 0x08, 0x00, 0x01, 0x00, 0x03, b'a', b'/', b'b', 0x00]
        );
        // Two topics in one packet: remaining length 11 = 2 (id) + 4 ("a") + 5 ("bc").
        assert_eq!(
            subscribe(0x0102, &["a", "bc"]),
            [0x82, 0x0B, 0x01, 0x02, 0x00, 0x01, b'a', 0x00, 0x00, 0x02, b'b', b'c', 0x00]
        );
        // UNSUBSCRIBE: 0xA2, no QoS byte.
        assert_eq!(
            unsubscribe(2, &["a/b"]),
            [0xA2, 0x07, 0x00, 0x02, 0x00, 0x03, b'a', b'/', b'b']
        );
        assert_eq!(pingreq(), [0xC0, 0x00]);
        assert_eq!(disconnect(), [0xE0, 0x00]);
        // A remaining length of 128 needs two bytes: 0x80 0x01 (spec section 2.2.3).
        let long = "x".repeat(200);
        let p = subscribe(1, &[&long]);
        assert_eq!(
            &p[..3],
            &[0x82, 0xCD, 0x01],
            "2 + 2 + 200 + 1 = 205 = 0xCD 0x01"
        );
    }

    fn publish(topic: &str, payload: &[u8], qos: u8) -> Vec<u8> {
        let mut b = Vec::new();
        put_str(&mut b, topic);
        if qos > 0 {
            b.extend_from_slice(&[0, 7]);
        }
        b.extend_from_slice(payload);
        packet(0x30 | (qos << 1), &b)
    }

    /// FR-SPOT-05: the packets a broker sends decode correctly, including when they arrive split
    /// at any byte, several to a read, or with a two-byte length.
    /// trace: FR-SPOT-05
    #[test]
    fn fr_spot_05_mqtt_decoder_reads_broker_packets() {
        let mut r = PacketReader::new();
        assert_eq!(
            r.push(&[0x20, 0x02, 0x00, 0x00]).unwrap(),
            [Packet::ConnAck {
                session_present: false,
                code: 0
            }]
        );
        assert_eq!(
            r.push(&[0x20, 0x02, 0x01, 0x05]).unwrap(),
            [Packet::ConnAck {
                session_present: true,
                code: 5
            }]
        );
        assert_eq!(
            r.push(&[0x90, 0x03, 0x00, 0x01, 0x00]).unwrap(),
            [Packet::SubAck {
                packet_id: 1,
                codes: vec![0]
            }]
        );
        assert_eq!(
            r.push(&[0xB0, 0x02, 0x00, 0x02]).unwrap(),
            [Packet::UnsubAck { packet_id: 2 }]
        );
        assert_eq!(r.push(&[0xD0, 0x00]).unwrap(), [Packet::PingResp]);
        assert_eq!(
            r.push(&[0x40, 0x02, 0x00, 0x01]).unwrap(),
            [Packet::Other(4)],
            "an unused type is skipped, not an error"
        );

        // A PUBLISH, QoS 0 and QoS 1 (which carries a packet id to skip).
        let want = Packet::Publish {
            topic: "pskr/x".into(),
            payload: b"{\"f\":1}".to_vec(),
            qos: 0,
        };
        assert_eq!(r.push(&publish("pskr/x", b"{\"f\":1}", 0)).unwrap(), [want]);
        let q1 = r.push(&publish("t", b"hi", 1)).unwrap();
        assert_eq!(
            q1,
            [Packet::Publish {
                topic: "t".into(),
                payload: b"hi".to_vec(),
                qos: 1
            }]
        );

        // Split at every possible byte boundary: same result.
        let bytes = publish(
            "pskr/filter/v2/20m/FT8/AA1AAA/BB2BBB/FN31/JO50/291/230",
            &[b'z'; 300],
            0,
        );
        assert!(
            bytes[1] & 0x80 != 0,
            "test setup: a two-byte remaining length"
        );
        for cut in 0..=bytes.len() {
            let mut r = PacketReader::new();
            let mut got = r.push(&bytes[..cut]).unwrap();
            got.extend(r.push(&bytes[cut..]).unwrap());
            assert_eq!(got.len(), 1, "cut at {cut}");
            assert_eq!(r.buffered(), 0);
        }
        // One byte at a time.
        let mut r = PacketReader::new();
        let mut got = Vec::new();
        for b in &bytes {
            got.extend(r.push(&[*b]).unwrap());
        }
        assert_eq!(got.len(), 1);
        // Several packets in one read.
        let mut r = PacketReader::new();
        let mut three = publish("a", b"1", 0);
        three.extend(publish("b", b"2", 0));
        three.extend([0xD0, 0x00]);
        assert_eq!(r.push(&three).unwrap().len(), 3);
    }

    /// FR-SPOT-05: nothing a broker sends can grow memory without bound or panic the decoder.
    /// trace: FR-SPOT-05
    #[test]
    fn fr_spot_05_mqtt_decoder_is_bounded_and_rejects_garbage() {
        // A packet declaring more than the cap is refused before anything is allocated for it.
        let mut r = PacketReader::new();
        let mut big = vec![0x30];
        put_len(&mut big, MAX_PACKET + 1);
        assert!(r.push(&big).is_err());
        // A length that never ends.
        assert!(PacketReader::new()
            .push(&[0x30, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F])
            .is_err());
        // Malformed bodies.
        assert!(
            PacketReader::new().push(&[0x20, 0x01, 0x00]).is_err(),
            "CONNACK too short"
        );
        assert!(
            PacketReader::new().push(&[0x36, 0x02, 0x00, 0x00]).is_err(),
            "QoS 3"
        );
        assert!(
            PacketReader::new().push(&[0x30, 0x02, 0x00, 0x09]).is_err(),
            "a topic longer than the packet"
        );
        assert!(
            PacketReader::new().push(&[0xD0, 0x01, 0x00]).is_err(),
            "PINGRESP with a body"
        );

        // A packet just under the cap is fine, and the buffer never holds more than one packet
        // however much arrives: ten megabytes of a valid-looking header followed by junk.
        let mut r = PacketReader::new();
        let mut header = vec![0x30];
        put_len(&mut header, MAX_PACKET);
        assert!(r.push(&header).unwrap().is_empty(), "waiting for the rest");
        assert!(r.buffered() <= MAX_PACKET + 5);
        let chunk = vec![0u8; 64 * 1024];
        for _ in 0..160 {
            let _ = r.push(&chunk); // completes packets of zeros, or errors: either way bounded
            assert!(r.buffered() <= MAX_PACKET + 5, "buffered {}", r.buffered());
        }

        // Deterministic pseudo-random bytes: never a panic, memory always bounded.
        let mut s = 0x9E3779B97F4A7C15u64;
        for round in 0..300 {
            let mut r = PacketReader::new();
            let junk: Vec<u8> = (0..(round * 7 + 3))
                .map(|_| {
                    s ^= s << 13;
                    s ^= s >> 7;
                    s ^= s << 17;
                    (s >> 24) as u8
                })
                .collect();
            let _ = r.push(&junk);
            assert!(r.buffered() <= MAX_PACKET + 5);
        }
    }
}
