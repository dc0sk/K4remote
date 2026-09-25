---
title: "Implementation Plan — CAT Server for Third-Party Logging Software"
status: Draft
version: "0.2"
updated: 2026-09-25
authors:
  - Simon Keimer (DC0SK)
---

# Implementation plan: CAT server interface (third-party logger integration)

> **Provenance.** Researched and drafted by an AI agent on 2026-07-19 from the vendor
> documentation in `docs/references/external/`, commissioned by DC0SK. Claims carry their
> evidence source inline. Anything marked *inference*, *recalled*, or *web* has **not** been
> confirmed against hardware or against vendor documentation held in this repo — see the
> open-questions section before acting on it. This is a research input, not an approved
> baseline.

Goal: let logging/contest/digital software (N1MM+, WSJT-X, DXLab, Log4OM, fldigi, …) running
next to K4 Remote see and set frequency/mode — and optionally key — the remote K4 *through*
our app, which owns the single link to the radio. This was already flagged in
`docs/references/external-references.md` (R-EXT-01, "Ideas to adopt" table): *"An embedded
CAT server for WSJT-X/logger integration (nice Phase-3 idea)"*. This plan makes it concrete.

Evidence discipline: statements are tagged **[repo]** (read from this codebase/docs),
**[web]** (verified against a cited external source this session), or **[recalled]**
(prior knowledge, unverified — must be confirmed before relied on).

---

## 0. Revision 0.2 (2026-09-25) — decisions and source-verified findings

**This section supersedes the rest of the document where they differ.**

**Decided by DC0SK (2026-09-25):** build the CAT server first among the deferred items;
K4-native raw CAT over TCP as recommended in §1; **CAT clients may key only while TX is armed
in the app *and* a "CAT clients may transmit" setting (default off) is on** (§5 as written);
**frequency/mode changes from a client are allowed while transmitting** (§5.6, §8.9 settled);
the software to capture against is **WSJT-X / JTDX, N1MM Logger+, and fldigi / flrig /
Log4OM / CQRLOG**. DC0SK will provide a local Hamlib installation, so a real `rigctl`/`rigctld`
K4 backend can be run against the server as a repeatable local test.

**Source-verified (Hamlib `rigs/kenwood/{k3,elecraft,kenwood}.c`, `src/{rig,iofunc,network}.c`;
flrig `src/rigs/elecraft/K4.cxx`; K4 PRG D12 — read 2026-09-25, file:line citations kept in the
session record, no code copied).** These replace the *[recalled]* claims above:

1. **Hamlib opens the K4 over TCP** when the pathname is `host:port`. Each read waits
   **500 ms** (one read retry); **during open, command retries are 0**, so one missing or slow
   reply fails the open.
2. **Hamlib's open sequence:** `PS;` (expects `PS1`), `K40;`, `ID;`, `K2;`×2, `K22;`, `OM;`
   (**exactly 15 characters**, fixed positions), `K3;`, `RVM;` (**fatal on failure**), `RVD;`,
   `AI;`/`AI0;`, then `FR;` `FT;` `TQ;` (each exactly `XX0`/`XX1`). On close it restores `K2<n>;`.
3. **Hamlib verifies every SET with `ID;`** (reply must start `ID`, length ≥ 5 — i.e. `ID017`);
   `RX;` and `K22;` sleep 200 ms instead.
4. **Hamlib steady state:** `FA;`/`FB;` (11 digits), `MD;`/`MD$;`, `BW;`/`BW$;` (×10 Hz), `DT;`
   only when MD is 6/9, `IF;` (**exactly 37 characters**, cached 500 ms), `FR;FT;TQ;`. set_mode
   sends `MDn`, `BWnnnn`, `DT0`; split sends `FR0;FT1;` then copies A→B with `FB…`; PTT sends
   `TX;` (+`ID;`) / `RX;` then polls `TQ;` up to 5 times; VFO select is emulated in Hamlib.
5. **flrig** sends no `ID/K2/K3/PS`; it sends `AI0;`, `K41;`, `OM;` (checks for `P`), reads
   `FA/FB/MD/BW` for both VFOs, `FT0`, `IF;` for split, `TX;`/`RX;` + `TQ;` for PTT, and
   `SW83;SW44;` to select a VFO.
6. **Hamlib only understands a bare `?;`.** The K4's `<cmd>?;` error form is taken as a data
   reply and mis-parsed (e.g. `TQ?` reads as "not transmitting"). **The server must never answer
   a GET it supports with `<cmd>?;`.**
7. **N1MM+** (docs only): control on 9200, spectrum automatically on **9201**; the K4 section
   defers to the K3 one ("K31 mode all the time"; sends `RX;` on Esc). Its poll set is
   undocumented — capture needed.
8. **`IF` layout** (PRG p16) matches our `apply_if` exactly (indices after `IF`: freq 0–10, sign
   16, offset 17–20, r 21, x 22, t 26, m 27, s 29, p 30); Hamlib reads the TX VFO from byte 30
   (always `0` on the K4, which is correct for split).
9. **Meta modes collide.** Our upstream link runs in **K41** (`k4-transport`, sent at connect),
   Hamlib uses **K40/K22**, flrig **K41**, N1MM **K31**. `K41` changes the replies of `DS GT$ ID
   IF IS$ NB$ PA$ PC RA$ SM$ VT$` (PRG p17–18); `ID;` is `ID017;` under K40 but `ID<text>;`
   under K41. The PRG states per-client scope only for `AI` (p7); per-client meta mode is
   **unconfirmed**. How K41 changes `IF` is **undocumented**.

**Design changes that follow (v0.2):**

- **Per-client meta mode** (`K2n`, `K3n`, `K4n` held in `ClientState`, default K40/K20/K30 as a
  fresh legacy client) and **per-client reply formatting** for every meta-dependent command.
  Meta-dependent GETs are **never passed through** (the radio would answer in *our* K41 form).
