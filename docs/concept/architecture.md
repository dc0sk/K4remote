---
title: "Architecture & Concept"
status: Draft
version: "0.13"
updated: 2026-07-03
authors:
  - Simon Keimer (DC0SK)
owns: [ARC, ADR]
---

# Architecture & Concept

**Version:** 0.1 (Draft) · **Date:** 2026-06-25 · **Author:** DC0SK
Trace: owns `ARC-`, `ADR-`. Realizes [../requirements/system-requirements.md](../requirements/system-requirements.md);
verified through [../test/test-strategy.md](../test/test-strategy.md).

---

## 1. Concept overview

K4 Remote is a **client** application emulating the role of an Elecraft K4/0 remote panel in
software. It connects to a K4 acting as a server, mirrors the radio's state, lets the
operator control it, carries full-duplex audio, supports transmit (voice + CW), and is
prepared to render a spectrum/waterfall in Phase 2.

The design principle is a **hardware-independent, UI-independent protocol core** (testable
without a radio and without a GUI) surrounded by thin adapters: transport on one side, audio
devices and the iced GUI on the other. This directly serves `NFR-MAINT-01` and `NFR-TEST-02`
and keeps strict traceability tractable.

```
            ┌──────────────────────────── GUI (iced) — ARC-08 ─────────────────────────────┐
            │  Connection │ Tuning/VFO │ Mode/RX │ Meters │ TX/Keying │ [Spectrum placeholder]│
            └───────▲───────────────────────────────┬───────────────────────────────────────┘
        messages /  │ subscriptions (iced)          │ commands (intents)
                    │                                ▼
            ┌───────┴────────────────── Application / State Core ──────────────────────────┐
            │  ARC-05 RadioState (single source of truth)   ARC-06 Command/Intent handler   │
            │  ARC-07 Session manager (keep-alive, reconnect, fail-safe)                     │
            └───────▲───────────────▲──────────────────────────────────▲────────────────────┘
                    │ decoded RESP  │ encode SET             RX frames   │  TX frames
            ┌───────┴───────┐  ┌────┴───────────┐            ┌───────────┴──────────────┐
            │ ARC-03 CAT    │  │ ARC-04 CAT     │            │ ARC-09 Audio engine       │
            │ decoder       │  │ encoder        │            │ (Opus/PCM, jitter buffer, │
            └───────▲───────┘  └────┬───────────┘            │  device I/O) [RISK-01]    │
                    │ bytes         │ bytes                  └───────────▲──────────────┘
            ┌───────┴───────────────┴────────── ARC-02 Transport trait ─┴──────────────────┐
            │  ARC-02a Tcp (9205 SHA-384 / 9204 TLS-PSK) +ARC-02f FrameCodec   ARC-02b Serial[P2]│
            └───────────────────────────────────────────────────────────────────────────────┘
                                              ▲
                                       TCP/IP │ (and audio/stream sockets)
                                    ┌─────────┴──────────┐
                                    │   Elecraft K4      │  ARC-01 (external)
                                    │  (server radio)    │
                                    └────────────────────┘
```

## 2. Component model (`ARC`)

