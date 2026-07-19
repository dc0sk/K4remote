//! Spectrum/waterfall render-helper tests. trace: FR-PAN-02, FR-PAN-03
use k4_stream::render::{dbm_to_color, dbm_to_y, hz_to_x, row_scroll_px};

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
