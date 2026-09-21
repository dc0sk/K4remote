---
title: "External References"
status: Draft
version: "0.8"
updated: 2026-09-21
authors:
  - Simon Keimer (DC0SK)
owns: [R-EXT]
---

# External References

External projects and resources that inform the K4 Remote design. Vendor PDFs/HTML in this
folder are the primary normative source; the items below are secondary/community sources.

| Date added | Author |
|---|---|
| 2026-06-25 | DC0SK |

---

## R-EXT-01 — QK4 (mikeg-dal)

- **URL:** https://github.com/mikeg-dal/QK4
- **What:** A mature, working **remote control application for the Elecraft K4** — exactly our
  problem domain. Acts as a software K4/0: CAT control, full-duplex Opus audio, GPU spectrum +
  waterfall, CW keying (incl. hardware keyers/K-Pod), DX cluster, KPA1500 control.
- **Stack:** C++ / **Qt 6.7+** (Multimedia, ShaderTools, SerialPort, Svg, Qt RHI for GPU render),
  **libopus**, OpenSSL (TLS/PSK), CMake. Targets macOS / Windows / Linux / Raspberry Pi.
- **License:** **GNU GPL v3.0**, © 2025–2026 Mike Garcia.

### ⚠️ Licensing constraint (read before using)

QK4 is **GPLv3**. To keep K4 Remote under a license of our choosing, we treat QK4 as a
**reference for protocol facts and architecture ideas only**:

- **Protocol facts** (byte offsets, framing, port numbers, hashing scheme, command sequences)
  are **interoperability information** — facts, not copyrightable expression — and may be
  reimplemented freely (clean-room) in Rust.
- We do **NOT** copy, translate, or transliterate QK4 source code, comments, or structure into
  this project. Implementation is independent and verified against the vendor docs and a real
  radio.
- Architectural *patterns* (layering, threading model) are used as inspiration, not derivation.

> Recorded as a project rule. See `CON-09` in vision-and-scope.

### Consulted again 2026-07-21 — remote audio level

While diagnosing near-silent received audio, QK4's `src/audio/audioengine.cpp` was read to see
how another client handles the K4's stream level. **Facts taken** (interoperability observations,
no code): QK4 leaves its own output at unity gain and treats the K4's `AG` command as the volume
control, and its per-receiver MAIN/SUB sliders **attenuate only** (clamped 0..1) rather than
boosting. The second of those informed our decision to make the per-pane `VOL` a trim rather than
a second boost stage — an architectural *idea*, used as this file permits.

Worth recording that the first observation **did not hold on DC0SK's K4**: measurement showed `AG`
sets the radio's own speaker volume in both directions (knob and over the link) and does **not**
change the streamed level, which sits near -45 dBFS regardless. So K4 Remote supplies the gain
itself. See the project memory note and `CHANGELOG.md` 0.5.0.

> **Compliance note.** One comment line from that file was quoted verbatim in a commit message
> (`feat(audio): more local gain headroom for a quiet stream`) as a citation for the claim. No QK4
> source, comment, or structure has been incorporated into this project's code. Flagged here
> because the rule above names comments explicitly.

---

### Why it matters most: the streaming protocol our vendor docs omit

The Elecraft Programmer's Reference says the streaming-data wire format is "available on
request" and does **not** publish it. QK4's authors reverse-engineered it via direct socket
sessions against a real K4/0 server and documented it. This **resolves `RISK-01`** (previously
the top project risk) by giving us a concrete, testable target for v1 audio and Phase-2
spectrum. Treat the following as **community-verified, not vendor-official** — confirm against
the real radio (`ASM-03`/`ASM-05`).

---

## K4/0 streaming & remote protocol — extracted facts

*(Source: QK4 `src/network/protocol.{h,cpp}`, `tcpclient.cpp`, `dsp/panadapter_rhi.cpp`,
`dsp/rhi_utils.h`, `audio/*`, `docs/k4-protocol-quirks.md`. Reimplement clean-room.)*

### Transport & ports

| Port | Mode | Auth |
|---|---|---|
| **9205** | Unencrypted TCP | App writes **SHA-384(password)** as a **lowercase hex string**, raw (NOT framed), immediately after TCP connect. The radio replying with any framed packet = auth success. |
| **9204** | **TLS 1.2+ / PSK** | Password is the **PSK key**; identity optional. Cert verification off (PSK, no certs). Offer only TLS1.2+ PSK ciphers. |

- Connection timeout 10 s; auth timeout 5 s.
- **Keep-alive:** send `PING<unixEpochSeconds>;` once per second; reply `PONG…` is used to
  measure latency. (Note: timestamped `PING`, not bare `PING;`.)
- **Disconnect:** send `RRN;`.

