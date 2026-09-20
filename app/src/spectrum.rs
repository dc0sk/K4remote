//! iced `Canvas` rendering of the spectrum trace + waterfall (FR-PAN-02/03).
//!
//! Drawing against a real GPU surface is L4 (visual); the scaling/colour math it
//! relies on lives in `k4_stream::render` and is unit-tested.

use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::widget::image;
use iced::{Color, Pixels, Point, Rectangle, Renderer, Size, Theme};

use crate::spots::{unix_now, SpotHandle};
use crate::worker::{PanHandle, PanRow};
use k4_stream::render::{
    axis_ticks, bin_to_x, column_to_bin, db_grid_step, dbm_to_color, dbm_to_y, hz_per_bin,
};

/// Upper bound on the rasterised waterfall width, in texels.
///
/// The pane drives the width now, but a texture is uploaded per frame, so this
/// caps the per-frame cost on a very wide (or HiDPI) window. Well past the
/// column count any real pan carries, so it never crops detail in practice.
const MAX_TEXTURE_WIDTH: usize = 2048;

/// Rasterise the waterfall history into an RGBA buffer, `tex_w` texels wide
/// and one row tall per history row (newest first).
///
/// Kept out of `draw` so the pixel maths is unit-testable without a GPU
/// surface: the canvas only uploads the result. Each row is looked up through
/// [`column_to_bin`], so it stays pinned to the absolute frequencies it was
/// sampled at — retuning scrolls the history (FR-PAN-06) and rows sampled at
/// another span map at their own scale. Columns no row covers are left fully
/// transparent.
///
/// trace: FR-PAN-09
pub(crate) fn waterfall_rgba(
    rows: &[PanRow],
    view_center_hz: i64,
    view_span_hz: u32,
    top_dbm: f32,
    range_db: f32,
    tex_w: usize,
) -> Vec<u8> {
    let mut rgba = vec![0u8; tex_w * rows.len() * 4];
    if tex_w == 0 {
        return rgba;
    }
    let min_db = top_dbm - range_db;
    for (r, row) in rows.iter().enumerate() {
        let base = r * tex_w * 4;
        for c in 0..tex_w {
            let Some(bin) = column_to_bin(
                c,
                tex_w,
                view_center_hz,
                view_span_hz,
                row.center_hz,
                row.span_hz,
                row.bins.len(),
            ) else {
                continue; // scrolled out of view → transparent
            };
            let (cr, cg, cb) = dbm_to_color(row.bins[bin], min_db, top_dbm);
            let p = base + c * 4;
            rgba[p] = cr;
            rgba[p + 1] = cg;
            rgba[p + 2] = cb;
            rgba[p + 3] = 0xFF;
        }
    }
    rgba
}

/// Canvas program drawing a spectrum trace (top) and waterfall (bottom).
pub struct Spectrum<'a, Message> {
    /// Latest trace from the UI snapshot: the fallback until the shared history has a row.
    pub latest: &'a [f32],
    /// The shared pan history. The trace is read from its newest row at draw time so it is as
    /// fresh as the waterfall (FR-PAN-13); the CPU waterfall reads its rows from here too,
    /// each carrying the geometry it was sampled at so history scrolls with the VFO (FR-PAN-06).
    pub pan: &'a PanHandle,
    /// Which receiver this pane shows: 0 = main/A, 1 = sub/B.
    pub rx: usize,
    /// The spots to label on the spectrum (FR-SPOT-01/02), and how old one may be and still show
    /// (FR-SPOT-03).
    pub spots: &'a SpotHandle,
    pub spot_max_age_secs: u64,
    /// The waterfall is drawn by the GPU widget under this canvas (FR-PAN-12), so the canvas
    /// must leave that band transparent and skip the CPU rasteriser.
    pub gpu_waterfall: bool,
    /// dBm at the top of the spectrum window.
    pub top_dbm: f32,
    /// dB span of the spectrum window.
    pub range_db: f32,
    /// This pane's VFO (false = A, true = B), passed to the interaction hooks.
    pub is_b: bool,
    /// Pane centre frequency + span (Hz), mapping a click-x to a frequency for
    /// click-to-QSY. `span_hz == 0` disables QSY (span unknown / fixed-tune).
    pub center_hz: u64,
    pub span_hz: u32,
    /// This pane's VFO frequency (Hz), for the carrier line. Equals
    /// `center_hz` when the pan tracks the VFO, but not under `#FXT`.
    pub vfo_hz: u64,
    /// RF passband edges `(lo, hi)` in absolute Hz for the overlay, from
    /// `k4_protocol::cat::rf_passband_hz`. `None` = mode/filter not yet known.
    pub passband_hz: Option<(u64, u64)>,
    /// Left-click → tune this VFO to the clicked frequency.
    pub on_qsy: fn(bool, u64) -> Message,
    /// Wheel scroll → step this VFO up (`+1`) / down (`-1`).
    pub on_wheel: fn(bool, i32) -> Message,
}

