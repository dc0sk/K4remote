---
title: "Hardware-in-the-loop (HIL) test runs"
status: Active
version: "1.0"
updated: 2026-07-11
---

# L4 hardware-in-the-loop runs

The hardware-free suite (L1–L3, `cargo test`) validates everything that can be
checked against the protocol simulator. The items below can only be confirmed
against a **real Elecraft K4/K4D** and are recorded here, one file per session:
`YYYY-MM-DD-<operator>.md`, using the template in this file.

Until a run confirms them, these are **validation gaps** (the code is written and
unit-tested where possible; the wire/behaviour contract is unverified on iron).

## Checklist to confirm on hardware

- **Framing & auth** — real connect handshake (plaintext 9205 + TLS-PSK 9204),
  SHA-384 auth, init sequence order (`ASM-05`, FR-AUTH-01/02/03).
- **Audio E2E** — full-duplex 12 kHz Opus RX playback + TX mic capture, L=main /
  R=sub channel mapping (FR-AUD-RX-01/TX-01, `device.rs`).
- **Panadapter** — spectrum + waterfall from the live `#PAN` stream, and the
  **mini-pan 0x03** layout (assumed identical to 0x02).
- **`ME` menu sweep** — full-menu export RESP shape (FR-CFG-06).
- **ACM/ACS antenna masks** — the a–g → `AR$` enabled-antenna mapping (best-effort).
- **Per-pan `$` targeting** — sub-RX pan/spectrum routing.
- **Click-to-QSY** — the VFO-centred pan assumption for click tuning.
- **Waived performance budgets** (see [r3-waivers.md](../r3-waivers.md)):
  - `NFR-PERF-01` control round-trip ≤150 ms (LAN).
  - `NFR-PERF-CW` CW keying jitter ≤10 ms.
  - `FR-UI-07` UI stays responsive under network/audio load.
- **D-verified UI** — the `FR-UI-*` items whose method is Demonstration.
- **K-Pod** (`FR-KPOD-04`, built with `--features kpod`) — enumerate the HID
  device (VID `0x04D8` / PID `0xF12D`), confirm the report framing (feature-report
  vs interrupt read on EP0), rocker → VFO A/B/RIT-XIT selection, encoder tuning in
  both directions, and the indicator LEDs D1/D2/D3.

## Template

```md
# HIL run — YYYY-MM-DD — <operator>

- Radio: K4 / K4D, firmware <ver>
- Client: k4remote <git sha>, OS <...>
- Transport: plaintext 9205 / TLS-PSK 9204 / serial

| Item | Result | Notes |
|---|---|---|
| Connect + auth (plaintext) | pass/fail | |
| Connect + auth (TLS-PSK) | pass/fail | |
| RX audio playback | pass/fail | |
| TX mic capture | pass/fail | |
| Spectrum + waterfall | pass/fail | |
| Mini-pan (0x03) | pass/fail | |
| ME menu sweep | pass/fail | |
| ACM/ACS antenna map | pass/fail | |
| Per-pan $ targeting | pass/fail | |
| Click-to-QSY | pass/fail | |
| NFR-PERF-01 round-trip (ms) | <measured> | ≤150 ms budget |
| NFR-PERF-CW keying jitter (ms) | <measured> | ≤10 ms budget |
| FR-UI-07 responsiveness under load | pass/fail | |
| K-Pod enumerate (VID/PID) | pass/fail | `--features kpod` |
| K-Pod report framing | pass/fail | feature-report vs interrupt |
| K-Pod rocker → VFO A/B/RIT | pass/fail | |
| K-Pod encoder tuning | pass/fail | both directions |
| K-Pod indicator LEDs | pass/fail | D1/D2/D3 |

Issues found / follow-ups:
```
