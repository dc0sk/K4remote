---
title: "Mode-Adaptive UI Concept — MAIN RX + TRANSMIT regroup"
status: Draft
version: "0.1"
updated: 2026-07-08
authors:
  - Simon Keimer (DC0SK)
  - Fable (agent, commissioned by DC0SK)
---

# Mode-Adaptive UI Concept

Design concept + action plan for making the K4 Remote UI **operating-mode
aware** and regrouping the **MAIN RX** and **TRANSMIT** frames by function.
Extends `ui-design.md` — this is the elaboration of its "mode-dependent
context" idiom, which the K4 itself uses: *"Some configuration controls change
based on the operating mode. In this example, the APF and TEXT DECODE functions
pertain to CW"* (Intro rev C5). The K4's XMTR multifunction knob does the same
on the TX side: its two per-mode parameters are **WPM + PITCH in CW** and
**MIC + CMP in voice**, and the CMP meter is *only shown in voice modes*
(already mirrored by `show_cmp` in `vfo_panel`).

## 0. Mode classes

Five presentation classes, derived from the mode string (`ui.mode_a`/`mode_b`):

| Class | Modes | Character |
|---|---|---|
| **CW** | CW, CW-R | keyed carrier; pitch-centred narrow RX |
| **Voice** | LSB, USB | mic audio; speech processing |
| **Data** | DATA, DATA-R (FSK, FSK-D when added) | soundcard/AFSK; decode |
| **AM** | AM | voice minus SSB-isms; wide filter |
| **FM** | FM | channelised; squelch/repeater; fixed filters |

Refines the existing `tx_mode_class()` (returns `'C'/'D'/'V'` for the `SD`
command) — keep that for CAT, add a UI-side `ModeClass` that separates AM and
FM out of `'V'`.

**Which mode drives what:** the **MAIN RX frame** adapts to the *active RX
VFO's* mode (`mode_b` when the frame reads "RX B" — today `fm_panel` keys off
`mode_a` only, a latent bug to fix in passing). The **TRANSMIT frame** adapts
to the *TX VFO's* mode (B under split, else A).

## 1. Per-mode control matrix

Legend: **S** = show · **D** = de-emphasise (visible, dimmed, still clickable) ·
**H** = hide (lives in the mode strip) · **A** = always show (safety/core).

### MAIN RX frame

| Control | CW/CW-R | LSB/USB | DATA/-R | AM | FM | Rationale |
|---|---|---|---|---|---|---|
| Mode buttons, BAND ± | A | A | A | A | A | Mode switching must never depend on mode. |
| BW chip | S | S | S | S | D | K4 FM filters are fixed channel widths. |
| FL1/2/3 + NORMALIZE | S | S | S | S | D | Per-mode presets — keep everywhere, dim in FM. |
| SHIFT ⇄ HI/LO + slider | S | S | S | D | H | Passband shaping meaningless on FM's fixed filters. |
| ATT / PRE chips | S | S | S | S | S | RF front-end — mode-independent. |
| AGC chip | S | S | S | S | D | FM limiting makes AGC mostly moot. |
| NB / NR chips + levels | S | S | S | S | S | Noise mitigation applies everywhere. |
| NOTCH + pitch slider | D | S | S | S | H | In CW it would notch the *desired* tone. |
| AUTO NCH chip | H | S | D | S | H | Auto-notch eats a CW signal; pointless in FM. |
| APF + APF-BW | **S (CW only)** | H | H | H | H | K4: APF "applies only in CW mode". The canonical hide case. |
| SQL slider | D | D | D | D | **S** | FM's primary control — promote in FM, dim elsewhere. |
| AF / RF | A | A | A | A | A | Core listening controls. |
| SUB / DIV | S | S | S | S | S | Receiver topology, not mode. |
| SCAN | S | S | S | S | S | Not worth moving. |
| FM sub-panel (RPT/PL/CTCSS) | H | H | H | H | **S** | Repeater plumbing only exists in FM. |
| **(new)** SPOT | **S (CW)** | H | H | H | H | CW ops zero-beat constantly; today buried on Fn→SWITCHES. |
| **(new)** TEXT DECODE shortcut | S | H | S | H | H | Decode pertains to CW/data. |

