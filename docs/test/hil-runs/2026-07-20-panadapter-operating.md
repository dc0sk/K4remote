---
title: "HIL run — 2026-07-20 — panadapter operating check"
status: Draft
version: "0.1"
updated: 2026-07-20
authors:
  - Simon Keimer (DC0SK)
---

# HIL run — 2026-07-20 — DC0SK

- Radio: Elecraft K4 (live)
- Client: k4remote on `main` after `c70c758` (FR-PAN-05…09 all merged)

Focus: the panadapter work merged this cycle had been verified only by unit
tests and against the simulator. `k4-sim` emits no PAN frames, so nothing in
FR-PAN-05…09 had ever been observed against a radio.

| Item | Result | Notes |
|---|---|---|
| Click-to-QSY lands the passband correctly (`FR-PAN-05`) | **pass** | Operator report: "frequency selection via the pan is fine now" |
| Display span matches `#SPN` (`FR-PAN-08`) | **pass (implied)** | The click→Hz mapping is scaled by `tier / #SPN`; it could not read correctly unless the tier crop were right |
| Mini-pan enable from the DISPLAY screen (`FR-UI-14`) | **FAIL** | Operator could not turn the mini-pan on — see below |

## Evidence tier

`FR-PAN-05` moves from *unit test + simulator* to **field-confirmed** for the
click-to-QSY path.

`FR-PAN-08` is confirmed only **indirectly**: a correct click mapping is not
achievable with a wrong tier crop, since the error scales by `tier / #SPN`.
That is strong evidence but not a direct observation, and it does not by itself
establish that the radio streams a tier wider than `#SPN` — if the two happen
to be equal on the spans tested, the crop is a no-op and remains unexercised.
**Still open:** capture raw PAN frames and compare `sample_rate × 1000` against
the `#SPN` in force.

Not covered by this run, and still unverified against hardware:

- `FR-PAN-06` waterfall scrolling on retune
- `FR-PAN-07` axis labels / `#REF`+`#SCL` window
- `FR-PAN-09` the rasterised waterfall — this rewrote how every waterfall pixel
  is produced and has never been seen to render

## Mini-pan defect

The mini-pan could not be enabled from the DISPLAY screen. Two independent
faults were found by inspection afterwards; which one the operator hit is not
yet established.

1. **MiniPAN decoded with the wrong header.** `R-EXT-01` gives the `0x03`
   payload a 5-byte header (`type, version, sequence, reserved, receiver`) with
   bins from offset 5 and *no* geometry fields. `PanFrame::decode` applied the
   main pan's 27-byte header to both, which consumes 22 bins as phantom
   metadata and **rejects outright any mini frame shorter than 27 bytes**. The
   existing test asserted only that `mini == true`, never the layout, so it
   gave no protection.
2. **`#MP$-1` was indistinguishable from "off".** D12 documents `-1` as *the
   mini-pan cannot be turned on with the current radio settings*. The parser
   mapped both `-1` and `0` to `Some(false)`, so a refusal by the radio looked
   exactly like a normal off state and the button appeared dead.

Both are fixed; see the accompanying change.

### Resolved: what `#MP$-1` actually means

Established on the radio by DC0SK, 2026-07-20:

> **The mini-pan requires dual-pan to be OFF when the sub receiver is
> disabled.** With dual-pan on and no sub RX, the K4 reports `#MP$-1` and the
> mini-pan cannot be displayed. Turning dual-pan off (or enabling the sub RX)
> allows it.

This is documented in **neither** D12 nor D14 — the `#MP$` NOTE says only "based
on current radio settings", and D14's Mini-Pan sections give no precondition at
all. It was found by experiment.

It also makes sense of the mechanism: D14 (p.1489) says tapping an S-meter
*switches to* the mini-pan **for that receiver**, so the mini-pan occupies a
receiver's meter area. With dual-pan on and no second receiver, there is no
coherent place for it.

Both faults in the fix above were therefore real but *neither* was the cause the
operator hit: the radio was refusing all along, and the pre-fix UI could not say
so. The header-size defect would have broken rendering once the refusal was
lifted.
