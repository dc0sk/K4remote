---
title: "System Requirements Specification"
status: Draft
version: "0.14"
updated: 2026-07-06
authors:
  - Simon Keimer (DC0SK)
owns: [FR, NFR]
---

# System Requirements Specification (SRS)

**Version:** 0.1 (Draft) ÃÂ· **Date:** 2026-06-25 ÃÂ· **Author:** DC0SK
Trace: owns `FR-`, `NFR-`. Up Ã¢ÂÂ [stakeholder-requirements.md](stakeholder-requirements.md);
down Ã¢ÂÂ [../concept/architecture.md](../concept/architecture.md) (`ARC`) and
[../test/test-strategy.md](../test/test-strategy.md) (`TC`).

**Legend.** Pri: `M` must (v1) ÃÂ· `S` should (v1) ÃÂ· `C` could ÃÂ· `W2` Phase 2.
Ver(ification): `T` automated test ÃÂ· `D` demonstration ÃÂ· `I` inspection ÃÂ· `A` analysis.
Each requirement is written as a single testable "shall". Vendor references cite the K4
Programmer's Reference rev. D12 (`PRG`) command mnemonics.

> All requirements below are **Status: Proposed** in this draft baseline unless noted.

---

## A. Connection & Transport Ã¢ÂÂ `FR-CONN`

| ID | Statement (the system shallÃ¢ÂÂ¦) | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-CONN-01` | connect to a K4 server by opening a direct TCP socket to host:port (default **9205** plaintext, **9204** TLS-PSK) and completing the authentication handshake (`FR-AUTH-*`). NOTE: a software client does **not** use the `RRT` command Ã¢ÂÂ that is the K4-to-K4 "dial a remote server" form (PRG `RRT`); our client *is* the dialing party. | STK-01 | M | T | Given a simulated server, connect opens the socket and runs the auth+init handshake; on success the session reports `Connected`. |
| `FR-CONN-02` | disconnect cleanly on operator request by sending `RRN;` (PRG `RRT`) and releasing the socket. | STK-01 | M | T | Disconnect emits `RRN;` and transitions to `Disconnected`; socket closed. |
| `FR-CONN-03` | report connection state (`Disconnected`, `Connecting`, `Connected`, `Reconnecting`, `Error`) to the UI as it changes. | STK-01 | M | T | Each transition produces exactly one state event with cause. |
| `FR-CONN-04` | surface connection failures (refused, timeout, auth rejected, host unreachable) with a distinguishable, human-readable reason. | STK-01 | M | T | Each simulated failure maps to its own error variant + message. |
| `FR-CONN-ABSTRACT` | expose all radio I/O through a transport-agnostic interface so that an alternative transport (USB/serial CAT) can be added without changing CAT/UI layers. | STK-18 | S | I/T | A second mock transport implements the trait and drives the CAT engine unchanged in tests. |
| `FR-CONN-05` | apply a configurable connect timeout and fail (not hang) if no handshake response arrives within it. | STK-01 | S | T | With a non-responding server, connect fails within timeout ÃÂ±tolerance. |

## B. CAT Protocol Engine Ã¢ÂÂ `FR-CAT`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-CAT-01` | encode SET commands and decode RESP messages for the supported command set, treating `;` as the frame terminator (PRG Syntax). | STK-02/03 | M | T | Round-trip encodeÃ¢ÂÂdecode of each supported command yields the original typed value. |
| `FR-CAT-02` | parse a continuous byte stream into discrete commands, tolerating commands split across or batched within reads. | STK-02 | M | T | A stream with fragmented/concatenated frames yields the correct ordered command list. |
| `FR-CAT-03` | recognise the error reply `<cmd>?;` (unparsable/out-of-range) and report it against the originating request (PRG Error Checking). | STK-02 | M | T | Injected `FA?;` is surfaced as an error tied to the pending `FA` request. |
| `FR-CAT-04` | ignore/round-trip-log unknown or unsupported command frames without crashing or desynchronising the parser. | STK-17 | M | T | An unknown `ZZ9;` frame is logged and the next valid frame still parses. |
| `FR-CAT-05` | distinguish main vs. sub-receiver (`$`) variants and target the correct VFO/receiver (PRG `$` modifier). | STK-02/03 | M | T | `MD$3;` decodes as mode-set on sub RX; `MD3;` on main RX. |
| `FR-CAT-AI` | enable an Auto-Info mode (`AI`, target `AI5`/`AI4` immediate) on connect and update internal radio state from unsolicited RESP messages (PRG `AI`, NOTE2 per-client). | STK-04 | M | T | After `AI`, a pushed `FAÃ¢ÂÂ¦;` updates VFO-A state with no GET sent. |
| `FR-CAT-06` | maintain a coherent in-memory **radio state model** updated by both GET responses and Auto-Info, as the single source of truth for the UI. | STK-02/04 | M | T | Concurrent updates leave state consistent; UI reads reflect last value per field. |
| `FR-CAT-07` | reconcile state on (re)connect by issuing a defined GET burst (incl. `IF;`) to seed the model (PRG `IF`). | STK-01/02 | M | T | On connect, the seed GET set is sent and responses populate all seeded fields. |

## B2. Binary Frame & Authentication Layer Ã¢ÂÂ `FR-STREAM`

*Realizes the K4/0 wire protocol documented in [../references/external-references.md](../references/external-references.md)
(`R-EXT-01`). All facts to be confirmed against a real radio (`ASM-05`). Clean-room per `CON-09`.*