### TRANSMIT frame

| Control | CW/CW-R | LSB/USB | DATA/-R | AM | FM | Rationale |
|---|---|---|---|---|---|---|
| ARM / PTT / EMERGENCY STOP | A | A | A | A | A | Safety chain — pinned, identical pixels. |
| PWR H/L/X + slider | A | A | A | A | A | Power awareness is a safety matter. |
| TUNE / TUNE LP | S | S | S | S | S | Carrier tune is mode-independent. |
| ATU TUNE / ATU | S | S | S | S | S | Antenna matching — station-level. |
| ANT / REM ANT, RX ANT / SUB ANT | S | S | S | S | S | Station-level routing. |
| XMIT / TEST | S | S | S | S | S | TEST (no-RF) useful in all modes. |
| VOX | H | **S** | S | S | S | Audio-triggered TX — a CW op keys with the paddle. |
| QSK | **S** | H | H | H | H | Full break-in is *only* defined for CW. |
| QSK FULL/DELAY + slider (today TX→KEYER) | **S** | H | H | H | H | Promote from the buried KEYER tab into the CW strip. |
| AUTOSPOT | **S** | H | H | H | H | `SP3` needs a CW tone to lock to. |
| MON slider | S | S | S | S | S | Sidetone (CW) / TX monitor (voice) — universal. |
| VOX G / A-VOX | H | **S** | D | S | S | Anti-VOX guards a *mic* — meaningless in CW. |
| DVR 1–8 + STOP | H | **S** | H | S | S | DVR plays recorded *voice*; CW/DATA use `KY` text. |
| **(new)** KEYER SPEED −/+ | **S** | H | H | H | H | XMTR knob's CW per-mode parameter. |
| **(new)** CW PITCH quick slider | **S** | H | H | H | H | The other CW per-mode parameter. |
| **(new)** CMP quick slider | H | **S** | H | S | D | Voice per-mode parameter; K4 shows CMP meter in voice only. |
| **(new)** MIC GAIN quick stepper | H | **S** | H | S | S | Other voice per-mode parameter. |
| **(new)** SEND-TEXT shortcut | S | H | **S** | H | H | `KY` send is CW/data's "DVR". |
| TX tabs EQ/KEYER/MIC/LINE/ANT/TEXT | S | S | S | S | S | Config screens stay complete — default tab follows mode, irrelevant tabs dimmed. |

## 2. Interaction principle

1. **Three visibility tiers.** *Always (A)*: identical widget/position every
   mode (safety chain, power, freq, S-meter, AF/RF, mode buttons). *De-emphasise
   (D)*: keeps its exact slot, `Inactive`/`Track` colours (a new
   `BtnKind::Dim`), still clickable — dimming, never removal, is the rule for
   anything in a *shared* row (removal reflows). *Hide (H)*: only inside the
   mode strip, where the whole row's content swaps atomically.
2. **One fixed-height "mode strip" per frame, always present.** Reuse the
   FR-UI-19 fixed-slot precedent (the panadapter slot is fixed to `SCREEN_H`
   "so the frame doesn't resize when swapping"). MAIN RX gets a permanent 4th
   row of fixed height whose content is chosen by mode class; TRANSMIT's mode
   row likewise. This **removes the existing layout jump** (today `push_maybe(FM
   → fm_panel)` grows the RX frame on entering FM).
