---
title: "Feature Completeness Review & Action Plan"
status: Draft
version: "0.2"
updated: 2026-07-07
authors:
  - Fable (agent, commissioned by DC0SK)
---

# Feature Completeness Review & Action Plan

Supersedes the original v0.1 gap analysis (most of whose "top gaps" are now
closed — v0.50–0.67 in [test-strategy.md](../test/test-strategy.md)). Reconstructed
from the code, the test-strategy change log, the SRS, and D12 / Intro C5.
Analysis input for future sessions — items are **candidates**, not accepted
requirements.

## Already done — do NOT redo

Connection/session (TCP + TLS-PSK + serial, PING/PONG, reconnect, ~50-GET
connect seed, **periodic resync** ~3 s locals / ~8 s full burst, peer cache);
VFO/band/mode (FA/FB, direct MHz entry for A, band up/down/direct/stack/XVTR,
split + click-a-pane TX-VFO select, AB copy/swap, RIT/XIT on/off/clear, mode
LSB/USB/CW/DATA); **RX main+sub** (`$`-retargeted): AF/RF/SQL sliders, ATT/PRE/NB/
NR/AGC toggles with read-back, filter presets FL1–3 + normalize, **shift**, BW
cycle, **manual/auto notch + APF**, **sub-RX + diversity**, sub-RX read-back, RX/TX
EQ; **TX**: PTT/arm/e-stop, CW paddle stream, **power**/compression/CW-pitch/QSK
delay, keyer, mic, line in/out, VOX on/off, text send (KY), switch grid with live
state; **metering**: S-meter + in-panadapter **S-meter + TX RF/ALC/SWR/CMP bars**
(`TM`); **text decode** (`TD`/`TB`); **scan** (`SW149` + `IF s`); antennas +
**subset cycling** (`ACM`/`ACS`); display (spectrum/waterfall, DISPLAY screen,
layout adopted from `#DPM`); MENU list, quick memories, remote power, **config
export/import w/ SHA-256**, diagnostics, audio, packaging/CI.

## Open items (prioritized)

### A. VFO / tuning — the biggest remaining hole
| # | Item | CAT | Prio |
|---|---|---|---|
| A1 | VFO up/down **steps + mouse-wheel** tuning; VFO-B direct entry (today: type-MHz sets A only) | `UP/DN(B)`, `VT$` rate, or client `FA`/`FB` math | **Must** |
| A2 | **Click-to-tune / scroll on the spectrum** (QSY) — pane click only picks TX VFO | `FA`/`FB` from cursor x→Hz | **Must** |
| A3 | **RIT/XIT offset** adjust + display (only on/off/clear exist; `IF` `+yyyy` field unparsed) | `RO$snnnn;`, `RU$/RD$`, `IF` offset | **Must** |
| A4 | VFO **lock** read-back (LOCK A/B are blind `SW63/151`) | `LK$` | Should |
| A5 | REV momentary, B SET, VFO link/band-indep/offset | `SW160/161`, `BS`, `LN`/`BI`/`VO$` | Could |

### B. Mode
| # | Item | CAT | Prio |
|---|---|---|---|
| B1 | **AM/FM (+ CW-R/DATA-R) not selectable** — mode row is LSB/USB/CW/DATA only (state parses all 8) | `MD4/5/7/9`, `MA$` | **Must** |
| B2 | FM extras: repeater offset, CTCSS | `RP`, `PL$` | Should |
| B3 | DATA sub-mode select (feeds text decode) | `DT$` (FR-MODE-04) | Should |

### C. Filter / DSP
| # | Item | CAT | Prio |
|---|---|---|---|
| C1 | **HI/LO-cut editing** — derived from BW+IS (see plan below) | `BW`+`IS` (no dedicated cmd) | Should |
| C2 | **Passband graphic** on the spectrum edge | render-only | Should |
| C3 | **NB/NR level sliders + SSNR** (toggles exist; levels implicit) | `NB$nnmf`, `NR$nnm`, `NRS$nnm` | Should |
| C4 | Preamp **level** rotation 0–3 (UI is on/off; `PA` level digit discarded) | `PA$nm` | Should |
| C5 | Attenuator 3 dB step; AF limiter (AGC-off) | `RA$+/-`, `AL` | Could |
| C6 | Sub balance, RX mix/effects | `BL`, `MX`, `FX` | Could |