### Connect/auth handshake sequence (client → server)

1. TCP connect (or TLS-PSK handshake on 9204).
2. *(9205 only)* write SHA-384 hex of password, raw + flush.
3. On first inbound packet → authenticated. Then send, in order:
   1. optional **startup macro** (before RDY, so the dump reflects it),
   2. `RDY;` — triggers a **comprehensive state dump** of the whole radio state,
   3. `K41;` — enable K4 (advanced) protocol mode,
   4. `ER1;` — enable long-format error messages,
   5. `EM<n>;` — audio encode mode (see below),
   6. `SL<n>;` — streaming-latency tier.

### Binary frame format (wraps **all** payloads)

```
[ START_MARKER 4B ][ payload length 4B ][ payload ][ END_MARKER 4B ]
  FE FD FC FB        big-endian u32                  FB FC FD FE   (mirror image)
```

- Header+trailer = 12 bytes. `payload[0]` = **PayloadType**.
- Parser must reassemble across TCP read boundaries; if no start marker is found, **retain the
  last 3 bytes** (a 4-byte marker may be split across reads).
- Buffer cap 1 MB → clear on overflow. Bad end marker → skip 4 bytes, resync.

### Payload types (`payload[0]`)

| Value | Type |
|---|---|
| `0x00` | CAT (ASCII) |
| `0x01` | Audio (Opus/PCM) |
| `0x02` | PAN (panadapter/spectrum) |
| `0x03` | MiniPAN |

### CAT payload (`0x00`)

- Inbound: `[0x00][0x00][0x00][ASCII response…]` → ASCII starts at offset **3**.
- Outbound build: `[0x00][0x00][0x00][ASCII command incl. ';']`.

### PAN packet (`0x02`) — the spectrum

| Offset | Field | Type |
|---|---|---|
| 0 | type | u8 (=0x02) |
| 1 | version | u8 |
| 2 | sequence | u8 |
| 3 | pan type | u8 |
| 4 | receiver | u8 (0=Main/VFO A, 1=Sub/VFO B) |
| 5 | data length | u16 **LE** |
| 7 | reserved | 4 bytes |
| 11 | center freq | i64 **LE**, Hz |
| 19 | sample rate | i32 **LE** → **tier span Hz = sampleRate × 1000** |
| 23 | noise floor | i32 **LE**, **÷10 = dB** |
| 27 | bins… | 1 byte per bin to end of payload |

- **Bin value → dBm: `dBm = raw_byte − 146.0`** (`K4_DBM_OFFSET = 146.0`). Bin count =
  `payload_len − 27`.
- Cropping to a narrower display span = take the **center** `span/tierSpan × totalBins` bins.

### MiniPAN packet (`0x03`)

| Offset | Field |
|---|---|
| 0 | type (=0x03) |
| 1 | version |
| 2 | sequence |
| 3 | reserved |
| 4 | receiver (0/1) |
| 5 | bins… (1 byte/bin, same `−146` dBm mapping) |

### Audio packet (`0x01`)

| Offset | Field | Type |
|---|---|---|
| 0 | type (=0x01) | u8 |
| 1 | version (0x01) | u8 |
| 2 | sequence | u8 (0–255 wrapping, for ordering) |
| 3 | encode mode | u8 (`0`=RAW32, `1`=RAW16, `2`=Opus int16, `3`=Opus float) |
| 4 | frame size | u16 **LE** = samples/channel (matches SL tier) |
| 6 | sample-rate code | u8 (`0` = 12000 Hz) |
| 7 | audio data | format per encode mode |

- **RX audio:** 12 kHz **stereo** Opus — **left = Main, right = Sub**.
- **TX audio:** 12 kHz **mono**; frame size reconfigured per SL tier.
- Default frame 240 samples (= 20 ms @ 12 kHz). Opus app type = VOIP.
- Maps to the documented `EM` command (`EM3` Opus-float is the practical default).

### Streaming-latency tiers (`SL`)

- Tiers ≈ **20 / 40 / 60 / 120 ms** of audio bundled per packet. `SL` is **not echoed** and has
  no query form → set it optimistically and mirror locally.

### CW keying

- Uses the `KZ` family (`KZ.`/`KZ-`/`KZ<space>`/`KZP` pause/`KZL` length) per the Programmer's
  Reference, carried as CAT. (QK4 also supports hardware keyers/K-Pod — out of our v1 scope.)

### CAT quirks worth inheriting (verified by QK4 against a real K4)

- **`$` = sub-RX/VFO B** variant for many commands (`MD$`, `BW$`, `RO$`, `#REF$`…). Dispatch
  must match **longest prefix first** so `RO$` beats `RO`.
