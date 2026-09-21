//! The text and placement of a nameplate's tooltip (FR-SPOT-10). Pure, so what an operator reads
//! and where it lands are tested rather than assumed.

use crate::layout::{CHAR_W, PLATE_PAD};
use crate::{Network, Spot};

/// Height of one tooltip line, pixels.
pub const LINE_H: f32 = 12.0;

/// A frequency as megahertz to the hertz, e.g. `14.074742 MHz`.
pub fn format_freq_mhz(hz: u64) -> String {
    format!("{}.{:06} MHz", hz / 1_000_000, hz % 1_000_000)
}

/// How long ago, in the fewest words: `just now`, `42 s ago`, `7 min ago`, `3 h ago`.
pub fn format_age(age_secs: u64) -> String {
    match age_secs {
        0..=4 => "just now".to_string(),
        5..=59 => format!("{age_secs} s ago"),
        60..=3599 => format!("{} min ago", age_secs / 60),
        _ => format!("{} h ago", age_secs / 3600),
    }
}

/// The network's name as the operator knows it.
pub fn network_name(network: Network) -> &'static str {
    match network {
        Network::Rbn => "RBN",
        Network::DxCluster => "DX cluster",
        Network::PskReporter => "PSK Reporter",
        Network::Pota => "POTA",
    }
}

/// The lines of a spot's tooltip. Fields the network did not give are left out, not shown blank.
pub fn tooltip_lines(spot: &Spot, now: u64) -> Vec<String> {
    let mut lines = vec![spot.call.clone()];
    let mut freq = format_freq_mhz(spot.freq_hz);
    if let Some(mode) = &spot.mode {
        freq.push_str(" · ");
        freq.push_str(mode);
    }
    lines.push(freq);
    let mut who = network_name(spot.network).to_string();
    if let Some(spotter) = &spot.spotter {
        who.push_str(" · heard by ");
        who.push_str(spotter);
    }
    lines.push(who);
    let mut when = String::new();
    if let Some(snr) = spot.snr_db {
        when.push_str(&format!("{snr} dB · "));
    }
    when.push_str(&format_age(now.saturating_sub(spot.time)));
    lines.push(when);
    if let Some(comment) = &spot.comment {
        lines.push(comment.clone());
    }
    lines
}

/// The size a tooltip needs for `lines`, pixels.
pub fn tooltip_size(lines: &[String]) -> (f32, f32) {
    let widest = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    (
        widest as f32 * CHAR_W + 2.0 * PLATE_PAD,
        lines.len() as f32 * LINE_H + 2.0 * PLATE_PAD,
    )
}