| ID | Component | Responsibility | Realizes (FR/NFR) |
|---|---|---|---|
| `ARC-01` | **K4 server (external)** | The radio; ground truth. Out of our build scope. | context |
| `ARC-02` | **Transport trait** | Abstract byte channel + connect/disconnect lifecycle; hides TCP vs serial. | FR-CONN-ABSTRACT, NFR-MAINT-01 |
| `ARC-02a` | **TcpRemoteTransport** | Single multiplexed TCP socket (port 9205 plaintext+SHA-384, or 9204 TLS-PSK); auth + post-auth init (`RDY/K41/ER1/EM/SL`); `RRN` on close. | FR-CONN-01..05, FR-AUTH-* |
| `ARC-02f` | **FrameCodec** | K4 binary envelope: `START`+BE-len+payload+`END`; reassembly across reads; type dispatch (CAT/Audio/PAN/MiniPAN). | FR-STREAM-01..03 |
| `ARC-02b` | **SerialTransport** | USB/RS232 **CAT-only** (raw `;`-delimited, no framing/audio); generic over Read+Write, `serial` feature opens real ports. | FR-CONN-ABSTRACT, FR-CAT-02 |
| `ARC-03` | **CAT decoder** | Byte stream → typed `Command`/`Response`; framing on `;`; resync on garbage. | FR-CAT-01..05, NFR-REL-01 |
| `ARC-04` | **CAT encoder** | Typed intent → byte-exact SET/GET command. | FR-CAT-01, FR-VFO/MODE/RX/TX-* |
| `ARC-05` | **RadioState model** | Single source of truth; updated by RESP + Auto-Info; observable by UI. | FR-CAT-06/07, FR-CAT-AI |
| `ARC-06` | **Command/Intent handler** | Maps UI intents → encoder; applies safety gating (TX arm). | FR-TX-SAFE-*, control FRs |
| `ARC-07` | **Session manager** | Keep-alive (`PING`), link-loss detection, reconnect/backoff, connect-time state seed, fail-safe. | FR-SES-*, FR-TX-SAFE-01, NFR-REL-* |
| `ARC-08` | **GUI (iced)** | Views, subscriptions, two-way binding to RadioState; placeholder pane for spectrum. | FR-UI-*, NFR-USE-01 |
| `ARC-09` | **Audio engine** | Stream socket I/O, Opus/PCM codec, jitter buffer, device capture/playback. | FR-AUD-* |
| `ARC-10` | **StreamCodec (seam)** | Decodes payload bodies above `ARC-02f`: audio packet (Opus/PCM, 12 kHz, L=Main/R=Sub) now; PAN/MiniPAN bins (`dBm=byte−146`) in P2. Localizes any protocol correction. | FR-AUD-04/05, FR-PAN-01 |
| `ARC-11` | **Panadapter renderer (P2)** | dB array → spectrum trace + waterfall (iced canvas / wgpu). | FR-PAN-02..04 |
| `ARC-12` | **Config store** | Profiles, prefs, secure secret storage. | FR-CFG-* |
| `ARC-13` | **Diagnostics/logging** | Structured logs, redaction, optional raw CAT console. | FR-DIAG-*, NFR-MAINT-LOG |
| `ARC-14` | **K4 protocol simulator (test)** | Mock server speaking the CAT/handshake protocol for hardware-free tests. | NFR-TEST-02 |
| `ARC-15` | **UI view-model helpers** (`app/src/ui.rs`) | Pure, iced-free presentation logic: `ViewMode` (single-A/B/dual) cycling, dot-grouped frequency formatting, semantic-colour role selection, two-line button state derivation, shade palette + S-meter scale, and the connect-control phase mapping (`ConnPhase` → label/action). Keeps the iced view (`ARC-08`) a thin projection and makes the testable `FR-UI-*` items unit-testable. | FR-UI-08..19, NFR-USE-01 |

## 3. Proposed crate / module layout

A Cargo workspace keeps the protocol core free of GUI/audio dependencies (`NFR-MAINT-01`):

```
k4remote/
├─ crates/
│  ├─ k4-protocol/     # ARC-03/04/05/06: Command/Response types, codec, RadioState. NO iced, NO audio.
│  ├─ k4-transport/    # ARC-02/02a(/02b): Transport trait + TCP (later serial).
│  ├─ k4-session/      # ARC-07: keep-alive, reconnect, fail-safe orchestration.
│  ├─ k4-audio/        # ARC-09/10(audio): codec + jitter buffer + device I/O.
│  ├─ k4-stream/       # ARC-10: StreamCodec seam (audio framing; P2 dB/bin).
│  ├─ k4-sim/          # ARC-14: protocol simulator for tests (dev-dependency).
│  ├─ k4-config/       # ARC-12: profiles/prefs (secret-free), redaction, SecretStore (OS keychain).
│  └─ k4-diag/         # ARC-13: structured levelled bounded diagnostic log.
├─ app/                # ARC-08/11/12/13: iced GUI binary, wires everything together.
└─ xtask/              # traceability gate + build helpers (NFR-TEST-01).
```