| ID | Statement (the system shallÃ¢ÂÂ¦) | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-STREAM-01` | frame and de-frame all traffic using the K4 binary envelope: `START(FE FD FC FB)` + big-endian u32 length + payload + `END(FB FC FD FE)`, reassembling across TCP read boundaries. | STK-01/05 | M | T | Byte-exact build; a split/concatenated stream de-frames to the correct payload list; partial-marker tail retained. |
| `FR-STREAM-02` | dispatch payloads by first-byte type: `0x00` CAT, `0x01` Audio, `0x02` PAN, `0x03` MiniPAN; unknown types logged and skipped without desync. | STK-01 | M | T | Each type routes to its handler; an unknown type is skipped and the next frame parses. |
| `FR-STREAM-03` | recover from a corrupted frame (bad END marker, oversize length) by resyncing to the next START marker, bounded by a max buffer size. | STK-17 | M | T | Injected corruption resyncs; buffer never grows past the cap. |
| `FR-AUTH-01` | authenticate on the unencrypted port (default **9205**) by sending `SHA-384(password)` as a lowercase hex string, raw (unframed), immediately after connect. | STK-01/14 | M | T | Auth bytes equal the known-answer SHA-384 hex of the test password. |
| `FR-AUTH-02` | optionally connect on the **TLS-PSK** port (default **9204**) using the password as PSK key (TLS 1.2+), as a selectable secure transport. | STK-14 | S | T/D | TLS-PSK profile negotiates and authenticates against a PSK-capable test endpoint. |
| `FR-AUTH-03` | run the post-auth init sequence in order: optional startup macro, `RDY;`, `K41;`, `ER1;`, `EM<n>;`, `SL<n>;`. | STK-01 | M | T | The emitted command order matches the specification exactly. |
| `FR-SES-PING` | send keep-alive as `PING<unixEpochSeconds>;` at ~1 Hz and derive link latency from the `PONG` reply. | STK-01 | M | T | `PING` carries a monotonic epoch; latency computed from matched `PONG`. |

## C. Session Ã¢ÂÂ `FR-SES`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-SES-01` | send a keep-alive at ~1 Hz (format per `FR-SES-PING`) and treat receipt of `PONG` as liveness (PRG PING/PONG; CON-05). | STK-01 | M | T | Timer emits a ping each ~1 s; missing `PONG` flagged. |
| `FR-SES-02` | detect link loss within Ã¢ÂÂ¤3 s of the server's 10 s drop threshold being approached (missed PONGs / socket error) and signal it. | STK-01/20 | M | T | Simulated silence Ã¢ÂÂ link-loss event within bound. |
| `FR-SES-RECONNECT` | optionally auto-reconnect with bounded backoff after unexpected link loss, re-running the connect handshake and state seed. | STK-20 | S | T | After dropped link, client retries with backoff and restores state on success. |
| `FR-SES-MULTI` | read and display the remote client count via `CC;` (PRG `CC`) to indicate shared control. | STK-19 | C | T | `CC2;` is reflected as "2 clients" in state. |

## D. VFO / Frequency / Band Ã¢ÂÂ `FR-VFO`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-VFO-01` | set VFO A and VFO B frequency in Hz via `FA`/`FB`, accepting the operator's entry and emitting the canonical 11-digit form (PRG `FA`/`FB`). | STK-02 | M | T | Entering 14.074 MHz emits `FA00014074000;`. |
| `FR-VFO-02` | display VFO A/B frequency from RESP, parsing the 11-digit Hz form. | STK-02 | M | T | `FB00007100000;` shows 7.100000 MHz on VFO B. |
| `FR-VFO-03` | tune by step increments (configurable step) and by direct numeric entry. | STK-02 | M | T | A +1 step at 100 Hz step changes target freq by exactly 100 Hz. |
| `FR-VFO-04` | switch bands Ã¢ÂÂ band up/down, **direct band select** by number, band-stacking recall, and transverter bands (PRG `BN`/`BN$`/`BN^`/`XV`). | STK-02 | M | T | Band-up and direct `BN00`Ã¢ÂÂ¦`BN10;` encode; band-stack `BN^;` and `XV` encode; RESP updates the band field. |
| `FR-VFO-05` | control RIT/XIT on/off and offset, and clear them (PRG `RT`/`XT`/`RC`, `IF` flags). | STK-03 | S | T | Enabling RIT and a +50 Hz offset is reflected in state and via `IF`. |
| `FR-VFO-06` | control split on/off (PRG `FT`). | STK-02 | S | T | `FT1;`/`FT0;` toggles split state. |
| `FR-VFO-07` | copy/swap the VFOs Ã¢ÂÂ AÃ¢ÂÂB, BÃ¢ÂÂA, and swap, for frequency or full state (PRG `AB`). | STK-02 | S | T | `AB0`Ã¢ÂÂ¦`AB5;` encode the copy/swap variants. |
| `FR-VFO-ID` | set/display the station ID text (PRG `ID`) to support identification. | STK-13 | S | T | Setting ID emits `ID<text>;`; RESP updates displayed ID. |

## E. Mode & Bandwidth Ã¢ÂÂ `FR-MODE`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-MODE-01` | select operating mode for main/sub RX via `MD`/`MD$` and reflect RESP (PRG `MD`). | STK-03 | M | T | Selecting CW emits the documented `MD` value; RESP updates mode. |
| `FR-MODE-02` | set and display receive bandwidth/filter via `BW`/`BW$` (PRG `BW`). | STK-03 | M | T | Bandwidth set round-trips through state. |
| `FR-MODE-03` | select filter presets where applicable (PRG `FP`). | STK-03 | C | T | `FP2;` reflected in state. |
| `FR-FIL-01` | adjust the passband **shift** / AF center pitch (`IS`; `$`=sub) and offer filter **normalize** (`FP~`). | STK-03 | C | T | `set_shift_hz(1500)` emits `IS0150;`; `filter_normalize()` emits `FP~;`. |
| `FR-FIL-02` | adjust the passband **hi-cut / lo-cut** edges per receiver, derived from `BW`/`IS` (`HI = IS + BW/2`, `LO = IS â BW/2`; the K4 has no dedicated PRG command â D14 FILTER knob). | STK-03 | C | T | `set_passband_edges_hz(300, 2700)` emits `BW0240;` + `IS0150;`; edges round-trip via `passband_edges`. |
| `FR-FIL-03` | draw a **passband overlay** (filter width + VFO centre) on the panadapter. | STK-09 | C | D | A translucent BW-wide band + centre line is drawn on each pane. |
| `FR-MODE-04` | set the data sub-mode for DATA modes (PRG `DT`/`IF` data field). | STK-03 | C | T | Data sub-mode selection reflected in state. |

