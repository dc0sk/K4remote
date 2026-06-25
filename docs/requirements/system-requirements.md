---
title: "System Requirements Specification"
status: Draft
version: "0.2"
updated: 2026-06-25
authors:
  - Simon Keimer (DC0SK)
owns: [FR, NFR]
---

# System Requirements Specification (SRS)

**Version:** 0.1 (Draft) · **Date:** 2026-06-25 · **Author:** DC0SK
Trace: owns `FR-`, `NFR-`. Up → [stakeholder-requirements.md](stakeholder-requirements.md);
down → [../concept/architecture.md](../concept/architecture.md) (`ARC`) and
[../test/test-strategy.md](../test/test-strategy.md) (`TC`).

**Legend.** Pri: `M` must (v1) · `S` should (v1) · `C` could · `W2` Phase 2.
Ver(ification): `T` automated test · `D` demonstration · `I` inspection · `A` analysis.
Each requirement is written as a single testable "shall". Vendor references cite the K4
Programmer's Reference rev. D12 (`PRG`) command mnemonics.

> All requirements below are **Status: Proposed** in this draft baseline unless noted.

---

## A. Connection & Transport — `FR-CONN`

| ID | Statement (the system shall…) | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-CONN-01` | connect to a K4 server by opening a direct TCP socket to host:port (default **9205** plaintext, **9204** TLS-PSK) and completing the authentication handshake (`FR-AUTH-*`). NOTE: a software client does **not** use the `RRT` command — that is the K4-to-K4 "dial a remote server" form (PRG `RRT`); our client *is* the dialing party. | STK-01 | M | T | Given a simulated server, connect opens the socket and runs the auth+init handshake; on success the session reports `Connected`. |
| `FR-CONN-02` | disconnect cleanly on operator request by sending `RRN;` (PRG `RRT`) and releasing the socket. | STK-01 | M | T | Disconnect emits `RRN;` and transitions to `Disconnected`; socket closed. |
| `FR-CONN-03` | report connection state (`Disconnected`, `Connecting`, `Connected`, `Reconnecting`, `Error`) to the UI as it changes. | STK-01 | M | T | Each transition produces exactly one state event with cause. |
| `FR-CONN-04` | surface connection failures (refused, timeout, auth rejected, host unreachable) with a distinguishable, human-readable reason. | STK-01 | M | T | Each simulated failure maps to its own error variant + message. |
| `FR-CONN-ABSTRACT` | expose all radio I/O through a transport-agnostic interface so that an alternative transport (USB/serial CAT) can be added without changing CAT/UI layers. | STK-18 | S | I/T | A second mock transport implements the trait and drives the CAT engine unchanged in tests. |
| `FR-CONN-05` | apply a configurable connect timeout and fail (not hang) if no handshake response arrives within it. | STK-01 | S | T | With a non-responding server, connect fails within timeout ±tolerance. |

## B. CAT Protocol Engine — `FR-CAT`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-CAT-01` | encode SET commands and decode RESP messages for the supported command set, treating `;` as the frame terminator (PRG Syntax). | STK-02/03 | M | T | Round-trip encode→decode of each supported command yields the original typed value. |
| `FR-CAT-02` | parse a continuous byte stream into discrete commands, tolerating commands split across or batched within reads. | STK-02 | M | T | A stream with fragmented/concatenated frames yields the correct ordered command list. |
| `FR-CAT-03` | recognise the error reply `<cmd>?;` (unparsable/out-of-range) and report it against the originating request (PRG Error Checking). | STK-02 | M | T | Injected `FA?;` is surfaced as an error tied to the pending `FA` request. |
| `FR-CAT-04` | ignore/round-trip-log unknown or unsupported command frames without crashing or desynchronising the parser. | STK-17 | M | T | An unknown `ZZ9;` frame is logged and the next valid frame still parses. |
| `FR-CAT-05` | distinguish main vs. sub-receiver (`$`) variants and target the correct VFO/receiver (PRG `$` modifier). | STK-02/03 | M | T | `MD$3;` decodes as mode-set on sub RX; `MD3;` on main RX. |
| `FR-CAT-AI` | enable an Auto-Info mode (`AI`, target `AI5`/`AI4` immediate) on connect and update internal radio state from unsolicited RESP messages (PRG `AI`, NOTE2 per-client). | STK-04 | M | T | After `AI`, a pushed `FA…;` updates VFO-A state with no GET sent. |
| `FR-CAT-06` | maintain a coherent in-memory **radio state model** updated by both GET responses and Auto-Info, as the single source of truth for the UI. | STK-02/04 | M | T | Concurrent updates leave state consistent; UI reads reflect last value per field. |
| `FR-CAT-07` | reconcile state on (re)connect by issuing a defined GET burst (incl. `IF;`) to seed the model (PRG `IF`). | STK-01/02 | M | T | On connect, the seed GET set is sent and responses populate all seeded fields. |

