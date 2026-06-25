# K4 Remote

A cross-platform **remote control panel for the Elecraft K4** transceiver, written in Rust
with an [iced](https://iced.rs) GUI — control, metering, spectrum + waterfall, full-duplex
audio, and transmit (voice + CW) over Ethernet or USB/serial.

Built **requirements-first and test-driven**, with strict traceability from stakeholder needs
down to tests. See [`docs/`](docs/) for the full requirements + concept baseline; the process
and ID scheme are in [`docs/README.md`](docs/README.md).

> Status: **v1 feature-complete, hardware bring-up pending.** 79 hardware-free tests pass; the
> traceability gate (`cargo xtask`) is green. The remaining work is L4 validation against a
> real K4 + audio device.

## Features

- **Transport (UI-selectable):** plaintext Ethernet (port 9205, SHA-384 auth), **TLS-PSK**
  Ethernet (9204), or **USB/serial** CAT — with auto-reconnect (bounded backoff).
- **Control:** VFO A/B, band, mode, bandwidth, AGC, NB/NR, preamp, attenuator, RIT/XIT, split.
- **Metering:** S-meter (bars + dBm) with S-unit mapping.
- **Spectrum + waterfall:** decodes the K4 dB/bin stream; GPU canvas render.
- **Audio:** full-duplex 12 kHz Opus (jitter buffer, resampling, cpal device I/O).
- **Transmit:** PTT, voice, and CW keying (`KZ`) — all behind an explicit **TX arm** with an
  emergency stop and link-loss fail-safe.
- **Operability:** persisted connection profiles, **OS-keychain** password storage (secrets
  never written to config), structured diagnostics + a raw-CAT console.

## Workspace

A Cargo workspace; the protocol core has **no UI/audio dependency** (testable without hardware):

| Crate | Role |
|---|---|
| `k4-protocol` | Binary framing, auth hashing, CAT codec + `RadioState`, CW, serial line decoder |
| `k4-transport` | `CatLink`/`Transport` traits; TCP (plaintext/TLS-PSK) + serial backends |
| `k4-session` | Keep-alive, link-loss + TX fail-safe, reconnect backoff, stream demux |
| `k4-stream` | Audio + PAN packet codecs; spectrum/waterfall render math |
| `k4-audio` | Opus codec, jitter buffer, resampler, cpal device I/O |
| `k4-sim` | Protocol simulator + loopback servers for hardware-free tests |
| `k4-config` | Profiles/prefs persistence (secret-free) + `SecretStore` (keychain) |
| `k4-diag` | Structured, levelled, bounded diagnostic log |
| `app` (`k4remote`) | iced GUI + background I/O worker |

Protocol provenance and the **GPLv3 clean-room note** (re: the QK4 reference) are in
[`docs/references/external-references.md`](docs/references/external-references.md).

## Build & run

Requires Rust 1.90+ and (for the default app build) system libs: **libopus**, **libasound**
(ALSA), **OpenSSL**, **libudev**. On Debian/Ubuntu:

```sh
sudo apt-get install -y libopus-dev libasound2-dev libssl-dev libudev-dev pkg-config
cargo run -p k4remote
```

Default app features: `audio-device`, `tls`, `keychain`, plus serial always-on. A minimal
build (`--no-default-features --features audio-device`) drops TLS/keychain.

## Development

```sh
cargo test --workspace          # 79 hardware-free tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
cargo xtask                     # requirement → test traceability gate (R3/R4)
```

Git hooks (enable with `git config core.hooksPath .githooks`): **pre-commit** runs fmt +
clippy; **pre-push** runs the test suite + `cargo audit`. CI is provided but disabled
(`.github/workflows/ci.yml.disabled` — rename to enable).

Feature-gated tests need their lib + feature: `cargo test -p k4-transport --features tls`.

## License

See [LICENSE](LICENSE).