- **The locally-answered set grows** — anything in an open sequence must answer from the app
  in microseconds, never cross the WAN: `PS` (`PS1` while the link is up), `ID` (per client
  mode; under K41 the radio's own ID text, cached), `K2/K3/K4` (per client), `AI` (per client),
  `OM`, `RVM`, `RVD` (cached from the radio once per connect: the app sends `OM; RVM; RVD; ID;`
  after its own init and stores the raw replies), `FR`/`FT` (from `RadioState` split; `FR` ≡
  `FT0`'s receive side), `TQ` (from the transmit state).
- **Pass-through** remains for *other* GETs the cache does not hold, with reply routing by
  mnemonic and a deadline — acceptable because no open sequence depends on them.
- **While the radio link is down: close connected clients and close each new one at once**
  (rather than `<cmd>?;`, which Hamlib mis-parses — finding 6). Loggers see a dropped
  connection, report it, and retry.
- **9201** (N1MM spectrum) is not served: nothing listens there, so N1MM gets a refused
  connection. Spectrum re-serve stays an unscoped idea (§10 C3).
- **A client SET is also applied locally** (`apply_local`), so an immediate read-back (Hamlib
  reads `FA;` before and after setting) answers the new value without waiting for the radio's
  echo — the same reconciliation as K-Pod tuning.

**Identifiers updated:** the requirements become **section Q, `FR-CATSRV-*`**, upstream a new
**`STK-22`** (operate alongside logging/contest/digital software) — `O` and `STK-21` are now
taken (amplifier, spot nameplates). Architecture: **ARC-16** (`k4-catsrv`), **ADR-16**.

**Still needs a live capture** (DC0SK's radio, or the Hamlib install against our server): the
radio's replies to `RVM;`/`RVD;`, `PS;`, `K2;`/`K3;`/`K4;` under K40 vs K41, `FR;`, `AI;`; the
`IF` reply under K41 and K31; whether meta mode is per connection; a SET followed by `ID;`
(nothing, then one `ID017;`); the exact bytes of an unknown-command error; a `rigctl -m K4
-vvvvv` trace; WSJT-X's poll set; N1MM's full stream; DXLab Commander.

### 0.1 Adversarial review of v0.2 (2026-09-25) and what it changes

An independent review against the same sources tried to break the design above. Its findings
are adopted; they **override §0 and §4–§5** where they differ.

**Transmit safety (§5, `FR-CATSRV-07`) — the design was not safe as written:**

1. **A deny-list of keying commands cannot be made complete.** `SW17` is both KEYPAD 1 and
   "play message M1"; `SW17/51/18/52` (M1–M4), `SW162–165`, `SW50` (VOX), `SW132` (TEST),
   `SWT…`/`SWH…` (Hamlib sends `SWT16` for the tuner and `SW40` for tune), `TS1` (TX test "still
   keys any downstream gear"), `DAMP…` (voice memory) all key or can key. → **Client SETs are
   forwarded only from an allowlist** of commands known not to key; the keying set is gated;
   **everything else is dropped and logged** (reversing §4.3's "unknown SETs default to
   forward"). All `SW`/`SWT`/`SWH` are treated as keying.
2. **The opt-in must be at the seam, not only in the server's classifier**, so a misclassified
   command cannot bypass it: client-originated commands carry their origin into the session,
   which checks *arm* **and** *CAT may transmit* itself.
3. **The stop direction must never be gated:** `RX`, `KY @` (CW stop, Hamlib), `KY0`, `TU0`,
   `PB0`, `DA0` are always forwarded — and a client's `RX;` goes through `end_tx()`, not a raw
   send, or the app keeps believing it transmits (mic streaming, playback suppressed).
4. **Block list grows:** all `RR*` (e.g. `RRC0` disables remote connections, `RRP…` changes the
   password), `EC`, `LB`, plus `PS` (SET), `EM`, `SL`, `ER`, `RDY`; `K2/K3/K4`, `AI`, `ID` are
   handled per client, never forwarded.

**Hamlib open and meta modes:**

5. The open continues after `FR;FT;TQ;` with `FA; FB; IF;`, `MD;`, `BW;`/`BW$;` (exactly 6/7),
   and **`DT;`/`DT$;` whenever the mode is DATA** — WSJT-X's normal mode. → `DT` is answered
   locally too.
6. **`SM` must be rescaled, not rewrapped**: Hamlib reads `SM;` as exactly `SMnnnn` in the K3
   form (RVM < 4.37); `SM$` is 0–15 in K30 and 0–21 in K31, 0–42 in our K41 link. The same goes
   for AI pushes.
7. **`PC` under K22**: Hamlib reads `PCnnnx` and sets `PC0501`-style; whether the radio accepts
   the K22 SET form while our link is in K41 is **unverified** — capture before forwarding it.
8. **Meta modes are coupled** ("K4n; turns off K2 meta-mode and changes K3 meta-mode", PRG): the
   per-client state models them as one state machine, not three independent flags.
9. **AI push** must format per client the K41-dependent lines (`PC RA$ SM$ GT$ NB$ PA$ IS$ DS ID
   IF`) and never tee `ER…`, `PONG`, the `RDY` dump, `EM`/`SL` acknowledgements or `CC`.

**Corrections to §0's findings:** the per-read timeout is about **1 s** with one read retry
(500 ms / no retry applies to the `PS` probe); Hamlib's TX-VFO byte is `IF` index **28** after
the prefix (fixed `0` by the PRG template), and `b`/`d` are documented per meta mode (`b` K22
band-change, `d` K31 DATA sub-mode — N1MM runs K31 and expects it); a bare `?;` is not clean
either (the K4 caps make Hamlib retry with sleeps); flrig also sends `FT0;` at open (clearing
split) and `PCX;`. Several line citations to our code in §4 are stale.

**Link down (replaces §0's "close clients"):** Hamlib has no reconnect — a closed socket makes
WSJT-X show its modal *Rig Control Error* and stop (behaviour of WSJT-X itself inferred, not read
from source). So for an outage shorter than a grace period the server **keeps clients, answers
from the last cache with `TQ0`, and drops their SETs**; only after the grace period does it close
them.

**Optimistic apply (§4.2):** Hamlib verifies a set frequency by reading it back up to three
times, so the optimistic value would be confirmed even if the radio rejected it. → **Revert the
optimistic field on the radio's `<cmd>?;`**, and **do not apply optimistically while
transmitting** (the radio may ignore a retune mid-TX).

**Phasing consequence:** phase A delivers **read + allowlisted non-keying SETs** only; all
keying (with the seam-level opt-in) is phase B, after captures.

---

## 1. Summary + recommendation

**Dialect: emulate the K4's own network CAT service — raw `;`-terminated Elecraft K4 CAT
over plain TCP, defaulting to port 9200 on 127.0.0.1.**

The decisive fact: the real K4 already exposes exactly this service on its Ethernet port
(TCP 9200, plain ASCII CAT, no framing, no auth), and mainstream software connects to it
natively — N1MM+ has a "connect via TCP, tested with the Elecraft K4" radio option
**[web: n1mmwp.hamdocs.com update 1.0.9429; wt8p.com K4/FT8 notes]**, and DXLab Commander
has a "Communicate via TCP" checkbox for the K4 **[web: dxlabsuite.com wiki, fetched and
quoted this session]**. Hamlib has an Elecraft K4 backend, which covers WSJT-X, JTDX,
fldigi, Log4OM and others **[recalled — verify]**. By emulating that service, K4 Remote
*looks like a K4 on localhost*: users configure their logger exactly as they would for a
LAN-attached K4, just with address `127.0.0.1`.

It is also by far the cheapest dialect for us: every byte of it is the dialect this codebase
already speaks. `k4_protocol::cat::LineDecoder` parses `;`-terminated client input verbatim
**[repo: crates/k4-protocol/src/cat.rs:14–45]**; `RadioState::apply_cat` is the parser whose
mirror-image formatter we need **[repo: crates/k4-protocol/src/state.rs]**; the SET encoders
in `cat.rs` (e.g. `set_vfo_a_hz` → `FA00014074000;`) already produce the exact RESP wire
forms **[repo]**. A TS-2000 or rigctld emulation would each be a *second* protocol plus a
semantic mapping layer; neither buys coverage the K4 dialect doesn't already have (§2).

**Transport: TCP only in phase 1**, localhost-bound by default (§3, §6). Virtual serial is
deferred: on Windows it genuinely requires a third-party kernel driver (com0com) that we can
document but not ship (§3.2), and the marquee Windows loggers no longer need it.

**Serve from the cached `RadioState`**, forwarding SETs through the existing
`Session`/worker path, with a pass-through escape hatch for commands we don't model (§4.3).

**Transmit: CAT clients may key only when the operator has armed TX in our UI *and* opted
in** via a "CAT clients may transmit" setting; unkey (`RX;`) is always honoured; e-stop and
link-loss fail-safes are unchanged because every client command funnels through the same
`Session` seam that enforces them (§5).

**Phase 1 scope**: single new crate `k4-catsrv` (proposed ARC-16) + worker integration + a
settings row. Answer the core poll set (`FA/FB/IF/MD/AI/ID/…`) from cache, forward safe
SETs, refuse session-owned commands, no TX. That alone makes N1MM+/Commander/Hamlib-based
software track and tune the radio.

---

## 2. Dialect analysis

### Candidates

| Dialect | Who speaks it | Minimum useful subset | Effort for us | Risk | Verdict |
|---|---|---|---|---|---|
| **Elecraft K4 CAT over TCP (emulate the radio's own port-9200 service)** | N1MM+ (native TCP radio, "tested with the Elecraft K4") **[web]**; DXLab Commander ("Communicate via TCP") **[web]**; K4-Companion **[web: github.com/DaleFarnsworth/K4-Companion]**; Hamlib K4 backend → WSJT-X/JTDX/fldigi/Log4OM/CQRLOG/gpredict-style clients **[recalled]**; anything that supports "K3/K4 on a COM port" if later bridged to serial | `FA FB IF MD MD$ FT AI ID K2/K3/K4-mode TX RX` + tolerant ignore of the rest; most loggers poll `IF`/`FA`/`FB`/`MD` **[recalled]** | **Lowest** — parser (`LineDecoder`), encoders, and state model all exist [repo]; new work = GET-response formatters + server plumbing | Exact `IF` layout tail and `ID`/`OM`/`RVM` compatibility replies must match the PRG; per-client `AI` semantics must match (PRG `AI` NOTE2 per-client [repo: FR-CAT-AI]) | **Phase 1** |
| **Kenwood TS-2000 emulation** | Very broad legacy support **[recalled]** | `FA FB IF MD AI TX RX` (Kenwood syntax; `IF` layout and mode digits differ from Elecraft) **[recalled]** | Medium — second formatter set + mode/field mapping; Elecraft CAT *derives from* Kenwood but has diverged (K4 `BW` vs Kenwood filter codes, `$` sub-RX, 11-digit freq is shared) **[recalled]** | Subtle semantic mismatches (RIT fields, mode numbering for DATA) cause silent wrong-mode logging | **Skip** — no logger in our target list needs it that doesn't already do Elecraft or Hamlib |
| **Hamlib `rigctld` protocol (TCP 4532)** | Any Hamlib client via rig model 2 "NET rigctl": WSJT-X, fldigi, Log4OM, gqrx, SDR tools **[web: hamlib.sourceforge.net/html/rigctld.1.html — text line protocol, `\set_freq 14266000`, `RPRT x` error replies, default port 4532]** | `\get_freq \set_freq \get_mode \set_mode \get_vfo \get_ptt \set_ptt \dump_state \chk_vfo` **[web/recalled: dump_state details not in the manpage; the reply format must be cribbed from Hamlib source — mark recalled]** | Low-medium — trivial line protocol, but semantic mapping (Hamlib mode names `USB/LSB/CW/PKTUSB`, passband widths, VFO targeting) + the underdocumented `dump_state` handshake | Hamlib clients can already reach us through the Hamlib **K4 backend** pointed at our TCP port, making this redundant for phase 1 **[recalled — verify]** | **Phase 3 option** — add only if testing shows Hamlib's K4 backend misbehaves against our emulation |
| **Several at once** | union | — | Additive | Each dialect is an independent compatibility surface to test | Only as later phases |

### Why K4-native wins

1. **Configuration story**: "point your logger at 127.0.0.1 port 9200, radio type Elecraft
   K4" — identical to the vendor-documented LAN setup users already know
   **[web: wt8p.com/configuring-elecraft-k4d-for-ft8/]**.
2. **Zero translation layer**: commands from clients are *already in the radio's language*;
   unknown ones can be forwarded to the radio verbatim (§4.3). A TS-2000 or rigctld front
   end can never do that — every command must be understood to be translated.
3. **Symmetric testability**: our formatter's output can be fed back through our own
   `apply_cat` for round-trip unit tests, and `k4-sim` gives an end-to-end harness with no
   hardware (§4.5, NFR-TEST-02).

Normative command source: K4 Programmer's Reference (project cites rev. D12 [repo: SRS
legend]; public copies at ftp.elecraft.com **[web: search hit, rev. C10/D4 HTML/PDF]**).

---

## 3. Transport to the clients

### 3.1 TCP (phase 1)

Plain `TcpListener`, default bind `127.0.0.1:9200`, port and bind address configurable.
Matches the project's deliberate v1 sync-`std::net` style (ADR-14) and the exact shape of
`k4-sim`'s listener (`TcpListener::bind` + thread per connection) **[repo:
crates/k4-sim/src/lib.rs:40–52]**. Thread-per-client is fine at logger scale (1–3 clients,
line-rate traffic).

Note: the raw-CAT dialect means *no frame envelope and no auth* — clients write ASCII CAT
directly, unlike our upstream 9205/9204 link (`FrameCodec` + SHA-384/TLS-PSK) [repo:
FR-STREAM-01, FR-AUTH-01/02]. That is faithful to the real K4's port 9200 **[web/recalled:
no evidence of authentication on the K4's 9200 service; verify on real radio]** and is why
localhost-only is the mandatory default (§6).

### 3.2 Virtual serial port — the honest version

Some Windows software only enumerates COM ports (OmniRig-based loggers, older HRD)
**[recalled]**. Plainly:

- **Windows**: a user-space process **cannot create a COM port**. Full stop. It requires a
  kernel-mode driver. Options: (a) ship/install **com0com** (open-source null-modem driver;
  signing/installation friction, and we'd be taking on driver support burden) — we should
  *document* it, not bundle it; (b) commercial drivers (licensing cost); (c) nothing. With
  com0com installed by the user, our job is easy: open one end of the pair with the existing
  `serialport` dependency (already used by `SerialTransport` [repo:
  crates/k4-transport/src/lib.rs:404–426]) and pump bytes to/from the same server core.
- **Linux**: we *can* do it driverless — `posix_openpt`/`openpty` creates a pty pair in
  user space; we expose the slave path (e.g. `/dev/pts/5`, plus a stable symlink like
  `~/.config/k4remote/cat-tty`) in the UI. Loggers running under Wine can map it to a COM
  port **[recalled]**.
- **macOS**: ptys work the same way; relevance is low (Mac loggers that support Elecraft
  tend to support network control) **[recalled]**.

**Decision**: defer all of this to a "could" phase. The Windows loggers that matter most
(N1MM+, DXLab) speak TCP to a K4 today **[web]**, and Hamlib-based software can use a
network pathname **[recalled — verify]**. The serial bridge is a compatibility long-tail
item, and on Windows it can never be seamless without a driver we shouldn't own.

---

## 4. Architecture

### 4.1 Placement: new crate `crates/k4-catsrv` (proposed ARC-16)

Follows the workspace rule (protocol-adjacent logic in an iced-free crate, NFR-MAINT-01
[repo: architecture.md §3]):

- **`k4-catsrv` (pure core, fully unit-testable)**
  - `ClientState`: per-client `LineDecoder` (reused verbatim from `k4_protocol::cat`), the
    client's `AI` level, and its extended-mode flags (`K2x/K3x/K4x` are *per connection* on
    the real radio, like `AI` — PRG `AI` NOTE2 [repo: FR-CAT-AI]; K-mode per-client is
    **[recalled — verify]**), plus a pending-passthrough queue (§4.3).
  - `Policy`: allow/gate/block classification of command mnemonics (§4.4, §5).
  - `fn handle(client: &mut ClientState, cmd: &str, state: &RadioState, policy: &Policy) ->
    Vec<Action>` where `Action` is `Reply(String)` | `Forward(String)` |
    `ForwardKeying(String)` | `Broadcast` bookkeeping | `Drop{log_reason}`. Pure — no
    sockets, no threads — so every FR acceptance test runs against it directly.
- **GET-response formatters in `k4-protocol`** (new `cat_resp` module): the mirror image of
  `apply_cat` — `resp_fa(&RadioState) -> Option<String>` ("FA{:011};"), `resp_if(...)`
  (fixed-width `IF` synthesis matching the layout `apply_if` already decodes: freq 0..11,
  RIT sign/mag 16..21, RIT/XIT flags 21/22, TX 26, mode 27, scan 29, split 30 [repo:
  state.rs:681–700]), `resp_md`, `resp_ft`, etc. Placing them beside `apply_cat` enables
  the round-trip property test *format(state) → apply_cat → same fields*. Positions of `IF`
  we don't currently parse (23–25, 28, 31+) must be filled per PRG D12 and verified against
  a real radio (**open question**, §8).
- **I/O shell** (listener + thread-per-client): either in `k4-catsrv` behind a small
  `serve()` API or in `app/src/catsrv.rs` beside `worker.rs`. Recommend inside the crate
  (mirroring `k4-sim`, which also keeps its `TcpListener` in-crate [repo]), so the app only
  wires channels.

### 4.2 Worker integration — how client commands reach the radio

The worker (`app/src/worker.rs`) already is the single owner of the session and services a
command channel each loop (`rx.try_recv()` drain at run() step 1 [repo: worker.rs:632–673]),
and the K-Pod module already demonstrates the exact pattern needed: a device thread
forwarding events over an `mpsc` channel, drained once per worker loop, applied to the
session [repo: worker.rs kpod module]. The CAT server plugs in identically:

```
logger --TCP--> client thread --mpsc: (client_id, cat_line)--> worker loop
                                                                 |  policy/core says:
                                                                 |  Reply -> route back to that client
                                                                 |  Forward -> session.send(cmd)
                                                                 |  Keying -> session.begin_tx()/end_tx() (gated)
worker pump(): inbound.cat lines --tee--> server (AI broadcast + passthrough reply routing)
```

Key properties:

- **All radio-bound traffic goes through `Session`** — the same seam that enforces the arm
  gate, e-stop, and link-loss fail-safe [repo: k4-session/src/lib.rs `begin_tx`/`fail_safe`/
  `emergency_stop`]. No second path to the socket exists, so safety cannot be bypassed.
- **Reads are free**: a logger polling `IF;FA;MD;` at 10 Hz touches only the in-memory
  `RadioState` (`session.state()` [repo]); zero added radio traffic. This matters on a WAN
  link where the upstream socket also carries Opus audio and PAN frames (ADR-13).
- **AI push**: the worker already receives every state-bearing CAT line in `inbound.cat`
  [repo: session `pump()` returns them]. Tee each line (minus `PONG`) to the server, which
  forwards to clients with `AI` ≥ on. Locally-originated optimistic sets that the K4 does
  not echo (the `apply_local` path, e.g. K-Pod tuning [repo: worker.rs:1169–1173]) are teed
  the same way, so an AI client's view matches our UI's optimistic view.
- **UI/optimistic model untouched**: a client SET (e.g. WSJT-X `FA…;`) is just another
  writer through `session.send`; the radio's echo/AI response updates `RadioState`
  (FR-CAT-06) and our UI follows exactly as if the operator had turned the knob on the
  radio itself. Optionally `apply_local` the client's SET for snappier UI mirroring — same
  reconciliation story as K-Pod tuning.

### 4.3 Serving from cache — and where it breaks (with mitigations)

Cache-first is right because: (1) latency — cache answers in microseconds vs a WAN RTT per
poll; (2) upstream bandwidth — the single multiplexed socket carries audio/PAN (ADR-13),
and a 10 Hz × N-clients poll storm must not compete with Opus frames; (3) coherence — one
authoritative `RadioState` (ADR-04) means our UI and every logger agree.

It breaks in three places; each has a policy:

1. **Commands `RadioState` doesn't track** (examples visible by absence in state.rs: `FR`
   receive-VFO, `TQ` transmit query **[recalled that loggers use these — verify by
   capture]**, `OM` option query, `RVM`/`RV` firmware revision, band-plan queries).
   → **Pass-through**: forward the GET to the radio; record `(client_id, mnemonic,
   deadline)` in a pending map; when a teed inbound line matches the longest pending prefix,
   route it to that client (and still broadcast to AI clients). Timeout ⇒ drop, log to
   diagnostics. This is exactly the self-identifying-reply property the codebase already
   leans on for `<cmd>?;` errors [repo: state.rs:260–267].
2. **Session-owned / dangerous commands** — must never be forwarded: `RRN` (would
   disconnect *our* upstream link), `EM`/`SL` (would break the audio stream we negotiated),
   `RDY` (triggers the full state dump; instead answer from cache with our own dump of
   known state, or ignore), `PS` (power-off; FR-PWR-01 requires a guarded confirm even in
   our own UI [repo]), `ER`, and per-client-local ones handled by the core itself: `AI`,
   `K2/K3/K4` mode selects, `ID`, `RRT`.
3. **Stale cache while disconnected**: if our upstream session is down, cached answers
   would silently feed loggers a dead radio's state. Policy: while `session.is_none()`,
   respond to GETs with the K4 error form `<cmd>?;` (or refuse new client connections) —
   honest failure the software surfaces to the user. (Open question §8 on which of the two
   is friendlier to specific loggers.)

Unknown *SETs* from clients default to **forward** (the radio's own tolerant parser and
`<cmd>?;` error reply handle them — FR-CAT-03/04), minus the block list. This is the
payoff of choosing the radio's own dialect: we don't have to understand a command to serve
it faithfully.

### 4.4 Command classification (initial table)

| Class | Mnemonics (initial) | Handling |
|---|---|---|
| Cache-answered GET | `FA FB MD MD$ FT IF SM SMH BW AG RG SQ PC RA GT NB NR PA AN AR BN SB DV RT XT RO KS CW` (everything `apply_cat` parses [repo: state.rs]) | Format from `RadioState`, reply to requester only |
| Locally emulated | `AI` (per-client), `ID` (fixed reply — K3-family compat value **[recalled: `ID017;` — verify vs PRG]**), `K2/K3/K4` mode, `RDY` | Core replies/records; never forwarded |
| Forwarded SET | Any SET not blocked/keying (`FA… MD… BW… AG…`, unknowns) | `session.send`; tee radio read-back |
| Keying (gated §5) | `TX` `KY…` `KZ…` (element stream) `PB…` (DVR playback keys the TX) `SW` codes that key **[recalled — audit `SW` table]** | Only when armed + opt-in |
| Always allowed unkey | `RX` | Forward unconditionally |
| Blocked | `RRN RRT PS EM SL ER RDY`(as SET) | Drop + diagnostics log |
| Pass-through GET | Everything else ending in `;` | Forward with pending-reply routing + timeout |

### 4.5 What `k4-sim` is reused for

`k4-sim` (ARC-14) is *structurally* the template — `TcpListener::bind` + accept thread +
`response_for(&str) -> Option<String>` is precisely a CAT server's skeleton [repo:
crates/k4-sim/src/lib.rs] — but it speaks the **framed, authenticated K4/0 remote protocol**
(SHA-384 + `FrameDecoder`), which logging clients do *not* speak. So:

- **Direct reuse**: `LineDecoder` (client input), the `response_for` dispatch pattern, the
  thread-per-connection lifecycle, and `k4-sim` itself as the **fake radio** in end-to-end
  tests: `SimServer` (radio) ← worker/session ← `k4-catsrv` ← plain `TcpStream` (fake
  logger). That chain is a hardware-free integration test of the whole feature
  (NFR-TEST-02), with `SimServer::received()` asserting exactly what reached the "radio".
- **Possible extraction**: grow `k4-sim`'s `response_for` into a state-backed responder and
  share it with `k4-catsrv`'s formatters — both are "answer CAT GETs from a `RadioState`".
  Worth doing if it falls out naturally; not a goal.

### 4.6 UI surface (`app/src/main.rs`)

- Settings dialog (FR-UI-23 already houses connection/audio settings [repo]): a "CAT
  server" group — enable toggle (default off), port (default 9200), bind (localhost
  default, free-text for LAN with warning), "allow CAT clients to transmit" toggle
  (default off).
- Status strip / header: small indicator with connected-client count (the analogue of the
  existing `CC` client-count display, FR-SES-MULTI/FR-UI-STATUS-01 [repo]).
- Diagnostics console (FR-DIAG-01/02): new category `catsrv`, logging client
  connect/disconnect and (debug level) per-line traffic — this doubles as the capture tool
  for discovering what each logger actually sends (§8, de-risking the [recalled] claims).

---

## 5. Transmit-safety analysis (`FR-TX-SAFE-*` interaction)

Existing gates [repo: k4-session/src/lib.rs, SRS §H]: explicit arm (FR-TX-SAFE-03,
`begin_tx` returns false unless `tx_armed && connected`), e-stop (FR-TX-SAFE-04,
`emergency_stop` unkeys + disarms), link-loss fail-safe (FR-TX-SAFE-01/NFR-REL-FAILSAFE,
`fail_safe()` on timeout or I/O error), radio-side `KZF` (FR-TX-SAFE-02).

**Policy — "honour only when armed, and only when opted in":**

1. A keying command from a CAT client (`TX;`, `KY…;`, `KZ…;`, `PB…;`) is executed **iff**
   (a) the global setting *CAT clients may transmit* is enabled (default **off**), **and**
   (b) TX is currently **armed** in the K4 Remote UI (FR-TX-SAFE-03 unchanged — the arm is
   ours, a client can never arm/disarm). Execution goes through `Session::begin_tx()` /
   `end_tx()` — *not* a raw `session.send("TX;")` — so the `transmitting` flag, the TX
   audio path (mic capture only streams while `is_transmitting()` [repo: worker.rs
   747–763]), and the fail-safes all stay coherent. This also gives digital-mode users the
   right audio behaviour for free: WSJT-X's TX audio reaches the K4 by the user selecting a
   virtual audio cable as our TX input device (FR-AUD-DEV-01) and WSJT-X's CAT PTT opening
   that stream.
2. `RX;` (unkey) from any client is **always** forwarded, armed or not — the safe direction
   must never be gated.
3. When a keying command is refused, the server stays wire-silent (K4 SETs have no reply, so
   an error string would confuse clients) but logs at info level and pulses the ARM control
   in the UI — the exact affordance FR-TX-PTT-01 already defines for a disarmed hotkey press
   [repo: SRS §H]. The operator sees *why* WSJT-X's Tune button "did nothing".
4. **E-stop dominates**: FR-TX-SAFE-04's action unkeys and disarms, and because clients can
   only key while armed, e-stop instantly revokes CAT keying too. No new mechanism needed —
   composition works because of the single-seam design (§4.2).
5. **Link loss**: `fail_safe()` fires exactly as today. Additionally the CAT server marks
   clients' view stale (§4.3 item 3) so a logger cannot "re-key" a half-dead link.
6. **Frequency/mode changes while transmitting: allowed.** WSJT-X's split handling ("Fake
   It") legitimately retunes at TX boundaries **[recalled]**, and the K4 itself arbitrates
   what is tunable mid-TX. Blocking would break more than it protects; the radio-side and
   our keying gates are the actual safety boundary. (Listed in §8 as a decision to confirm;
   a conservative per-setting "freeze VFO during TX" toggle is cheap if wanted.)

Rationale: this composes with, and never weakens, ADR-08's defence-in-depth stance — the
CAT server adds a *third* gate (the opt-in) in front of the existing two for any TX
initiated by software we don't control.

---

## 6. Discovery / configuration

- **Bind `127.0.0.1` by default.** The dialect has no authentication (§3.1) and — behind
  the opt-in — can key a transmitter. Loopback-only means the attack/mistake surface is
  "software the user already runs on this PC", which matches the threat level of the real
  K4's own LAN service. Non-loopback bind is allowed but requires deliberately typing a
  bind address and shows a persistent warning (aligns with NFR-SEC-02's "treat the network
  as untrusted; recommend VPN" stance [repo]).
- **Port 9200 default** — the K4's documented CAT port **[web]**, so logger instructions
  are literally the vendor's. Configurable for conflicts (e.g. two K4 Remote instances).
- **Disabled by default**; enabling is persisted (FR-CFG-02/05 pattern [repo]).
- **What the user sees**: settings group (§4.6), header client count, diagnostics category,
  and a short user-manual section: "N1MM+: Radio = Elecraft K4, TCP, 127.0.0.1:9200.
  WSJT-X: Rig = Elecraft K4, network server 127.0.0.1:9200" (final wording after §8
  capture testing).
- Optional later: mDNS advertisement mimicking the radio's `K4-SNxxxxx.local` name — **not**
  in scope (spoofing a real radio's discovery identity on a LAN is more confusing than
  helpful; localhost users don't need discovery).

---

## 7. Compatibility matrix (phase-1 target set)

| Software | How it would connect | Phase-1 result | Needs / notes | Evidence |
|---|---|---|---|---|
| **N1MM+** | Radio = Elecraft K4, port type TCP, `127.0.0.1:9200` | Freq/mode track + set; CW messages once TX opt-in armed (`KY`) | May probe port+1 (9201) for spectrum — must degrade gracefully; our PAN re-serve is a possible later phase | TCP-to-K4 support **[web: n1mmwp.hamdocs.com update 1.0.9429]**; port 9200 + spectrum-on-next-port **[web: n1mmwp docs via search]** |
| **DXLab Commander** | K4, "Communicate via TCP", IP `127.0.0.1` | Freq/mode track + set | Port assumption to verify (docs don't state it; presumably 9200) | **[web: dxlabsuite.com wiki, quoted]** |
| **WSJT-X / JTDX** | Rig "Elecraft K4", network server `127.0.0.1:9200`; PTT = CAT | Track + set; TX gated by arm+opt-in | Exact Hamlib K4 command set unknown to us — capture first; fallback: external `rigctld -m <K4> -r 127.0.0.1:9200` | Hamlib K4 backend + network pathname **[recalled — verify]**; rigctld protocol itself **[web: hamlib manpage]** |
| **fldigi** | Hamlib (as above) or flrig | Track + set expected | flrig K4-over-TCP support **[recalled — verify]** | — |
| **Log4OM 2** | Hamlib, or its TCP CAT option | Track + set expected | Forum thread "TCIP CAT Control for K4" exists **[web: search hit, forum.log4om.com t=8636 — not fetched]** | partially |
| **Ham Radio Deluxe** | COM port only (Elecraft via serial) **[recalled]** | **Not in phase 1** | Needs the Phase-C serial bridge (com0com on Windows) | — |
| **OmniRig-based apps** | COM port only **[recalled]** | **Not in phase 1** | Same | — |
| **K4-Companion, QK4-style panels** | These speak the *framed 9205 remote protocol*, not raw 9200 CAT | Out of scope | Would require emulating the full K4/0 server incl. audio — a different (much bigger) feature | **[repo: R-EXT-01]** / **[web: K4-Companion repo]** |

---

## 8. Open questions and risks

1. **Exact `IF` response layout** including the fields we don't yet parse (positions 23–25,
   28, 31+ after the prefix). Derive from PRG D12; verify byte-for-byte against a real K4
   (`ASM-05` discipline [repo]). Wrong `IF` widths are the classic way to break Kenwood-
   derived parsers.
2. **`ID;` reply value** for the K4 (K3-family compat `ID017;` is [recalled]) and whether
   loggers gate features on `OM`/`RVM` option/revision replies. Mitigation: capture-first —
   ship the diagnostics `catsrv` category in the first PR, run each target logger against
   the dev build, and fill the emulated-command table from real captures instead of
   recollection.
3. **Hamlib K4 backend over TCP**: does it work against a pathname `host:port`, and which
   init commands does it send (`K4x`? `AI`? `FR/FT`?)? [recalled → must test]. If it
   misbehaves, the rigctld dialect (Phase C) is the fallback, and its protocol is verified
   **[web]**.
4. **Per-client `AI` and K-mode semantics**: PRG says AI is per-client [repo: FR-CAT-AI
   cites NOTE2]; confirm K2/K3/K4 extended mode is too, and what `AI` levels the 9200
   service supports.
5. **Behaviour while upstream-disconnected** (§4.3 item 3): `<cmd>?;` vs refusing
   connections — pick per observed logger behaviour.
6. **AI + polling duplicates**: a client polling `FA;` while also receiving AI pushes gets
   occasional duplicate lines; the K4's own multi-client environment implies clients
   tolerate this **[recalled]** — confirm during capture testing.
7. **Adversarial input**: `LineDecoder` has a 64 KiB cap [repo: cat.rs:8] — good; add
   per-client outbound queue bounds and a client cap (e.g. 8) so a runaway poller can't
   balloon the worker loop. The pending-passthrough map needs both a deadline and a size
   bound.
8. **`SW` audit**: front-panel switch emulation (FR-SW-01) can reach keying functions
   (e.g. message-play switches); the gated-keying classification must enumerate those codes
   or gate all `SW` when unarmed (conservative default).
9. **Mid-TX tuning allowed** (§5.6) — confirm the stance with the project owner; cheap to
   make configurable.
10. **Traceability overhead**: every M/S requirement below needs an ID-referencing test
    (NFR-TEST-01, `xtask trace` gate [repo]); the SRS, architecture.md (ARC-16 + an
    ADR-16), and user manual all take same-PR updates per the project's workflow.

---

## 9. Proposed requirements (SRS §-format)

New section **"O. CAT server for third-party software — `FR-CATSRV`"**. Upstream: cites
STK-19 (shared control — closest existing stakeholder need [repo: FR-SES-MULTI]); a new
stakeholder requirement (suggest `STK-21`: *operate alongside logging/contest/digital
software on the same computer*) should be added to stakeholder-requirements.md in the same
change. Priorities use the SRS legend; all rows are phase-3 scope (suggest extending the
legend with `W3` if the owner prefers, shown here as `S`/`C` within the feature).

| ID | Statement (the system shall…) | Up | Pri | Ver | Acceptance criteria |
|---|---|---|---|---|---|
| `FR-CATSRV-01` | provide an optional **CAT server** accepting plain-TCP connections and speaking raw `;`-terminated Elecraft K4 CAT (no framing, no auth), disabled by default, defaulting to `127.0.0.1:9200` with configurable port and bind address; binding to a non-loopback address shall display a persistent warning. | STK-21/19 | S | T | With the server enabled, a `TcpStream` to 127.0.0.1:9200 exchanges CAT lines; default config binds loopback; a non-loopback bind sets the warning flag. |
| `FR-CATSRV-02` | support multiple concurrent CAT clients with independent per-client parse state, `AI` level, and K-mode; one client's disconnect or malformed input shall not disturb another's session or the radio link. | STK-21/19 | S | T | Two simulated clients interleave commands; each receives only its own replies; garbage from one (incl. >64 KiB unterminated input) leaves the other and the upstream session functional. |
| `FR-CATSRV-03` | answer GET commands for state held in `RadioState` **from the cache** in the K4 RESP wire format, without generating radio traffic; at minimum `FA FB MD MD$ FT IF AI ID`. | STK-21 | S | T | Against a seeded `RadioState`, each listed GET returns a string that `apply_cat` round-trips to the same field values; the (mock) radio link records zero sends for cache-answered GETs. |
| `FR-CATSRV-04` | maintain a per-client Auto-Info mode set via `AI`, pushing state-change RESP lines (radio read-back and locally-applied optimistic sets) to clients with AI enabled and not to clients without. | STK-21/04 | S | T | After client A sends `AI1;` and client B does not, an inbound `FA…;` from the (mock) radio is delivered to A only. |
| `FR-CATSRV-05` | forward client SET commands to the radio through the session command path, and forward **unrecognised GETs** to the radio with the reply routed back to the requesting client by mnemonic match within a bounded pending window (timeout ⇒ dropped and logged). | STK-21 | S | T | A client `FA00014074000;` reaches the mock radio via the session; a client `XX;` is forwarded and the mock's `XX…;` reply returns to that client; an unanswered passthrough expires without leaking queue entries. |
| `FR-CATSRV-06` | never forward **session-owned or hazardous** commands (`RRN`, `RRT`, `PS`, `EM`, `SL`, `ER`, `RDY` as SET), handling `AI`/`ID`/K-mode/`RDY` locally per client and dropping the rest with a diagnostics entry. | STK-21/17 | S | T | Each blocked mnemonic sent by a client produces no send on the mock radio link and one diagnostics record; `ID;` is answered locally with the documented compat value. |
| `FR-CATSRV-07` | execute keying commands from CAT clients (`TX`, `KY`, `KZ`, `PB`, keying `SW` codes) **only** when the *CAT clients may transmit* setting (default off) is enabled **and** TX is armed (`FR-TX-SAFE-03`), routing them through the session's `begin_tx`/`end_tx` so `FR-TX-SAFE-01/02/04` apply unchanged; `RX;` shall always be forwarded; a refused keying command shall be wire-silent, logged, and surfaced via the ARM-control pulse (`FR-TX-PTT-01` affordance). | STK-08/13/21 | S | T | With the setting off or TX disarmed, a client `TX;` emits nothing on the radio link and logs; with setting on + armed, it keys via `begin_tx`; emergency stop during client-initiated TX unkeys and subsequent client `TX;` is refused until re-arm; client `RX;` unkeys in every state. |
| `FR-CATSRV-08` | while the upstream radio session is down, not serve stale cached state: GETs receive the K4 error form `<cmd>?;` (or the connection is refused) until the session is re-established and re-seeded. | STK-21/17 | S | T | After simulated link loss, a client `FA;` receives `FA?;` (or connect is refused per the chosen policy); after reconnect + seed, cache answers resume. |
| `FR-CATSRV-09` | surface the CAT server in the UI: an enable/port/bind/allow-TX settings group, a connected-client count indicator, and a `catsrv` diagnostics category logging client connect/disconnect and per-line traffic at debug level with no secrets. | STK-11/12/17 | C | D | Settings round-trip and persist; the header count matches open sockets; the diagnostics console shows a connecting logger's traffic. |
| `FR-CATSRV-10` | **[Phase C]** expose the same CAT service on a virtual serial endpoint: a pty pair created by the app on Linux/macOS (path shown in settings), and on Windows by bridging to one end of a user-installed com0com pair selected in settings (no kernel driver is shipped). | STK-21/18 | C | T/D | On Linux, opening the advertised pty with a terminal and sending `FA;` returns the cached frequency; on Windows the bridge is demonstrated against a com0com pair. |
| `FR-CATSRV-11` | **[Phase C, contingent on §8.3]** additionally serve the Hamlib `rigctld` text protocol (default 127.0.0.1:4532): at minimum `\get_freq \set_freq \get_mode \set_mode \get_vfo \get_ptt \set_ptt \dump_state`, with `RPRT x` error semantics. | STK-21 | C | T | `echo "\get_freq" | nc localhost 4532` returns the cached frequency; error paths return negative `RPRT` codes. |

Companion `NFR` (optional): *the CAT server shall add no measurable latency to the UI/radio
path and shall be absent from the build's protocol core dependencies* — arguably already
covered by NFR-PERF-01/NFR-MAINT-01; note it in the ADR instead of a new NFR.

---

## 10. Phased delivery plan

PR-per-change per the project workflow; each phase lands green through `xtask trace`.

| Phase | Content | New/changed | Rough effort |
|---|---|---|---|
| **0. Spike + capture** | `cat_resp` formatters for `FA/FB/MD/IF`; round-trip tests vs `apply_cat`; throwaway listener; run N1MM+/WSJT-X/Commander against it with `catsrv` diag capture to convert §8's [recalled] items into facts | `k4-protocol/src/cat_resp.rs` (or module), scratch harness | 1–2 days |
| **A. Core server (read + set, no TX)** | `k4-catsrv` crate: `ClientState`/`Policy`/`handle` + listener; worker channel integration; cache GETs, forwarded SETs, block list, disconnected policy; FR-CATSRV-01/02/03/05/06/08 tests incl. the `SimServer`-backed end-to-end | new crate; `app/src/worker.rs` wiring; SRS §O; architecture.md ARC-16/ADR-16 | 3–5 days |
| **B. AI push, TX gating, UI** | Per-client `AI` tee; pending-passthrough routing polish; keying gate + arm-pulse affordance + allow-TX setting; settings group, client-count indicator, diag category; FR-CATSRV-04/07/09; user-manual section with per-logger recipes | `k4-catsrv`, `app/src/main.rs`, `k4-config` prefs, docs | 3–4 days |
| **C1 (opt). Serial bridge** | pty endpoint (Linux/macOS), com0com bridge + docs (Windows); FR-CATSRV-10 | `k4-catsrv` feature, settings | 2–3 days |
| **C2 (opt). rigctld dialect** | Only if Hamlib-K4-over-TCP testing fails; FR-CATSRV-11 | `k4-catsrv` module | 2–3 days |
| **C3 (idea, unscoped)** | N1MM spectrum re-serve on port+1 — wire format currently unknown to us; pure research first | — | unknown |

Total to a useful, safe release (0+A+B): **~7–11 days**.

---

## Sources

- Repo: files cited inline (`crates/k4-protocol/src/{cat.rs,state.rs}`,
  `crates/k4-{transport,session,sim}/src/lib.rs`, `app/src/worker.rs`,
  `docs/concept/architecture.md`, `docs/requirements/system-requirements.md`,
  `docs/references/external-references.md`).
- [rigctld(1) — Hamlib TCP rig control daemon](https://hamlib.sourceforge.net/html/rigctld.1.html)
- [N1MM Logger+ Update 1.0.9429 — TCP radio connection, tested with Elecraft K4](https://n1mmwp.hamdocs.com/mmfiles/n1mm-logger-update-1-0-9429-february-1-2022/)
- [N1MM Logger+ Spectrum Display Window (K4 port 9200 / spectrum on next port)](https://n1mmwp.hamdocs.com/manual-windows/spectrum-display-window/)
- [DXLab: Getting Started with Elecraft Transceiver Control (K4 "Communicate via TCP")](https://www.dxlabsuite.com/dxlabwiki/TransceiverControlElecraft)
- [WT8P: Configuring Elecraft K4D for WSJT-X / FT8](https://www.wt8p.com/configuring-elecraft-k4d-for-ft8/)
- [Elecraft K4 Programmer's Reference (ftp.elecraft.com)](https://ftp.elecraft.com/K4/Manuals%20Downloads/K4%20Programmer's%20Reference%20rev%20C10/K4ProgrammersReferencerev.C10.html)
- [K4-Companion (TCP/IP K4 remote control)](https://github.com/DaleFarnsworth/K4-Companion)
