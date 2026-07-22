# Changelog

All notable changes to K4 Remote are documented here.

The format is based on [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Entries before 0.4.0 are summarised retrospectively — this file was introduced
during the 0.4.0 release, so earlier detail lives in the git history and in the
change ledgers under [`docs/test/test-strategy.md`](docs/test/test-strategy.md)
and [`docs/requirements/system-requirements.md`](docs/requirements/system-requirements.md).

## [Unreleased]

### Fixed

- **The NB chip no longer overflows its own box.** It read `On · WIDE`, which
  did not fit and wrapped to a second line, making that chip taller than the
  rest of the row. It now shows just the filter — `WIDE`, `NAR`, `NONE` —
  matching ATT (`6 dB`) and AGC (`Slow`); the chip lights up when the noise
  blanker is on, so the `On ·` prefix was saying what the colour already said.

## [0.6.0] — 2026-07-21

### Added

- **Tap the mode you are already in to reach its alternate** — the reverse or
  opposite sideband, paired as the radio pairs them: LSB ⇄ USB, CW ⇄ CW-R,
  DATA ⇄ DATA-R. Tapping again returns you. AM and FM have no alternate and are
  unaffected. Previously that tap re-sent the mode already in effect and did
  nothing. (`FR-UI-ALT-01`)
- The diagnostics console reports the **worker loop's own rate** every few
  seconds — filter it on `perf`. It reads like
  `worker 340 loops/s, publish 20/s costing 12.4 ms/s`, and it is there so a
  report of "the spectrum is lagging" can be answered with a measurement.

### Changed

- **Controls no longer resize as their content changes** (`FR-UI-STABLE-01`).
  A control whose label varies over a known set now reserves the width of its
  widest possible label. The worst case was **ARM TX**, which grew to
  `TX ARMED — DISARM` on arming and shoved **PTT** and **EMERGENCY STOP**
  sideways — the two controls you least want moving under a cursor that may be
  reaching for them. Also fixed on the ATU buttons and the per-receiver volume
  and mute controls.

  Not yet a complete sweep: the connect button, the theme button, the filter
  shift/edge toggle, the APF widths and several numeric readouts still resize.

### Fixed

- **The radio's stream could fall behind, showing as a lagging spectrum and
  waterfall**, especially with the diagnostics console open. The worker
  published a full snapshot — both waterfalls, the spectrum and the whole radio
  state, deep-copied — on *every* read from the socket, hundreds of times a
  second, while the display only samples it ten times a second. That work
  competed directly with draining the socket. Publishing is now capped at
  20 Hz.

  *If you still see the stream fall behind, the new `perf` line is the thing to
  report — it says whether the worker is keeping up.*

## [0.5.0] — 2026-07-21

Audio: a per-receiver listening level, volume controls that can actually make
the K4's quiet stream loud, and — because this release began with a "no sound"
report that took hours to pin down — the diagnostics to tell you *why* there is
no sound, in one line instead of an afternoon.

### Added

- **Per-receiver volume and mute**, beside each spectrum pane's **A** / **B**
  badge. Balances the two receivers in your headphones. Local to the app: it
  does not touch the radio's AF gain, so it changes nothing at the front panel
  or for anyone else connected. Mute keeps the level, so unmuting returns to
  where you had it, and mute always starts clear. (`FR-RX-VOL-01`)
- **Audio diagnostics.** The console now reports `RX audio: N decoded / M
  played` and a per-channel **peak level in dBFS**, which separates "no audio
  arriving", "arriving but cannot be played", and "playing, but the audio
  itself is silent" — three cases that previously looked identical. Failures to
  open the output device or build the decoder are reported with their actual
  error instead of being swallowed.
- Two diagnostic examples: `cargo run -p k4-audio --example test_tone
  --features device` plays a tone through the real playback path (1 kHz left =
  Main, 600 Hz right = Sub), which proves or clears the whole client side in
  seconds; `--example list_audio` prints the devices the app can see.

### Changed

- **Volume controls read 0–100 %** and follow a perceptual (cubic) curve
  instead of being a raw multiplier. Unity sits near 40 % of the travel and the
  top reaches **+24 dB**, because the K4 can stream audio at around -45 dBFS —
  far too quiet for a noisy room at the old ceiling. Existing settings are
  migrated to the position that reproduces the same loudness, so upgrading does
  not change how loud your radio is.
- The per-pane **VOL** only attenuates; overall loudness is the master's job.
- Playback is clamped to the sample range, so the extra gain clips rather than
  distorting.
- The **TX** tag is gone from the transmitting spectrum pane — the accent
  border already says which pane transmits.

### Fixed

- **ESC in the diagnostics window** closed the Settings dialog in the main
  window and left the log window open. Key presses were handled without regard
  to which window they came from.

### Known: the K4 streams quietly, and `AG` does not change that

Measured on one K4: the streamed level sits around **-45 dBFS**, and **`AG`
does not control it** — neither the radio's front-panel knob nor `AG` sent over
the link changes the streamed level; both change the radio's own speaker
volume. Until a radio-side control is found, **the app's own Volume is the
lever**, which is why it now reaches +24 dB. Note that digital gain raises the
stream's noise along with the signal.

Two consequences worth knowing:

- **Turning the radio's AF down to quiet the shack will not quiet your stream —
  and turning it up will not raise it.** To listen remotely with a silent
  shack, switch the radio's internal speaker off (menu *Speaker, Internal*, or
  `ME0001.0;`) instead.
- If you hear nothing, the console's peak line now tells you whether the radio
  is sending sound at all.

## [0.4.1] — 2026-07-21

### Fixed

- **Control popups opened in the corner of the window instead of at the
  control** — a regression introduced in 0.4.0 while reducing the cost of
  tracking the pointer. The popup is anchored again where you opened it.

### Changed

- **Holding a control chip now opens its settings popup**, which is what the
  radio itself does — "Hold [ATTN] to bring up the attenuator controls",
  "hold [LEVEL] to bring up the noise blanker controls". Right-click still
  opens the same popup.

  This replaces the stand-in behaviours the holds carried in 0.4.0, from before
  those panels existed: holding **ATT** stepped the attenuator 3 dB, **NB**
  cycled the filter mode, and **AGC** switched AGC off. Each of those is now a
  control inside the corresponding popup, so nothing is lost — but if you had
  learned the old gestures, they have changed.

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

[0.6.0]: https://github.com/dc0sk/K4remote/releases/tag/v0.6.0
[0.5.0]: https://github.com/dc0sk/K4remote/releases/tag/v0.5.0
[0.4.1]: https://github.com/dc0sk/K4remote/releases/tag/v0.4.1
[0.4.0]: https://github.com/dc0sk/K4remote/releases/tag/v0.4.0
[0.3.0]: https://github.com/dc0sk/K4remote/releases/tag/v0.3.0
[0.2.3]: https://github.com/dc0sk/K4remote/releases/tag/v0.2.3
[0.2.2]: https://github.com/dc0sk/K4remote/releases/tag/v0.2.2
[0.2.1]: https://github.com/dc0sk/K4remote/releases/tag/v0.2.1
[0.2.0]: https://github.com/dc0sk/K4remote/releases/tag/v0.2.0
