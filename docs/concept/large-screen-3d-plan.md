---
title: "Large screens, 3D spectrum and the GRAPHICS tab — design (FR-UI-26, FR-PAN-15, FR-UI-27)"
status: Draft
version: "0.3"
updated: 2026-10-08
authors:
  - Simon Keimer (DC0SK)
---

# Large screens, 3D spectrum and the GRAPHICS tab — design

Three requirements from DC0SK (2026-10-08), with his answers to the open questions: **fit only**
(no UI zoom); **both** 3D styles, selectable; **tilt and depth** sliders; renderer **Auto / GPU /
CPU** for the next start, with the 3D view working on **both** renderers. This note fixes how,
before code. **(tuned)** marks an initial value to revisit on screen.

## 0. Review

v0.1 was reviewed adversarially (Fable, 2026-10-08): **REVISE (major)**, not release-blocking. Each
finding was checked against the source and taken:

| # | Finding | Disposition |
|---|---|---|
| 1 | **Blocker.** "The panadapter already fills" was false twice: the pane is `Length::Fixed(SCREEN_H)` = 300 px (`main.rs:9113`, mirrored by the menu-screen slot at `:7155` for FR-UI-19), and the whole body is inside `scrollable(body)` (`:9401`), under which a `Fill` height collapses. v0.1 read only the innermost widget | §2 rewritten: the slot and the menu screen become `Fill`, the main body is no longer scrollable; a structural test asserts no scrollable encloses the slot |
| 2 | **Blocker.** `ICED_BACKEND=wgpu` with no adapter fails both iced backends (tiny-skia accepts only its own name), so iced exits | §4: GPU sets `wgpu,tiny-skia` |
| 3 | The renderer in use is not observable from iced 0.13; `K4_WATERFALL=gpu` under tiny-skia already gives a blank waterfall | §4: an `AtomicBool` set in a GPU primitive's `prepare` is the honest "GPU in use"; `gpu_available()` follows the backend choice |
| 4 | "CPU cost bounded by pane height" is not what the code does: the CPU waterfall rasterises all rows × width every frame, and clones the history per draw | §2: both paths draw `min(rows, band px)` rows; the CPU waterfall keeps an incremental RGBA ring; the golden test's edge avoidance is re-derived |
| 5 | "Front row = classic spectrum" was false: the classic trace is the top 40 %; overlays assume it | §3.1: h0 and the overlay anchoring specified; ages pinned to `k/(D−1)`; depth capped at the pixels available |
| 6 | The CPU 3D path as specified (256 full-width fills per frame) would not keep up on tiny-skia | §3.4: a floating horizon into an RGBA image, newest first, O(depth × columns) |
| 7 | GPU: line strips + fills are two pipelines (2 × depth draws), and wgpu lines are 1 px | §3.4: one triangle-list draw with a depth attachment; lines as thin quads |
| 8 | `WindowOpened` carries no id; `Settings` has no `maximized` | §2: `open_main.then(\|id\| window::maximize(id, true))`; prefs loaded in `main` for §4 |
| 9 | serde fails the whole load on an unknown enum variant | §5: `#[serde(other)]` on the default variant |
| 10 | A pure draw-order test cannot catch a reversed GPU vertex mapping; a GPU-vs-CPU golden must run at the same column count | §6 |

## 0.1 As built (v0.3, 2026-10-08)

- **The 3D view uses the CPU rasteriser on both renderers** (§3.4's CPU path; under wgpu the image
  is uploaded and drawn by the GPU). Measured in release mode at the CPU caps (1024 × 640 image,
  512 columns): traces 1.8 ms (depth 64) / 4.2 ms (256), surface 3.8 / 6.5 ms per **rebuilt**
  frame — and a frame is rebuilt only when a row arrives or the view changes. That is well inside
  budget, so the **dedicated GPU shader path (§3.4 GPU) is deferred**: re-open it if the 3D view is
  measured to cost too much on a target (e.g. the Pi) or needs per-pixel columns on the GPU.
- **Spot nameplates** are drawn in the 3D view in the classic view's lanes (so a click or a hover finds
  the same plate), each tick reaching the front row through the same projection that draws it;
  the frequency axis and the passband sit along the front edge. (v0.3 first shipped without them;
  added afterwards.)
- The GPU golden gained a capped-band case; the waterfall golden's H became 512 (§2).

## 1. What exists (read from the code, 2026-10-08)

- **Main window:** opened at `max(1280, 1320) × 1000`, `min_size` 1320 × 1000 (`main.rs:1263`). On
  this computer's single 3440 × 1440 screen most of it is unused.
- **Panadapter:** a `Canvas` at `Length::Fill` both ways; the canvas splits its height 40 % spectrum /
  60 % waterfall (`spectrum.rs:352`). With the GPU waterfall, a `Shader` widget draws the bottom
  part under the transparent canvas (`main.rs:~9010`). No container caps the width.
