---
title: "Software Audit — gaps, traceability, coverage"
status: Final
version: "1.0"
updated: 2026-07-11
authors:
  - Fable (agent, commissioned by DC0SK)
scope: "Repo @ fa8ea0d (main). Parent-verified headline findings: SRS corruption, xtask R3-soft, worker pump-error swallow."
---

# Software Audit — K4 Remote

**Repo:** `/home/dc0sk/git/K4remote` @ `fa8ea0d` (main, clean tree) · **Audit date:** 2026-07-11
**Toolchain health:** `cargo test --workspace` = **144 passed / 0 failed / 0 ignored** (incl. 2 TLS-PSK
tests). `cargo clippy --all-targets -- -D warnings` = clean. `cargo fmt --check` = clean.
`cargo xtask` = exit 0, "155 declared / 109 traced / 46 uncovered (informational) / 0 dangling".

## 1. Executive summary

**Overall: AMBER (B−).** The library core (`k4-protocol`, `k4-session`, `k4-config`, `k4-stream`,
`k4-audio`) is genuinely well-tested with byte-exact and behavioural tests, and the safety chain
(arm/e-stop/link-loss fail-safe) is implemented and unit-tested. But the traceability gate is
**materially weaker than its own documentation claims**, the entire GUI/worker layer (≈6,800 lines,
incl. reconnect orchestration and optimistic-UI reconciliation) has **0% test coverage**, and the
SRS file itself is **corrupted**.

Top findings:

1. **The gate does not enforce R3.** `docs/README.md` and `test-strategy.md` §4 say the build
   **fails** if any M/S requirement lacks a test and that a coverage report is emitted. In reality
   `xtask/src/main.rs:7-8,108` prints uncovered requirements as *"informational while scaffolding"*
   and exits 0; `docs/test/coverage.generated.md` does not exist; R1/R2/R5 are not implemented. 46
   requirements are uncovered and CI stays green.
2. **The gate's parser both over- and under-counts.** A `trace:` comment anywhere in non-test source
   satisfies coverage (5 requirements are "covered" this way), while a trace list wrapped onto a
   second comment line is silently dropped — `FR-TX-SAFE-03/04` **are** properly tested
   (`k4-session/tests/session.rs:168,177`) yet the gate reports them *uncovered* because the list
   wraps at `session.rs:1-2`.
