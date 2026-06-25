---
title: "Test Strategy & Traceability"
status: Draft
version: "0.21"
updated: 2026-06-25
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
| FR-PAN-CTL-01 | STK-10 | ARC-04 | TC-PAN-01 (`#SPN/#REF/...` encode) | L1 | P |
| FR-PAN-01 | STK-09 | ARC-10 | TC-PAN-02 (PAN decode: meta + bins, dBm=byte−146) | L3 | V\* |
| FR-PAN-02 | STK-09 | ARC-11 | TC-PAN-03 (dbm→y scaling) + canvas (L4) | L1 | Impl |
| FR-PAN-03 | STK-09 | ARC-11 | TC-PAN-04 (waterfall colormap) + canvas (L4) | L1 | Impl |
| FR-UI-04 | STK-08/11 | ARC-08 | TC-UI-01 (arm/e-stop affordances) | L4 | P |
| FR-UI-07 | STK-11 | ARC-08 | TC-UI-02 (no UI block under load) | L5 | P |
| FR-CFG-03 | STK-12/14 | ARC-12 | TC-CFG-01 (no plaintext secret) | L1 | V\* |
| FR-CFG-01 | STK-12 | ARC-12 | TC-CFG-02 (profile TOML save/load) | L1 | V\* |
| FR-CFG-02 | STK-12 | ARC-12 | TC-CFG-03 (prefs round-trip) | L1 | V\* |
| FR-DIAG-03 | STK-14/17 | ARC-13 | TC-DIAG-01 (secret redaction) | L1 | V\* |
| FR-DIAG-01 | STK-17 | ARC-13 | TC-DIAG-02 (DiagLog level/cap/format) | L1 | V\* |
| FR-DIAG-02 | STK-17 | ARC-13 | raw CAT console (send + Inbound.cat surfaced) | L1/L4 | Impl |
| NFR-PERF-01 | STK-01/02 | ARC-03..06 | TC-NFR-01 (control latency bench) | L5 | P |
| NFR-PERF-CW | STK-07 | ARC-06 | TC-NFR-02 (CW jitter bench) | L5 | P |
| NFR-REL-FAILSAFE | STK-08 | ARC-06/07 | TC-NFR-03 (≤1s safe state) | L5 | P |
| NFR-REL-01 | STK-17 | ARC-03 | TC-NFR-04 (fuzz no-panic) | L5 | P |
| NFR-SEC-01 | STK-14 | ARC-13 | TC-NFR-05 (no secret in logs) | L1 | V\* |
| NFR-TEST-01 | STK-15 | xtask | TC-NFR-06 (trace gate green) | L5 | P |
| NFR-PORT-01 | STK-16 | app | TC-NFR-07 (CI build matrix) | L5 | P |

> `V*` = implemented; test passes locally via `cargo test`. Promote to `V` once a CI pipeline runs the gate. As of this baseline: **79 tests pass**; password stored in the OS keychain via the `SecretStore` abstraction (FR-CFG-03), never in the TOML config (k4-protocol 30, k4-stream 7, k4-audio 12, k4-sim 1, k4-transport 3, k4-session 12). `cargo xtask`: 43 traced, 0 dangling (R4 ✓). `cargo audit`: clean. TLS-PSK is a default app feature (real OpenSSL); the 2 TLS tests run in the default suite. The iced app builds clippy-clean (FR-UI-* are L4, pending a display). Opus codec + cpal device I/O are the remaining L4 audio step.

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
