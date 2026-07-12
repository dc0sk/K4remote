//! Elecraft **K-Pod** USB HID control surface — VFO selection + frequency
//! control (FR-KPOD-01/02/03).
//!
//! Pure protocol per the *K-Pod USB Application Interface Specification* v1.03
//! (© Elecraft, N6HZ): a generic HID device (Vendor `0x04D8`, Product `0xF12D`)
//! that exchanges fixed 8-byte packets. The host sends a 1-byte command + 7 data
//! bytes and then reads back an 8-byte report. This module is dependency-free and
//! fully unit-tested; the actual USB HID I/O lives behind the `hidapi` feature
//! (L4 hardware — see [`device`]).
//!
//! Behaviour follows the K-Pod Owner's Manual (Rev F): the **rocker** switch
//! assigns the knob to VFO A, VFO B, or RIT/XIT, and indicator LEDs D1/D2/D3
//! reflect that selection; the **encoder** tunes the selected control.

#![cfg_attr(not(feature = "hidapi"), forbid(unsafe_code))]

/// USB Vendor ID from the device descriptor.
pub const VENDOR_ID: u16 = 0x04D8;
/// USB Product ID from the device descriptor.
pub const PRODUCT_ID: u16 = 0xF12D;

/// Command bytes (PC → K-Pod), the first byte of the 8-byte command packet.
pub mod cmd {
    /// Get update — signal the K-Pod to return an event report.
    pub const GET_UPDATE: u8 = b'u';
    /// Get ID — returns `"KPOD"`.
    pub const GET_ID: u8 = b'=';
    /// Get firmware version (BCD in the report's `ticks` field).
    pub const GET_VERSION: u8 = b'v';
    /// Hard reset (also resets the USB interface).
    pub const RESET: u8 = b'r';
    /// Configure encoder scale + beeper mute.
    pub const CONFIGURE: u8 = b'C';
    /// LED / Aux output control.
    pub const LED_AUX: u8 = b'O';
    /// Beeper control.
    pub const BEEP: u8 = b'Z';
}

/// Rocker-switch position — selects what the encoder controls (Owner's Manual).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rocker {
    /// Left — knob tunes VFO A (wire `0b10`; indicator D1).
    VfoA,
    /// Center — knob tunes VFO B (wire `0b00`; indicator D2).
    VfoB,
    /// Right — knob adjusts the RIT/XIT offset (wire `0b01`; indicator D3).
    RitXit,
    /// Error / indeterminate (wire `0b11`).
    Unknown,
}

impl Rocker {
    /// Decode the 2-bit rocker field (`controls` bits 5–6).
    fn from_bits(bits: u8) -> Rocker {
        match bits & 0b11 {
            0b00 => Rocker::VfoB,
            0b01 => Rocker::RitXit,
            0b10 => Rocker::VfoA,
            _ => Rocker::Unknown,
        }
    }
}

/// A decoded 8-byte K-Pod report packet (`cmd | int16 ticks | controls | 4×spare`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Report {
    /// Echoed command: `b'u'` when a new event occurred, `0` when idle.
    pub cmd: u8,
    /// Encoder ticks since the last poll: positive = CW (up), negative = CCW.
    pub ticks: i16,
    /// Pressed function switch F1–F8 (`1`–`8`), or `0` for none.
    pub button: u8,
    /// `true` = the switch was held, `false` = tapped.
    pub hold: bool,
    /// Rocker-switch position.
    pub rocker: Rocker,
}

impl Report {
    /// Parse an 8-byte report packet. `ticks` is a little-endian `int16` at bytes
    /// 1–2; `controls` (byte 3) packs the button (bits 0–3), tap/hold (bit 4),
    /// and rocker (bits 5–6).
    ///
    /// trace: FR-KPOD-03
    pub fn parse(pkt: &[u8; 8]) -> Report {
        let controls = pkt[3];
        Report {
            cmd: pkt[0],
            ticks: i16::from_le_bytes([pkt[1], pkt[2]]),
            button: controls & 0x0F,
            hold: controls & 0x10 != 0,
            rocker: Rocker::from_bits(controls >> 5),
        }
    }