3. **The SRS is corrupted.** `docs/requirements/system-requirements.md` contains a duplicated head
   block spliced into the middle of the `FR-DATA-01` row (statement truncates at `` `DT`/`DT ``,
   frontmatter restarts, row resumes later). ~40 requirement rows are declared twice
   (`grep -c FR-CONN-01` = 2). 98 lines carry double-encoded UTF-8 mojibake (only this file).
   *Parent note: introduced by commit `168c211` (the DATA-sub-mode PR); a `perl -0pi` edit to insert
   the `FR-DATA-01` row duplicated the head block.*
4. **Real M-priority code gaps exist behind the green build:** `FR-CAT-03` (error reply `<cmd>?;`)
   has zero handling in `k4-protocol`; `FR-CONN-05` connect timeout is not implemented
   (`k4-transport/src/lib.rs:108,135` use bare `TcpStream::connect`); and `app/src/worker.rs:594`
   silently swallows `session.pump()` I/O errors, so a hard socket error during TX unkeys only after
   the 5 s tick timeout — putting **NFR-REL-FAILSAFE's ≤1 s bound** in doubt on that path.
5. **The gate is trustworthy for R4 only** (no dangling IDs — independently confirmed). "Gate green"
   ≠ "requirements met". The *manual* traceability matrix in `test-strategy.md` is more honest than
   the automated gate: it correctly marks FR-CAT-03/04, FR-CONN-03/04, FR-TX-01, NFR-PERF-\*,
   NFR-REL-\* as status `P` (planned).

The project is deliberately pre-hardware; hardware-validation gaps are consistently and honestly
flagged in the change-log (mini-pan 0x03 layout, `ME` sweep RESP shape, ACM/ACS antenna-mask
mapping, per-pan `$` targeting, VFO-centred pan assumption). Those are *not* counted as defects.

## 2. Gaps

Hygiene greps: no `TODO`/`FIXME`/`todo!`/`unimplemented!`, no `#[ignore]`, no `#[allow]`; one
`unreachable!()` (`app/src/main.rs:2418`, UI msg dispatch); `unwrap/expect/panic` in non-test
source: 23 sites, all inspected — test helpers, sim mutex `.expect`, a `from_utf8(const).unwrap()`
(`crypto.rs:108`); none load-bearing. 5 explicit "follow-up" markers.

| # | Item | Location | Sev | Type | Recommendation |
|---|------|----------|-----|------|----------------|
| G1 | `pump()` I/O errors swallowed: socket error during TX does not trigger immediate fail-safe; unkey waits for the 5 s `link_timeout` in `tick()`. NFR-REL-FAILSAFE demands safe state ≤1 s | `app/src/worker.rs:594`; `k4-session/src/lib.rs:179-201,212-216` | **Critical** | Code | On `pump()` `Err`, apply the fail-safe + mark the link lost immediately; add a fault-injection test |
| G2 | `FR-CAT-03` (M/T): error reply `<cmd>?;` never recognised or mapped to the originating request | `k4-protocol/src/cat.rs`, `state.rs` (0 hits); matrix `P` | High | Code | Implement pending-request error surfacing + test |
| G3 | `FR-CONN-05` (S/T): no connect timeout — `TcpStream::connect(addr)?` blocks at OS default | `k4-transport/src/lib.rs:108,135` | High | Code | Use `TcpStream::connect_timeout`; test against a non-responding listener |
| G4 | SRS file corrupted: duplicated head block, ~40 rows declared twice; 98 mojibake lines | `docs/requirements/system-requirements.md`; commit `168c211` | High | Docs | De-duplicate, fix encoding; add an xtask check for duplicate IDs / repeated frontmatter |
| G5 | App orchestration layer 0% tested: reconnect + state restore, optimistic-VFO reconciliation, `sync_locals` genuine-transition, PTT hotkey toggle/hold, half-duplex playback suppression | `app/src/worker.rs` (0/511), `app/src/main.rs` (0/4264) | High | Code/tests | Extract reconcile/sync logic into pure functions (the `ui.rs` pattern proves this — 93.6% covered) |
| G6 | `NFR-PERF-01/CW/AI`, `NFR-REL-01` (all M): no benchmark harness, no fuzz/garbage-input test | matrix `P` | Med | Tests | A no-panic fuzz loop over `FrameDecoder`+`apply_cat` is cheap |
| G7 | `FR-AUD-ENC` (M/T): `EM3` default implemented but untested; no UI path selects raw PCM on LAN | `k4-transport/src/lib.rs:84,189` | Med | Code+tests | Assert init emits `EM3;`; expose encode-mode setting or descope |
| G8 | `FR-UI-08` acceptance drift: SRS says `ViewMode` cycles A→B→dual→A but `ViewMode::next` was removed (v0.30); trace is a token default assertion | `app/src/ui.rs` vs SRS | Low | Docs | Update the acceptance criterion to the segmented selector |
| G9 | Panic path in UI dispatch | `app/src/main.rs:2418` `unreachable!()` | Low | Code | Replace with a no-op arm |
| G10 | Hardware-validation gaps (flagged by the author): frame/auth/audio/PAN layouts vs real K4, mini-pan 0x03, `ME` sweep shape, ACM/ACS map, per-pan `$`, click-QSY assumption, audio E2E, D-verified `FR-UI-*` | changelog cites | Med (aggregate) | **Hardware** | Schedule an L4 HIL run; record in `docs/test/hil-runs/` (does not exist yet) |

Orphan features: none material — every sampled feature maps to an ID; the optimistic-tuning
*mechanism* has no requirement of its own (only FR-VFO-08's symptom fix) and is the least-specified
behaviour in the codebase.

## 3. Traceability

**What the gate actually guarantees** (`xtask/src/main.rs`):
- R4, hard: every `FR-`/`NFR-` token on a line containing `trace:` names a declared SRS ID
  (`main.rs:113-122`). Verified independently: 0 dangling. ✔
- R3, soft only: uncovered list printed, never fails (`main.rs:7-8,108`). ✖ contradicts
  `docs/README.md` ("fails the build if R1–R5 are violated") and `test-strategy.md` §4.

**Blind spots (each demonstrated):**

| Blind spot | Evidence |
|---|---|
| A `trace:` in a plain source comment with no assertion satisfies coverage | `FR-AUD-MON-01`/`FR-TX-PTT-01` → `main.rs:1546` comment; `FR-FIL-03`/`FR-PAN-04` → `spectrum.rs:40,136`; `FR-TX-SAFE-01` → `k4-session/src/lib.rs:205` (also tested — but the gate credits the wrong site) |
| Multi-line trace lists silently truncated → false "uncovered" | `k4-session/tests/session.rs:1-2`: line 2 (`FR-TX-SAFE-01/03/04`) has no `trace:` → gate lists SAFE-03/04 as uncovered despite real tests at `session.rs:168,177` |
| Verification method (`T/D/I/A`) and priority ignored — the 46-item uncovered list mixes ~21 legitimately demo/inspection-verified items with ~17 genuine M/S `T` violations | see below |
| R1/R2/R5, `STK-*`/`R-EXT-*` IDs, duplicate-ID detection, and the promised `coverage.generated.md` — all unimplemented | `xtask/src/main.rs:23-29` matches only `FR-`/`NFR-`; `ls docs/test/` = `test-strategy.md` only |
| The gate itself has zero tests | llvm-cov: `xtask/src/main.rs` 0% |

**Genuine R3 violations** (M/S priority, Ver includes `T`, no trace anywhere): `FR-AUD-ENC`,
`FR-CAT-03`, `FR-CAT-04`, `FR-CONN-03`, `FR-CONN-04`, `FR-CONN-05`, `FR-PAN-CTL-02`, `FR-TX-01`†,
`FR-UI-07`, `FR-VFO-03`, `FR-VFO-ID`, `NFR-PERF-01`, `NFR-PERF-AI`, `NFR-PERF-CW`, `NFR-REL-01`,
`NFR-REL-FAILSAFE`, `NFR-TEST-02`†. († = substantively tested but untraced.)

**Trace quality (10-requirement sample):** FR-VFO-01 (byte-exact) **strong**; FR-AUTH-01 (SHA-384
known-answer) **strong**; FR-TX-SAFE-01/03/04 (behavioural, mock link + injected clock) **strong**;
FR-CAT-06 (`$`-routing) **strong**; NFR-SEC-03 (wrong-pw + tamper + nonce-uniqueness) **strong**;
NFR-SEC-01 (redaction) **adequate**; FR-SES-RECONNECT (**partial** — pure backoff only; actual
reconnect/re-seed path in `worker.rs:671-689` untested); FR-UI-08 (**token**); FR-VFO-04 in `ui.rs`
(**token**, though the real encoder test in `cat.rs` is strong); FR-DIAG-01 **adequate**. Net: the
*tested* traces are mostly genuine; weakness is in what's untraced, not fake assertions.

**Three-way consistency:** `test-strategy.md` v1.10 is current and its per-requirement statuses are
more accurate than the gate. Drift: strategy §4 describes a gate that doesn't exist; SRS header reads
"Version 0.1" while frontmatter says 0.14; FR-UI-08 acceptance vs removed API; strategy §6's
`docs/test/hil-runs/` never created despite two "verified live against a real K4" claims.

**Verdict:** *Gate green means only "no dangling IDs and it compiled".* It does not mean requirements
are tested, let alone met. Trust the manual matrix, not the gate banner.

## 4. Coverage

`cargo llvm-cov --workspace --summary-only` (lines):

| Area | Lines cover | Notes |
|---|---|---|
| **TOTAL** | **29.64%** (2392/8070) | Regions 27.5%, functions 50.8% |
| k4-protocol | cat 96.7 · cw 100 · frame 95.5 · auth 100 · **state.rs 72.5** | `apply_cat` ~130 uncovered lines — newest parse branches (TM/RP/PL/ACM/VT/DT) |
| k4-session | 78.3 | uncovered: error-propagation paths |
| k4-transport | 82.3 | TLS handshake partially covered |
| k4-audio | codec 100 · jitter 100 · resample 94 · ring 87 · **device.rs 0** | device.rs is cpal/hardware — L4 by design |
| k4-config | backup 100 · crypto 88.6 · peer 93.7 · lib 71 · **secret.rs 38.5** | uncovered = KeyringStore (needs OS keychain) |
| k4-stream / k4-sim | 81–100 / 96.6 | good |
| **app** | ui.rs **93.6** · **main.rs 0 (4264)** · **worker.rs 0 (511)** · meter 0 · spectrum 0 | the single biggest hole |
| xtask | 0 | the gate is untested |

**Highest-risk under-tested areas, ranked:**
1. **Worker error/reconnect paths** (`worker.rs:589-691`): pump-error swallow (G1), auto-reconnect +
   re-seed, phase transitions on loss. Add: *NFR-REL-FAILSAFE* (mock link erroring mid-TX ⇒ safe
   ≤1 s), *FR-SES-RECONNECT* (drop ⇒ reseeded restore), *FR-CONN-03/04* (each failure ⇒ distinct status).
2. **Optimistic-UI reconciliation** (`main.rs`, added v0.99–1.00): staleness fallback, rapid-click
   accumulation, echo reconcile — all 0%. Extract pure and test under *FR-VFO-03/08*.
3. **CAT parser breadth** (`state.rs`): no fuzz (*NFR-REL-01*); no error-reply handling (*FR-CAT-03*);
   silent-drop catch-alls untested (*FR-CAT-04*: assert unknown `ZZ9;` leaves state unchanged).
4. **TX-audio gating end-to-end**: session-level gate tested, but the worker's
   mic-capture→encode→send loop and half-duplex suppression (`worker.rs:601-662`) are untested.
5. **Crypto/auth**: well-tested; residual risks are design-level (SHA-384-of-password sent raw on
   9205 — inherent to the K4 protocol, mitigated by TLS-PSK default; `MasterKey` zeroization via a
   plain loop could be optimised away — use `zeroize`).

## 5. Prioritised action list

| # | Action | Tag | Kind |
|---|--------|-----|------|
| 1 | On `session.pump()` error, apply fail-safe + declare link lost immediately (`worker.rs:594`); add mid-TX fault-injection test tracing `NFR-REL-FAILSAFE` | **Critical** | Fix code |
| 2 | Repair `system-requirements.md`: remove the duplicated head block, restore `FR-DATA-01`, fix mojibake | **High** | Fix docs |
| 3 | Make xtask match its spec: parse Pri/Ver, fail on M/S+`T` uncovered; count only traces in `tests/`/`#[cfg(test)]`; handle wrapped trace lists; detect duplicate declared IDs; emit `coverage.generated.md` — or downgrade the claims in `docs/README.md` / strategy §4 | **High** | Fix traceability |
| 4 | Implement `FR-CAT-03` error-reply recognition + test; add `FR-CAT-04` unknown-frame test | **High** | Fix code+tests |
| 5 | Add missing traces for already-passing behaviour: `FR-TX-01` → `session.rs:168`, `NFR-TEST-02` → sim suite; un-wrap `session.rs:1-2` trace list | **High** | Fix traceability (cheap) |
| 6 | Implement `FR-CONN-05` via `TcpStream::connect_timeout` + test | **Med** | Fix code |
| 7 | Add a no-panic fuzz loop (frames + CAT garbage) tracing `NFR-REL-01`/`NFR-PERF-AI` | **Med** | Fix tests |
| 8 | Extract worker/main pure logic (reconcile, sync_locals, reconnect decisions) and test it; target ≥50% on `worker.rs` | **Med** | Fix code+tests |
| 9 | Assert init emits `EM3;` (`FR-AUD-ENC`); decide on the LAN-PCM sub-claim | **Med** | Fix tests/docs |
| 10 | Run the first documented L4 HIL session (`docs/test/hil-runs/`): framing, mini-pan 0x03, `ME` sweep, ACM/ACS map, per-pan `$`, audio E2E, D-verified `FR-UI-*` | **Med** | **Needs a real K4** |
| 11 | Replace `main.rs:2418` `unreachable!()`; adopt `zeroize` for `MasterKey`; fix FR-UI-08 acceptance text; reconcile SRS header vs frontmatter version | **Low** | Polish |

**Bottom line:** the requirements-first discipline is real and the tested core is strong — but the
automated gate currently *performs* rigour rather than enforcing it (R3 toothless, both false-positive
and false-negative trace accounting), the app layer is a coverage void around genuinely
safety-relevant glue, and one Critical error-path (pump-error during TX) deserves a fix before any
on-air use.