- **History:** `WATERFALL_ROWS = 64` rows × up to `SPECTRUM_WIDTH = 1024` bins, shared between worker
  and UI (`PanShared`), mirrored on the GPU as an `R32Float` ring texture plus a per-row parameter
  texture (centre offset, span, bins) — `waterfall_gpu.rs`. The CPU rasteriser in `spectrum.rs` is
  the fallback and the golden reference.
- **Renderer choice:** iced picks wgpu or tiny-skia from `ICED_BACKEND` (`iced_renderer`
  `fallback.rs:224`), else tries wgpu first. The app separately decides `gpu_waterfall` from
  `waterfall_gpu::gpu_available()` — a wgpu adapter probe, overridable by `K4_WATERFALL=cpu|gpu`.
- **iced 0.13 window API:** `window::maximize(id, true)` exists; there is no monitor-size query.

## 2. FR-UI-26 — fit a large screen

- **Open maximised:** `open_main.then(|id| window::maximize(id, true))` at start (iced 0.13's
  `Settings` has no `maximized` field, and `WindowOpened` carries no id). The window manager sizes
  it to the screen's free area. `min_size` stays.
- **The pane must be allowed to grow** — today it cannot: the panadapter slot is pinned to
  `SCREEN_H` = 300 px and the main body is wrapped in `scrollable`. Changes:
  - the pane and the menu-screen slot both become `Length::Fill` in height, so they still match
    exactly when a primary screen replaces the spectrum (FR-UI-19 keeps holding, now at any size);
  - the main body is **no longer scrollable**: `min_size` (1320 × 1000) is the guarantee that the
    fixed panels fit, so at the minimum the pane gets what is left (at least the old 300 px — checked
    on screen at 1320 × 1000), and at any larger size it gets all the rest.
  - The structural test asserts that no `scrollable` encloses the slot and that both slots are
    `Fill` — not merely that `Fill` appears somewhere.