## B2. Binary Frame & Authentication Layer — `FR-STREAM`

*Realizes the K4/0 wire protocol documented in [../references/external-references.md](../references/external-references.md)
(`R-EXT-01`). All facts to be confirmed against a real radio (`ASM-05`). Clean-room per `CON-09`.*

| ID | Statement (the system shall…) | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-STREAM-01` | frame and de-frame all traffic using the K4 binary envelope: `START(FE FD FC FB)` + big-endian u32 length + payload + `END(FB FC FD FE)`, reassembling across TCP read boundaries. | STK-01/05 | M | T | Byte-exact build; a split/concatenated stream de-frames to the correct payload list; partial-marker tail retained. |
| `FR-STREAM-02` | dispatch payloads by first-byte type: `0x00` CAT, `0x01` Audio, `0x02` PAN, `0x03` MiniPAN; unknown types logged and skipped without desync. | STK-01 | M | T | Each type routes to its handler; an unknown type is skipped and the next frame parses. |
| `FR-STREAM-03` | recover from a corrupted frame (bad END marker, oversize length) by resyncing to the next START marker, bounded by a max buffer size. | STK-17 | M | T | Injected corruption resyncs; buffer never grows past the cap. |
| `FR-AUTH-01` | authenticate on the unencrypted port (default **9205**) by sending `SHA-384(password)` as a lowercase hex string, raw (unframed), immediately after connect. | STK-01/14 | M | T | Auth bytes equal the known-answer SHA-384 hex of the test password. |
| `FR-AUTH-02` | optionally connect on the **TLS-PSK** port (default **9204**) using the password as PSK key (TLS 1.2+), as a selectable secure transport. | STK-14 | S | T/D | TLS-PSK profile negotiates and authenticates against a PSK-capable test endpoint. |
| `FR-AUTH-03` | run the post-auth init sequence in order: optional startup macro, `RDY;`, `K41;`, `ER1;`, `EM<n>;`, `SL<n>;`. | STK-01 | M | T | The emitted command order matches the specification exactly. |
| `FR-SES-PING` | send keep-alive as `PING<unixEpochSeconds>;` at ~1 Hz and derive link latency from the `PONG` reply. | STK-01 | M | T | `PING` carries a monotonic epoch; latency computed from matched `PONG`. |

## C. Session — `FR-SES`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-SES-01` | send a keep-alive at ~1 Hz (format per `FR-SES-PING`) and treat receipt of `PONG` as liveness (PRG PING/PONG; CON-05). | STK-01 | M | T | Timer emits a ping each ~1 s; missing `PONG` flagged. |
| `FR-SES-02` | detect link loss within ≤3 s of the server's 10 s drop threshold being approached (missed PONGs / socket error) and signal it. | STK-01/20 | M | T | Simulated silence → link-loss event within bound. |
| `FR-SES-RECONNECT` | optionally auto-reconnect with bounded backoff after unexpected link loss, re-running the connect handshake and state seed. | STK-20 | S | T | After dropped link, client retries with backoff and restores state on success. |
| `FR-SES-MULTI` | read and display the remote client count via `CC;` (PRG `CC`) to indicate shared control. | STK-19 | C | T | `CC2;` is reflected as "2 clients" in state. |

