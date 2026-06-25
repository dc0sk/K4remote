---
title: "Stakeholder Requirements"
status: Draft
version: "0.1"
updated: 2026-06-25
authors:
  - Simon Keimer (DC0SK)
owns: [STK]
---

# Stakeholder Requirements

**Version:** 0.1 (Draft) · **Date:** 2026-06-25 · **Author:** DC0SK
Trace: owns `STK-`. Upstream of [system-requirements.md](system-requirements.md); downstream of
[vision-and-scope.md](vision-and-scope.md). Each `STK` is realized by ≥1 `FR`/`NFR` (rule R1).

These are **solution-independent needs**. They say *what* stakeholders need, not *how*.
`Pri`: M=must (v1), S=should (v1), C=could, W2=Phase 2.

| ID | Stakeholder | Need (the system shall enable the operator to…) | Pri | Realized by (FR/NFR) |
|---|---|---|---|---|
| `STK-01` | SH-1 | Establish and maintain a remote connection to a specific K4 over the network, and see clearly whether the link is up. | M | FR-CONN-*, FR-SES-*, NFR-REL-* |
| `STK-02` | SH-1 | Tune the radio (VFO A/B frequency, band) and trust that the displayed frequency matches the radio. | M | FR-VFO-*, FR-CAT-* |
| `STK-03` | SH-1 | Select operating mode, bandwidth/filter, and core receiver settings (AGC, gains, attenuator, preamp, NB/NR). | M | FR-MODE-*, FR-RX-* |
| `STK-04` | SH-1 | See live receive signal strength and key radio status without manual polling. | M | FR-MTR-*, FR-CAT-AI |
| `STK-05` | SH-1 | Hear the receiver audio remotely with usable quality and latency. | M | FR-AUD-RX-*, NFR-PERF-* |
| `STK-06` | SH-1 | Transmit by voice (PTT + mic audio) from the remote position. | M | FR-TX-*, FR-AUD-TX-* |
| `STK-07` | SH-1 | Send CW remotely from a paddle/key with acceptable timing. | M | FR-TX-CW-*, NFR-PERF-CW |
| `STK-08` | SH-1 | Be protected from accidental or runaway transmission, including on link loss. | M | FR-TX-SAFE-*, NFR-REL-FAILSAFE |
| `STK-09` | SH-1 | View a real-time spectrum and waterfall of the band being monitored. | W2 | FR-PAN-* |
| `STK-10` | SH-1 | Adjust the panadapter display (span, reference, scale, averaging, waterfall colour/height). | S | FR-PAN-CTL-* |
| `STK-11` | SH-1 | Operate through a clear, responsive GUI that resembles a real operating position. | M | FR-UI-*, NFR-USE-* |
| `STK-12` | SH-1 | Save and reuse connection profiles and preferences. | S | FR-CFG-* |
| `STK-13` | SH-4 | Operate transmit lawfully — only deliberate keying, with station identification possible. | M | FR-TX-SAFE-*, FR-VFO-ID |
| `STK-14` | SH-5 | Keep the link secure (authenticated, secrets protected) and bandwidth-appropriate. | M | NFR-SEC-*, FR-AUD-ENC |
| `STK-15` | SH-2 | Maintain the software sustainably with automated tests and full requirement→test traceability. | M | NFR-TEST-*, NFR-MAINT-* |
| `STK-16` | SH-2 | Run the application on common desktop platforms. | S | NFR-PORT-* |
| `STK-17` | SH-1/2 | Diagnose connection and protocol problems from logs without a debugger. | S | FR-DIAG-*, NFR-MAINT-LOG |
| `STK-18` | SH-2 | Add the USB/serial (local) transport later without re-architecting. | S | FR-CONN-ABSTRACT, ADR-02 |
| `STK-19` | SH-1 | Be informed when other clients are connected to the same radio (shared-control awareness). | C | FR-SES-MULTI |
| `STK-20` | SH-1 | Recover gracefully and automatically from transient network interruptions. | S | FR-SES-RECONNECT, NFR-REL-* |

## Change history

| Date | Ver | Author | Change |
|---|---|---|---|
| 2026-06-25 | 0.1 | DC0SK | Initial draft baseline (STK-01..20). |
