---
title: "K4 Remote — Documentation & RE Process"
status: Draft
version: "0.2"
updated: 2026-07-19
authors:
  - Simon Keimer (DC0SK)
---

# K4 Remote — Documentation & Requirements Engineering Process

A remote control panel for the **Elecraft K4** transceiver, written in **Rust**, with a
GUI including (later) spectrum and waterfall displays.

This `docs/` tree is the single source of truth for the requirements-engineering (RE) and
concept phase. Development is **requirements-driven and test-driven (TDD)** with **strict,
consistent traceability** from stakeholder needs down to test cases and test results.

| Status | Version | Date | Author |
|---|---|---|---|
| Draft baseline for review | 0.1 | 2026-06-25 | Simon Keimer (DC0SK) |

---

## 1. Document map

| Doc | Purpose | IDs owned |
|---|---|---|
| [requirements/vision-and-scope.md](requirements/vision-and-scope.md) | Problem, stakeholders, goals, scope, phases, assumptions, constraints, risks | `ASM-`, `CON-`, `RISK-` |
| [requirements/stakeholder-requirements.md](requirements/stakeholder-requirements.md) | What stakeholders need (solution-independent) | `STK-` |
| [requirements/system-requirements.md](requirements/system-requirements.md) | Functional + non-functional software requirements (testable) | `FR-`, `NFR-` |
| [concept/architecture.md](concept/architecture.md) | Concept, architecture, components, data flow, ADRs | `ARC-`, `ADR-` |
| [concept/ui-design.md](concept/ui-design.md) | UI/UX design concept: layout, switchable view mode, semantic colour, interaction model | — (elaborates `FR-UI-*`, `ADR-15`) |
| [concept/k4-screens.md](concept/k4-screens.md) | K4 on-screen config screens (extracted from the manual): per-primary spec + reusable components + CAT gaps + action list | `SCR-` (elaborates `FR-UI-19`) |
| [requirements/k4-operating-gap-analysis.md](requirements/k4-operating-gap-analysis.md) | Operating features of the real K4 (incl. the tap/hold touch model) vs. what the app implements; prioritised gaps | — (proposes `FR-*`) |
| [concept/station-accessories-plan.md](concept/station-accessories-plan.md) | Plan for KAT500 / KPA500 / KPA1500 support: transport, protocols, amplifier safety, phasing | — (proposes `FR-ACC/KPA/KAT-*`) |
| [concept/cat-server-plan.md](concept/cat-server-plan.md) | Plan for a CAT server so third-party logging software can drive the radio through the app | — (proposes `FR-CATSRV-*`) |
| [test/r5-unreached-encoders.md](test/r5-unreached-encoders.md) | R5 waivers: CAT encoders deliberately left with no non-test caller, each with a reason | — |
| [test/test-strategy.md](test/test-strategy.md) | Test approach, levels, verification methods, traceability matrix | `TC-` |
| [user-manual.md](user-manual.md) | **User-facing** operating manual (install, connect, tune, TX, K-Pod, settings) | — |
| [references/](references/) | Vendor documentation (Elecraft K4 Programmer's Reference, manuals) | — |
| [references/external-references.md](references/external-references.md) | Community references (QK4) + extracted K4/0 streaming protocol facts | `R-EXT-` |

## 1a. Document conventions

**Every Markdown document under `docs/` MUST begin with a YAML frontmatter block.** Required
keys: `title`, `status` (`Draft` · `Approved` · `Withdrawn`), `version`, `updated` (ISO date),
`authors` (list). Documents that own an ID prefix also carry `owns: [PREFIX, …]`. The frontmatter
is the machine-readable header; the human-readable Status/Version line in the body may mirror it.

## 2. Identifier scheme

All artifacts carry a stable, never-reused ID. IDs are immutable once published in a
baseline; if a requirement is dropped it is marked `Withdrawn`, not deleted, and its ID is
retired.

| Prefix | Artifact | Example |
|---|---|---|
| `STK-NN` | Stakeholder requirement | `STK-03` |
| `ASM-NN` | Assumption | `ASM-02` |
| `CON-NN` | Constraint | `CON-01` |
| `RISK-NN` | Risk | `RISK-01` |
| `FR-<AREA>-NN` | Functional software requirement | `FR-VFO-02` |
| `NFR-<AREA>-NN` | Non-functional requirement | `NFR-PERF-01` |
| `ADR-NN` | Architecture decision record | `ADR-04` |
| `ARC-NN` | Architecture element / component | `ARC-06` |
| `TC-<AREA>-NN` | Test case | `TC-VFO-02` |

### Functional areas (`<AREA>`)

| Area | Meaning |
|---|---|
| `CONN` | Connection & transport (Ethernet remote, USB/serial) |
| `STREAM` | Binary frame envelope, payload dispatch (CAT/Audio/PAN) |
| `AUTH` | Authentication (SHA-384 / TLS-PSK) & post-auth init |
| `CAT` | CAT protocol engine (codec, SET/GET/RESP, Auto-Info) |
| `SES` | Session: keep-alive, reconnect, multi-client |
| `VFO` | Frequency, VFO, band, RIT/XIT, split |
| `MODE` | Operating mode, bandwidth, filters |
| `RX` | Receiver controls (AGC, gains, atten/preamp, NB/NR) |
| `MTR` | Metering (S-meter, power, SWR, ALC) |
| `TX` | Transmit, PTT, CW keying, voice keying |
| `AUD` | Audio streaming (RX/TX, Opus/PCM) |
| `PAN` | Panadapter / waterfall (Phase 2) |
| `UI` | GUI shell, layout, panels, theming |
| `CFG` | Configuration & persistence |
| `SEC` | Security |
| `DIAG` | Logging & diagnostics |

Non-functional areas: `PERF`, `REL`, `USE`, `PORT`, `MAINT`, `TEST`, `SEC`.

## 3. Requirement attributes

Every `FR`/`NFR` is recorded with:

- **ID**, **Title**, **Statement** (single "shall", testable, unambiguous)
- **Rationale**
- **Source / Trace-up** — upstream `STK-` (and vendor doc reference where applicable)
- **Priority** — `M` (must, v1) · `S` (should, v1 if time) · `C` (could) · `W2` (Phase 2)
- **Verification method** — `T` test (automated) · `D` demonstration · `I` inspection · `A` analysis
- **Acceptance criteria** — the conditions a test asserts
- **Status** — `Proposed` · `Approved` · `Implemented` · `Verified` · `Withdrawn`
- **Trace-down** — `ARC-` element(s) and `TC-` test case(s)

## 4. Traceability model (the V)

```
STK (need)  ──►  FR / NFR (system req)  ──►  ARC / ADR (design)  ──►  TC (test)  ──►  Result
   ▲                    │                                                  │
   └──────── every FR/NFR traces up to ≥1 STK ──────────────────┘         │
                        └──────── every FR/NFR traces down to ≥1 TC ───────┘
```

Tracing rules (see [test/test-strategy.md](test/test-strategy.md) §Coverage gate):

- **R1** Every `STK` is satisfied by ≥1 `FR`/`NFR`. (no orphan needs) — *manual review*
- **R2** Every `FR`/`NFR` traces up to ≥1 `STK`. (no gold-plating) — *manual review*
- **R3** Every `M`/`S` `FR`/`NFR` whose verification includes **Test** is covered by ≥1
  trace **in a test context**, or is waived with a reason in
  [test/r3-waivers.md](test/r3-waivers.md). — **enforced by `cargo xtask`**
- **R4** Every `trace:` ID names a declared requirement (no dangling traces). —
  **enforced by `cargo xtask`**
- **R5** Every implemented `FR` is realized by ≥1 named `ARC` element. — *manual review*

`cargo xtask` (the **coverage gate**) parses the SRS requirement table (priority +
verification method), collects `trace:` annotations, and **fails the build** on: an
unwaived R3 gap, a dangling trace (R4), a duplicate declared ID, or a waiver for an
unknown requirement. It also writes [test/coverage.generated.md](test/coverage.generated.md).
Source-comment `trace:` annotations document intent but do **not** satisfy R3 — only
traces inside `tests/` files or `#[cfg(test)]` modules count. R1/R2/R5 remain review
rules (not yet automated).

## 5. TDD workflow per requirement

1. Pick an `Approved` `FR`/`NFR`.
2. Write the `TC` first (red), tagging the requirement ID in the test name.
3. Implement the minimal `ARC` code to pass (green).
4. Refactor; keep the trace annotation.
5. Update the requirement `Status` → `Verified` and the matrix once the `TC` passes in CI.

## 6. Baseline & change control

- Each published version is a **baseline** (git tag `req-vX.Y`).
- Changes after baseline go through a change note appended to the affected doc's
  **Change History** with date, author, affected IDs, and reason.
- This is a **Draft baseline (0.1)** intended for review; nothing is `Approved` until the
  stakeholder sign-off recorded in §Change History of each document.