## F. Receiver Controls Ã¢ÂÂ `FR-RX`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-RX-01` | set AF gain (`AG`/`AG$`) and RF gain (`RG`/`RG$`) with documented ranges and reflect RESP. | STK-03 | M | T | Setting AF 30 emits `AG030;`; out-of-range clamped/rejected per PRG. |
| `FR-RX-02` | control the RX attenuator (`RA`) and preamp where present, including on/off and level. | STK-03 | M | T | Attenuator 12 dB on emits documented `RAÃ¢ÂÂ¦;`; state reflects RESP. |
| `FR-RX-03` | select AGC mode off/slow/fast (`GT`/`GT$`). | STK-03 | S | T | AGC fast reflected in state. |
| `FR-RX-04` | control the noise blanker (`NB`) and noise reduction (`NR`) on/off (and level where defined). | STK-03 | S | T | NB on/off and level round-trip. |
| `FR-RX-NOTCH-01` | control the **manual notch** (on/off + pitch 150Ã¢ÂÂ5000 Hz, `NM`) and **auto-notch** (`NA`) per receiver. | STK-03 | C | T | `set_manual_notch(true,1000)` emits `NM10001;`; `set_auto_notch(true)` emits `NA1;`; RESP reflected. |
| `FR-RX-APF-01` | toggle the **audio peaking filter** and select its bandwidth 30/50/150 Hz (`AP`), in CW. | STK-03 | C | T | `set_apf(true,2)` emits `AP12;`; RESP reflected. |
| `FR-RX-05` | select the RX antenna where applicable (`AR`/`AN`). | STK-03 | C | T | Antenna selection reflected in state. |
| `FR-RX-06` | enable/disable and balance the sub receiver (`SB`, `BL`). | STK-03 | C | T | Sub-RX on and balance reflected in state. |
| `FR-RX-SQL-01` | set and display the main-receiver squelch threshold 0Ã¢ÂÂ40 (`SQ`; `$`=sub). | STK-03 | S | T | `set_squelch(22)` emits `SQ022;`, clamps to 40, and reflects the `SQ` RESP. |
| `FR-DIV-01` | enable/disable **diversity** reception (`DV`) and the **sub receiver** (`SB`), reflecting both states; note diversity implies sub-RX on. | STK-03 | C | T | `set_diversity(true)` emits `DV1;`, `set_sub_rx(true)` emits `SB1;`; DV/SB RESP update state. |

## G. Metering Ã¢ÂÂ `FR-MTR`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-MTR-01` | display the receive S-meter, updated automatically (`SM`/`SMH` auto-delivery under AI; PRG `SM`/`SMH`). | STK-04 | M | T | Pushed `SM$08;` updates the meter without a GET. |
| `FR-MTR-02` | display the high-resolution S-meter in dBm when available (`SMH`). | STK-04 | S | T | `SMH-073;` shows Ã¢ÂÂ73 dBm. |
| `FR-MTR-03` | during transmit, display the TX meters Ã¢ÂÂ RF power, SWR, ALC, and CMP (voice) Ã¢ÂÂ from the auto-delivered `TM` data. | STK-04 | S | T | `TMÃ¢ÂÂ¦;` populates `tx_alc`/`tx_cmp`/`tx_fwd_w`/`tx_swr_x10`. |
| `FR-MTR-04` | represent meter values on a calibrated scale (bars/dBm/S-units) consistent with the K4 bar graph. | STK-04/11 | S | A/T | Mapping function: bar 00Ã¢ÂÂS0 baseline, 42Ã¢ÂÂtop, monotonic; unit-tested. |

## H. Transmit, Keying & Safety Ã¢ÂÂ `FR-TX`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-TX-01` | initiate and end transmit (PTT) explicitly (`TX;`/`RX;`) only on deliberate operator action. | STK-06/13 | M | T | TX requires an explicit arm+activate; no implicit path sets TX. |
| `FR-TX-02` | set transmit power (`PC`). | STK-06 | S | T | Power 50 W emits documented `PCÃ¢ÂÂ¦;`. |
| `FR-TX-CMP-01` | set speech compression 0Ã¢ÂÂ30 (`CP`, SSB modes) and reflect the RESP. | STK-06 | C | T | `set_compression(15)` emits `CP015;`, clamps to 30, and reflects the `CP` RESP. |
| `FR-TX-MON-01` | adjust the **monitor level** (sidetone/speech, `ML`) for the current mode class. | STK-03 | C | T | `set_monitor(0, 20)` = `ML0020;`. |
| `FR-TX-DLY-01` | set full break-in QSK and the VOX/QSK delay per mode class (`SD`). | STK-06 | C | T | `set_qsk_delay(false,'C',25)` emits `SD0C025;`; full-QSK sets `x=1`. |
| `FR-TX-CW-01` | send CW from a connected paddle/key by emitting the remote key data stream (`KZ` with `.`/`-`/`U`/`D`/`P` elements; PRG `KZ`). | STK-07 | M | T | A dit then dah produces the documented `KZ` element sequence. |
| `FR-TX-CW-02` | apply the configurable key-down initial delay (`KZL`, default 80 ms) and honour it in the stream timing. | STK-07 | S | T | `KZL` value is sent and used in stream scheduling. |
| `FR-TX-CW-03` | send CW message/text keying (`KY`) for stored messages. | STK-07 | C | T | A stored message emits a `KY` command with the text. |
| `FR-TX-SAFE-01` | **fail safe on link loss:** if the link drops while transmitting, immediately cease keying/PTT locally and require re-arming before further TX. | STK-08/13 | M | T | Simulated link loss during TX Ã¢ÂÂ local TX state cleared; re-arm required. |
| `FR-TX-SAFE-02` | configure and rely on the radio-side CW fail-safe timeout (`KZF`, 1Ã¢ÂÂ10 min) so a stalled stream cannot hold the key down indefinitely. | STK-08 | M | T | `KZF` is set on connect; value configurable. |
| `FR-TX-SAFE-03` | provide an explicit, unmistakable TX **arm** control in the UI; transmit is impossible while disarmed. | STK-08/13 | M | T | With TX disarmed, all TX triggers are inert (no `TX;`/`KZ` emitted). |
| `FR-TX-SAFE-04` | provide an always-available emergency "stop transmit / unkey" action. | STK-08 | M | T | Emergency stop emits `RX;` and clears keying regardless of UI focus. |

