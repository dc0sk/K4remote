---
title: "QSO logging to Wavelog (API v2) — design (FR-LOG)"
status: Draft
version: "0.2"
updated: 2026-10-27
authors:
  - Simon Keimer (DC0SK)
---

# QSO logging to Wavelog (API v2) — design (FR-LOG)

DC0SK wants K4 Remote to log contacts to his **Wavelog** logbook (the one the DARC hosts for its
members) through Wavelog's **API v2**, with the API token entered in K4 Remote and kept with the
other secrets it handles. Write tests run against a **local Wavelog instance**, never against his
real logbook. His choices (2026-10-27): a log form in K4 Remote, prefilled from a spot click,
with a callsign lookup while typing; a local queue that retries when Wavelog cannot be reached;
the station location picked from Wavelog's list.

**v0.2** takes every finding of the independent review of v0.1 (§10, verdict "revise, major"):
the duplicate key was wrong and with it the retry rule; errors are classified by Wavelog's error
code, not the HTTP status; the queue is a directory of files, not one rewritten file; the token
rules are mechanisms (a type, one account, tests) instead of sentences; power, split, time and
the band table are defined.

## 1. What the API does — checked against a real instance

Sources: the API v2 reference (docs.wavelog.org/developer/api-v2/ and its `qso`, `station`,
`lookup`, `token` pages), the Wavelog PHP source in the running container, and **probes against a
local Wavelog** (§6; release 3.2.x, database version 307). Where they differ, the probe wins.

- **Availability:** API v2 exists from **Wavelog 3.1.0**. `GET …/api/v2/status` is public. An
  unknown v2 resource answers a **JSON** 404 (`error.code: not_found`); a Wavelog without v2
  answers an **HTML** "Page Not Found".
- **Authentication:** `Authorization: Bearer wl2_…` (`wl2_` + 40 hex); Wavelog stores only its
  SHA-256. Scopes needed: **`qso:write`** (log), **`station:read`** (locations), **`lookup:read`**
  (lookup). `GET /api/v2/token` (no scope) returns `scopes`, `owner` and `expires_at`. Failed
  authentication is rate-limited **per client IP** — a dead token also throttles the lookup.
- **Create:** `POST /api/v2/qso`, JSON, scalar values only. Required: `station_profile_id`,
  `call`, `band`, `mode`, `qso_date` (`YYYY-MM-DD`), `time_on` (`HHMM` or `HHMMSS` — probed:
  `075030` is stored as 07:50:30). Optional ADIF fields in lower case: `submode`, `freq` and
  `freq_rx` (**Hz**), `time_off`, `rst_sent`, `rst_rcvd`, `name`, `comment`, `tx_pwr`, `operator`, …
  Success: **201** with the stored QSO (`data.id`).
- **The band is not checked when `freq` is sent** (probed: `band: "21m"` with `freq` 14.25 MHz
  → 201, stored as "21m"). K4 Remote derives the band itself, from a table equal to Wavelog's band
  names (§5).
- **Modes are stored as ADIF mode + submode, and both the duplicate rule and the lookup use the
  main mode** (probed): `FT4` → `MFSK`/`FT4`, `JS8` → `MFSK`/`JS8`, `PSK31` → `PSK`/`PSK31`; a JS8
  QSO in the same minute as an FT4 one with the same call is a **duplicate** (both `MFSK`); a
  lookup with `mode=MFSK` or `PSK` matches them, `mode=FT4` or `PSK31` does not.
- **Duplicate key — minute + call + band + main mode + station location** (source:
  `Logbook_model.php` `import()`; probed: same call/band/minute in CW after SSB → 201, a second
  QSO). A duplicate answers **400 `validation_error`** with `details.duplicate` non-empty; the
  data is not merged (probed: same key with another RST → duplicate, not updated).
