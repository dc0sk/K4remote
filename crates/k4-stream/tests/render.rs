//! Spectrum/waterfall render-helper tests. trace: FR-PAN-02, FR-PAN-03
use k4_stream::render::{
    axis_ticks, db_grid_step, dbm_to_color, dbm_to_y, hz_per_bin, hz_to_x, pan_window,
    row_scroll_px,
};

/// dBm→y maps the top of the window to 0 and the bottom to `height`, clamping.
///
/// trace: FR-PAN-02
#[test]
fn fr_pan_02_dbm_to_y_scaling() {
    let (top, range, h) = (-30.0, 100.0, 200.0);
    assert_eq!(dbm_to_y(-30.0, top, range, h), 0.0); // top
    assert_eq!(dbm_to_y(-130.0, top, range, h), 200.0); // bottom
    assert_eq!(dbm_to_y(-80.0, top, range, h), 100.0); // middle
    assert_eq!(dbm_to_y(0.0, top, range, h), 0.0); // above top → clamped
    assert_eq!(dbm_to_y(-200.0, top, range, h), 200.0); // below bottom → clamped
}

/// Colormap clamps to its endpoints and interpolates between stops.
///
/// trace: FR-PAN-03
#[test]
fn fr_pan_03_colormap_endpoints_and_midpoint() {
    assert_eq!(dbm_to_color(-130.0, -120.0, -20.0), (0, 0, 0)); // ≤ min → black
    assert_eq!(dbm_to_color(0.0, -120.0, -20.0), (255, 40, 0)); // ≥ max → red
                                                                // midpoint (t = 0.5) hits the green stop exactly
    assert_eq!(dbm_to_color(-70.0, -120.0, -20.0), (0, 180, 100));
}

/// A degenerate range does not panic and yields the low end.
///
/// trace: FR-PAN-03
#[test]
fn fr_pan_03_colormap_degenerate_range() {
    assert_eq!(dbm_to_color(-50.0, -50.0, -50.0), (0, 0, 0));
}

/// A frequency at the view centre maps to mid-canvas; the span edges map to
/// the canvas edges; a zero span degenerates to the centre.
/// trace: FR-PAN-06
#[test]
fn fr_pan_06_hz_to_x_maps_span_across_the_canvas() {
    let (c, span, w) = (14_200_000.0, 50_000, 800.0);
    assert_eq!(hz_to_x(14_200_000.0, c, span, w), 400.0);
    assert_eq!(hz_to_x(14_175_000.0, c, span, w), 0.0); // centre − span/2
    assert_eq!(hz_to_x(14_225_000.0, c, span, w), 800.0); // centre + span/2
    assert_eq!(hz_to_x(14_187_500.0, c, span, w), 200.0); // quarter point
    assert_eq!(hz_to_x(1.0, c, 0, w), 400.0); // span unknown → centre
}

/// Waterfall rows are pinned to the frequencies they were sampled at, so
/// retuning slides the history sideways rather than smearing it.
/// trace: FR-PAN-06
#[test]
fn fr_pan_06_rows_scroll_by_the_retune_delta() {
    let (span, w) = (50_000, 800.0);
    // Row captured at the current centre sits unshifted.
    assert_eq!(row_scroll_px(14_200_000, 14_200_000, span, w), 0.0);
    // Tuning *up* 12.5 kHz (a quarter span) pushes older rows LEFT by w/4:
    // their content is now below the new centre.
    assert_eq!(row_scroll_px(14_200_000, 14_212_500, span, w), -200.0);
    // Tuning down moves them right, symmetrically.
    assert_eq!(row_scroll_px(14_200_000, 14_187_500, span, w), 200.0);
    // A retune of a full span puts the row exactly one canvas off-screen.
    assert_eq!(row_scroll_px(14_200_000, 14_250_000, span, w), -800.0);
    // Unknown span cannot be scrolled; fall back to no shift.
    assert_eq!(row_scroll_px(14_200_000, 14_250_000, 0, w), 0.0);
}

/// A row's scroll offset agrees with `hz_to_x` for the same frequency: both
/// place the row's centre where that frequency now falls.
/// trace: FR-PAN-06
#[test]
fn fr_pan_06_scroll_agrees_with_hz_to_x() {
    let (span, w) = (50_000, 800.0);
    let (row_c, view_c) = (14_195_000_i64, 14_200_000_i64);
    let via_scroll = w / 2.0 + row_scroll_px(row_c, view_c, span, w);
    let via_map = hz_to_x(row_c as f64, view_c as f64, span, w);
    assert!(
        (via_scroll - via_map).abs() < 1e-3,
        "{via_scroll} vs {via_map}"
    );
}

/// trace: FR-PAN-07
#[test]
fn fr_pan_07_pan_window_from_ref_and_scale() {
    assert_eq!(pan_window(-130, 70), (-60.0, 70.0)); // −130…−60 dBm
    assert_eq!(pan_window(-100, 50), (-50.0, 50.0));
    assert_eq!(pan_window(-130, 0), (-120.0, 10.0)); // degenerate → #SCL min
                                                     // The window is exactly `scale` tall, whatever the reference.
    for (r, sc) in [(-200i16, 10u16), (-130, 70), (0, 150), (60, 150)] {
        let (top, range) = pan_window(r, sc);
        assert_eq!(range, f32::from(sc));
        assert_eq!(top - range, f32::from(r), "bottom must equal #REF");
    }
}

/// The dB grid adapts to the window instead of a fixed 20 dB step, which drew
/// a single line at a 10 dB scale and eight at 150 dB.
/// trace: FR-PAN-07
#[test]
fn fr_pan_07_db_grid_step_adapts_to_range() {
    assert_eq!(db_grid_step(10.0), 2.0);
    assert_eq!(db_grid_step(70.0), 20.0);
    assert_eq!(db_grid_step(150.0), 50.0);
    // Across the whole documented #SCL range, division count stays readable.
    for scale in 10..=150u16 {
        let (_, range) = pan_window(-130, scale);
        let n = range / db_grid_step(range);
        assert!((1.0..=8.0).contains(&n), "scale {scale} → {n} divisions");
    }
}

/// Axis ticks span the full view inclusive of both edges and stay centred.
/// trace: FR-PAN-07
#[test]
fn fr_pan_07_axis_ticks_span_the_view() {
    let t = axis_ticks(14_200_000, 50_000, 4);
    assert_eq!(
        t,
        vec![14_175_000, 14_187_500, 14_200_000, 14_212_500, 14_225_000]
    );
    assert_eq!(t.len(), 5); // divisions + 1
    assert_eq!(axis_ticks(14_200_000, 50_000, 0), Vec::<i64>::new());
    // Ticks agree with the frequency→pixel mapping used to place them.
    let (span, w) = (50_000u32, 800.0f32);
    for (i, &hz) in t.iter().enumerate() {
        let x = hz_to_x(hz as f64, 14_200_000.0, span, w);
        assert!((x - i as f32 * w / 4.0).abs() < 1e-3, "tick {i} at {x}");
    }
}

/// Display resolution is span / displayed columns.
/// trace: FR-PAN-07
#[test]
fn fr_pan_07_hz_per_bin() {
    assert_eq!(hz_per_bin(50_000, 192), 50_000.0 / 192.0);
    assert_eq!(hz_per_bin(6_000, 192), 6_000.0 / 192.0);
    assert_eq!(hz_per_bin(50_000, 0), 0.0); // no bins yet
                                            // Halving the span halves the Hz each column covers (finer resolution).
    assert_eq!(hz_per_bin(25_000, 192) * 2.0, hz_per_bin(50_000, 192));
}
