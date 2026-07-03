---
title: "UI Design Concept"
status: Draft
version: "0.2"
updated: 2026-07-02
authors:
  - Simon Keimer (DC0SK)
---

# UI Design Concept

The canonical UI/UX concept for the K4 Remote GUI shell. It elaborates the
`FR-UI-*` requirements (system-requirements.md §K) and `ADR-15`, and is sourced
from `R-EXT-02` (the K4 native LCD renders + third-party panels).

**Design thesis (rev. 0.2, 2026-07-02):** reproduce the **K4's own operating
conventions** so a K4 operator is instantly at home (interoperability
faithfulness, like the protocol), and style the client **after the references**
— the K4 LCD's banded frame mixed with the reference client's visual language
(dark layered surfaces, rounded button grids, big white frequency readouts,
proportional S-meter bars). All expression is **re-implemented from scratch
with our own values**; no third-party assets, iconography, branding, or code
are copied. See the adopt/diverge table in `R-EXT-02` and the revised `ADR-15`.

---

## 1. Layout — fixed horizontal bands, A/B symmetric

The main window stacks fixed-height bands, mirrored A-left / B-right (the one
asymmetric element — the shared transmit box — sits **between** the VFOs so split
awareness is unavoidable):

```
┌─────────────────────┬──────────┬─────────────────────┐
│ A  14.070.000  USB  │  ◀ TX    │  B  14.061.10  USB  │   1. VFO header (per VFO):
│ S════════ −82 dBm   │  SPLIT   │  S═══════  −88 dBm  │      freq · mode · S-meter
│ AGC-S PRE1 ANT1 NB  │  RIT/XIT │  AGC-S PRE1 ANT1    │   2. RX icon strip (lit=on)
├─────────────────────┴──────────┴─────────────────────┤
│            panadapter   (single A / B / dual)         │   3. spectrum + waterfall
│            waterfall     scale in dBm or S-units       │      (ViewMode-driven)
├───────────────────────────────────────────────────────┤
│  ?  MENU  Fn  DISPLAY  BAND  MAIN RX  SUB RX  TX       │   4. 7 fixed primaries
└───────────────────────────────────────────────────────┘      → swap a context row
```

## 2. ViewMode — switchable single/dual (mirrors the K4 `PAN=A/B/A+B`)

The panadapter/header region is driven by a **`ViewMode`** the operator toggles,
exactly like the K4's `DISPLAY → PAN=A / PAN=B / PAN=A+B`:

| `ViewMode` | Header | Panadapter | Use |
|---|---|---|---|
| `SingleA` | VFO A focus | one pane (A) | single-RX operating; narrow windows |
| `SingleB` | VFO B focus | one pane (B) | sub-RX / split watch |
| `Dual` | A and B mirrored | two panes side-by-side | diversity / dual-watch |

- Cycling order matches the radio: `SingleA → SingleB → Dual → SingleA …`.
- The layout **reflows** to the active mode (`FR-UI-08`). A narrow window may
  default to `SingleA`; this is a presentation choice, not a separate layout.
- Maps to the per-receiver PAN packet `receiver` field (`R-EXT-01`): `Dual`
  consumes both, `SingleX` consumes one.

## 3. Interaction model — 7 primaries + context row

The bottom row holds 7 always-present primary buttons. Tapping one **swaps a
context sub-row** in just above it; everything uses this one idiom (`FR-UI-13`):

| Primary | Context row |
|---|---|
| `BAND` | band grid (`1.8 … 50`, `GEN`, `MEM`, `XVTR`) |
| `DISPLAY` | panadapter controls (PAN=A/B/A+B, REF/SCALE, SPAN, WTRFALL…) |
| `MAIN RX` / `SUB RX` | RX config (ANT CFG, RX EQ, AGC, AFX, APF/TEXT-DEC — mode-dependent) |
| `TX` | transmit config (MIC, VOX, keyer weight, EQ) |
| `Fn` | special functions / macros |
| `MENU` | settings (our **panel**, not the radio's locked scroll list) |

Supporting idioms adopted from the K4: **tap-to-edit** (tap mode icon → mode grid;
tap MHz digits → memory recall), **mode-dependent context** (CMP meter in voice,
TEXT DEC in CW), **mini-pan** zoom tuning aid (tap S-meter), and a **B SET**-style
modifier retargeting shared controls to VFO B.

## 4. Visual language

- **Theme (`FR-UI-15`):** dark, near-black background with **layered surfaces**
  — background → panel → control step up strictly in luminance (`ui::Shade`,
  luminance ordering is unit-tested) so depth reads without heavy chrome.
  Flat, slightly-rounded rectangular buttons in grids; **engaged toggles fill
  blue** with white text (the reference client's idiom); transmit-critical
  controls carry a red edge and fill amber while engaged.
- **Semantic colour** (`FR-UI-10`) — *our palette, the K4's meaning*:

  | Role | Meaning |
  |---|---|
  | **TxActive** (amber/orange) | transmit state and transmit-side values |
  | **VfoA** (blue) | VFO A / main receiver |
  | **VfoB** (green) | VFO B / sub receiver, and "active/selected" |
  | **RxValue** (near-white) | receive-side readouts |
  | **Caution** (yellow) | warnings (e.g. high SWR) |
  | **Inactive** (dim grey) | an off/available control |

  TX indication must be unmistakable (`FR-UI-06`, `NFR-USE-01`).
- **Two-line state buttons** (`FR-UI-11`): top = function label, bottom = live
  value/unit derived from `RadioState` (e.g. `ATT`/`Off`, `AGC`/`Slow`,
  `BW`/`2.80`). The button *is* the status readout — no separate status panel.
- **Frequency readout** (`FR-UI-09`): large, light weight, **dot-grouped to kHz**
  (`14.070.000`), the K4's own grouping.
- **S-meter (`FR-UI-15`):** a **proportional bar** on the K4 meter face —
  S1 ≈ −121 dBm … S9+60 dB ≈ −13 dBm with S9 = −73 dBm (`s_meter_fraction`,
  unit-tested, clamped) — green fill turning caution-yellow at ≥ S9, plus a
  numeric S-unit + dBm readout (`SMH`).

## 5. What is deliberately *ours* (provenance)

Per the revised `ADR-15`: every visual is **re-implemented from scratch** — our
own palette constants, spacing, and widget code; **no** third-party assets,
iconography, branding, or code are copied (`R-EXT-02` class B, `CON-09`). We
also diverge functionally where the desktop calls for it: a **resizable**
window with responsive band stacking (the K4 is a fixed 7" panel), a real
settings/menu panel, explicit TX arm / emergency-stop affordances
(`FR-TX-SAFE`), and no skeuomorphic hardware-knob clusters.

## 6. Realisation & test seam

Layout/styling is demonstrated (`D`) on a running app; the **decidable view-model
logic is pure and unit-tested** (`T`) with no iced dependency, in `app/src/ui.rs`
(`ARC-15`): `ViewMode` cycling, dot-grouped frequency formatting, semantic-colour
role selection, two-line button state derivation, band layout/reflow, the layered
shade palette, and the S-meter scale. The iced layer (`main.rs`) only maps roles/
shades to colours and states to widgets. This keeps the visual layer thin and the
traceability gate green for the testable `FR-UI-*` items.
