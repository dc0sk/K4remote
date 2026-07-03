---
title: "Test Strategy & Traceability"
status: Draft
version: "0.44"
updated: 2026-07-03
authors:
  - Simon Keimer (DC0SK)
owns: [TC]
---

# Test Strategy & Traceability

**Version:** 0.1 (Draft) · **Date:** 2026-06-25 · **Author:** DC0SK
Trace: owns `TC-`. Closes the V-model loop: `FR`/`NFR` → `TC` → result.
Governs rules R3/R4 from [../README.md](../README.md) §4.

---

## 1. Test philosophy

Development is **test-driven**: for each `Approved` requirement, the test case is written
first (red), the minimal code makes it pass (green), then refactor. Tests **name the
requirement ID** they verify so traceability is mechanical, not manual.

Naming convention (Rust): the test function name and a `// trace:` annotation carry the ID.

```rust
/// trace: FR-VFO-01
#[test]
fn fr_vfo_01_set_frequency_emits_canonical_11_digit_hz() { ... }
```

## 2. Test levels

| Level | Scope | Tooling | Hardware? | Primary requirements |
|---|---|---|---|---|
| **L1 Unit** | One module: codec, state transitions, meter scaling, jitter buffer logic, safety gating. | `cargo test` | No | FR-CAT, FR-VFO/MODE/RX, FR-MTR-04, FR-TX-SAFE, FR-AUD-02 |
| **L2 Component/Integration** | Engine + transport mock + `k4-sim`: connect→seed→AI→control round-trips, keep-alive, reconnect, link-loss fail-safe. | `cargo test` + `k4-sim` | No (simulator) | FR-CONN, FR-SES, FR-CAT-06/07, FR-TX-SAFE-01 |
| **L3 Contract / fixtures** | Frame + stream decoders vs. byte fixtures built from the `R-EXT-01` layouts (and later real captures). | fixture files | No (fixtures) | FR-STREAM-*, FR-AUTH-01, FR-AUD-04, FR-PAN-01 |
| **L4 System (HIL)** | Whole app against a **real K4 / K4 server**. | manual + scripted | **Yes** (ASM-05) | FR-AUD-RX/TX, FR-PAN-02..04, end-to-end demos |
| **L5 Non-functional** | Latency/jitter benchmarks, fuzz/robustness, security/redaction, build matrix. | `criterion`, fuzz, CI | Mixed | NFR-PERF-*, NFR-REL-*, NFR-SEC, NFR-PORT |

Verification method per requirement (`T/D/I/A`) is recorded in the SRS; `D`/`A`/`I` items are
evidenced by L4 demos, benchmark analysis, or documented inspection rather than a unit test.

## 3. Test infrastructure

- **`k4-sim` protocol simulator (`ARC-14`)** — the workhorse. Accepts the `RRT` handshake,
  responds to GETs with SET-format RESP, can be scripted to push Auto-Info frames, inject
  `<cmd>?;` errors, drop the link, and (later) emit stream fixtures. Enables L1–L3 with **no
  radio** (`NFR-TEST-02`).
- **Transport mock** — in-memory `Transport` impl feeding canned byte streams (fragmented,
  concatenated, garbage) for decoder robustness (`FR-CAT-02/04`, `NFR-REL-01`).
- **Stream fixtures** — captured/sample audio (and P2 dB/bin) byte blobs once the streaming
  spec is obtained (`OP-1`).
- **Fault injection** — link-loss-during-TX harness for `FR-TX-SAFE-01` / `NFR-REL-FAILSAFE`.

## 4. Coverage gate — `xtask trace` (rules R1–R5)

A build task parses the requirement tables (`FR-`/`NFR-` IDs) and scans test sources for
`trace:` annotations, then asserts:

- **R3** every `M`/`S` requirement has ≥1 referencing test → else **fail**.
- **R4** every `trace:` ID refers to a real requirement → else **fail** (no dangling traces).
- Emits a coverage report (`docs/test/coverage.generated.md`) listing requirement → tests →
  last result. CI fails on any R1–R5 violation. This makes traceability a **build invariant**.

## 5. Traceability matrix (seed)

Maintained partly by hand (design intent) and verified/augmented by `xtask trace`.
`Status`: P=Proposed, A=Approved, Impl=Implemented, V=Verified. Tests are the planned `TC` IDs.