### D. TX
| # | Item | CAT | Prio |
|---|---|---|---|
| D1 | **Monitor level** (MON, unreachable) | `MLmnnn` | **Must** |
| D2 | **VOX gain / anti-VOX** (only `VX` on/off) | `VG`, `VI` | Should |
| D3 | **Autospot** (SPOT is blind `SW42`) | `SPn` 0–3 | Should |
| D4 | TX power **low/mW ranges** (`set_tx_power` emits `PC…H` only) | `PCnnnr` L/H/X | Should |
| D5 | ESSB + TX DATA bandwidth | `ES`, `DW` | Could |
| D6 | **DVR voice-message** play/record (8 slots) | `PBn`, `DA…` | Should |
| D7 | TX TEST read-back; TX inhibit indicator | `TS`, `IN` | Could |

### E. Display / panadapter
| # | Item | CAT | Prio |
|---|---|---|---|
| E1 | **Per-receiver spectrum in dual view** — worker ignores `PanFrame.receiver`; both panes draw the same trace (bug) | packet receiver byte | **Must** |
| E2 | **Per-pan `#` targeting** (A/B) — display cmds apply globally | `#REF$`, `#NB$`, `#WFC$`, `#VFA/B` | Should |
| E3 | **Mini-pan** (0x03 frames dropped) | `#MP$`, 0x03 stream (FR-UI-14) | Should |
| E4 | `#FXT` fixed-tune + pan-NB wiring into DISPLAY | `#FXT`, `#NB`, `#NBL` | Should |
| E5 | Waterfall colour range / display mode | `#WBS`, `#DSM` (FR-PAN-CTL-02) | Could |
| E6 | **Status readout** (UTC, supply V/I, dBV, client count, ID) | `UT`, `DB$`, `CC`, `ID`, `SI` | Could |

### F. Memory / scan
- F1 **Full memory channels** — only quick-mems M1–M4; `MC`/`MB` still "[Pending]" in D12, so STORE/RCL/BANK stay `SW`-emulation without read-back (revisit when `MC` lands). *Could.*

### G. Config / antennas / other
| # | Item | CAT | Prio |
|---|---|---|---|
| G1 | **Antenna names** ("2:YAGI") + TX-antenna subset; verify `ACM`/`ACS` mapping live | `ACN`, `ACT` | Should |
| G2 | **Full-menu backup** — export replays ~30 tracked settings, not the 89 MENU items | `MEDF`/`ME` sweep (FR-CFG-07) | Should |
| G3 | Menu value display/edit (MENU only opens items on the radio) | `MEDF`/`ME` | Could |
| G4 | Station ID + remote client count | `ID`, `CC` | Could |
| G5 | Audio encode/latency knobs (`EM3`/`SL2` hard-coded) | `EM`, `SL` | Should |
| G6 | `CAT ?;` error mapping/surfacing | `<cmd>?;` (FR-CAT-03) | Could |

## HI/LO-cut plan (item C1, detail)

**No dedicated CAT command** — the K4 FILTER knob's HI/LO view is an alternate
presentation of BW + IS: `LO = IS − BW/2`, `HI = IS + BW/2` (inverse
`BW = HI − LO`, `IS = (HI + LO)/2`).

- **Protocol:** `passband_edges(bw, center) → (lo, hi)` and
  `set_passband_edges_hz(lo, hi) → ("BW…;", "IS…;")` reusing `set_bandwidth_hz`
  /`set_shift_hz`; add `RadioState::passband_edges(sub)`. No new wire commands,
  no seed changes (BW/IS + `$` already seeded).
- **UI:** a `BW/SHFT ⇄ HI/LO` toggle that swaps the SHIFT slider for **LO** + **HI**
  sliders; dragging either recomputes BW+IS and sends both to the active VFO.
- **Requirement:** `FR-FIL-02`; cat + state tests (round-trip, midpoint
  rounding, 50 Hz min-width clamp).
- **Verify live:** per-mode BW/IS clamps, command ordering, CW-mode semantics
  (passband centers on sidetone — HI/LO may be read-only in CW).

## Suggested execution order

1. **Operating feel (Must):** A1 step/wheel tuning + `VT$`, A2 click-to-QSY,
   A3 RIT/XIT offset, B1 AM/FM/CW-R/DATA-R mode row.
2. **Knob completion:** D1 `ML`, D2 `VG`/`VI`, D3 `SP` autospot, D4 PC ranges,
   C3 NB/NR levels + `NRS`, C4 preamp level.
3. **Display:** E1 per-receiver spectra (bug), E4 `#FXT`/pan-NB, E2 per-pan
   targeting, E3 mini-pan.
4. **Polish/config:** C1/C2 HI-LO cut + passband graphic, G1 antenna names/ACT +
   live `ACM`/`ACS` verification, G2 full-menu backup, D6 DVR, B2/B3 FM/DATA
   sub-modes, G5 EM/SL settings.

Accuracy caveats to clear during a hardware-in-the-loop pass: the `ACM`/`ACS`
antenna-mask a–g→`AR$` mapping and the `PC` low-range parse.