## 4. Key data flows

1. **Control (GET/SET).** UI intent → `ARC-06` gates (TX arm) → `ARC-04` encodes → `ARC-02`
   sends. Replies arrive as bytes → `ARC-03` decodes → `ARC-05` updates → UI subscription
   re-renders. (`FR-CAT-01`, `FR-UI-02`)
2. **Auto-Info push.** On connect, enable `AI`; unsolicited RESP frames flow decoder →
   state → UI with no polling. (`FR-CAT-AI`, `FR-MTR-01`)
3. **Keep-alive / fail-safe.** `ARC-07` emits `PING;` ~1 Hz; missing `PONG`/socket error →
   link-loss event → if transmitting, `ARC-06` forces RX + disarm. (`FR-SES-01/02`,
   `FR-TX-SAFE-01`, `NFR-REL-FAILSAFE`)
4. **RX audio.** Stream socket → `ARC-10` framing → `ARC-09` Opus decode → jitter buffer →
   output device. (`FR-AUD-RX-01`, `FR-AUD-02`)
5. **TX voice.** Input device → encode → only if TX armed+active → stream socket. (`FR-AUD-TX-01`)
6. **CW.** Paddle/key events → `ARC-06` → `KZ` element stream via `ARC-04`. (`FR-TX-CW-01`)
7. **Spectrum (P2).** Stream socket → `ARC-10` dB/bin decode → `ARC-11` render. (`FR-PAN-01..03`)

## 5. Architecture Decision Records

