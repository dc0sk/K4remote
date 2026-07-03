---
title: "K4 On-Screen Configuration Screens — Spec & Action List"
status: Draft
version: "0.12"
updated: 2026-07-02
authors:
  - Simon Keimer (DC0SK)
---

# K4 On-Screen Configuration Screens — Spec & Action List

Specification for the K4's on-screen configuration screens that our remote panel
reproduces **in place of the spectrum frame** when a primary softkey is active
(`FR-UI-19`). Each screen is one of the radio's *additional* functions — it must
**not** duplicate controls already present in the main UI (VFO band, mode/filter
box, TX safety panel, connection panel).

**Provenance.** Extracted from *Intro to the Elecraft K4, rev C5*
(`docs/references/external/`, `R-EXT-02`) — Elecraft's own drawn renders of the
7″ touchscreen. These are the radio's own UI conventions (interoperability
faithfulness, like the protocol); reproduced clean-room per `CON-09`. Page
numbers below cite that document. CAT commands are confirmed against the manual's
macro examples (p.44) where shown, else flagged for confirmation against the K4
Programmer's Reference / a real radio (`ASM-05`).

**How to read this.** §1 catalogs the screens grouped by the primary that opens
them, each with concise "shall" requirements. §2 lists the reusable UI patterns.
§3 lists the CAT commands the screens need (and which our protocol layer already
has). §4 is the prioritized action list. Screens the K4 opens from a **hardware
switch or gesture** (not a softkey) are listed under §1.8 as context — lower
priority for a v1 remote panel.

---

## 1. Screen catalog (by primary)

### 1.1 MENU — configuration menu list (p.29)

A scrollable list of the radio's configuration parameters; the K4's single
biggest settings surface.

- **SCR-MENU-01** — shall present a scrollable list of menu parameters, each
  showing its name and current value, with one focused entry (≈5 visible).
- **SCR-MENU-02** — shall edit the focused value via `−/+` and a rotary/scroll
  input, showing units.
- **SCR-MENU-03** — shall require unlocking a lock-marked parameter (e.g.
  *Reference Frequency*) before allowing edits.
- **SCR-MENU-04** — shall provide a **NORM** action restoring a parameter's
  default, indicated when the value already equals default.
- **SCR-MENU-05** *(C)* — shall support assigning a menu entry to a programmable
  function key (PF1–PF4).
- **Data source:** the K4 exposes menu items over CAT (`MENU:` addressed items,
  per the macro example `MENU: LCD Brightness`); the concrete per-item command
  set must be enumerated from the Programmer's Reference before wiring.

### 1.2 Fn — special-functions palette (p.36) + its sub-screens

Row of 14 functions on 7 dual-action (tap/hold) buttons:
`F1/F2 · F3/F4 · F5/F6 · F7/F8 · SCRN CAP/MACROS · SW LIST/UPDATE · DX LIST/F14`.

- **SCR-FN-01** — shall present the Fn palette as 7 tap/hold buttons exposing
  F1–F8 (user macros) plus SCRN CAP, MACROS, SW LIST, UPDATE, DX LIST, F14.
- **SCR-FN-02** — shall display user labels on the F-keys and trigger their
  assigned macros.
- **Macro editor (p.44)** — hold MACROS:
  - **SCR-MAC-01** — shall list programmable switches (PF1–PF4, Fn.F1–F8, REM
    ANT, K-Pod) with their label and macro text.
  - **SCR-MAC-02** — shall edit each switch's label (text entry) and macro
    command string (one or more K4 commands, incl. `#`-prefixed display
    commands), per the Programmer's Reference.
  - *Example (from manual):* `Fn.F1: SPLIT+10 → FT1;AB3;UPB1000;`.
- **SW list / Software update (p.11)** — tap SW LIST / hold UPDATE:
  - **SCR-SW-01** *(C)* — shall surface installed software revisions and update
    status; a remote-initiated install is optional and must be guarded.
