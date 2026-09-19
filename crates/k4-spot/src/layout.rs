//! Where spot nameplates go on the panadapter (FR-SPOT-01/02): pure geometry, so it is testable
//! without a window.
//!
//! A nameplate is a small callsign label plus a **marker** — a tick at the spot's frequency. The
//! marker's x comes from [`spot_x`], the same frequency→column mapping as the trace and the
//! waterfall, so it stays on the signal as the view retunes or the span changes. The label may be
//! moved to keep neighbours from covering each other ([`declutter`]); the marker never is.

/// Height of one label lane, pixels.
pub const LANE_H: f32 = 13.0;
/// Top of the first lane: clear of the span/resolution readout in the corner.
pub const LANES_TOP: f32 = 14.0;
/// Most lanes ever used.
pub const MAX_LANES: usize = 3;
/// Estimated width of one label character, pixels. The label font is proportional, so this is an
/// average; it is deliberately a little generous so the estimate errs towards *not* overlapping.
pub const CHAR_W: f32 = 6.0;
/// Padding either side of the text inside the plate.
pub const PLATE_PAD: f32 = 4.0;
/// Clear space kept between two plates in the same lane.
pub const LABEL_GAP: f32 = 3.0;

/// Estimated plate width for a callsign, pixels.
pub fn label_width(call: &str) -> f32 {
    call.chars().count() as f32 * CHAR_W + 2.0 * PLATE_PAD
}

/// How many label lanes fit in a spectrum band `spectrum_h` pixels tall without taking more than
/// about half of it (the trace needs the rest). 0 means the band is too short to label anything.
pub fn max_lanes(spectrum_h: f32) -> usize {
    let room = spectrum_h * 0.5 - LANES_TOP;
    if room < LANE_H {
        return 0;
    }
    ((room / LANE_H) as usize).min(MAX_LANES)
}

/// Horizontal position of `freq_hz` across a view of `width` pixels centred on `center_hz` and
/// `span_hz` wide — `None` when the frequency lies outside the view.
///
/// This is the mapping the trace and waterfall use: bin `i` of `n` sits at `(i + 0.5) / n * width`
/// (`k4_stream::render::bin_to_x`, FR-PAN-11), which is the frequency `lo + (i + 0.5) / n * span`.
pub fn spot_x(freq_hz: u64, center_hz: u64, span_hz: u32, width: f32) -> Option<f32> {
    if span_hz == 0 || width.is_nan() || width <= 0.0 {
        return None;
    }
    let span = f64::from(span_hz);
    let lo = center_hz as f64 - span / 2.0;
    let frac = (freq_hz as f64 - lo) / span;
    (0.0..=1.0)
        .contains(&frac)
        .then(|| (frac * f64::from(width)) as f32)
}

/// A spot to place: where its marker is, how wide its plate is, and how new it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Item {
    /// Marker x, pixels (from [`spot_x`]).
    pub x: f32,
    /// Plate width, pixels (from [`label_width`]).
    pub label_w: f32,
    /// When the spot was made, Unix seconds — newer spots are placed first.
    pub time: u64,
}

/// One placed plate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// Index into the items handed to [`declutter`].
    pub index: usize,
    /// Which lane, 0 = the top one.
    pub lane: usize,
    /// Left edge of the plate, pixels. The marker stays at the item's `x`.
    pub left: f32,
}

/// The result of laying out a set of plates.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Layout {
    pub placed: Vec<Placed>,
    /// Spots that did not fit in any lane and are shown only as a count.
    pub dropped: usize,
}