/// The spots shown on a pane and where their plates go: the one layout that drawing, hover and
/// click all use, so a click lands on what was drawn (FR-SPOT-10).
struct SpotView {
    spots: Vec<k4_spot::Spot>,
    items: Vec<k4_spot::layout::Item>,
    layout: k4_spot::layout::Layout,
    now: u64,
}

impl<Message> Spectrum<'_, Message> {
    /// Every fresh spot inside the view, and the layout of their plates for a spectrum band
    /// `spec_h` tall and `w` wide. `None` when there is nothing to show or no room for a plate.
    fn spot_view(&self, w: f32, spec_h: f32) -> Option<SpotView> {
        use k4_spot::layout::{declutter, label_width, max_lanes, spot_x, Item};
        let lanes = max_lanes(spec_h);
        if lanes == 0 || self.span_hz == 0 {
            return None;
        }
        let now = unix_now();
        let mut spots = Vec::new();
        let mut items = Vec::new();
        for s in self.spots.lock().ok()?.spots() {
            if !k4_spot::is_fresh(now, s.time, self.spot_max_age_secs) {
                continue;
            }
            let Some(x) = spot_x(s.freq_hz, self.center_hz, self.span_hz, w) else {
                continue;
            };
            items.push(Item {
                x,
                label_w: label_width(&s.call),
                time: s.time,
            });
            spots.push(s.clone());
        }
        if spots.is_empty() {
            return None;
        }
        let layout = declutter(&items, w, lanes);
        Some(SpotView {
            spots,
            items,
            layout,
            now,
        })
    }

    /// The spot whose nameplate is under `pos` (in the pane), if any (FR-SPOT-10).
    fn spot_at(&self, pos: Point, w: f32, spec_h: f32) -> Option<k4_spot::Spot> {
        let view = self.spot_view(w, spec_h)?;
        let i = k4_spot::layout::hit_test(&view.items, &view.layout, pos.x, pos.y)?;
        view.spots.into_iter().nth(i)
    }

    /// Draw a nameplate for every current spot inside the view: a small callsign label in one of a
    /// few lanes at the top of the spectrum band, and a tick down from it to the trace at the
    /// spot's frequency. Labels are moved to keep neighbours from covering each other; the tick
    /// never is, so it always marks the true frequency (FR-SPOT-01/02). Each is coloured for its
    /// source and fades as it ages (FR-SPOT-11), and hovering one shows its details (FR-SPOT-10).
    fn draw_spots(
        &self,
        frame: &mut Frame,
        w: f32,
        spec_h: f32,
        latest: &[f32],
        pointer: Option<Point>,
    ) {
        use k4_spot::layout::{hit_test, LANES_TOP, LANE_H, PLATE_H, PLATE_PAD};
        use k4_spot::style::{age_alpha, source_rgb};
        use k4_spot::tooltip::{tooltip_lines, tooltip_origin, tooltip_size, LINE_H};
        let Some(view) = self.spot_view(w, spec_h) else {
            return;
        };

        let plate = Color::from_rgba8(18, 22, 32, 0.88);
        for p in &view.layout.placed {
            let (spot, it) = (&view.spots[p.index], &view.items[p.index]);
            let (r, g, b) = source_rgb(spot.network);
            let alpha = age_alpha(view.now.saturating_sub(spot.time), self.spot_max_age_secs);
            let y = LANES_TOP + p.lane as f32 * LANE_H;
            // The tick runs from the plate down to the trace at this frequency (never upward, so a
            // strong signal above the labels still gets a short tick rather than a backwards one).
            let n = latest.len();
            let trace_y = if n > 1 {
                let bin = (((it.x / w) * n as f32) as usize).min(n - 1);
                dbm_to_y(latest[bin], self.top_dbm, self.range_db, spec_h)
            } else {
                spec_h
            };
            let top = y + PLATE_H;
            frame.stroke(
                &Path::line(Point::new(it.x, top), Point::new(it.x, trace_y.max(top))),
                Stroke::default()
                    .with_width(1.0)
                    .with_color(Color::from_rgba8(r, g, b, alpha * 0.9)),
            );
            frame.fill_rectangle(Point::new(p.left, y), Size::new(it.label_w, PLATE_H), plate);
            frame.fill_text(Text {
                content: spot.call.clone(),
                position: Point::new(p.left + PLATE_PAD, y + 1.0),
                color: Color::from_rgba8(r, g, b, alpha),
                size: Pixels(9.0),
                ..Text::default()
            });
        }
        if view.layout.dropped > 0 {
            frame.fill_text(Text {
                content: format!("+{}", view.layout.dropped),
                position: Point::new(34.0, 2.0),
                color: Color::from_rgb8(170, 176, 188),
                size: Pixels(9.0),
                ..Text::default()
            });
        }

        // Hover: the details of the spot under the pointer, drawn last so nothing covers them.
        let Some(at) = pointer else { return };
        let Some(i) = hit_test(&view.items, &view.layout, at.x, at.y) else {
            return;
        };
        let spot = &view.spots[i];
        let lines = tooltip_lines(spot, view.now);
        let size = tooltip_size(&lines);
        let (x, y) = tooltip_origin((at.x, at.y), size, (w, spec_h * 2.5));
        let (r, g, b) = source_rgb(spot.network);
        frame.fill_rectangle(
            Point::new(x, y),
            Size::new(size.0, size.1),
            Color::from_rgba8(18, 22, 32, 0.97),
        );
        frame.stroke(
            &Path::rectangle(Point::new(x, y), Size::new(size.0, size.1)),
            Stroke::default()
                .with_width(1.0)
                .with_color(Color::from_rgba8(r, g, b, 0.7)),
        );
        for (n, line) in lines.iter().enumerate() {
            frame.fill_text(Text {
                content: line.clone(),
                position: Point::new(x + PLATE_PAD, y + PLATE_PAD + n as f32 * LINE_H),
                color: if n == 0 {
                    Color::from_rgb8(r, g, b)
                } else {
                    Color::from_rgb8(220, 224, 234)
                },
                size: Pixels(9.0),
                ..Text::default()
            });
        }
    }
}