- **RIT/XIT offset register routing** (`RO` vs `RO$`) depends on split/BSET state; `RU`/`RD`
  adjust whichever is active; `XT1;`/`XT0;` set forms are **not echoed**; clear `RO$` via
  `RO$+0000;`.
- **Tolerant parser** is essential — unknown frames/commands must not desync the stream.

---

## Ideas to adopt vs. leave

| Adopt | Leave / re-decide for our stack |
|---|---|
| Layered split: transport ↔ binary protocol ↔ radio-state ↔ controllers ↔ UI | Qt-specific signal/slot wiring (we use iced messages + channels) |
| Single authoritative `RadioState` updated from the `RDY;` dump + Auto-Info | Qt RHI shaders for spectrum (we evaluate iced `Canvas`/`wgpu` — `RISK-04`) |
| Dedicated I/O thread; audio on its own thread; jitter buffer (~40 ms prebuffer) | C++ memory/threading model |
| Optimistic local mirroring of non-echoed sets (`SL`) | — |
| An embedded **CAT server** for WSJT-X/logger integration (nice Phase-3 idea) | Not in our v1 scope |
| Verifying every protocol fact via direct socket sessions before trusting it | — |

## Open confirmations (against a real radio — `ASM-05`)

- Confirm port behaviour (9204 TLS-PSK vs 9205 SHA-384) on current firmware.
- Confirm PAN `version`/`pan type` field semantics and whether bins are ever >1 byte.
- Confirm the `RDY;` dump contents/ordering for our state-seed (`FR-CAT-07`).

---

## R-EXT-02 — UI design references (K4 front panel + third-party panels)

Visual/UX references that inform the GUI design. These are the source for the
`FR-UI-*` layout requirements and `docs/concept/ui-design.md`. **Two distinct
provenance classes — they are not treated the same:**

### A. Elecraft K4 native LCD — *normative, interoperability-faithful*

- **Sources (in `docs/references/external/`):**
  - *Intro to the Elecraft K4, rev C5* — clean drawn renders of the 7" LCD in
    every operating state (pp. 8–44): display zones, dual panadapter, mode/band
    grids, RX/TX config rows, S-meter, RIT/XIT box, mini-pan, keyboard, memories,
    EQ, status display.
  - *K4 product photo* (`k4-front-white-bg-product.webp`) — front-panel layout.
- **Author / status:** Elecraft (vendor). The K4's own on-screen layout, colour
  semantics, and control grouping.
- **Use:** Reproducing the **K4's own UI conventions** is *interoperability
  faithfulness for the operator*, the same class as matching the CAT/streaming
  protocol — an operator who knows the K4 should recognise our panel. We adopt the
  **functional layout and semantics** (A/B symmetry, the shared TX/RIT box between
  VFOs, the 7-primary + context-row model, semantic colours, two-line state
  buttons, switchable single/dual panadapter). These are facts of the radio, not
  third-party expression.

### B. K4-Control for iOS (Roskosch) — *secondary, adapt-don't-clone*

- **Sources:** `K4-Control for iOS.pdf` (the app's manual, incl. iPad/iPhone
  screenshots) and `main-window-ipad.png`.
- **What:** A mature **commercial** third-party iPad/iPhone remote panel for the
  K4 — our problem domain on a touch device. Confirms that the K4's conventions
  translate well to a software panel (segmented `Waterfall/Modes/Tools` switcher,
  gradient S-meter, dual-VFO mirror, FT8/CW decode screens, connection flow on
  port 9205).
- **Provenance constraint (cf. `CON-09`):** this is **proprietary**. We adopt
  its **UI conventions and visual language** — dark layered surfaces, grids of
  rounded state buttons with a blue "engaged" fill, big white frequency
  readouts, proportional S-meter bars, panel groupings (per the 2026-07-02
  direction, see `ADR-15` rev.) — re-implemented from scratch with **our own
  values** (palette constants, spacing, widget code). We do **not** copy its
  assets, iconography, or branding, and we do not extract or reuse any of its
  artwork or code.

### What we adopt vs. deliberately diverge

| Adopt (reference-faithful) | Diverge (ours) |
|---|---|
| A-left / B-right symmetry; shared TX/SPLIT/RIT-XIT box between the VFOs | Own palette *values* & widget code (re-implemented, no copied assets/icons/branding) |
| Switchable view: single-A · single-B · dual (mirrors `PAN=A/B/A+B`) | Resizable desktop window with responsive stacking; narrow width stacks bands |
| 7 fixed primary buttons → swap a context sub-row above them | A real menu/settings panel instead of a locked scroll list |
| Semantic colour: amber=TX/transmit, blue=A/main, green=B/sub, white=RX | Mouse/widget controls; drop hardware-knob metaphors (XMTR/FILTER/RF-SQL) |
| Two-line state buttons (`LABEL` + live value); dot-grouped freq readout | Explicit TX arm / emergency-stop affordances (our `FR-TX-SAFE` additions) |
| Dark layered theme; rounded button grids with blue "engaged" fill; big white freq readouts; proportional S-meter bar (iOS app visual language, 2026-07-02 direction) | — |
| Tap-to-edit; mini-pan zoom tuning aid; dual-pan; vertical scale in dBm/S-units | — |