| Requirement | Up (STK) | Component (ARC) | Test case(s) (TC) | Lvl | Status |
|---|---|---|---|---|---|
| FR-CONN-01 | STK-01 | ARC-02a | TC-CONN-01 (handshake bytes), TC-CONN-02 (accept→Connected) | L2 | V\* |
| FR-CONN-02 | STK-01 | ARC-02a | TC-CONN-03 (RRN on disconnect) | L2 | V\* |
| FR-CONN-03 | STK-01 | ARC-02/07 | TC-CONN-04 (state events) | L2 | P |
| FR-CONN-04 | STK-01 | ARC-02a | TC-CONN-05 (failure variants) | L2 | P |
| FR-CONN-ABSTRACT | STK-18 | ARC-02 | TC-CONN-06 (mock transport drives a trait consumer) | L2 | V\* |
| FR-STREAM-01 | STK-01/05 | ARC-02f | TC-STR-01 (build, split-read, multi-frame; +L2 sim round-trip) | L1/L2 | V\* |
| FR-STREAM-02 | STK-01 | ARC-02f | TC-STR-02 (type dispatch + unknown) | L1 | V\* |
| FR-STREAM-03 | STK-17 | ARC-02f | TC-STR-03 (garbage + corrupted-END resync) | L1 | V\* |
| FR-AUTH-01 | STK-01/14 | ARC-02a | TC-AUTH-01 (SHA-384 hex known-answer) | L1 | V\* |
| FR-AUTH-02 | STK-14 | ARC-02a | TC-AUTH-03 (TLS-PSK loopback, right/wrong key) — opt-in `tls` | L2 | V\* |
| FR-AUTH-03 | STK-01 | ARC-02a/07 | TC-AUTH-02 (init sequence order) | L2 | V\* |
| FR-SES-PING | STK-01 | ARC-07 | TC-SES-04 (PING<epoch> + latency) | L2 | V\* |
| FR-CAT-01 | STK-02/03 | ARC-03/04 | TC-CAT-01 (round-trip per cmd) | L1 | P |
| FR-CAT-02 | STK-02 | ARC-03 | TC-CAT-02 (fragmented/batched) | L1 | V\* |
| FR-CAT-03 | STK-02 | ARC-03 | TC-CAT-03 (`?;` error mapping) | L1 | P |
| FR-CAT-04 | STK-17 | ARC-03 | TC-CAT-04 (unknown frame resync) | L1 | P |
| FR-CAT-05 | STK-02/03 | ARC-03/04 | TC-CAT-05 (`$` sub-RX target) | L1 | V\* |
| FR-CAT-AI | STK-04 | ARC-05 | TC-CAT-06 (AI push updates state) | L2 | V\* |
| FR-CAT-06 | STK-02/04 | ARC-05 | TC-CAT-07 (state coherence) | L1 | V\* |
| FR-CAT-07 | STK-01/02 | ARC-05/07 | TC-CAT-08 (connect seed burst) | L2 | V\* |
| FR-SES-01 | STK-01 | ARC-07 | TC-SES-01 (PING ~1Hz) | L2 | V\* |
| FR-SES-02 | STK-01/20 | ARC-07 | TC-SES-02 (link-loss detect) | L2 | V\* |
| FR-SES-RECONNECT | STK-20 | ARC-07 | TC-SES-03 (backoff + restore) | L2 | V\* |
| FR-VFO-01 | STK-02 | ARC-04 | TC-VFO-01 (FA canonical 11-digit Hz) | L1 | V\* |
| FR-VFO-02 | STK-02 | ARC-03/05 | TC-VFO-02 (parse FB) | L1 | V\* |
| FR-VFO-03 | STK-02 | ARC-06 | TC-VFO-03 (step + direct entry) | L1 | P |
| FR-VFO-04 | STK-02 | ARC-04/05 | TC-VFO-04 (band switch) | L1 | V\* |
| FR-MODE-01 | STK-03 | ARC-04/05 | TC-MODE-01 (MD/MD$) | L1 | V\* |
| FR-MODE-02 | STK-03 | ARC-04/05 | TC-MODE-02 (BW) | L1 | V\* |
| FR-RX-01 | STK-03 | ARC-04/05 | TC-RX-01 (AG/RG range) | L1 | V\* |
| FR-RX-02 | STK-03 | ARC-04/05 | TC-RX-02 (attenuator) | L1 | V\* |
| FR-RX-03 | STK-03 | ARC-04/05 | TC-RX-03 (AGC GT encode/parse) | L1 | V\* |
| FR-RX-04 | STK-03 | ARC-04/05 | TC-RX-04 (NB/NR encode/parse) | L1 | V\* |
| FR-VFO-05 | STK-03 | ARC-04/05 | TC-VFO-05 (RIT/XIT/clear) | L1 | V\* |
| FR-VFO-07 | STK-02 | ARC-04 | TC-VFO-07 (`AB` copy/swap encode) | L1 | V\* |
| FR-EQ-01 | STK-03/06 | ARC-04 | TC-EQ-01 (`RE`/`TE`/`REF` 8-band encode, ±16 clamp) | L1 | V\* |
| FR-KEY-01 | STK-07 | ARC-04 | TC-KEY-01 (`KP`/`KS` keyer config encode) | L1 | V\* |
| FR-AUD-CFG-01 | STK-06 | ARC-04 | TC-AUD-CFG-01 (`MI/MG/MS/LI/LO` encode) | L1 | V\* |
| FR-ANT-01 | STK-02 | ARC-04 | TC-ANT-01 (`AN`/`AR` encode) | L1 | V\* |
| FR-MENU-01 | STK-11 | ARC-04 | TC-MENU-01 (`MO/MEDF/ME` encode) + TC-MENU-02 (89-item `menu_items` table + `menu_search` filter) | L1 | V\* |
| FR-SW-01 | STK-02/11 | ARC-04 | TC-SW-01 (`SW` switch-emulation encode; M1–M4/PF key codes) | L1 | V\* |
| FR-VOX-01 | STK-06 | ARC-04 | TC-VOX-01 (`VX` VOX on/off encode) | L1 | V\* |
| FR-TX-MSG-01 | STK-07 | ARC-04 | TC-TXMSG-01 (`KY` text send encode, 60-char truncate) | L1 | V\* |
| FR-PAN-CTL-01 | STK-10 | ARC-04 | TC-PAN-01 (`#`-display family: `#DPM/#SPN/#REF/#SCL/#AVG/#PKM/#WFC/#WFH/#NB/#NBL` encode) | L1 | V\* |
| FR-MTR-01 | STK-04 | ARC-05/08 | TC-MTR-01 (SM auto-update) | L2 | V\* |
| FR-MTR-02 | STK-04 | ARC-05 | TC-MTR-03 (SMH dBm parse) | L1 | V\* |
| FR-MTR-04 | STK-04/11 | ARC-05 | TC-MTR-02 (bar→S-unit mapping) | L1 | V\* |
| FR-TX-01 | STK-06/13 | ARC-06 | TC-TX-01 (explicit TX only) | L1 | P |
| FR-TX-CW-01 | STK-07 | ARC-06/04 | TC-TX-02 (KZ element stream) | L1 | V\* |
| FR-TX-CW-02 | STK-07 | ARC-06 | TC-TX-06 (KZL delay encode) | L1 | V\* |
| FR-TX-SAFE-02 | STK-08 | ARC-07 | TC-TX-07 (KZF fail-safe encode) | L1 | V\* |
| FR-TX-SAFE-01 | STK-08/13 | ARC-06/07 | TC-TX-03 (link-loss unkey) | L2 | V\* |
| FR-TX-SAFE-03 | STK-08/13 | ARC-06 | TC-TX-04 (disarmed = inert) | L1 | V\* |
| FR-TX-SAFE-04 | STK-08 | ARC-06 | TC-TX-05 (e-stop emits RX) | L1 | V\* |
| FR-AUD-RX-01 | STK-05 | ARC-09/10 | TC-AUD-01 (decode→playback) | L1/L4 | Impl |
| FR-AUD-TX-01 | STK-06 | ARC-09 | TC-AUD-02 (mic only when armed) | L1 | V\* |
| FR-AUD-ENC | STK-05/14 | ARC-09 | TC-AUD-03 (EM3 default/PCM LAN) | L1 | P |
| FR-AUD-02 | STK-05 | ARC-09 | TC-AUD-04 (jitter buffer reorder) | L1 | V\* |
| FR-AUD-04 | STK-05 | ARC-10 | TC-AUD-05 (audio packet decode, L=Main/R=Sub) | L3 | V\* |
| FR-AUD-05 | STK-05 | ARC-09 | TC-AUD-06 (seq order/dedup/late-drop) | L1 | V\* |
| FR-AUD-DEV-01 | STK-05/06/12 | ARC-09/08 | TC-AUD-DEV-01 (device enum + selection persists; routing L4) | L1/L4 | V\*(partial) |
| FR-AUD-LVL-01 | STK-05/06 | ARC-09 | TC-AUD-LVL-01 (volume/mic-gain persist + clamp; PCM scaling L4) | L1/L4 | V\*(partial) |
| FR-PAN-01 | STK-09 | ARC-10 | TC-PAN-02 (PAN decode: meta + bins, dBm=byte−146) | L3 | V\* |
| FR-PAN-02 | STK-09 | ARC-11 | TC-PAN-03 (dbm→y scaling) + canvas (L4) | L1 | Impl |
| FR-PAN-03 | STK-09 | ARC-11 | TC-PAN-04 (waterfall colormap) + canvas (L4) | L1 | Impl |
| FR-UI-04 | STK-08/11 | ARC-08 | TC-UI-01 (arm/e-stop affordances) | L4 | P |
| FR-UI-07 | STK-11 | ARC-08 | TC-UI-02 (no UI block under load) | L5 | P |
| FR-UI-08 | STK-11 | ARC-15 | TC-UI-03 (ViewMode cycle + pane visibility) | L1 | V\* |
| FR-UI-09 | STK-11 | ARC-15 | TC-UI-04 (dot-grouped freq formatting) | L1 | V\* |
| FR-UI-10 | STK-11/08 | ARC-15 | TC-UI-05 (semantic colour roles; TX override; distinct) | L1 | V\* |
| FR-UI-11 | STK-11 | ARC-15 | TC-UI-06 (two-line state buttons from state) | L1 | V\* |
| FR-UI-12 | STK-11/08 | ARC-15/08 | TC-UI-09 (band_layout panes/centre-box/reflow); A/B symmetric layout + shared TX/RIT box (demo, L4) | L1 | V\* |
| FR-UI-13 | STK-11 | ARC-15/08 | TC-UI-07 (context-row toggle exclusive) + TC-UI-08 (primaries order + mode-dependent items); visual reveal (L4) | L1 | V\* |
| FR-UI-14 | STK-09/11 | ARC-11 | mini-pan tuning aid (demo, P2) | L4 | P |
| FR-UI-15 | STK-11 | ARC-15/08 | TC-UI-10 (shade luminance ordering) + TC-UI-11 (S-meter face scale, clamped); themed render (demo, L4) | L1 | V\* |
| FR-UI-16 | STK-01/11 | ARC-15/08 | TC-UI-12 (connect_button phase → label/action); cancel aborts in-flight attempt (demo, L4) | L1 | V\* |
| FR-UI-17 | STK-11 | ARC-15/08 | TC-UI-13 (ThemeMode cycle/label/effective + per-theme palettes); live theme render (demo, L4) | L1 | V\* |
| FR-UI-18 | STK-11 | ARC-15/08 | TC-UI-14 (about_lines author/license/URL); About box render+dismiss (demo, L4) | L1 | V\* |
| FR-UI-19 | STK-11 | ARC-15/08 | TC-UI-15 (screen_kind: every primary maps to a screen); all 7 screens render in the spectrum slot (demo, L4) | L1 | V\* |
| FR-UI-20 | STK-11/04 | ARC-05/15 | TC-STATE-RB (`apply_cat` parses every config RESP → `RadioState`); screens seed on connect (live-verified against a real K4) | L1 | V\* |
| FR-UI-21 | STK-11 | ARC-08 | TC-UI-21 (`DEFAULT_WINDOW_SIZE` landscape: w>h) | L1 | V\* |
| FR-UI-22 | STK-01/11 | ARC-15/08 | TC-UI-22 (`conn_status` label+colour per phase) | L1 | V\* |
| FR-UI-23 | STK-11/12 | ARC-08/12 | TC-UI-23 (settings dialog hosts connection + peer cache + audio device/level controls) | L4 | V\* |
| FR-CFG-03 | STK-12/14 | ARC-12 | TC-CFG-01 (no plaintext secret) | L1 | V\* |
| FR-CFG-01 | STK-12 | ARC-12 | TC-CFG-02 (profile TOML save/load) | L1 | V\* |
| FR-CFG-04 | STK-12/14 | ARC-12 | TC-PEER-01 (peer upsert/find/remove; master seal+unlock; wrong-pw fails) | L1 | V\* |
| FR-CFG-05 | STK-12 | ARC-12 | TC-CFG-03 (last session + peer cache persist across save/load) | L1 | V\* |
| FR-CFG-02 | STK-12 | ARC-12 | TC-CFG-03 (prefs round-trip) | L1 | V\* |
| FR-DIAG-03 | STK-14/17 | ARC-13 | TC-DIAG-01 (secret redaction) | L1 | V\* |
| FR-DIAG-01 | STK-17 | ARC-13 | TC-DIAG-02 (DiagLog level/cap/format) | L1 | V\* |
| FR-DIAG-02 | STK-17 | ARC-13 | raw CAT console (send + Inbound.cat surfaced) | L1/L4 | Impl |
| NFR-PERF-01 | STK-01/02 | ARC-03..06 | TC-NFR-01 (control latency bench) | L5 | P |
| NFR-PERF-CW | STK-07 | ARC-06 | TC-NFR-02 (CW jitter bench) | L5 | P |
| NFR-REL-FAILSAFE | STK-08 | ARC-06/07 | TC-NFR-03 (≤1s safe state) | L5 | P |
| NFR-REL-01 | STK-17 | ARC-03 | TC-NFR-04 (fuzz no-panic) | L5 | P |
| NFR-SEC-01 | STK-14 | ARC-13 | TC-NFR-05 (no secret in logs) | L1 | V\* |
| NFR-SEC-03 | STK-14 | ARC-12 | TC-CRYPTO-01 (Argon2+ChaCha20-Poly1305; tamper/wrong-pw rejected; random salt/nonce) | L1 | V\* |
| NFR-TEST-01 | STK-15 | xtask | TC-NFR-06 (trace gate green) | L5 | P |
| NFR-PORT-01 | STK-16 | app | TC-NFR-07 (CI build matrix) | L5 | P |
| NFR-PORT-02 | STK-16 | app | TC-NFR-08 (CI matrix builds/tests Linux x86_64+arm64, macOS, Windows; x86_64 release verified locally) | L5 | V\*(partial) |
| NFR-PKG-01 | STK-16 | app | TC-NFR-09 (.deb built+verified locally; PKGBUILD .SRCINFO valid; release workflow ships .deb/.tar.gz/.zip) | L5 | V\*(partial) |

