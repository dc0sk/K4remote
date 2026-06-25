//! Pure rendering helpers for the spectrum + waterfall (FR-PAN-02/03).
//!
//! These contain the display math — dBm→pixel scaling and the waterfall
//! colormap — so it is unit-testable independently of any GUI toolkit. The
//! actual canvas drawing (iced) consumes these.

/// Map a dBm value to a vertical pixel coordinate in `[0, height]`, where the
/// top (`y = 0`) corresponds to `top_dbm` and the bottom (`y = height`) to
/// `top_dbm − range_db`. Values outside the window are clamped.
///
/// trace: FR-PAN-02
pub fn dbm_to_y(dbm: f32, top_dbm: f32, range_db: f32, height: f32) -> f32 {
    if range_db <= 0.0 {
        return height;
    }
    let frac = ((top_dbm - dbm) / range_db).clamp(0.0, 1.0);
    frac * height
}

/// Waterfall colormap: map a dBm level within `[min_db, max_db]` to an RGB
/// colour along a black → blue → green → yellow → red gradient. Out-of-range
/// values clamp to the endpoints.
///
/// trace: FR-PAN-03
pub fn dbm_to_color(dbm: f32, min_db: f32, max_db: f32) -> (u8, u8, u8) {
    let t = if max_db <= min_db {
        0.0
    } else {
        ((dbm - min_db) / (max_db - min_db)).clamp(0.0, 1.0)
    };

    const STOPS: [(f32, (u8, u8, u8)); 5] = [
        (0.00, (0, 0, 0)),
        (0.25, (0, 0, 160)),
        (0.50, (0, 180, 100)),
        (0.75, (230, 220, 0)),
        (1.00, (255, 40, 0)),
    ];

    for window in STOPS.windows(2) {
        let (t0, c0) = window[0];
        let (t1, c1) = window[1];
        if t <= t1 {
            let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 };
            let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * f).round() as u8;
            return (lerp(c0.0, c1.0), lerp(c0.1, c1.1), lerp(c0.2, c1.2));
        }
    }
    STOPS[STOPS.len() - 1].1
}
