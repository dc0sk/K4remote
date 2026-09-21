//! Engine.IO 4 / Socket.IO 4 text packets, as FreeDV Reporter speaks them over a WebSocket. Pure
//! and offline: a text message becomes a [`Packet`], and the few packets this client sends are built
//! here.
//!
//! Only what the reporter uses is understood: the Engine.IO `open`, `ping`/`pong` and `close`, and
//! on the default namespace the Socket.IO `connect`, `disconnect`, `connect_error` and `event`.
//! Anything else — binary attachments, acknowledgements, another namespace, an upgrade probe — is
//! [`Packet::Ignored`], never an error: a server that grows a feature must not be able to make this
//! client fall over. What *is* malformed is an error.

use crate::json::{self, Value};

/// The reply to a server ping.
pub const PONG: &str = "3";

/// The Socket.IO protocol revision the reporter's clients announce.
pub const PROTOCOL_VERSION: u32 = 2;

/// Longest event name accepted.
const MAX_EVENT_NAME: usize = 64;
/// Longest session id accepted.
const MAX_SID: usize = 128;
/// Bounds for the server's ping timing, milliseconds: what it says is clamped into this range, so a
/// hostile `pingInterval` of years cannot switch the liveness check off, nor one of zero spin it.
pub const MIN_PING_MS: u64 = 1_000;
pub const MAX_PING_MS: u64 = 120_000;
/// Used when the open packet does not say (Engine.IO's own defaults).
const DEFAULT_PING_INTERVAL_MS: u64 = 25_000;
const DEFAULT_PING_TIMEOUT_MS: u64 = 20_000;

/// One packet from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Packet {
    /// Engine.IO `open`: the session's ping timing.
    Open {
        sid: String,
        ping_interval_ms: u64,
        ping_timeout_ms: u64,
    },
    Ping,
    Pong,
    /// Engine.IO `close`.
    Close,
    /// Socket.IO `connect`: the namespace accepted us.
    Connect {
        sid: Option<String>,
    },
    /// Socket.IO `disconnect`.
    Disconnect,
    /// Socket.IO `connect_error`: the server refused us.
    ConnectError,
    /// Socket.IO `event`: its name and first argument (`Null` if it has none).
    Event {
        name: String,
        args: Value,
    },
    /// Valid, and nothing this client uses.
    Ignored,
}

fn bounded_id(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .filter(|s| {
            !s.is_empty() && s.len() <= MAX_SID && s.bytes().all(|b| (b'!'..=b'~').contains(&b))
        })
        .map(str::to_string)
}

fn ping_ms(v: Option<&Value>, default: u64) -> u64 {
    v.and_then(Value::as_u64)
        .unwrap_or(default)
        .clamp(MIN_PING_MS, MAX_PING_MS)
}

/// Read one text message from the server.
pub fn parse(text: &str) -> Result<Packet, String> {
    let mut chars = text.chars();
    let kind = chars.next().ok_or("an empty packet")?;
    let rest = chars.as_str();
    match kind {
        '0' => {
            let v = json::parse(rest).map_err(|e| format!("the open packet: {e}"))?;
            if !matches!(v, Value::Obj(_)) {
                return Err("the open packet is not an object".into());
            }
            Ok(Packet::Open {
                sid: bounded_id(v.get("sid")).unwrap_or_default(),
                ping_interval_ms: ping_ms(v.get("pingInterval"), DEFAULT_PING_INTERVAL_MS),
                ping_timeout_ms: ping_ms(v.get("pingTimeout"), DEFAULT_PING_TIMEOUT_MS),
            })
        }
        '1' if rest.is_empty() => Ok(Packet::Close),
        '2' if rest.is_empty() => Ok(Packet::Ping),
        '3' if rest.is_empty() => Ok(Packet::Pong),
        // A ping or pong with a body is the upgrade probe, which is not used on a WebSocket
        // opened directly; a `noop` (6) and an `upgrade` (5) likewise.
        '1'..='3' | '5' | '6' => Ok(Packet::Ignored),
        '4' => parse_message(rest),
        c if c.is_ascii_digit() => Ok(Packet::Ignored),
        _ => Err("a packet does not begin with a digit".into()),
    }
}

