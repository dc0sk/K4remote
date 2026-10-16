# Patched dependencies

Two iced 0.13.0 crates, copied from crates.io (MIT licence, © the iced authors,
<https://github.com/iced-rs/iced>) and used through `[patch.crates-io]` in the workspace
`Cargo.toml`. They carry upstream fixes that were made on iced's master after 0.13 and
released only in 0.14. Without them the CPU (software) renderer was unusable on the radio
(found by DC0SK on 2026-10-10 and 2026-10-11). Apart from the fixes below, the only differences from the published
crates are the removed `.cargo-ok`, `.cargo_vcs_info.json` and `Cargo.toml.orig`. `iced_graphics/fonts/`
is as published (Fira Sans: SIL Open Font Licence).

| Crate | File | Change | What it fixed |
|---|---|---|---|
| `iced_graphics` 0.13.0 | `src/damage.rs`, `group` | sort the damage regions with `total_cmp`, not `partial_cmp(..).unwrap_or(Equal)` | A centred or right-aligned canvas label has infinite width, so its region's centre is NaN. The old comparator is then no total order, and Rust's sort panics: "user-provided comparison function does not correctly implement a total order", whenever a frequency label changed. |
| `iced_tiny_skia` 0.13.0 | `src/layer.rs`, `draw_primitive_group` / `draw_primitive_cache` | store `clip_bounds * transformation` | A canvas's shapes were clipped to the canvas's own rectangle taken as window coordinates. A pane away from the window's top-left corner lost its trace and grid (pane B entirely), and the damage tracking compared mismatched rectangles (flicker). |
| `iced_tiny_skia` 0.13.0 | `src/engine.rs`, `draw_primitive`, `Primitive::Stroke` | widen the stroke's box by the stroke width before testing it against the layer | A horizontal or vertical line has a zero-height or zero-width box; `Rectangle::intersection` returned none, so the line was taken for off-screen and never drawn: the dB grid, the frequency grid and the spot pins. |
| `iced_tiny_skia` 0.13.0 | `src/engine.rs`, `draw_image`, `Image::Raster` | always clip a raster image to its own bounds within the region being redrawn | The draw rounds an image's window position down to a whole row of the image. With one row of waterfall history (just after start), a row is the whole band tall, so the image could start above the pane. Unclipped, the waterfall's newest row was smeared over the widgets above, outside any redrawn region, so it stayed and flashed as the window's buffers alternated. |

Tests (each fails against the published code and passes against the patched copy):
- `fr_ui_27_the_cpu_renderer_survives_centred_canvas_text` (app/src/graphics.rs): the sort panics
  with the published comparator.
- `fr_ui_27_the_cpu_renderer_draws_the_trace_of_a_pane_away_from_the_corner` (app/src/spectrum.rs):
  pane B's trace has 0 pixels with the published `layer.rs`; the grid has 8 rows and 6 columns
  patched, 1 and 0 with the published stroke test.
- `fr_ui_27_the_cpu_renderer_keeps_the_waterfall_inside_its_pane` (app/src/spectrum.rs): 1–4 rows
  at five pane positions; with the published image draw, one row at y = 410 painted 29 400 pixels
  (49 rows) outside the pane.

All four were confirmed on the radio by DC0SK on 2026-10-11 (CPU renderer, 3440×1440, Wayland).

**Remove this directory and the `[patch.crates-io]` entries** when the app moves to iced 0.14 or later.