## D. VFO / Frequency / Band — `FR-VFO`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-VFO-01` | set VFO A and VFO B frequency in Hz via `FA`/`FB`, accepting the operator's entry and emitting the canonical 11-digit form (PRG `FA`/`FB`). | STK-02 | M | T | Entering 14.074 MHz emits `FA00014074000;`. |
| `FR-VFO-02` | display VFO A/B frequency from RESP, parsing the 11-digit Hz form. | STK-02 | M | T | `FB00007100000;` shows 7.100000 MHz on VFO B. |
| `FR-VFO-03` | tune by step increments (configurable step) and by direct numeric entry. | STK-02 | M | T | A +1 step at 100 Hz step changes target freq by exactly 100 Hz. |
| `FR-VFO-04` | switch bands and reflect band-stack changes (PRG `BN`). | STK-02 | M | T | Band-up command and RESP update the band field. |
| `FR-VFO-05` | control RIT/XIT on/off and offset, and clear them (PRG `RT`/`XT`/`RC`, `IF` flags). | STK-03 | S | T | Enabling RIT and a +50 Hz offset is reflected in state and via `IF`. |
| `FR-VFO-06` | control split on/off (PRG `FT`). | STK-02 | S | T | `FT1;`/`FT0;` toggles split state. |
| `FR-VFO-ID` | set/display the station ID text (PRG `ID`) to support identification. | STK-13 | S | T | Setting ID emits `ID<text>;`; RESP updates displayed ID. |

## E. Mode & Bandwidth — `FR-MODE`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-MODE-01` | select operating mode for main/sub RX via `MD`/`MD$` and reflect RESP (PRG `MD`). | STK-03 | M | T | Selecting CW emits the documented `MD` value; RESP updates mode. |
| `FR-MODE-02` | set and display receive bandwidth/filter via `BW`/`BW$` (PRG `BW`). | STK-03 | M | T | Bandwidth set round-trips through state. |
| `FR-MODE-03` | select filter presets where applicable (PRG `FP`). | STK-03 | C | T | `FP2;` reflected in state. |
| `FR-MODE-04` | set the data sub-mode for DATA modes (PRG `DT`/`IF` data field). | STK-03 | C | T | Data sub-mode selection reflected in state. |

## F. Receiver Controls — `FR-RX`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-RX-01` | set AF gain (`AG`/`AG$`) and RF gain (`RG`/`RG$`) with documented ranges and reflect RESP. | STK-03 | M | T | Setting AF 30 emits `AG030;`; out-of-range clamped/rejected per PRG. |
| `FR-RX-02` | control the RX attenuator (`RA`) and preamp where present, including on/off and level. | STK-03 | M | T | Attenuator 12 dB on emits documented `RA…;`; state reflects RESP. |
| `FR-RX-03` | select AGC mode off/slow/fast (`GT`/`GT$`). | STK-03 | S | T | AGC fast reflected in state. |
| `FR-RX-04` | control the noise blanker (`NB`) and noise reduction (`NR`) on/off (and level where defined). | STK-03 | S | T | NB on/off and level round-trip. |
| `FR-RX-05` | select the RX antenna where applicable (`AR`/`AN`). | STK-03 | C | T | Antenna selection reflected in state. |
| `FR-RX-06` | enable/disable and balance the sub receiver (`SB`, `BL`). | STK-03 | C | T | Sub-RX on and balance reflected in state. |

## G. Metering — `FR-MTR`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-MTR-01` | display the receive S-meter, updated automatically (`SM`/`SMH` auto-delivery under AI; PRG `SM`/`SMH`). | STK-04 | M | T | Pushed `SM$08;` updates the meter without a GET. |
| `FR-MTR-02` | display the high-resolution S-meter in dBm when available (`SMH`). | STK-04 | S | T | `SMH-073;` shows −73 dBm. |
| `FR-MTR-03` | during transmit, display available TX metering (power, SWR, ALC) where the radio reports them. | STK-04 | S | T | TX metering fields populate from RESP during a simulated TX. |
| `FR-MTR-04` | represent meter values on a calibrated scale (bars/dBm/S-units) consistent with the K4 bar graph. | STK-04/11 | S | A/T | Mapping function: bar 00→S0 baseline, 42→top, monotonic; unit-tested. |