fn parse_message(rest: &str) -> Result<Packet, String> {
    let mut chars = rest.chars();
    let kind = chars.next().ok_or("an empty Socket.IO packet")?;
    let mut body = chars.as_str();
    // A namespace other than the default (`/`) is not ours: `/name,` before the data.
    if body.starts_with('/') {
        return Ok(Packet::Ignored);
    }
    match kind {
        '0' => {
            if body.is_empty() {
                return Ok(Packet::Connect { sid: None });
            }
            let v = json::parse(body).map_err(|e| format!("the connect packet: {e}"))?;
            Ok(Packet::Connect {
                sid: bounded_id(v.get("sid")),
            })
        }
        '1' => Ok(Packet::Disconnect),
        '4' => Ok(Packet::ConnectError),
        '2' => {
            // An acknowledgement id may precede the data; this client never acknowledges, so it is
            // read and dropped.
            let ack = body.bytes().take_while(u8::is_ascii_digit).count();
            if ack > 16 {
                return Err("an acknowledgement id of more than 16 digits".into());
            }
            body = &body[ack..];
            let v = json::parse(body).map_err(|e| format!("an event: {e}"))?;
            let name = v
                .at(0)
                .and_then(Value::as_str)
                .ok_or("an event has no name")?;
            if name.is_empty()
                || name.len() > MAX_EVENT_NAME
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':' | b'.'))
            {
                return Err("an event name is empty, too long, or has odd characters".into());
            }
            Ok(Packet::Event {
                name: name.to_string(),
                args: v.at(1).cloned().unwrap_or(Value::Null),
            })
        }
        // Acknowledgements and binary events: not used, not an error.
        _ => Ok(Packet::Ignored),
    }
}

