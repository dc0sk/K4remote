---
title: "Packaging & Distribution"
status: Draft
version: "0.1"
updated: 2026-07-03
authors:
  - Simon Keimer (DC0SK)
---

# Packaging & distribution (NFR-PORT-02 / NFR-PKG-01)

K4 Remote builds from one Rust workspace for **Linux x86_64**, **Linux arm64
(Raspberry Pi OS)**, **Windows x86_64**, and **macOS**. CI
(`.github/workflows/ci.yml`) builds, tests, lints, and runs the traceability gate
on all four; the release workflow (`.github/workflows/release.yml`) publishes
installable artifacts on a `vX.Y.Z` tag.

## Runtime / build libraries (default features)

| Library | Purpose (feature) | Debian/RPi pkg | Arch pkg |
|---|---|---|---|
| libopus | Opus audio codec (`opus`) | `libopus-dev` | `opus` |
| ALSA | audio device I/O (`audio-device`) | `libasound2-dev` | `alsa-lib` |
| OpenSSL | TLS-PSK (`tls`) | `libssl-dev` | `openssl` |
| libudev | serial transport (`serial`) | `libudev-dev` | `systemd-libs` |
| libsecret | keychain (`keychain`) | `libsecret-1-dev` | `libsecret` |

On Windows/macOS, OpenSSL is built from source via the `vendored-tls` feature
(`cargo build -p k4remote --features vendored-tls`), so only libopus needs a
system package (Homebrew `opus` / vcpkg `opus`).

## Icons

`packaging/icons/` holds the master SVG, rasterized PNGs (16–512), a
font-independent `k4remote.svg` (hicolor scalable), and `k4remote.ico` (Windows).
Installed into the hicolor theme (Linux), an `.icns` (macOS), and embedded in the
`.exe` (Windows).

## Debian / Raspberry Pi OS (`.deb`)

Metadata lives in `app/Cargo.toml` (`[package.metadata.deb]`).

```sh
cargo install cargo-deb
cargo build -p k4remote --release
cargo deb -p k4remote --no-build      # → target/debian/k4remote_<ver>_<arch>.deb
```

Build the arm64 package on an arm64 host (or a Raspberry Pi) the same way.

## Arch / Manjaro (`PKGBUILD`)

`packaging/PKGBUILD` builds from the tagged source.

```sh
cd packaging && makepkg -si          # build + install
```

## Windows (`.zip` + installer)

The release workflow ships two Windows artifacts: a `.zip` of `k4remote.exe`
(portable) and a **`setup.exe` installer** built with
[Inno Setup 6](https://jrsoftware.org/isinfo.php) from
`packaging/windows/k4remote.iss`. The installer places the app under
*Program Files*, adds Start-menu (and optional desktop) shortcuts, and registers
an uninstaller; the app version is passed in from the release tag.

Build it locally on Windows (Inno Setup installed):

```powershell
cargo build -p k4remote --release --features vendored-tls
iscc /DMyAppVersion=0.2.2 packaging\windows\k4remote.iss   # → k4remote-windows-x86_64-setup.exe
```

## macOS (`.app` / `.dmg`)

Bundle metadata lives in `app/Cargo.toml` (`[package.metadata.bundle]`).

```sh
cargo install cargo-bundle
cargo bundle -p k4remote --release   # → target/release/bundle/osx/K4 Remote.app
```