## I. Audio Streaming Ã¢ÂÂ `FR-AUD`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-AUD-RX-01` | receive and play the K4 receive audio stream(s) (two RX channels) over the streaming connection. | STK-05 | M | T/D | RX stream frames are decoded and rendered to the audio output (simulated stream in unit test, real in demo). |
| `FR-AUD-TX-01` | capture microphone audio and send it as the transmit audio stream during voice TX. | STK-06 | M | T/D | While TX armed+active, mic frames are encoded and sent; none sent otherwise. |
| `FR-AUD-ENC` | negotiate the audio encode mode (`EM0Ã¢ÂÂEM3`), defaulting to Opus (`EM3`) for WAN and allowing raw PCM on LAN (PRG `EM`; CON-06). | STK-05/14 | M | T | Default emits `EM3;`; LAN profile can select `EM0/1`. |
| `FR-AUD-02` | buffer received audio with a jitter buffer to tolerate network jitter, with a bounded added latency. | STK-05 | M | T | Reordered/late frames within the buffer window play in order; latency Ã¢ÂÂ¤ target. |
| `FR-AUD-03` | set/observe remote streaming audio latency where supported (`SL`). | STK-05 | C | T | `SL` value round-trips. |
| `FR-AUD-04` | decode the K4 audio packet (`0x01`): header (version, sequence, encode mode, frame size u16 LE, sample-rate code), then Opus/PCM data; RX = 12 kHz stereo (L=Main, R=Sub), TX = 12 kHz mono (`R-EXT-01`). | STK-05 | M | T | Decoder yields correct PCM from sample/captured audio fixtures; channel split L=Main/R=Sub verified. |
| `FR-AUD-05` | order/seq-check audio packets using the wrapping sequence byte and drop/conceal as needed. | STK-05 | S | T | Out-of-order/duplicate sequence numbers handled per policy. |
| `FR-AUD-DEV-01` | let the operator **select the audio devices** Ã¢ÂÂ RX playback (output) and TX microphone (capture) Ã¢ÂÂ from the OS-enumerated device lists via dropdowns in the settings dialog; the choice persists (`FR-CFG-02`) and is applied to the audio streams. | STK-05/06/12 | S | D/T | Available output/input devices are listed; selecting one routes RX playback / TX capture to it and the choice survives restart. |
| `FR-AUD-LVL-01` | provide a **RX volume** slider (local playback level of the received audio) and a **TX mic-level** slider (local capture gain before encode), each adjustable live; these are client-side levels, distinct from the radio's `AG`/`MG`. | STK-05/06 | S | T/D | Moving the volume slider scales RX playback amplitude; the mic slider scales captured mic amplitude; both take effect without reconnect. |
| `FR-AUD-MON-01` | optionally **mute the radio TX monitor** (`ML=0`) on connect so a remote session doesn't drive the shack speaker. | STK-11 | C | D | With the option on, connecting sends `ML0000;ML1000;ML2000;`. |

## J. Panadapter / Waterfall Ã¢ÂÂ `FR-PAN` (Phase 2; control subset in v1)

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-PAN-CTL-01` | control the panadapter/display via the `#` command family: mode/dual (`#DPM`), span (`#SPN`, 6Ã¢ÂÂ368 kHz), reference (`#REF`), scale (`#SCL`), averaging (`#AVG`), peak (`#PKM`), fixed/freeze (`#FXT`/`#FRZ`), waterfall palette/height (`#WFC`/`#WFH`), panadapter NB (`#NB`/`#NBL`) (PRG Display Commands). | STK-10 | S | T | Each `#` command encodes within documented range and round-trips. |
| `FR-PAN-CTL-02` | control waterfall colour mode (`#WFC`), colour range (`#WBS`), height (`#WFH`), and display mode (`#DSM`). | STK-10 | S | T | Each waterfall `#` command round-trips. |
| `FR-PAN-CTL-03` | freeze/unfreeze the panadapter+waterfall (`#FRZ`) and set peak mode (`#PKM`). | STK-10 | C | T | Freeze toggles state. |
| `FR-PAN-01` | **[Phase 2]** decode the PAN packet (`0x02`): receiver, center freq (i64 LE Hz), sample rate (i32 LE; span = ÃÂ1000 Hz), noise floor (i32 LE ÃÂ·10 dB), and bins as 1 byte each where **dBm = byte Ã¢ÂÂ 146** (`R-EXT-01`). | STK-09 | W2 | T | Decoder produces the expected dBm array + metadata from sample fixtures; MiniPAN (`0x03`) likewise. |
| `FR-PAN-02` | **[Phase 2]** render a real-time spectrum trace from decoded frames. | STK-09 | W2 | D | Visual spectrum updates at target FPS in demo. |
| `FR-PAN-03` | **[Phase 2]** render a scrolling waterfall with the selected colour map. | STK-09 | W2 | D | Waterfall scrolls and colours map per `#WFC`. |
| `FR-PAN-04` | **[Phase 2]** support click/scroll on the spectrum to tune (QSY) the VFO. | STK-09 | W2 | D | Clicking a frequency sends the corresponding `FA`/`FB`. |

