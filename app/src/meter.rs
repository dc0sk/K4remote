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
    /// TX ALC (bars), CMP (dB), forward power (raw `TM` `ccc`, in the units of `pwr_range` — see
    /// [`rf_reading`]), SWR (×0.1).
    pub alc: u16,
    pub cmp: u16,
    pub fwd_w: u16,
    /// The transmit power range from `PC` (`H` QRO / `L` QRP / `X` mW); `None` until known.
    pub pwr_range: Option<char>,
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

/// The RF bar's fill and label for a raw `TM` forward-power value (FR-MTR-03). The field is in
/// the units of the current power range (PRG D12 `TM`: watts in QRO, tenths of a watt in QRP), so
/// it is read against `PC`'s range: `H` in watts on 110 W, `L` in tenths of a watt on 10 W, `X` —
/// an external transverter band, where the radio's RF scale changes to mW (D14 p.79) — in tenths
/// of a milliwatt on 10 mW, the unit `PC` uses for that range (the PRG does not state `TM`'s unit
/// there). An unknown range reads as QRO.
pub fn rf_reading(raw: u16, range: Option<char>) -> (f32, String) {
    let (full, label) = match range {
        Some('L') => (100.0, format!("{:.1} W", f32::from(raw) / 10.0)),
        Some('X') => (100.0, format!("{:.1} mW", f32::from(raw) / 10.0)),
        _ => (110.0, format!("{raw} W")),
    };
    ((f32::from(raw) / full).min(1.0), label)
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
        let (rf_frac, rf_label) = rf_reading(self.fwd_w, self.pwr_range);
        let mut rows: Vec<(&str, f32, String)> = vec![
            ("RF", rf_frac, rf_label),
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

#[cfg(test)]
mod tests {
    use super::rf_reading;

    /// FR-MTR-03: the `TM` forward-power field is in the units of the current power range (PRG
    /// D12 `TM`: "watts in QRO mode, tenths of a watt in QRP mode"), so the RF bar and its label
    /// follow `PC`'s range: QRO in watts on a 110 W scale, QRP in tenths of a watt on 10 W, and
    /// the transverter mW range in tenths of a milliwatt on 10 mW (D14 p.79: on an external
    /// transverter band the RF power scale changes to mW). The same raw `050` reads 50 W, 5.0 W
    /// and 5.0 mW. Scales and units are contract pins, written as numbers.
    /// trace: FR-MTR-03
    #[test]
    fn fr_mtr_03_rf_power_follows_the_power_range() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-6;
        let (f, s) = rf_reading(50, Some('H'));
        assert!(close(f, 50.0 / 110.0) && s == "50 W", "{f} {s}");
        let (f, s) = rf_reading(50, Some('L'));
        assert!(close(f, 0.5) && s == "5.0 W", "{f} {s}");
        let (f, s) = rf_reading(50, Some('X'));
        assert!(close(f, 0.5) && s == "5.0 mW", "{f} {s}");
        // Full scale and the ends of each range.
        assert_eq!(rf_reading(110, Some('H')), (1.0, "110 W".into()));
        assert_eq!(rf_reading(100, Some('L')), (1.0, "10.0 W".into()));
        assert_eq!(rf_reading(100, Some('X')), (1.0, "10.0 mW".into()));
        assert_eq!(rf_reading(0, Some('X')), (0.0, "0.0 mW".into()));
        // A reading past full scale fills the bar, never overflows it; the label keeps the value.
        assert_eq!(rf_reading(999, Some('L')), (1.0, "99.9 W".into()));
        // Range not yet known: read as QRO, as before this change.
        assert_eq!(rf_reading(50, None), rf_reading(50, Some('H')));
    }

    /// FR-MTR-03: the TX meter is given the radio's power range, so the reading above is used with
    /// the range the radio reported. Structural — the needle is in main.rs, not in this file.
    /// trace: FR-MTR-03
    #[test]
    fn fr_mtr_03_the_meter_is_given_the_power_range() {
        let main = include_str!("main.rs");
        assert!(
            main.contains("pwr_range: self.ui.radio.tx_power_range,"),
            "the TX meter is not given the radio's power range"
        );
    }
}
