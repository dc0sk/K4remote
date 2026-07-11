---
title: "R3 traceability waivers"
status: Active
version: "1.0"
updated: 2026-07-11
---

# R3 waivers

Requirements exempted from the R3 rule (Must/Should + Test must have a
test-context trace), each with a reason. `cargo xtask` reads the `` `ID` ``
in the first column and excludes it from the R3 gate. A waiver whose ID is not
declared in the SRS fails the build (so stale waivers can't accumulate).

Waivers are for requirements that **cannot** be meaningfully unit-tested in the
hardware-free suite — timing/performance budgets that need a bench harness or a
real radio, architectural properties verified by design/demonstration, or
features not yet implemented. Everything else must carry a real test.

| Requirement | Reason |
|---|---|
| `FR-UI-07` | Architectural property (Ver `T/D`): the worker owns all blocking network/audio I/O on its own thread and the UI renders from a lock-free snapshot (`app/src/worker.rs`, `app/src/main.rs`). "The UI never blocks" is structural, not a value a unit test can assert; verified by design review and demonstrated under load in the L4 HIL run. |
| `NFR-PERF-01` | Latency budget (≤150 ms control round-trip, Ver `T/A`): requires wall-clock measurement against a LAN/radio, not available in the hardware-free suite. To be measured in the L4 HIL run and recorded in `docs/test/hil-runs/`. |
| `NFR-PERF-CW` | CW keying-jitter budget (≤10 ms, Ver `T`): requires real-time timing measurement of paddle→`KZ` emission under an OS scheduler; belongs to a bench/HIL run, not a deterministic unit test. |
| `FR-VFO-ID` | Station-ID (`ID`) set/display (Ver `T`, priority `S`): not yet implemented — planned. Remove this waiver and add a round-trip test when the feature lands. |