    /// Whether this report carries a new event (`cmd == 'u'`).
    pub fn is_event(&self) -> bool {
        self.cmd == cmd::GET_UPDATE
    }
}

/// The 8-byte "get update" command packet used to poll the K-Pod.
pub fn poll_packet() -> [u8; 8] {
    let mut p = [0u8; 8];
    p[0] = cmd::GET_UPDATE;
    p
}

/// The `Configure` command packet: `scale_100` sets 100 encoder counts/turn
/// (else the 200-count default); `mute` mutes the beeper.
pub fn configure_packet(scale_100: bool, mute: bool) -> [u8; 8] {
    let mut p = [0u8; 8];
    p[0] = cmd::CONFIGURE;
    p[1] = (u8::from(scale_100) << 1) | u8::from(mute);
    p
}

/// The `LED/Aux` command packet with an explicit bit pattern
/// (`b0..b2` = AUX1–3, `b3..b6` = LED1–4, `b7` = LEDR/rocker-controlled).
pub fn led_aux_packet(bits: u8) -> [u8; 8] {
    let mut p = [0u8; 8];
    p[0] = cmd::LED_AUX;
    p[1] = bits;
    p
}

/// LED bit pattern that lights the indicator matching the current selection —
/// D1 for VFO A, D2 for VFO B, D3 for RIT/XIT — the K-Pod's default mapping.
///
/// trace: FR-KPOD-03
pub fn selection_leds(rocker: Rocker) -> u8 {
    match rocker {
        Rocker::VfoA => 1 << 3,   // LED1 / D1
        Rocker::VfoB => 1 << 4,   // LED2 / D2
        Rocker::RitXit => 1 << 5, // LED3 / D3
        Rocker::Unknown => 0,
    }
}

/// What a K-Pod event should do to the radio, given the tuning step per tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Tune VFO A (`vfo_b = false`) or VFO B by `delta_hz` (rocker left/center).
    Tune { vfo_b: bool, delta_hz: i64 },
    /// Adjust the RIT/XIT offset by `delta_hz` (rocker right).
    RitXit { delta_hz: i64 },
    /// Nothing to do (idle report, zero ticks, or an indeterminate rocker).
    None,
}

/// Map a report's encoder ticks to a tuning action on the rocker-selected
/// control. `step_hz` is the frequency change per encoder tick.
///
/// trace: FR-KPOD-01, FR-KPOD-02
pub fn action_for(report: &Report, step_hz: u32) -> Action {
    if !report.is_event() || report.ticks == 0 {
        return Action::None;
    }
    let delta = i64::from(report.ticks) * i64::from(step_hz);
    match report.rocker {
        Rocker::VfoA => Action::Tune {
            vfo_b: false,
            delta_hz: delta,
        },
        Rocker::VfoB => Action::Tune {
            vfo_b: true,
            delta_hz: delta,
        },
        Rocker::RitXit => Action::RitXit { delta_hz: delta },
        Rocker::Unknown => Action::None,
    }
}

/// Running tuning target so rapid encoder ticks accumulate locally instead of
/// being lost to the radio's echo latency (the same idea as the app's optimistic
/// VFO): seed from the radio's frequency on the first tick, add each delta, and
/// clear once the radio confirms the value.
///
/// trace: FR-KPOD-02
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tuner {
    target: Option<u64>,
}

impl Tuner {
    /// Apply `delta_hz` to the running target (seeded from `radio_hz` if none),
    /// returning the new frequency to command, or `None` if no base is known.
    pub fn tune(&mut self, radio_hz: Option<u64>, delta_hz: i64) -> Option<u64> {
        let base = self.target.or(radio_hz)?;
        let new = (base as i64 + delta_hz).max(0) as u64;
        self.target = Some(new);
        Some(new)
    }