## H. Transmit, Keying & Safety — `FR-TX`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-TX-01` | initiate and end transmit (PTT) explicitly (`TX;`/`RX;`) only on deliberate operator action. | STK-06/13 | M | T | TX requires an explicit arm+activate; no implicit path sets TX. |
| `FR-TX-02` | set transmit power (`PC`). | STK-06 | S | T | Power 50 W emits documented `PC…;`. |
| `FR-TX-CW-01` | send CW from a connected paddle/key by emitting the remote key data stream (`KZ` with `.`/`-`/`U`/`D`/`P` elements; PRG `KZ`). | STK-07 | M | T | A dit then dah produces the documented `KZ` element sequence. |
| `FR-TX-CW-02` | apply the configurable key-down initial delay (`KZL`, default 80 ms) and honour it in the stream timing. | STK-07 | S | T | `KZL` value is sent and used in stream scheduling. |
| `FR-TX-CW-03` | send CW message/text keying (`KY`) for stored messages. | STK-07 | C | T | A stored message emits a `KY` command with the text. |
| `FR-TX-SAFE-01` | **fail safe on link loss:** if the link drops while transmitting, immediately cease keying/PTT locally and require re-arming before further TX. | STK-08/13 | M | T | Simulated link loss during TX → local TX state cleared; re-arm required. |
| `FR-TX-SAFE-02` | configure and rely on the radio-side CW fail-safe timeout (`KZF`, 1–10 min) so a stalled stream cannot hold the key down indefinitely. | STK-08 | M | T | `KZF` is set on connect; value configurable. |
| `FR-TX-SAFE-03` | provide an explicit, unmistakable TX **arm** control in the UI; transmit is impossible while disarmed. | STK-08/13 | M | T | With TX disarmed, all TX triggers are inert (no `TX;`/`KZ` emitted). |
| `FR-TX-SAFE-04` | provide an always-available emergency "stop transmit / unkey" action. | STK-08 | M | T | Emergency stop emits `RX;` and clears keying regardless of UI focus. |

## I. Audio Streaming — `FR-AUD`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-AUD-RX-01` | receive and play the K4 receive audio stream(s) (two RX channels) over the streaming connection. | STK-05 | M | T/D | RX stream frames are decoded and rendered to the audio output (simulated stream in unit test, real in demo). |
| `FR-AUD-TX-01` | capture microphone audio and send it as the transmit audio stream during voice TX. | STK-06 | M | T/D | While TX armed+active, mic frames are encoded and sent; none sent otherwise. |
| `FR-AUD-ENC` | negotiate the audio encode mode (`EM0–EM3`), defaulting to Opus (`EM3`) for WAN and allowing raw PCM on LAN (PRG `EM`; CON-06). | STK-05/14 | M | T | Default emits `EM3;`; LAN profile can select `EM0/1`. |
| `FR-AUD-02` | buffer received audio with a jitter buffer to tolerate network jitter, with a bounded added latency. | STK-05 | M | T | Reordered/late frames within the buffer window play in order; latency ≤ target. |
| `FR-AUD-03` | set/observe remote streaming audio latency where supported (`SL`). | STK-05 | C | T | `SL` value round-trips. |
| `FR-AUD-04` | decode the K4 audio packet (`0x01`): header (version, sequence, encode mode, frame size u16 LE, sample-rate code), then Opus/PCM data; RX = 12 kHz stereo (L=Main, R=Sub), TX = 12 kHz mono (`R-EXT-01`). | STK-05 | M | T | Decoder yields correct PCM from sample/captured audio fixtures; channel split L=Main/R=Sub verified. |
| `FR-AUD-05` | order/seq-check audio packets using the wrapping sequence byte and drop/conceal as needed. | STK-05 | S | T | Out-of-order/duplicate sequence numbers handled per policy. |

