//! iced `Canvas` rendering of the spectrum trace + waterfall (FR-PAN-02/03).
//!
//! Drawing against a real GPU surface is L4 (visual); the scaling/colour math it
//! relies on lives in `k4_stream::render` and is unit-tested.

use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Pixels, Point, Rectangle, Renderer, Size, Theme};

use crate::worker::PanRow;
use k4_stream::render::{dbm_to_color, dbm_to_y, row_scroll_px};

/// Canvas program drawing a spectrum trace (top) and waterfall (bottom).
pub struct Spectrum<'a, Message> {
    /// Latest trace, downsampled dBm bins.
    pub latest: &'a [f32],
    /// Waterfall rows, newest first, each carrying the pan geometry it was
    /// sampled at so the history can scroll with the VFO (FR-PAN-06).
    pub waterfall: &'a [PanRow],
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
            // select this pane's TX VFO in dual view.
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left))
                if self.span_hz > 0 =>
            {
                let frac = (pos.x / bounds.width).clamp(0.0, 1.0) as f64 - 0.5;
                let hz = (self.center_hz as f64 + frac * self.span_hz as f64).max(0.0) as u64;
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
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h) = (bounds.width, bounds.height);
        let spec_h = h * 0.4;
        let wf_h = h - spec_h;

        frame.fill_rectangle(Point::ORIGIN, Size::new(w, h), Color::from_rgb8(10, 10, 14));

        // dB grid + scale over the spectrum area (drawn under the trace).
        let grid = Color::from_rgba8(255, 255, 255, 0.07);
        let label = Color::from_rgba8(150, 156, 168, 0.85);
        let bottom_dbm = self.top_dbm - self.range_db;
        // Horizontal lines + dB labels every 20 dB, aligned to a 20 dB boundary.
        let step = 20.0_f32;
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
        // Vertical grid lines (quarter divisions) across the spectrum band.
        for i in 1..4 {
            let x = w * i as f32 / 4.0;
            frame.stroke(
                &Path::line(Point::new(x, 0.0), Point::new(x, spec_h)),
                Stroke::default().with_width(1.0).with_color(grid),
            );
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
        if self.latest.len() > 1 {
            let n = self.latest.len();
            let trace = Path::new(|b| {
                for (i, &dbm) in self.latest.iter().enumerate() {
                    let x = i as f32 / (n - 1) as f32 * w;
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

        // Waterfall (newest row at the top of its band).
        //
        // Each row is drawn at the offset its own centre frequency now falls
        // at, so retuning *scrolls* the history sideways and a signal stays on
        // one vertical line instead of smearing across the waterfall
        // (FR-PAN-06). Rows that have scrolled fully off-canvas are skipped,
        // and partially-visible ones are clipped per cell.
        let rows = self.waterfall.len();
        if rows > 0 {
            let row_h = (wf_h / rows as f32).max(1.0);
            let min_db = self.top_dbm - self.range_db;
            for (r, row) in self.waterfall.iter().enumerate() {
                let cols = row.bins.len().max(1);
                // A row sampled at a different span also occupies a different
                // width; scale it so its bins keep their true frequencies.
                let row_w = if self.span_hz > 0 && row.span_hz > 0 {
                    w * row.span_hz as f32 / self.span_hz as f32
                } else {
                    w
                };
                let dx = row_scroll_px(row.center_hz, self.center_hz as i64, self.span_hz, w)
                    - (row_w - w) / 2.0;
                if dx >= w || dx + row_w <= 0.0 {
                    continue; // scrolled out of view
                }
                let col_w = (row_w / cols as f32).max(1.0);
                let y = spec_h + r as f32 * row_h;
                for (c, &dbm) in row.bins.iter().enumerate() {
                    let x = dx + c as f32 * col_w;
                    if x + col_w <= 0.0 || x >= w {
                        continue;
                    }
                    // Clip against the canvas so a scrolled row cannot bleed
                    // outside the pane.
                    let x0 = x.max(0.0);
                    let x1 = (x + col_w + 1.0).min(w);
                    let (cr, cg, cb) = dbm_to_color(dbm, min_db, self.top_dbm);
                    frame.fill_rectangle(
                        Point::new(x0, y),
                        Size::new(x1 - x0, row_h + 1.0),
                        Color::from_rgb8(cr, cg, cb),
                    );
                }
            }
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
                    let x = i as f32 / (n - 1) as f32 * w;
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