/// The Socket.IO `connect` this client sends: the **read-only `view` role** and the protocol
/// revision, and **nothing else** — no callsign, no grid, no software or system description. In
/// the `report` role those are what make a station publicly visible; K4 Remote is receive-only
/// (`FR-SPOT-12`) and never asks for it.
pub fn connect_view() -> String {
    format!("40{{\"role\":\"view\",\"protocol_version\":{PROTOCOL_VERSION}}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(name: &str, args: Value) -> Packet {
        Packet::Event {
            name: name.into(),
            args,
        }
    }

    /// FR-SPOT-08: the packets a real session carries parse to the right thing, including the
    /// ones worded to catch a careless reader (an id before the event data, a namespace, a body
    /// on a ping).
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_sio_packets_parse() {
        assert_eq!(
            parse(
                r#"0{"sid":"AbC_123","upgrades":[],"pingInterval":5000,"pingTimeout":5000,"maxPayload":1000000}"#
            ),
            Ok(Packet::Open {
                sid: "AbC_123".into(),
                ping_interval_ms: 5000,
                ping_timeout_ms: 5000
            })
        );
        assert_eq!(parse("2"), Ok(Packet::Ping));
        assert_eq!(parse("3"), Ok(Packet::Pong));
        assert_eq!(parse("1"), Ok(Packet::Close));
        assert_eq!(parse("40"), Ok(Packet::Connect { sid: None }));
        assert_eq!(
            parse(r#"40{"sid":"xyz-1"}"#),
            Ok(Packet::Connect {
                sid: Some("xyz-1".into())
            })
        );
        assert_eq!(parse("41"), Ok(Packet::Disconnect));
        assert_eq!(parse(r#"44{"message":"nope"}"#), Ok(Packet::ConnectError));
        assert_eq!(
            parse(r#"42["freq_change",{"sid":"a","freq":14236000}]"#),
            Ok(event(
                "freq_change",
                json::parse(r#"{"sid":"a","freq":14236000}"#).unwrap()
            ))
        );
        assert_eq!(
            parse(r#"42["connection_successful"]"#),
            Ok(event("connection_successful", Value::Null))
        );
        // Only the first argument is kept.
        assert_eq!(parse(r#"42["e",1,2,3]"#), Ok(event("e", Value::Num(1.0))));
        // An acknowledgement id in front of the data is read and dropped.
        assert_eq!(
            parse(r#"4212["e",{"a":1}]"#),
            Ok(event("e", json::parse(r#"{"a":1}"#).unwrap()))
        );
        // The ping timing is clamped into range; absent means Engine.IO's defaults.
        for (json_text, want) in [
            (r#"{"pingInterval":1,"pingTimeout":1}"#, (1_000, 1_000)),
            (
                r#"{"pingInterval":99999999999,"pingTimeout":99999999999}"#,
                (120_000, 120_000),
            ),
            (r#"{"pingInterval":"x","pingTimeout":-5}"#, (25_000, 20_000)),
            (r#"{}"#, (25_000, 20_000)),
            // Engine.IO's own defaults, written as literals (25 s and 20 s), so the constants are
            // checked and not merely compared with themselves.
            (r#"{"pingInterval":2500}"#, (2_500, 20_000)),
            (r#"{"pingTimeout":2500}"#, (25_000, 2_500)),
            (r#"{"pingInterval":2500.5}"#, (25_000, 20_000)),
        ] {
            let Ok(Packet::Open {
                ping_interval_ms,
                ping_timeout_ms,
                ..
            }) = parse(&format!("0{json_text}"))
            else {
                panic!("{json_text}");
            };
            assert_eq!((ping_interval_ms, ping_timeout_ms), want, "{json_text}");
        }
        // The ping limits are exact: 999 is raised to 1000, 1000 is kept, 120000 is kept, 120001 is lowered.
        for (ms, want) in [
            (999u64, 1_000u64),
            (1_000, 1_000),
            (120_000, 120_000),
            (120_001, 120_000),
        ] {
            let Ok(Packet::Open {
                ping_interval_ms,
                ping_timeout_ms,
                ..
            }) = parse(&format!(r#"0{{"pingInterval":{ms},"pingTimeout":{ms}}}"#))
            else {
                panic!("{ms}");
            };
            assert_eq!((ping_interval_ms, ping_timeout_ms), (want, want), "{ms}");
        }
        // A session id is kept up to 128 characters and dropped beyond.
        for (len, kept) in [(128usize, true), (129, false)] {
            let sid = "s".repeat(len);
            let Ok(Packet::Connect { sid: got }) = parse(&format!(r#"40{{"sid":"{sid}"}}"#)) else {
                panic!("{len}");
            };
            assert_eq!(got.is_some(), kept, "{len}");
        }
        // Valid but not ours: never an error.
        for ignored in [
            "5",
            "6",
            "2probe",
            "3probe",
            "10",
            "40/admin,",
            "42/admin,[\"e\"]",
            "43[]",
            "45-[\"x\",{\"_placeholder\":true,\"num\":0}]",
            "46-1[]",
            "47",
            "9",
            "7abc",
        ] {
            assert_eq!(parse(ignored), Ok(Packet::Ignored), "{ignored:?}");
        }
        // A session id with odd characters is dropped, not kept.
        assert_eq!(
            parse(r#"0{"sid":"a b"}"#)
                .map(|p| matches!(p, Packet::Open { ref sid, .. } if sid.is_empty())),
            Ok(true)
        );
    }

    /// FR-SPOT-08: malformed packets are errors, not guesses.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_sio_malformed_packets_are_errors() {
        let long_name = format!("42[\"{}\"]", "a".repeat(65));
        let long_ack = format!("42{}[\"e\"]", "1".repeat(17));
        for (name, text) in [
            ("empty", ""),
            ("not a digit", "x"),
            ("letter first", "a[]"),
            ("open without json", "0"),
            ("open with junk", "0not json"),
            ("open not an object", "0[1]"),
            ("open with trailing text", r#"0{} x"#),
            ("empty socket.io packet", "4"),
            ("connect with junk", "40junk"),
            ("event without data", "42"),
            ("event not json", "42nope"),
            ("event not an array", r#"42{"a":1}"#),
            ("empty array", "42[]"),
            ("name not a string", "42[1,2]"),
            ("empty name", r#"42[""]"#),
            ("name with a space", r#"42["a b"]"#),
            ("name with a slash", r#"42["a/b"]"#),
            ("name too long", long_name.as_str()),
            ("ack id too long", long_ack.as_str()),
            ("trailing comma", r#"42["e",]"#),
            ("duplicate key in args", r#"42["e",{"a":1,"a":2}]"#),
        ] {
            assert!(parse(text).is_err(), "{name} must be an error: {text:?}");
        }
        // A name of exactly the maximum length is fine.
        assert!(parse(&format!("42[\"{}\"]", "a".repeat(64))).is_ok());
        // Names may use the punctuation real ones do.
        assert!(parse(r#"42["a_b-c:d.e"]"#).is_ok());
    }

    /// FR-SPOT-08 / FR-SPOT-12: what this client sends is the read-only `view` connect, exactly,
    /// and the pong — and nothing that identifies the operator.
    /// trace: FR-SPOT-08, FR-SPOT-12
    #[test]
    fn fr_spot_12_the_connect_is_view_only_and_anonymous() {
        assert_eq!(connect_view(), r#"40{"role":"view","protocol_version":2}"#);
        assert_eq!(PONG, "3");
        let sent = connect_view().to_ascii_lowercase();
        for identifying in [
            "callsign",
            "grid",
            "version\":\"",
            "os",
            "rx_only",
            "report",
        ] {
            // "os" appears inside "protocol": check as the JSON keys they would be.
            let key = format!("\"{identifying}\"");
            assert!(!sent.contains(&key), "the connect carries {identifying}");
        }
        assert!(!sent.contains("\"role\":\"report\""));
    }
}