## J. Panadapter / Waterfall — `FR-PAN` (Phase 2; control subset in v1)

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-PAN-CTL-01` | control panadapter span (`#SPN`, 6–368 kHz), reference (`#REF`), scale (`#SCL`), averaging (`#AVG`), FPS (`#FPS`) (PRG Display Commands). | STK-10 | S | T | Each `#` command encodes within documented range and round-trips. |
| `FR-PAN-CTL-02` | control waterfall colour mode (`#WFC`), colour range (`#WBS`), height (`#WFH`), and display mode (`#DSM`). | STK-10 | S | T | Each waterfall `#` command round-trips. |
| `FR-PAN-CTL-03` | freeze/unfreeze the panadapter+waterfall (`#FRZ`) and set peak mode (`#PKM`). | STK-10 | C | T | Freeze toggles state. |
| `FR-PAN-01` | **[Phase 2]** decode the PAN packet (`0x02`): receiver, center freq (i64 LE Hz), sample rate (i32 LE; span = ×1000 Hz), noise floor (i32 LE ÷10 dB), and bins as 1 byte each where **dBm = byte − 146** (`R-EXT-01`). | STK-09 | W2 | T | Decoder produces the expected dBm array + metadata from sample fixtures; MiniPAN (`0x03`) likewise. |
| `FR-PAN-02` | **[Phase 2]** render a real-time spectrum trace from decoded frames. | STK-09 | W2 | D | Visual spectrum updates at target FPS in demo. |
| `FR-PAN-03` | **[Phase 2]** render a scrolling waterfall with the selected colour map. | STK-09 | W2 | D | Waterfall scrolls and colours map per `#WFC`. |
| `FR-PAN-04` | **[Phase 2]** support click/scroll on the spectrum to tune (QSY) the VFO. | STK-09 | W2 | D | Clicking a frequency sends the corresponding `FA`/`FB`. |

## K. GUI Shell — `FR-UI`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-UI-01` | present a connection panel (host/port/password/profile, connect/disconnect, live status). | STK-11 | M | D | All connection actions reachable; status visible. |
| `FR-UI-02` | present primary operating controls (VFO A/B freq, band, mode, bandwidth, key RX controls) bound to the radio state model. | STK-11 | M | D | Control changes reflect in state and vice-versa (two-way). |
| `FR-UI-03` | present metering (S-meter; TX power/SWR during TX) updating in real time. | STK-11 | M | D | Meter animates from pushed updates. |
| `FR-UI-04` | present transmit controls with the explicit TX arm/disarm and emergency-stop affordances (`FR-TX-SAFE`). | STK-11/08 | M | D | Arm/disarm and e-stop are prominent and unambiguous. |
| `FR-UI-05` | reserve a designated, clearly-labelled placeholder region for the Phase-2 spectrum/waterfall. | STK-09/11 | M | I | A placeholder pane exists where the panadapter will mount. |
| `FR-UI-06` | reflect the connection/transmit state visibly enough to prevent mode confusion (e.g. distinct TX indication). | STK-11/08 | M | D | TX state is visually unmistakable. |
| `FR-UI-07` | keep the UI responsive (no freeze) while network/audio I/O runs (UI on its own thread/async runtime). | STK-11 | M | T/D | UI thread never blocks on I/O in design; demonstrated under load. |

## L. Configuration — `FR-CFG`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-CFG-01` | persist and load connection profiles (host, port, audio profile, options). | STK-12 | S | T | A saved profile reloads identically across restarts. |
| `FR-CFG-02` | persist UI/operating preferences (step size, default AI mode, audio device selection). | STK-12 | S | T | Preferences survive restart. |
| `FR-CFG-03` | store the connection **password securely** (OS keychain or, at minimum, not in plaintext logs/config). | STK-12/14 | M | I/T | Password never written to logs; storage is not plaintext config. |

## M. Diagnostics — `FR-DIAG`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-DIAG-01` | provide structured, levelled logging of transport, CAT frames, and session events. | STK-17 | S | T | Log records carry level, timestamp, category; CAT frames optionally traced. |
| `FR-DIAG-02` | offer a raw CAT command/console view for troubleshooting (send arbitrary command, see raw RESP). | STK-17 | C | D | Operator can send `IF;` and see the raw reply. |
| `FR-DIAG-03` | never log secrets (passwords) or full audio payloads. | STK-14/17 | M | I | Inspection confirms redaction. |

---

