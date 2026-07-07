//! iced `Canvas` rendering of the spectrum trace + waterfall (FR-PAN-02/03).
//!
//! Drawing against a real GPU surface is L4 (visual); the scaling/colour math it
//! relies on lives in `k4_stream::render` and is unit-tested.

use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Pixels, Point, Rectangle, Renderer, Size, Theme};

use k4_stream::render::{dbm_to_color, dbm_to_y};

/// Canvas program drawing a spectrum trace (top) and waterfall (bottom).
pub struct Spectrum<'a, Message> {
    /// Latest trace, downsampled dBm bins.
    pub latest: &'a [f32],
    /// Waterfall rows, newest first, downsampled dBm bins.
    pub waterfall: &'a [Vec<f32>],
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
        let rows = self.waterfall.len();
        if rows > 0 {
            let row_h = (wf_h / rows as f32).max(1.0);
            let min_db = self.top_dbm - self.range_db;
            for (r, row) in self.waterfall.iter().enumerate() {
                let cols = row.len().max(1);
                let col_w = (w / cols as f32).max(1.0);
                let y = spec_h + r as f32 * row_h;
                for (c, &dbm) in row.iter().enumerate() {
                    let (cr, cg, cb) = dbm_to_color(dbm, min_db, self.top_dbm);
                    frame.fill_rectangle(
                        Point::new(c as f32 * col_w, y),
                        Size::new(col_w + 1.0, row_h + 1.0),
                        Color::from_rgb8(cr, cg, cb),
                    );
                }
            }
        }

        vec![frame.into_geometry()]
    }
}