- **DX prefix list (p.37)** — tap DX LIST:
  - **SCR-DX-01** *(C)* — shall show a scrollable prefix/country list with text
    search and next/previous match navigation.

### 1.3 DISPLAY — panadapter setup (p.23–25)

Configures the panadapter(s). Lower row of 7 dual buttons + an upper row with
target selectors and a parameter adjuster.

- **SCR-DSP-01** — shall let the operator select and adjust panadapter
  parameters: **reference level** (incl. AUTO), **scale**, **span**, **center**,
  **average**, **peak**, **fixed/freeze**, **cursors**, **waterfall height**,
  **waterfall palette**, and **panadapter NB** (with AUTO slaved to the RX NB).
- **SCR-DSP-02** — shall select the adjustment **target**: monitor (LCD / EXT /
  both) and panadapter (A / B / both).
- **SCR-DSP-03** — shall switch panadapter mode among single-A, single-B, dual
  A+B (this is our existing `ViewMode`, `FR-UI-08`; the DISPLAY screen is the K4's
  home for it — reuse, don't duplicate).
- **SCR-DSP-04** — shall adjust the selected parameter via `−/+` and rotary,
  showing the current value (e.g. `REF -130`), and support amplitude units of
  S-units or dBm.
- **CAT:** the `#`-display family — `#DPM` (pan mode = our `ViewMode`), `#SPN`
  span, `#REF` reference, `#SCL`, `#AVG`, `#PKM`, `#FXT/#FRZ`, `#WFC/#WFH`,
  `#NB/#NBL` (§3.2). Confirmed from D12 + QK4; add to `k4-protocol`.

### 1.4 BAND — band selection & stacking (p.13–14)

Two-row band button group; band-stacking; general coverage; memories; transverter
sub-group.

- **SCR-BAND-01** — shall present one-tap buttons for HF/6 m bands
  (1.8, 3.5, 5, 7, 10, 14, 18, 21, 24, 28, 50) with the active band highlighted.
- **SCR-BAND-02** — shall support **band-stacking**: repeated taps on the current
  band cycle its recently-used frequency/mode registers.
- **SCR-BAND-03** — shall provide **GEN** (general coverage) and **MEM** (recent
  frequency memories) entries.
- **SCR-BAND-04** *(C)* — shall provide an **XVTR** sub-group of 12 transverter
  bands with return to HF.
- **CAT:** direct band select is **`BN$nn;`** (00=160 m…10=6 m; §3.2), stacking
  via `BN$^;`, transverter via `XV`. Confirmed from D12 + QK4.

### 1.5 MAIN RX / SUB RX — receiver configuration (p.19, 21, 32–33, 43)

A config row (blue for main, green for sub, mode-adaptive) opening several
sub-screens:
`ANT CFG · RX EQ · LINE OUT/VFO LNK · AGC · AFX ON/DELAY · APF BW · TEXT DECODE`.

- **SCR-RX-01** — shall present the per-receiver config row, color-coded and
  mode-adaptive (APF/TEXT DECODE in CW).
- **RX graphic equalizer (p.21)** — RX EQ:
  - **SCR-EQ-01** — shall present an **8-band graphic EQ** at
    **100 / 200 / 400 / 800 / 1200 / 1600 / 2400 / 3200 Hz**, each with a slider,
    `−/+` steppers, and a numeric dB readout (±16 dB travel).
  - **SCR-EQ-02** — shall let a band be selected (Hz button or slider) for
    rotary/scroll adjustment.
  - **SCR-EQ-03** — shall provide **FLAT** (all bands → 0 dB) and a dismiss.
  - **SCR-EQ-04** — shall apply per receiver (main via MAIN RX, sub via SUB RX)
    and share the layout with TX EQ.
- **RX antenna config (p.32–33)** — ANT CFG:
  - **SCR-ANT-01** *(C)* — shall select the RX antenna (ANT1–3, RX1, RX2, =TX
    ANT, =OPP TX) with user-assigned names.
  - **SCR-ANT-02** *(C)* — shall configure the RX-ANT switch behaviour: *display
    all* vs *use subset* (per-antenna checkboxes).
- **LINE OUT (p.43)** — LINE OUT:
  - **SCR-LO-01** *(C)* — shall adjust left/right line-out levels with a
    RIGHT=LEFT gang mode.
- **TEXT DECODE (p.27–28)** *(C/W2)* — shall show decoded RX text (CW/PSK/FSK) and
  a TX text buffer; dual windows for main+sub.

### 1.6 TX — transmit configuration (p.40–42)

TX config row:
`ANT CFG · TX EQ · LINE IN · MIC INP/MIC CFG · VOX GN/ANTIVOX · PDL NOR/IAMB A · WEIGHT`.

- **SCR-TX-01** — shall present the TX config row (orange), tap/hold-aware.
- **TX graphic equalizer** — TX EQ: same 8-band spec as **SCR-EQ-01..03**,
  applied to the transmit audio.
- **Keyer (p.40)** — WEIGHT / PDL / IAMB:
  - **SCR-KEY-01** — shall adjust CW **keying weight** (value, `−/+`, rotary).
  - **SCR-KEY-02** — shall toggle **paddle normal/reverse** and **iambic A/B**.
- **Mic config (p.41)** — MIC INP / MIC CFG:
  - **SCR-MIC-01** *(C)* — shall select the TX audio input (FRONT / REAR / LINE
    IN / FRONT+LINE / REAR+LINE).
  - **SCR-MIC-02** *(C)* — shall configure the mic: bias on/off, preamp on/off,
    and (front mic) button set (none / PTT / PTT+UP·DN), plus mic-gain with ALC
    target visible during TX.
- **Line in (p.42)** — LINE IN:
  - **SCR-LI-01** *(C)* — shall select the TX line-in source (soundcard / jack)
    and set its level.
- **TX metering (p.22)** — display-only: RF / ALC / SWR / (voice) CMP bars,
  positioned under the transmit VFO. *(Belongs in the VFO band during TX, not the
  screen — noted for `FR-UI-03`.)*

### 1.7 Presentation used across the screens

The K4 renders these three overlay shapes; our spectrum-frame slot maps to shape
(a)/(b):

- (a) **button/adjuster row** above the softkeys (BAND, MODE, most config rows);
- (b) **mid-screen pane** with a side control column (manual, DX list, memories,
  macro editor, MENU list);
- (c) **full-screen modal** (software update) — for us, a modal/overlay.

### 1.8 Hardware/gesture-invoked screens (context, lower priority)

Opened by a K4 hardware switch or a screen gesture, not one of the 7 softkeys —
so they are **not** part of the spectrum-frame swap, but our remote panel needs an
equivalent affordance eventually:

- **MODE group (p.15–16)** — mode select (already in our controls box).
- **B SET (p.16)** — retargets shared controls to VFO B.
- **Mini-pan (p.26)** — S-meter-invoked fine-tune aid (`FR-UI-14`, already `C`).
- **SPOT / auto-spot (p.38)** — CW tuning aid.
- **Frequency memories (p.39)** — STORE/RCL list (freq+mode per VFO, FM offset/PL).
- **Status display settings (p.34)** — date/time / ID / TX params / sig level.
- **On-screen keyboard (p.35)** — shared text-entry service (the remote panel can
  use the native OS keyboard, but must preserve CW/data prosign + EOT/EOL entry).
- **Help / built-in manual (p.10)** — hyperlinked manual + context help.

---

## 2. Reusable UI components (build once, use everywhere)

1. **Parameter adjuster** — `label · [ON] · value+units · −/+ · (rotary) · dismiss`
   (KEYING WEIGHT, LINE IN/OUT, MANUAL NOTCH, MENU value edit). One component.
2. **Dual-action button** — distinct **tap** (top label) and **hold** (bottom
   label) actions; used pervasively. First-class long-press support needed.
3. **Scrollable list** with knob/scroll + arrow navigation and optional search
   (MENU, DX list, memories, macro editor).
4. **Graphic equalizer** — 8 vertical sliders + dB readouts + FLAT (RX/TX EQ).
5. **Color language** — blue = main RX / VFO A, green = sub RX / VFO B,
   orange = TX; active choice = filled/inverted highlight. *(Already our
   `ColorRole` palette, `FR-UI-10`.)*

---

## 3. CAT commands the screens need

Resolved from the *K4 Programmer's Reference rev. D12* and cross-checked against
QK4's machine-readable command table (`R-EXT-03`, `R-EXT-01`; the two agree
except where noted). All commands terminate with `;`; `$` = sub-RX/VFO-B variant;
`#…` = Display group; `#H…` = external-monitor variant. Add each to `k4-protocol`
with a round-trip test **before** wiring; confirm the "verify" rows on a real
radio (`ASM-05`).

