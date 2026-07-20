---
title: "HIL run — 2026-07-20 — panadapter rendering against a live K4"
status: Draft
version: "0.1"
updated: 2026-07-20
authors:
  - Simon Keimer (DC0SK)
---

# HIL run — 2026-07-20 (second session) — DC0SK

- Radio: Elecraft K4, live, TLS-PSK to `192.168.121.58:9204`
- Client: `k4remote` v0.3.0 release build, Linux (Manjaro), X11, window 1920×1050
- Operator: absent; session driven by an assistant via synthetic X11 events
- Transmit: **never armed**. No PTT, no tune, no carrier at any point — confirmed
  in the UI and by the absence of any arm/TX/tune entry in the client log.

Focus: the v0.3.0 panadapter work shipped on unit-test and simulator evidence
only. `k4-sim` emits no PAN frames, so **none of FR-PAN-06/07/09 had ever been
observed rendering**. This session put the app in front of a real spectrum
stream for the first time.

## Results

| Requirement | Result | What was actually observed |
|---|---|---|
| `FR-PAN-09` waterfall as one texture | **pass** | Both panes render a live waterfall in dual view. **First observation ever** — this code rewrote every waterfall pixel and had shipped unseen. |
| `FR-PAN-07` axis + level scales | **pass** | Axis labelled `7.070 / 7.095 / 7.120 / 7.145 / 7.170`; readout `100.0 kHz span · 188 Hz/bin`; dB grid adapted to the window (10 dB steps over a ~50 dB `#REF`/`#SCL` window) rather than the old fixed 20 dB. |
| `FR-PAN-06` scroll + clip | **partial pass** | After a large retune, history rows scrolled out of the view and were clipped to background, leaving only newly-arrived rows — the off-canvas branch working as designed. The *small*-retune case (a signal holding one vertical line while the history slides) was **not** observed; see limitations. |
| `FR-PAN-05` passband overlay | **consistent, not conclusive** | On VFO A in LSB the shaded band sat offset from the pane centre toward the low side, which is what the sideband sense requires. Not measured against the filter edges, so this corroborates rather than proves. Click-to-QSY placement was separately confirmed correct by the operator (see `2026-07-20-panadapter-operating.md`). |
| `FR-ATU-01` ATU control | **pass (display only)** | The radio reports a tuner fitted; `ATU AUTO` and the `ATU IN` switch render live state. Tuning was **deliberately not exercised** — `TU3` keys the transmitter and the operator was absent. |
| `FR-UI-TIP-01` tooltips | **pass** | Tips appear after the dwell and carry their CAT command. Captured: *"Arm transmit. Nothing can key the radio until this is on — PTT, CW keying and TUNE are all gated by it."* and *"Connection, audio devices, levels and application preferences."* |
| Connection / shell | **pass** | TLS-PSK connect with the keychain password, UTC clock, S-meter (S3, −106 dBm), dual-pan A+B, mini-pan strip rendering, per-band switch state (`ANT 1`, `RX A 2`, `SUB A 6`). |

Not exercised: `FR-TX-TUNE-01` (keys the transmitter), `FR-UI-UPD-01` (live
HTTPS round-trip), `FR-PAN-08` tier cropping as a *direct* observation.

## Limitations of this run

Driving the GUI with synthetic X11 events proved unreliable in ways that limit
what can be claimed:

- Wheel events coalesced and arrived in late bursts, so the click→step
  accounting is not trustworthy. This is why `FR-PAN-06` is recorded as a
  partial pass: the dramatic off-canvas case was observed, the subtle
  frequency-alignment case was not.
- Screen state was not verified between clicks, so a softkey press intended to
  *open* a screen sometimes closed it, and the following click landed on the
  panadapter as a click-to-QSY. Several radio-state changes below are
  attributable to that, not to the app.

A future session of this kind should drive the radio through the app's raw CAT
console rather than the pointer, and verify the visible screen before each
click.

## Radio state disturbed

Recorded for honesty and so the operator can restore:

- **VFO A left at 4.920.000 USB; it began at 7.120.000 LSB.** VFO B untouched at
  7.137.500 LSB.
- `BW` and `SHIFT` read 2.80 / 1500 Hz at the end against 2.50 / 1450 Hz at the
  start. A 40 m band-stack recall mid-session did correctly restore BW, NR, RIT
  and AGC, so these were re-disturbed afterwards.
- Band-stack registers were written while tuning across bands; one 40 m register
  now holds a frequency around 8.9 MHz.

No transmit occurred, no configuration file was modified, and the client
disconnected cleanly (`RRN;`). The radio was left powered on and receiving
rather than switched off, because D12 provides no CAT power-**on** (see
`FR-PWR-01`) — powering it down would have stranded it until physically
restarted.

## Finding raised

Wheel-tuning over a pane steps by the radio's `VT`, which was **1 MHz** on this
radio, while the app's own `prefs.tune_step_hz` read 100. A single stray scroll
over the panadapter can therefore move the VFO a megahertz, cross bands, and
recall band-stack registers that also change mode and DSP settings — with no
on-screen indication of the step size beforehand. Raised as issue #130.
