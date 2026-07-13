# K4 Remote

A cross-platform **remote control panel for the Elecraft K4** transceiver, written in Rust with
an [iced](https://iced.rs) GUI: rig control, metering, spectrum + waterfall, full-duplex audio,
and transmit (voice + CW) — over Ethernet or USB/serial.

It is developed **requirements-first and test-driven**, with strict traceability from
stakeholder needs down to individual tests, enforced by a build gate.

![K4 Remote main window](docs/screenshots/main.png)

> **Status:** v1 feature-complete; hardware bring-up pending.
> 144 hardware-free tests pass · clippy/fmt clean · traceability gate green.
> The only remaining work is validating audio / PTT / spectrum / serial against a real K4.
>
> *(Screenshots show the app driven by the protocol simulator; the panadapter fills in from a
> live radio's stream.)*

---

## Mode-adaptive UI

The panel is **operating-mode aware** (on by default, switchable in Settings): controls the
current mode doesn't use are dimmed or tucked away, and a fixed-height **mode strip** surfaces
the ones it does — so each mode stays lean without the layout jumping around.

**CW** — an APF / SPOT / DECODE strip appears, and the transmit panel shows keyer WPM, CW pitch
and QSK delay:

![RX frame in CW](docs/screenshots/rx-cw.png)

**FM** — the passband/filter controls dim (FM's filters are fixed) and a repeater-offset + PL/CTCSS
strip takes their place:

![RX frame in FM](docs/screenshots/rx-fm.png)

**DATA** — a sub-mode selector (DATA A / AFSK A / FSK D / PSK D) plus text decode:

![RX frame in DATA](docs/screenshots/rx-data.png)

The transmit panel adapts the same way (voice: VOX / compression / mic / DVR; CW: keyer timing),
and the VFO frames support click-to-tune digits, a clickable mode cycle, and optimistic stepping.

## Features

| Area | What it does |
|---|---|
| **Connect** | Plaintext Ethernet (9205, SHA-384 auth), **TLS-PSK** (9204), or **USB/serial** CAT — selectable in the UI, with auto-reconnect (bounded backoff). |
| **Control** | VFO A/B with **per-digit click tuning** + optimistic stepping, band, clickable **mode cycle**, bandwidth, LO/HI filter edges, AGC, NB/NR, preamp, attenuator, RIT/XIT, split. |
| **Mode-adaptive UI** | Per-mode control emphasis + mode strips (CW / voice / DATA / AM / FM), switchable in Settings; follows the active RX and the transmit VFO. |
| **Metering** | S-meter (bar count + dBm) with S-unit mapping; TX RF/ALC/SWR/COMP bars while transmitting. |
| **Spectrum** | Decodes the K4 dB/bin stream → live spectrum trace + scrolling waterfall + mini-pan (GPU canvas), with click-to-QSY and wheel tuning. |
| **Audio** | Full-duplex 12 kHz **Opus** — jitter buffer, resampling, cpal device I/O (L=Main, R=Sub). |
| **Transmit** | PTT, voice, and CW keying — all behind an explicit **TX arm**, with an emergency stop, link-loss fail-safe, and a configurable **PTT keyboard hotkey** (toggle or hold). |
| **K-Pod** | Optional Elecraft **K-Pod** USB control surface (`--features kpod`): the rocker assigns the knob to VFO A / VFO B / RIT-XIT (with indicator LEDs) and the encoder tunes it. The **F1–F8 switches** (tap + hold) run configurable CAT macros — set them in Settings → *K-Pod function switches* from a preset list or a free-form command string, seeded from the Elecraft sample macros. |
| **Operability** | Persisted connection profiles, **OS-keychain** password storage (secrets never written to config), **K4 settings export/import** (SHA-256-stamped `.cfg`), an optional separate **diagnostics window**, and a raw-CAT console. |

## Quick start

Requires **Rust 1.90+** and, for the default build, these system libraries:
**libopus**, **ALSA** (libasound), **OpenSSL**, **libudev**.

```sh
# Debian / Ubuntu
sudo apt-get install -y libopus-dev libasound2-dev libssl-dev libudev-dev pkg-config

cargo run -p k4remote
```

Default app features: `audio-device`, `tls`, `keychain` (serial is always on). A minimal build
without TLS/keychain: `cargo run -p k4remote --no-default-features --features audio-device`.

## How it's built

A Cargo workspace whose **protocol core has no UI or audio dependency**, so the bulk of the
logic is unit-tested without a radio or sound card:

| Crate | Role |
|---|---|
| `k4-protocol` | Binary framing, auth hashing, CAT codec + `RadioState`, CW, serial line decoder |
| `k4-transport` | `CatLink`/`Transport` traits; TCP (plaintext / TLS-PSK) + serial backends |
| `k4-session` | Keep-alive, link-loss + TX fail-safe, reconnect backoff, stream demux |
| `k4-stream` | Audio + panadapter packet codecs; spectrum/waterfall render math |
| `k4-audio` | Opus codec, jitter buffer, resampler, cpal device I/O |
| `k4-sim` | Protocol simulator + loopback servers for hardware-free tests |
| `k4-config` | Profiles/prefs persistence (secret-free) + `SecretStore` (OS keychain) |
| `k4-diag` | Structured, levelled, bounded diagnostic log |
| `app` (`k4remote`) | iced GUI + background I/O worker; pure `ui.rs` view-model for testable layout logic |

The full requirements + concept baseline lives in [`docs/`](docs/) — the RE process, ID scheme,
and traceability rules are in [`docs/README.md`](docs/README.md); the mode-adaptive UI concept is
in [`docs/concept/mode-aware-ui.md`](docs/concept/mode-aware-ui.md). The K4/0 streaming protocol
was recovered from the GPLv3 [QK4](https://github.com/mikeg-dal/QK4) project and
**reimplemented clean-room** (interoperability facts only, no source copied); see
[`docs/references/external-references.md`](docs/references/external-references.md).

## Development

```sh
cargo test --workspace                       # 144 hardware-free tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
cargo xtask                                  # requirement → test traceability gate (R3/R4)
```

Feature-gated tests need their feature, e.g. `cargo test -p k4-transport --features tls`.

Git hooks (enable once: `git config core.hooksPath .githooks`) run **fmt + clippy** on commit
and the **test suite + `cargo audit`** on push. A CI workflow is included but disabled
(`.github/workflows/ci.yml.disabled` — rename to enable).

## Roadmap

- **L4 hardware bring-up** — validate audio, PTT, spectrum, and the serial path against a real K4.
- App-level keychain/serial polish from real-world use.
- Possible Phase-2/3: an embedded CAT server for WSJT-X / loggers.

## License

See [LICENSE](LICENSE).