## K. GUI Shell Ã¢ÂÂ `FR-UI`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-UI-01` | present a connection panel (host/port/password/profile, connect/disconnect, live status). | STK-11 | M | D | All connection actions reachable; status visible. |
| `FR-UI-02` | present primary operating controls (VFO A/B freq, band, mode, bandwidth, key RX controls) bound to the radio state model. | STK-11 | M | D | Control changes reflect in state and vice-versa (two-way). |
| `FR-UI-03` | present metering (S-meter; TX power/SWR during TX) updating in real time. | STK-11 | M | D | Meter animates from pushed updates. |
| `FR-UI-04` | present transmit controls with the explicit TX arm/disarm and emergency-stop affordances (`FR-TX-SAFE`). | STK-11/08 | M | D | Arm/disarm and e-stop are prominent and unambiguous. |
| `FR-UI-05` | reserve a designated, clearly-labelled placeholder region for the Phase-2 spectrum/waterfall. | STK-09/11 | M | I | A placeholder pane exists where the panadapter will mount. |
| `FR-UI-06` | reflect the connection/transmit state visibly enough to prevent mode confusion (e.g. distinct TX indication). | STK-11/08 | M | D | TX state is visually unmistakable. |
| `FR-UI-07` | keep the UI responsive (no freeze) while network/audio I/O runs (UI on its own thread/async runtime). | STK-11 | M | T/D | UI thread never blocks on I/O in design; demonstrated under load. |
| `FR-UI-08` | provide a switchable main-window **view mode** Ã¢ÂÂ single VFO A, single VFO B, or dual (A+B) Ã¢ÂÂ that reflows the header + panadapter layout, mirroring the K4 `PAN=A/B/A+B` selection (`R-EXT-02`, `ui-design.md`). | STK-11 | M | T | `ViewMode` cycles AÃ¢ÂÂBÃ¢ÂÂdualÃ¢ÂÂA; each mode reports which receiver pane(s) are shown. |
| `FR-UI-09` | render operating frequencies with the K4's **dot-grouped** formatting to kHz (e.g. `14.070.000`). | STK-11 | M | T | Formatter maps Hz Ã¢ÂÂ grouped string across band edges and sub-kHz values. |
| `FR-UI-10` | apply **semantic colour roles** to operating state Ã¢ÂÂ transmit (amber), VFO A/main (blue), VFO B/sub & active (green), RX values (white), caution (yellow), inactive (dim) Ã¢ÂÂ so TX/RX and A/B are unmistakable (`FR-UI-06`). | STK-11/08 | M | T | Role selector returns the correct role for TX/RX, A/B, active/inactive inputs. |
| `FR-UI-11` | present operating controls as **two-line state buttons** (function label + live value derived from the radio state), e.g. `ATT`/`Off`, `AGC`/`Slow`, `BW`/`2.80`. | STK-11 | S | T | StateÃ¢ÂÂ(label,value) derivation matches the radio state for representative controls. |
| `FR-UI-12` | lay out the two VFOs **symmetrically** (A-left/B-right) with the shared TX/SPLIT/RIT-XIT indicator **between** them, reflowing responsively (single pane / dual side-by-side / stacked when narrow). | STK-11/08 | M | T/D | `band_layout(width, mode)` yields the right panes, centre-box placement, and narrow-stack reflow (test); A/B panels mirror with the shared transmit box between them (demo). |
| `FR-UI-13` | use a consistent **primary-button + context-row** interaction model: a fixed row of primaries (`MENU/Fn/DISPLAY/BAND/MAIN RX/SUB RX/TX`) each swaps a context sub-row of controls (mode-dependent where the K4 is). | STK-11 | S | T/D | State machine: tapping a primary toggles its row, only one open at a time; context items are mode-dependent (test). Visual reveal (demo). |
| `FR-UI-14` | offer a **mini-pan** tuning aid (a narrow zoomed spectrum around the active VFO, invoked from the S-meter) for fine tuning. | STK-09/11 | C | D | A zoomed spectrum appears over the main pan and tracks the VFO. |
| `FR-UI-15` | style the client after the references (`R-EXT-02`): a **dark layered theme** whose background/panel/control surfaces step up strictly in luminance, and a **proportional S-meter bar** on the K4 face (S1 Ã¢ÂÂ Ã¢ÂÂ121 dBm Ã¢ÂÂ¦ S9+60 Ã¢ÂÂ Ã¢ÂÂ13 dBm, S9 = Ã¢ÂÂ73 dBm), green with a caution colour Ã¢ÂÂ¥ S9. | STK-11 | S | T/D | Shade palette is strictly luminance-ordered and `s_meter_fraction` maps the face endpoints/S9 correctly, clamped (test); themed panels/buttons/meters render per `ui-design.md` (demo). |
| `FR-UI-STATUS-01` | show a **status strip** with the radio UTC clock (`UT`) and remote client count (`CC`). | STK-09 | C | D | The header shows `HH:MM:SS UTC` and client count when connected. |
| `FR-UI-16` | present the connect control as a function of the connection phase: **Connect** while idle, **Cancel** while an attempt is in flight (opening/handshaking or awaiting retry), and **Disconnect** once a session is up; tapping it while it shows **Cancel** shall abort the in-flight attempt and return to disconnected. The connect attempt shall not freeze the UI or the worker (`FR-UI-07`). | STK-01/11 | S | T/D | `connect_button(phase)` yields the correct (label, action) for each phase (test); a live attempt to a non-responsive host shows **Cancel**, and tapping it returns to **Connect** with the attempt aborted (demo). |
| `FR-UI-17` | offer a **theme selector** cycling **Dark Ã¢ÂÂ Light Ã¢ÂÂ Contrast Ã¢ÂÂ System**, applied live to the whole UI; `System` follows the OS light/dark preference. Each theme resolves the surface-shade and semantic-role palettes (`FR-UI-10/15`). | STK-11 | C | T/D | `ThemeMode` cycles the four modes with distinct labels and resolves to a concrete palette (`System` per the detected OS preference) (test); each theme renders coherently (demo). |
| `FR-UI-18` | provide an **About** affordance showing the author, the **software version**, the license, the project URL, and a **donate** link; the license, project URL, and donate entries shall open in the OS browser when activated. | STK-11 | C | T/D | The About constants (author/license+URL/project URL/donate URL) and `app_version()` are present (test); the About box shows them, the links open externally, and it dismisses (demo). |
| `FR-UI-19` | when a primary softkey (`MENU/Fn/DISPLAY/BAND/MAIN RX/SUB RX/TX`) is active, display that primary's K4 **configuration screen in place of the spectrum frame** (`R-EXT-02`) Ã¢ÂÂ **not** replacing the mode/filter controls and **not** a separate window Ã¢ÂÂ and restore the spectrum when it is deselected. The screen shows the radio's *additional* functions (e.g. RX/TX equalizer, display setup, band stacking), not controls already present elsewhere in the UI; the VFO band, controls, softkey row, and lower panels stay visible and operational. | STK-11 | S | T/D | `menu_screen_synopsis` maps each primary to a distinct screen (test); selecting a primary swaps only the spectrum frame and deselecting restores it, with the rest of the UI untouched (demo). |
| `FR-UI-20` | **seed the configuration screens from the radio** on connect: the connect GET burst requests each screen's values (`RE/TE/KP/KS/MI/MG/LO/AN/AR/VX/BN/#REF/#SPN/#SCL/#DPM/#WFC/#WFH`), the parsed `RadioState` is surfaced into the snapshot, and each screen (EQ/DISPLAY/TX/RX) reflects the radio's **current** values once per connection rather than local defaults; later user edits are not overwritten. | STK-11/04 | S | T/D | `RadioState::apply_cat` parses each RESP form (test); on connect the screens show the radio's reported values (demo/live). |
| `FR-UI-21` | **start in a landscape window** (wider than tall), matching the horizontal layout. | STK-11 | C | T/D | `DEFAULT_WINDOW_SIZE` has width > height (test); the window opens landscape (demo). |
| `FR-UI-22` | show a **phase-coloured connection indicator** in the header Ã¢ÂÂ a dot + label that is green when connected, amber while connecting, and grey when disconnected. | STK-01/11 | C | T/D | `conn_status(phase)` returns the correct (label, colour role) per phase (test); the header dot changes colour across a connect cycle (demo). |
| `FR-UI-23` | provide an **application settings dialog** (opened from a settings affordance) housing the **connection** settings (host/port/TLS/password/serial), **audio-device** selection (`FR-AUD-DEV-01`), and **audio levels** (`FR-AUD-LVL-01`); the connection form **moves into this dialog** rather than occupying a permanent panel on the main window. | STK-11/12 | S | D | Opening settings shows connection + audio controls; connecting works from the dialog; the main window no longer carries the permanent connection panel. |

