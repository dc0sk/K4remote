---
title: "Band conditions on the band buttons — design (FR-UI-25)"
status: Draft
version: "0.2"
updated: 2026-10-04
authors:
  - Simon Keimer (DC0SK)
---

# Band conditions on the band buttons — design (FR-UI-25)

`FR-UI-25` asks for each band button's label to be coloured by that band's current conditions,
from as many sources as are available (HamQSL at minimum), configured on a PROPAGATION Settings
tab with every source opt-out, an adjustable update interval, and no trouble without an internet
connection. This note fixes how, before any code. Everything marked **(tuned)** is an initial
value to revisit once real data has been watched.

## 0. Review

v0.1 was reviewed adversarially (Fable, 2026-10-04) before any code: **REVISE (narrow)**, not
release-blocking. Every finding was checked against the source and taken:

| # | Finding | Disposition |
|---|---|---|
| 1 | "Activity is counted before the window filter" was true of the networks, not the code: every source applies the window **inside** `poll` (`telnet.rs:331`, `mqtt_source.rs:407`, `freedv_source.rs:331`, `polled.rs:197`), so the worker never sees other bands | §1.2/§8: an **activity tap** in each source, called before the window check |
| 2 | `#[serde(default)]` on a bool is `false` — "all enabled by default" would have shipped as all off | §7: `default = "default_true"` per field and an `impl Default`, the crate's pattern |
| 3 | The locator belongs to the station, not to propagation | §7: `station_locator` on `Prefs` |
| 4 | `polled::clamp_interval` is 30 s–3600 s: it undercuts HamQSL's hourly floor and cannot reach a day | §6/§7: HamQSL's own bounds, stored in seconds |
| 5 | A changed setting rebuilds a polled source, which fetches at once (`next_attempt = now`) — ticking a box would refetch inside the hour | §6: the HamQSL source keeps its schedule and last result across a reconfigure |
| 6 | Freshness looser than the requirement; a failed fetch after a success kept colours for hours | §4: two limits — last successful fetch, and the feed's own `updated` |
| 7 | "Distinct callsigns summed over sources" double-counts one station heard by two networks | §4: the **union** of normalised calls per band |
| 8 | 6 m would be red almost always (`Band Closed` is its normal state); the locator→region rule was undefined | §2: 6 m from activity only |
| 9 | Contrast tested against the idle background only; buttons lighten on hover | §5: both shades |
| 10 | The size cap is the fetcher's, not the parser's; sabotage list lacked items 1 and 7 | §6, §9 |
| 11 | For activity sources "off" means not counted, not disconnected | §7: the tab says so |

## 1. What the sources actually give

### 1.1 HamQSL (N0NBH) — read 2026-10-04

- **Feed:** `https://www.hamqsl.com/solarxml.php`, XML, ~1.6 KB. One capture is committed as the
  parser's fixture (`crates/k4-spot/tests/fixtures/hamqsl-2026-10-04.xml`).
- **Band ratings:** `<calculatedconditions>` holds eight `<band name=… time=…>` elements: four
  groups — `80m-40m`, `30m-20m`, `17m-15m`, `12m-10m` — each for `day` and `night`, valued
  `Good`, `Fair` or `Poor` (the only values the capture shows; anything else is *unrated*, never
  guessed).
- **VHF:** `<calculatedvhfconditions>` reports E-skip and aurora as `Band Closed` in the capture;
  the open values are not documented and not observed. Not used (§2).