| ID | Decision | Rationale | Trace |
|---|---|---|---|
| `ADR-01` | **Rust, Cargo workspace, layered crates.** | Stakeholder constraint `CON-01`; layering enables `NFR-MAINT-01`/`NFR-TEST-02`. | CON-01 |
| `ADR-02` | **Transport behind a trait; Ethernet remote first, serial later.** | `CON-03`/`STK-18`; lets v1 ship without serial while keeping the seam. | FR-CONN-ABSTRACT |
| `ADR-03` | **iced for the GUI.** | Stakeholder decision `CON-02`; Elm-style messages give a deterministic, testable UI flow aligned with strict tracing. Spectrum canvas risk tracked as `RISK-04`. | CON-02 |
| `ADR-04` | **Single authoritative `RadioState`, updated by RESP + Auto-Info; UI is a pure projection.** | Avoids UI/radio drift; one place to test state transitions. | FR-CAT-06 |
| `ADR-05` | **Prefer Auto-Info (`AI5`/`AI4`) over polling**, with a GET seed burst on connect. | Lower latency, less traffic; matches K4 multi-client model. | FR-CAT-AI, FR-CAT-07 |
| `ADR-06` | **Async runtime (e.g. tokio) for all I/O; UI never blocks on I/O.** | `FR-UI-07`, `NFR-PERF-*`; bridges to iced via subscriptions/channels. | FR-UI-07 |
| `ADR-07` | **Isolate streaming framing in a `StreamCodec` seam (`ARC-10`).** | Quarantines `RISK-01` (undocumented protocol): audio now, dB/bin later, same seam; mock fixtures for tests. | RISK-01, FR-AUD-04, FR-PAN-01 |
| `ADR-08` | **Mandatory software TX safety layer** (explicit arm, e-stop, link-loss unkey) in addition to radio-side `KZF`. | Defence in depth for `STK-08`/`STK-13`; not solely reliant on the radio. | FR-TX-SAFE-* |
| `ADR-09` | **Protocol simulator (`k4-sim`) as primary test double.** | Hardware-free TDD (`NFR-TEST-02`); real-radio integration is a separate, smaller test tier. | NFR-TEST-02 |
| `ADR-10` | **Opus (`EM3`) default audio encode, PCM selectable on LAN.** | `CON-06`; WAN bandwidth/quality balance with LAN low-latency option. | FR-AUD-ENC |
| `ADR-11` | **Tolerant CAT parser:** unknown frames logged + skipped, never fatal. | `NFR-REL-01`, `FR-CAT-04`; resilience to firmware/doc drift (`RISK-05`). | FR-CAT-04 |
| `ADR-12` | **Implement to the community-verified K4/0 protocol (`R-EXT-01`) as a clean-room reimplementation**, confirmed against a real radio; treat facts as a `FrameCodec`/`StreamCodec`-local concern. | Resolves `RISK-01`; `CON-09` (GPL) requires fact-only reuse, no source copying; seam keeps corrections cheap. | FR-STREAM-*, FR-AUTH-*, FR-AUD-04, FR-PAN-01 |
| `ADR-13` | **Single multiplexed socket** carries CAT + audio + spectrum (per `R-EXT-01`), not separate per-stream ports. | Matches the K4/0 server; simplifies transport; framing demuxes by type. | FR-STREAM-02 |
| `ADR-14` | **v1 protocol bring-up uses synchronous `std::net` for `TcpRemoteTransport`**, behind the `Transport` trait. Async (tokio, ADR-06) migration is deferred until the session/audio layers need concurrency. | Minimises deps and makes the first L2 connect tests deterministic; the trait is the seam so the swap is localized and does not touch CAT/state. Supersedes ADR-06 *for the transport crate only* in v1. | FR-CONN-01, ADR-02, ADR-06 |
| `ADR-15` | **UI is reference-faithful in layout, semantics and — rev. 2026-07-02 — visual language.** Reproduce the K4's operating conventions (A/B symmetry, shared TX/RIT box, switchable single/dual view mirroring `PAN=A/B/A+B`, 7-primary + context-row model, semantic colours, two-line state buttons) as *interoperability faithfulness for the operator*, and style them after the references (`R-EXT-02`): dark layered theme, rounded button grids with a blue "engaged" fill, big white frequency readouts, proportional S-meter bars (`FR-UI-15`). All styling is **re-implemented from scratch with our own values** — no assets, iconography, branding, or code from any third-party app is copied (cf. `CON-09`). Decidable presentation logic (incl. the shade palette and S-meter scale) lives in a pure, iced-free `ARC-15` module so the testable `FR-UI-*` items are unit-tested while layout/styling is demonstrated. *Rev. supersedes the original "original visual identity" stance on explicit user direction (2026-07-02).* | Familiar to K4 / reference-app operators; clean provenance (conventions adopted, expression re-implemented); pure view-model keeps the iced layer thin and the traceability gate green. | FR-UI-08..19, NFR-USE-01, R-EXT-02 |

## 6. Requirement → component coverage (design check, rule R5)

| Requirement group | Primary `ARC` |
|---|---|
| FR-CONN-* | ARC-02, ARC-02a |
| FR-CAT-* | ARC-03, ARC-04, ARC-05 |
| FR-SES-* | ARC-07 |
| FR-VFO/MODE/RX-* | ARC-04, ARC-05, ARC-06, ARC-08 |
| FR-MTR-* | ARC-05, ARC-08 |
| FR-TX-* / safety | ARC-06, ARC-07, ARC-08 |
| FR-AUD-* | ARC-09, ARC-10 |
| FR-PAN-* (P2) | ARC-10, ARC-11 |
| FR-UI-* | ARC-08, ARC-15 |
| FR-CFG-* | ARC-12 |
| FR-DIAG-* | ARC-13 |
| NFR-TEST-* | ARC-14, xtask |

