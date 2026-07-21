# Changelog

All notable changes to K4 Remote are documented here.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Entries before 0.4.0 are summarised retrospectively — this file was introduced
during the 0.4.0 release, so earlier detail lives in the git history and in the
change ledgers under [`docs/test/test-strategy.md`](docs/test/test-strategy.md)
and [`docs/requirements/system-requirements.md`](docs/requirements/system-requirements.md).

## [0.4.0] — 2026-07-21

The K4's **interaction grammar**: every control chip now carries the radio's own
tap, hold, and settings-panel behaviour. Plus a round of correctness fixes found
by operating against a live K4 — including **four transmit-safety faults**, of
which the most serious allowed the transmitter to be keyed with the TX arm off.
Anyone running 0.3.0 should update.

### Added

- **Right-click settings popups** for the receiver chips — ATT, PRE, AGC, NB, NR,
  NOTCH and APF each open the panel the K4 opens on the paired switch's hold
  (D14 p.1318, p.1368), carrying that control's level, mode and on/off together.
  The popup opens at the control, and is dismissed by clicking outside, `ESC`, or
  its close button. (`FR-UI-POPUP-01`)
- **Tap vs. hold** on control chips, using the radio's own ~½ s threshold
  (D14 p.359): AGC taps between slow and fast while a hold switches it off,
  the attenuator hold steps its 3 dB ladder, and the NB hold cycles the filter
  mode (NONE/NARROW/WIDE) that was previously readable but not settable.
  (`FR-UI-HOLD-01`)
- **Fixed-tune** (`#FXT`) reachable from the DISPLAY screen.
- A build gate (`cargo xtask`, rule R5) for CAT encoders that nothing calls, so
  a command that is written but never wired is a build failure rather than dead
  code.
- A **keyboard emergency stop**: **`ESC` while on air** stops transmission
  (off air it keeps closing popups and dialogs as before), with
  **`Ctrl+Shift+X`** as an unconditional backstop. Both are handled ahead of
  every other key, including text entry, so neither can be swallowed by
  whatever holds focus. (`FR-TX-SAFE-05`)

### Fixed

- **Transmit was possible with the TX arm off.** The arm interlock was enforced
  in three code paths but *not* at the point every command reaches the radio, so
  the front-panel switch emulations — **TUNE**, **TUNE LP**, **ATU TUNE** and
  **XMIT** — along with DVR playback and the diagnostics console all bypassed
  it and keyed the transmitter while disarmed. It is now enforced at that single
  seam. Commands that *stop* transmission are never gated, and a refusal is
  reported rather than failing silently.
- **The emergency stop could not stop transmission the app had not started.**
  Whether the radio was on air was judged from local intent alone, while the
  radio's own report went unread — so a tune begun from the switch row, a
  front-panel PTT, VOX or the K-Pod were all invisible to it. The radio's report
  is now consulted, transmit-capable commands are tracked where they are sent,
  and the radio is polled for its transmit state twice a second instead of once
  every five seconds.
- **The emergency stop was itself incomplete**, ending a tune only when it
  believed one was running — so it could disarm TX while the radio kept
  transmitting. It now sends every stop unconditionally.
- **The diagnostics console could make the application unresponsive, and it
  could exit**, within seconds of heavy CAT traffic. The log buffer was rebuilt
  on every worker-loop iteration (which is paced by inbound frames, not by a
  timer) and the whole console text was re-laid-out ten times a second. Both are
  now throttled and shared rather than copied. *The lag and the exit are gone,
  but the mechanism of the exit itself was never captured — if you see the
  application close unexpectedly, please report it.*
- **The attenuator ignored 3, 6 and 9 dB.** The `RA` level was interpolated
  without zero-padding, so single-digit levels ran into the on/off field —
  3 dB went out as `RA31;`, which the radio read as a malformed level and
  discarded, leaving the previous level in place. Only 12/15/18/21 dB worked,
  and 0 dB (OUT) worked by coincidence. This had also silently broken three of
  the eight rungs of the `[ATTN]` hold since it shipped.
- **Press-and-hold never fired.** An iced `Button` with an `on_press` captures
  both press and release, so the wrapper timing the gesture saw nothing — the
  tap still worked, which made the control look healthy while the hold did
  nothing at all.
- **The TX indicator stayed dark during an ATU tune.** It was driven by the
  transmit flag, which a tune deliberately leaves clear so the microphone path
  stays closed; it now lights for any route to air.
- **CAT command rejections were invisible** — the radio's error replies are now
  surfaced to the operator instead of being swallowed.
- **Panadapter trace and waterfall disagreed.** The two used different
  pixel conventions (bins as points vs. bins as cells), which stretched the
  trace relative to the waterfall — up to 1.7 % plus a half-bin offset at low
  bin counts.
- **Span is tracked per pan.** The K4 keeps a span for each pan and the DISPLAY
  controls address one at a time, so changing the span left the other pane
  streaming its old span while both were labelled with the new one. Related:
  only `#SPN` is per-pan targeted — `#REF` and `#SCL` are global, and sending
  them with the sub-pan modifier made every DISPLAY control silently do nothing
  whenever TARGET was B.
- **The mini-pan** now stays visible above an open menu screen, and is no longer
  hidden when the radio reports it unavailable.
- The panadapter wheel steps by the **displayed** span, and the pan noise
  blanker is reachable.
- The attenuator slider no longer fights the radio's read-back while being
  dragged.

### Changed

- The DISPLAY screen puts **TARGET** next to the PAN selector, where it applies.

## [0.3.0] — 2026-07-20

### Added

- A **mode-aware panadapter**: click-to-QSY that places the passband by sideband
  sense, a waterfall that scrolls with the VFO, and labelled frequency and level
  scales synced to the radio.
- **ATU and TUNE** control.
- An About-box **update check**.
- Switchable control **tooltips**.

## [0.2.3] — 2026-07

### Added

- Band buttons follow the transmit VFO; in-app **MENU value editing**.

## [0.2.2] — 2026-07

### Added

- A Windows **`setup.exe` installer**.

## [0.2.1] — 2026-07

### Fixed

- K-Pod tap/hold discrimination and hold-while-tuning.

## [0.2.0] — 2026-07

### Added

- **Elecraft K-Pod** support: rocker and encoder tuning, and configurable
  F1–F8 tap/hold macros.
- A filterable, selectable **diagnostics console**.

### Fixed

- RIT/XIT sync.

[0.4.0]: https://github.com/dc0sk/K4remote/releases/tag/v0.4.0
[0.3.0]: https://github.com/dc0sk/K4remote/releases/tag/v0.3.0
[0.2.3]: https://github.com/dc0sk/K4remote/releases/tag/v0.2.3
[0.2.2]: https://github.com/dc0sk/K4remote/releases/tag/v0.2.2
[0.2.1]: https://github.com/dc0sk/K4remote/releases/tag/v0.2.1
[0.2.0]: https://github.com/dc0sk/K4remote/releases/tag/v0.2.0