## L. Configuration Ã¢ÂÂ `FR-CFG`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-CFG-01` | persist and load connection profiles (host, port, audio profile, options). | STK-12 | S | T | A saved profile reloads identically across restarts. |
| `FR-CFG-02` | persist UI/operating preferences (step size, default AI mode, audio device selection). | STK-12 | S | T | Preferences survive restart. |
| `FR-CFG-03` | store the connection **password securely** (OS keychain or, at minimum, not in plaintext logs/config). | STK-12/14 | M | I/T | Password never written to logs; storage is not plaintext config. |
| `FR-CFG-04` | maintain a **peer cache** of successfully-connected servers (name/host/port/TLS/username). Each peer's password shall be stored **either** in the OS **credential manager** **or** in the **local config file encrypted** under a key derived (KDF) from a user-supplied **master password** that is entered on connect to unlock decryption; passwords are **never** stored in plaintext. The operator shall be able to **select** a cached peer to populate the connection and **delete** the selected peer's entry. | STK-12/14 | S | T/D | EncryptÃ¢ÂÂdecrypt of a peer password round-trips under the correct master password and **fails under a wrong one**; keychain mode stores/retrieves; a peer is added on a successful connect; deleting the selected peer removes it (config + secret). |
| `FR-CFG-05` | **remember the last session and settings** across restarts: the last-connected peer (prefilled connection), the transport mode (TLS/serial), and user settings (theme, master-password storage mode, and Ã¢ÂÂ once implemented Ã¢ÂÂ audio device/level choices). | STK-12 | S | T/D | After a restart the app prefills the last connection and restores the saved settings. |
| `FR-CFG-06` | **export/import the K4 settings** to a `K4-<serial>-<timestamp>.cfg` file (replayable CAT commands) guarded by a **SHA-256** hash, and **play** an imported file back to the radio. | STK-12 | C | T | `backup::export`/`import` round-trips commands and rejects a hash mismatch. |
| `FR-CFG-07` | optionally include the **menu items** in the config export by sweeping `ME<id>` and replaying `ME<id>.<value>`. | STK-12 | C | T | `menu_query(30)`=`ME0030;`; a captured `ME0030.<v>` round-trips into the export. |

## M. Diagnostics Ã¢ÂÂ `FR-DIAG`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-DIAG-01` | provide structured, levelled logging of transport, CAT frames, and session events. | STK-17 | S | T | Log records carry level, timestamp, category; CAT frames optionally traced. |
| `FR-DIAG-02` | offer a raw CAT command/console view for troubleshooting (send arbitrary command, see raw RESP). | STK-17 | C | D | Operator can send `IF;` and see the raw reply. |
| `FR-DIAG-04` | show the diagnostics console in a **separate window**, off by default, toggled from Settings. | STK-13 | C | D | Settings toggles a detached diagnostics window; closing the main window quits. |
| `FR-DIAG-03` | never log secrets (passwords) or full audio payloads. | STK-14/17 | M | I | Inspection confirms redaction. |

## N. Radio configuration commands (screen support) Ã¢ÂÂ `FR-EQ / FR-KEY / FR-AUD-CFG / FR-ANT / FR-MENU`

*Control capabilities behind the K4 on-screen configuration screens
([../concept/k4-screens.md](../concept/k4-screens.md), `FR-UI-19`). Command
syntax per the Programmer's Reference D12, cross-checked vs QK4 (`R-EXT-03`).*