/// Where to put a tooltip of `size` for a pointer at `cursor` inside a pane of `bounds`: just
/// below and to the right of the pointer, moved to the other side of it where that would run off the
/// pane, and finally clamped so it is never cut off.
pub fn tooltip_origin(cursor: (f32, f32), size: (f32, f32), bounds: (f32, f32)) -> (f32, f32) {
    const GAP: f32 = 12.0;
    let mut x = cursor.0 + GAP;
    if x + size.0 > bounds.0 {
        x = cursor.0 - GAP - size.0;
    }
    let mut y = cursor.1 + GAP;
    if y + size.1 > bounds.1 {
        y = cursor.1 - GAP - size.1;
    }
    (
        x.clamp(0.0, (bounds.0 - size.0).max(0.0)),
        y.clamp(0.0, (bounds.1 - size.1).max(0.0)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot() -> Spot {
        Spot::new("aa1aaa", 14_074_742, 1_000, Network::PskReporter)
            .unwrap()
            .with_mode("FT8")
            .with_snr(-9)
            .with_spotter("BB2BBB")
    }

    /// FR-SPOT-10: the tooltip says who, where, how it was heard and how old, and leaves out what
    /// the network did not say.
    /// trace: FR-SPOT-10
    #[test]
    fn fr_spot_10_tooltip_text() {
        assert_eq!(
            tooltip_lines(&spot(), 1_000 + 185),
            [
                "AA1AAA",
                "14.074742 MHz · FT8",
                "PSK Reporter · heard by BB2BBB",
                "-9 dB · 3 min ago"
            ]
        );
        // Only the essentials: no mode, no SNR, no spotter.
        let bare = Spot::new("W1AW", 7_030_000, 1_000, Network::Rbn).unwrap();
        assert_eq!(
            tooltip_lines(&bare, 1_002),
            ["W1AW", "7.030000 MHz", "RBN", "just now"]
        );
        // A comment is shown when there is one.
        let with = Spot::new("K1ABC", 14_200_000, 0, Network::DxCluster)
            .unwrap()
            .with_comment("CQ DX up 2");
        assert_eq!(
            tooltip_lines(&with, 4000).last().map(String::as_str),
            Some("CQ DX up 2")
        );
        // A spot stamped in the future (clock skew) is "just now", never a wrapped huge age.
        assert_eq!(tooltip_lines(&bare, 500)[3], "just now");

        // Frequency to the hertz, including small and large values.
        assert_eq!(format_freq_mhz(14_074_742), "14.074742 MHz");
        assert_eq!(format_freq_mhz(1_800_000), "1.800000 MHz");
        assert_eq!(format_freq_mhz(144_174_000), "144.174000 MHz");
        assert_eq!(format_freq_mhz(999), "0.000999 MHz");
        // Age wording at each boundary.
        for (secs, want) in [
            (0, "just now"),
            (4, "just now"),
            (5, "5 s ago"),
            (59, "59 s ago"),
            (60, "1 min ago"),
            (3599, "59 min ago"),
            (3600, "1 h ago"),
            (86_400, "24 h ago"),
        ] {
            assert_eq!(format_age(secs), want, "{secs} s");
        }
        assert_eq!(network_name(Network::Rbn), "RBN");
        assert_eq!(network_name(Network::DxCluster), "DX cluster");
        assert_eq!(network_name(Network::PskReporter), "PSK Reporter");
        assert_eq!(network_name(Network::Pota), "POTA");
    }

    /// FR-SPOT-10: a tooltip sits beside the pointer and is always fully inside the pane, flipped to
    /// the other side of the pointer near the right and bottom edges.
    /// trace: FR-SPOT-10
    #[test]
    fn fr_spot_10_tooltip_stays_inside_the_pane() {
        let bounds = (800.0, 300.0);
        let size = (120.0, 60.0);
        // Room on both sides: below and to the right of the pointer.
        assert_eq!(tooltip_origin((100.0, 50.0), size, bounds), (112.0, 62.0));
        // Near the right edge: flips to the pointer's left.
        let (x, _) = tooltip_origin((780.0, 50.0), size, bounds);
        assert_eq!(x, 780.0 - 12.0 - 120.0);
        // Near the bottom: flips above the pointer.
        let (_, y) = tooltip_origin((100.0, 290.0), size, bounds);
        assert_eq!(y, 290.0 - 12.0 - 60.0);
        // Every pointer position gives a tooltip wholly inside the pane.
        for cx in (0..=800).step_by(20) {
            for cy in (0..=300).step_by(20) {
                let (x, y) = tooltip_origin((cx as f32, cy as f32), size, bounds);
                assert!(x >= 0.0 && x + size.0 <= bounds.0, "x {x} at ({cx},{cy})");
                assert!(y >= 0.0 && y + size.1 <= bounds.1, "y {y} at ({cx},{cy})");
            }
        }
        // A pane smaller than the tooltip pins it to the corner instead of going negative.
        assert_eq!(tooltip_origin((5.0, 5.0), size, (50.0, 30.0)), (0.0, 0.0));
        // Size follows the widest line.
        let lines = vec!["ab".to_string(), "abcdef".to_string()];
        assert_eq!(
            tooltip_size(&lines),
            (
                6.0 * CHAR_W + 2.0 * PLATE_PAD,
                2.0 * LINE_H + 2.0 * PLATE_PAD
            )
        );
    }
}
