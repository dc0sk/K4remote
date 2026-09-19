//! Toolkit-neutral maths for the GPU waterfall (FR-PAN-12).
//!
//! The waterfall is drawn by a fragment shader from a **ring texture**: one texel row per pan row,
//! row `seq % cap` holding the row with sequence number `seq`. Only rows that arrived since the
//! last frame are uploaded. Everything the shader needs that is not pure GPU plumbing lives here,
//! so it can be unit-tested against the reference implementation ([`crate::render::column_to_bin`],
//! [`crate::render::dbm_to_color`]) without a GPU.

use crate::render::dbm_to_color;

/// Entries in the colour lookup table.
pub const LUT_LEN: usize = 256;

/// The waterfall colour map as a 256-entry RGBA table, entry `i` being the colour at normalised
/// level `i / 255`.
///
/// [`dbm_to_color`] depends only on the level's position within `[min, max]`, so one table serves
/// every `#REF`/`#SCL` window: the shader normalises a bin to `t` and indexes this.
pub fn waterfall_lut_rgba() -> Vec<u8> {
    let mut out = Vec::with_capacity(LUT_LEN * 4);
    for i in 0..LUT_LEN {
        let (r, g, b) = dbm_to_color(i as f32 / (LUT_LEN - 1) as f32, 0.0, 1.0);
        out.extend_from_slice(&[r, g, b, 0xFF]);
    }
    out
}

/// The LUT entry for a normalised level `t` in `[0, 1]`: the **nearest** entry, not the floor.
/// Truncating would cost a whole step (about 2.5 counts on the steepest colour segment) where
/// rounding costs half. The WGSL does the same: `i32(t * 255.0 + 0.5)`.
pub fn lut_index(t: f32) -> usize {
    ((t.clamp(0.0, 1.0) * (LUT_LEN - 1) as f32 + 0.5) as usize).min(LUT_LEN - 1)
}

/// One row to copy into the ring texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upload {
    /// Sequence number of the row (the running count of rows ever pushed, minus one for the first).
    pub seq: u64,
    /// Index of the row in the newest-first history slice.
    pub index: usize,
}

/// Which rows still need uploading, **oldest first**.
///
/// * `uploaded` — rows already in the ring (the `total` seen at the last upload),
/// * `total` — rows ever pushed,
/// * `available` — rows currently held in the newest-first history (at most the ring capacity).
///
/// Only rows that are still held can be uploaded, so a long stall uploads at most `available`.
/// `uploaded > total` means the counter went backwards (a reset): everything held is re-uploaded.
pub fn rows_to_upload(uploaded: u64, total: u64, available: usize) -> Vec<Upload> {
    let fresh = if uploaded > total {
        total
    } else {
        total - uploaded
    };
    let take = (fresh.min(available as u64)) as usize;
    (0..take)
        .rev()
        .map(|index| Upload {
            seq: total - 1 - index as u64,
            index,
        })
        .collect()
}

/// Ring row holding the row with sequence number `seq`.
pub fn ring_slot(seq: u64, cap: usize) -> usize {
    (seq % cap as u64) as usize
}

/// Ring row of the **newest** row. `total` must be non-zero.
pub fn head_slot(total: u64, cap: usize) -> usize {
    ring_slot(total.saturating_sub(1), cap)
}

/// How far a row's centre sits from the view centre, Hz.
///
/// Expressed as an offset — a small number — because the shader works in `f32`, which cannot
/// resolve a few hertz at an absolute 50 MHz. The subtraction is done here in integers.
pub fn row_offset_hz(view_center_hz: i64, row_center_hz: i64) -> f32 {
    (row_center_hz - view_center_hz) as f32
}