    /// Drop the running target once the radio's read-back confirms it, so control
    /// hands back to the real value (and an externally-tuned change is adopted).
    pub fn sync(&mut self, radio_hz: Option<u64>) {
        if self.target == radio_hz {
            self.target = None;
        }
    }
}

#[cfg(feature = "hidapi")]
pub mod device;

#[cfg(test)]
mod tests {
    use super::*;

    fn pkt(cmd: u8, ticks: i16, controls: u8) -> [u8; 8] {
        let t = ticks.to_le_bytes();
        [cmd, t[0], t[1], controls, 0, 0, 0, 0]
    }

    /// The rocker selects the VFO/RIT target and the encoder tick sign gives the
    /// tune direction (FR-KPOD-01/02); the report fields decode per the spec.
    ///
    /// trace: FR-KPOD-01, FR-KPOD-02, FR-KPOD-03
    #[test]
    fn fr_kpod_report_and_action() {
        // controls: rocker left (0b10 << 5), tap, button 3.
        let r = Report::parse(&pkt(b'u', 5, (0b10 << 5) | 0x03));
        assert!(r.is_event());
        assert_eq!(r.ticks, 5);
        assert_eq!(r.button, 3);
        assert!(!r.hold);
        assert_eq!(r.rocker, Rocker::VfoA);
        assert_eq!(
            action_for(&r, 10),
            Action::Tune {
                vfo_b: false,
                delta_hz: 50
            }
        );

        // Rocker center + negative ticks → VFO B down.
        let r = Report::parse(&pkt(b'u', -3, 0b00 << 5));
        assert_eq!(r.rocker, Rocker::VfoB);
        assert_eq!(
            action_for(&r, 10),
            Action::Tune {
                vfo_b: true,
                delta_hz: -30
            }
        );

        // Rocker right (0b01) + hold flag → RIT/XIT.
        let r = Report::parse(&pkt(b'u', 2, (0b01 << 5) | 0x10));
        assert!(r.hold);
        assert_eq!(r.rocker, Rocker::RitXit);
        assert_eq!(action_for(&r, 10), Action::RitXit { delta_hz: 20 });

        // Idle report (cmd 0) or zero ticks → no action.
        assert_eq!(action_for(&Report::parse(&pkt(0, 9, 0)), 10), Action::None);
        assert_eq!(
            action_for(&Report::parse(&pkt(b'u', 0, 0)), 10),
            Action::None
        );
    }

    /// Command packets and the selection-LED mapping match the spec/manual.
    ///
    /// trace: FR-KPOD-03
    #[test]
    fn fr_kpod_command_packets() {
        assert_eq!(poll_packet()[0], b'u');
        assert_eq!(
            configure_packet(true, false),
            [b'C', 0b10, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            configure_packet(false, true),
            [b'C', 0b01, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(led_aux_packet(0x18), [b'O', 0x18, 0, 0, 0, 0, 0, 0]);
        // D1 for VFO A, D2 for VFO B, D3 for RIT/XIT.
        assert_eq!(selection_leds(Rocker::VfoA), 1 << 3);
        assert_eq!(selection_leds(Rocker::VfoB), 1 << 4);
        assert_eq!(selection_leds(Rocker::RitXit), 1 << 5);
    }

    /// The running tuner accumulates rapid ticks and hands back on confirm.
    ///
    /// trace: FR-KPOD-02
    #[test]
    fn fr_kpod_tuner_accumulates() {
        let mut t = Tuner::default();
        // Seed from the radio, accumulate two ticks before any echo returns.
        assert_eq!(t.tune(Some(14_000_000), 10), Some(14_000_010));
        assert_eq!(t.tune(Some(14_000_000), 10), Some(14_000_020)); // radio still stale
                                                                    // Radio confirms the latest → target cleared, control handed back.
        t.sync(Some(14_000_020));
        assert_eq!(t, Tuner::default());
        // No base known → nothing to command.
        assert_eq!(Tuner::default().tune(None, 10), None);
        // Never underflows below 0.
        assert_eq!(Tuner::default().tune(Some(5), -100), Some(0));
    }
}
