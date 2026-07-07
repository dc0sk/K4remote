---
title: "UI Style Guide"
status: Draft
version: "0.1"
updated: 2026-07-06
authors:
  - Simon Keimer (DC0SK)
---

# UI Style Guide

The visual language of the K4 Remote panel. It is **reference-faithful** to the
K4's own 7″ LCD and the reference iOS/desktop client (`R-EXT-02`) in *meaning and
layout*, but uses **our own monitor-tuned palette**, not a pixel copy (`ADR-15`).

The decidable parts of this style — colour roles, surface shades, and theme
resolution — are pure functions in **`app/src/ui.rs`** (ARC-15), unit-tested
without a display (`trace: FR-UI-10 / FR-UI-15 / FR-UI-17`). The iced layer
(`app/src/main.rs`, ARC-08) maps them to concrete `iced::Color` and widget
styles. **This document is descriptive of that code — the source is
authoritative.** Design rationale and provenance live in
[`ui-design.md`](ui-design.md).

## Principles

1. **Semantic colour, not decorative.** Every accent is a *role* (VFO A, TX,
   caution…), never a raw colour at the call site. Change the role once; the
   whole UI follows.
2. **Layered surfaces convey depth.** A small set of `Shade`s (background →
   panel → control → edge) step in luminance so grouping reads without heavy
   chrome — the visual grammar of the K4 LCD.
3. **Operating-critical state is glanceable.** Frequency, mode, and TX/RX state
   are large, high-contrast, and update within 200 ms (`NFR-USE-01`).
4. **Four themes, one palette shape.** Dark / Light / Contrast / System share the
   same roles and shades; only the values change.
5. **ASCII-safe glyphs.** The default font ships text glyphs but **not**
   box-drawing arrows (▼▲◀▶⇄ render as tofu). Use `−`, `+`, `>`, `A > B`, etc.

## Themes (`FR-UI-17`)

Cycle with the header **Theme** button: **Dark → Light → Contrast → System → …**

| Theme | When | Character |
|---|---|---|
| **Dark** *(default)* | normal / shack use | Near-black LCD field; the base palette. |
| **Light** | bright rooms / daylight | White ground; accents darkened for contrast. |
| **Contrast** | accessibility / low vision | Pure black & white with brightened accents. |
| **System** | match the desktop | Follows the OS light/dark preference; resolves to **Dark** or **Light** (`detect_system_dark`). |

`System` has no palette of its own — it resolves to Dark or Light, so the tables
below give the three concrete palettes.

## Colour roles (`ColorRole`)

Semantic accents. *Our meaning follows the K4; the values are our own.*

| Role | Meaning | Dark | Light | Contrast |
|---|---|---|---|---|
| `VfoA` | VFO A / main RX | `#3D9BFF` | `#1E66D0` | `#4DB1FF` |
| `VfoB` | VFO B / sub RX; also *active/selected* | `#33CC66` | `#1E8A44` | `#3DF07A` |
| `TxActive` | transmit state + TX-side values | `#FF9A1E` | `#C76A00` | `#FFB02E` |
| `RxValue` | receive readouts (ink / near-white) | `#ECEFF2` | `#1A1E24` | `#FFFFFF` |
| `Caution` | warnings, e.g. high SWR | `#FFD433` | `#B88600` | `#FFEE00` |
| `Inactive` | off / available control (dim) | `#666B72` | `#7A8088` | `#B0B0B0` |

## Surface shades (`Shade`)

Layered greys/tints. Each step changes luminance so *recessed* (wells, meter
tracks) reads distinct from *raised* (panels, controls).

| Shade | Use | Dark | Light | Contrast |
|---|---|---|---|---|
| `Bg` | window background | `#0B0D10` | `#EEF1F4` | `#000000` |
| `Panel` | grouping panel behind related controls | `#14171B` | `#FFFFFF` | `#0A0A0A` |
| `Track` | recessed well: meter track, waterfall margin | `#1A1D22` | `#E4E8ED` | `#141414` |
| `Control` | interactive control (button) at rest | `#24282E` | `#E8ECF1` | `#1E1E1E` |
| `ControlHover` | control under the pointer | `#2F343B` | `#DADFE6` | `#303030` |
| `Edge` | hairline border / divider | `#3A4048` | `#C6CDD5` | `#FFFFFF` |

