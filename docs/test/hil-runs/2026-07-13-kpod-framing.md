# HIL run — 2026-07-13 — DC0SK

- Radio: n/a (K-Pod-only session)
- Client: k4remote (feat/kpod fix branch), Linux (Manjaro), hidapi hidraw backend
- Device: Elecraft K-Pod, USB `04D8:F12D` → `/dev/hidraw1`

Focus: validate the K-Pod HID wire framing (`FR-KPOD-04`), which was previously
unverified.

| Item | Result | Notes |
|---|---|---|
| K-Pod enumerate (VID/PID) | **pass** | `hidapi` lists `04d8:f12d` "Elecraft Inc. KPOD Application", `/dev/hidraw1`, usage_page `0xff00` |
| Device node permissions | pass | `/dev/hidraw1` is `crw-rw-rw-` (accessible) |
| `open(0x04D8, 0xF12D)` | **pass** | |
| Feature-report exchange (`SET/GET_REPORT`, EP0) | **FAIL** | `ioctl (SFEATURE): Broken pipe` — the K-Pod does **not** implement HID feature reports |
| `write()` / `read()` exchange (interrupt) | **pass** | 9-byte OUT (report-ID `0` + 8) then 8-byte IN report body |
| Idle poll | pass | `'u'` → report `cmd = 0` (no event) parsed as idle |
| Encoder ticks | **pass** | live events captured: `ticks=1`, then `ticks=111` (accumulated) |
| Rocker decode | **pass** | reported `rocker = VfoA` (left) as set on the device |
| Sustained poll (~4 s) | pass | 182 polls, 2 events, **0 errors** |

**Outcome / fix:** `device.rs` originally used `send_feature_report` /
`get_feature_report`, which the K-Pod rejects — so every poll errored and the
app never recognised the device. Changed to `write()` + `read_timeout()`
(interrupt endpoints). Confirmed working above. The worker was also moved to a
dedicated K-Pod poll thread so the blocking USB read can't stall the audio/CAT
loop, and recognition is now independent of the radio connection.

Follow-ups: confirm rocker → VFO B / RIT-XIT selection and encoder tuning of a
real radio's VFO end-to-end (needs the K4 in the loop); button/tap-hold mapping
is out of scope for the current VFO-selection + frequency-control requirement.