- **Freshness:** `<updated> 04 Oct 2026 1626 GMT</updated>`.
- **Fetch rate:** the page asks clients to update **no more than hourly** ("that is the update
  period for the flux parameters (rest are 3-hour updates)"). Interval: minimum and default
  **60 min**, maximum **24 h**.
- **Terms:** "Credit to HAMQSL.com would be appreciated"; no licence is stated. The PROPAGATION tab
  and every band tooltip that uses it say "Band conditions: HamQSL.com (N0NBH)". No HamQSL images
  are used.

### 1.2 Spot activity

The spot worker (`app/src/spot_sources.rs`) runs RBN and a DX cluster (telnet), PSK Reporter
(MQTT), POTA (polled) and FreeDV Reporter (WebSocket). Each source applies the UI's frequency
window **inside its own `poll`**, before a spot reaches the worker, and the window also picks which
PSK Reporter band topics are subscribed. What reaches the *network* side:

- **RBN, DX cluster, FreeDV:** every band, whatever the window.
- **PSK Reporter:** only the subscribed bands. The design does **not** subscribe to more, which would
  multiply its traffic and undercut the spot feeds' bounds (`FR-SPOT-05`/`-14`); its activity covers
  the bands in view.
- **POTA:** activations, not propagation evidence; not used.

So each streaming source gets an **activity tap**: an optional sink, set by the worker, that `poll`
hands every *parsed and accepted* spot (callsign, frequency, time) **before** the window check. The
store still receives only in-window spots and the window statistics are unchanged; the tap adds no
network traffic. A network's activity exists only while that network runs for spots (`FR-SPOT-04`).

## 2. Mapping onto the K4's bands

| K4 band (`BN`) | HamQSL group | Notes |
|---|---|---|
| 160 m (00) | — | activity only |
| 80 m, 60 m, 40 m (01–03) | `80m-40m` | 60 m lies inside the group's range |
| 30 m, 20 m (04, 05) | `30m-20m` | |
| 17 m, 15 m (06, 07) | `17m-15m` | |
| 12 m, 10 m (08, 09) | `12m-10m` | |
| 6 m (10) | — | activity only (§1.1) |
| XVTR bands | — | never coloured |

## 3. Day or night: the radio's site

The station's **Maidenhead locator** (4 or 6 characters, e.g. `JO31`), entered on the PROPAGATION
tab and stored as a station setting (§7); empty by default. With a locator, *day* is the sun above
the horizon at the centre of that square (solar elevation > 0°, from UTC and the NOAA
solar-position approximation — a fraction of a degree, enough for a day/night switch). Without one,
*day* is 06:00–18:00 on this computer's clock, and the tab says that is the assumption. An invalid
locator is refused at entry, not stored.

## 4. From sources to one rating per band

Ratings are `Good > Fair > Poor`, or *unrated*.

- **Forecast** (HamQSL): the band's group rating for the current day/night at the station.
- **Activity** (spots): the number of **distinct normalised callsigns** heard on the band in the last
  **15 min (tuned)**, as the **union** across the enabled activity sources — a station heard by RBN
  and the cluster counts once. (RBN's `call` is the station heard, not the skimmer, so many skimmers
  hearing one station already count once.) `≥ 10` **(tuned)** reads as *Good*, `≥ 3` **(tuned)** as
  *Fair*; fewer is **no evidence**, never *Poor*: a quiet band may be open with nobody reporting.
- **Combination:** activity can only **raise** a forecast, by at most one step (*Poor* → *Fair*,
  *Fair* → *Good*), never lower it. Activity is counted worldwide, so it shows the band is in use
  somewhere, not that it is open from the station; one step is as far as that evidence goes. With
  RBN on, 40 m and 20 m will usually earn that step — expected, and the tooltip says why. For a band
  the forecast does not rate (160 m, 6 m), activity alone gives at most *Fair*.
- **Freshness.** HamQSL's rating counts only while **both** hold: the last successful fetch is no
  older than **2 × the update interval** (one missed fetch is tolerated, a lost connection is not),
  and the feed's own `updated` is no older than **3 h + the update interval** (the feed's slowest
  cycle, the 3-hour conditions update, plus one fetch). Activity ages out with its 15-min window.
  The requirement's "older than the source's refresh interval" is read as these two limits; the SRS
  row is amended to say so.
- **Unrated → normal label colour**, as are: colouring switched off, and every source off or
  unreachable.

Every band button's tooltip spells it out, so colour is never the only carrier and never
unexplained: e.g. "20 m: Fair — HamQSL day Fair (updated 16:26 UTC); 14 stations heard in 15 min
(RBN, PSK Reporter). Band conditions: HamQSL.com (N0NBH)."

## 5. Colours

*Good* green, *Fair* amber, *Poor* red, on the label text only. Each reaches **≥ 4.5:1** (WCAG AA
for text) against the band button's background **both idle (`Shade::Control`) and hovered
(`Shade::ControlHover`)**, held by a test using the existing WCAG `contrast` function. Red/green is
the hardest pair for colour-blind operators — one more reason the tooltip carries the rating in
words.

## 6. Fetching HamQSL; offline

- **Its own polled source**, with the `k4_spot::polled` mechanics — request on a short-lived thread
  with a timeout, size-capped by the fetcher (64 KiB; the feed is ~1.6 KB), a finished answer
  collected by the worker's `poll`, back-off and retry on failure, nothing on the UI, CAT or audio
  paths (`FR-SPOT-09`) — but its own interval bounds: `HAMQSL_INTERVAL_MIN_SECS = 3600`,
  `…_DEFAULT_SECS = 3600`, `…_MAX_SECS = 86400`.
- **It keeps its schedule across a reconfigure.** Unlike the spot slots, it is not rebuilt when a
  setting changes: a new interval reschedules from the *last* fetch (never sooner than one minimum
  interval after it), and switching HamQSL off and on within the hour reuses the last result instead
  of refetching. Only the first start fetches at once.
