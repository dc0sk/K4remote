---
title: "Implementation Plan — KAT500 / KPA500 / KPA1500 Station Accessories"
status: Draft
version: "0.1"
updated: 2026-07-19
authors:
  - Simon Keimer (DC0SK)
---

# Implementation Plan — KAT500 / KPA500 / KPA1500 Station-Accessory Support

> **Provenance.** Researched and drafted by an AI agent on 2026-07-19 from the vendor
> documentation in `docs/references/external/`, commissioned by DC0SK. Claims carry their
> evidence source inline. Anything marked *inference*, *recalled*, or *web* has **not** been
> confirmed against hardware or against vendor documentation held in this repo — see the
> open-questions section before acting on it. This is a research input, not an approved
> baseline.

**Status:** Proposal (pre-ADR) · **Date:** 2026-07-19 · **Scope:** Elecraft KAT500 automatic
antenna tuner, KPA500 (500 W) and KPA1500 (1500 W) amplifiers, integrated into K4 Remote so a
fully remote station (radio + amp + tuner) can be operated and monitored.

Citation convention: `[KAT p.N]` = *KAT500 Automatic Antenna Tuner Serial Command Reference*
(rev. 9/6/2023, fw 02.12); `[K5 p.N]` = *KPA500 Programmer's Reference Rev A2*;
`[K15 p.N]` = *KPA1500 Programming Reference V3* (rev. 6/1/2026, fw 03.0);
`[K5OM p.N]` / `[KATOM p.N]` / `[K15OM]` = the respective owner's manuals;
`[K4PRG]` = K4 Programmer's Reference rev. D12. Page numbers are **PDF pages** of the file in
`docs/references/external/`. Facts are **verified** against these documents unless explicitly
marked *(inference)*.

---

## 1. Summary + recommendation