### Mini-pan availability (`#MP$`) — field-established, not in the vendor docs

`#MP$-1` means "the mini-pan cannot be turned on with the current radio
settings" (D12). Neither D12 nor D14 says *which* settings. Established on a
real K4 (2026-07-20, DC0SK):

> **Dual-pan must be off when the sub receiver is disabled.** With dual-pan on
> and no sub RX, the radio refuses with `#MP$-1`; turning dual-pan off — or
> enabling the sub RX — allows the mini-pan.

Consistent with D14 p.1489, which describes tapping an S-meter as switching to
the mini-pan *for that receiver*: the mini-pan occupies a receiver's meter area,
so with dual-pan on and only one receiver there is nowhere for it to go.

### Open confirmations (against the real radio — `ASM-05`)

- Confirm the exact on-screen colour semantics (esp. the orange/amber transmit
  family and the blue/green A/B coding) against current firmware.
- Confirm `PAN=A/B/A+B` selection maps cleanly onto our `ViewMode` and the
  per-receiver PAN packets (`receiver` field, `R-EXT-01`).

---

## R-EXT-03 — Elecraft vendor documents (normative)

The authoritative Elecraft sources held in `docs/references/external/`. Unlike
QK4 (`R-EXT-01`, GPL — facts only) these are vendor documents; we use them as the
**normative** source for CAT commands and radio behaviour, reimplemented
clean-room per `CON-09` (facts/interoperability, not copied text).

| Document | Rev | Provides | Used for |
|---|---|---|---|
| **K4 Programmer's Reference** (`.pdf` / `.html`) | D12 | The full CAT command set: mnemonics, SET/GET/RESP syntax, ranges | **Normative CAT source** — resolves the command gaps in `concept/k4-screens.md` §3 (EQ, keyer, mic, line, band, `#`-display) and the `FR-CAT`/`FR-VFO`/`FR-RX`/`FR-TX` encoders |
| **K4 Command Index** (by-description RevD5; RevD3) | D5/D3 | Quick command lookup by function/description | Fast mnemonic lookup while wiring screens |
| **K4 Built-In Operating Manual** | D14 | Full operating behaviour of every feature/screen | Authoritative behaviour reference behind the screen specs (`concept/k4-screens.md`) and `FR-UI-*` |
| **Intro to the Elecraft K4** | C5 | Drawn renders of the 7″ touchscreen in every state | Source of the on-screen screen catalog (`R-EXT-02`, `concept/k4-screens.md`) |
| *Remote K4 On-Off Control Methods* | — | Remote power-on/off methods | `FR-CONN` / power-control (future) |

- **HTML vs PDF:** `K4ProgrammersReferencerev.D12.html` is a Google-Docs export
  (heavy inline CSS) — prefer the `.pdf` (or the Command Index) for lookup; the
  `.html` is convenient for text search once the `<head>` CSS is stripped.
- **Resolution rule:** any CAT command flagged "to confirm" in a spec or code
  comment is resolved against the **Programmer's Reference D12** here, then
  verified against a real radio (`ASM-05`) before being marked confirmed.

---

## R-EXT-04 — SDRoxide (spot-marker design inspiration)

- **What:** A desktop SDR transceiver application installed on the author's machine
  (`/usr/bin/sdroxide`, docs in `/usr/share/doc/sdroxide/README.md`). Consulted 2026-09-19 for how it
  presents **spotting-network spots as markers on the panadapter**.
- **License:** ships several license files — GPL-3.0-or-later plus bundled third-party components
  (AGPL-3.0, GPL-2.0, BSD, MIT). Treated as **copyleft**, the same clean-room posture as
  `R-EXT-01` / `CON-09`: ideas and observable behaviour only; no source, comments or structure copied.
- **Facts taken** (observable behaviour, restated in our own words): spots from several networks
  (DX cluster, POTA, SOTA, PSK Reporter, RBN, WSPRnet, FreeDV Reporter) are shown as clickable,
  colour-coded markers along the spectrum; each network is enabled separately with its own host/port
  or poll interval; there is one **maximum spot age** (its default is 15 min) and a **current-band-only**
  option; clicking a spot tunes to it. These informed `FR-SPOT-03/-04/-10/-11`.