### 3.1 Already in `k4-protocol` (reuse) — with corrections

`MD/MD$` mode (1 LSB…9 DATA-R, no 8), `BW$` bandwidth (**value = Hz ÷ 10**, e.g.
`BW$1200;` = 12.00 kHz), `AG$` AF gain (000–060), `RG$` RF gain (**written with a
minus sign**, `RG$-nn;`, 00–60), `RA$` atten (0/3/…/21 dB + on flag), `PA$`
preamp (0–3 + on flag), `GT$` AGC (0 off/1 slow/2 fast), `NB$`/`NR$` (level + on),
`FA/FB` freq, `FT` split, `RT$`/`XT`/`RC$` RIT-XIT, band up/down.

### 3.2 New commands per screen (add clean-room)

| Screen | Function | Command | Syntax | Range / values |
|---|---|---|---|---|
| RX EQ | 8-band RX EQ | `RE` | `RE‹a…h›;` — 8 × 3-char signed (`+00`); `REF;` = flat | −16..+16 dB; bands 100/200/400/800/1200/1600/2400/3200 Hz |
| TX EQ | 8-band TX EQ | `TE` | `TE‹a…h›;` | −16..+16 dB; same bands |
| TX (keyer) | weight+paddle+iambic | `KP` | `KPionnn;` i=iambic A/B, o=N/R paddle, nnn=weight | weight 090–125 (×0.01) |
| TX (keyer) | speed | `KS` | `KSnnn;` | 008–100 WPM |
| TX (mic) | input source | `MI` | `MIn;` | 0 front,1 rear,2 line,3 front+line,4 rear+line |
| TX (mic) | gain | `MG` | `MGxxx;` | 000–080 |
| TX (mic) | preamp/bias/buttons | `MS` | `MSabcde;` a=front preamp(0/10/20 dB), b=front bias, c=front UP/DN, d=rear preamp, e=rear bias | per-field |
| TX (line) | line-in source+level | `LI` | `LIuuullls;` s=0 USB/1 jack | levels *verify* |
| MAIN RX (line) | line-out L/R | `LO` | `LOlllrrrm;` m=0 indep/1 R=L | 000–040 *verify* |
| BAND | band select | `BN$` | `BN$nn;` / `BN$+;` / `BN$-;` / `BN$/;` | 00=160m…10=6m; 16–25 XVTR |
| BAND | band-stack recall | `BN$` | `BN$^;` (next register) | — |
| BAND | transverter band | `XV` | `XV$n;` | 1–12 |
| DISPLAY | pan mode | `#DPM` | `#DPMn;` | 0 single-A, 1 single-B, 2 dual → maps to our `ViewMode` |
| DISPLAY | span | `#SPN$` | `#SPN$n;` | 6000–368000 Hz |
| DISPLAY | reference | `#REF$` | `#REF$n;` (`#AR…` auto) | −200..+60 |
| DISPLAY | scale | `#SCL` | `#SCLn;` | 10–150 |
| DISPLAY | averaging | `#AVG` | `#AVGnn;` | 01–20 |
| DISPLAY | peak | `#PKM` | `#PKMn;` / `#PKM/;` | 0/1 |
| DISPLAY | fixed / freeze | `#FXT`,`#FXA`,`#FRZ` | `#FXTn;`(0/1), `#FXAn;`(0–4 mode), `#FRZn;`(0/1) | — |
| DISPLAY | waterfall palette | `#WFC$` | `#WFC$n;` | 0 gray,1 color,2 teal,3 blue,4 sepia |
| DISPLAY | waterfall height | `#WFH` | `#WFHnnn;` | 000–100 % |
| DISPLAY | pan NB | `#NB$`,`#NBL$` | `#NB$n;`(0 off/1 on/2 auto), `#NBL$n;`(0–14) | — |
| Fn / VFO | copy / swap | `AB` | `ABn;` | 0 A→B, 1 B→A, 2 swap freq; 3–5 = all |
| Fn / VFO | step up/down | `UP`/`UPB`,`DN`/`DNB` | `UP;` etc. (one **step**, no count) | arbitrary QSY → set `FA/FB` |
| TX / ANT | TX antenna | `AN` | `ANn;` / `AN/;` | 1–3 |
| RX / ANT | RX antenna | `AR$` | `AR$n;` | 0 off…7 (KAT4 RX ANT) |
| MENU | menu-addressed param | `ME` | `MEiiii.nnnn;` set, `MEDFiiii;` def, `MOiiii;` open | per menu-item id |