impl<Message> canvas::Program<Message> for Spectrum<'_, Message> {
    type State = ();

    /// Click-to-QSY + wheel-tuning on the panadapter.
    /// trace: FR-PAN-04
    fn update(
        &self,
        _state: &mut (),
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        let Some(pos) = cursor.position_in(bounds) else {
            return (canvas::event::Status::Ignored, None);
        };
        match event {
            // Click → QSY. Return `Ignored` so a wrapping mouse_area can still
            // select this pane's TX VFO in dual view. A click on a spot's nameplate tunes to that
            // spot's frequency (FR-SPOT-10); anywhere else, to the clicked position. Tuning only:
            // nothing here can key the transmitter.
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.span_hz > 0 =>
            {
                let hz = match self.spot_at(pos, bounds.width, bounds.height * 0.4) {
                    Some(spot) => spot.freq_hz,
                    None => {
                        let frac = (pos.x / bounds.width).clamp(0.0, 1.0) as f64 - 0.5;
                        (self.center_hz as f64 + frac * self.span_hz as f64).max(0.0) as u64
                    }
                };
                (
                    canvas::event::Status::Ignored,
                    Some((self.on_qsy)(self.is_b, hz)),
                )
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                let dir = if y > 0.0 {
                    1
                } else if y < 0.0 {
                    -1
                } else {
                    0
                };
                if dir != 0 {
                    (
                        canvas::event::Status::Captured,
                        Some((self.on_wheel)(self.is_b, dir)),
                    )
                } else {
                    (canvas::event::Status::Ignored, None)
                }
            }
            _ => (canvas::event::Status::Ignored, None),
        }
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let spec_h = h * 0.4;
        let wf_h = h - spec_h;

        // The newest row, at draw time. Falls back to the snapshot's latest trace before the
        // shared history has anything.
        let newest: Vec<f32> = self
            .pan
            .lock()
            .ok()
            .and_then(|p| p.newest(self.rx).map(|r| r.bins.clone()))
            .unwrap_or_else(|| self.latest.to_vec());
        let latest: &[f32] = &newest;

        // In GPU mode the waterfall band belongs to the shader widget underneath, so only the
        // spectrum band gets a background here.
        let bg_h = if self.gpu_waterfall { spec_h } else { h };
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(w, bg_h),
            Color::from_rgb8(10, 10, 14),
        );

        // dB grid + scale over the spectrum area (drawn under the trace).
        let grid = Color::from_rgba8(255, 255, 255, 0.07);
        let label = Color::from_rgba8(150, 156, 168, 0.85);
        let bottom_dbm = self.top_dbm - self.range_db;
        // Horizontal lines + dB labels, on a step chosen for the window so the
        // grid stays readable across the whole `#SCL` 10–150 dB range
        // (FR-PAN-07); aligned to a step boundary.
        let step = db_grid_step(self.range_db);
        let mut db = (self.top_dbm / step).floor() * step;
        while db >= bottom_dbm {
            let y = dbm_to_y(db, self.top_dbm, self.range_db, spec_h);
            frame.stroke(
                &Path::line(Point::new(0.0, y), Point::new(w, y)),
                Stroke::default().with_width(1.0).with_color(grid),
            );
            frame.fill_text(Text {
                content: format!("{db:.0}"),
                position: Point::new(3.0, y + 1.0),
                color: label,
                size: Pixels(9.0),
                ..Text::default()
            });
            db -= step;
        }
        // Vertical grid lines, each labelled with the frequency it marks, so
        // the horizontal resolution is readable off the display rather than
        // being four anonymous divisions (FR-PAN-07).
        const DIVS: u32 = 4;
        if self.span_hz > 0 {
            for (i, hz) in axis_ticks(self.center_hz as i64, self.span_hz, DIVS)
                .into_iter()
                .enumerate()
            {
                let x = w * i as f32 / DIVS as f32;
                // Skip the canvas edges for the line; still label them.
                if i > 0 && i < DIVS as usize {
                    frame.stroke(
                        &Path::line(Point::new(x, 0.0), Point::new(x, spec_h)),
                        Stroke::default().with_width(1.0).with_color(grid),
                    );
                }
                // MHz with enough decimals to resolve one division.
                let text = format!("{:.3}", hz as f64 / 1e6);
                frame.fill_text(Text {
                    content: text,
                    // Nudge the edge labels inward so they stay on-canvas.
                    position: Point::new(x.clamp(20.0, w - 20.0), spec_h - 11.0),
                    color: label,
                    size: Pixels(9.0),
                    horizontal_alignment: iced::alignment::Horizontal::Center,
                    ..Text::default()
                });
            }

            // Resolution readout: span and Hz per displayed column.
            let per_bin = hz_per_bin(self.span_hz, latest.len());
            let span_khz = self.span_hz as f32 / 1000.0;
            frame.fill_text(Text {
                content: if per_bin > 0.0 {
                    format!("{span_khz:.1} kHz span · {per_bin:.0} Hz/bin")
                } else {
                    format!("{span_khz:.1} kHz span")
                },
                position: Point::new(w - 4.0, 2.0),
                color: label,
                size: Pixels(9.0),
                horizontal_alignment: iced::alignment::Horizontal::Right,
                ..Text::default()
            });
        } else {
            for i in 1..DIVS {
                let x = w * i as f32 / DIVS as f32;
                frame.stroke(
                    &Path::line(Point::new(x, 0.0), Point::new(x, spec_h)),
                    Stroke::default().with_width(1.0).with_color(grid),
                );
            }
        }

        // Passband overlay: the *RF* passband, in absolute Hz, as computed by
        // `k4_protocol::cat::rf_passband_hz` — asymmetric about the VFO on
        // USB/LSB/CW, symmetric on AM/FM. Drawn by frequency rather than
        // assumed centred, so it agrees with click-to-QSY by construction.
        // trace: FR-FIL-03, FR-PAN-05
        if let (true, Some((lo, hi))) = (self.span_hz > 0, self.passband_hz) {
            let x_of = |hz: u64| {
                let frac = (hz as f64 - self.center_hz as f64) / self.span_hz as f64 + 0.5;
                (frac as f32).clamp(0.0, 1.0) * w
            };
            let (x_lo, x_hi) = (x_of(lo), x_of(hi));
            if x_hi > x_lo {
                frame.fill_rectangle(
                    Point::new(x_lo, 0.0),
                    Size::new(x_hi - x_lo, spec_h),
                    Color::from_rgba8(0x3D, 0x7E, 0xFF, 0.12),
                );
            }
            // The VFO carrier line, at its real position in the pan: under
            // fixed-tune (`#FXT`) the pan centre and the VFO diverge, so this
            // is not necessarily mid-canvas. For USB/LSB it marks an *edge* of
            // the shaded band rather than its middle.
            let cx = x_of(self.vfo_hz);
            frame.stroke(
                &Path::line(Point::new(cx, 0.0), Point::new(cx, spec_h)),
                Stroke::default()
                    .with_width(1.0)
                    .with_color(Color::from_rgba8(0x3D, 0x7E, 0xFF, 0.5)),
            );
        }

        // Spectrum trace.
        if latest.len() > 1 {
            let n = latest.len();
            let trace = Path::new(|b| {
                for (i, &dbm) in latest.iter().enumerate() {
                    let x = bin_to_x(i, n, w);
                    let y = dbm_to_y(dbm, self.top_dbm, self.range_db, spec_h);
                    if i == 0 {
                        b.move_to(Point::new(x, y));
                    } else {
                        b.line_to(Point::new(x, y));
                    }
                }
            });
            frame.stroke(
                &trace,
                Stroke::default()
                    .with_width(1.0)
                    .with_color(Color::from_rgb8(0, 230, 120)),
            );
        }

        // Spot nameplates over the spectrum (FR-SPOT-01/02).
        self.draw_spots(&mut frame, w, spec_h, latest, cursor.position_in(bounds));

        // Waterfall (newest row at the top of its band), rasterised into one
        // RGBA image and drawn with a single `draw_image`.
        //
        // This used to emit one `fill_rectangle` per bin per row — ~12k quads
        // per pane per frame at 64×192, and the cost scaled with the column
        // count, which is what capped the display at 192 columns (FR-PAN-09).
        // One texture is a fixed cost, so the width can now follow the pane.
        //
        // Geometry is unchanged: `column_to_bin` pins each row to the absolute
        // frequencies it was sampled at, so retuning still *scrolls* the
        // history (FR-PAN-06), rows sampled at another span still map at their
        // own scale, and anything scrolled off-canvas is simply transparent.
        let cpu_rows: Vec<PanRow> = if self.gpu_waterfall {
            Vec::new()
        } else {
            self.pan
                .lock()
                .map(|p| p.rows(self.rx).iter().cloned().collect())
                .unwrap_or_default()
        };
        let rows = cpu_rows.len();
        if rows > 0 && w >= 1.0 && wf_h >= 1.0 && self.span_hz > 0 {
            let tex_w = (w.round() as usize).clamp(1, MAX_TEXTURE_WIDTH);
            let rgba = waterfall_rgba(
                &cpu_rows,
                self.center_hz as i64,
                self.span_hz,
                self.top_dbm,
                self.range_db,
                tex_w,
            );
            // One texel per row: let the GPU stretch it over the waterfall
            // band. `Nearest` keeps the columns crisp rather than smearing
            // adjacent bins together.
            let handle = image::Handle::from_rgba(tex_w as u32, rows as u32, rgba);
            frame.draw_image(
                Rectangle::new(Point::new(0.0, spec_h), Size::new(w, wf_h)),
                canvas::Image::new(handle).filter_method(image::FilterMethod::Nearest),
            );
        }

        vec![frame.into_geometry()]
    }
}

