//! Band-condition colours for the band buttons (FR-UI-25): turns what the spot worker gathers
//! ([`BandInputs`]) and the operator's settings into one rating and one explanation per band, and
//! picks each rating's colour for the theme. Pure apart from [`local_hour`]; the design is
//! `docs/concept/band-conditions-plan.md` v0.2.

use k4_config::PropagationPrefs;
use k4_spot::bandcond::{self, Rating};
use k4_spot::Network;

use crate::spot_sources::BandInputs;
use crate::ui::EffectiveTheme;

/// The credit HamQSL asks for, shown wherever its data is used.
pub const HAMQSL_CREDIT: &str = "Band conditions: HamQSL.com (N0NBH)";

/// One band button's state: its rating (`None` = normal label colour) and its tooltip.
#[derive(Debug, Clone, PartialEq)]
pub struct BandView {
    pub rating: Option<Rating>,
    pub tip: String,
}

/// Each band of [`bandcond::BANDS`], rated from the inputs at `unix`. `local_hour` is this
/// computer's hour, used for day/night only when no valid locator is set.
pub fn band_views(
    inputs: &BandInputs,
    prefs: &PropagationPrefs,
    locator: &str,
    unix: u64,
    local_hour: u8,
) -> Vec<BandView> {
    let site = bandcond::locator_centre(locator);
    let day = bandcond::is_day(site, unix, local_hour);
    let when = match site {
        Some(_) => format!(
            "{} at {}",
            if day { "day" } else { "night" },
            locator.trim()
        ),
        None => format!(
            "{} by this computer's clock",
            if day { "day" } else { "night" }
        ),
    };
    let forecast = inputs.forecast.as_ref().filter(|_| prefs.hamqsl);
    let networks: Vec<&str> = inputs.counted.iter().map(|n| network_name(*n)).collect();
    bandcond::BANDS
        .iter()
        .zip(inputs.activity)
        .map(|(band, heard)| {
            let label = band.replace('m', " m");
            let fc = forecast
                .and_then(|f| bandcond::group_for_band(band).and_then(|g| f.rating(g, day)));
            let act = bandcond::activity_rating(heard);
            let rating = bandcond::combine(fc, act);
            let mut parts = Vec::new();
            if let Some(r) = fc {
                parts.push(format!("HamQSL {when}: {}", r.label()));
            }
            if heard > 0 {
                let plural = if heard == 1 { "station" } else { "stations" };
                let mins = bandcond::ACTIVITY_WINDOW_SECS / 60;
                parts.push(format!(
                    "{heard} {plural} heard in {mins} min ({})",
                    networks.join(", ")
                ));
            }
            let tip = if !prefs.colour_bands {
                format!("{label}: band-condition colours are off (Settings → PROPAGATION)")
            } else if let Some(r) = rating {
                let mut t = format!("{label}: {} — {}", r.label(), parts.join("; "));
                if fc.is_some() {
                    t.push_str(&format!(". {HAMQSL_CREDIT}"));
                }
                t
            } else if parts.is_empty() {
                format!("{label}: no current band-condition data")
            } else {
                format!("{label}: not rated — {}", parts.join("; "))
            };
            BandView {
                rating: rating.filter(|_| prefs.colour_bands),
                tip,
            }
        })
        .collect()
}

fn network_name(n: Network) -> &'static str {
    match n {
        Network::Rbn => "RBN",
        Network::DxCluster => "DX cluster",
        Network::PskReporter => "PSK Reporter",
        Network::FreeDvReporter => "FreeDV",
        Network::Pota => "POTA",
    }
}

/// A rating's label colour in a theme. Every one reaches 4.5:1 (WCAG AA for text) against the
/// band button idle and hovered, in every theme — held by a test.
pub fn rating_rgb(theme: EffectiveTheme, r: Rating) -> (u8, u8, u8) {
    match (theme, r) {
        (EffectiveTheme::Dark, Rating::Good) => (0x4C, 0xD9, 0x64),
        (EffectiveTheme::Dark, Rating::Fair) => (0xF5, 0xC2, 0x42),
        (EffectiveTheme::Dark, Rating::Poor) => (0xFF, 0x80, 0x80),
        (EffectiveTheme::Light, Rating::Good) => (0x1A, 0x6E, 0x2E),
        (EffectiveTheme::Light, Rating::Fair) => (0x7A, 0x4F, 0x00),
        (EffectiveTheme::Light, Rating::Poor) => (0xB0, 0x20, 0x20),
        (EffectiveTheme::Contrast, Rating::Good) => (0x5C, 0xFF, 0x7A),
        (EffectiveTheme::Contrast, Rating::Fair) => (0xFF, 0xD6, 0x40),
        (EffectiveTheme::Contrast, Rating::Poor) => (0xFF, 0x80, 0x80),
    }
}

