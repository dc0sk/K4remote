//! The 3D spectrum's geometry and its CPU rasteriser (FR-PAN-15): frequency across, level up,
//! time receding into the background, newest row in front. Pure — the pane size and the rows are
//! passed in — so every rule is tested here. Design: `docs/concept/large-screen-3d-plan.md` v0.2 §3.
//!
//! **Projection** (oblique perspective): row `k` (0 = newest) of a depth `D` has age
//! `a = k / (D − 1)`, pinned to `D` so the scene does not rescale while the history fills. Its
//! baseline sits at `y0 = H − a·t·H` (the front row on the bottom edge), it is narrowed toward the
//! centre by `s = 1 − a·p`, and a level fraction `z` rises `z·h0·s` above the baseline, with
//! `h0 = (1 − t)·H`. `t` is the tilt.
//!
//! **Occlusion** by a floating horizon: rows are drawn newest first, and per screen column only
//! what rises above the highest point drawn so far is visible. With baselines rising monotonically
//! with age and `z ≥ 0` that is exact. A column a row does not cover (a gap after a retune) neither
//! draws nor raises the horizon.

use crate::render::{column_to_bin, dbm_to_color};

/// How much the oldest row is narrowed (tuned).
pub const PERSPECTIVE: f32 = 0.35;

/// Which 3D style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Each row a line; nearer lines hide farther ones.
    Traces,
    /// A continuous surface coloured by level.
    Surface,
}

/// The projection for one pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Proj {
    pub w: f32,
    pub h: f32,
    /// Fraction of the pane height the history climbs, 0.2–0.9.
    pub tilt: f32,
    /// Rows of history the scene is laid out for (`D`).
    pub depth: usize,
}

impl Proj {
    /// Age of row `k`: 0 in front, 1 at the configured depth.
    pub fn age(&self, k: usize) -> f32 {
        if self.depth <= 1 {
            0.0
        } else {
            k as f32 / (self.depth - 1) as f32
        }
    }

    /// The front row's level height: the part of the pane the history does not climb into.
    pub fn h0(&self) -> f32 {
        (1.0 - self.tilt) * self.h
    }

    /// Screen position of (frequency fraction `x` ∈ [0,1] of the view, level fraction `z` ∈ [0,1],
    /// row `k`). The front row (`k = 0`) spans the full width with its baseline on the bottom edge.
    pub fn project(&self, x: f32, z: f32, k: usize) -> (f32, f32) {
        let a = self.age(k);
        let s = 1.0 - a * PERSPECTIVE;
        let sx = self.w * 0.5 + (x - 0.5) * self.w * s;
        let y0 = self.h - a * self.tilt * self.h;
        (sx, y0 - z.clamp(0.0, 1.0) * self.h0() * s)
    }

    /// How many rows to draw: no more than the depth, the rows available, or the pixel rows the
    /// history climbs (`t·H`), and at least one when there is a row.
    pub fn rows_drawn(&self, available: usize) -> usize {
        let px = (self.tilt * self.h).floor().max(1.0) as usize;
        self.depth.min(available).min(px)
    }
}

/// One history row as the 3D view needs it: its bins and the frequencies they were sampled at.
#[derive(Debug, Clone, Copy)]
pub struct Row<'a> {
    pub bins: &'a [f32],
    pub center_hz: i64,
    pub span_hz: u32,
}

/// The level fractions of one row across `cols` columns of the current view (`None` where the row
/// did not sample — a gap): each column through [`column_to_bin`], so a retuned row lands at its own
/// frequencies.
pub fn row_levels(
    row: Row<'_>,
    cols: usize,
    view_center_hz: i64,
    view_span_hz: u32,
    top_dbm: f32,
    range_db: f32,
) -> Vec<Option<f32>> {
    let min_db = top_dbm - range_db;
    (0..cols)
        .map(|c| {
            let bin = column_to_bin(
                c,
                cols,
                view_center_hz,
                view_span_hz,
                row.center_hz,
                row.span_hz,
                row.bins.len(),
            )?;
            let v = row.bins[bin];
            let z = if range_db > 0.0 {
                (v - min_db) / range_db
            } else {
                0.0
            };
            Some(if z.is_finite() {
                z.clamp(0.0, 1.0)
            } else {
                0.0
            })
        })
        .collect()
}