- **Not taken:** it also *uploads* reception reports; K4 Remote deliberately does not (`FR-SPOT-12`).
- **Caution:** its configuration directory holds the operator's account credentials in plain text.
  Only the *structure* of that file was used; nothing from it belongs in this repository.

## R-EXT-05 — Spotting networks

The candidate sources for `FR-SPOT-*`. Each is read from its own published documentation before a
source is built (`OP-7`). **Read: the Reverse Beacon Network, the DX-cluster line format, PSK Reporter,
POTA, SOTA, WSPRnet and FreeDV Reporter** — POTA, SOTA, WSPRnet and FreeDV Reporter on 2026-09-21, with
the gaps recorded under each.

### Reverse Beacon Network and DX-cluster telnet feeds (read 2026-09-19)

**Primary, from RBN itself** — <https://www.reversebeacon.net/pages/Telnet+servers+30>:
- `telnet.reversebeacon.net` port **7000** carries CW and RTTY spots, port **7001** carries FT8.
- These are "stripped down relay servers specifically designed for maximum throughput" with **no
  filtering features**.
- **Stated intent — this matters:** RBN says "retail" DX clusters should connect to these nodes and
  that **end-users should connect to the retail nodes**. A desktop client connecting straight to the
  relay goes against that stated intent, however common the practice (see the SDRoxide cross-check).
  Whether K4 Remote should is a **decision for DC0SK**, recorded in `OP-7`.

**Line format — third-party, not RBN:** RBN's own pages (the telnet page, "Get Smart About the RBN")
do **not** give the spot-line format, the login prompt, or any rate guidance. The format below comes
from a user manual for AR-Cluster software (W9ZRX, "Using AR-Cluster V6",
<https://www.k3lr.com/w9zrx/Using%20AR-Cluster%20V6.pdf>), which shows three lines:
a hand-entered spot and two skimmer spots. In our own words: the layout is
`DX de <spotter>: <frequency> <callsign> <comment> <time>Z`; a skimmer spotter is marked by `-#` on
its callsign; a skimmer's comment carries mode, signal-to-noise in dB, speed in WPM or BPS, and
whether the station is calling CQ. The **frequency unit is not stated** — kilohertz is inferred from
the band plan (7000.7 on 40 m, 3580.9 on 80 m, 18140.0 on 17 m). The time is `HHMMZ`, **a time of
day with no date**. The short token before the time on those lines (`+KP2`, `OM`) is a per-user
display option in that software and may not be present on the relay. The parser reads whitespace
separated tokens, not fixed columns, to be independent of a server's padding.

**Observed directly, once (2026-09-19):** with DC0SK's permission, a single connection to
`telnet.reversebeacon.net:7000`, about ten seconds, **nothing sent**. The relay answered over IPv6 in
0.35 s with exactly `Please enter your call: ` — 24 bytes, **no newline, no telnet negotiation** —
and then waited. It did not close the connection within ten seconds. This is one observation of one
server, not a specification.

**Not found in any source read (and so not yet known):**
- the login prompt of a **retail cluster** (the source accepts common wordings, untested),
- **rate or volume guidance** from RBN,
- whether RBN's relay accepts filter commands (see below),
- what the relay sends after login, and its real volume.

**Decision (DC0SK, 2026-09-19):** both routes stay selectable — an RBN entry with the relay prefilled
but off, and a DX-cluster entry — with a note on the RBN entry that RBN prefers end-users to connect to
retail clusters.

**Cross-check, SDRoxide** (R-EXT-04, its own shipped manual — documentation, not code): it connects to
the RBN relay directly by default with only the operator's callsign as the login; says the network
carries "thousands a minute"; and **keeps RBN spots out of its spot list** on purpose, because they are
measurements and not invitations to call, using them for a propagation map instead. It also says a
`set/filter` command narrows the feed — which **conflicts** with RBN's own "no filtering features" for
the relay, so that is treated as **unverified**.

**Searched and found not to contain the format** (recorded so nobody repeats it): RBN's "Get Smart
About the RBN" page; N6TV's 2015 CW-skimmer slides (image-only, no text); the HamPost skimmer guide.

### PSK Reporter (read and observed 2026-09-20)

Two interfaces, both public and both meant for this kind of use.

**Query API, documented** — <https://pskreporter.info/pskdev.html>:
- `GET https://retrieve.pskreporter.info/query`, response an XML `receptionReports` document of
  `receptionReport` elements. Documented parameters include `frange` (`lower-upper` in Hz),
  `flowStartSeconds` (a negative number of seconds, at most 24 hours), `mode`, `rptlimit`, `rronly`,
  `noactive`, `nolocator`, `senderCallsign` / `receiverCallsign` / `callsign` ("use only one of the
  three") and an optional `appcontact` (an email address, if you want the operator to be able to
  contact you). Documented attributes: `receiverCallsign`, `receiverLocator`, `senderCallsign`,
  `senderLocator`, `frequency` (unsigned integer, **Hz**), `flowStartSeconds` (**Unix seconds**), `sNR`
  (integer), `mode` (an ADIF MODE or SUBMODE).
- **Usage rules, in the page's terms:** retrieve "no more often than once every five minutes" (so all
  receivers have time to report); the operator "reserves the right to block or rate limit anybody who
  imposes a significant load", and may in future **require compression** from frequent users. **No
  numeric limit is stated.**
