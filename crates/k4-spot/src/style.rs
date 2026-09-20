//! How a nameplate looks (FR-SPOT-11): a colour per source, a fade with age, and the arithmetic
//! that checks both stay legible. Pure, so the choices are testable, not just eyeballed.
//!
//! The panadapter's spectrum band is drawn on a fixed dark background whatever the theme is (the
//! canvas fills it with [`SPECTRUM_BG`]), so legibility is checked against that background and the
//! plate's, not against the theme.

use crate::Network;

/// The spectrum band's background, as the canvas paints it in every theme.
pub const SPECTRUM_BG: (u8, u8, u8) = (10, 10, 14);

/// A nameplate's background (drawn translucent over the spectrum).
pub const PLATE_BG: (u8, u8, u8) = (18, 22, 32);

/// The faintest a spot is drawn, at the age limit. Below this the older spots stop being readable.
pub const FADE_FLOOR: f32 = 0.5;

/// The colour that identifies a source: its tick and its callsign text.
///
/// Chosen to differ in hue and in lightness, so they stay apart for viewers who cannot tell red
/// from green, and to be light enough to read on a dark plate.
pub fn source_rgb(network: Network) -> (u8, u8, u8) {
    match network {
        // Skimmer spots: amber.
        Network::Rbn => (255, 200, 80),
        // Reception reports: sky blue.
        Network::PskReporter => (120, 200, 255),
        // Human spots from a DX cluster: lilac.
        Network::DxCluster => (206, 166, 255),
    }
}

/// How opaque a spot of `age_secs` is drawn, given the age limit: 1.0 when fresh, falling in a
/// straight line to [`FADE_FLOOR`] at the limit and staying there beyond it. Never rises with age.
pub fn age_alpha(age_secs: u64, max_age_secs: u64) -> f32 {
    if max_age_secs == 0 {
        return 1.0;
    }
    let f = (age_secs as f32 / max_age_secs as f32).clamp(0.0, 1.0);
    1.0 - (1.0 - FADE_FLOOR) * f
}

/// `fg` drawn at `alpha` over `bg`.
pub fn blend(fg: (u8, u8, u8), bg: (u8, u8, u8), alpha: f32) -> (u8, u8, u8) {
    let mix = |f: u8, b: u8| (f32::from(f) * alpha + f32::from(b) * (1.0 - alpha)).round() as u8;
    (mix(fg.0, bg.0), mix(fg.1, bg.1), mix(fg.2, bg.2))
}

/// Relative luminance of an sRGB colour, as WCAG 2 defines it.
pub fn luminance(c: (u8, u8, u8)) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.0) + 0.7152 * lin(c.1) + 0.0722 * lin(c.2)
}

/// WCAG contrast ratio between two colours, from 1 (identical) to 21 (black on white).
pub fn contrast(a: (u8, u8, u8), b: (u8, u8, u8)) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCES: [Network; 3] = [Network::Rbn, Network::PskReporter, Network::DxCluster];

    /// FR-SPOT-11: the fade starts at fully opaque, never rises as a spot ages, reaches its floor at
    /// the age limit and stays there, and a zero limit means no fade.
    /// trace: FR-SPOT-11
    #[test]
    fn fr_spot_11_age_fade_is_monotonic_and_floored() {
        let max = 900;
        assert_eq!(age_alpha(0, max), 1.0);
        assert!((age_alpha(max, max) - FADE_FLOOR).abs() < 1e-6);
        assert!(
            (age_alpha(max * 10, max) - FADE_FLOOR).abs() < 1e-6,
            "beyond the limit stays at the floor"
        );
        assert!(
            (age_alpha(max / 2, max) - (1.0 + FADE_FLOOR) / 2.0).abs() < 1e-6,
            "linear in between"
        );
        let mut prev = 2.0f32;
        for age in 0..=max + 100 {
            let a = age_alpha(age, max);
            assert!(a <= prev, "the fade must never rise: age {age}");
            assert!((FADE_FLOOR..=1.0).contains(&a));
            prev = a;
        }
        assert_eq!(age_alpha(500, 0), 1.0, "no age limit, no fade");
    }

    /// FR-SPOT-11: each source has its own colour, well apart from the others, and every colour is
    /// legible on the plate at full strength and at its faintest.
    /// trace: FR-SPOT-11
    #[test]
    fn fr_spot_11_source_colours_are_distinct_and_legible() {
        for (i, a) in SOURCES.iter().enumerate() {
            for b in &SOURCES[i + 1..] {
                let (ca, cb) = (source_rgb(*a), source_rgb(*b));
                let d = |x: u8, y: u8| f64::from(x) - f64::from(y);
                let dist =
                    (d(ca.0, cb.0).powi(2) + d(ca.1, cb.1).powi(2) + d(ca.2, cb.2).powi(2)).sqrt();
                assert!(dist > 80.0, "{a:?} and {b:?} are too alike ({dist:.0})");
            }
        }
        for n in SOURCES {
            let c = source_rgb(n);
            // Text on the plate: WCAG AA for normal text is 4.5; fresh spots are far above it.
            assert!(
                contrast(c, PLATE_BG) >= 7.0,
                "{n:?} fresh on the plate: {:.1}",
                contrast(c, PLATE_BG)
            );
            // The faintest spot, drawn at the fade floor over the plate, is still readable (3:1).
            let faint = blend(c, PLATE_BG, FADE_FLOOR);
            assert!(
                contrast(faint, PLATE_BG) >= 3.0,
                "{n:?} at the floor: {:.1}",
                contrast(faint, PLATE_BG)
            );
            // And its tick, over the bare spectrum background.
            let tick = blend(c, SPECTRUM_BG, FADE_FLOOR);
            assert!(
                contrast(tick, SPECTRUM_BG) >= 3.0,
                "{n:?} tick at the floor: {:.1}",
                contrast(tick, SPECTRUM_BG)
            );
        }
    }

    /// The contrast arithmetic itself, against values fixed by the WCAG definition.
    /// trace: FR-SPOT-11
    #[test]
    fn fr_spot_11_contrast_arithmetic_matches_wcag() {
        assert!(
            (contrast((0, 0, 0), (255, 255, 255)) - 21.0).abs() < 1e-9,
            "black on white is 21:1"
        );
        assert!((contrast((255, 255, 255), (255, 255, 255)) - 1.0).abs() < 1e-9);
        // Mid grey #767676 on white is the classic 4.54:1 (just over AA).
        assert!((contrast((0x76, 0x76, 0x76), (255, 255, 255)) - 4.54).abs() < 0.01);
        assert_eq!(blend((255, 0, 0), (0, 0, 0), 1.0), (255, 0, 0));
        assert_eq!(blend((255, 0, 0), (0, 0, 0), 0.0), (0, 0, 0));
        assert_eq!(blend((200, 100, 0), (0, 0, 0), 0.5), (100, 50, 0));
    }
}