| ID | Statement (the system shallÃ¢ÂÂ¦) | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-EQ-01` | control the **8-band graphic equalizer** for RX (per receiver) and TX Ã¢ÂÂ set each 100Ã¢ÂÂ3200 Hz band's gain (Ã¢ÂÂ16..+16 dB) and flatten (PRG `RE`/`TE`/`REF`). | STK-03/06 | S | T | `RE`/`TE` encode 8 signed 3-char band fields within ÃÂ±16; `REF;` flattens. |
| `FR-KEY-01` | configure the **CW keyer**: weight, paddle normal/reverse, iambic mode A/B, and speed (PRG `KP`/`KS`). | STK-07 | S | T | `KP`/`KS` encode within range (weight 90Ã¢ÂÂ125, speed 8Ã¢ÂÂ100 WPM). |
| `FR-KEY-02` | set the **CW sidetone/pitch** 250Ã¢ÂÂ950 Hz (`CW`, ÃÂ10). | STK-07 | C | T | `set_cw_pitch(600)` emits `CW60;`, clamps to 250Ã¢ÂÂ950 Hz. |
| `FR-CW-SPOT-01` | trigger **autospot** (`SP3`) to auto-tune the RX onto a CW signal. | STK-03 | C | T | `set_spot(3)` = `SP3;`. |
| `FR-AUD-CFG-01` | configure **transmit audio input/output**: mic input source, mic gain, mic preamp/bias/buttons, and line in/out levels (PRG `MI`/`MG`/`MS`/`LI`/`LO`). | STK-06 | S | T | Each encoder emits the documented field layout within range. |
| `FR-ANT-01` | select the **transmit antenna** (ANT1Ã¢ÂÂ3) and per-receiver **RX antenna** (PRG `AN`/`AR`). | STK-02 | C | T | `AN`/`AR` encode within range. |
| `FR-MENU-01` | access the radio's **configuration menu** by item id Ã¢ÂÂ open, query definition, and set a menu parameter (PRG `MO`/`MEDF`/`ME`). | STK-11 | C | T | `MO`/`MEDF`/`ME` encode the 4-digit id (and value for set). |
| `FR-SW-01` | **emulate front-panel switch** tap/hold by code (PRG `SW`) to reach functions that have no dedicated CAT command Ã¢ÂÂ notably quick memories M1Ã¢ÂÂM4 (recall/store) and PF1Ã¢ÂÂPF4 (the memory-channel `MC` command being pending in D12). | STK-02/11 | C | T | `switch(code)` encodes `SW<code>;`; the quick-memory/PF/switch tables carry the correct codes. |
| `FR-SCAN-01` | **start/stop memory scan** (`SW149`) and display scan-in-progress from the `IF` `s` flag. | STK-02 | C | T | The `IF` `s` field (index 29) sets `scanning`; the SCAN control emits `SW149;` and lights while scanning. |
| `FR-VOX-01` | control **VOX** on/off per transmit mode (PRG `VX`). | STK-06 | C | T | `set_vox(mode,on)` encodes `VX<mode><0/1>;`. |
| `FR-VOX-02` | adjust **VOX gain** (`VG`) and **anti-VOX** (`VI`) levels. | STK-03 | C | T | `set_vox_gain('V',20)`=`VGV020;`, `set_antivox(15)`=`VI015;`. |
| `FR-FM-01` | in FM, set the **repeater offset** (`RP`) and **PL/CTCSS tone** (`PL`). | STK-03 | C | T | `set_repeater('+',600)`=`RP+00600;`, `set_pl_tone(13,true)`=`PL131;`. |
| `FR-TX-MSG-01` | **send CW/DATA text messages** for transmission (PRG `KY`, Ã¢ÂÂ¤60 chars). | STK-07 | C | T | `send_text(text)` encodes `KY <text>;` and truncates to 60 chars. |
| `FR-DVR-01` | play a **DVR voice message** 1–8 (`PB`), or cancel (`PB0`). | STK-03 | C | T | `set_dvr(1)`=`PB1;`, `set_dvr(0)`=`PB0;`. |
| `FR-TXT-01` | enable **text decode** for the active receiver (`TD` mode/threshold/lines) and display the decoded receive text polled from the buffer (`TB`, `s` field, `;`-safe). | STK-07 | C | T | `set_text_decode(2,0,3)` emits `TD203;`; a `TBÃ¢ÂÂ¦` RESP appends its text to `decode_text`, preserving embedded `;`. |
| `FR-PWR-01` | provide **remote power control** Ã¢ÂÂ **power off** (PRG `PS0`) and **restart** (`PS8`) Ã¢ÂÂ with the power-off action **guarded** against accidental activation (explicit confirm). Powering the radio **on** is not possible via CAT (the interface is unpowered when off, per D12). | STK-11 | C | T/D | `set_power(n)` encodes `PS<n>;` (0/8/88); the UI power-off requires a two-step confirm (demo). |

---

## Non-Functional Requirements Ã¢ÂÂ `NFR`

| ID | Statement | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `NFR-PERF-01` | Control round-trip latency (UI action Ã¢ÂÂ command sent Ã¢ÂÂ RESP applied) shall be Ã¢ÂÂ¤150 ms on a LAN, excluding network RTT. | STK-01/02 | M | T/A | Measured engine-side latency budget met in benchmark harness. |
| `NFR-PERF-CW` | Local CW keying jitter introduced by the client (paddle event Ã¢ÂÂ `KZ` emission) shall be Ã¢ÂÂ¤10 ms typical. | STK-07 | M | T | Timing test: emission delay distribution within bound. |
| `NFR-PERF-AUDIO` | End-to-end added audio latency from client buffering shall be Ã¢ÂÂ¤120 ms (target; tunable jitter buffer). | STK-05 | M | A/D | Measured/configured buffer within target; documented trade-off. |
| `NFR-PERF-AI` | The state model shall absorb AI5 update bursts without unbounded queue growth or UI stall. | STK-04 | M | T | Sustained burst test: bounded memory, no dropped UI refresh deadline. |
| `NFR-REL-FAILSAFE` | On any link loss while transmitting, the client shall reach a non-transmitting safe state within Ã¢ÂÂ¤1 s. | STK-08 | M | T | Fault-injection test meets the time bound. |
| `NFR-REL-01` | The client shall not crash on malformed/unknown CAT input; it shall degrade gracefully and keep the session. | STK-17 | M | T | Fuzz/garbage input test: no panic, parser resyncs. |
| `NFR-SEC-01` | The remote connection password shall never appear in logs, crash reports, or telemetry. | STK-14 | M | I/T | Log/redaction test passes; inspection confirms. |
| `NFR-SEC-02` | The application shall treat the network link as untrusted and document/recommend a secure tunnel (VPN) for Internet use. | STK-14 | S | I | Documentation present; no secret sent before handshake auth. |
| `NFR-SEC-03` | Master-password encryption of cached peer passwords (`FR-CFG-04`) shall use a memory-hard KDF (Argon2) with a per-store random salt and an authenticated cipher (AEAD, e.g. ChaCha20-Poly1305) with a per-secret random nonce; a wrong master password shall fail decryption authentication rather than yield plaintext. | STK-14 | S | T | Tampered ciphertext or a wrong master password fails the AEAD tag check; salt/nonce are random per store/secret. |
| `NFR-USE-01` | Operating-critical state (frequency, mode, TX/RX) shall be readable at a glance and update within 200 ms of a change. | STK-11 | M | D | Usability demo against checklist. |
| `NFR-PORT-01` | The application shall build and run on Linux and at least one of Windows/macOS from a single Rust codebase. | STK-16 | S | D | CI builds the target platforms; app launches. |
| `NFR-PORT-02` | The application shall build and run on **Raspberry Pi OS (arm64/aarch64)**, **Linux x86_64**, **Windows x86_64**, and **macOS**, from the single Rust codebase. | STK-16 | S | D | Each target builds in CI and the app launches on it. |
| `NFR-PKG-01` | The project shall provide **distribution packaging**: a **Debian package** (`.deb`, x86_64 + arm64) and an **Arch `PKGBUILD`** for Linux, plus native bundles for **Windows x86_64** and **macOS**. | STK-16 | C | I/D | Each packaging recipe produces an installable artifact for its platform. |
| `NFR-MAINT-01` | The codebase shall be organised into independently testable layers (transport, CAT, state, audio, UI) with no UI dependency in the protocol core. | STK-15 | M | I | Dependency check: protocol crate has no UI/iced dependency. |
| `NFR-MAINT-LOG` | Diagnostic logging shall be sufficient to reconstruct a failed session's command/event sequence. | STK-17 | S | I | Replay from logs demonstrated. |
| `NFR-TEST-01` | Every `M` and `S` functional/non-functional requirement shall be covered by Ã¢ÂÂ¥1 automated test referencing its ID (rule R3/R4). | STK-15 | M | I | Traceability gate `xtask trace` green. |
| `NFR-TEST-02` | The protocol/state core shall be testable without real hardware via a transport mock / K4 protocol simulator. | STK-15 | M | T | Full CAT/state test suite runs with no hardware. |

---

## Open points / to resolve before "Approved"

- ~~`OP-1`~~ **Resolved by `R-EXT-01`** (single multiplexed socket; ports 9205/9204; framing + PAN/audio layouts). Confirm on real radio (`ASM-05`).
- ~~`OP-2`~~ **Resolved:** TX/RX share the `EM` encode negotiation; audio is one multiplexed stream over the control socket (not separate ports).
- ~~`OP-3`~~ **Partially resolved:** audio is **12 kHz** (RX stereo, TX mono), Opus VOIP, frame size per `SL` tier. Device-selection UX now specified (`FR-AUD-DEV-01`, in the settings dialog `FR-UI-23`).
- `OP-4` Decide CW source: physical paddle via serial/USB at the client, on-screen, or keyboard Ã¢ÂÂ affects `FR-TX-CW-01` input layer. *(QK4 supports hardware keyers/K-Pod; out of our v1 scope.)*
- `OP-5` Confirm required regulatory identification behaviour for `STK-13`/`FR-VFO-ID`.
- `OP-6` Choose default transport security: plaintext+SHA-384 (9205) vs TLS-PSK (9204) for Internet use (`NFR-SEC-02`, `FR-AUTH-02`).

## Change history

| Date | Ver | Author | Change |
|---|---|---|---|
| 2026-06-25 | 0.1 | DC0SK | Initial draft baseline (FR/NFR). |
| 2026-06-25 | 0.2 | DC0SK | Added FR-STREAM/FR-AUTH groups; unblocked FR-AUD-04/FR-PAN-01 with concrete layouts; resolved OP-1..3; added OP-6. |
| 2026-06-26 | 0.3 | DC0SK | Added FR-UI-08..14 (view mode, dot-grouped freq, semantic colour, two-line state buttons, A/B symmetry, primary+context model, mini-pan) from R-EXT-02 / ui-design.md / ADR-15. |
| 2026-07-02 | 0.4 | DC0SK | Added FR-UI-15 (reference-faithful dark layered theme + proportional S-meter bar) Ã¢ÂÂ visual-identity pass per updated ADR-15 direction. |
| 2026-07-02 | 0.5 | DC0SK | Added FR-UI-16 (phase-driven Connect/Cancel/Disconnect control; a connection attempt is cancellable and runs off the UI/worker blocking path). |
| 2026-07-02 | 0.6 | DC0SK | Added FR-UI-17 (theme selector: dark/light/contrast/system) and FR-UI-18 (About box: author/license/URL). Also fixed dual-pane spectrum height to match single view (FR-UI-12). |
| 2026-07-02 | 0.7 | DC0SK | Added FR-UI-19 (primary softkeys open a K4 config screen in place of the spectrum frame; controls/lower panels stay). Corrected scope after a wrong first attempt that replaced the controls box + duplicated existing controls. |
| 2026-07-02 | 0.8 | DC0SK | Phase-0 config-screen commands: added ÃÂ§N (FR-EQ-01, FR-KEY-01, FR-AUD-CFG-01, FR-ANT-01, FR-MENU-01) + FR-VFO-07 (VFO copy/swap); extended FR-VFO-04 (direct band/stack/XVTR) and FR-PAN-CTL-01 (full `#` display family). CAT encoders + tests added to `k4-protocol` (`docs/concept/k4-screens.md` ÃÂ§3.2). |
| 2026-07-02 | 0.9 | DC0SK | Added FR-SW-01 (front-panel switch emulation `SW`) Ã¢ÂÂ enables quick memories (M1Ã¢ÂÂM4) and PF1Ã¢ÂÂPF4 remotely, since the `MC` memory-channel command is pending in D12. |
| 2026-07-02 | 0.10 | DC0SK | Added FR-VOX-01 (VOX `VX`) and FR-TX-MSG-01 (CW/DATA text send `KY`). Completed outbound-only screen work: BAND XVTR (`XV`), TX TEXT/VOX tabs, Fn Switches + DX-list tabs. |
| 2026-07-03 | 0.11 | DC0SK | Added FR-UI-20 (config screens seed from the radio on connect Ã¢ÂÂ read-back). FR-AUTH-02 (TLS-PSK, port 9204) now **implemented** and verified live against a real K4 (`connect_tls`): learned the exact PSK scheme (identity empty, key = raw password bytes, TLS 1.2 `PSK-AES256-CBC-SHA384`) and fixed a handshake-timeout + OpenSSL-security-level bug. Keychain I/O moved off the UI thread (FR-UI-07 hardening). |
| 2026-07-03 | 0.12 | DC0SK | UI polish: extended FR-UI-18 (About now shows version + donate link + openable license/project/donate links); added FR-UI-21 (landscape default window) and FR-UI-22 (phase-coloured connection indicator). Also: TX box CLR sized to match SPLIT/RIT/XIT (2ÃÂ2 grid). |
| 2026-07-03 | 0.13 | DC0SK | Recorded new (proposed) requirements: NFR-PORT-02 (targets: RPi OS arm64 / Linux x86_64 / Windows x86_64 / macOS) + NFR-PKG-01 (Debian .deb, Arch PKGBUILD, Windows/macOS bundles); FR-UI-23 (application settings dialog housing the connection form + audio controls); FR-AUD-DEV-01 (RX/TX audio-device selection dropdowns); FR-AUD-LVL-01 (RX volume + TX mic-level sliders). Not yet implemented. |
| 2026-07-06 | 0.14 | DC0SK | Added FR-PWR-01 (remote power off `PS0` / restart `PS8`, guarded power-off). No power-on via CAT (D12). Control on the Fn Ã¢ÂÂ SWITCHES tab. |
