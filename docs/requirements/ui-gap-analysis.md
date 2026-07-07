---
title: "UI & Requirements Gap Analysis"
status: Draft
version: "0.1"
updated: 2026-07-07
authors:
  - Fable (agent, commissioned by DC0SK)
---

# UI & Requirements Gap Analysis

A rescan of *Intro to the Elecraft K4* rev C5 and the *K4 Programmer's Reference*
rev D12 against the current SRS ([system-requirements.md](system-requirements.md))
and the implemented UI (`app/src/`), to find K4 functionality that is documented
but not yet in our requirements or UI. This is an input to a requirements-update
session — items here are **candidates**, not yet accepted requirements.

## Top gaps to close first

1. **Squelch is completely absent** (`SQ`/`SQ$`) and **AF/RF gain have encoders
   but no UI controls** — the whole RF/SQL multifunction knob (M.RF / M.SQL /
   S.RF / S.SQL / BAL) has no remote equivalent. Core operating controls.
2. **TX power (`PC`) is spec'd (`FR-TX-02`) but unimplemented**; the companion
   **QSK/VOX delay (`SD`)** isn't even spec'd. This is the PWR/DLY knob pair.
3. **Speech compression (`CP`), TX monitor level (`ML`), CW pitch (`CW`)** — the
   rest of the XMTR knob (MIC/CMP, WPM/PITCH, hold-MON) — are missing. Only MIC
   (`MG`) and WPM (`KS`) exist.