/// The networks whose spots count as activity, from the settings.
pub fn activity_networks(p: &PropagationPrefs) -> Vec<Network> {
    [
        (p.activity_rbn, Network::Rbn),
        (p.activity_dx_cluster, Network::DxCluster),
        (p.activity_psk_reporter, Network::PskReporter),
        (p.activity_freedv, Network::FreeDvReporter),
    ]
    .into_iter()
    .filter_map(|(on, n)| on.then_some(n))
    .collect()
}

/// This computer's local hour (0–23) at Unix time `unix`, or the UTC hour if the C library cannot
/// say.
pub fn local_hour(unix: u64) -> u8 {
    let utc = ((unix / 3600) % 24) as u8;
    let Ok(t) = libc::time_t::try_from(unix) else {
        return utc;
    };
    // SAFETY: an all-zero `tm` is a valid value (integers, and on some platforms a nullable
    // pointer); `t` and `tm` live across the call, which only reads `t` and writes `tm`; both
    // functions are the re-entrant forms, safe to call from any thread.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    #[cfg(unix)]
    let ok = unsafe { !libc::localtime_r(&t, &mut tm).is_null() };
    #[cfg(windows)]
    let ok = unsafe { libc::localtime_s(&mut tm, &t) == 0 };
    match u8::try_from(tm.tm_hour) {
        Ok(h) if ok && h < 24 => h,
        _ => utc,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k4_spot::bandcond::{parse_hamqsl, BANDS};

    const FIXTURE: &[u8] =
        include_bytes!("../../crates/k4-spot/tests/fixtures/hamqsl-2026-10-04.xml");
    const UPDATED: u64 = 1_791_131_160; // 04 Oct 2026 16:26 UTC

    fn idx(band: &str) -> usize {
        BANDS.iter().position(|b| *b == band).unwrap()
    }

    fn inputs(forecast: bool, heard: &[(&str, usize)]) -> BandInputs {
        let mut activity = [0usize; 11];
        for (b, n) in heard {
            activity[idx(b)] = *n;
        }
        BandInputs {
            forecast: forecast.then(|| parse_hamqsl(FIXTURE).unwrap()),
            activity,
            counted: vec![Network::Rbn, Network::PskReporter],
            hamqsl: None,
        }
    }

    /// FR-UI-25: every band button has a band-condition view, in the button order, and the
    /// buttons' bands are exactly the ones rated.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_every_band_button_has_a_view() {
        let labels: Vec<String> = crate::ui::band_buttons()
            .iter()
            .map(|(l, _)| format!("{l}m"))
            .collect();
        assert_eq!(
            labels, BANDS,
            "band_buttons() and bandcond::BANDS must list the same bands"
        );
        let v = band_views(
            &inputs(false, &[]),
            &PropagationPrefs::default(),
            "",
            UPDATED,
            12,
        );
        assert_eq!(v.len(), crate::ui::band_buttons().len());
    }

    /// FR-UI-25: at JO31 in the late afternoon of the capture (16:26 UTC, 4 October, sun up)
    /// the day ratings apply; a sparse band raises nothing; a busy band raises one step; 160 m
    /// is rated from activity alone, capped at Fair; no data, no rating.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_band_views_combine_the_sources() {
        use Rating::*;
        let p = PropagationPrefs::default();
        let i = inputs(true, &[("20m", 14), ("40m", 2), ("160m", 12)]);
        let v = band_views(&i, &p, "JO31", UPDATED, 0);
        assert_eq!(v[idx("80m")].rating, Some(Poor), "80m-40m day Poor");
        assert_eq!(
            v[idx("40m")].rating,
            Some(Poor),
            "two stations are no evidence"
        );
        assert_eq!(
            v[idx("20m")].rating,
            Some(Good),
            "30m-20m day Fair, raised by activity"
        );
        assert_eq!(v[idx("10m")].rating, Some(Poor));
        assert_eq!(
            v[idx("160m")].rating,
            Some(Fair),
            "activity alone, capped at Fair"
        );
        assert_eq!(v[idx("6m")].rating, None, "no data for 6 m");
        let tip = &v[idx("20m")].tip;
        assert!(tip.starts_with("20 m: Good"), "{tip}");
        assert!(tip.contains("HamQSL day at JO31: Fair"), "{tip}");
        assert!(
            tip.contains("14 stations heard in 15 min (RBN, PSK Reporter)"),
            "{tip}"
        );
        assert!(
            tip.contains(HAMQSL_CREDIT),
            "the credit HamQSL asks for: {tip}"
        );
        assert_eq!(v[idx("6m")].tip, "6 m: no current band-condition data");
        // At night at JO31 (00:00 UTC) the night ratings apply: 80m-40m night is Fair.
        let night = band_views(&i, &p, "JO31", UPDATED - 16 * 3600 - 26 * 60, 12);
        assert_eq!(night[idx("80m")].rating, Some(Fair));
        assert!(
            night[idx("80m")].tip.contains("night at JO31"),
            "{}",
            night[idx("80m")].tip
        );
    }

    /// FR-UI-25: without a locator the local clock decides, and the tooltip says so.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_no_locator_uses_the_local_clock() {
        let i = inputs(true, &[]);
        let p = PropagationPrefs::default();
        let day = band_views(&i, &p, "", 0, 12);
        let night = band_views(&i, &p, "not a locator", 0, 2);
        assert_eq!(
            day[idx("80m")].rating,
            Some(Rating::Poor),
            "day: 80m-40m Poor"
        );
        assert_eq!(
            night[idx("80m")].rating,
            Some(Rating::Fair),
            "night: 80m-40m Fair"
        );
        assert!(day[idx("80m")].tip.contains("day by this computer's clock"));
    }

    /// FR-UI-25: colouring off, or HamQSL unticked, takes effect; an unrated band keeps the normal
    /// colour.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_switches_take_effect() {
        let i = inputs(true, &[("20m", 14)]);
        let off = PropagationPrefs {
            colour_bands: false,
            ..PropagationPrefs::default()
        };
        assert!(band_views(&i, &off, "JO31", UPDATED, 0)
            .iter()
            .all(|v| v.rating.is_none()));
        let no_hamqsl = PropagationPrefs {
            hamqsl: false,
            ..PropagationPrefs::default()
        };
        let v = band_views(&i, &no_hamqsl, "JO31", UPDATED, 0);
        assert_eq!(v[idx("20m")].rating, Some(Rating::Fair), "activity alone");
        assert_eq!(v[idx("80m")].rating, None, "no forecast used");
        assert!(
            !v[idx("20m")].tip.contains("HamQSL"),
            "{}",
            v[idx("20m")].tip
        );
        assert_eq!(
            activity_networks(&PropagationPrefs {
                activity_dx_cluster: false,
                ..PropagationPrefs::default()
            }),
            [Network::Rbn, Network::PskReporter, Network::FreeDvReporter]
        );
    }

    /// FR-UI-25: every rating colour reaches 4.5:1 against the band button idle and hovered, in
    /// every theme.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_colours_are_readable() {
        use crate::ui::{shade_rgb, Shade};
        for theme in [
            EffectiveTheme::Dark,
            EffectiveTheme::Light,
            EffectiveTheme::Contrast,
        ] {
            for r in [Rating::Good, Rating::Fair, Rating::Poor] {
                for bg in [Shade::Control, Shade::ControlHover] {
                    let c = k4_spot::style::contrast(rating_rgb(theme, r), shade_rgb(theme, bg));
                    assert!(c >= 4.5, "{theme:?} {r:?} on {bg:?}: {c:.2}");
                }
            }
        }
    }

    /// FR-UI-25: the local hour is an hour, and advances with the clock.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_local_hour() {
        let h = local_hour(UPDATED);
        assert!(h < 24);
        // One hour later is one hour on, as long as the test host's zone does not change its
        // offset in that hour (none of the usual CI or desk zones does then).
        assert_eq!(local_hour(UPDATED + 3600), (h + 1) % 24);
    }

    /// FR-UI-25: the HamQSL bounds agree across the two crates that state them.
    /// trace: FR-UI-25
    #[test]
    fn fr_ui_25_hamqsl_bounds_agree() {
        assert_eq!(
            (
                k4_config::HAMQSL_INTERVAL_MIN_SECS,
                k4_config::HAMQSL_INTERVAL_DEFAULT_SECS,
                k4_config::HAMQSL_INTERVAL_MAX_SECS
            ),
            (
                bandcond::HAMQSL_INTERVAL_MIN_SECS,
                bandcond::HAMQSL_INTERVAL_DEFAULT_SECS,
                bandcond::HAMQSL_INTERVAL_MAX_SECS
            )
        );
    }
}