- **A failure** shows as a status line on the PROPAGATION tab ("HamQSL: unreachable, retrying at
  17:40") — never a dialog, never a blocked control. With no source reachable every label keeps its
  normal colour and the BAND screen behaves exactly as today.
- **The parser** (in `k4-spot`, no XML dependency) has its own size cap (64 KiB) independent of the
  fetcher's, reads only the `band` and `updated` elements, and treats a malformed body as a failed
  fetch.

## 7. Settings (persisted schema)

**On `Prefs` (station):** `station_locator: String`, `#[serde(default)]` (empty).

**New section `propagation: PropagationPrefs`**, `#[serde(default)]` on the section, with an
`impl Default` and `#[serde(default = "default_true")]` on each bool:

| Field | Type | Default | Notes |
|---|---|---|---|
| `colour_bands` | bool | `true` | master switch for the colouring |
| `hamqsl` | bool | `true` | |
| `activity_rbn` | bool | `true` | "off" = not counted; the network still runs for spots |
| `activity_dx_cluster` | bool | `true` | as above |
| `activity_psk_reporter` | bool | `true` | as above |
| `activity_freedv` | bool | `true` | as above |
| `hamqsl_interval_secs` | u64 | `3600` | read through an accessor clamped to 3600–86400 |

`k4-config` keeps its own copy of the three HamQSL bounds (it does not depend on `k4-spot`); a
parity test in the app holds the two copies equal, as for the FreeDV refresh bounds. No migration is
needed: a missing section or field is its default.

## 8. Where it lives

- `k4-spot::bandcond` (pure): the HamQSL parser; the band mapping; the locator and solar elevation;
  the activity counter (a union per band over a sliding window); the combination; the freshness
  rules. Time is passed in; nothing touches a network or a clock.
- `k4-spot` sources: the activity tap (`set_activity_tap`) in the telnet, MQTT and FreeDV sources,
  called before the window check.
- `app/src/spot_sources.rs`: runs the HamQSL source next to the spot slots (not in them, §6); owns
  the activity counter and the taps; publishes one `BandConditions` snapshot per tick.
- `k4-config`: `station_locator`, `PropagationPrefs`.
- `app/src/main.rs`: the PROPAGATION tab; the BAND screen's label colours and tooltips.

## 9. Evidence plan

Tests (all `fr_ui_25_*`):

1. Parser: the committed fixture gives eight group ratings and `updated`; a body past the parser's
   cap, a truncated body and a missing `calculatedconditions` are errors; an unknown rating value is
   unrated.
2. Mapping: sweeps `band_buttons()` from the list (a band added without a mapping fails), and XVTR
   bands are never rated.
3. Day/night: at JO31 the sun is up at 12:00 UTC in June and down at 00:00 UTC; inside a polar night
   it stays down; no locator falls back to the local clock; invalid locators are refused.
4. Activity: one call heard by RBN and the cluster counts once; the window ages calls out;
   a switched-off source's spots are not counted.
5. **The tap sees outside the window:** with the window on 20 m, an RBN spot on 40 m is counted for
   40 m and is not stored (through the real telnet source's `poll`).
6. Combination: forecast alone; activity alone (capped at *Fair*); both (raise by one, never lower,
   never above *Good*); none → unrated; sparse activity is no evidence.
7. Freshness: a last successful fetch older than 2 × interval, or an `updated` older than
   3 h + interval, leaves HamQSL out.
8. Schedule: a reconfigure (interval change, HamQSL off and on) within the hour does not fetch;
   an interval below 3600 s reads as 3600.
9. Colours: 4.5:1 against idle and hovered backgrounds; colouring off or unrated gives the normal
   colour.
10. Settings: a configuration without the section loads with every source on (the `default_true`
    trap); the HamQSL bounds agree across crates.
11. Offline: with a fetcher that always fails, `poll` never blocks, retries back off, and the bands
    stay unrated with only the status line reporting it.

Sabotage, one per rule, each must turn its test red: drop "never lower"; drop the one-step cap; set
`HAMQSL_INTERVAL_MIN_SECS` to 59; remove the freshness check; flip the solar elevation sign; call
the tap after the window check instead of before (5); count per source and sum instead of the union
(4); rebuild the HamQSL source on reconfigure (8); `#[serde(default)]` instead of `default_true` on
one bool (10). On screen (demo, `--demo` with an injected snapshot): coloured labels, tooltips, the
PROPAGATION tab. On the air: whether the colours match what the bands do is a check for DC0SK
(#221).

## 10. Open, not decided here

- HamQSL's VHF open values (§1.1): 6 m stays activity-only until one is observed.
- The activity thresholds and window (§4) are first guesses, kept as named constants.