/// Stack plates into at most `lanes` lanes so none covers another.
///
/// Newest spots are placed first, each in the first lane where it fits, so when the lanes are full
/// it is the **oldest** that go, and they are counted in [`Layout::dropped`] rather than drawn on
/// top of one another. A plate is centred on its marker but kept inside `[0, width]`; the marker
/// itself is never moved. Deterministic: ties in age break towards the left.
pub fn declutter(items: &[Item], width: f32, lanes: usize) -> Layout {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        items[b]
            .time
            .cmp(&items[a].time)
            .then(items[a].x.total_cmp(&items[b].x))
            .then(a.cmp(&b))
    });
    // The extents already taken in each lane.
    let mut taken: Vec<Vec<(f32, f32)>> = vec![Vec::new(); lanes];
    let mut out = Layout::default();
    for i in order {
        let it = items[i];
        let left = (it.x - it.label_w / 2.0).clamp(0.0, (width - it.label_w).max(0.0));
        let right = left + it.label_w;
        let slot = taken.iter().position(|lane| {
            lane.iter()
                .all(|&(l, r)| right + LABEL_GAP <= l || left >= r + LABEL_GAP)
        });
        match slot {
            Some(lane) => {
                taken[lane].push((left, right));
                out.placed.push(Placed {
                    index: i,
                    lane,
                    left,
                });
            }
            None => out.dropped += 1,
        }
    }
    out.placed
        .sort_by(|a, b| a.left.total_cmp(&b.left).then(a.index.cmp(&b.index)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use k4_stream::render::bin_to_x;

    /// FR-SPOT-01: a spot at the frequency of a trace bin's cell centre lands on exactly the x the
    /// trace draws that bin at; outside the view it draws nothing; and it follows a retune or a
    /// change of span.
    /// trace: FR-SPOT-01
    #[test]
    fn fr_spot_01_marker_follows_frequency() {
        let (center, width) = (14_074_000u64, 1000.0f32);
        // Bin counts chosen so every cell centre is a whole number of hertz.
        for &(span, n) in &[
            (48_000u32, 48usize),
            (48_000, 96),
            (48_000, 480),
            (96_000, 32),
            (24_000, 240),
        ] {
            let lo = center as f64 - span as f64 / 2.0;
            for i in 0..n {
                let hz = lo + (i as f64 + 0.5) / n as f64 * span as f64;
                assert_eq!(
                    hz.fract(),
                    0.0,
                    "test setup: cell centre must be a whole hertz"
                );
                let x = spot_x(hz as u64, center, span, width).expect("inside the view");
                let want = bin_to_x(i, n, width);
                assert!(
                    (x - want).abs() < 1e-2,
                    "span {span} n {n} bin {i}: {x} vs {want}"
                );
            }
        }

        // The edges of the view are the edges of the pane; outside is not drawn.
        assert_eq!(spot_x(center - 24_000, center, 48_000, width), Some(0.0));
        assert_eq!(spot_x(center + 24_000, center, 48_000, width), Some(width));
        assert_eq!(spot_x(center - 24_001, center, 48_000, width), None);
        assert_eq!(spot_x(center + 24_001, center, 48_000, width), None);
        // Unknown span or an empty pane: nothing to place on.
        assert_eq!(spot_x(center, center, 0, width), None);
        assert_eq!(spot_x(center, center, 48_000, 0.0), None);

        // Retune: the marker moves by the retune, in pixels, and the picture scrolls with it.
        let f = center + 6_000;
        let before = spot_x(f, center, 48_000, width).unwrap();
        let after = spot_x(f, center + 1_000, 48_000, width).unwrap();
        assert!((before - after - 1_000.0 / 48_000.0 * width).abs() < 1e-2);
        // Span: halving the span doubles the distance from the centre.
        let wide = spot_x(f, center, 48_000, width).unwrap() - width / 2.0;
        let narrow = spot_x(f, center, 24_000, width).unwrap() - width / 2.0;
        assert!((narrow - 2.0 * wide).abs() < 1e-2);
    }

    fn item(x: f32, w: f32, time: u64) -> Item {
        Item {
            x,
            label_w: w,
            time,
        }
    }

    /// No two plates in a lane may overlap (or touch inside the gap).
    fn assert_no_overlap(items: &[Item], l: &Layout, width: f32) {
        for (i, a) in l.placed.iter().enumerate() {
            let (aw, ar) = (items[a.index].label_w, a.left + items[a.index].label_w);
            assert!(
                a.left >= 0.0 && ar <= width + 1e-3,
                "plate {a:?} leaves the pane"
            );
            for b in l.placed.iter().skip(i + 1).filter(|b| b.lane == a.lane) {
                let (bl, br) = (b.left, b.left + items[b.index].label_w);
                assert!(
                    ar + LABEL_GAP <= bl + 1e-3 || br + LABEL_GAP <= a.left + 1e-3,
                    "plates {a:?} and {b:?} overlap in lane {} (width {aw})",
                    a.lane
                );
            }
        }
    }

    /// FR-SPOT-02: crowded plates are stacked into a bounded number of lanes without overlapping,
    /// the marker is never moved (the plate stays over its marker whenever it fits inside the
    /// pane), the newest spots are placed first so the oldest are the ones dropped, and the
    /// dropped ones are counted.
    /// trace: FR-SPOT-02
    #[test]
    fn fr_spot_02_declutter_lanes_and_overflow() {
        let width = 800.0;

        // Well separated: everything on the top lane, nothing dropped.
        let sparse = [
            item(100.0, 40.0, 10),
            item(300.0, 40.0, 11),
            item(600.0, 40.0, 12),
        ];
        let l = declutter(&sparse, width, 3);
        assert_eq!(l.dropped, 0);
        assert!(l.placed.iter().all(|p| p.lane == 0));
        assert_no_overlap(&sparse, &l, width);
        for p in &l.placed {
            let it = sparse[p.index];
            assert!(
                p.left <= it.x && it.x <= p.left + it.label_w,
                "plate stays over its marker"
            );
        }

        // Five on the same frequency, three lanes: the three newest are placed, the two oldest
        // dropped and counted.
        let pile: Vec<Item> = (0..5).map(|i| item(400.0, 50.0, 100 + i)).collect();
        let l = declutter(&pile, width, 3);
        assert_eq!(l.placed.len(), 3);
        assert_eq!(l.dropped, 2);
        let mut kept: Vec<u64> = l.placed.iter().map(|p| pile[p.index].time).collect();
        kept.sort_unstable();
        assert_eq!(kept, [102, 103, 104], "the oldest are the ones dropped");
        let mut lanes: Vec<usize> = l.placed.iter().map(|p| p.lane).collect();
        lanes.sort_unstable();
        assert_eq!(lanes, [0, 1, 2], "one per lane, none over the budget");
        assert_no_overlap(&pile, &l, width);

        // A plate near an edge is kept inside the pane: it moves, its marker (x) does not.
        let edge = [item(3.0, 50.0, 1), item(797.0, 50.0, 2)];
        let l = declutter(&edge, width, 1);
        assert_eq!(l.dropped, 0);
        for p in &l.placed {
            assert!(p.left >= 0.0 && p.left + 50.0 <= width);
        }
        assert_no_overlap(&edge, &l, width);

        // A dense random-ish set never overlaps and never exceeds its lanes; every spot is either
        // placed or counted.
        let mut s = 12345u64;
        let dense: Vec<Item> = (0..200)
            .map(|i| {
                s = s
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                item(
                    ((s >> 33) % 800) as f32,
                    30.0 + ((s >> 20) % 30) as f32,
                    1000 + i,
                )
            })
            .collect();
        for lanes in [0usize, 1, 2, 3] {
            let l = declutter(&dense, width, lanes);
            assert!(l.placed.iter().all(|p| p.lane < lanes), "lanes {lanes}");
            assert_eq!(
                l.placed.len() + l.dropped,
                dense.len(),
                "every spot accounted for"
            );
            assert_no_overlap(&dense, &l, width);
        }
        assert_eq!(
            declutter(&dense, width, 0).dropped,
            dense.len(),
            "no lanes: all counted"
        );

        // Deterministic.
        assert_eq!(declutter(&dense, width, 3), declutter(&dense, width, 3));

        // Lane budget follows the band height. A lane needs LANES_TOP + LANE_H = 27 px of the
        // half-band the labels may use, so the band must be at least 54 px tall for one lane.
        assert_eq!(max_lanes(50.0), 0, "too short to label anything");
        assert_eq!(max_lanes(53.9), 0);
        assert_eq!(max_lanes(54.0), 1);
        assert_eq!(max_lanes(60.0), 1);
        assert_eq!(max_lanes(100.0), 2);
        assert_eq!(max_lanes(400.0), MAX_LANES);
        // Plate width grows with the callsign.
        assert!(label_width("DL/DC0SK/P") > label_width("K1A"));
    }
}