4. **Passband tuning is BW-only**: no shift (`IS$`), no hi/lo-cut view, no
   NORMalize, no filter presets (`FP`, spec'd `FR-MODE-03`).
5. **Diversity (`DV`) and sub-RX on/off (`SB`) have no CAT path** — DIV is a
   blind `SW152;` tap, SUB isn't reachable.
6. **Notch (`NM$`/`NA$`) and APF (`AP$`)** — a whole switch column (NTCH,
   FIL/APF) — are missing.
7. **~20 front-panel tap/hold functions are unreachable** (SPOT-hold autospot,
   NB/NR level adjust, RATE/KHZ, SUB/DIV, B SET, REV, STORE/RCL/BANK, MODE-alt,
   knob holds NORM/BAL).

## A. RX controls

| Feature (Intro ref) | CAT (D12) | Status | Gap / proposed requirement |
|---|---|---|---|
| Main/sub squelch — M.SQL, S.SQL | `SQ$nnn;` 000–040, toggle `SQ$/`, `SQ$+/-`; non-FM needs MENU 106 | **Missing** | `FR-RX-SQL-01`: set/display main+sub squelch (`SQ`/`SQ$`), incl. all-mode squelch enable (ME 0106). |
| Main/sub RF gain — M.RF, S.RF | `RG$-nn;` 0…−60, `RG$/` | **Partial** — `FR-RX-01` spec'd, `set_rf_gain` exists, no UI | Impl gap on `FR-RX-01`: main+sub RF-gain sliders. |
| AF gain — AF / SUB AF | `AG$nnn;` 000–060 | **Partial** — `set_af_gain` exists, no radio-side UI (volume slider is client-side) | Impl gap on `FR-RX-01`: radio AF sliders main+sub. |
| Balance — BAL (knob hold) | `BLm+nn;`, `BL~` | **Missing impl** — `FR-RX-06` mentions `BL` | Backlog with sub-RX work. |
| Sub RX on/off — SUB tap | `SBn;`, `SB/`; `SW83` | **Missing impl** — `FR-RX-06` | Raise priority; prereq for diversity/dual-pan/S.RF/S.SQL. |
| Manual + auto notch — NTCH tap/hold | `NM$nnnnm;`, `NA$n;`; SW31/140/146 | **Missing** | `FR-RX-NOTCH-01`: manual notch (on/off + pitch) + auto-notch per RX. |
| APF — FIL hold (CW) | `AP$mb;` b=30/50/150 Hz; SW144 | **Missing** | `FR-RX-APF-01`: toggle APF + bandwidth (`AP`) in CW. |
| NB/NR levels — NB/NR hold | `NB$nnmf;`, `NR$nnm;`, `NRS$nnm;`; SW142/143 | **Partial** — UI sends blind taps, level path absent, SSNR missing | Impl gap on `FR-RX-04`; add `NRS`. |
| Preamp levels | `PA$nm;` n=0–3 | **Partial** — toggle only, no level rotation/dB | Extend `FR-RX-02` to rotate `PA`. |
| AGC off + AF limiter | `GT$0` + `AL nn` | **Partial** — `AL` absent | Note `AL` in `FR-RX-03`. |
| Audio effects / RX mix | `FX`, `MX` | Missing (low) | Optional `FR-RX-FX-01`. |

## B. TX controls

| Feature | CAT | Status | Gap |
|---|---|---|---|
| PWR — power (XMTR knob) | `PCnnnr;` r=L/H/X; `PO`, `PP` | **Missing impl** — `FR-TX-02` spec'd | Implement `FR-TX-02` incl. QRP/QRO/mW ranges. |
| DLY — VOX/QSK delay | `SDxyzzz;`, `SD/` | **Missing** — only blind `SW134;` | `FR-TX-DLY-01`: full-QSK / per-mode delay (`SD`). |
| MIC gain | `MGxxx;` 0–80 | **Covered** (`FR-AUD-CFG-01`) | — |
| CMP — speech compression | `CPnnn;` 000–030 (SSB) | **Missing** | `FR-TX-CMP-01`: set compression + show CMP bar on TX. |
| WPM | `KSnnn;` | **Covered** (`FR-KEY-01`) | — |
| PITCH — CW sidetone | `CWnn;` 25–95 ×10 Hz | **Missing** | `FR-KEY-02`: CW sidetone/pitch (`CW`). |
| MON — monitor level (hold) | `MLmnnn;` | **Missing** — blind `SW128;` | `FR-TX-MON-01`: per-mode monitor level (`ML`). |
| VOX gain / anti-VOX | `VGmnnn;`, `VInnn;` | **Missing** — only `VX` on/off | Extend `FR-VOX-01` or add `FR-VOX-02`. |
| TX TEST | `TSn;`; `SW132` | **Partial** — hold emulated, no `TS` readback | Use `TS` with RESP. |
| ESSB | `ESnbb;` | **Missing** | `FR-TX-ESSB-01`. |
| TX DATA bandwidth | `DWnn;` | **Missing** (minor) | Fold with ESSB. |
| DVR voice/AF messages | `DA…`, `PB n`; SW19/138/139/137 | **Missing** | `FR-DVR-01` (C): record/save/play/stop DVR. |
| TX metering (RF/SWR/ALC/CMP) | `TMaaabbbcccddd;`, enable `TM1;` | **Partial** — `FR-MTR-03` spec'd, `TM` not parsed, no bars | Impl gap: parse `TM`, render bars. |

## C. Filter / DSP passband

Intro p17: FILTER knob = width (BW) + shift (SHFT), *or* hi-cut (HI) + lo-cut (LO); hold = NORMalize.

| Feature | CAT | Status | Gap |
|---|---|---|---|
| Bandwidth | `BW$nnnn;` ×10 Hz | **Partial** — `FR-MODE-02`; UI is 8-step preset cycle, no continuous/sub-RX | Continuous BW per RX. |
| Shift (SHFT) | `IS$nnnn;` | **Missing** | `FR-FIL-01`: passband shift/center per RX (`IS`). |
| HI/LO-cut | derived from `BW`+`IS` | **Missing** | `FR-FIL-02`: HI/LO editing mapping to `BW`/`IS`. |
| NORM (hold) | `FP~;` / `SW129;` | **Missing** | Include in `FR-FIL-01/02`. |
| Filter presets FL1/2/3 | `FP$n;`, `FP~` | **Missing impl** — `FR-MODE-03`; FIL `SW33` absent | Implement `FR-MODE-03` (consider raising to S). |
| Passband graphic | render `BW`/`IS` | **Missing** | Extend `FR-UI-02` with a per-VFO passband indicator. |

## D. Diversity

Intro p12: tap SUB = sub RX on; **hold SUB = DIVERSITY**. CAT `DVn;`, `DV\` toggle (also toggles sub RX; VFO A band/mode/filter copied to B; RIT on A affects both). Currently only a blind `DIV` tap (`SW152;`), no readback, no sub-RX precondition, no `=OPP TX` antenna.

- Proposed `FR-DIV-01`: enable/disable diversity (`DV`, `DV\` semantics), reflect state, indicate sub-RX mirrors VFO A while active. Surface `=OPP TX` in the RX-ant panel (extend `FR-ANT-01`).

## E. Scan

Intro p12: **hold SCAN** to scan between A/B of the last-recalled memory. CAT: SW-switch only (`SW149;`); scan-in-progress is the `IF` response `s` field.

- Proposed `FR-SCAN-01`: start/stop scan via `SW149;`, display scan-in-progress from the `IF` `s` flag, stop on any tune/PTT.

## F. Front-panel switches — full tap/hold matrix

**Bold** = not currently reachable from our UI.

| Switch (tap / hold) | SW (tap / hold) | Dedicated CAT | Coverage |
|---|---|---|---|
| FREQ ENT / **SCAN** | SW53 / SW149 | — | Partial: entry native; SCAN missing (§E) |
| **RATE / KHZ** | SW73 / SW150 | `VT$` | Missing radio-side rate → `FR-VFO-08` (`VT`). |
| **SUB / DIV** | SW83 / SW152 | `SB` / `DV` | SUB missing; DIV tap-only (§A/§D) |
| LOCK A / LOCK B | SW63 / SW151 | `LK$n;` | Partial: taps wired but blind; `LK` gives readback. |
| **MODE / ALT** | SW43 / SW148 | `MD$`, `MA$` | Partial: **AM/FM missing** from mode row, no alt-mode, no sub-RX mode. |
| **B SET** | SW44 | `BS` | Missing sub-RX context (our `$` variants already exist in the CAT layer). |
| A/B, A→B, B→A | SW41/72/147 | `AB0–5;` | Covered (Fn→VFO ops); not near the VFOs as on radio. |
| SPLIT | SW145 | `FTn;` | Covered (also now click-a-frame TX-VFO select). |
| **REV (press/release)** | SW160/161 | — | Missing momentary. `FR-VFO-REV-01`. |
| RIT / XIT / CLR | SW54/74/64 | `RT`/`XT`/`RC` | Covered; **offset adjust missing** (`RO$`/`RU`/`RD`) — impl gap on `FR-VFO-05`. |
| PRE / **ATTN-hold** | SW61 / SW141 | `PA$`, `RA$` (+3 dB steps) | Partial: toggles only, no step-adjust. |
| NB / **LEVEL-hold** | SW32 / SW142 | `NB$` | Partial (§A). |
| NR / **ADJ-hold** | SW62 / SW143 | `NR$`/`NRS$` | Partial (§A). |
| **NTCH / MANUAL/AUTO** | SW31 / SW140,146 | `NM$`, `NA$` | Missing (§A). |
| **FIL / APF** | SW33 / SW144 | `FP$`, `AP$` | Missing (§A/§C). |
| SPOT / **AUTO-SPOT hold** | SW42 / — | `SPn;` 0–3 | Partial: tap-only. Add autospot (`SP0–3`). |
| **STORE / RCL / BANK** + M1–M4 | SW20/34/137 + SW17/51/18/52 | `MC` pending | Partial: M1–M4/PF1–PF4 covered; STORE/RCL/BANK missing. |
| TUNE·TUNE LP / ATU TUNE·ATU / ANT·REM ANT / XMIT·TEST / VOX·QSK / RX ANT·SUB ANT | (implemented) | `AT`,`AN`,`TS`,`VX`,`SD` | Covered as blind taps; stateful `AT`/`TS`/`SD` are the gaps above. |
| XMTR/FILTER/RF-SQL knob taps; MON/NORM/BAL holds | SW80–82; SW128–130 | `ML`,`FP~`,`BL` | MON present; NORM/BAL missing — superseded by direct controls. |
| POWER | — | `PS0/8` | Covered (`FR-PWR-01`). |

Umbrella: `FR-SW-02` — expose every front-panel tap/hold either through its
stateful CAT command (preferred) or `SW` emulation, so nothing is unreachable.

## G. Display / panadapter / status

| Feature | CAT | Status | Gap |
|---|---|---|---|
| Fixed-tune / freeze | `#FXT`, `#FRZ` | Partial — `set_pan_fixed` exists, no `DispMsg::Fixed` | Wire `#FXT` into DISPLAY. |
| WF colour range, display mode | `#WBS`, `#DSM` | **Missing** — `FR-PAN-CTL-02` spec'd | Implement or descope. |
| Pan NB | `#NB`/`#NBL` | Partial — encoders exist, not in DISPLAY | Wire in. |
| Per-pan targeting (A/&/B) | `#` per-pan | **Missing** — `apply_disp` never targets A vs B | Extend `FR-PAN-CTL-01`. |
| Status area (date/time, ID, TX V/I/W/SWR, dBV) | `ID`, `DB$`, `TM` | **Missing** | `FR-UI-STATUS-01` (C). |
| Mini-pan | `#MPRUN` / 0x03 frames | `FR-UI-14` (C) unimpl | — |
| **Text decode/encode** (CW/PSK/FSK RX text + kbd TX) | `TD$mtl;`, `TB$;`, `KY` | **Missing** — headline K4 feature, only outbound `KY` (`FR-TX-MSG-01`) | `FR-TXT-01`: per-RX decode (`TD$`), poll/show (`TB$`), send (`KY`). |

## H. Other

- **VFO link/band-independence/offset** (`LN`/`BI`/`VO$`): `FR-VFO-09` batch (C).
- **FM support**: `RP` (repeater), `PL$` (CTCSS); FM not selectable in mode row → `FR-FM-01` (C).
- **Antenna config/names**: `ACT` (TX-ant subset = "ANT CFG"), `ACN` (names) → show real antenna names like "2:YAGI"; extend `FR-ANT-01`.
- **ATU mode** `AT` (auto/bypass): the ATU hold `SW158` is blind.
- **Transmit-icon row** (SPLIT/VOX/ANT/ATU/QSK between the VFOs): depends on the missing state readbacks.
- **`IF` parsing completeness**: `IF` carries RIT/XIT offset+flags, TX state, scan flag, split, data sub-mode — add a test asserting all fields feed `RadioState` (scan field currently unused).

## Summary counts

- **Missing entirely (SRS + UI)**: squelch, QSK/VOX delay, compression, monitor
  level, CW pitch, passband shift & hi/lo-cut, notch, APF, diversity, scan,
  sub-RX B SET/REV/RATE-KHZ, ESSB, DVR, text decode, status display, per-pan
  targeting — ~17 features.
- **Spec'd but unimplemented**: `FR-TX-02` (PC), `FR-MODE-03` (FP), `FR-RX-06`
  (SB/BL), `FR-MTR-03` (TM parse), `FR-PAN-CTL-02` (#WBS/#DSM), the offset half
  of `FR-VFO-05`.
- **Implemented but UI-orphaned**: `set_af_gain`, `set_rf_gain`, `set_pan_fixed`,
  `set_pan_nb`, `set_pan_nb_level`, `set_mode_sub` (in `crates/k4-protocol/src/cat.rs`).
