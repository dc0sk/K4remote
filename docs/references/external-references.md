---
title: "External References"
status: Draft
version: "0.3"
updated: 2026-07-02
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
