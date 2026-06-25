---
title: "Vision & Scope"
status: Draft
version: "0.2"
updated: 2026-06-25
authors:
  - Simon Keimer (DC0SK)
owns: [ASM, CON, RISK]
---

# Vision & Scope

**Project:** K4 Remote — Rust remote control panel for the Elecraft K4
**Version:** 0.1 (Draft baseline) · **Date:** 2026-06-25 · **Author:** Simon Keimer (DC0SK)
Trace: this document owns `ASM-`, `CON-`, `RISK-`. Upstream of [stakeholder-requirements.md](stakeholder-requirements.md).

---

## 1. Problem statement

The Elecraft K4 is a high-performance HF/6 m transceiver that supports remote control over
Ethernet (the path used by Elecraft's own K4/0 remote front panel) and locally over
USB/RS232. There is no open, cross-platform, software-only remote panel that reproduces the
operating experience — frequency/mode control, metering, full-duplex audio, CW/voice
transmit, and (eventually) a spectrum + waterfall display — built on a maintainable,
test-driven Rust codebase.

## 2. Vision

> A maintainable, cross-platform Rust application that lets a single operator fully operate
> their K4 from a remote location: tune and configure the radio, hear receive audio, talk
> and send CW, watch the meters in real time, and — in a later phase — view a live spectrum
> and waterfall, all with the responsiveness and correctness expected of a real operating
> position.

## 3. Stakeholders

| ID | Stakeholder | Interest |
|---|---|---|
| SH-1 | **Operator / end user** (radio amateur, e.g. DC0SK) | Operate the K4 remotely with low latency and correct, safe behaviour |
| SH-2 | **Developer / maintainer** | Clear requirements, testable design, sustainable Rust codebase |
| SH-3 | **Elecraft (vendor)** | Owns the CAT and streaming protocols; source of the "on request" streaming spec |
| SH-4 | **Regulator (national PTT / amateur licence terms)** | Lawful transmit operation, identification, control of emissions |
| SH-5 | **Remote-site owner / network** | Bandwidth, security, reliability of the link |

## 4. Goals & success criteria

| Goal | Success measure |
|---|---|
| G1 Reliable remote control | Operator can change frequency/mode/key settings and see confirmed state within target latency (`NFR-PERF`) |
| G2 Real-time situational awareness | S-meter and key status update live via Auto-Info, no manual polling needed |
| G3 Full-duplex operating | Operator hears RX audio and can transmit voice and CW from the remote position |
| G4 Correctness & safety | TX is impossible without explicit, deliberate control; fail-safe on link loss |
| G5 Maintainability via TDD | Every must/should requirement has an automated test; traceability gate is green in CI |
| G6 Extensibility to spectrum | Phase-2 spectrum/waterfall integrates without re-architecting (clean streaming seam) |

## 5. Scope

### 5.1 In scope — Version 1 (v1)

- **Transport:** direct Ethernet connection to the K4 server (ports 9205 plaintext+SHA-384
  or 9204 TLS-PSK; `PING`/`PONG` keep-alive); transport abstraction designed so USB/serial is
  addable later. (`RRC`/`RRP` are server-side config set by the radio owner, not the client.)
- **CAT engine:** command codec for SET/GET/RESP, Auto-Info (`AI`) handling, error handling.
- **Control:** VFO A/B frequency, band, mode, bandwidth/filter, RIT/XIT, split, AGC,
  AF/RF gain, attenuator, preamp, NB/NR, antenna, basic menu-independent operating controls.
- **Metering:** S-meter (`SM`/`SMH`), TX power/SWR/ALC where available.
- **Transmit:** PTT, CW keying via remote paddle/key stream (`KZ`, `KZL`, `KZF`), voice TX.
- **Audio:** full-duplex RX (two channels) + TX (one channel) streaming, Opus/PCM (`EM`).
- **GUI:** iced-based shell with control panels, meters, tuning, and connection management;
  a reserved, non-functional placeholder area for the future spectrum/waterfall.
- **Config & security:** connection profiles, password handling, persistence.
- **Diagnostics:** structured logging of the CAT/session layer.

### 5.2 In scope — Phase 2 (post-v1)

- **Spectrum + waterfall** rendering from the K4 **dB/bin** streaming data (`PAN-*`).
- Panadapter interaction (click-to-tune, span/ref/scale already controllable in v1 via `#`
  commands even before the visual is present).
- **USB/serial (CAT-only)** transport implementation behind the existing abstraction.
- IQ streaming consumers (decoders/skimmer-style), if pursued.

### 5.3 Out of scope

- **Remote power-ON of the K4.** The K4 cannot be powered on via any command; it requires a
  hardware ground pulse on ACC pin 8 (Y-BOX/K-ON/web power switch — see *Remote K4 On-Off
  Control Methods*). The app may power **off** (`PS0`) and assumes external hardware for
  power-on. (See `ASM-04`.)
- Acting as a **K4 server** for other clients (we build a client only).
- Firmware update orchestration beyond passing through documented `PS` semantics.
- Logging/contest/digital-mode decode software (may integrate later, not a v1 goal).
- Multi-radio / non-K4 rig support.

## 6. Phasing

| Phase | Theme | Headline deliverable |
|---|---|---|
| **P0** | RE & concept (this baseline) | Approved requirements + architecture concept |
| **P1a** | Transport + CAT skeleton | Connect, keep-alive, GET/RESP round-trip, logging |
| **P1b** | Control + metering UI | Tune/mode/RX controls, live S-meter |
| **P1c** | Audio + transmit | Full-duplex audio, PTT, CW + voice keying, fail-safe |
| **P2** | Spectrum/waterfall + USB | dB/bin visual, serial transport |

## 7. Assumptions

| ID | Assumption | Impact if false |
|---|---|---|
| `ASM-01` | Target K4 runs firmware supporting K4-to-K4 remote (R36+) and the documented `RRT`/server commands. | Ethernet transport unavailable; fall back to USB-first plan. |
| `ASM-02` | The **streaming-data protocol** is known well enough to implement: documented community-side via QK4 (see [../references/external-references.md](../references/external-references.md), `R-EXT-01`) and confirmable against a real radio. Elecraft's official spec may still be requested to corroborate. | If the reverse-engineered facts are wrong/version-specific, audio/spectrum need rework; mitigated by `ASM-05` verification. |
| `ASM-03` | The documented CAT command set (Programmer's Reference rev. D12) accurately reflects the firmware on the target radio. | Codec mismatches; integration tests against real radio needed. |
| `ASM-04` | External hardware (e.g. N6TV Y-BOX/K-ON + web power switch) handles remote power-ON. | Operator cannot cold-start the radio remotely; documented as user responsibility. |
| `ASM-05` | A real K4 (or K4 server) is available for integration/acceptance testing. | Hardware-in-the-loop tests deferred; rely on protocol simulator only. |
| `ASM-06` | One operator at a time from this client (single-operator model); multi-client awareness is read-only. | Concurrency model simplified. |

## 8. Constraints

| ID | Constraint | Source |
|---|---|---|
| `CON-01` | Implementation language is **Rust**. | Stakeholder directive |
| `CON-02` | GUI framework is **iced**. | Stakeholder decision (ADR-03) |
| `CON-03` | Ethernet remote is the **first** transport; design must abstract transport for later USB/serial. | Stakeholder decision (ADR-02) |
| `CON-04` | CAT wire protocol, command semantics, and the `RRT` connection handshake are **defined by Elecraft** and cannot be altered. | Vendor (Programmer's Reference) |
| `CON-05` | The K4 server drops clients silent >10 s; client must `PING;` ~1/s. | Vendor (Remote Access Commands) |
| `CON-06` | Audio encode modes limited to `EM0–EM3` (raw 16/32-bit, Opus 16/32-bit); Opus is the practical WAN default. | Vendor (`EM` command) |
| `CON-07` | Transmit operation must comply with the operator's amateur licence (identification, band/mode limits). | Regulatory (SH-4) |
| `CON-08` | Spectrum/waterfall + audio **wire format is undocumented in vendor references**; the implementation target is the community-verified protocol in `R-EXT-01`, to be confirmed against a real radio. | Vendor docs gap / `R-EXT-01` |
| `CON-09` | QK4 (`R-EXT-01`) is **GPLv3**. Use it only as a source of **protocol facts and architectural ideas** via clean-room reimplementation in Rust; do **not** copy/translate its source. | Licensing |

## 9. Risks

| ID | Risk | L×I | Mitigation | Owner |
|---|---|---|---|---|
| `RISK-01` | ~~Streaming protocol unavailable~~ **Mitigated.** Protocol now documented community-side (`R-EXT-01`): framing, ports, auth, PAN/audio layouts, dBm offset −146, 12 kHz Opus. Residual risk: facts are reverse-engineered, may be firmware-version-specific or incomplete. | ~~H×H~~ → **L×M** | Implement to `R-EXT-01`; confirm each fact against a real radio (`ASM-05`); keep the `StreamCodec` seam so a corrected layout is a localized change; optionally request Elecraft's official spec to corroborate. | SH-2 |
| `RISK-02` | Real-time audio latency/jitter over WAN unacceptable for QSO. | M×H | Opus + jitter buffer; measure early (P1c spike); expose latency target `NFR-PERF`. | SH-2 |
| `RISK-03` | Accidental/unsafe transmit (stuck PTT, runaway CW, link loss mid-TX). | M×H | Mandatory fail-safe: link-loss → unkey; CW fail-safe timeout (`KZF`); explicit TX arming in UI. | SH-1/2 |
| `RISK-04` | iced custom canvas insufficient for high-FPS waterfall. | M×M | P2 rendering spike; isolate render behind `ARC` seam; consider GPU canvas/`wgpu` path. | SH-2 |
| `RISK-05` | CAT doc vs. firmware drift; undocumented edge cases. | M×M | Integration tests vs. real radio (`ASM-05`); tolerant parser that logs unknown frames. | SH-2 |
| `RISK-06` | Security of remote link (password in `RRT`, exposure over Internet). | M×H | Treat link as untrusted; recommend VPN/tunnel; never log secrets; `NFR-SEC`. | SH-5 |
| `RISK-07` | Single-developer bandwidth vs. broad v1 scope (audio+TX+control). | M×M | Strict phasing P1a→c; must/should prioritisation; defer spectrum. | SH-2 |

## 10. Change history

| Date | Ver | Author | Change |
|---|---|---|---|
| 2026-06-25 | 0.1 | DC0SK | Initial draft baseline. |
| 2026-06-25 | 0.2 | DC0SK | Integrated QK4 (R-EXT-01): downgraded RISK-01, updated ASM-02/CON-08, added CON-09 (GPL clean-room). |
