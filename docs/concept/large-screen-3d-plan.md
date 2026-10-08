---
title: "Large screens, 3D spectrum and the GRAPHICS tab — design (FR-UI-26, FR-PAN-15, FR-UI-27)"
status: Draft
version: "0.1"
updated: 2026-10-08
authors:
  - Simon Keimer (DC0SK)
---

# Large screens, 3D spectrum and the GRAPHICS tab — design

Three requirements from DC0SK (2026-10-08), with his answers to the open questions: **fit only**
(no UI zoom); **both** 3D styles, selectable; **tilt and depth** sliders; renderer **Auto / GPU /
CPU** for the next start, with the 3D view working on **both** renderers. This note fixes how,
before code. **(tuned)** marks an initial value to revisit on screen.

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

- **Open maximised:** after the main window opens, the existing `WindowOpened` handler issues
  `window::maximize(main_window, true)` once. The window manager sizes it to the screen's free area
  (panels and docks excluded) — the only "free area" figure available, and the right one. The
  `min_size` stays, so a small screen still gets a usable minimum.
- **The panadapter already fills**: the pane is `Length::Fill` in both directions and every other
  panel has a fixed or content height, so a maximised window gives all spare height and width to
  it. Nothing in the layout changes for that; a structural test pins it.
- **History grows with height:** `WATERFALL_ROWS` becomes **256** (tuned). Memory: 256 × 1024 × 4 B
  × 2 receivers = 2 MiB, shared, not copied per tick (`FR-PAN-12`); the GPU ring is 1 MiB per pane.
  Both renderers draw **one history row per row of screen pixels up to the rows available**, and
  stretch only when the pane is taller than the history: a tall pane shows more time, a short one
  the most recent rows. The CPU rasteriser's cost grows with the rows it draws, which is bounded by
  the pane's pixel height, so it does not grow on a small screen. *(Review question: the Pi target,
  `NFR-PORT-02` — is 256 acceptable there, or should the row count follow the pane height at
  runtime?)*

## 3. FR-PAN-15 — the 3D view

### 3.1 Geometry (one pure module, both renderers)

`k4-stream::view3d` (pure, no GPU) owns the projection, so the CPU and GPU paths cannot disagree:

- A point is (frequency fraction `x` ∈ [0,1] across the current view, level fraction `z` ∈ [0,1]
  from the display window `top_dbm`/`range_db`, age `a` ∈ [0,1] from newest to the oldest row shown).
- **Projection:** a fixed oblique-perspective — rows recede **up and inward**: row at age `a` is drawn
  with its baseline at `y0(a) = H·(1 − a·t)` and horizontally compressed toward the centre by
  `s(a) = 1 − a·p`, where `t` is the tilt (fraction of the pane height the history climbs) and `p`
  the perspective shrink (tuned, 0.35). Height: `y = y0(a) − z·h(a)` with `h(a) = h0·s(a)`.
  This is a 2D projection of a 3D scene, chosen over a full camera so that the front row is
  exactly the classic spectrum (`a = 0` → `y0 = H`, `s = 1`) and click-to-QSY needs no ray-cast.
- **Each row keeps its own centre and span:** a row's bins are placed by the same column↔bin rule as
  the waterfall (`render::column_to_bin`), so a retune shifts the history (`FR-PAN-06`).