*Every v1 `M`/`S` requirement maps to at least one component (R5 satisfied at concept level;
per-test mapping is maintained in the traceability matrix.)*

## 7. Open architecture questions

- `AQ-1` Exact socket topology of the K4 server (single control socket + separate stream
  sockets vs. multiplexed) — pending streaming spec (`OP-1`).
- `AQ-2` iced spectrum rendering path: native `Canvas` vs. embedded `wgpu` shader for the
  waterfall (P2 spike, `RISK-04`).
- `AQ-3` CW input acquisition on the client host (serial paddle / keyboard / on-screen) — `OP-4`.
- `AQ-4` Threading bridge between tokio I/O tasks and iced runtime (channel vs. subscription).

## Change history

| Date | Ver | Author | Change |
|---|---|---|---|
| 2026-06-25 | 0.1 | DC0SK | Initial draft concept + ADR-01..11. |
| 2026-06-25 | 0.2 | DC0SK | Added ARC-02f FrameCodec, enriched ARC-02a/ARC-10; ADR-12 (clean-room to R-EXT-01), ADR-13 (single multiplexed socket). |
| 2026-06-25 | 0.3 | DC0SK | ADR-14 (sync std::net transport); implemented ARC-02a (live TCP), ARC-07 (session), ARC-08 (iced P1b UI skeleton + worker bridge). |
| 2026-06-25 | 0.4 | DC0SK | Stream demux wired: poll_frames at the link, Session.pump demultiplexes by PayloadType, worker routes audio→JitterBuffer and PAN→PanFrame. |
| 2026-06-25 | 0.5 | DC0SK | Added k4-config crate realizing ARC-12 (config store): TOML profiles/prefs, secret-free, redact. |
| 2026-06-25 | 0.6 | DC0SK | Added k4-diag crate realizing ARC-13 (diagnostics): levelled bounded DiagLog; raw CAT console via Inbound.cat. |
| 2026-06-25 | 0.7 | DC0SK | Implemented ARC-02b SerialTransport (CAT-only raw-line adapter as a CatLink) + LineDecoder. Second transport backend proves FR-CONN-ABSTRACT. |
| 2026-06-26 | 0.8 | DC0SK | Added ARC-15 (UI view-model helpers) + ADR-15 (K4-faithful layout, original visual identity, switchable ViewMode) from R-EXT-02 UI study. |
| 2026-06-25 | 0.8 | DC0SK | ARC-12 gains SecretStore (MemoryStore tested + KeyringStore feature) for OS-keychain password storage (FR-CFG-03). |
| 2026-07-02 | 0.9 | DC0SK | ADR-15 revised on user direction: visual language now reference-faithful (dark layered theme, blue-engaged button grids, proportional S-meter — FR-UI-15), still re-implemented from scratch, no copied assets. |
| 2026-07-02 | 0.10 | DC0SK | FR-UI-16: connect control is phase-driven (Connect/Cancel/Disconnect) via `ConnPhase` in ARC-15; the worker bridge runs the blocking connect handshake on a short-lived thread and polls its result, so an attempt never freezes the UI/worker and is cancellable. |
| 2026-07-02 | 0.11 | DC0SK | FR-UI-17 theme selector (dark/light/contrast/system) via `ThemeMode` + per-theme shade/role palettes in ARC-15; the iced view resolves colours against the active theme. FR-UI-18 About box (`about_lines`). Dual-pane spectrum height matched to single view. |
| 2026-07-02 | 0.12 | DC0SK | FR-UI-19: primary softkey opens a K4 config screen (`menu_screen_synopsis`) in the spectrum frame only; controls box + lower panels untouched. Per-screen content pending definition. |
| 2026-07-03 | 0.13 | DC0SK | ARC-02a TLS-PSK (`connect_tls`) realized + live-verified; ARC-05 `RadioState` extended for config-screen read-back and surfaced via the snapshot to seed the screens on connect (FR-UI-20). Keychain writes moved off the UI thread. |