> `V*` = implemented; test passes locally via `cargo test`. Promote to `V` once a CI pipeline runs the gate. As of this baseline: **113 tests pass**; password stored in the OS keychain via the `SecretStore` abstraction (FR-CFG-03), never in the TOML config (k4-protocol 38, k4-stream 7, k4-audio 12, k4-sim 1, k4-transport 3, k4-session 12, **app/k4remote 21** UI view-model). `cargo xtask`: 71 traced, 0 dangling (R4 ✓). `cargo audit`: clean. TLS-PSK is a default app feature (real OpenSSL); the 2 TLS tests run in the default suite. The iced app builds clippy-clean; the **testable** `FR-UI-08..11/15/16/17/18/19` are unit-tested via the pure `ARC-15` view-model (`app/src/ui.rs`), while the visual `FR-UI-04/07/12/13/14` remain L4 (pending a display). Opus codec + cpal device I/O are the remaining L4 audio step.

*(Lower-priority `C`/`W2` requirements — FR-MODE-03/04, FR-RX-03..06, FR-VFO-05/06, FR-PAN-02..04,
FR-SES-MULTI, FR-DIAG-02, etc. — get `TC` IDs when promoted to `Approved`.)*

## 6. Test result recording

- Each CI run publishes a junit/`cargo test` report; `xtask trace` joins it to requirement
  IDs and regenerates `coverage.generated.md` with the **last result and date per `TC`**.