/// The shader's column→bin lookup, in the `f32` arithmetic the WGSL uses.
///
/// `x` is the horizontal position across the view in `[0, 1]` (the fragment centre, i.e.
/// `(column + 0.5) / columns`). Equivalent to [`crate::render::column_to_bin`]; a test holds the two
/// together. `None` = nothing to draw here.
pub fn shader_bin(
    x: f32,
    view_span_hz: f32,
    row_offset_hz: f32,
    row_span_hz: f32,
    row_bins: u32,
) -> Option<u32> {
    if row_bins == 0 || row_span_hz <= 0.0 || view_span_hz <= 0.0 {
        return None;
    }
    let dx = (x - 0.5) * view_span_hz;
    let frac = (dx - row_offset_hz) / row_span_hz + 0.5;
    if !(0.0..1.0).contains(&frac) {
        return None;
    }
    Some(((frac * row_bins as f32) as u32).min(row_bins - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::column_to_bin;

    /// The LUT is `dbm_to_color` sampled at 256 levels: endpoints exact, and indexing it by the
    /// nearest level never strays more than a few counts from the exact colour.
    /// trace: FR-PAN-12
    #[test]
    fn fr_pan_12_lut_matches_the_reference_colour_map() {
        let lut = waterfall_lut_rgba();
        assert_eq!(lut.len(), LUT_LEN * 4);
        assert_eq!(&lut[0..4], &[0, 0, 0, 255], "bottom of the scale is black");
        assert_eq!(
            &lut[255 * 4..],
            &[255, 40, 0, 255],
            "top of the scale is red"
        );
        for i in 0..LUT_LEN {
            let (r, g, b) = dbm_to_color(i as f32 / 255.0, 0.0, 1.0);
            assert_eq!(
                &lut[i * 4..i * 4 + 3],
                &[r, g, b],
                "entry {i} is the reference colour"
            );
        }
        // A level between two entries: the nearest entry is close to the exact colour.
        let (min_db, top_dbm) = (-130.0f32, -40.0f32);
        let mut worst = 0i32;
        for k in 0..=9000 {
            let dbm = min_db + (top_dbm - min_db) * (k as f32 / 9000.0);
            let exact = dbm_to_color(dbm, min_db, top_dbm);
            let t = ((dbm - min_db) / (top_dbm - min_db)).clamp(0.0, 1.0);
            let idx = lut_index(t); // what the shader does
            let c = &lut[idx * 4..idx * 4 + 3];
            for (a, b) in [(c[0], exact.0), (c[1], exact.1), (c[2], exact.2)] {
                worst = worst.max((a as i32 - b as i32).abs());
            }
        }
        assert!(worst <= 2, "LUT quantisation error {worst} > 2 counts");
    }

    /// Only new rows are uploaded, oldest first, each to its own ring slot; a stall uploads at most
    /// what is still held; a counter that went backwards re-uploads everything held.
    /// trace: FR-PAN-12
    #[test]
    fn fr_pan_12_uploads_only_new_rows_oldest_first() {
        assert!(
            rows_to_upload(10, 10, 10).is_empty(),
            "nothing new, nothing uploaded"
        );
        assert!(rows_to_upload(0, 0, 0).is_empty());

        // Three new rows: seqs 10, 11, 12, oldest first; index 0 is the newest.
        let up = rows_to_upload(10, 13, 13);
        assert_eq!(
            up,
            vec![
                Upload { seq: 10, index: 2 },
                Upload { seq: 11, index: 1 },
                Upload { seq: 12, index: 0 },
            ]
        );

        // A long stall: 1000 rows arrived, only 64 are still held.
        let up = rows_to_upload(5, 1005, 64);
        assert_eq!(up.len(), 64);
        assert_eq!(up.first().unwrap().seq, 1005 - 64);
        assert_eq!(up.last().unwrap().seq, 1004);

        // History cleared then refilled: total 103, uploaded 100, only 3 held.
        assert_eq!(rows_to_upload(100, 103, 3).len(), 3);

        // Counter went backwards (a reset): re-upload what is held.
        assert_eq!(rows_to_upload(500, 7, 7).len(), 7);

        // Ring slots wrap and the head is the newest row's slot.
        assert_eq!(ring_slot(64, 64), 0);
        assert_eq!(ring_slot(65, 64), 1);
        assert_eq!(head_slot(65, 64), 0, "newest is seq 64 -> slot 0");
        assert_eq!(head_slot(1, 64), 0);
        // Every uploaded row lands in a distinct slot while fewer than `cap` are pending.
        let slots: std::collections::HashSet<_> = rows_to_upload(0, 64, 64)
            .iter()
            .map(|u| ring_slot(u.seq, 64))
            .collect();
        assert_eq!(slots.len(), 64);
    }

    /// The offset is done in integers, so it stays exact where an absolute `f32` frequency would not.
    /// trace: FR-PAN-12
    #[test]
    fn fr_pan_12_row_offset_is_exact_at_high_frequency() {
        // 50.313 MHz: f32 cannot tell 50_313_000 from 50_313_003 apart.
        assert_eq!(row_offset_hz(50_313_000, 50_313_003), 3.0);
        assert_eq!(row_offset_hz(50_313_003, 50_313_000), -3.0);
        assert_eq!(row_offset_hz(14_074_000, 14_074_000), 0.0);
        assert_eq!(
            (50_313_003i64 as f32) - (50_313_000i64 as f32),
            4.0,
            "the trap this avoids"
        );
    }

    /// The shader's f32 lookup agrees with the reference `column_to_bin` everywhere except exactly
    /// on a bin or range boundary (where f32 and f64 may round the other way), and even then by at
    /// most one bin. Covers rows sampled at a different centre and span than the view.
    /// trace: FR-PAN-12
    #[test]
    fn fr_pan_12_shader_lookup_matches_column_to_bin() {
        let mut checked = 0u64;
        let mut boundary = 0u64;
        for &view_span in &[6_000u32, 24_000, 48_000, 200_000, 368_000] {
            for &columns in &[300usize, 1024, 1280, 2048] {
                for &(dc, span_mul) in &[
                    (0i64, 1.0f64),
                    (view_span as i64 / 3, 1.0),
                    (-(view_span as i64) / 2, 1.0),
                    (0, 0.5),
                    (0, 2.0),
                    (view_span as i64, 1.0), // wholly off to the side
                ] {
                    for &bins in &[1024usize, 512, 37, 1] {
                        let view_center = 14_074_000i64;
                        let row_center = view_center + dc;
                        let row_span = ((view_span as f64) * span_mul) as u32;
                        for c in 0..columns {
                            let reference = column_to_bin(
                                c,
                                columns,
                                view_center,
                                view_span,
                                row_center,
                                row_span,
                                bins,
                            );
                            let x = (c as f32 + 0.5) / columns as f32;
                            let got = shader_bin(
                                x,
                                view_span as f32,
                                row_offset_hz(view_center, row_center),
                                row_span as f32,
                                bins as u32,
                            );
                            checked += 1;
                            if got.map(|b| b as usize) == reference {
                                continue;
                            }
                            // A disagreement must be a boundary case: within an epsilon of a bin
                            // edge or of the row's range edge, and at most one bin apart.
                            boundary += 1;
                            let hz = (view_center as f64 - view_span as f64 / 2.0)
                                + (c as f64 + 0.5) * view_span as f64 / columns as f64;
                            let frac = (hz - (row_center as f64 - row_span as f64 / 2.0))
                                / row_span as f64;
                            let scaled = frac * bins as f64;
                            let edge = (scaled - scaled.round()).abs()
                                < 1e-3 * (bins as f64).max(1.0)
                                || (frac.abs() < 1e-4)
                                || ((frac - 1.0).abs() < 1e-4);
                            assert!(
                                edge,
                                "unexplained mismatch: col {c}/{columns} span {view_span} row \
                                 dc {dc} x{span_mul} bins {bins}: ref {reference:?} got {got:?} frac {frac}"
                            );
                            if let (Some(a), Some(b)) = (got, reference) {
                                assert!((a as i64 - b as i64).abs() <= 1);
                            }
                        }
                    }
                }
            }
        }
        assert!(
            checked > 100_000,
            "the sweep must be substantial ({checked})"
        );
        assert!(
            boundary * 1000 < checked,
            "boundary disagreements should be rare: {boundary} of {checked}"
        );
    }
}