- **Draw order:** oldest first, newest last (painter's algorithm), so nearer rows cover farther ones.
- **Tilt:** 0.2–0.9 of the pane height, default 0.6 (tuned). **Depth:** 16–256 rows, default 64.
- **Columns drawn:** one per pixel column up to the row's bins on the GPU; on the CPU a reduced
  **192 columns** (tuned) per row by the peak rule (`render::resample_peak`) so a narrow carrier
  survives — `FR-PAN-15` allows fewer columns on the CPU.

### 3.2 Stacked traces

Each row is a polyline; under it, a polygon down to that row's baseline is filled with the
background colour, which is what hides the farther rows behind it (the classic hidden-line trick —
no depth buffer needed). The line is coloured by the row's level through the waterfall's colour map
at its peak, or (simpler, chosen) faded with age from the trace colour to the grid colour.

### 3.3 Shaded surface

Adjacent rows i and i+1 form a strip of quads, drawn back to front; each quad is filled with the
colour map at the mean level of its four corners, darkened slightly with age for depth cue (tuned).
No lighting model: the colour map carries height, as the waterfall does.

### 3.4 Rendering paths

- **GPU** (`app/src/view3d_gpu.rs`, a `Shader` widget beside `waterfall_gpu`): reuses the waterfall's
  ring and parameter textures for the history. A vertex shader generates the grid from
  `vertex_index` (rows × columns), samples the ring for each vertex's level, and applies
  `view3d`'s projection; the same constants are passed as uniforms. Traces: line-strip-per-row plus
  the background fill triangles; surface: triangle strips. Back-to-front order comes from issuing
  rows oldest first — no depth buffer, the same occlusion rule as the CPU path.
- **CPU** (`spectrum.rs`): the same projection through iced `Canvas` paths, 192 columns per row,
  geometry rebuilt only when a new row arrives (`canvas::Cache`), so an idle pan costs nothing.
- **Overlays:** the frequency axis, grid labels, passband overlay and click-to-QSY stay on the
  canvas and act on the **front row**, which is exactly the classic spectrum's geometry. **Spot
  nameplates** are drawn on the front row only (v1).

## 4. FR-UI-27 — the GRAPHICS tab

- **Panadapter view:** a pick-list — *Spectrum + waterfall* (default) / *3D traces* / *3D surface* —
  applied at once.
- **Tilt** and **Depth** sliders (shown for the 3D views), applied at once.
- **Renderer (next start):** Auto / GPU / CPU, saved; the tab says it takes effect at the next start.
- **Status line:** "In use: GPU (wgpu, adapter …) — detected" / "CPU (software) — chosen in
  Settings" / "CPU — no GPU adapter found" / "… — `K4_WATERFALL` override".
- **Applying the renderer at start:** before iced starts (first thing in `main`, single-threaded),
  the saved setting sets `ICED_BACKEND` — `wgpu` for GPU, `tiny-skia` for CPU, left unset for Auto
  — unless the user already set `ICED_BACKEND` or `K4_WATERFALL` in the environment, which win (and
  the status says so). `gpu_available()` then also honours the setting. A GPU choice on a machine
  with no adapter falls back to CPU and says so, never refuses to start.

## 5. Settings (persisted schema)

New `GraphicsPrefs` section on `Prefs`, `#[serde(default)]` on the section, `impl Default`, every
field with an explicit default function (the `default_true` lesson, FR-UI-25):

| Field | Type | Default | Notes |
|---|---|---|---|
| `pan_view` | enum `PanView { Classic, Traces3d, Surface3d }` | `Classic` | serde `snake_case`; an unknown value loads as `Classic` (not a failed config) |
| `tilt_pct` | u8 | 60 | read clamped 20–90 |
| `depth_rows` | u16 | 64 | read clamped 16–256 |
| `renderer` | enum `Renderer { Auto, Gpu, Cpu }` | `Auto` | unknown → `Auto` |

## 6. Evidence plan

Tests: the projection (front row = classic geometry; a point's age moves it up and inward; tilt and
depth bounds) and the draw order (oldest first) in `k4-stream::view3d`; a retuned row shifted by
its own centre; the CPU path's column count and peak rule; the GPU path against the CPU path on an
offscreen target where a pixel is not on an edge (as the waterfall's golden test, `#[ignore]`,
GPU-required); settings defaults (incl. an unknown enum value) and persistence; the renderer
decision table (setting × env overrides × adapter present) as a pure function; structural tests for
"opened maximised", "pane fills", the tab, and the view switch. Sabotage per rule. On screen
(`--demo`, Xvfb): both 3D styles, a retune, the tab, both renderers (`ICED_BACKEND` honoured); the
3440 × 1440 fit is checked on DC0SK's screen (#221) since the virtual display is not his monitor.

## 7. Open, for review

1. `WATERFALL_ROWS` 256 vs a runtime count following the pane height (Pi, §2).
2. The 2D oblique projection (§3.1) vs a real camera: simpler, exact front row; is it a convincing 3D?
3. Hidden-line by background fill (traces) and painter's order (surface) without a depth buffer: any
   case where it draws wrong (rows of different spans after a retune)?
4. Setting `ICED_BACKEND` from inside the process (§4): safe if done before any thread starts? Is
   there a cleaner iced 0.13 hook?
