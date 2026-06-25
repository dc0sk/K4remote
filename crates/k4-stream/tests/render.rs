//! Spectrum/waterfall render-helper tests. trace: FR-PAN-02, FR-PAN-03
use k4_stream::render::{dbm_to_color, dbm_to_y};

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