## Non-Functional Requirements — `NFR`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `NFR-PERF-01` | Control round-trip latency (UI action → command sent → RESP applied) shall be ≤150 ms on a LAN, excluding network RTT. | STK-01/02 | M | T/A | Measured engine-side latency budget met in benchmark harness. |
| `NFR-PERF-CW` | Local CW keying jitter introduced by the client (paddle event → `KZ` emission) shall be ≤10 ms typical. | STK-07 | M | T | Timing test: emission delay distribution within bound. |
| `NFR-PERF-AUDIO` | End-to-end added audio latency from client buffering shall be ≤120 ms (target; tunable jitter buffer). | STK-05 | M | A/D | Measured/configured buffer within target; documented trade-off. |
| `NFR-PERF-AI` | The state model shall absorb AI5 update bursts without unbounded queue growth or UI stall. | STK-04 | M | T | Sustained burst test: bounded memory, no dropped UI refresh deadline. |
| `NFR-REL-FAILSAFE` | On any link loss while transmitting, the client shall reach a non-transmitting safe state within ≤1 s. | STK-08 | M | T | Fault-injection test meets the time bound. |
| `NFR-REL-01` | The client shall not crash on malformed/unknown CAT input; it shall degrade gracefully and keep the session. | STK-17 | M | T | Fuzz/garbage input test: no panic, parser resyncs. |
| `NFR-SEC-01` | The remote connection password shall never appear in logs, crash reports, or telemetry. | STK-14 | M | I/T | Log/redaction test passes; inspection confirms. |
| `NFR-SEC-02` | The application shall treat the network link as untrusted and document/recommend a secure tunnel (VPN) for Internet use. | STK-14 | S | I | Documentation present; no secret sent before handshake auth. |
| `NFR-USE-01` | Operating-critical state (frequency, mode, TX/RX) shall be readable at a glance and update within 200 ms of a change. | STK-11 | M | D | Usability demo against checklist. |
| `NFR-PORT-01` | The application shall build and run on Linux and at least one of Windows/macOS from a single Rust codebase. | STK-16 | S | D | CI builds the target platforms; app launches. |
| `NFR-MAINT-01` | The codebase shall be organised into independently testable layers (transport, CAT, state, audio, UI) with no UI dependency in the protocol core. | STK-15 | M | I | Dependency check: protocol crate has no UI/iced dependency. |
| `NFR-MAINT-LOG` | Diagnostic logging shall be sufficient to reconstruct a failed session's command/event sequence. | STK-17 | S | I | Replay from logs demonstrated. |
| `NFR-TEST-01` | Every `M` and `S` functional/non-functional requirement shall be covered by ≥1 automated test referencing its ID (rule R3/R4). | STK-15 | M | I | Traceability gate `xtask trace` green. |
| `NFR-TEST-02` | The protocol/state core shall be testable without real hardware via a transport mock / K4 protocol simulator. | STK-15 | M | T | Full CAT/state test suite runs with no hardware. |

---

## Open points / to resolve before "Approved"

- ~~`OP-1`~~ **Resolved by `R-EXT-01`** (single multiplexed socket; ports 9205/9204; framing + PAN/audio layouts). Confirm on real radio (`ASM-05`).
- ~~`OP-2`~~ **Resolved:** TX/RX share the `EM` encode negotiation; audio is one multiplexed stream over the control socket (not separate ports).
- ~~`OP-3`~~ **Partially resolved:** audio is **12 kHz** (RX stereo, TX mono), Opus VOIP, frame size per `SL` tier. Still to decide: device-selection UX (`FR-CFG-02`).
- `OP-4` Decide CW source: physical paddle via serial/USB at the client, on-screen, or keyboard — affects `FR-TX-CW-01` input layer. *(QK4 supports hardware keyers/K-Pod; out of our v1 scope.)*
- `OP-5` Confirm required regulatory identification behaviour for `STK-13`/`FR-VFO-ID`.
- `OP-6` Choose default transport security: plaintext+SHA-384 (9205) vs TLS-PSK (9204) for Internet use (`NFR-SEC-02`, `FR-AUTH-02`).

## Change history

| Date | Ver | Author | Change |
|---|---|---|---|
| 2026-06-25 | 0.1 | DC0SK | Initial draft baseline (FR/NFR). |
| 2026-06-25 | 0.2 | DC0SK | Added FR-STREAM/FR-AUTH groups; unblocked FR-AUD-04/FR-PAN-01 with concrete layouts; resolved OP-1..3; added OP-6. |