- **Errors by `error.code`:** `validation_error` (with `details.missing`, `details.validation` or
  `details.duplicate`, and sometimes **no `details`**), `invalid_json`, `unauthorized`,
  `invalid_token`, `token_expired` (401); `insufficient_scope` (403, `details.required_scope`);
  **`forbidden`** (403 — probed: a `station_profile_id` not the token owner's, "station_profile_id
  does not belong to the API key owner"); `not_found` (404); `method_not_allowed` (405);
  `rate_limited` (429, `details.retry_after` and a `Retry-After` header); `internal_error` (500).
  A 500 can come **after** the insert was committed (insert, then re-read, then respond).
- **Stations:** `GET /api/v2/station` → `id`, `name`, `callsign`, `gridsquare`, `active`.
- **Lookup:** `GET /api/v2/lookup?callsign=…&band=…&mode=<main mode>&detail=full` → `dxcc`,
  `cont`, `name`, `gridsquare`, `call_worked`, `call_worked_band`, `call_worked_band_mode`.
- **Club tokens:** the server forces `operator` to the acting member; a personal token leaves it
  as sent.

## 2. The parts

1. **`k4-log` (new crate, no HTTP or TLS stack — like `k4-spot` takes a fetcher):** the QSO
   record and its mapping from the radio's state (§5), the request bodies, the reply classifier
   (§3), the queue directory (§4), and `Secret` (§7). Pure and hardware-free.
2. **App — a "Log" panel** under the panadapter row: call, RST sent/received, name, comment, a
   time field (date and time as one value), and a line with what will be logged from the radio.
   One **Log** button (and Enter). A lookup line beside the call (§5). Shown whenever URL, token
   and station location are saved — reachability does not matter, the queue takes the QSO.
3. **App — Settings → LOGGING tab:** URL, token (entered once, then shown as `wl2_…` + last four),
   **Test**, station location, and the queue: pending and held entries with reasons, "send now",
   "save as ADIF…", "discard" (asks once per entry).
4. **App — a logging worker thread:** owns the transport and the sender's lock (§4), sends one
   entry at a time in queue order, retries, looks up; the UI never waits on the network.

## 3. Sending: may-have-reached, and the reply classes

Each entry records **`last_attempt`**: `not_sent` (never sent, or the last try provably did not
reach the server: name resolution, connection refused, TLS failure, or any 4xx) or `unknown` (the
request may have been processed: a timeout or reset after it was written, or a 5xx). An entry's
body is **frozen** once first sent; editing a held entry resets `last_attempt` to `not_sent`.

The classifier reads **`error.code`** (and the status only where there is no JSON `error`), and
never assumes `details` is an object:

| Reply | Class | Entry / queue |
|---|---|---|
| 201 | **Logged** | removed; the Wavelog id shown |
| `validation_error` + `details.duplicate`, `last_attempt == unknown` | **Already logged** (an earlier try got through) | removed; "already in the log" |
| `validation_error` + `details.duplicate`, `last_attempt == not_sent` | **Duplicate** | **held**: "Wavelog already has a QSO with <call> on <band> <mode> at <minute> — discard, or edit and resend" |
| other `validation_error`, `invalid_json` | **Rejected** | held with Wavelog's message (HTML removed) |
| `forbidden` | **Wrong location** | held: "station location <id> is not yours any more — pick one and re-target" |
| `invalid_token`, `token_expired`, `unauthorized` | **Not authorised** | queue **paused** (and the lookup stopped) until a new token is saved |
| `insufficient_scope` | **Scope missing** | queue paused: "the token lacks <scope>" |
| JSON `not_found` | **Not found** | held: "report this" (a K4 Remote fault) |
| non-JSON 404 | **No API v2** | queue paused: "no API v2 here (Wavelog 3.1.0 or later), or a wrong URL" |
| 3xx | **Redirect** | queue paused: "the URL redirects to <host> — enter that URL"; never followed |
| `method_not_allowed` | **Blocked** | queue paused, with the message (a proxy refusing POST) |
| `rate_limited` | **Throttled** | kept; next try after `Retry-After`/`details.retry_after`, at most 1 h |
| TLS certificate refused | **Certificate** | queue paused: "certificate refused for <host>"; no click-to-approve (a token travels on it) |
| 5xx, timeout, connection failure | **Unreachable** | kept, `last_attempt` set as above; retried after 30 s, doubling to 30 min |

"Send now" and logging a new QSO retry at once. A paused queue keeps its entries.

## 4. The queue: a directory of files

- **Every QSO goes through it**, also when Wavelog is reachable: the entry is written and
  `fsync`ed before the form reports "queued"; then it is sent.
- Directory `wavelog-queue/` in the config directory; **one file per entry**,
  `<queued_at>-<uuid>.json`, written as `.tmp` + `fsync` + rename. Removing an entry is `unlink`;
  updating one (held, attempts) is the same temp + rename. No shared rewrite, so neither two K4
  Remote instances nor a crash during a write can lose another entry; a torn write leaves only a
  `.tmp`, ignored and cleaned up.
- **One sender:** the worker sends only while holding an `flock` on `wavelog-queue/.lock`; a
  second instance shows "queue sent by another K4 Remote" and only adds entries.
- Each file: `{"v":1, "id", "queued_at", "url", "station_profile_id", "qso": {…API fields…},
  "state": "pending"|"held", "reason", "attempts", "last_attempt": "not_sent"|"unknown"}`.
  A file that does not parse, or has an unknown `v`, is renamed `.unreadable` on its own.
- `url` and `station_profile_id` are taken when the QSO is logged; an entry whose URL no longer
  matches the configured one, or has no token, is **held**: "no token for <url>".
- **No QSO is lost to configuration:** a held entry can be **saved as ADIF** (its fields are ADIF
  names already) before it is discarded.
- **Logging switched off or the URL cleared:** the queue is kept, not sent, not discarded; its
  counts stay visible.
- Bounded at 10 000 entries (pending + held); past that the Log button refuses, naming "save as
  ADIF" as the way out.

## 5. The log form and the mapping

- **No radio connected:** frequency, band and mode are editable (otherwise read-only, from the
  radio), and Log works from what is typed. The demo shows the radio's demo values.
- **Times:** `time_on` is taken when the call field first gets text, `time_off` when Log is
  pressed; both UTC, with seconds; date and time edited as one value (midnight).
- **Frequencies:** `freq` = the **transmit** VFO (B in split), `freq_rx` = the receive VFO when it
  differs; band from `freq`.
- **Band table = Wavelog's bands:** `160m 80m 60m 40m 30m 20m 17m 15m 12m 10m 6m 4m 2m 1.25m 70cm
  33cm 23cm 13cm 9cm 6cm 3cm 1.25cm`, with the edges of Wavelog's own frequency-to-band function
  as the fixture (Wavelog is MIT-licensed; attributed). A frequency in no band refuses the Log
  with a form error. Transverter bands follow from the displayed frequency.
- **Modes (ADIF mode / submode):** LSB, USB → `SSB` / `LSB`, `USB`; CW, CW-R → `CW`; AM → `AM`; FM
  → `FM`; DATA, DATA-R → the operator's choice, sent as ADIF pairs: FT8 → `FT8`; FT4 → `MFSK`/`FT4`;
  JS8 → `MFSK`/`JS8`; RTTY → `RTTY`; PSK31 → `PSK`/`PSK31`; the last choice is remembered. The
  lookup is asked with the **main** mode (`SSB`, `CW`, `MFSK`, …).
- **Power:** `tx_pwr` in **watts** from the K4's power setting and its range (H: W, L: 0.1 W,
  X: mW); left out when either is unknown. With the KPA1500 in OPERATE, the radiated power is the
  amplifier's — open point §9.
- **Call:** trimmed and upper-cased. **RST** defaults: `59` phone, `599` CW and RTTY, empty for
  FT8/FT4/JS8; editable. **`operator`:** the operator's callsign from Settings (as used for the
  cluster login), so a club logbook attributes the QSO.
- **Spot click:** still tunes as before, and puts the spot's callsign into an **empty** call field;
  never over one being typed, never sends.
- **Lookup while typing:** 400 ms after the last keystroke, when the call has at least three
  characters and a digit; one request with band and main mode; shows DXCC, name, grid and
  worked / on this band / on this band and mode / new. Answers for a call no longer in the field
  are dropped; offline, or while the queue is paused for authorisation, there is no lookup.
  The call goes to the operator's own logbook server only.
- **Out of scope:** editing or deleting a QSO already in Wavelog — corrections are made in
  Wavelog.

## 6. The local test instance

A rootless podman pod bound to `127.0.0.1`, made by `tools/wavelog-test/setup.sh` (MariaDB 11.8 +
Wavelog, installer completed by `install.py`, test user `N0CALL`, one station location, tokens
`FULL` and `LOOKUP_ONLY` inserted as Wavelog's own model makes them) and removed by `teardown.sh`.
Checked 2026-10-27 on a second instance from nothing: setup, API answering, a write with the
lookup-only token refused (403), teardown.

Tests:
- **Unit (`k4-log`):** the mode, band, power, split and time mapping; request bodies; every reply
  class of §3 from recorded replies (the probe bodies above as fixtures, including one without
  `details`); `last_attempt` and the duplicate rule; the queue directory (atomic writes, a torn
  `.tmp`, an unreadable file kept aside, the bound).
- **App, against a mock HTTP server:** logged; already logged after a lost reply; duplicate held;
  rejected; wrong location; 401 pauses and stops the lookup; redirect; unreachable then back; two
  queue owners (one sends, both add); the `Authorization` header carries the token while **no**
  diagnostics line, status string or held reason contains it.
- **Against the local instance** (`#[ignore]`; run with `K4_WAVELOG_URL`/`K4_WAVELOG_TOKEN`):
  log (HHMMSS), a resend after a simulated lost reply logs once, a duplicate held, a missing scope,
  a wrong location, the lookup by main mode, the station list. The test refuses to run unless the
  host is `127.0.0.1`, `::1` or `localhost`.
- **Never** against the DARC instance: the first real QSO is DC0SK's, on the radio.

## 7. The token

- **One** secret-store account, `wavelog-token`, in the app's store (`KeyringStore("k4remote")`),
  read off the UI thread with a time limit (as the connection password, FR-CFG-08). The URL it
  belongs to is stored in the config beside it (`[wavelog] url`); **changing the URL deletes the
  token** and asks for a new one, so "sent only to the URL it was saved for" is an invariant of
  the config, not a naming scheme.
- **The URL is normalised once:** scheme and host lower-case, trailing `/` and `/index.php`
  removed; K4 Remote appends `/index.php/api/v2/…` itself (both forms answer on the probed
  instance — open point §9 for the DARC one).
- **HTTPS only**, except `http://127.0.0.1`, `http://[::1]` and `http://localhost`. Redirects are
  never followed (§3).
- **`Secret`**, a newtype in `k4-log`: `Debug` prints `wl2_…<last four>`, no `Display`, no
  `Serialize`; the only accessor is called in one place, the transport. A structural test forbids
  that accessor anywhere else (as `spot_sources.rs` bans `to_socket_addrs` by file), and the new
  transport file joins that DNS ban list too.
- The Test shows the token's `scopes`, `owner` and `expires_at`, and warns ahead of the expiry.
  A missing `qso:write` is an error; a missing `lookup:read` or `station:read` turns the lookup or
  the location list off with a warning (the location id can then be typed).

## 8. Requirements

`FR-LOG-01` … `FR-LOG-08` in the SRS, section R, rewritten to this version.

## 9. Open points for DC0SK

- The URL of the DARC Wavelog; whether it runs 3.1.0 or later (the Test will say) and whether it
  needs the `/index.php/` form.
- Power: the K4's set power is what K4 Remote knows; with the KPA1500 in OPERATE, log the
  amplifier's output instead?

## 10. Review of v0.1 (2026-10-27)

Independent review (Fable), against the design, the K4 Remote source, the Wavelog source in the
container and read-only probes: **revise (major)**, 11 findings, all taken here. Its write probes
were run afterwards and confirmed every testable claim: (a) the band is unchecked when `freq` is
sent; (b) FT4/PSK31/JS8 stored as `MFSK`/`PSK` + submode — and the duplicate rule and the lookup
compare the main mode (found by a probe collision: JS8 after FT4 in one minute was a duplicate);
(c) seconds are stored; (d) a foreign `station_profile_id` is 403 `forbidden`; (e) the same key in
another mode is a new QSO; (f) a duplicate is not merged.