/// Rasterise `levels` (newest row first; each row's level fractions across the same columns) into
/// a `w × h` RGBA image by the floating horizon. Pixels nothing covers stay transparent.
pub fn render_rgba(
    levels: &[Vec<Option<f32>>],
    proj: Proj,
    style: Style,
    top_dbm: f32,
    range_db: f32,
) -> Vec<u8> {
    let (w, h) = (proj.w.max(0.0) as usize, proj.h.max(0.0) as usize);
    let mut img = vec![0u8; w * h * 4];
    if w == 0 || h == 0 {
        return img;
    }
    let n = proj.rows_drawn(levels.len());
    // The highest (smallest y) point drawn so far in each pixel column; `h` = nothing yet.
    let mut horizon = vec![h as f32; w];
    let min_db = top_dbm - range_db;
    for (k, row) in levels.iter().take(n).enumerate() {
        let a = proj.age(k);
        let cols = row.len();
        if cols == 0 {
            continue;
        }
        let x_of = |c: usize| (c as f32 + 0.5) / cols as f32;
        // Columns of this row in screen space, as (screen x, screen y, level).
        let pts: Vec<Option<(f32, f32, f32)>> = row
            .iter()
            .enumerate()
            .map(|(c, z)| {
                z.map(|z| {
                    let (sx, sy) = proj.project(x_of(c), z, k);
                    (sx, sy, z)
                })
            })
            .collect();
        // This row's new horizon, applied after the whole row so its own segments don't hide
        // each other.
        let mut next = horizon.clone();
        for pair in pts.windows(2) {
            let (Some(p0), Some(p1)) = (pair[0], pair[1]) else {
                continue; // a gap: nothing drawn, the horizon untouched
            };
            let (xa, xb) = (p0.0.min(p1.0), p0.0.max(p1.0));
            let x_start = xa.floor().max(0.0) as usize;
            let x_end = (xb.ceil() as usize).min(w);
            for x in x_start..x_end {
                let xc = x as f32 + 0.5;
                let t = if p1.0 > p0.0 {
                    ((xc - p0.0) / (p1.0 - p0.0)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let y = p0.1 + (p1.1 - p0.1) * t;
                let z = p0.2 + (p1.2 - p0.2) * t;
                let top = horizon[x];
                if y >= top {
                    continue; // hidden behind a nearer row
                }
                let colour = match style {
                    Style::Traces => trace_rgb(a),
                    Style::Surface => {
                        let (r, g, b) = dbm_to_color(min_db + z * range_db, min_db, top_dbm);
                        darken((r, g, b), 1.0 - 0.45 * a)
                    }
                };
                let y_top = y.max(0.0) as usize;
                let y_bot = match style {
                    // A line one pixel thick, joined vertically to its neighbour so a steep line
                    // has no holes.
                    Style::Traces => {
                        let y_prev = if xc < p1.0 { p0.1 } else { p1.1 };
                        (y.max(y_prev.min(top)) as usize + 1).min(top.ceil() as usize)
                    }
                    // Everything between this row and what is already drawn in front of it.
                    Style::Surface => top.ceil() as usize,
                }
                .min(h);
                for yy in y_top..y_bot.max(y_top + 1).min(h) {
                    let i = (yy * w + x) * 4;
                    img[i..i + 4].copy_from_slice(&[colour.0, colour.1, colour.2, 0xFF]);
                }
                next[x] = next[x].min(y);
            }
        }
        horizon = next;
    }
    img
}

/// The trace colour, faded toward the grid colour with age.
fn trace_rgb(a: f32) -> (u8, u8, u8) {
    let lerp = |p: f32, q: f32| (p + (q - p) * a.clamp(0.0, 1.0)).round() as u8;
    (lerp(0.0, 60.0), lerp(230.0, 70.0), lerp(120.0, 80.0))
}

fn darken((r, g, b): (u8, u8, u8), f: f32) -> (u8, u8, u8) {
    let m = |c: u8| (f32::from(c) * f.clamp(0.0, 1.0)).round() as u8;
    (m(r), m(g), m(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: Proj = Proj {
        w: 200.0,
        h: 100.0,
        tilt: 0.6,
        depth: 11,
    };

    /// FR-PAN-15: the front row spans the full width on the bottom edge; age moves a point up and
    /// toward the centre; a level rises above its own baseline, less for older rows.
    /// trace: FR-PAN-15
    #[test]
    fn fr_pan_15_projection() {
        assert_eq!(P.project(0.0, 0.0, 0), (0.0, 100.0));
        assert_eq!(P.project(1.0, 0.0, 0), (200.0, 100.0));
        assert_eq!(P.project(0.5, 1.0, 0), (100.0, 100.0 - P.h0()));
        let (x_old, y_old) = P.project(0.0, 0.0, 10);
        assert!(
            (y_old - 40.0).abs() < 1e-4,
            "oldest baseline climbs t·H: {y_old}"
        );
        assert!(
            (x_old - 200.0 * 0.5 * PERSPECTIVE).abs() < 1e-3,
            "and narrows: {x_old}"
        );
        let rise = |k| P.project(0.5, 0.0, k).1 - P.project(0.5, 1.0, k).1;
        assert!(rise(10) < rise(0), "older rows are drawn smaller");
        assert_eq!(P.age(0), 0.0);
        assert_eq!(P.age(10), 1.0);
    }

    /// FR-PAN-15: ages are pinned to the configured depth, so the scene does not rescale while the
    /// history fills; never more rows than the depth, the rows available, or the pixel rows.
    /// trace: FR-PAN-15
    #[test]
    fn fr_pan_15_depth_is_pinned_and_capped() {
        assert_eq!(
            P.age(5),
            0.5,
            "half the depth, half way back — however many rows exist"
        );
        assert_eq!(P.rows_drawn(3), 3);
        assert_eq!(P.rows_drawn(50), 11);
        let deep = Proj { depth: 256, ..P };
        assert_eq!(
            deep.rows_drawn(256),
            60,
            "t·H = 60 pixel rows to put them on"
        );
    }

    fn flat(z: f32, cols: usize) -> Vec<Option<f32>> {
        vec![Some(z); cols]
    }

    fn px(img: &[u8], w: usize, x: usize, y: usize) -> [u8; 4] {
        let i = (y * w + x) * 4;
        [img[i], img[i + 1], img[i + 2], img[i + 3]]
    }

    /// FR-PAN-15: a farther row shows only where it rises above the nearer one. A tall front row
    /// hides a low back row at the centre; the back row's peak above the front row's top shows.
    /// trace: FR-PAN-15
    #[test]
    fn fr_pan_15_nearer_rows_hide_farther_ones() {
        let proj = Proj {
            depth: 2,
            tilt: 0.3,
            ..P
        };
        let cols = 40;
        let front = flat(0.9, cols); // tall, everywhere
        let mut back = flat(0.0, cols);
        back[20] = Some(1.0); // one high peak in the middle
        let img = render_rgba(
            &[front.clone(), back.clone()],
            proj,
            Style::Surface,
            -40.0,
            90.0,
        );
        let w = proj.w as usize;
        // Where the back row is flat (z = 0, baseline 70) the front row (top at 100 − 0.9·70 = 37)
        // covers it: at x = 30, y = 69 is the front row's colour, not the back row's.
        let (_, back_y) = proj.project(30.5 / 200.0, 0.0, 1);
        let y = back_y as usize - 1;
        let front_colour = dbm_to_color(-130.0 + 0.9 * 90.0, -130.0, -40.0);
        assert_eq!(
            &px(&img, w, 30, y)[..3],
            &[front_colour.0, front_colour.1, front_colour.2]
        );
        // Below the back row's baseline, inside the front row: still the front row — a farther row
        // never paints over a nearer one. (x = 60 lies inside the back row's narrowed extent,
        // 35–165 px; at x = 30 the back row draws nothing either way.)
        let (back_left, _) = proj.project(0.0, 0.0, 1);
        assert!(
            back_left < 60.0,
            "x = 60 must lie inside the back row: {back_left}"
        );
        assert_eq!(
            &px(&img, w, 60, back_y as usize + 15)[..3],
            &[front_colour.0, front_colour.1, front_colour.2],
            "the back row overdrew the front row"
        );
        // The back row's peak rises above the front row's top: its pixels just under the peak's tip
        // are drawn, and above the front row's line.
        let (peak_x, peak_y) = proj.project((20.0 + 0.5) / cols as f32, 1.0, 1);
        let (_, front_top) = proj.project(0.5, 0.9, 0);
        assert!(peak_y < front_top, "the peak is above the front row's line");
        // Just above the front row's line, at the peak: the back row's colour, drawn.
        let above = px(&img, w, peak_x as usize, front_top as usize - 2);
        assert_eq!(
            above[3], 0xFF,
            "the back row's peak shows above the front row"
        );
        assert_ne!(
            &above[..3],
            &[front_colour.0, front_colour.1, front_colour.2],
            "and it is not the front row"
        );
        // Above everything: transparent.
        assert_eq!(px(&img, w, 5, 1)[3], 0);
    }

    /// FR-PAN-15: a gap in a nearer row (a part of a retuned row outside its own span) leaves the
    /// farther row visible there instead of hiding it.
    /// trace: FR-PAN-15
    #[test]
    fn fr_pan_15_a_gap_leaves_the_farther_row_visible() {
        let proj = Proj {
            depth: 2,
            tilt: 0.3,
            ..P
        };
        let cols = 40;
        let mut front = flat(0.9, cols);
        front[..20].fill(None); // the left half not sampled by the front row
        let back = flat(0.5, cols);
        let img = render_rgba(&[front, back], proj, Style::Traces, -40.0, 90.0);
        let w = proj.w as usize;
        // The back row's line on the left, where the front row has a gap.
        let (bx, by) = proj.project(10.5 / cols as f32, 0.5, 1);
        let col: Vec<u8> = (0..proj.h as usize)
            .map(|y| px(&img, w, bx as usize, y)[3])
            .collect();
        assert!(
            col[(by as usize).saturating_sub(1)..=(by as usize + 1)].contains(&0xFF),
            "the farther row's line shows through the gap at y≈{by}"
        );
        // And the gap itself draws nothing: below the back row's line that column is empty.
        assert!(
            col[by as usize + 3..].iter().all(|a| *a == 0),
            "the front row drew inside its gap: {:?}",
            &col[by as usize + 3..]
        );
    }

    /// FR-PAN-15: a retuned row is laid out at its own frequencies: a peak sampled 1/4 span higher
    /// lands 1/4 of the width to the right in the level fractions.
    /// trace: FR-PAN-15
    #[test]
    fn fr_pan_15_a_retuned_row_keeps_its_frequencies() {
        let mut bins = vec![-130.0f32; 400];
        bins[200] = -40.0; // peak at the row's own centre
        let row = Row {
            bins: &bins,
            center_hz: 14_074_000 + 12_000,
            span_hz: 48_000,
        };
        let lv = row_levels(row, 400, 14_074_000, 48_000, -40.0, 90.0);
        let peak = lv.iter().position(|z| *z == Some(1.0)).unwrap();
        assert!(
            (295..=305).contains(&peak),
            "peak at column {peak}, expected ~300"
        );
        assert_eq!(lv[50], None, "left of the row's own span: a gap");
    }
}