- **Not documented:** whether a query with no callsign parameter is allowed (see the observation
  below), and any MQTT feed.

**MQTT feed, documented** — <https://www.mqtt.pskreporter.info/>: host `mqtt.pskreporter.info`; ports
1883 (TCP), 1884 (TLS), 1885 (WebSocket), 1886 (WebSocket + TLS); topic
`pskr/filter/v2/{band}/{mode}/{tx_call}/{rx_call}/{tx_grid}/{rx_grid}/{tx_dxcc}/{rx_dxcc}` with the MQTT
wildcards `+` and `#`; variants `v2` (filtered), `v2raw` (unfiltered) and `v2raw_1pc` (a 1 % sample); a
flat JSON payload with `sq` (sequence), `f` (Hz), `md` (mode), `rp` (SNR, dB), `t` and `t_tx` (epoch
times), `sc`/`sl` (sender callsign and locator), `rc`/`rl` (receiver), `sa`/`ra` (DXCC) and `b` (band).
**Not documented:** authentication, rate limits, fair-use rules, and the band token's format.

**Observed directly (2026-09-20), minimal and capped, nothing identifying sent** (no `appcontact`, no
account; a random MQTT client id):
- **One HTTP request** for a 1 kHz `frange` with `flowStartSeconds=-900&rptlimit=5&rronly=1`: HTTP 200,
  `text/xml`, gzip when asked, `cache-control: public,max-age=90`, served through a CDN. **A query with
  no callsign works.** The response carries `lastSequenceNumber` and `maxFlowStartSeconds` elements, and
  reports with extra attributes beyond the documented ones (`senderDXCC`, `senderDXCCCode`,
  `senderDXCCLocator`, `senderLotwUpload`, `senderEqslAuthGuar`). **`frange` and `rptlimit` were not
  hard bounds:** reports well outside the requested range came back, and more records than `rptlimit`.
  A client must filter and cap the response itself.
- **One MQTT subscription, 1.4 s, capped at 40 KB**, to the whole `pskr/filter/v2/#` tree: plain TCP on
  port 1883 was accepted with no authentication (CONNACK 0, SUBACK QoS 0). **179 messages arrived in
  1.4 s — about 130 a second for the whole tree**, so an unfiltered subscription is a firehose. A real
  topic looks like `pskr/filter/v2/20m/FT8/AA1AAA/BB2BBB/FN31/JO50/291/230` (band token `20m`, mode
  `FT8`), and the payload like `{"sq":72916355844,"f":14074742,"md":"FT8","rp":-9,"t":1789899105,
  "t_tx":1789899090,"sc":"AA1AAA","sl":"FN31pr","rc":"BB2BBB","rl":"JO50ab","sa":291,"ra":230,"b":"20m"}`
  (callsigns replaced here). `sq` is on the same scale as the HTTP `lastSequenceNumber`.
- **Band tokens seen (a 3.9 s sample, 1 111 messages, ~290 a second):** `160m`, `80m`, `40m`, `30m`,
  `20m`, `17m`, `15m`, `12m`, `10m`, `2m`, `13cm`, `3cm`; every payload was flat JSON and the topic's band
  segment always equalled the payload's `b`. `60m`, `6m` and `4m` were **not seen** and are assumed to
  follow the same `<n>m` pattern — unverified. Modes seen: FT8, FT4, WSPR, FT2, CW.
- **A band-scoped subscription works:** `pskr/filter/v2/20m/#` (5.3 s) delivered **only** 20 m
  (277 of 277 messages, ~53 a second, ~12 KB/s) against 130–290 a second for the whole tree.
- **TLS, observed once (2026-09-21):** one TLS handshake with port 1884, then hang up — nothing sent after it, no MQTT, no login. **The certificate is trusted by the public authorities** (it verified against `webpki-roots`), so TLS needs no manual approval on the real service.
- These are single observations of a live service, not a specification.

**Decision (DC0SK, 2026-09-20):** K4 Remote uses the **live MQTT feed over plain TCP** (port 1883), not
the query API, with a hand-written client. It subscribes to `pskr/filter/v2/<band>/#` for the bands the
VFOs are on and to nothing when there is no radio; TLS (port 1884) followed (`FR-SPOT-13`). PSK Reporter
documents no rate or fair-use rules for the MQTT feed.

