---
title: "R5 waivers — CAT encoders with no non-test caller"
status: Draft
version: "0.1"
updated: 2026-07-20
authors:
  - Simon Keimer (DC0SK)
---

# R5 waivers — encoders with no non-test caller

Rule **R5**: every traced `pub fn` in `k4-protocol::cat` should be called from
somewhere outside its own module and outside test code. An encoder that is
written, traced and unit-tested but never called ships as a capability that
reads *delivered* in the coverage table and is **unreachable to an operator**.

That is not hypothetical — it is how two capabilities shipped:

- the whole ATU/TUNE family (`AT`, `TU`) — [#118](https://github.com/dc0sk/K4remote/issues/118)
- the panadapter noise blanker (`#NB`, `#NBL`) — [#127](https://github.com/dc0sk/K4remote/issues/127)

Note a requirement-level check would have caught **neither**: `FR-PAN-CTL-01`
covers ten commands and asks only that each *encodes and round-trips*, which is
satisfiable with no UI whatsoever.

Entries below are encoders deliberately left uncalled. Each needs a reason —
"an alternate form is used instead" is legitimate; "we never got round to it"
means it belongs in an issue, not here.

| Encoder | Reason |
|---|---|
| `set_tx_power` | Superseded by `set_tx_power_range`, which the UI uses; kept as the simple `PC…H` form for callers that do not need the range selector. |
| `set_atu_mode` | The UI drives the ATU with `atu_toggle` (`AT/`), matching the radio's own in/bypass switch. `set_atu_mode` is the explicit-value form, kept for completeness. |
| `click_anchor` | A pure classifier consumed by `vfo_for_click` inside the same module; public so the anchoring rule is testable and documented, not to be called directly. |
| `set_nb` | The UI sends `set_nb_level`, which carries the on/off flag alongside the level; `set_nb` is the bare toggle. |
| `set_rit` / `set_xit` | The UI uses the radio's toggle forms so the button follows the radio's own state; these are the explicit-value forms. |
| `set_band_sub` | Sub-receiver band selection has no UI yet — the BAND screen targets the main receiver. Tracked as future work under `FR-VFO-04`. |
| `menu_open` / `menu_query_def` | The MENU screen uses `menu_query` + `menu_set`; these are the open-by-number and query-default forms, unused so far. |
| `set_pan_fixed` | **Temporary.** `#FXT` is a real gap, not an alternate form — tracked as [#133](https://github.com/dc0sk/K4remote/issues/133). Waived only so R5 can be enforcing from the start rather than shipping as an ignorable report. Remove this row when the DISPLAY-screen control lands. |

## Not waived — real gaps

None outstanding. `set_pan_fixed` is waived above **with an expiry**: it is a
genuine gap (#133), waived only so the R5 gate can be enforcing today instead of
shipping as a report nobody acts on. A waiver with a reason and an issue is
visible; a non-failing report is not.