/// Thin mini-pan overview strip (0x03 stream): just a filled trace, no
/// waterfall or grid. trace: FR-UI-14
pub struct MiniPan<'a> {
    pub latest: &'a [f32],
    pub top_dbm: f32,
    pub range_db: f32,
}

impl<Message> canvas::Program<Message> for MiniPan<'_> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        frame.fill_rectangle(Point::ORIGIN, Size::new(w, h), Color::from_rgb8(12, 12, 18));
        if self.latest.len() > 1 {
            let n = self.latest.len();
            let trace = Path::new(|b| {
                for (i, &dbm) in self.latest.iter().enumerate() {
                    let x = bin_to_x(i, n, w);
                    let y = dbm_to_y(dbm, self.top_dbm, self.range_db, h);
                    if i == 0 {
                        b.move_to(Point::new(x, y));
                    } else {
                        b.line_to(Point::new(x, y));
                    }
                }
            });
            frame.stroke(
                &trace,
                Stroke::default()
                    .with_width(1.0)
                    .with_color(Color::from_rgb8(0x5A, 0xC8, 0xFA)),
            );
        }
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row of `bins` dBm values at a given centre/span.
    fn row(center_hz: i64, span_hz: u32, bins: Vec<f32>) -> PanRow {
        PanRow {
            bins,
            center_hz,
            span_hz,
        }
    }

    /// Alpha of texel `(col, r)` — 0 means "nothing drawn here".
    fn alpha(rgba: &[u8], tex_w: usize, col: usize, r: usize) -> u8 {
        rgba[(r * tex_w + col) * 4 + 3]
    }

    /// The buffer is exactly `tex_w × rows × 4` bytes, as `Handle::from_rgba`
    /// requires — a mismatch here would be a renderer-side panic, not a visual
    /// glitch.
    /// trace: FR-PAN-09
    #[test]
    fn fr_pan_09_buffer_is_exactly_the_declared_size() {
        for (rows_n, tex_w) in [(1usize, 1usize), (64, 192), (64, 2048), (7, 800)] {
            let rows: Vec<PanRow> = (0..rows_n)
                .map(|_| row(14_200_000, 50_000, vec![-100.0; 192]))
                .collect();
            let rgba = waterfall_rgba(&rows, 14_200_000, 50_000, -30.0, 100.0, tex_w);
            assert_eq!(rgba.len(), tex_w * rows_n * 4, "{rows_n}×{tex_w}");
        }
        // No rows, or a zero-width texture, yields an empty buffer rather than
        // panicking.
        assert!(waterfall_rgba(&[], 14_200_000, 50_000, -30.0, 100.0, 800).is_empty());
        assert!(waterfall_rgba(
            &[row(14_200_000, 50_000, vec![-100.0; 8])],
            14_200_000,
            50_000,
            -30.0,
            100.0,
            0
        )
        .is_empty());
    }

    /// An aligned row fills its whole scanline; a row retuned by a quarter span
    /// fills only the part still in view, and the rest stays transparent —
    /// the scroll of FR-PAN-06, in pixels.
    /// trace: FR-PAN-09
    #[test]
    fn fr_pan_09_scrolled_row_is_partly_transparent() {
        let tex_w = 800;
        let rows = vec![
            row(14_200_000, 50_000, vec![-60.0; 400]), // aligned with the view
            row(14_187_500, 50_000, vec![-60.0; 400]), // captured a quarter-span down
        ];
        let rgba = waterfall_rgba(&rows, 14_200_000, 50_000, -30.0, 100.0, tex_w);

        // Row 0 is fully painted.
        assert!((0..tex_w).all(|c| alpha(&rgba, tex_w, c, 0) == 0xFF));

        // Row 1 sat a quarter-span lower, so it now covers only the left
        // three-quarters of the view.
        let painted = (0..tex_w)
            .filter(|&c| alpha(&rgba, tex_w, c, 1) == 0xFF)
            .count();
        assert!(
            (595..=605).contains(&painted),
            "expected ~3/4 of {tex_w} painted, got {painted}"
        );
        assert_eq!(alpha(&rgba, tex_w, 0, 1), 0xFF, "left edge still in view");
        assert_eq!(alpha(&rgba, tex_w, 799, 1), 0, "right edge scrolled off");
    }

    /// A row scrolled a full span away contributes nothing at all.
    /// trace: FR-PAN-09
    #[test]
    fn fr_pan_09_fully_scrolled_row_paints_nothing() {
        let tex_w = 256;
        let rows = vec![row(14_200_000, 50_000, vec![-60.0; 192])];
        let rgba = waterfall_rgba(&rows, 14_260_000, 50_000, -30.0, 100.0, tex_w);
        assert!((0..tex_w).all(|c| alpha(&rgba, tex_w, c, 0) == 0));
    }

    /// Levels map through the colormap: a hot bin is not the same colour as
    /// the noise floor, and the window (`#REF`/`#SCL`) is honoured.
    /// trace: FR-PAN-09
    #[test]
    fn fr_pan_09_levels_map_through_the_colormap() {
        let tex_w = 4;
        let rows = vec![row(14_200_000, 50_000, vec![-120.0, -120.0, -40.0, -40.0])];
        let rgba = waterfall_rgba(&rows, 14_200_000, 50_000, -30.0, 100.0, tex_w);
        let texel = |c: usize| &rgba[c * 4..c * 4 + 3];
        assert_eq!(texel(0), texel(1), "equal levels → equal colour");
        assert_ne!(texel(0), texel(3), "a hot bin must differ from the floor");
        // The same bins under a narrower window render differently.
        let tight = waterfall_rgba(&rows, 14_200_000, 50_000, -30.0, 20.0, tex_w);
        assert_ne!(
            &rgba[0..3],
            &tight[0..3],
            "changing #REF/#SCL must change the pixels"
        );
    }

    /// Widening the texture past the bin count must not read out of bounds or
    /// leave gaps: every column of an aligned row is painted.
    /// trace: FR-PAN-09
    #[test]
    fn fr_pan_09_texture_wider_than_the_bins_is_gapless() {
        let tex_w = 2048;
        let rows = vec![row(14_200_000, 50_000, vec![-60.0; 17])];
        let rgba = waterfall_rgba(&rows, 14_200_000, 50_000, -30.0, 100.0, tex_w);
        assert!((0..tex_w).all(|c| alpha(&rgba, tex_w, c, 0) == 0xFF));
    }
}