### POTA (read and observed 2026-09-21)

**Documentation:** the official API pages, <https://docs.pota.app/api/index.html>, are a placeholder
that says the content is under construction — **there is no published schema, rate limit or usage
rule.** The spot format below therefore comes from one observation, not from POTA's documents.

**Observed directly, once (2026-09-21):** one unauthenticated `GET https://api.pota.app/spot/activator`
with a plain `User-Agent` naming this project and nothing else. HTTP/2 200, `application/json`, 5 150
bytes, an `age: 8` header (so it is served through a cache), **no rate-limit headers**. The body is a
JSON **array** (12 spots at that moment) of flat objects with these fields: `spotId` (integer),
`activator` (string), `frequency` (**a string, in kHz** — `"10136.0"` for a 30 m FT8 spot),
`mode`, `reference` (park, e.g. `CA-0040`), `parkName` (null here), `spotTime` (`2026-09-21T05:07:00`,
**no zone designator — UTC is assumed, not stated**), `spotter`, `comments`, `source`, `invalid` (null
here), `name`, `locationDesc`, `grid4`, `grid6`, `latitude`, `longitude`, `count` and `expire`
(integer, 1796 here — read as seconds until the spot lapses, **unconfirmed**). The endpoint path came
from memory of other clients, not from a POTA document; the search hit for the bare `/spot` path was a
third-party proxy. This is one observation of one response, not a specification.

**A POTA spot is an invitation to call**, made by a person or a skimmer (`source`), not a reception
measurement: it is the same kind of thing as a DX-cluster spot and belongs with that group.

**Not found:** a polling interval POTA wants, whether the endpoint is meant for third-party clients,
what `expire` counts, whether `spotTime` is UTC, and what `invalid` and `source` can hold.

### SOTA (read 2026-09-21)

**Documentation, primary:** the API's terms page, <https://api2.sota.org.uk/docs>. In its words the
API is for "reasonable public usage", carries "reasonable usage limits" with **no number**, and users
who cause "undue load", use deprecated endpoints or "excessively" consume it may be blocked from SOTA
infrastructure. Commercial applications need a licence agreement. **Access condition:** "any
application developer (including developers of libraries that connect to the SOTA API)" must be a
member of the SOTA Reflector **and of its "API-consumers" group** before using the API. That is a
condition on the **developer**, is not something this project can satisfy on DC0SK's behalf, and was
**not** checked or attempted. A moderator's post in the API-consumers discussion says a separate
"database" API exists, is not considered stable and is not widely documented — it is not a candidate.

**Not found in any page read:** the spots endpoint path and response schema. The terms page names
spot fetching as the usage most often done badly, and a search snippet suggests
`https://api2.sota.org.uk/api/spots/<hours>` (with a hours limit of 72) — **third-party, unverified,
and not contacted**, because of the access condition above.

### WSPRnet (read 2026-09-21)

