---
title: "Gap Analysis — K4 Operating Features & Touch-Interaction Model"
status: Draft
version: "0.1"
updated: 2026-07-19
authors:
  - Simon Keimer (DC0SK)
---

# Gap Analysis: K4 Operating Features vs. K4 Remote

> **Provenance.** Researched and drafted by an AI agent on 2026-07-19 from the vendor
> documentation in `docs/references/external/`, commissioned by DC0SK. Claims carry their
> evidence source inline. Anything marked *inference*, *recalled*, or *web* has **not** been
> confirmed against hardware or against vendor documentation held in this repo — see the
> open-questions section before acting on it. This is a research input, not an approved
> baseline.

Sources (all in `docs/references/external/`), cited as:

- **D14** = *K4 Built-In Operating Manual, rev D14.pdf* (126 pp; page numbers from `pdftotext` page breaks)
- **Intro** = *Intro to the Elecraft K4, rev C5.pdf* (46 pp)
- **iOS** = *K4-Control for iOS.pdf* (91 pp; third-party app by Roskosch, referenced from the Elecraft docs page — treat as *a* touch mapping, not Elecraft's own; D14 p.81 confirms Elecraft lists "a third-party iOS application" as a supported client type)
- **PRG** = *K4 Programmer's Reference rev. D12* (grepped via the `.html` copy)
- **CmdIdx** = *K4-Command Index by Description, rev D5* (single-sheet command index)

Baseline established from `docs/requirements/system-requirements.md` v0.20, `docs/requirements/ui-gap-analysis.md` v0.2 (2026-07-07), `crates/k4-protocol/src/cat.rs` / `state.rs`, and `app/src/main.rs` as of commit `c70c758`. Anything marked **[inference]** is my judgement, not documented fact.

---

## 1. Summary — highest-value missing things

1. **No press-and-hold anywhere in the app.** The entire K4 interaction model is built on tap (white) vs. hold ≥ ~0.5 s (yellow) for *every* switch and many on-screen buttons (D14 p.16). Our UI is single-click only (verified: no long-press handling in `app/src/main.rs`).
2. **ATU family completely absent** — ATU in/bypass (`AT`), ATU tune (`TU3`/`TU4`), TUNE carrier / TUNE LP (`TU1`/`TU2`) (D14 pp.19–21; PRG `AT`, `TU`). Not in the SRS, not in the 07-07 gap list, not in `cat.rs`. Genuinely new finding.
3. **Tap-again-for-alternate idiom** — tapping the active mode button again selects the alternate mode (LSB↔USB, CW→CW-R) (D14 p.32/48); knob-button pairs alternate PWR↔DLY, BW↔HI-CUT etc. (D14 pp.23–24). PRG gives `MD$/;` toggle for free.
4. **Hold-and-drag fine-tuning**: hold a finger on a pan signal → mini-pan pops up, slide left/right to fine-tune, lift to dismiss (D14 pp.70–71; Intro p.26). We have click-to-QSY only.
5. **Frequency-memory operating loop** — tap a VFO's MHz digits → recall window; 200 memories + 4 per-band quick memories; `MC`/`MB` still `[Pending]` in PRG D12, so an **app-side memory store** (as the iOS app ships, iOS p.11) is the practical route.
6. **Tap VFO digit to set tuning step** (underlined digit = current step) (D14 p.27). We use digits only for increment/decrement (FR-VFO-08).
7. **Message play/record loop** — CW/FSK/PSK M1–M4 × 2 banks with chaining, auto-repeat, prosigns (D14 p.50); DVR record (`DA`) beyond bare `PB` play (D14 p.75; PRG `DA`).
8. **Tap S-meter → mini-pan**, tap status area → status-display group (UTC / ID / V·I·P·SWR / rel-dB sig level) (D14 pp.27, 29, 75–76).
9. **RX audio record/playback** — 90 s rolling loop of received audio, hold [AF REC]/[AF PLAY] (D14 pp.74–75). Client-side implementation is nearly free for us. **[inference]**
10. **Audio character family** — main/sub mix `MX`, balance `BL`, audio effects `FX` (sim-stereo / pitch-map), AF limiter `AL` (D14 pp.71, 74; PRG). None encoded.

---

## 2. Touch-screen / direct-manipulation model

### 2.1 The K4's own conventions (D14, Intro)

**Tap vs. hold, everywhere.** Every physical switch has a tap function (white label) and a hold function (yellow label); hold = press longer than ~½ s (D14 p.16). On-screen buttons follow the same convention: a yellow label on a button means a hold function exists (D14 p.16). Examples: [RATE] tap / [KHZ] hold on one switch (D14 p.16); [NB]/[NR]/[NTCH] tap = on/off with last settings, hold = adjust the setting (Intro p.18); [ATTN] hold steps attenuation in 3 dB steps (Intro p.18); AGC button tap = slow/fast, **hold = AGC off** (D14 p.40).

**Tap-a-value-region idioms** (all verified in D14):

| Screen region | Tap does | Source |
|---|---|---|
| VFO kHz/Hz digits | select tuning step; the current step digit is **underlined** | D14 p.27, p.31 |
| VFO MHz digits | open the frequency-memory store/recall window | D14 p.27, p.44 |
| Mode ID icon next to a VFO | open the mode button group *for that VFO* | D14 p.27, p.48 |
| A mode button that is already active | switch to its **alternate** (LSB↔USB, CW→CW-R, DATA→DATA-R) | D14 p.32, p.48 |
| S-meter | toggle the **mini-pan** for that receiver | D14 p.27, p.66–67 |
| RIT/XIT offset box | toggle RIT on/off (mouse: left-click = RIT, **right-click = XIT**) | D14 p.27, p.36 |
| Filter passband graphic | cycle filter presets FL1→FL2→FL3 | D14 p.28 |
| RX antenna name icon under each S-meter | select the receive antenna | D14 p.16 |
| Status area (lower-left, UTC clock) | open the STATUS DISPLAY button group | D14 p.29, p.75 |
| Band button (already on that band) | cycle the 3 band-stacking registers | D14 p.44 |
| Menu lock symbol | unlock a protected menu entry before editing | D14 p.37; Intro p.29 |
| EQ slider ends (+/−) or the slider itself | adjust that band; FLAT centres all | Intro p.21 |

**Panadapter gestures** (D14 pp.70–71):
- **Tap-to-QSY**: a brief tap moves the VFO cursor to that frequency; in TRACK cursor mode it also re-centres the pan.
- **Hold-and-slide**: tap a signal and *keep the finger down* → the mini-pan is displayed; slide left/right to fine-tune; mini-pan disappears on lift. Same gesture with a mouse (Intro p.26: "HOLD your finger or mouse on a signal of interest, then slide it left or right").
- **Lock disables QSY**: a locked VFO also disables tap-to-QSY for its pan (D14 p.31, p.71).
- **Cursor modes** TRACK / FIXED1 / FIXED2 / SLIDE1 / SLIDE2 / STATIC govern how the pan window follows the cursor (D14 p.70; PRG `#VFA`/`#VFB`, alias `#CUR$`), with off-screen cursor direction arrows (blue A / green B triangles, D14 p.66).
- **Mouse model**: a mouse icon appears in each pan showing the wheel-tuning target (A/B/R = RIT); left/right buttons are configurable QSY targets (MENU "Mouse L/R Button QSY": Left-Only vs Left=A/Right=B, D14 pp.96–97); clicking the thumbwheel alternates VFO↔RIT target; clicking *on the mouse icon itself* changes the A/B assignment without QSY (D14 p.67).

**Multi-function-knob idiom** (D14 pp.23–25): each knob has two two-line LCD buttons; the *selected* button is marked with a colour indicator (orange = XMTR, blue = main, green = sub). Tapping a button a **second time alternates between its two functions**: XMTR lower = PWR↔DLY, upper = MIC↔CMP (voice) or WPM↔PITCH (CW/FSK/PSK — per-mode relabel); FILTER = BW↔HI-CUT / SHFT↔LO-CUT; RF/SQL = RF↔SQL per receiver. Holding the knob reaches a third function: MON (XMTR), NORM (FILTER), BAL (RF/SQL).

**Popups**: every button group / selector window has a "dismiss" button (curved-arrow) (Intro p.20, D14 p.77); text entry pops an on-screen alphanumeric keyboard, with an attached hardware keyboard always accepted as an alternative (D14 p.31).

**B SET as a mode**: tapping [B SET] (or `<SUB RX>`) retargets subsequent controls — RATE, KHZ, FREQ ENT, MODE, preamp/attenuator/filter — at VFO B/sub, indicated by the green B SET icon (D14 p.35, p.41; Intro p.16; PRG `BS`).

### 2.2 What Elecraft's supported iOS client adds (iOS)

- **Long-tap a button → its related settings**: long-tap VOX → VOX level/delay/anti-VOX sheet (iOS p.7); long-tap the power meter → swap it for ALC/CMP/etc. (iOS p.8).
- **Tap the frequency display → direct-entry screen** (iOS pp.8, 13).
- **Waterfall gestures**: *drag* horizontally to move the centre frequency; *double-tap* to QSY the active VFO; long-tap to pick a frequency in the FT8 tool (iOS pp.13, 64).
- **Tuning Panel** floating window: `<< < > >>` step buttons plus a **flickable tuning wheel** with configurable direction (iOS pp.13–14; visible in `main-window-ipad.png`).
- **Supplemental floating panels** via a "Show" menu: Tuning, RIT/XIT, Memories (app-side, explicitly "separate from those stored in the radio", iOS p.11), Macro quick-access buttons, DTMF/1750 Hz panel (iOS pp.10–11).
- List idiom: tap = details, double-tap = tune (DX-cluster spots, FT8 receive list) (iOS pp.18, 62–63).

### 2.3 Our current model vs. the above

We already have: click-to-QSY with mode-aware passband anchoring (FR-PAN-05), wheel tuning per pane (`PaneWheel`), per-digit ±click tuning (FR-VFO-08), direct MHz entry for both VFOs, two-line state buttons (FR-UI-11), primary softkey rows (FR-UI-13), mini-pan render + `#MP$` toggle, mode-adaptive controls (FR-UI-24). Missing, in order of how much they change "feel": press-and-hold semantics; tap-again-for-alternate; hold-drag fine-tune; tap-digit-to-set-step + step underline; tap-value-to-edit popups (frequency display, memory window from MHz digits); dismissable button-group popups vs. our persistent panels **[inference: our panels are persistent; the K4 idiom is transient groups]**; S-meter/status-area/RIT-box/filter-graphic tap targets.

---

## 3. Missing feature families

Cross-check note: items marked ⟲ were already in `ui-gap-analysis.md` v0.2 and are **still open**; items marked ★ are **new** (not in that list or the SRS). The 07-07 list's A1/A2/A3, B1/B2/B3, C1/C3/C4, D1/D2/D3/D4, E1 (worker now indexes `PanFrame.receiver`), E3 (mini-pan canvas) are verifiably closed in the code and are *not* repeated here.

### 3.1 ★ ATU / TUNE (nothing implemented)

- What the radio does: [ATU TUNE] runs a 1–4 s match (5–10 W), per-band-per-antenna LC memories (32/combination); second tap within 5 s = extended search; hold [ATU] toggles in/bypass with an orange ATU icon (D14 pp.20–21). [TUNE] emits a carrier at PWR level; [TUNE LP] at a separate menu-set low power (D14 p.19).
- CAT: `ATn` 0/1/2 = not-inst/bypass/auto, `AT/;` toggle (PRG); `TUn` 0=exit, 1=tune, 2=tune LP, 3=ATU tune, 4=ATU extended search (PRG). `AT` RESP is already parsed in `state.rs` but never displayed or sent.
- Have: nothing in UI; `AT` parse only. **Highest-value new family** — remote operators must be able to match antennas. **[inference on value]**

### 3.2 ⟲ Frequency memories & scan (partial)

- Radio: 200 general memories (STORE/RCL windows, scroll with VFO A, `<STO>`/`<RCL>`/[CLR], "active recall" scrolling mode) + 4 quick memories per band; MHz-digit tap opens recall; `<MEM>` band button = recent memories (D14 pp.76–78, p.44). Scan runs between VFO A/B of the last-recalled memory, hold [SCAN], long-hold = live scan (D14 p.33).
- CAT: `MC` (Memory Channel) and `MB` (Message Bank) both literally "[Pending] TBD" in PRG D12 (verified). Quick memories only via `SW` emulation.
- Have: quick-mem M1–M4 via `SW` tap/hold (FR-SW-01), scan `SW149` + `IF` flag (FR-SCAN-01).
- Gap: no general-memory UX at all. The supported iOS client solves this **client-side** (iOS p.11) — an app-side memory bank (name, freq A/B, mode, filter) needs no new CAT support. ★ *client-side memory store* is the new, actionable part.

### 3.3 Messages: CW/FSK/PSK text + DVR voice (partial)

- Radio: 2 banks × 4 message buffers; [REC] then M1–M4 records via keyboard; tap plays, tap-during-play **chains**, hold = auto-repeat (interval in MENU), cancel via [REC]/[XMIT]/[TUNE]/paddle; prosign punctuation `( + = % * !`; EOL/EOT specials for FSK/PSK (D14 p.50). DVR: record from mic per slot (90 s), banks I/II, `<SAVE>`/[CLR], auto-repeat, "MSG RPT" flashing icon (D14 p.75).
- CAT: `KY` (have), `PB` play (have), `DA` = DVR record/status incl. auto-repeat wait time (PRG, verified) ★, `MB` pending.
- Have: `KY` free-text send, `PB1–8`/`PB0`, quick-mem SW taps. Missing: message-slot editor (client-side), chaining/repeat, DVR **record**, bank select, play-status display from `DA`.

### 3.4 ★ TX aids: TUNE/TEST/power readback

- `TU1`/`TU2` carrier tune (see 3.1); `TS` TX-test mode (zero-power practice; TX icon flashes) — the documented way to set up mic gain/CMP and practice CW off-air (D14 pp.20, 52–53; PRG `TS`); `PO` power output in 0.1 W with an **auto-delivery mode** (`PO1;`) (PRG); `PP` per-band power (PRG); `IN` TX-inhibit status (CmdIdx). ⟲ D7 covered TS/IN; `TU`, `PO`, `PP` are new.

### 3.5 ⟲/★ VFO behaviours

- ⟲ `LK`/`LK$` lock read-back (lock must also disable our click-to-QSY, D14 p.71 — ★ that coupling is new); ⟲ REV momentary (`SW`), `BS` B SET, `LN` VFO link (icon left of VFO B, offset preserved while tuning, D14 p.40), `BI` band-independence, `VO$` fixed VFO offset ±99999 Hz (PRG).
- ★ `UP/DN/UPB/DNB` move-by-current-step and `VC` per-mode coarse-step select (PRG) — relevant for K-Pod/keyboard tuning parity.
- ★ Mode toggle/cycle forms `MD$/;`, `MD$+;`, `MD$-;` (PRG under `MA`) — enables the tap-again-for-alternate idiom with one command.

### 3.6 ★ Audio character (RX ergonomics)

- `MX` main/sub mix (SO2V-style blends, D14 p.71 "RX Audio Mix with Sub ON"), `BL` balance, `FX` audio effects 0/1/2 = off/sim-stereo/pitch-map (D14 p.74), `AL` AF limiter 1–30 when AGC off (PRG; D14 p.40 note). ⟲ C6 listed BL/MX/FX; AL was C5. All still unimplemented (verified: none encoded).
- ★ RX audio record/play: 90 s loop, hold [AF REC]/[AF PLAY], A/B channels, ±5 s seek, `*` marker (D14 pp.74–75). No CAT equivalent found in PRG **[verified absent in CmdIdx; inference: do it client-side from our own RX stream]**.

### 3.7 FM extras (partial)

- Have: `RP` repeater offset, `PL` tone (fm_panel). Missing: ★ `DM` DTMF digits (FM-only, PRG) + stored CMD1–6 sequences + keypad UX (D14 pp.43, 54); the iOS app also offers a 1750 Hz burst panel (iOS p.10).

### 3.8 ⟲ Antenna configuration (partial)

- `ACN` antenna names (shown in the TX icon area, e.g. "2:YAGI", D14 p.30) and `ACT` TX-antenna access mask (rotation constraint, PRG); RX-antenna *tap targets* under each S-meter (D14 p.16). We cycle `AN`/`AR` + `ACM`/`ACS` but display no names.

### 3.9 ⟲/★ Display/pan family (partial)

- Still open from 07-07: E2 per-pan `#` targeting (the radio's LCD/EXT × A/&/B target-selector model, D14 p.68), E4 `#FXT`+pan-NB in DISPLAY (we encode both; UI wiring unverified), E5 `#WBS`/`#DSM`.
- ★ New: `#FPS` frames-per-second, `#SFL` spectrum fill (none/gray/gold), `#AR` auto-ref (auto reference level — the radio's REF LVL "AUTO" mode, D14 p.69), `#VFA`/`#VFB`/`#CUR$` cursor display modes (TRACK/FIXED/SLIDE/STATIC, D14 p.70), `SS` screenshot capture, `SC` screen count (PRG). Auto-REF is the notable one: the K4 places the baseline at the measured noise floor.

### 3.10 ⟲ Status display & system info (partial)

- Radio: status-area button group — DATE/TIME (+manual set), ID/TIME (editable callsign), TX PARAM (V/I upper, P/SWR lower; hold = all module voltages/currents/temps), SIG LEVEL relative-dB with "Set 0 dB" (for antenna A/B tests), Show Stat module-health list (D14 pp.75–76). CAT: `DB$` rel-dB meter, `SI` system auto-info (PRG), `ID` (have parse? — `ID` is in the SRS as FR-VFO-ID, not in `cat.rs`).
- Have: UTC clock + client count (FR-UI-STATUS-01). The rel-dB signal meter is a genuinely useful remote-operating tool (antenna comparisons). **[inference on value]**

### 3.11 ⟲ Transverter setup (partial)

Have `XV` band select. Missing: per-band config `XVN/XVM/XVR/XVI/XVO/XVP` (CmdIdx "Transverter Band Setup"); mW power scale switch when an XVTR band is active (D14 p.79 — PWR and RF-power scale change to mW).

### 3.12 ★ Data-mode rate & ⟲ TX bandwidths

- ★ `DR$` data rate: AFSK/FSK 45/75 baud, PSK 31/63 (PRG) — pairs with our existing `DT` sub-mode + text decode.
- ⟲ D5: `ES` ESSB on/off + 3.0–4.5 kHz BW, `DW` TX DATA bandwidth 2.0–4.0 kHz (PRG; D14 p.43).

### 3.13 Radio-side macros (partial, client-side exists)

Radio: macro editor (Fn→hold MACROS) assigns command strings to PF1–PF4, Fn F1–F8, REM ANT, K-Pod switches, plus a **startup macro** (D14 pp.89–90). We implemented the same concept client-side for K-Pod slots (FR-KPOD-06). Missing: on-screen macro quick-access buttons in the app itself (the iOS app has a Macros panel, iOS p.11) — trivial extension of the existing macro table. **[inference]**

### 3.14 Remote power-on (documented limit)

D14 p.89 and *Remote K4 On-Off Control Methods.pdf* confirm: power-off via `PS0;` works, power-**on** requires pulsing ACC pin 8 ≥ 200 ms — no CAT/Wake-on-LAN path. Already correctly captured in FR-PWR-01; no action beyond possibly documenting third-party on-switch hardware in user docs.

---

## 4. Smaller operating details (papercuts)

1. **Step-size visibility**: the current tuning step is shown by underlining one VFO digit (D14 p.27). Our step is invisible state.
2. **Keypad entry semantics**: 1–2 digits = MHz, 3+ = kHz, optional decimal points, `[X]` deletes last digit; on transverter bands ≥3 digits = MHz (D14 p.44). Worth matching in our direct-entry field.
3. **TX meters swap sides with SPLIT**: the TX bar graphs replace the S-meter of the *transmit* VFO, "a clear reminder … whether SPLIT is in effect" (D14 pp.27–28). Verify our in-pan TX meters follow the TX VFO's pane.
4. **CMP bar only in SSB/ESSB** (D14 p.27) — already mirrored (`show_cmp`).
5. **A>B / B>A double-tap copies all settings** incl. preamp/ATT and (cross-band) antenna selections (D14 p.33) — we encode `AB0–AB5`; the two-step tap UX (second tap within 2 s upgrades to full copy) is the missing bit.
6. **Message-play cancel routes**: [REC], [XMIT], [TUNE], or touching the paddle all cancel play (D14 p.50) — e-stop style consistency for our message play.
7. **VOX + message buttons**: with VOX on, tapping M1–M4 transmits immediately; with text decode on, typing transmits (D14 p.21).
8. **XMIT pre-arm in CW/FSK/PSK**: tapping XMIT arms (switches KEY OUT chain) but emits nothing until keyed (D14 p.20) — matches our arm model; worth reflecting the distinct "pre-armed" state.
9. **Menu ergonomics**: lock icon must be tapped before editing; `<NORM>` restores default; keypad icon appears when numeric entry allowed (D14 p.37).
10. **AGC hold = off** with automatic AF-limiter fallback (`AL`) (D14 p.40).
11. **Per-mode spot pitch and mini-pan cursor semantics**: solid line = carrier (CW/AM/FM/PSK) or suppressed carrier (SSB) or mark tone (FSK, with dotted space-tone line) (Intro p.26).
12. **Sidetone pitch advice**: 10 Hz steps but multiples of 50 Hz align IIR filters (D14 p.49) — relevant to our CW-pitch slider detents. **[inference]**
13. **Orange TX icon cluster**: VOX / ATU / QSK / MSG I-II / "MSG RPT" flashing / B SET / SPLIT ON / antenna name (D14 pp.29–30) — our header shows split/RIT/XIT but not VOX/ATU/QSK/MSG state.
14. **Off-screen VFO cursor arrows** on the pan (blue/green triangles, D14 p.66).
15. **Band-button re-tap = band stack** (D14 p.44): we expose `BN^` but the *tap-same-band-again* gesture is the K4 idiom.
16. **Locked VFO must refuse our click-to-QSY and wheel** (D14 p.71) — currently no lock state at all (LK read-back open ⟲A4).

---

## 5. Prioritised recommendations

| Proposed ID | Requirement (one line) | Value | Effort | Extends |
|---|---|---|---|---|
| FR-ATU-01 | Control the ATU: in/bypass (`AT`), ATU tune / extended tune (`TU3/TU4`), with ATU state icon | **High** | S | new §N family |
| FR-TX-TUNE-01 | Provide TUNE / TUNE LP carrier (`TU1/TU2/TU0`) and TX TEST (`TS`) with distinct flashing TX-Test indication | **High** | S | FR-TX-01 |
| FR-UI-HOLD-01 | Adopt tap/hold semantics on state buttons: tap = toggle/last value, hold = open that control's settings (levels, config) | **High** | M | FR-UI-11/13 |
| FR-UI-ALT-01 | Tapping the active mode button again selects the alternate mode (`MD$/;`); mode icon near each VFO opens the mode group | **High** | S | FR-MODE-01 |
| FR-PAN-10 | Hold-and-drag on the spectrum: press shows the mini-pan, dragging fine-tunes, release dismisses | **High** | M | FR-PAN-04/05, FR-UI-14 |
| FR-VFO-STEP-01 | Tap a frequency digit to select the tuning step; render the active step digit underlined; honour per-mode coarse rate (`VC`) | **High** | S–M | FR-VFO-03/08 |
| FR-MEM-01 | Client-side frequency-memory bank (store/recall/name/delete; recall window opened by tapping the MHz digits), quick-mems kept on `SW` | **High** | M | FR-SW-01 |
| FR-MSG-01 | Message centre: edit/play/chain/auto-repeat CW-FSK-PSK messages (`KY`), DVR record/play (`DA`/`PB`), bank select, play-status display | Med | M | FR-TX-MSG-01, FR-DVR-01 |
| FR-VFO-LOCK-01 | VFO lock state (`LK`) read-back; a locked VFO refuses click-QSY, wheel, and digit tuning | Med | S | ⟲A4, FR-PAN-04 |
| FR-AUD-CHAR-01 | Audio character controls: mix `MX`, balance `BL`, effects `FX`, AF limiter `AL` | Med | S | FR-RX-06 |
| FR-AUD-REC-01 | Client-side 90 s rolling RX-audio record/replay (AF REC/PLAY equivalent) | Med | M | FR-AUD-RX-01 |
| FR-MTR-05 | Status display group: TX V/I/P/SWR (`PO`, `SI`), relative-dB signal level with Set-0 (`DB$`), editable ID | Med | M | FR-UI-STATUS-01, ⟲E6 |
| FR-ANT-02 | Show user antenna names (`ACN`) on TX/RX antenna controls; honour `ACT` mask; RX-ant tap targets under each meter | Med | S | ⟲G1, FR-ANT-01 |
| FR-PAN-11 | Pan cursor modes (`#VFA/#VFB`: TRACK/FIXED/SLIDE/STATIC), off-screen cursor arrows, auto-ref (`#AR`), spectrum fill (`#SFL`), FPS (`#FPS`) | Med | M | FR-PAN-CTL-01, ⟲E2/E5 |
| FR-DATA-02 | Data rate select (`DR$`: 45/75 Bd, PSK31/63) beside the DT sub-mode strip | Med | S | FR-DATA-01 |
| FR-TX-BW-01 | ESSB on/off + BW (`ES`) and TX DATA bandwidth (`DW`) | Low–Med | S | ⟲D5, FR-AUD-CFG-01 |
| FR-FM-02 | DTMF keypad + 6 stored sequences (`DM`), 1750 Hz burst | Low | S | FR-FM-01 |
| FR-XVTR-01 | Transverter band setup (`XVN/XVM/XVR/XVI/XVO/XVP`) + mW power scale on XVTR bands | Low | M | FR-VFO-04 |
| FR-MACRO-01 | On-screen macro quick-access buttons reusing the K-Pod macro table | Low | S | FR-KPOD-06 |
| FR-UI-KEY-01 | On-screen alphanumeric keyboard equivalence: all text fields accept hardware keyboard (parity note, mostly already true) | Low | S | FR-UI-23 |

Effort scale: S ≈ a day-ish, M ≈ a few days, per this repo's demonstrated PR cadence. **[inference]**

### Contradictions / cautions noticed

- **Port numbers**: D14 p.81 says remote front-panel clients use **9204** (TCP or UDP selectable); our SRS/R-EXT-01 assigns 9205 plaintext / 9204 TLS-PSK. Both may be true (9204 outer port, TLS vs plain negotiated) — but D14 also offers a **UDP** mode our transport model doesn't mention at all. Flag for a hardware-in-the-loop check before any FR-CONN change.
- **`RO` targeting**: CmdIdx lists "RIT/XIT Offset `RO$`", but the project memory + `cat.rs` comment establish bare `RO` is required for the main VFO (RO$ is sub-RX). The command index's `$` suffixes are a *labelling convention* ("$ = sub variant exists"), not the recommended form — same trap as the `#REF$` LCD-mnemonic issue fixed in SRS 0.18.
- **iOS doc**: only the *Center* pan mode is supported there (iOS p.9) — its drag-waterfall gesture presumes centre mode; the real K4's six cursor modes (D14 p.70) are the richer model to copy.

*(File generated 2026-07-19 by the gap-mining agent; single deliverable, no repo files modified.)*