#[cfg(test)]
mod spot_click_tests {
    use super::*;
    use crate::spots::unix_now;
    use crate::worker::PanShared;
    use iced::widget::canvas::Program;
    use k4_spot::{Network, Spot, SpotStore};
    use std::sync::{Arc, Mutex};

    const CENTER: u64 = 14_074_000;
    const SPAN: u32 = 48_000;
    const W: f32 = 1000.0;
    const H: f32 = 300.0;

    fn qsy(is_b: bool, hz: u64) -> (bool, u64) {
        (is_b, hz)
    }
    fn wheel(is_b: bool, dir: i32) -> (bool, u64) {
        (is_b, dir as u64)
    }

    fn store(spots: &[(&str, u64, u64)]) -> SpotHandle {
        let mut s = SpotStore::default();
        for (call, freq, age) in spots {
            s.insert(Spot::new(call, *freq, unix_now() - age, Network::Rbn).unwrap());
        }
        Arc::new(Mutex::new(s))
    }

    fn with<R>(spots: &SpotHandle, f: impl FnOnce(&Spectrum<'_, (bool, u64)>) -> R) -> R {
        let pan: PanHandle = Arc::new(Mutex::new(PanShared::default()));
        let sp = Spectrum {
            latest: &[],
            pan: &pan,
            rx: 0,
            gpu_waterfall: true,
            spots,
            spot_max_age_secs: 900,
            top_dbm: -40.0,
            range_db: 90.0,
            is_b: false,
            center_hz: CENTER,
            span_hz: SPAN,
            vfo_hz: CENTER,
            passband_hz: None,
            on_qsy: qsy,
            on_wheel: wheel,
        };
        f(&sp)
    }

    fn click(sp: &Spectrum<'_, (bool, u64)>, x: f32, y: f32) -> Option<(bool, u64)> {
        let bounds = Rectangle::new(Point::ORIGIN, Size::new(W, H));
        let (_, msg) = sp.update(
            &mut (),
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            bounds,
            mouse::Cursor::Available(Point::new(x, y)),
        );
        msg
    }

    /// What a click at `x` tunes to when it is not on a plate: the position under the pointer.
    fn position_hz(x: f32) -> u64 {
        let frac = f64::from(x / W).clamp(0.0, 1.0) - 0.5;
        (CENTER as f64 + frac * f64::from(SPAN)) as u64
    }

    /// FR-SPOT-10: a click on a nameplate tunes to that spot's own frequency, not to wherever on the
    /// plate the pointer happened to be; a click anywhere else still tunes to the pointer's position;
    /// a spot that has aged out or lies outside the view cannot be clicked.
    /// trace: FR-SPOT-10
    #[test]
    fn fr_spot_10_click_on_a_nameplate_tunes_to_the_spot() {
        let near = CENTER + 10_000;
        let far = CENTER - 20_000;
        let store = store(&[
            ("K1AAA", near, 30),
            ("K2BBB", far, 60),
            ("K3OLD", CENTER + 2_000, 3_600), // older than the 900 s limit
            ("K4OUT", CENTER + 90_000, 10),   // outside the view
        ]);
        with(&store, |sp| {
            let spec_h = H * 0.4;
            let view = sp.spot_view(W, spec_h).expect("two spots are shown");
            assert_eq!(
                view.spots.len(),
                2,
                "the old and the out-of-view spot are not shown"
            );
            let plate = |call: &str| {
                let p = view
                    .layout
                    .placed
                    .iter()
                    .find(|p| view.spots[p.index].call == call)
                    .expect("placed");
                let it = view.items[p.index];
                let y = k4_spot::layout::LANES_TOP + p.lane as f32 * k4_spot::layout::LANE_H;
                (p.left, it.label_w, y, it.x)
            };

            // Click near the LEFT edge of K1AAA's plate: the pointer's position is nowhere near the
            // spot's frequency, yet the click tunes to the spot exactly.
            let (left, w_plate, y, marker_x) = plate("K1AAA");
            let (cx, cy) = (left + 1.0, y + 3.0);
            assert_ne!(
                position_hz(cx),
                near,
                "test setup: the pointer is not on the frequency"
            );
            assert_eq!(click(sp, cx, cy), Some((false, near)));
            // …and near the right edge.
            assert_eq!(click(sp, left + w_plate - 1.0, cy), Some((false, near)));
            assert!(
                (marker_x - (left + w_plate / 2.0)).abs() <= w_plate,
                "the plate is by its marker"
            );

            // The other spot's plate tunes to the other spot's frequency.
            let (l2, _, y2, _) = plate("K2BBB");
            assert_eq!(click(sp, l2 + 2.0, y2 + 3.0), Some((false, far)));

            // Just past the plate, and over the trace: the pointer's own position.
            assert_eq!(
                click(sp, left + w_plate + 2.0, cy),
                Some((false, position_hz(left + w_plate + 2.0)))
            );
            assert_eq!(click(sp, cx, 200.0), Some((false, position_hz(cx))));
            // Where the OLD spot's plate would have been (2 kHz right of centre): no plate, so the
            // click tunes to the position, and to nothing else.
            let old_x = W * (0.5 + 2_000.0 / SPAN as f32);
            assert_eq!(click(sp, old_x, 20.0), Some((false, position_hz(old_x))));
        });
    }

    /// FR-SPOT-10: the click path only tunes. It returns the tune message for the pane it belongs to
    /// and never anything else; a right click and a click outside the pane do nothing.
    /// trace: FR-SPOT-10
    #[test]
    fn fr_spot_10_only_a_left_click_inside_the_pane_tunes() {
        let store = store(&[("K1AAA", CENTER + 10_000, 30)]);
        with(&store, |sp| {
            let bounds = Rectangle::new(Point::ORIGIN, Size::new(W, H));
            let right = sp.update(
                &mut (),
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
                bounds,
                mouse::Cursor::Available(Point::new(500.0, 20.0)),
            );
            assert_eq!(right.1, None, "a right click does nothing");
            let outside = sp.update(
                &mut (),
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Available(Point::new(W + 50.0, 20.0)),
            );
            assert_eq!(outside.1, None, "a click outside the pane does nothing");
            let unavailable = sp.update(
                &mut (),
                canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                bounds,
                mouse::Cursor::Unavailable,
            );
            assert_eq!(unavailable.1, None);
            // With no span known there is nothing to map a click to.
            let no_span = Spectrum {
                span_hz: 0,
                ..clone_view(sp)
            };
            assert_eq!(click(&no_span, 500.0, 20.0), None);
        });
    }

    /// A copy of the program with the same borrows, for a variation.
    fn clone_view<'a>(sp: &Spectrum<'a, (bool, u64)>) -> Spectrum<'a, (bool, u64)> {
        Spectrum {
            latest: sp.latest,
            pan: sp.pan,
            rx: sp.rx,
            gpu_waterfall: sp.gpu_waterfall,
            spots: sp.spots,
            spot_max_age_secs: sp.spot_max_age_secs,
            top_dbm: sp.top_dbm,
            range_db: sp.range_db,
            is_b: sp.is_b,
            center_hz: sp.center_hz,
            span_hz: sp.span_hz,
            vfo_hz: sp.vfo_hz,
            passband_hz: sp.passband_hz,
            on_qsy: sp.on_qsy,
            on_wheel: sp.on_wheel,
        }
    }
}