- **WSPRnet itself:** the downloads page (<https://www.wsprnet.org/drupal/downloads>) could not be
  read here (HTTP 403 to this tool). Search summaries say the whole database is published as monthly
  CSV files, and that its **API is available only by contacting the WSPRnet custodian**, with a stated
  rule not to repeat the same query more often than every 2 minutes and to avoid the 2-minute
  boundaries when uploads land. Both are **third-party summaries**, not read from WSPRnet.
- **wspr.live** (<https://wspr.live/>), a separate, third-party service that mirrors the spots: a
  read-only SQL-over-HTTP interface at `db1.wspr.live` (http port 80, https port 443), database `wspr`,
  table `rx`, with columns among them `time`, `band`, `frequency` (Hz), `rx_sign`, `tx_sign`, `rx_loc`,
  `tx_loc`, `snr`. Its stated terms: free for your own projects **as long as the results are accessible
  free of charge to everyone**; **no commercial or profit-oriented use**; **20 requests a minute**; and
  queries should be limited by time and band. It is not WSPRnet, and depending on it makes K4 Remote
  depend on someone else's volunteer service.
- **WSPR spots are reception reports** (a station heard a beacon), like PSK Reporter's, and in
  two-minute cycles. The PSK Reporter MQTT sample above already carried mode `WSPR`; **how much of
  WSPRnet that covers is not known**, so a separate source may add little. Not measured.

### FreeDV Reporter (read 2026-09-21)

- **Documentation: none found.** `qso.freedv.org` serves a web page; no protocol document was found.
  The wire protocol is defined only by the FreeDV project's own client, and the project's repository
  (`drowe67/freedv-gui`, default branch `master`) **does not contain the reporter client source at the
  path its build and dialog refer to** (`src/reporting/`) — that directory is absent from the tree, so
  the source itself was **not read**.
- **Secondary, from a third-party client that says it mirrors freedv-gui 2.1.0** (a pull request
  description for the Zeus project, one implementer's account, not FreeDV's): Socket.IO v4 over
  Engine.IO 4 on a WebSocket, one namespace, text frames only, no acknowledgements; after the open
  packet the client sends `40` plus an auth object; ping/pong with a watchdog; events are `42` frames;
  the protocol is called "protocol_version 2". Two roles: a **view** role that sends no personal data
  and is dropped after 60 s without a poll, and a **report** role for operators who opt in with
  callsign and grid. Events named there: `new_connection`, `remove_connection`, `freq_change`,
  `tx_report`, `rx_report`, `message_update`, `bulk_update`, `qsy_request`. The implementer tested live
  against the service. **Field names, frequency units and the auth object were not recorded.**
- **What a FreeDV Reporter entry is:** a station *currently on* a frequency (and whether it is
  transmitting or was heard), a live presence list — not a stream of timestamped spots. It fits the
  nameplate only if a station row is turned into a spot with its own age.
- **Consequence:** the service is reached over a Socket.IO/WebSocket connection, so a source needs a
  WebSocket client, which was absent from the tree before this work; a plain HTTP poll cannot do it.
  **An earlier draft of this note said TLS is also needed. That was an assumption, not something read
  or observed, and the cross-check below contradicts it.**

### SDRoxide cross-check (read 2026-09-21)

R-EXT-04's rule holds: nothing was copied, and its account configuration was not read. Its public
source was read for **interface facts only** — addresses, headers, intervals, message names — as was
done for RBN, at DC0SK's invitation ("how did SDRoxide solve the API issues").

- **POTA:** the same address as observed above, `api.pota.app/spot/activator`, polled on an interval
  with a floor of 15 s, a 10 s connect and 20 s overall timeout, and a `User-Agent` naming the program
  and its version. Nothing else is sent. This **confirms the endpoint** independently.
- **SOTA:** it fetches the last 50 spots from `api-db2.sota.org.uk/api/spots/50/all` — **not** the
  `api2` host whose terms page was read here. SOTA's own moderator described a separate "database" API
  as not considered stable and not widely documented (see above), so SDRoxide is using the interface
  SOTA says not to rely on. The request carries no credential or token of any kind. **SDRoxide does not
  solve the access condition; it does not appear to address it.** Whether its author is in the
  "API-consumers" group cannot be told from public information. Frequencies are MHz strings.
- **WSPRnet:** it talks to wsprnet.org directly. It **uploads** spots (`/post`, the callsign in the query
  is the identity, no authentication) — which K4 Remote never does (`FR-SPOT-12`) — and it **reads**
  `/drupal/wsprnet/spots/json` filtered by callsign, for spots *of* the operator's own callsign or
  *by* it. It has **no general band feed** from WSPRnet, so it does not solve the "spots for the whole
  view" problem either. Whether that read endpoint is what the custodian's "API" means is unknown.
- **FreeDV Reporter:** this answers most of the gaps above. A plain **`ws://qso.freedv.org:80/socket.io/?EIO=4&transport=websocket`**
  connection — **no TLS** — carries a Socket.IO connect frame whose auth object has `protocol_version`
  2 and a `role`: **`view`** with nothing else (read-only, no callsign), or `report` with callsign,
  grid, software version, `rx_only` and OS (which makes the station publicly visible; K4 Remote would
  only ever use `view`). Nothing may be sent before the server's `connection_successful` event. The
  server pings every 5 s with a 5 s timeout. Events: `new_connection` (a session id with callsign and
  grid), `remove_connection`, `freq_change` (the frequency in hertz), `tx_report`, `rx_report` (who
  heard whom, with SNR), `message_update`, `bulk_update` (the table on connect), `qsy_request`. A row
  becomes a nameplate when it has a callsign and a non-zero frequency. **Not tested here; from source
  only.**

### Summary — what is buildable from documentation

| Network | Documented? | Access | Needs |
|---|---|---|---|
| POTA | schema observed once, no rules | open, unauthenticated | HTTPS GET + JSON |
| SOTA | terms yes, schema no (SDRoxide uses the unstable `api-db2` host) | **developer must join API-consumers** | HTTPS GET + JSON, after that |
| WSPRnet | via third parties only | API by contacting the custodian; wspr.live open but third-party | HTTPS GET, or nothing |
| FreeDV Reporter | none; protocol from two other clients | open, read-only `view` role | WebSocket + Socket.IO (plain `ws`, as SDRoxide does; TLS unverified) |

### Not yet read
The retail DX-cluster login prompt and volume, and RBN's own guidance on rates — see above.