3. **Height budget.** Window min 1320×964. The RX frame permanently gains one
   strip row (~34 px = today's FM-only row); the TX mode strip *replaces* the
   MON+DVR rows (net ±0). Verify at 1320×964 before merging each phase.
4. **No repositioning of survivors.** A control shown in two modes keeps its
   x-position if the surrounding rows are shared; strip content is left-aligned
   per class and swapped wholesale, so no partial-reflow jitter.
5. **Mode changes are radio events.** Adaptation keys off the *confirmed*
   `mode_a/mode_b` snapshot, so a rejected mode change can't desync the UI.
6. **Never adapt away discoverability.** Full config stays reachable in every
   mode via the softkey screens; the strips are shortcuts, the screens the
   catalogue. (A Settings "SHOW ALL" escape hatch is offered in Phase 5.)

## 3. Regrouped MAIN RX frame

Grouping: **MODE/TUNE** · **SIGNAL + NOISE + RX config** · **FILTER/DSP** ·
**MODE STRIP** (fixed height).

```
┌ RX A ──────────────────────────────────────────────────────────────────────────────────┐
│ MODE  [LSB][USB][CW][CW-R][DATA][DATA-R][AM][FM]  ·  [BAND-][BAND+]  ·  [SCAN]           │
│ SIG   [ATT][PRE][AGC]  AF ======  RF ======   NOISE [NB]=lvl= [NR]=lvl=   RX [SUB][DIV]  │
│ FILT  [BW 2.7k][FL1][FL2][FL3][NORM]  [SHFT/HILO] ======   [NOTCH]=pitch= [AUTO NCH]     │
│ -- mode strip (fixed h) ---------------------------------------------------------------- │
│  CW:    [APF][APF 50]  ·  [SPOT][AUTOSPOT]  ·  [TEXT DECODE]                             │
│  Voice: SQL ====== (dim)                          (auto-notch emphasised above)         │
│  Data:  [TEXT DECODE]  ·  SQL === (dim)     (sub-mode selector when DT lands — §5)       │
│  AM:    SQL === (dim)                                                                    │
│  FM:    SQL ======  ·  RPT [S][+][-] +600 kHz  ·  [PL On] [-] 88.5 Hz [+]                │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

Changes: (a) ATT/PRE/AGC join the gain sliders they modulate; (b) NB/NR chips
merge with their level sliders (today chip is row 1, level row 3); (c) NOTCH
merges with its pitch slider and the misleading bare "PITCH" label becomes
"NOTCH" (it is `notch_pitch`, not CW pitch); (d) APF/SPOT/decode/FM move into
the strip; (e) SQL lives in the strip so row 2 holds only universal gain.

## 4. Regrouped TRANSMIT frame

Grouping: **TX CONTROL/SAFETY + POWER** (pinned) · **STATION** (RF-path
switches) · **MODE STRIP** (fixed height).

```
┌ TRANSMIT ──────────────────────────────────────────────────────────────────────────────┐
│ [ARM TX] [PTT] [EMERGENCY STOP]        PWR [H][L][X] ====== 10.0 W      [XMIT | TEST]    │
│ STN  [TUNE|TUNE LP] · [ATU TUNE|ATU IN] · [ANT 1|REM ANT] · [RX A 1|SUB A 1]   MON === 25│
│ -- mode strip (fixed h) ---------------------------------------------------------------- │
│  CW:    [QSK: FULL] DLY === 300 ms · SPEED [-] 24 WPM [+] · PITCH === 600 Hz · [AUTOSPOT]│
│  Voice: [VOX] VOX G === · A-VOX === · CMP === 12 · MIC [-] 40 [+] · DVR [1..8][STOP]     │
│  Data:  [VOX] VOX G === · [LINE: USB] LVL [-][+] · [msg text…            ][SEND]         │
│  AM:    [VOX] VOX G === · A-VOX === · CMP === · MIC [-][+] · DVR [1..8][STOP]            │
│  FM:    [VOX] VOX G === · A-VOX === · MIC [-][+] · DVR [1..8][STOP]                      │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

Changes: (a) XMIT/TEST joins row 1 (TX control, not antenna plumbing); (b) the
VOX/QSK dual-switch dissolves — the strip shows the one that exists in the
current mode; (c) MON stays universal; VOX G/A-VOX/DVR move to the voice strip;
(d) the CW strip pulls QSK+delay/keyer speed/pitch out of the buried TX→KEYER
tab; (e) the data strip surfaces `KY` send inline. Row count unchanged.

## 5. Phased action plan (low risk → higher, each shippable)

**Phase 0 — pure visibility model (no visual change).** In `app/src/ui.rs`
(the pure, unit-testable layer):
```rust
pub enum ModeClass { Cw, Voice, Data, Am, Fm }
impl ModeClass { pub fn from_mode(m: &str) -> Option<Self> { /* "CW"|"CW-R" => Cw … */ } }
pub enum Vis { Show, Dim, Hide }
pub enum RxCtl { Apf, AutoNotch, ManualNotch, Squelch, ShiftHiLo, Bw, FilterPresets, Agc, /* … */ }
pub enum TxCtl { Qsk, Vox, AntiVox, Dvr, Autospot, Cmp, MicGain, KeyerSpeed, SendText, /* … */ }
pub fn rx_ctl_vis(c: RxCtl, m: ModeClass) -> Vis { /* the §1 matrix */ }
pub fn tx_ctl_vis(c: TxCtl, m: ModeClass) -> Vis { /* … */ }
```
Table-driven unit tests (one per matrix row). Add `active_mode()`/`tx_mode()`
in `main.rs` (fixing the `fm_panel`-uses-`mode_a` bug).

**Phase 1 — de-emphasis only (zero layout risk).** Map `Vis::Dim` to a new
`BtnKind::Dim` in the chips/gain rows and `tx_switch_grid`. Nothing moves or
hides. Ships alone; instantly gives the "leaner" feel.

**Phase 2 — MAIN RX mode strip (kills the FM layout jump).** Extract the RX
controls block into `rx_controls_panel`; add `rx_mode_strip(class)` of fixed
height; move `fm_panel` content into its FM arm; add the CW arm (APF/SPOT/
AUTOSPOT) + decode toggle; replace `push_maybe(FM…)` with an unconditional
`push(strip)`. Verify height at 964 px.

**Phase 3 — MAIN RX regroup (rows 1–3 per §3).** Re-order existing widgets;
merge NB/NR/NOTCH chips with their sliders; retitle the notch label. No new
messages. Prototype row-3 width at 1320 px in both HI/LO and SHIFT states.

**Phase 4 — TRANSMIT regroup + TX mode strip (§4).** Rework `tx_switch_grid`
into `station_row` + `tx_mode_strip(class)`. Split the VOX/QSK dual cell per
mode. Move QSK/delay, keyer speed, CW pitch, CMP, mic gain into the strips —
**all messages already exist**; layout only. TX→KEYER/MIC tabs keep their
copies (shared state).

**Phase 5 — mode-aware defaults + polish.** (a) `tx_screen` default tab follows
`ModeClass`; (b) DATA sub-mode selector — **needs new CAT wiring** (`DT` setter/
parse/seed, no `DT` in `cat.rs` yet); (c) optional Settings "SHOW ALL" escape
hatch forcing `Vis::Show` everywhere.

**Live-radio verification list** (beyond `k4-sim`): `MD$` read-back driving the
RX strip when RX B is active; `SD` per-mode-class delay banks (CW↔SSB re-reports
the class's own delay); `SP3` outside CW (expect no-op); `NA`/`AP` acceptance
per mode (if the K4 NAKs, Dim must not imply functionality); FM `BW`/`IS` on
fixed filters; DVR playback in CW/DATA (expect no-op).

## Notable bugs surfaced by this review

- **Layout-jump bug**: `fm_panel` is appended via `push_maybe`, so entering FM
  grows the MAIN RX frame and shifts everything below. The always-present
  fixed-height mode strip fixes this.
- **Adaptation keyed to the wrong VFO**: `fm_panel` and `tx_mode_class()` read
  `mode_a` even when the frame is RX B / TX is on B under split.
- **Mislabelled slider**: the gain-row "PITCH" is `notch_pitch` (manual-notch
  centre), not CW sidetone pitch.
- Almost everything is **layout-only** to implement — QSK, CW pitch, keyer
  speed, CMP, mic gain, decode, SPOT all have existing messages/CAT setters.
  Only the DATA sub-mode (`DT`) needs new protocol work.
