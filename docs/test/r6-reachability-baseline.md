---
title: "R6 baseline — public functions with no production reference"
status: Draft
version: "0.6"
updated: 2026-10-28
authors:
  - Simon Keimer (DC0SK)
---

# R6 baseline — public functions with no production reference

Rule **R6** (`cargo xtask`, NFR-TEST-01): every public function in `crates/*/src` and `app/src`
must be reachable from production code. Reachable means its name occurs in a live context: code
outside any function, `main`, a method of an `impl Trait for Type` block, an attribute's strings,
or the body of a function that is itself reachable. Test code (a test-cfg item, an out-of-line
test module, `tests/` directories), comments, strings and `use` lines do not count, and neither
does a call from a function nothing reaches — so a chain hanging off a dormant function is listed
link by link. Tested code the product never runs looks covered and traceable, and it isn't: `KZF`
(#222) and paddle CW (#227) both got through that way.

The check follows names, not types, so it can still miss things (two functions of one name share a
fate; a field named like a function keeps it alive), but a function it flags really is unreached.
Until 2026-10-16 it only counted a name's occurrences, which let orphan chains, same-named
functions and attribute-prefixed definitions through (audit T, U, X). This file is therefore a
**ratchet**, not a waiver list:

- a function that is unreferenced and **not** listed here fails the build: wire it up, make it
  private, delete it, or add it here with a reason;
- an entry here that is **no longer** unreferenced (now called, renamed or gone) also fails the
  build, so the list can only shrink.

Format: one line per function, `` - `path:name` — reason ``. Retrofitted 2026-09-29 with the 29
found that day; the reasons were checked against each function's callers.

## Dormant — tracked capability, not built

- `crates/k4-session/src/lib.rs:send_cw` — DORMANT(#227): paddle CW (FR-TX-CW-01) not built; kept arm-gated for a paddle input.
- `crates/k4-session/src/lib.rs:set_cw_delay` — DORMANT(#227): `KZL` (FR-TX-CW-02) belongs to paddle CW; wire with it or retire.
- `crates/k4-protocol/src/cw.rs:encode_kz` — DORMANT(#227): only `send_cw` calls it; it follows `send_cw`.
- `crates/k4-protocol/src/cw.rs:encode_kzl` — DORMANT(#227): only `set_cw_delay` calls it; it follows `set_cw_delay`.

## Superseded


## CAT encoders already waived under R5

See `r5-unreached-encoders.md` for each one's reason.

- `crates/k4-protocol/src/cat.rs:menu_open` — R5 waiver (FR-MENU-01).
- `crates/k4-protocol/src/cat.rs:menu_query_def` — R5 waiver (FR-MENU-01).
- `crates/k4-protocol/src/cat.rs:set_band_sub` — R5 waiver (FR-VFO-04).
- `crates/k4-protocol/src/cat.rs:set_nb` — R5 waiver (FR-RX-04).
- `crates/k4-protocol/src/cat.rs:set_rit` — R5 waiver (FR-VFO-05).
- `crates/k4-protocol/src/cat.rs:set_tx_power` — R5 waiver (FR-TX-02).
- `crates/k4-protocol/src/cat.rs:set_xit` — R5 waiver (FR-VFO-05).

## Test seams — public so a `tests/` file can reach them

- `crates/k4-spot/src/freedv_source.rs:set_connector` — replaces the connector in `tests/freedv_source.rs`.
- `crates/k4-spot/src/freedv_source.rs:set_refresh` — unclamped refresh for `tests/freedv_source.rs`.
- `crates/k4-spot/src/freedv_source.rs:stations` — roster count asserted by `tests/freedv_source.rs`.
- `crates/k4-spot/src/mqtt_source.rs:set_plain_connector` — replaces the connector in `tests/mqtt_source.rs`.
- `crates/k4-transport/src/lib.rs:push_inbound` — feeds the mock transport in `tests/transport.rs`.
- `crates/k4-transport/src/lib.rs:poll_cat` — the TCP transport's CAT-only read, used by `tests/tcp.rs`, `tests/tls.rs` and `tests/serial.rs`; production reads every frame through `poll_frames`.
- `crates/k4-transport/src/lib.rs:psk_loopback` — the TLS-PSK loopback server in `tls_support`, for `tests/tls.rs`.
- `crates/k4-stream/src/audio.rs:is_opus` — asserted by `tests/codecs.rs`.
- `crates/k4-audio/src/codec.rs:stereo` — builds the stereo Opus packet `tests/opus.rs` decodes, the shape the K4 streams for RX (FR-AUD-04); the app itself only encodes mono.
- `crates/k4-spot/src/style.rs:contrast` — WCAG ratio; its in-file tests use it to hold spot nameplate colours readable.
- `crates/k4-spot/src/style.rs:luminance` — only `contrast` calls it (the WCAG relative luminance); it follows `contrast`.
- `crates/k4-spot/src/mqtt.rs:buffered` — lets the in-file tests bound the decoder's buffer.

## Independent references in tests

Held beside the production mapping as a second derivation, so a test can check `column_to_bin` and
`axis_ticks` against something other than themselves. No acceptance cell cites them (R7).

- `crates/k4-stream/src/render.rs:hz_to_x` — frequency→pixel reference for the axis and scroll tests (FR-PAN-07, #229).
- `crates/k4-stream/src/render.rs:row_scroll_px` — pixel-offset reference that `fr_pan_09_agrees_with_row_scroll_px` holds `column_to_bin` to (FR-PAN-06, #229).

## CPU references of GPU code

- `app/src/spectrum.rs:waterfall_rgba` — since FR-UI-26 the canvas draws through `WfRing` (only new rows coloured); this full redraw is the reference the ring (`fr_ui_26_incremental_ring_equals_a_full_redraw`) and the GPU golden are held equal to.
- `crates/k4-stream/src/gpu_waterfall.rs:lut_index` — the WGSL shader does this rounding; this is the CPU reference its test checks.
- `crates/k4-stream/src/gpu_waterfall.rs:shader_bin` — the WGSL lookup restated in Rust, held to `column_to_bin` by a test (FR-PAN-12).

## Unused — API for a feature not built

- `crates/k4-kpod/src/lib.rs:configure_packet` — K-Pod encoder scale/beeper packet; no setting sends it.
