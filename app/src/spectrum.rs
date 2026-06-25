//! iced `Canvas` rendering of the spectrum trace + waterfall (FR-PAN-02/03).
//!
//! Drawing against a real GPU surface is L4 (visual); the scaling/colour math it
//! relies on lives in `k4_stream::render` and is unit-tested.

use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme};

use k4_stream::render::{dbm_to_color, dbm_to_y};

/// Canvas program drawing a spectrum trace (top) and waterfall (bottom).
pub struct Spectrum<'a> {
    /// Latest trace, downsampled dBm bins.
    pub latest: &'a [f32],
    /// Waterfall rows, newest first, downsampled dBm bins.
    pub waterfall: &'a [Vec<f32>],
    /// dBm at the top of the spectrum window.
    pub top_dbm: f32,
    /// dB span of the spectrum window.
    pub range_db: f32,
}

impl<Message> canvas::Program<Message> for Spectrum<'_> {
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
        let spec_h = h * 0.4;
        let wf_h = h - spec_h;

        frame.fill_rectangle(Point::ORIGIN, Size::new(w, h), Color::from_rgb8(10, 10, 14));

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
