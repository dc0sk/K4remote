# K4 Remote

A cross-platform **remote control panel for the Elecraft K4** transceiver, written in Rust with
an [iced](https://iced.rs) GUI: rig control, metering, spectrum + waterfall, full-duplex audio,
and transmit (voice + CW) — over Ethernet or USB/serial.

It is developed **requirements-first and test-driven**, with strict traceability from
stakeholder needs down to individual tests, enforced by a build gate.

> **Status:** v1 feature-complete; hardware bring-up pending.
> 79 hardware-free tests pass · clippy/fmt clean · traceability gate green.
> The only remaining work is validating audio / PTT / spectrum / serial against a real K4.

---

## Features

| Area | What it does |
|---|---|
| **Connect** | Plaintext Ethernet (9205, SHA-384 auth), **TLS-PSK** (9204), or **USB/serial** CAT — selectable in the UI, with auto-reconnect (bounded backoff). |
| **Control** | VFO A/B, band, mode, bandwidth, AGC, NB/NR, preamp, attenuator, RIT/XIT, split. |
| **Metering** | S-meter (bar count + dBm) with S-unit mapping. |
| **Spectrum** | Decodes the K4 dB/bin stream → live spectrum trace + scrolling waterfall (GPU canvas). |
| **Audio** | Full-duplex 12 kHz **Opus** — jitter buffer, resampling, cpal device I/O (L=Main, R=Sub). |
| **Transmit** | PTT, voice, and CW keying (`KZ`) — all behind an explicit **TX arm**, with an emergency stop and link-loss fail-safe. |
| **Operability** | Persisted connection profiles, **OS-keychain** password storage (secrets never written to config), structured diagnostics + a raw-CAT console. |

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
| `app` (`k4remote`) | iced GUI + background I/O worker |

The full requirements + concept baseline lives in [`docs/`](docs/) — the RE process, ID scheme,
and traceability rules are in [`docs/README.md`](docs/README.md). The K4/0 streaming protocol
was recovered from the GPLv3 [QK4](https://github.com/mikeg-dal/QK4) project and
**reimplemented clean-room** (interoperability facts only, no source copied); see
[`docs/references/external-references.md`](docs/references/external-references.md).

## Development

```sh
cargo test --workspace                       # 79 hardware-free tests
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
- Possible Phase-2/3: panadapter click-to-tune, an embedded CAT server for WSJT-X / loggers.

## License

See [LICENSE](LICENSE).