- A requirement's `Status` advances to `Verified` only when **all** its `TC` pass in CI.
- L4 (hardware) results are logged manually in a dated `docs/test/hil-runs/` note with the
  firmware revision of the radio under test (supports `RISK-05`/`ASM-03`).

## 7. Entry / exit criteria

- **Phase exit (P1a/b/c):** all `M` requirements in that phase `Verified`; trace gate green;
  no open `M`-severity defect.
- **v1 release:** all `M` requirements `Verified`; `S` requirements `Verified` or explicitly
  deferred with rationale; `RISK-01`/`RISK-02`/`RISK-03` mitigations demonstrated.

## Change history

| Date | Ver | Author | Change |
|---|---|---|---|
| 2026-06-25 | 0.1 | DC0SK | Initial draft strategy + seed matrix. |
| 2026-06-25 | 0.2 | DC0SK | Unblocked FR-AUD-04/FR-PAN-01 tests; added FR-STREAM/FR-AUTH/FR-SES-PING rows. |
| 2026-06-25 | 0.3 | DC0SK | Implemented FR-STREAM-01/02/03, FR-AUTH-01, FR-VFO-01, FR-CONN-ABSTRACT + first L2 sim round-trip; marked V*. |
| 2026-06-25 | 0.4 | DC0SK | Implemented TcpRemoteTransport + SimServer (live L2 connect/auth/init/PING) and RadioState/apply_cat; verified FR-CONN-01/02, FR-AUTH-03, FR-SES-PING, FR-CAT-05/06/07/AI. |
| 2026-06-25 | 0.5 | DC0SK | Session layer (keep-alive, link-loss fail-safe, TX arm/e-stop) + iced P1b UI skeleton. Verified FR-SES-01/02, FR-TX-SAFE-01/03/04. |
| 2026-06-25 | 0.6 | DC0SK | StreamCodec (audio 0x01 + PAN 0x02 decode) and jitter buffer. Verified FR-AUD-02/04/05, FR-PAN-01. |
| 2026-06-25 | 0.7 | DC0SK | CW keying: KZ element stream + KZL/KZF encoders, arm-gated session.send_cw. Verified FR-TX-CW-01/02, FR-TX-SAFE-02. |
| 2026-06-25 | 0.8 | DC0SK | Stream demux: CatLink.poll_frames + Session.pump returns Inbound (CAT→state, audio/spectrum→codecs); worker feeds jitter buffer + counts. |
| 2026-06-25 | 0.9 | DC0SK | Opus codec (k4-audio `opus` feature, default on): encode/decode round-trip, stereo RX (L=Main/R=Sub). Worker decodes jitter-buffered frames to PCM. FR-AUD-TX-01 verified; FR-AUD-RX-01 decode done (playback = L4). CI installs libopus. |
| 2026-06-25 | 0.10 | DC0SK | cpal device I/O (k4-audio `device` feature): SampleRing + LinearResampler (tested), AudioOutput/AudioInput (L4). Worker plays RX (Opus→resample→speaker) and captures TX (mic→resample→Opus→send, gated). Session.send_tx_audio; CatLink.send_frame primitive. |
| 2026-06-25 | 0.11 | DC0SK | S-meter (SM/SMH parse + S-unit map, seeded) and reconnect backoff (k4-session Backoff, worker auto-reconnect). Verified FR-MTR-01/02/04, FR-SES-RECONNECT. |
| 2026-06-25 | 0.12 | DC0SK | Control surface: CAT encoders + RESP parse for VFO B, mode, bandwidth, AF/RF gain, attenuator, band/split; UI band/split/atten buttons + readouts. Verified FR-VFO-02/04/06, FR-MODE-01/02, FR-RX-01/02. |
| 2026-06-25 | 0.13 | DC0SK | DSP control: AGC/NB/NR/preamp/RIT/XIT encoders + RESP parse + UI toggles. Verified FR-RX-03/04, FR-VFO-05. |
| 2026-06-25 | 0.14 | DC0SK | Phase-2 spectrum canvas: tested render helpers (dbm→y, waterfall colormap), worker trace+waterfall history, iced Canvas widget. FR-PAN-02/03 render math verified (canvas = L4). |
| 2026-06-25 | 0.15 | DC0SK | TLS-PSK transport (k4-transport `tls` feature, OpenSSL): connect_tls + PSK loopback test (right key connects + round-trips, wrong key fails). Verified FR-AUTH-02. Opt-in; default gate unchanged. |
| 2026-06-25 | 0.16 | DC0SK | App-level TLS toggle: WorkerCmd.use_tls + cfg-gated open_transport (TLS/plaintext), UI 'TLS' button auto-sets port 9204/9205. `tls` now a default app feature; CI installs libssl-dev. |
| 2026-06-25 | 0.17 | DC0SK | Config persistence: k4-config crate (TOML profiles+prefs, secret-free by construction, redact helper). App prefills last connection + saves on connect. Verified FR-CFG-01/02/03, NFR-SEC-01. |
| 2026-06-25 | 0.18 | DC0SK | Diagnostics: k4-diag crate (levelled bounded DiagLog), Session.Inbound.cat surfaces raw CAT, worker logs net/tx/rx, UI raw-CAT console + log view. Verified FR-DIAG-01/03; FR-DIAG-02 console (L4). |
| 2026-06-25 | 0.19 | DC0SK | USB/serial transport (ARC-02b): LineDecoder (raw `;`-CAT framing) + generic SerialTransport<Read+Write> as a CatLink (tested with mock port); `serial` feature opens real ports. Verified FR-CAT-02; FR-CONN-ABSTRACT has a 2nd real backend. |
| 2026-06-25 | 0.20 | DC0SK | Worker transport selection: AnyLink enum (Tcp/Serial) dispatched as CatLink; ConnectTarget; serial gets PING-less SessionConfig; UI 'Mode: Ethernet/Serial' toggle + serial port/baud fields. App always enables k4-transport/serial; CI installs libudev-dev. |
| 2026-06-25 | 0.21 | DC0SK | OS keychain storage: k4-config SecretStore trait + MemoryStore (tested) + KeyringStore (`keychain` feature, default-on in app). Profile.remember; app 'Remember' toggle stores/loads password from the keychain. FR-CFG-03 strengthened. |
| 2026-06-26 | 0.22 | DC0SK | UI design from R-EXT-02 (K4 native LCD): ARC-15 pure view-model (`app/src/ui.rs`) — ViewMode (single-A/B/dual, switchable like PAN=A/B/A+B), dot-grouped freq, semantic colour roles, two-line state buttons; wired into the iced view. Verified FR-UI-08/09/10/11 (TC-UI-03..06, 5 tests). 84 tests, 54 traced, 0 dangling. |
| 2026-06-26 | 0.23 | DC0SK | FR-UI-13 primary + context-row state machine (ARC-15): `Primary` (7 K4 primaries), `ContextRow` (exclusive toggle), mode-dependent `context_items` (APF/TEXT/KEYER only in CW); wired bottom primary row + context row into the view. Verified FR-UI-13 (TC-UI-07/08, 2 tests). 86 tests, 55 traced, 0 dangling. |
| 2026-06-26 | 0.24 | DC0SK | FR-UI-12 banded layout: `band_layout(width, mode)` + `Pane` (ARC-15) → responsive panes/centre-box/narrow-stack (TC-UI-09). View restructured into K4 bands: A/B-symmetric VFO header with shared TX/SPLIT/RIT centre box, per-pane spectrum band, primaries at the bottom; window-resize subscription; surfaced mode_b + sub S-meter to UiSnapshot. Removed superseded ViewMode::shows_a/b. 87 tests, 56 traced, 0 dangling. |
| 2026-07-02 | 0.25 | DC0SK | FR-UI-15 visual-identity pass (revised ADR-15): dark layered theme via `ui::Shade` (TC-UI-10) + proportional S-meter `s_meter_fraction` (TC-UI-11); iced view restyled after the references (blue-engaged button grids, badges, panels, A/B/A+B segmented selector, styled TX/PTT/E-STOP). 89 tests, 57 traced, 0 dangling. |
| 2026-07-02 | 0.26 | DC0SK | FR-UI-16: phase-driven connect control. `ConnPhase` + `connect_button` in ARC-15 (TC-UI-12); worker runs the blocking connect handshake on a short-lived thread and polls its result, so an attempt is cancellable (Connect→Cancel→Connect) and never freezes the UI/worker. 90 tests, 58 traced, 0 dangling. |
| 2026-07-02 | 0.27 | DC0SK | FR-UI-17 theme selector (`ThemeMode` dark/light/contrast/system + per-theme shade/role palettes, TC-UI-13) and FR-UI-18 About box (`about_lines`, TC-UI-14) in ARC-15; header gains Theme + About buttons; dual-pane spectrum height matched to single view. Removed now-unused `ViewMode::pane_count`. 92 tests, 60 traced, 0 dangling. |
| 2026-07-02 | 0.28 | DC0SK | FR-UI-19 (corrected): primary softkey shows a K4 config screen (`menu_screen_synopsis`, TC-UI-15) **in place of the spectrum frame only** — controls box + lower panels untouched. Reverted the wrong first attempt (which replaced controls + duplicated existing UI). Per-screen content pending user definition. 93 tests, 61 traced, 0 dangling. |
| 2026-07-02 | 0.29 | DC0SK | Phase-0 CAT commands added to `k4-protocol` (`docs/concept/k4-screens.md` §3.2) with byte-exact tests: RE/TE EQ, KP/KS keyer, MI/MG/MS/LI/LO audio, BN/XV band, `#`-display family, AB, AN/AR, ME menu. Matrix rows for FR-EQ-01/KEY-01/AUD-CFG-01/VFO-07/ANT-01/MENU-01; FR-PAN-CTL-01/FR-VFO-04 extended. 101 tests, 68 traced, 0 dangling. |
| 2026-07-02 | 0.30 | DC0SK | Phase A: FR-EQ-01 UI — graphic-EQ widget + RX/TX EQ screens (send RE/TE/REF via new WorkerCmds); `eq_bands` pure helper tested. Retired stub context sub-row (context_items/ViewMode::next/CycleViewMode removed). 102 tests, 68 traced, 0 dangling. |
| 2026-07-02 | 0.31 | DC0SK | Phase B: DISPLAY screen (FR-PAN-CTL-01) + BAND screen (FR-VFO-04) wired via generic WorkerCmd::Cat over the Phase-0 encoders; `band_buttons`/`waterfall_palettes` pure helpers tested (TC extends FR-VFO-04/FR-PAN-CTL-01). 103 tests, 68 traced, 0 dangling. |
| 2026-07-02 | 0.32 | DC0SK | Phase C: TX/Fn/MENU screens wired (KS/KP/MI/MG/MS/LI/AB/MO) via WorkerCmd::Cat; `mic_inputs` helper tested; retired obsolete `menu_screen_synopsis`; FR-UI-19 re-anchored on pure `screen_kind` (TC-UI-15). 104 tests, 68 traced, 0 dangling. |
| 2026-07-02 | 0.33 | DC0SK | Phase D: RX/TX antenna (AR/AR$/AN) + LINE OUT (LO) config-row tabs; added `set_rx_antenna_sub` + `ui::rx_antenna_names` (tests extend FR-ANT-01). 105 tests, 68 traced, 0 dangling. |
| 2026-07-02 | 0.34 | DC0SK | Phase-2 memories: FR-SW-01 front-panel switch emulation (`switch`/`SW`); Fn screen gains quick memories (RCL/STO M1–M4) + PF1–PF4. `quick_mem_keys`/`pf_keys` helpers tested. 107 tests, 69 traced, 0 dangling. |
| 2026-07-02 | 0.35 | DC0SK | Full MENU list: `menu_items` (89 items from D12) + `menu_search` (TC-MENU-02); MENU screen is a searchable scrollable list (tap → MO). 108 tests, 69 traced, 0 dangling. |
| 2026-07-02 | 0.36 | DC0SK | Outbound-only completions: FR-VOX-01 (`VX`), FR-TX-MSG-01 (`KY` text send); BAND XVTR (`XV` fixed to 2-digit), TX TEXT/VOX tabs, Fn Switches (`SW` SPOT/TUNE/ATU/DIV/LOCK/MON) + searchable DX-list tabs (`dx_prefixes`/`dx_search`). 110 tests, 71 traced, 0 dangling. |
| 2026-07-03 | 0.37 | DC0SK | Read-back: `RadioState` parses 20+ config-screen RESPs (`RE`/`TE`/`KP`/`KS`/`MI`/`MG`/`LO`/`AN`/`AR`/`VX`/`BN`/`#REF`/`#SPN`/`#SCL`/`#DPM`/`#WFC`/`#WFH`); connect seed requests them; snapshot carries `RadioState`; screens seed from the radio on connect (TC state read-back). TLS-PSK (`connect_tls`, FR-AUTH-02) fixed — handshake timeout + OpenSSL seclevel. 111 tests. |
| 2026-07-03 | 0.38 | DC0SK | FR-UI-20 (config-screen read-back seed) traced from `seed_from_radio` + `RadioState` test (TC-STATE-RB). FR-AUTH-02 verified **live** against a real K4 (correct PSK scheme confirmed clean-room vs QK4). Keychain I/O moved off the UI thread (was a latent Connect freeze). Removed one-off diagnostics; kept the `probe` state tool. 111 tests. |
| 2026-07-03 | 0.39 | DC0SK | UI polish requirements: FR-UI-18 extended (About version/donate/openable links; `about_content` test), FR-UI-21 (landscape default window, TC-UI-21), FR-UI-22 (phase-coloured connection indicator, TC-UI-22). Landscape + About + TX-box CLR verified by screenshot. 113 tests, 74 traced, 0 dangling. |
| 2026-07-03 | 0.40 | DC0SK | Recorded proposed requirements in the matrix: FR-UI-23 (settings dialog), FR-AUD-DEV-01 (audio-device selection), FR-AUD-LVL-01 (volume/mic sliders), NFR-PORT-02 (RPi arm64/Linux x86_64/Windows/macOS), NFR-PKG-01 (.deb/PKGBUILD/Win/macOS packaging). Status P (not yet implemented). |
| 2026-07-03 | 0.41 | DC0SK | Implemented peer cache + master-password crypto (k4-config `crypto`/`peer`: Argon2id + ChaCha20-Poly1305, TC-CRYPTO-01/TC-PEER-01) — FR-CFG-04, NFR-SEC-03 verified. Settings dialog (FR-UI-23): connection form moved in, saved-peers list (Use/Del), keychain-vs-master storage; peers cached on connect + persisted (FR-CFG-05, TC-CFG-03). Audio device/level controls still pending. 120 tests, 77 traced, 0 dangling. |
| 2026-07-03 | 0.42 | DC0SK | Audio device selection + level sliders: `k4-audio` device enumeration/`with_device`/`set_volume`/`set_mic_gain`; worker cmds; settings dialog Audio section (Speaker/Mic pick-lists + Volume/Mic-gain sliders). Persisted in prefs incl. theme (FR-AUD-DEV-01/LVL-01, FR-CFG-05). Completes FR-UI-23. 120 tests, 79 traced, 0 dangling. |
| 2026-07-03 | 0.43 | DC0SK | Packaging + CI (NFR-PORT-02/PKG-01): enabled `ci.yml` (fmt/clippy/test/gate/audit across Linux x86_64+arm64, macOS, Windows) + `release.yml` (tag → .deb/.tar.gz/.zip). Added cargo-deb metadata (`.deb` built + inspected locally), Arch `PKGBUILD` (.SRCINFO valid), macOS bundle metadata, `vendored-tls` feature (Win/macOS OpenSSL from source), LICENSE (GPL-3.0), desktop entry, packaging/README. |
| 2026-07-03 | 0.44 | DC0SK | App icon (`packaging/icons/`): master SVG → PNGs (16–512) + font-independent SVG + `.ico`. Wired into the `.deb` (hicolor theme, verified in the archive), Arch `PKGBUILD`, macOS bundle (`.icns` from PNGs), Windows `.exe` (build.rs + winresource), and the runtime iced window (taskbar). |
