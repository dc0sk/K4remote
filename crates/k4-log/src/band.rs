//! The band of a frequency, as Wavelog names and bounds it (FR-LOG-08).
//!
//! Wavelog stores the `band` a client sends without checking it whenever a `freq` comes with it
//! (probed 2026-10-27: "21m" with 14.25 MHz was accepted), so K4 Remote must get it right itself.
//! The edges are those of Wavelog's own `Frequency::GetBand` (application/libraries/Frequency.php,
//! Wavelog 3.2.x; MIT License, © 2019 Peter Goodhall, © 2024 DF2ET, DJ7NT, HB9HIL, LA8AJA),
//! including which edges are open (`>`) and which closed (`>=`). The names are those of Wavelog's
//! band table, which calls the 24 GHz band `1.25cm` (GetBand says `1.2cm`); the millimetre bands,
//! not in that table, are left out.

/// One band: its name and its frequency range in Hz, with Wavelog's open/closed lower edge (the
/// upper edge is always open).
struct Band {
    name: &'static str,
    low: u64,
    low_inclusive: bool,
    high: u64,
}

const fn b(name: &'static str, low: u64, low_inclusive: bool, high: u64) -> Band {
    Band {
        name,
        low,
        low_inclusive,
        high,
    }
}

const BANDS: &[Band] = &[
    b("160m", 1_000_000, false, 2_000_000),
    b("80m", 3_000_000, false, 4_000_000),
    b("60m", 5_330_000, true, 5_405_000),
    b("40m", 6_000_000, false, 8_000_000),
    b("30m", 9_000_000, false, 11_000_000),
    b("20m", 13_000_000, false, 15_000_000),
    b("17m", 17_000_000, false, 19_000_000),
    b("15m", 20_000_000, false, 22_000_000),
    b("12m", 23_000_000, false, 25_000_000),
    b("10m", 27_000_000, false, 30_000_000),
    b("6m", 49_000_000, false, 55_000_000),
    b("4m", 69_000_000, false, 71_000_000),
    b("2m", 140_000_000, false, 150_000_000),
    b("1.25m", 218_000_000, false, 226_000_000),
    b("70cm", 420_000_000, true, 450_000_000),
    b("33cm", 900_000_000, false, 930_000_000),
    b("23cm", 1_200_000_000, false, 1_300_000_000),
    b("13cm", 2_200_000_000, false, 2_600_000_000),
    b("9cm", 3_000_000_000, false, 4_000_000_000),
    b("6cm", 5_000_000_000, false, 6_000_000_000),
    b("3cm", 9_000_000_000, false, 11_000_000_000),
    b("1.25cm", 23_000_000_000, false, 25_000_000_000),
];

/// Wavelog's band for `hz`, or `None` outside every band (the form then refuses to log).
pub fn band_for(hz: u64) -> Option<&'static str> {
    BANDS
        .iter()
        .find(|b| (hz > b.low || (b.low_inclusive && hz == b.low)) && hz < b.high)
        .map(|b| b.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FR-LOG-08: every edge of Wavelog's table, from both sides — the names and the open/closed
    /// edges as `GetBand` has them (literal, a contract with Wavelog).
    /// trace: FR-LOG-08
    #[test]
    fn fr_log_08_bands_match_wavelog_at_every_edge() {
        let table: &[(&str, u64, bool, u64)] = &[
            ("160m", 1_000_000, false, 2_000_000),
            ("80m", 3_000_000, false, 4_000_000),
            ("60m", 5_330_000, true, 5_405_000),
            ("40m", 6_000_000, false, 8_000_000),
            ("30m", 9_000_000, false, 11_000_000),
            ("20m", 13_000_000, false, 15_000_000),
            ("17m", 17_000_000, false, 19_000_000),
            ("15m", 20_000_000, false, 22_000_000),
            ("12m", 23_000_000, false, 25_000_000),
            ("10m", 27_000_000, false, 30_000_000),
            ("6m", 49_000_000, false, 55_000_000),
            ("4m", 69_000_000, false, 71_000_000),
            ("2m", 140_000_000, false, 150_000_000),
            ("1.25m", 218_000_000, false, 226_000_000),
            ("70cm", 420_000_000, true, 450_000_000),
            ("33cm", 900_000_000, false, 930_000_000),
            ("23cm", 1_200_000_000, false, 1_300_000_000),
            ("13cm", 2_200_000_000, false, 2_600_000_000),
            ("9cm", 3_000_000_000, false, 4_000_000_000),
            ("6cm", 5_000_000_000, false, 6_000_000_000),
            ("3cm", 9_000_000_000, false, 11_000_000_000),
            ("1.25cm", 23_000_000_000, false, 25_000_000_000),
        ];
        for &(name, low, inclusive, high) in table {
            assert_eq!(
                band_for(low + 1),
                Some(name),
                "{name} just above its low edge"
            );
            assert_eq!(
                band_for(high - 1),
                Some(name),
                "{name} just below its high edge"
            );
            assert_eq!(band_for(high), None, "{name}: the high edge is open");
            assert_eq!(
                band_for(low),
                inclusive.then_some(name),
                "{name}: the low edge is {}",
                if inclusive { "closed" } else { "open" }
            );
        }
        // Common K4 frequencies, and gaps.
        assert_eq!(band_for(14_074_000), Some("20m"));
        assert_eq!(band_for(5_357_000), Some("60m"));
        assert_eq!(band_for(50_313_000), Some("6m"));
        assert_eq!(band_for(144_300_000), Some("2m"), "a transverter frequency");
        assert_eq!(band_for(12_000_000), None);
        assert_eq!(band_for(0), None);
    }
}
