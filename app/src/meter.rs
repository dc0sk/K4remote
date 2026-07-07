//! iced `Canvas` for the thin in-panadapter meters: the RX S-meter with a
//! labelled scale (S1–S9, +20/+40/+60 dB) and, during transmit, the TX bar
//! graphs RF / ALC / SWR / CMP (FR-UI-10/15, FR-MTR-03), mirroring the K4 LCD.

use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Pixels, Point, Rectangle, Renderer, Size, Theme};

/// Meter data for one receiver/transmitter pane.
pub struct Meter {
    /// Draw the TX bar graphs instead of the S-meter.
    pub tx: bool,
    /// RX signal, dBm (S-meter). `None` = no reading.
    pub s_dbm: Option<i32>,
    /// TX ALC (bars), CMP (dB), forward power (W), SWR (×0.1).
    pub alc: u16,
    pub cmp: u16,
    pub fwd_w: u16,
    pub swr_x10: u16,
    /// Show the CMP bar (voice modes only).
    pub show_cmp: bool,
}

// S-meter scale endpoints, dBm: S1 = S9 − 8 units × 6 dB; S9+60 dB at the top.
const S1_DBM: f32 = -121.0;
const TOP_DBM: f32 = -13.0; // S9 + 60 dB
const SPAN_DB: f32 = TOP_DBM - S1_DBM; // 108 dB

fn dbm_frac(dbm: f32) -> f32 {
    ((dbm - S1_DBM) / SPAN_DB).clamp(0.0, 1.0)
}

impl<Message> canvas::Program<Message> for Meter {
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
        if self.tx {
            self.draw_tx(&mut frame, w, h);
        } else {
            self.draw_s(&mut frame, w, h);
        }
        vec![frame.into_geometry()]
    }
}

impl Meter {
    fn draw_s(&self, frame: &mut Frame, w: f32, h: f32) {
        let dim = Color::from_rgba8(150, 156, 168, 0.85);
        let track = Color::from_rgba8(255, 255, 255, 0.10);
        let bar_h = (h * 0.34).min(9.0);
        // Track.
        frame.fill_rectangle(Point::new(0.0, 0.0), Size::new(w, bar_h), track);
        // Fill up to the current signal (green, caution-yellow above S9).
        if let Some(dbm) = self.s_dbm {
            let frac = dbm_frac(dbm as f32);
            let strong = dbm as f32 >= -73.0; // S9
            let fill = if strong {
                Color::from_rgb8(0xFF, 0xD4, 0x33)
            } else {
                Color::from_rgb8(0x1E, 0xC8, 0x64)
            };
            frame.fill_rectangle(Point::ORIGIN, Size::new(w * frac, bar_h), fill);
        }
        // Ticks + labels. (dBm, label) — label "" for minor marks.
        let marks: [(f32, &str); 11] = [
            (-121.0, "S1"),
            (-109.0, "S3"),
            (-97.0, "S5"),
            (-85.0, "S7"),
            (-73.0, "S9"),
            (-63.0, ""),
            (-53.0, "+20"),
            (-43.0, ""),
            (-33.0, "+40"),
            (-23.0, ""),
            (-13.0, "+60"),
        ];
        for (dbm, label) in marks {
            let x = (w * dbm_frac(dbm)).clamp(0.5, w - 0.5);
            frame.stroke(
                &Path::line(Point::new(x, 0.0), Point::new(x, bar_h + 2.0)),
                Stroke::default().with_width(1.0).with_color(dim),
            );
            if !label.is_empty() {
                frame.fill_text(Text {
                    content: label.to_string(),
                    position: Point::new((x + 1.0).min(w - 16.0), bar_h + 2.0),
                    color: dim,
                    size: Pixels(8.0),
                    ..Text::default()
                });
            }
        }
    }

    fn draw_tx(&self, frame: &mut Frame, w: f32, _h: f32) {
        let orange = Color::from_rgb8(0xFF, 0x9A, 0x1E);
        let dim = Color::from_rgba8(150, 156, 168, 0.85);
        let track = Color::from_rgba8(255, 255, 255, 0.10);
        // Rows: label, value proportional to its full-scale, readout text.
        let mut rows: Vec<(&str, f32, String)> = vec![
            ("RF", self.fwd_w as f32 / 110.0, format!("{} W", self.fwd_w)),
            ("ALC", self.alc as f32 / 15.0, format!("{}", self.alc)),
            (
                "SWR",
                ((self.swr_x10 as f32 - 10.0) / 40.0).max(0.0),
                format!("{:.1}", self.swr_x10 as f32 / 10.0),
            ),
        ];
        if self.show_cmp {
            rows.push(("CMP", self.cmp as f32 / 30.0, format!("{} dB", self.cmp)));
        }
        let row_h = 14.0_f32;
        let gap = 3.0_f32;
        let label_w = 30.0;
        let val_w = 44.0;
        let bar_x = label_w;
        let bar_w = (w - label_w - val_w).max(10.0);
        for (i, (label, frac, val)) in rows.iter().enumerate() {
            let y = i as f32 * (row_h + gap);
            let bar_y = y + 2.0;
            let bh = row_h - 4.0;
            frame.fill_text(Text {
                content: (*label).to_string(),
                position: Point::new(0.0, y),
                color: dim,
                size: Pixels(9.0),
                ..Text::default()
            });
            frame.fill_rectangle(Point::new(bar_x, bar_y), Size::new(bar_w, bh), track);
            frame.fill_rectangle(
                Point::new(bar_x, bar_y),
                Size::new(bar_w * frac.clamp(0.0, 1.0), bh),
                orange,
            );
            frame.fill_text(Text {
                content: val.clone(),
                position: Point::new(bar_x + bar_w + 4.0, y),
                color: dim,
                size: Pixels(9.0),
                ..Text::default()
            });
        }
    }
}