**Recommendation in one paragraph.** Add one new pure-protocol crate,
`crates/k4-accessory/`, modeled directly on the `k4-kpod` precedent (dependency-free codec +
state core, unit-tested without hardware, real I/O behind a cargo feature and a dedicated
worker-owned thread). Reach the devices over **their own links, not through the K4**: the
KPA1500 has a native TCP command server (default port 1500) and is connected directly over
the network; the KPA500 and KAT500 are RS-232-only and are reached either by a local serial
port (station-side use, reusing the existing `SerialTransport` pattern) or by a **raw
TCP↔serial bridge** (e.g. `ser2net`) at the remote site — the same `;`-terminated ASCII
flows over both, so one `AccessoryLink` byte-channel abstraction (a sibling of
`k4-transport`'s `CatLink`, without the K4 binary envelope or auth) covers all cases. The K4
`EC` pass-through is explicitly **rejected** as a control channel (one-way, and it disables
RS-232 input until radio restart — see §4.2). Accessory state lives in per-device state
structs in the new crate — *not* inside `RadioState` — polled by the worker (none of these
devices push unsolicited status) and surfaced through `UiSnapshot` as optional
amp/tuner sub-snapshots. Ship the **KPA1500 first** (native network, richest protocol,
built-in ATU covers the tuner story for KPA1500 owners), then KPA500+KAT500 over the bridge,
then the safety interlocks and the guided remote-tune workflow.

**Why this shape.**
- It matches ADR-01/ADR-02 (layered crates, transport behind a trait) and reuses the proven
  `k4-kpod` opt-in accessory pattern (`FR-KPOD-05`: runtime opt-in, app fully functional with
  or without the device, retry discovery, survive unplug).
- It keeps `k4-protocol`/`RadioState` K4-pure (`NFR-MAINT-01`); accessories have their own
  lifecycle, links, and failure modes.
- It answers the *remote* problem honestly: the only device Elecraft made remotable is the
  KPA1500; for the other two the practical, supportable answer is a raw network-serial
  bridge, which our transport seam absorbs with ~100 lines.

---

## 2. Protocol findings per device

### 2.1 KAT500 (tuner) — *KAT500 Serial Command Reference*

- **Physical/link:** ASCII over the rear "PC DATA" serial port; 4800/9600/19200/38400 bit/s,
  8N1, **no flow control**; utility auto-detects speed by sending `;` at each speed
  [KAT p.4]. 38400 recommended (`BR3;`) [KAT p.12]. No network interface on the device.
- **Framing:** GET/SET commands, `;` terminator, case-insensitive input, UPPERCASE responses
  (except boot block `kat500;` to `I;`) [KAT p.4, p.21]. **SETs generally produce no
  response** — verify with a follow-up GET; exception: `FT;`/`T;`/`FTNS;` produce a delayed
  `FT;` response when the tune completes [KAT p.4, p.20, p.29].
- **Flow control is application-level:** ≤64 bytes may be stacked; wait for responses
  [KAT p.4].
- **Sleep/wakeup:** with `SL1;` the MCU sleeps when idle; ~100 ms wake-up during which
  characters are lost — send `;` at ~100 ms intervals until it echoes `;` [KAT p.4, p.27].
- **No unsolicited push** → client must poll. *(verified by absence: no AI-like mode exists
  anywhere in the reference).*
- **Command families** (mnemonics verified):
  - *Identity/admin:* `I` (→`KAT500;`) [p.21], `RV` [p.25], `SN` [p.28], `BR` [p.12],
    `PS`/`PSI` power on/off logical [p.25], `SL` [p.27], `RST0/RST1` [p.25], `EEINIT` [p.15].
  - *Operating state:* `MD` mode `B`ypass/`M`anual/`A`uto [p.23], `AN` antenna 0(next)/1/2/3
    [p.9], `BN` band 00–10 (160 m–6 m, same table as K4 `BN`) [p.11], `BYP` `N`/`B` [p.12],
    `F`/`FA`/`FB`/`FX`/`FY`/`FC` frequency set/track (FA/FB take 9–11-digit Hz and trigger a
    memory-recall tune for the TX VFO) [p.16, p.20–21], `FT0/FT1` TX-VFO select [p.20],
    `IF` (accepts a pasted transceiver IF response to derive TX VFO/XIT) [p.21].
  - *Tuning:* `T`/`FT` full-search tune (delayed `FT;` on completion), `FTNS` tune-no-save,
    `CT` cancel, `TP` tune-poll (`TP1;` while tuning), `MT fffff;` memory-recall tune,
    `SM` save memory [p.20, p.14, p.23, p.28–29].
  - *Telemetry:* `VSWR` (`VSWR nn.nn;`), `VSWRB` (bypass SWR), `VFWD`/`VRFL` raw ADC counts
    0–4095 [p.30].
  - *Faults:* `FLT` → `FLTc;` with c = 0 none, 1 no-match, 2 power-above-design-limit,
    3 power-above-safe-relay-limit, 4 SWR-exceeds-amp-key-interrupt-threshold; **the
    amplifier key line is kept interrupted during any fault**; `FLTC` clears [KAT p.19].
  - *Amp-protection plumbing:* `AKIP` amplifier-key-interrupt power threshold (set **1500**
    for a KPA500 whose T/R switching tolerates hot key-line interruption) [p.7–8], `AMPI0/1`
    manual key-line interrupt relay [p.9], `ST bbt` per-band SWR thresholds, type `K` =
    amp-key-interrupt SWR threshold (default 2:1) [p.29].
  - *Low-level relays (diagnostic/advanced):* `C hh`/`L hh` capacitor/inductor bitmaps in
    hex, `SIDE T/A`, `ATTN` [p.13, p.22, p.26, p.10]; per-band config `AE`, `AP`, `AFT`,
    `AB`, `FDT` [p.6–7, p.10, p.18].
- **Frequency tracking at the station:** the KAT500 follows the K4 via **AUXBUS** and the
  BAND0–3 lines on the 15-pin ACC cable (E850463) [KATOM p.7 wiring table; KAT p.18 `FDT`
  "K3/K3S/K4 AUXBUS"; KAT p.20 `FX`]. So band/frequency-follow is hardware-side when the
  station is cabled per Elecraft's standard diagram — the client does not need to stream
  frequency to the tuner *(inference from the above, marked for hardware confirmation)*.

### 2.2 KPA500 (500 W amp) — *KPA500 Programmer's Reference Rev A2*

- **Physical/link:** RS-232 "PC" port, 4800–38400 bit/s (`^BRP`) [K5 p.2]; separate XCVR
  port (`^BRX`). No network interface.
- **Framing:** all commands prefixed with **`^`**, `;` terminator; GET = mnemonic + `;`;
  SET/RSP data formats identical unless noted; malformed/out-of-range commands are silently
  ignored; bare `;` is echoed (liveness probe) [K5 p.1].
- **Command set is small (~21 commands)** [K5 p.1 table]:
  - *Control:* `^OS` operate/standby (0/1) [p.3], `^ON0;` power **off** (RSP `^ON1;` when
    on; *no response when off*) [p.3], `^BN` band 00–10 (same table) [p.2], `^BC` standby-
    on-band-change [p.2], `^NH` INHIBIT-input enable [p.3], `^TR` T/R delay 0–50 ms [p.4],
    `^SP` fault speaker [p.3], `^FC` fan minimum 0–6 [p.3], `^AL` ALC threshold (per-band)
    [p.2], `^PJ` power adj 80–120 (per-band) [p.3], `^XI` radio interface type [p.4].
  - *Telemetry (GET-only):* `^WS` → `^WSppp sss;` power watts + SWR in tenths ("implied
    decimal", `000` SWR when not transmitting) [p.4], `^VI` volts/amps (implied decimal
    after 2nd digit) [p.4], `^TM` PA temp 0–150 °C [p.4], `^SN`, `^RVM` [p.3].
  - *Faults:* `^FL` → `^FLnn;` **decimal** fault identifier, `00` = none; `^FLC;` clears
    [p.3]. The Programmer's Ref has **no fault-code table**; the owner's manual's table maps
    FAULT NO. 0–15 to causes (0 comm failure, 2 HI CURR, 4 HI TEMP, 6 PWRIN HI, 8 60V HIGH,
    9 REFL HI, 11 PA DISS, 12 POUT HI, 13 60V FAIL, 14 270V ERR, 15 GAIN ERR) with
    attenuator-vs-hard fault classes [K5OM p.38 (manual p.37)]. *(inference: `^FLnn` returns
    those same numbers — consistent with "FAULT NO." being shown on a K3 via AUXBUS
    [K5OM p.38 note 1]; confirm on hardware.)*
  - *Remote power-on:* main firmware has **no power-on command**; when "off" the KPA500 is
    in its boot loader, where single-character (no `;`) upper-case commands apply: `P` =
    power on (no response), `I` → `KPA500` [K5 p.5]. `^ON0;` powers off [p.3].
- **No unsolicited push** → poll. *(verified by absence.)*

### 2.3 KPA1500 (1500 W amp + built-in ATU) — *KPA1500 Programming Reference V3*

- **Physical/link — three concurrent host channels** [K15 p.6]:
  1. "Host PC" USB (FTDI serial), 4800–**230400** bit/s 8N1 (`^BRP`) [p.6, p.19];
  2. **TCP command server, default port 1500** (changeable via `^CP`) — same command set;
  3. **UDP server on the same port** — one command per datagram, at most one response,
     may be dropped. "The Host PC USB connector, XCVR SERIAL HOST, a single TCP client and
     any number of UDP clients may be concurrently active." [K15 p.6]
  - Network config: `^DH` DHCP, `^IP`/`^GW`/`^NM` static, `^MA` MAC, `^WL` Wake-on-LAN
    magic packet [p.24, p.33, p.31, p.37, p.55].
- **Framing:** leading `^` + `;` terminator; case-insensitive input, UPPERCASE responses
  (boot block: `^kpa1500;`); **"the position of the semicolon relative to the leading caret
  is used for validity checking"** — no added spaces, no omitted leading zeros; SETs
  generally silent, delayed responses for `^FT` (tune complete) and `^TX`/`^RX` [K15 p.6–7].
- **Wakeup/power:** when "off", the MCU sleeps; wake by sending `;`s then `^ON1;` on USB, or
  WoL if enabled; `^ON0;` off, `^ON/;` toggle (fw ≥03.02); `^RV`/`^RVM`/`^SN`/`^I`/`^ON`
  work while "off" [K15 p.8, p.39, p.44, p.46].
- **Command families** (superset of KPA500 + ATU + network):
  - *Control:* `^OS` oper/stby (OPER transition **clears any current fault except
    temperature**) [p.39], `^OP` power-on mode [p.39], `^BN` band (same 00–10 table, "same
    as K3, K3S, KX2, KX3, and K4") [p.17 + p.8], `^AN` antenna 1–32 [p.15], `^BC`, `^TR`,
    `^FC` fan 0–5, `^NH` TX-inhibit enable (see contradiction §7), `^SP` alarm tone,
    `^BP T/H nn` front-panel button simulate [p.18], `^PF` PF-key macros [p.41].
  - *Telemetry (GET-only):* `^WS` (4-digit watts + SWR tenths; "provided for KPA500
    compatibility, but the KPA500 uses 3 digits for watts") [p.55], `^SW` SWR tenths [p.50],
    `^PWF`/`^PWR`/`^PWI`/`^PWD` fwd/refl/input/dissipated watts [p.43], `^VI` (KPA500-compat
    volts/amps) [p.54], `^PC` PA amps [p.40], `^TM` temp °C [p.51], `^VM1/2/3/5/H` supply
    rails in mV [p.54–55], `^FS` fan speed [p.30], `^LQ` full front-panel LED bitmap (power
    bar, SWR bar, FAULT/OVR/ANT/ATU/OPER/TX) — a one-command dashboard [p.35],
    `^VG` T/R state diagnostics [p.54], `^TQ` key status 0–3 (fw ≥3.07) [p.52].
  - *Faults:* `^FL` → `^FLhh;` — **two hex digits** (00 none, 10 watchdog, 20 PA current,
    40 temp, 60/61 input-power/gain-low, 70 invalid frequency, 80–85 supply rails, 90 refl
    power, 91 SWR very high (~18:1 "antenna not connected"), 92 ATU no-match, B0 dissipated,
    C0/C1 fwd power (C1 = fwd power too high for current ATU setting), F0 gain high);
    **"Faults cause the KPA1500 to switch to Mode STBY"**; `^FLC;` clears (not Mode);
    `^SF` fault log with timestamps; `^OC`/`^AS` overdrive-attenuator code; `^AD`
    attenuator reason text [K15 p.29, p.45, p.38, p.10].
  - *Built-in ATU:* `^FT` start full-search tune (delayed `^FT;` on completion), `^FE`
    cancel, `^TP` tune poll, `^AI` inline/bypass relays, `^AM` ATU mode Inline/Bypassed
    (per band/antenna forms), `^SB` bypass SWR, `^SM` save setting, `^EB`/`^EM` erase,
    `^CR`/`^LR`/`^SI` cap/inductor/side relays, `^ST A/B/N/S` SWR thresholds, `^HS` HiSWR
    retune, `^TB` permitted power for a given bypass SWR, `^DA`/`^DF` display settings
    [K15 p.30, p.28, p.51, p.13–14, p.45–49, p.22, p.36, p.46, p.51, p.23].
  - *Software keying (fw ≥3.07):* `^TX;` or `^TXnn;` (nn = 1–99 s timeout) simulates KEY IN;
    `^RX;` cancels; **designed dead-man behaviour**: "external control software … send
    something like `^TX60;` periodically … Should connection to the control software be
    lost, the amplifier will turn off ^TX when the timeout expires"; needs ~5 ms lead before
    exciter RF; hardware KEY IN line always ORs in [K15 p.53, p.44, p.52].
  - *Frequency tracking:* `^FR fffff;` (kHz) lets a program steer ATU memory recall without
    transmitting; `^FQ` TX frequency counter (8 kHz granularity); with RADIO TYPE 4 = **K4**
    the amp takes band + frequency from the 15-pin ACC cable BAND0–3 + AUXBUS, "K4 provides
    transmit frequency with 1 kHz granularity" [K15 p.30, p.57].
- **No unsolicited push** → poll (delayed `^FT;`/`^TX;`/`^RX;` responses aside).
  *(verified by absence.)*
- **Owner's-manual errata check:** Errata Rev B1-2 contains only cooling-clearance, TX-sample
  level, page-reference and MCU-version typo corrections — **no protocol impact**
  [Errata p.1].

### 2.4 The remote gap: `KAT500-Remote` / `KPA500-Remote`

Elecraft's remote apps are a Windows host/client pair: the host opens the COM port and
serves TCP with a **username/password** of its own; the client speaks the host's protocol
[KAT500-Remote p.1–2; KPA500-Remote p.1–2]. That client↔host wire protocol is
**undocumented** — there is no spec to implement against. Hence the recommendation to use a
raw TCP↔serial bridge (documented, trivial, protocol = the device's own ASCII) rather than
trying to interoperate with Elecraft's host application.

### 2.5 Can the K4 proxy? — the `EC` command

The K4 Programmer's Reference D12 has `EC` ("Echo Command to RS232 Port for KPA1500
control, etc.; **SET only**"): `EC ^AN0;` forwards `^AN0;` out the K4's RS-232 port; hex
form `ECx…` for binary. **But**: "To prevent any conflict with unknown data being sent by
external devices, **all incoming data on the RS232 port will be ignored after sending an EC
set command until the radio is restarted**" [K4PRG, `EC`]. So the K4 can *blind-fire*
commands at an attached amp but can never return a response — no telemetry, no fault
monitoring, and a destructive side effect. Additionally the K4 `OM` response flags amp
presence: `L` = generic linear amp detected, `1` = KPA1500 detected [K4PRG, `OM`] — useful
as a *detection hint* only.

---

## 3. What the three share and where they differ

| Aspect | KAT500 | KPA500 | KPA1500 |
|---|---|---|---|
| Prefix | none | `^` | `^` |
| Terminator / GET-SET model | `;`, silent SETs, delayed `FT;` [KAT p.4] | `;`, SET+GET verify [K5 p.1] | `;`, silent SETs, delayed `^FT;`/`^TX;` [K15 p.6] |
| Transport on device | RS-232 only, ≤38400 [KAT p.4] | RS-232 only, ≤38400 [K5 p.2] | USB-serial ≤230400 **+ TCP/UDP port 1500** [K15 p.6] |
| Null `;` liveness/wake probe | yes [KAT p.4] | yes (echoed) [K5 p.1] | yes [K15 p.9] |
| Band table `BN` 00–10 = K4's | yes [KAT p.11] | yes [K5 p.2] | yes [K15 p.17] |
| Unsolicited status push | none | none | none |
| Fault query / clear | `FLT`/`FLTC`, single digit 0–4 [KAT p.19] | `^FL`/`^FLC`, decimal nn [K5 p.3] | `^FL`/`^FLC`, **hex** hh + `^SF` log [K15 p.29, p.45] |
| Fault → safe state | key line kept interrupted [KAT p.19] | hard fault → STBY [K5OM p.38] | fault → STBY [K15 p.29] |
| Power/SWR telemetry | `VSWR`, `VFWD`/`VRFL` [KAT p.30] | `^WS` (3-digit W) [K5 p.4] | `^WS` (4-digit W), `^SW`, `^PWF/R/I/D` [K15 p.55, p.43] |
| Oper/Stby | n/a (MD B/M/A instead) | `^OS` [K5 p.3] | `^OS` [K15 p.39] |
| Tune control | `T`/`FT`/`FTNS`/`CT`/`TP` [KAT p.20, p.29] | n/a (no ATU) | `^FT`/`^FE`/`^TP` [K15 p.30, p.28, p.51] |
| ATU relay model (hex C/L bitmaps, SIDE, BYP) | `C`/`L`/`SIDE`/`BYP` [KAT p.13, p.22, p.26] | n/a | `^CR`/`^LR`/`^SI`/`^AI` — same concept, different relay tables [K15 p.22, p.36, p.46] |
| Remote power on | `PS1;` (MCU always powered) [KAT p.25] | boot-loader `P` (no `;`) [K5 p.5] | `^ON1;` after wake `;`s; WoL [K15 p.8, p.39, p.55] |
| Frequency follow from K4 | AUXBUS + BAND lines (hardware) [KAT p.18; KATOM p.7] | BAND lines/ACC (hardware) *(inference)* | ACC BAND0–3 + AUXBUS, RADIO TYPE 4 [K15 p.57] |
| Software keying | n/a (passive; AMPI relay) | none | `^TX/^RX/^TQ` with dead-man timeout [K15 p.52–53] |

**Sharing verdict:** one line-codec (semicolon framing, `^`-optional mnemonic + args), one
band table, one "poll scheduler" and one wake-probe routine serve all three. Command sets and
state structs differ enough (tuner vs amp; decimal vs hex faults; 3- vs 4-digit watts) that
they should be three typed modules, not one parameterized codec. KPA500's command set is
nearly a subset of KPA1500's, but response field widths and fault encodings differ — do
**not** unify their parsers; share only via a common `AmpCommon` view for the UI
*(inference/design choice)*.

---

## 4. Architecture

### 4.1 Crate/module layout (new: `ARC-16`..`ARC-18` suggested)

```
crates/k4-accessory/            # NEW — pure protocol + state, NO I/O deps (like k4-kpod)
├─ src/lib.rs                   # AccessoryKind, shared Band type (reuse the BN table),
│                               # line assembly (`;`-splitting incl. responses containing
│                               # spaces), wake-probe helper, PollPlan scheduler types
├─ src/kat500.rs                # Kat500Cmd encode / Kat500Resp decode / Kat500State
├─ src/kpa500.rs                # Kpa500Cmd / Kpa500Resp / Kpa500State (+ fault-code names
│                               # from K5OM p.38 table)
├─ src/kpa1500.rs               # Kpa1500Cmd / Kpa1500Resp / Kpa1500State (+ hex fault table
│                               # K15 p.29, ^LQ bitmap decode K15 p.35)
└─ src/sim.rs (dev)             # scripted device simulators for tests (ADR-09 spirit;
                                # or place in k4-sim behind a feature)
```

- `k4-transport` gains a small **`RawTcpTransport`** (a TCP byte channel with *no* K4
  envelope, no SHA-384 auth, configurable read timeout) usable for both the KPA1500's native
  port-1500 server and a ser2net bridge. The existing `SerialTransport`
  (CAT-only raw-line adapter, `serial` feature) is reused as-is for local serial. Both are
  exposed to the accessory layer through a minimal trait — either the existing
  `Transport` (send/recv bytes) or a new `AccessoryLink { send_line(&str), poll_lines() ->
  Vec<String> }` implemented over `Transport`. **Do not** route accessories through
  `CatLink`/`FrameCodec` — there is no binary envelope on these links.
- `app/src/worker.rs` gains an `accessory` module exactly parallel to the existing
  `mod kpod` (worker.rs:972): a dedicated thread per configured device owning the blocking
  I/O, a channel of typed events back to the worker tick, `service()` called from the worker
  loop, enable/disable via `WorkerCmd::SetAccessory…` mirroring `SetKpodEnabled`
  (worker.rs:949). Cargo feature `accessories` in `app/Cargo.toml` default-on (pure code;
  the heavy deps — none — don't justify default-off; serial is already always enabled for
  the app).
- `k4-config::Prefs` gains per-device profile fields (kind, endpoint = serial path+baud |
  host:port, enabled flag, poll rates), following the `kpod_enabled` precedent
  (k4-config/src/lib.rs:80).

### 4.2 Transport (Key question 1 — the answer)

| Deployment | KAT500 | KPA500 | KPA1500 |
|---|---|---|---|
| App runs *at* the station (LAN to K4) | local serial (`SerialTransport`) | local serial | TCP :1500 (or USB serial) |
| App truly remote (WAN) | **ser2net/raw-TCP bridge** on the site PC/RPi → device serial | same bridge, second port | **direct TCP :1500** (through the same VPN the K4 traffic should use, NFR-SEC-02) |

Rejected alternatives, with reasons:
1. **K4 `EC` pass-through** — write-only; *kills* RS-232 input until radio restart
   [K4PRG `EC`]; cannot poll `^WS`/`^FL`; unacceptable for a safety-relevant amp UI. May be
   noted in docs as a last-ditch "blind command" hack; not implemented.
2. **Elecraft KPA500-Remote/KAT500-Remote host protocol** — undocumented proprietary
   client/host wire format [KPA500-Remote p.1]; nothing to implement against.
3. **KPA1500 UDP** — lossy by design, "may be discarded" [K15 p.6]; fine for a fast meter
   poll someday, wrong for control. TCP first; UDP is a possible P2 optimisation for the
   1–5 Hz wattmeter poll.

Security note: the KPA1500 TCP server has **no authentication or TLS at all** (nothing in
[K15 p.6, p.22]) — the plan must state (docs + `NFR-SEC-02` extension) that accessory links
are LAN/VPN-only; never exposed to the Internet raw.

### 4.3 Polling model

None of the devices push state, so the accessory thread runs a **poll plan** (pure,
unit-testable schedule in `k4-accessory`):
- *Fast lane (~2–5 Hz, only while K4 `transmitting` or amp keyed):* `^WS`(+`^SW`)/`VSWR` —
  power + SWR meters; `^TQ` where fw ≥3.07.
- *Medium lane (~1 Hz):* `^OS`/`MD`, `^BN`/`BN`, `^AN`/`AN`, `^FL`/`FLT`, `^TP`/`TP` (only
  while a tune is pending), `BYP`/`^AI`.
- *Slow lane (~0.1 Hz):* `^TM`, `^VI`, `^FS`, `VFWD/VRFL` diagnostics.
- *On-connect seed:* identity (`I`/`^I`), `RV/^RV`, `SN/^SN`, then a full GET burst of the
  medium+slow lanes (mirrors `FR-CAT-07` seeding).
- *After every SET:* immediate corresponding GET (the devices are silent on SET
  [KAT p.4, K15 p.6]) — this is the accessory analogue of ADR-04's "state from RESP only".
- *Liveness:* the null `;` echo [K5 p.1, KAT p.6, K15 p.9] as the accessory PING; the wake
  routine (`;` every ~100 ms until echo [KAT p.4, K15 p.8]) runs on connect and after
  silence. Link-loss → bounded backoff reconnect, reusing `k4_session::Backoff`.

The KPA1500 `^LQ` LED bitmap [K15 p.35] is attractive (one round-trip = whole dashboard) but
mirrors LEDs, not engineering values; use it only as a cross-check, poll real values
*(design choice)*.

### 4.4 State → UI flow (Key question 3)

```
accessory thread (blocking I/O, per device)
   │ typed AccessoryEvent (parsed Resp / LinkUp / LinkLost)
   ▼
worker tick: state.apply(resp)  →  Kat500State / Kpa500State / Kpa1500State
   ▼
UiSnapshot { …, amp: Option<AmpSnapshot>, tuner: Option<TunerSnapshot> }
```

- `AmpSnapshot` (normalized across KPA500/KPA1500): `link: AccLinkPhase`, `model`,
  `powered`, `operate: bool`, `band: Option<Band>`, `antenna`, `fwd_w`, `swr_x10`,
  `temp_c`, `pa_volts_x10`, `pa_amps`, `fault: Option<AccFault { code_raw, name, hard }>`,
  `atu: Option<AtuBrief>` (KPA1500 only), `tuning: bool`.
- `TunerSnapshot` (KAT500): `mode: Byp|Man|Auto`, `antenna`, `band`, `swr_x100`, `bypass`,
  `tuning`, `fault`, `amp_key_interrupted` (inferred from fault presence [KAT p.19]).
- `RadioState` is untouched; the *only* cross-links are in the worker's gating logic (§5)
  which sees both the session's `RadioState` and the accessory states.

### 4.5 Protocol layer (Key question 2 — the answer)

**One crate, three typed modules + shared core** (§3 verdict). Not three crates (they share
the line codec, band table, poll scheduler, and will always ship together) and not an
extension of `k4-protocol` (different wire model — no `IF`, no AI, no `$` sub-RX modifier,
no binary frames; keeping `k4-protocol` K4-pure preserves `NFR-MAINT-01` and the existing
test seams).

---

## 5. Safety analysis (Key question 4)

The project's stance (ADR-08: software TX safety in *addition* to radio-side interlocks)
extends naturally. The amplifier hazards and their mitigations:

| Hazard | Existing mechanism | New mechanism proposed |
|---|---|---|
| RF applied with amp faulted/overheating | Amp auto-STBYs on hard fault [K15 p.29; K5OM p.38] (device-side) | Surface fault prominently (red banner, `FR-UI-04`-grade); **fault clear (`^FLC`) is an explicit guarded operator action, never automatic**; optional app-side TX inhibit while amp fault active (`FR-ACC-SAFE-02`) |
| Band mismatch: K4 QSYs, amp/tuner still on old band (wrong LPF → PA damage) | Hardware BAND lines/AUXBUS when cabled [K15 p.57; KATOM p.7] | App compares `RadioState` band (from `BN`/`IF`) with `^BN`/`BN` each poll; mismatch ⇒ warning + (configurable) block of the `ARC-06` TX gate; optional band-follow (send `^BN`, `F`) when station is *not* ACC-cabled (`FR-ACC-SAFE-01`) |
| Overdrive (K4 power too high for amp input) | Amp input-power fault 60/PWRIN HI + overdrive attenuator [K15 p.29; K5OM p.38] | When amp is OPER, apply a configurable **drive cap** to the K4 `PC` control (UI clamps + warns; per-amp default, e.g. ≤30 W KPA500) (`FR-ACC-SAFE-03`) *(inference: sensible defaults need manual/hardware confirmation)* |
| Emergency stop must kill the *amplified* signal | `FR-TX-SAFE-04` e-stop → `RX;` unkeys K4; the amp key line follows the radio (hardware) | E-stop additionally sends `^OS0;` to any connected amp (belt-and-braces) and `^RX;` if a `^TX` was ever issued; KAT500 untouched (passive) but `AMPI1;` available as a manual "key-line open" control (`FR-ACC-SAFE-04`) |
| Link loss *to the accessory* while transmitting | n/a today | Accessory link loss while K4 TX ⇒ operator alert; TX itself is not force-stopped (the amp still follows the hardware key line safely; killing a QSO for a telemetry dropout is worse) — but the TX-arm indicator degrades to "amp unmonitored" (`FR-ACC-SAFE-05`) *(design decision to review)* |
| Software keying runaway (KPA1500 `^TX`) | Device-side timeout form exists [K15 p.53] | **Policy: the app never sends bare `^TX;`** — only `^TXnn;` with nn ≤ 10 s, refreshed while needed, `^RX;` on stop — mirroring the `KZF` dead-man philosophy of `FR-TX-SAFE-02`. v1 scope: don't use `^TX` at all (hardware key line does the job); reserve for the tune wizard if needed |
| Remote ATU tune transmits a carrier | TX is already gated by arm (`FR-TX-SAFE-03`) | The **guided tune workflow** (KAT500 `FT`, KPA1500 `^FT`) runs *inside* the existing arm+e-stop regime: requires TX armed; sequence = save `PC` → set tune power (15–30 W for KAT500 full tune [KAT p.20]) → key via the normal gated path → start tune → poll `TP`/`^TP` → unkey → restore `PC`; e-stop and `CT`/`^FE` cancel at any step; a stall timeout unkeys (`FR-KAT-03` / `FR-KPA-06`) |
| KAT500 protecting the amp | `ST…K` SWR threshold + key-line interrupt + `AKIP` [KAT p.29, p.7] | Expose read-only visibility of `AKIP` and fault 3/4 explanations; setup checklist in docs (KPA500 owners: `AKIP 1500;` [KAT p.7]) |

Interaction with `FR-TX-SAFE-01` (link-loss fail-safe): unchanged for the K4 link. The
existing invariant "K4 link loss ⇒ local unkey + re-arm required" already guarantees the amp
stops amplifying (no drive, key line drops). Accessory support must **not weaken** this:
accessory I/O runs on its own thread and can never block the session tick (same isolation
argument as the K-Pod thread, worker.rs:965).

---

## 6. Proposed requirements (SRS §D3 "Station Accessories — `FR-ACC` / `FR-KAT` / `FR-KPA`")

Format per `docs/requirements/system-requirements.md` legend (Pri M/S/C/W2; Ver T/D/I/A).
Suggested Up-trace: STK-11 (operating position), STK-08/13 (TX safety), STK-01 (link
visibility), STK-18 (transport seam), STK-04 (live status). *(A new stakeholder item — e.g.
`STK-21` "operate the whole remote station, including amplifier and tuner" — would be
cleaner; flagging rather than inventing it here.)*

| ID | Statement (the system shall…) | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-ACC-01` | connect to a station accessory (KAT500, KPA500, KPA1500) over a configurable link — local serial port (path + baud) or raw TCP host:port (KPA1500 native server default **1500**; serial-over-TCP bridge for the others) — via the transport-agnostic accessory link. | STK-18/11 | S | T | A mock link drives each device engine unchanged; TCP and serial backends pass the same protocol test suite. |
| `FR-ACC-02` | frame accessory traffic as `;`-terminated ASCII GET/SET commands (KPA-family `^` prefix) and parse responses tolerantly — unknown/malformed responses logged and skipped without desync. | STK-11/17 | S | T | Round-trip encode/decode per supported command; garbage input resyncs at the next `;`. |
| `FR-ACC-03` | poll each connected accessory on a tiered schedule (meters fast while transmitting, status ~1 Hz, temperatures slow) and follow every SET with a verifying GET, since accessory SETs return no response. | STK-04 | S | T | Poll plan emits the documented GETs at the configured cadence; a SET is followed by its GET; state updates only from responses. |
| `FR-ACC-04` | detect accessory presence/liveness with the null-`;` echo, run the documented wake-up procedure (repeated `;` at ~100 ms) on connect, and reconnect with bounded backoff after link loss, surfacing link state per device. | STK-01 | S | T | Simulated sleep swallows chars until N `;`s; silence → link-loss event + backoff retries; per-device phase exposed in the snapshot. |
| `FR-ACC-05` | expose each accessory as a **runtime opt-in** (default off, persisted per profile); the app runs fully with any subset of accessories absent, misconfigured, or lost. | STK-11 | S | T | Prefs round-trip; with no device the app behaves exactly as today (demonstrated). |
| `FR-ACC-06` | verify device identity on connect (`I;`→`KAT500;`, `^I;`→`^KPA1500;`, KPA500 `^RVM`) and refuse to drive a mismatched device. | STK-11/17 | S | T | Wrong identity string → link enters an error state; no further SETs sent. |
| `FR-ACC-SAFE-01` | continuously compare the K4 band with each amp/tuner band and, on mismatch, show a prominent warning and (configurably) block the TX gate; optionally push band/frequency (`^BN`, `BN`, `F`/`^FR`) when the station is not ACC-cabled. | STK-08/13 | S | T | Simulated mismatch raises the flag within one poll cycle and the `ARC-06` gate refuses `TX;` when the interlock is enabled. |
| `FR-ACC-SAFE-02` | display amplifier faults (code + decoded name + hard/attenuator class) within one poll cycle; fault **clear** (`^FLC;`/`FLTC;`) shall be an explicit, guarded operator action, never automatic. | STK-08 | S | T | Injected `^FL90;` → fault surfaced; no `^FLC` is ever emitted without the confirmed UI action. |
| `FR-ACC-SAFE-03` | while an amplifier is in OPERATE, apply a configurable K4 drive-power cap (clamp + warn on `PC` above the cap). | STK-08 | C | T | With cap 30 W and amp OPER, a 50 W request emits `PC030…` + warning; cap inactive in STBY. |
| `FR-ACC-SAFE-04` | extend the emergency stop (`FR-TX-SAFE-04`) to also command any connected amplifier to STANDBY (`^OS0;`) and cancel any software keying (`^RX;`). | STK-08 | S | T | E-stop emits `RX;` then `^OS0;` (+`^RX;` if `^TX` pending) on all amp links, regardless of UI focus. |
| `FR-ACC-SAFE-05` | signal degraded supervision when an accessory link is lost while TX is armed (alert + "amp unmonitored" state) without force-stopping an in-progress transmission. | STK-08/01 | C | T | Killing the amp link during armed TX raises the alert; the K4 session is unaffected. |
| `FR-KPA-01` | control amplifier OPERATE/STANDBY (`^OS`) and reflect it, including the device's fault-driven STBY transitions. | STK-11 | S | T | `^OS1;`/`^OS0;` encode; a polled `^OS0;` after a fault updates state. |
| `FR-KPA-02` | display live amplifier telemetry: forward power + SWR (`^WS`, 3-digit KPA500 / 4-digit KPA1500 widths), PA temperature (`^TM`), PA volts/amps (`^VI`), and (KPA1500) reflected/input/dissipated power (`^PWR`/`^PWI`/`^PWD`). | STK-04 | S | T | Each documented field width parses to the right scaled value (implied decimals per spec). |
| `FR-KPA-03` | display and set the amplifier band (`^BN` 00–10, same table as the K4) and antenna (`^AN`), reflecting front-panel changes via polling. | STK-11 | S | T | Band/antenna round-trip; polled changes update the snapshot. |
| `FR-KPA-04` | decode fault codes per model — KPA500 decimal identifiers (owner's-manual fault table), KPA1500 two-hex-digit codes incl. overdrive codes (`^OC`) — into operator-readable causes. | STK-08/11 | S | T | Table-driven decode covers every documented code; unknown codes render as raw + "unknown". |
| `FR-KPA-05` | support remote power control per model: KPA1500 `^ON1;`/`^ON0;` with wake-`;` preamble (and optional Wake-on-LAN); KPA500 `^ON0;` off and boot-loader `P` on; both guarded like `FR-PWR-01`. | STK-11 | C | T/D | Power-on sequence emits wake `;`s then the command; UI requires two-step confirm. |
| `FR-KPA-06` | run the KPA1500 built-in-ATU tune as a guided, cancellable workflow (`^FT` start, `^TP` poll, `^FE` cancel) inside the existing TX arm/e-stop regime, restoring prior K4 power afterwards. | STK-08/11 | C | T/D | The wizard's command sequence matches the spec; e-stop or `^FE` aborts and unkeys; `PC` restored. |
| `FR-KAT-01` | control and reflect KAT500 operating state: mode (`MD` B/M/A), antenna (`AN` 0–3), band (`BN`), bypass (`BYP`), and show SWR (`VSWR`) and bypass SWR (`VSWRB`). | STK-11 | S | T | Each command round-trips; polled state updates the tuner snapshot. |
| `FR-KAT-02` | display KAT500 faults (`FLT` 0–4 with decoded meaning, incl. "amplifier key line interrupted during any fault") and offer guarded clear (`FLTC`). | STK-08 | S | T | Injected `FLT3;` decodes to the safe-relay-limit cause; clear only via confirmed action. |
| `FR-KAT-03` | run KAT500 full-search tune as a guided, cancellable workflow (`T`/`FT`/`FTNS` start, `TP` poll, `CT` cancel, delayed `FT;` completion) at tune-level drive (15–30 W) inside the TX arm/e-stop regime. | STK-08/11 | C | T/D | Sequence per spec; completion detected from the delayed `FT;`; abort path unkeys and restores `PC`. |
| `FR-KAT-04` | surface the amplifier-protection configuration read-only in v1: `AKIP` threshold, per-band `ST…K` key-interrupt SWR threshold, and `AMPI` key-line state. | STK-08 | C | T | GETs parse; values shown; no SET path in v1. |
| `FR-ACC-UI-01` | show an always-visible station strip when any accessory is enabled — amp OPER/STBY, forward power, SWR, temperature, fault; tuner mode, SWR, tune-in-progress — readable at a glance (NFR-USE-01 discipline). | STK-11/04 | S | D | Demo checklist: all listed items visible without opening a screen; fault turns the strip red. |
| `FR-ACC-UI-02` | provide a full station screen (in the `ScreenKind` spectrum-frame slot) with amp and tuner controls, meters, fault history (KPA1500 `^SF`), and the accessory settings (endpoints, interlocks, caps). | STK-11 | C | D | Screen reachable from the existing screen model; controls drive the documented commands. |
| `NFR-ACC-SEC-01` | accessory links (raw TCP and the KPA1500 native server) are unauthenticated; the system shall document LAN/VPN-only deployment and never send accessory traffic over an untrusted network by default. | STK-14 | S | I | Docs present; no accessory feature defaults to a public endpoint. |

---

## 7. Contradictions and doc defects to flag (do not silently resolve)

1. **KPA1500 `^NH` vs `^NI`** [K15 p.37]: the heading says "^NH TX Inhibit Enable" but both
   GET and SET formats are written `^NI;`/`^NIx;`. KPA500 uses `^NH` [K5 p.3]. Which
   mnemonic the KPA1500 firmware actually accepts must be confirmed on hardware; implement
   with the mnemonic behind one constant so the fix is one line.
2. **KPA1500 `^AI` typo** [K15 p.13]: "SET/RESPONSE format: ^AI1; if ATU is currently
   inline, **^AT0;** if ATU is currently bypassed" — `^AT0` is almost certainly a typo for
   `^AI0`; confirm on hardware.
3. **KPA1500 `^SF` cross-reference error** [K15 p.45]: "faultCode is described in the ^FC
   command above" — `^FC` is *Fan Minimum Speed* [p.27]; fault codes are under `^FL`
   [p.29].
4. **Fault-code encodings differ by model**: KAT500 single decimal digit 0–4 [KAT p.19];
   KPA500 decimal `nn` with the table only in the owner's manual [K5 p.3; K5OM p.38];
   KPA1500 two **hex** digits [K15 p.29]. Never share a decoder.
5. **KPA500 `^FL` table absence**: the Programmer's Reference defines the response format
   but no code table; the mapping to the owner's-manual FAULT NO. column is *(inference)*
   until hardware-confirmed.
6. **KPA1500 Owner's Manual Errata B1-2**: only mechanical/cooling/typo corrections; no
   protocol content [Errata p.1] — the Programming Reference is not corrected by it.
7. **KPA1500 doc self-dating**: the V3 reference is "Revised 6/1/2026 for firmware version
   03.0" yet describes commands "new in 3.07/3.08" (`^TX`, `^DW` max) [K15 p.1, p.25,
   p.53] — treat per-command "new in" notes, not the title page, as the availability
   authority, and gate on the polled `^RV` at connect.

---

## 8. Phased delivery plan

| Phase | Content | Ships value | Rough effort |
|---|---|---|---|
| **A0 — Spec & scaffolding** | SRS §D3 rows (above) into `system-requirements.md`; ADR-16 "accessory reach = own links, not K4 pass-through"; `ARC-16..18` rows in architecture.md; empty `k4-accessory` crate + line codec + band table + wake/poll-plan core with tests; `RawTcpTransport` in `k4-transport`. | traceability gate stays green; seam proven with mock | 2–3 days |
| **A1 — KPA1500 end-to-end** | `kpa1500.rs` codec+state (control, telemetry, faults, ATU monitor); scripted simulator; worker accessory thread + prefs + snapshot; **station strip** (`FR-ACC-UI-01`): OPER/STBY toggle, power/SWR/temp meters, fault banner + guarded clear, band/antenna display. | a remote KPA1500 station is fully operable — the single biggest win, no bridge hardware needed | 5–7 days |
| **A2 — KPA500 + KAT500** | `kpa500.rs`, `kat500.rs` codecs+states+sims; serial + bridge endpoints in settings; shared `AmpSnapshot` normalization; tuner strip segment (mode/SWR/BYP/tune-poll); docs for ser2net setup incl. recommended `AKIP 1500;` for KPA500 owners [KAT p.7]. | classic KAT500+KPA500 stations covered | 4–6 days |
| **A3 — Safety interlocks** | `FR-ACC-SAFE-01..05`: band-mismatch interlock into the `ARC-06` TX gate; e-stop fan-out; drive cap; degraded-supervision alert; fault-driven UI states; tests incl. fault-injection through the sims. | the amp can no longer be wrecked by a QSY or a stuck UI | 3–4 days |
| **A4 — Tune workflows + power + polish** | Guided KAT500/KPA1500 tune wizards (`FR-KAT-03`, `FR-KPA-06`); remote power on/off (`FR-KPA-05`, incl. KPA500 boot-loader `P` quirk and KPA1500 wake preamble/WoL); full station screen (`FR-ACC-UI-02`) with `^SF` fault log; hardware validation pass. | complete "operate the whole station" story | 4–6 days |

Total ≈ 18–26 focused days. Each phase is a PR-per-change series per the project workflow;
A1 alone is a releasable feature.

---

## 9. Open questions / needs hardware to confirm

1. **KPA1500 TCP server while "off"/asleep** — `^ON`/`^RV`/`^SN` are documented as available
   when sleeping *on the USB host* [K15 p.39, p.44]; whether the TCP server accepts
   connections and `^ON1;` while the main supplies are off (without WoL) is unstated.
   Determines whether remote power-on needs WoL or works in-band.
2. **`^NH` vs `^NI`** (§7.1) and **`^AI0` vs `^AT0`** (§7.2) — one live probe each.
3. **KPA500 `^FL` code values** vs the owner's-manual FAULT NO. table (§7.5) — provoke a
   benign fault (e.g. INVALID frequency at QRP) and read `^FL`.
4. **KPA500 boot-loader `P` over a TCP bridge** — single char, no terminator, no response
   [K5 p.5]; verify bridges pass it through and how long firmware boot takes before `;`
   echoes.
5. **KAT500 behind ser2net with `SL1;` sleep** — confirm the 100 ms-interval `;` wake works
   with bridge buffering; recommend `SL0;` in docs otherwise [KAT p.27].
6. **K4 ↔ KPA1500 `^XK` tune-power request** — documented for K3/K3S MCU ≥5.63 via AUXBUS
   [K15 p.58]; unknown whether the K4 honours it. If yes, the KPA1500 tune wizard can skip
   app-side keying entirely (much safer). Test on the real station.
7. **Poll-rate tolerance** — the devices have small input buffers and no flow control
   [KAT p.4; K15 p.6]; validate the fast-lane meter poll (2–5 Hz) doesn't starve front-panel
   responsiveness, especially at 38400 on bridged serial.
8. **Band-follow need** — if the user's station is fully ACC-cabled (AUXBUS + BAND lines),
   app-side band push (`FR-ACC-SAFE-01` optional half) may be permanently off; confirm the
   K4 actually drives KAT500/KPA1500 band via AUXBUS on QSY as inferred (§2.1, §2.3).
9. **Stakeholder trace** — add a `STK-21` "whole-station remote operation" row (or extend
   STK-11) before merging the SRS section, so the Up column doesn't overload STK-11.