## Buttons (`BtnKind`)

| Kind | Use | Fill | Text | Border | Hover |
|---|---|---|---|---|---|
| `Plain` | default / available action | `Control` shade | `RxValue` | `Edge` | → `ControlHover` |
| `Active` | engaged / selected | `#1E5FB8` | white | `#2F77D0` | — |
| `Amber` | transmit-related | `TxActive` (`#FF9A1E`) | black | `TxActive` | — |
| `Ptt` | push-to-talk (armed) | `Control` shade | `RxValue` | `Danger` | → `ControlHover` |
| `Danger` | destructive (power off, e-stop) | `#8E1F17` | white | `Danger` | — |

- **Danger accent** = `#E5483C` (borders on danger/PTT controls).
- The **`Active` fill blue `#1E5FB8`** is the button-selected blue (matches the
  reference client's engaged state). It is deliberately *distinct* from the
  `VfoA` role blue `#3D9BFF`, which is for **text/labels**, not fills.
- **Disabled** → `Panel` shade fill + `Inactive` text.
- **Toggles** show state, not just on/off colour: engaged toggles use the
  `Active` fill; a two-line toggle (`two_line_btn`) puts the label above the
  value.

## Typography

iced logical pixels; single sans family.

| Size | Use |
|---|---|
| 38 | VFO frequency readout (the hero number) |
| 20 | App title (`K4 REMOTE`) |
| 18 | Modal dialog titles (Settings, About) |
| 13 | Values, readouts, text inputs |
| 12 | Buttons, tab/segment labels, in-screen section headers |
| 11 | Panel & field labels (`TRANSMIT`, `DIAGNOSTICS`, stepper labels) |
| 10 | Notes / help / provenance lines |

Section labels (size 11) use `Inactive`; the value they describe uses `RxValue`
or its role colour.

## Layout & spacing

| Constant | Value | Meaning |
|---|---|---|
| `DEFAULT_WINDOW_SIZE` | `1280 × 884` | Landscape; sized to fit content without a startup scrollbar (`FR-UI-21`). |
| `VFO_BAND_H` | `160` | VFO header band height (A · centre TX/SPLIT/RIT · B). |
| `SCREEN_H` | `300` | Spectrum / config-screen slot — fixed so swapping screens doesn't jump (`FR-UI-19`). |
| `BOTTOM_PANEL_H` | `168` | `TRANSMIT` and `DIAGNOSTICS` panels — equal height. |

Body padding `14`; section spacing `10`; control spacing `6–8`; DISPLAY grid
cells are a fixed `200 px` so labels/buttons/values align across rows and
columns.

## Component patterns

- **Softkey screens (`FR-UI-19`)** — a primary softkey (`MENU/Fn/DISPLAY/BAND/
  MAIN RX/SUB RX/TX`) swaps *only* the spectrum slot for that screen; the slot is
  fixed `SCREEN_H`, the rest of the UI stays put.
- **Grid cell (`disp_stepper`)** — a fixed-width `label [−] value [+]` cell so
  steppers line up in a grid.
- **Modal dialog (`modal_scrim`)** — a centred card over a 60 %-black scrim
  (Settings, About).
- **Connection indicator (`conn_status`)** — a coloured dot + label: `VfoB`
  green = connected, `TxActive` amber = connecting, `Inactive` grey =
  disconnected (`FR-UI-22`).
- **S-meter** — proportional bar in a `Track` well; turns `Caution` yellow at/above
  S9 (`FR-UI-10/15`).
- **Guarded destructive actions** — power-off uses a two-step *arm → confirm*
  (Danger styling, auto-disarms on navigation/disconnect); the transmit
  **emergency stop** is an immediate Danger button.

## Where it lives

| Concern | Location | Trace |
|---|---|---|
| Roles, shades, theme resolution (pure) | `app/src/ui.rs` (ARC-15) | `FR-UI-10/15/17` |
| iced `Color` / button / container styling | `app/src/main.rs` (ARC-08) | `FR-UI-*` |
| Rationale, K4 / reference-client provenance | [`ui-design.md`](ui-design.md), `ADR-15` | — |