### 3.3 Residual gaps — verify on a real radio (`ASM-05`)

- **QSY-by-n is not a command** in D12: `UP/DN/UPB/DNB` step by the current step
  size only. Arbitrary QSY (e.g. the manual's `UPB1000;` macro example) is *not*
  a documented parameterised form — set the VFO with `FA/FB` instead. Verify.
- **`RE` leading field:** D12 prints `REnabcdefgh;` but never defines the `n`
  (`TE` has no such field) — likely a doc typo. Verify the payload length live.
- **`LI`/`LO` level ranges** print as "TBD" in D12 — confirm on radio.
- **Mic bias/preamp:** D12's `MS` breaks these out (fields a–e); QK4 only exposed
  the generic `MS` — use D12's field map, verify.

---

## 4. Action list (prioritized)

Legend: **[wire]** = confirmed CAT command available (§3) — build functional ·
**[C]** = could/later. *(After the D12 + QK4 extraction, every §1 primary screen
is now wireable; only a few residual fields (§3.3) await live verification.)*

**Phase 0 — protocol layer (do first) — ✅ DONE**
1. ✅ Added the §3.2 commands to `k4-protocol` (`cat.rs`) with byte-exact tests
   (`tests/cat.rs`): `RE`/`TE`/`REF` (EQ), `KP`/`KS` (keyer), `MI`/`MG`/`MS`/`LI`/`LO`
   (audio), `BN`/`BN$`/`BN^`/`XV` (band), the `#…` display family incl. `#DPM`,
   `AB`, `AN`/`AR`, `MO`/`MEDF`/`ME` (menu). Requirements `FR-EQ-01`, `FR-KEY-01`,
   `FR-AUD-CFG-01`, `FR-VFO-07`, `FR-ANT-01`, `FR-MENU-01` added; `FR-VFO-04` /
   `FR-PAN-CTL-01` extended. Thin `WorkerCmd`s are added per-screen in Phase A+
   (adding them now would be dead code until a screen sends them).

**Phase A — framework & the two named EQ screens — ✅ DONE**
2. ✅ Graphic-equalizer widget built (8 vertical sliders + dB readout + `−/+`
   steppers + FLAT), sized to the spectrum-frame slot. (parameter-adjuster and
   dual-action button widgets are still TODO for later screens.)
3. ✅ **RX EQ** (MAIN RX & SUB RX) and **TX EQ** (TX) screens live in the
   spectrum-frame slot; adjusting a band sends `RE`/`TE` and FLAT sends `REF`
   (RX) / zeros (TX) via new `WorkerCmd`s. Sub-RX shares the RX EQ command with a
   noted caveat (no `RE$`; §3.3). Values held in app state; reading the radio's
   current EQ back (RadioState `RE`/`TE` parse) is a follow-up. (SCR-EQ-*)
4. ✅ Retired the stub context sub-row (and the now-dead `context_items` /
   `ViewMode::next` / `CycleViewMode`).

**Phase B — DISPLAY & BAND — ✅ DONE**
5. ✅ **DISPLAY** screen — pan mode (drives `ViewMode` + sends `#DPM`), REF
   (`#REF`), SPAN (`#SPN`), SCALE (`#SCL`), AVG (`#AVG`), waterfall height
   (`#WFH`) + palette cycle (`#WFC`), PEAK (`#PKM`) and FREEZE (`#FRZ`) toggles —
   each pushing its `#` command live via a generic `WorkerCmd::Cat`. Center /
   cursors / monitor-target and pan-NB level deferred. (SCR-DSP-*)
6. ✅ **BAND** screen — direct band grid 160 m–6 m (`BN`), band up/down, and
   band-stack recall (`BN^`). GEN / memories / transverter (`XV`) deferred `[C]`.
   (SCR-BAND-*)

**Phase C — TX detail & MENU — ✅ DONE**
7. ✅ **TX** config screen — a tab row (EQ / KEYER / MIC / LINE) mirroring the
   K4's TX config row: keyer speed/weight/paddle/iambic (`KS`/`KP`), mic
   input/gain/preamp/bias (`MI`/`MG`/`MS`), line-in source/level (`LI`), plus the
   TX EQ tab. Rear-mic + button config deferred. (SCR-KEY-*, SCR-MIC-*, SCR-LI-*)
8. ✅ **Fn** screen — VFO copy/swap operations (`AB`). F1–F8 macro triggers
   (remote switch emulation), macro editor, SW list, DX list deferred `[C]`.
   (SCR-FN-*)
9. ✅ **MENU** screen — the **full K4 configuration-menu list** (89 items
   enumerated from the D12 menu table, `ui::menu_items`), **searchable**
   (`ui::menu_search`) and scrollable; tapping an item opens it on the radio
   (`MO`). In-app value **edit / lock / NORM** still needs `MEDF`/`ME` read-back
   (RESP parsing) — follow-up. (SCR-MENU-*)

**All seven primaries now open a functional screen in the spectrum-frame slot**
(`ui::screen_kind`, tested). Remaining per-screen depth (antenna config, line
out, memories, text decode, full menu list, macro editor) is Phase D / follow-up.

**Phase D — config-row sub-screens — ✅ (wireable part done)**
10. ✅ **RX antenna** (`AR`/`AR$`) and **LINE OUT** (`LO`) added as tabs on the
    MAIN/SUB RX screen; **TX antenna** (`AN`) added as a tab on the TX screen.
11. **Outbound-only completions done** (`FR-VOX-01`/`FR-TX-MSG-01`/`FR-SW-01`/
    `FR-VFO-04`): BAND **XVTR** select (`XV`); TX **TEXT** send (`KY`) and **VOX**
    (`VX`); Fn **SWITCHES** panel (SPOT/TUNE/ATU/DIV/LOCK/MON via `SW`) and a
    searchable **DX prefix list** (client-side, starter set).
12. **Still deferred — needs RESP read-back or a data stream** (next session):
    TEXT **DECODE** (RX decode stream), MENU value **edit/lock/NORM** and the
    EQ/DISPLAY/TX/RX screens reflecting the radio's current values on connect
    (`RadioState` parse of `RE`/`TE`/`#…`/`KP`/`MI`/`ME`… RESPs), the SPOT tuning
    plot, status display, on-screen keyboard, help overlay, and the memory *list*
    (needs the pending `MC` command). The DX list is a data-population task.

**Phase 2 — memories via switch emulation — ✅ DONE**
12. ✅ The K4 memory-channel command (`MC`) is **pending in D12**, so quick
    memories are delivered via **front-panel switch emulation** (`SW`, `FR-SW-01`):
    the Fn screen gains **RCL/STO M1–M4** (tap = recall/play `SW17/51/18/52`,
    hold = store `SW162–165`) and **PF1–PF4** (`SW153–156`). A full RESP-parsed
    memory *list* still awaits the `MC` command.

**Cross-cutting**
- Order is protocol-first: add + test each §3.2 command before its screen wires
  to it. Never fabricate — the §3.3 residuals stay `[layout]`/unsent until
  verified on a real radio (`ASM-05`).

---

## Change history

| Date | Ver | Author | Change |
|---|---|---|---|
| 2026-07-02 | 0.1 | DC0SK | Initial spec: screens extracted from *Intro to the K4 rev C5* (pp.8–45) via a 3-way parallel read; catalog by primary, reusable components, CAT gaps, prioritized action list. |
| 2026-07-02 | 0.2 | DC0SK | Cross-referenced R-EXT-03 (Programmer's Reference D12 / Command Index / Operating Manual D14) as the source resolving the §3 CAT-command gaps. |
| 2026-07-02 | 0.3 | DC0SK | Added QK4 (R-EXT-01) source as a facts-only cross-check for the §3 CAT commands, alongside the Programmer's Reference D12. |
| 2026-07-02 | 0.4 | DC0SK | Resolved §3 CAT commands from Programmer's Reference D12 + QK4 (reconciled): confirmed command table (RE/TE EQ, KP/KS keyer, MI/MG/MS mic, LI/LO line, BN/XV band, #-display incl. #DPM, AB, AN/AR, ME menu access); D12 broke out mic bias/preamp and gave menu-addressed access. Action list re-sequenced protocol-first; all §1 primary screens now [wire]. Residual gaps (QSY-by-n, RE leading field, LI/LO ranges) flagged for ASM-05. |
| 2026-07-02 | 0.5 | DC0SK | Phase 0 complete: §3.2 commands implemented in k4-protocol with tests; action list item 1 marked done. |
| 2026-07-02 | 0.6 | DC0SK | Phase A complete: graphic-EQ widget + RX/TX EQ screens in the spectrum-frame slot (wired to RE/TE/REF); stub context sub-row retired. |
| 2026-07-02 | 0.7 | DC0SK | Phase B complete: DISPLAY screen (pan mode/#DPM, REF/SPAN/SCALE/AVG/WF/palette/PEAK/FREEZE via #-commands) and BAND screen (BN direct select + band-stack), routed through a generic WorkerCmd::Cat. |
| 2026-07-02 | 0.8 | DC0SK | Phase C complete: TX config screen (tabbed EQ/KEYER/MIC/LINE → KS/KP/MI/MG/MS/LI), Fn screen (VFO ops AB), MENU screen (open items via MO). All 7 primaries now open a built screen; added pure `screen_kind` dispatch (re-anchors FR-UI-19). |
| 2026-07-02 | 0.9 | DC0SK | Phase D (wireable part): RX antenna (AR/AR$) + LINE OUT (LO) tabs on MAIN/SUB RX, TX antenna (AN) tab on TX. Gesture-invoked screens + TEXT DECODE + full MENU list deferred to Phase 2. |
| 2026-07-02 | 0.10 | DC0SK | Phase-2 memories via SW switch emulation (MC pending in D12): Fn screen gains quick memories (RCL/STO M1–M4) + PF1–PF4. |
| 2026-07-02 | 0.11 | DC0SK | Full MENU list: 89 config-menu items enumerated from the D12 menu table (`menu_items`), searchable (`menu_search`), scrollable; tap opens on the radio (MO). Value read-back still pending. |
| 2026-07-02 | 0.12 | DC0SK | Everything possible without read-back: BAND XVTR (XV), TX TEXT send (KY) + VOX (VX), Fn Switches (SW) + searchable DX list. Remaining = read-back / data-stream items for the next session. |