- **History grows with height, cost bounded by pixels:** `WATERFALL_ROWS` becomes **256** (tuned;
  2 MiB shared history, 1 MiB GPU ring per pane). Both renderers draw **`min(rows, band px)`** rows —
  a tall band shows more time, a short one the newest rows, never more rows than pixels:
  - GPU: the shader maps the band's pixel rows onto at most that many history rows;
  - CPU: the waterfall keeps an **incremental RGBA ring** — only new rows are rasterised (driven by
    the history's `total`, as `rows_to_upload` already is), with a full redraw only on a retune,
    span or scale change, or a resize — instead of rasterising every row every frame and cloning the
    history per draw.
  - The GPU-vs-CPU golden test's edge avoidance (derived for 64 rows) is re-derived for the new
    count.
  - Pi (`NFR-PORT-02`): its 1000-px-tall window keeps the band near today's height, and the
    incremental CPU ring makes a frame cheaper than today, not dearer.

## 3. FR-PAN-15 — the 3D view

### 3.1 Geometry (one pure module, both renderers)

`k4-stream::view3d` (pure, no GPU) owns the projection, so the CPU and GPU paths cannot disagree:

- A point is (frequency fraction `x` ∈ [0,1] across the view, level fraction `z` ∈ [0,1] from
  `top_dbm`/`range_db`, row index `k` = 0 newest … `D−1` oldest). Age `a = k/(D−1)` is pinned to the
  configured depth `D`, so the scene does not rescale while the history fills.
- **Projection** (oblique perspective, rows recede up and inward): the pane is `W × H`; the front
  row's baseline is the bottom (`y0(0) = H`), row `k`'s baseline `y0(a) = H − a·t·H`, its horizontal
  scale `s(a) = 1 − a·p` about the centre, and level height `h(a) = h0·s(a)` with `h0 = (1 − t)·H`
  (the front trace may rise to the top of the region the history does not use). `t` = tilt
  (0.2–0.9, default 0.6), `p` = 0.35 (tuned). The depth actually drawn is `min(D, rows, t·H px)` —
  never more rows than pixel rows to put them on.
- **Overlays in 3D** act on the **front row**: the frequency axis along the bottom, click-to-QSY and
  wheel tuning by `x` at the front, the passband overlay and spot nameplates on the front trace, the
  dB grid scaled to `h0`. In the classic view nothing changes.
- **Each row keeps its own centre and span:** columns map to a row's bins by `render::column_to_bin`,
  so a retune shifts the history and a part of a row outside its own span is a **gap** — no line,
  no fill there.
- **Occlusion rule:** a nearer row hides what lies below its own line. With baselines rising
  monotonically with age and `z ≥ 0`, drawing newest first and keeping, per screen column, the
  highest point drawn so far (a *floating horizon*) is exact: a farther row shows only where it
  rises above that horizon. A gap does not raise the horizon.

### 3.2 Stacked traces

Each row is a line; only its parts above the current horizon are drawn. Colour: the trace colour
faded toward the grid colour with age.

### 3.3 Shaded surface

Between row `k` and the horizon, each column's newly visible span is filled with the colour map at
that row's level, darkened slightly with age (tuned). The colour map carries height, as in the
waterfall.

### 3.4 Rendering paths

- **CPU** (`spectrum.rs`): both styles render into an **RGBA image** by the floating horizon,
  newest row first, one pass of `depth × columns` — traces set the pixels where a row crosses above
  the horizon, the surface fills each column's newly visible span. Columns = the pane's pixel width
  capped at **512** (tuned) on the CPU, by the peak rule (`render::resample_peak`) so a narrow
  carrier survives. The image is rebuilt only when a new row arrives or the view changes; an idle pan
  costs nothing.
- **GPU** (`app/src/view3d_gpu.rs`, a `Shader` widget): one pipeline, **one triangle-list draw**
  generated from `vertex_index`, reading the waterfall's ring and parameter textures, with a **depth
  attachment** (depth = age): the surface as quads between adjacent rows; traces as thin quads along
  each row plus background-coloured quads below it down to its baseline, so nearer rows hide farther
  ones by depth testing rather than by draw order. The projection constants come from `view3d`.
- **Agreement:** a `#[ignore]`, GPU-required golden test renders both paths **at the same column
  count** and compares away from edges, as the waterfall's does; a reversed vertex mapping fails it.

## 4. FR-UI-27 — the GRAPHICS tab

- **Panadapter view:** a pick-list — *Spectrum + waterfall* (default) / *3D traces* / *3D surface* —
  applied at once.
- **Tilt** and **Depth** sliders (shown for the 3D views), applied at once.
- **Renderer (next start):** Auto / GPU / CPU, saved; the tab says it takes effect at the next start.
- **Status line:** "In use: GPU (wgpu, adapter …) — detected" / "CPU (software) — chosen in
  Settings" / "CPU — no GPU adapter found" / "… — `K4_WATERFALL` override".
- **Applying the renderer at start:** `main` loads the prefs before `iced::daemon` starts (no other
  thread exists yet — edition 2021, so `set_var` is safe there) and sets `ICED_BACKEND`:
  `wgpu,tiny-skia` for GPU (iced tries wgpu, then falls back instead of exiting), `tiny-skia` for
  CPU, unset for Auto — unless the user set `ICED_BACKEND` or `K4_WATERFALL` themselves, which win.
  `gpu_available()` follows the same decision, so the GPU waterfall is never chosen under the
  software renderer (today `K4_WATERFALL=gpu` with tiny-skia draws a blank waterfall).
- **What is in use, honestly:** iced 0.13 does not report its backend. A GPU primitive's `prepare`
  runs only under wgpu, so it sets an `AtomicBool`; the status line reports "GPU (wgpu)" once that is
  set, and "CPU (software)" otherwise, with the reason (detected, chosen in Settings, no adapter
  found, environment override). The adapter name, when shown, comes from the app's own probe and is
  labelled as such.

## 5. Settings (persisted schema)

New `GraphicsPrefs` section on `Prefs`, `#[serde(default)]` on the section, `impl Default`, every
field with an explicit default function (the `default_true` lesson, FR-UI-25):

| Field | Type | Default | Notes |
|---|---|---|---|
| `pan_view` | enum `PanView { Classic, Traces3d, Surface3d }` | `Classic` | serde `snake_case`, `#[serde(other)]` on `Classic` so an unknown value loads as `Classic` instead of failing the whole config |
| `tilt_pct` | u8 | 60 | read clamped 20–90 |
| `depth_rows` | u16 | 64 | read clamped 16–256 |
| `renderer` | enum `Renderer { Auto, Gpu, Cpu }` | `Auto` | `#[serde(other)]` on `Auto` |

## 6. Evidence plan

Tests:
- `view3d`: front row on the bottom, a point's age moves it up and inward, `a = k/(D−1)`, depth
  capped by pixels, tilt/depth bounds; the floating horizon on hand-made rows — a nearer row hides a
  lower farther one, a farther row shows above it, a **gap** after a retune leaves the farther row
  visible; a retuned row shifted by its own centre.
- CPU paths: the incremental waterfall ring equals a full redraw after N rows, and after a retune;
  the 3D image's column count and the peak rule; `min(rows, band px)`.
- GPU: the `#[ignore]` golden for both 3D styles and the re-derived waterfall golden.
- Settings: defaults, an unknown enum value loading as the default, persistence; the renderer
  decision as a pure function of (setting, `ICED_BACKEND`, `K4_WATERFALL`, adapter present) → (the
  `ICED_BACKEND` value to set, `gpu_waterfall`, the reason text).
- Structural: opened maximised; no scrollable encloses the panadapter slot; both slots `Fill`; the
  tab; the view switch.

Sabotage per rule, including: the horizon not updated (farther rows overdraw); a gap raising the
horizon; ages pinned to rows available instead of `D`; `wgpu` without the `tiny-skia` fallback; the
slot back to `Fixed(SCREEN_H)`; the scrollable restored. On screen (`--demo`, Xvfb): both 3D
styles, a retune, the tab, both renderers, the 1320 × 1000 minimum (nothing clipped, pane ≥ 300 px),
and a large virtual screen; the look on DC0SK's 3440 × 1440 is his check (#221).

## 7. Open

- The tuned values (`p`, column cap, age darkening) are first guesses.
- Spot nameplates on the front row only in 3D (v1).
