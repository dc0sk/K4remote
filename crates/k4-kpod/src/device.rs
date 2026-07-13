//! Real USB HID I/O for the K-Pod (feature `hidapi`, L4 hardware).
//!
//! The K-Pod exchanges fixed 8-byte packets over the HID **interrupt** endpoints:
//! the host `write()`s the command (an OUT report, prefixed with the report-ID
//! byte `0` that hidapi requires) and then `read()`s the 8-byte reply. It does
//! **not** implement HID feature reports (`GET_REPORT`/`SET_REPORT` over EP0
//! return "Broken pipe" — confirmed on real hardware). The K-Pod uses report ID
//! `0`, so `read()` returns the report body directly.

use hidapi::{HidApi, HidDevice, HidError};

use crate::{poll_packet, Report, PRODUCT_ID, VENDOR_ID};

/// Read timeout for a command reply (ms). The K-Pod answers promptly; a short
/// timeout keeps the worker loop responsive if a reply is ever missed.
const READ_TIMEOUT_MS: i32 = 40;

/// An open K-Pod HID device.
pub struct Kpod {
    dev: HidDevice,
}

impl Kpod {
    /// Open the first attached K-Pod (by USB VID/PID). Returns an error if no
    /// K-Pod is present or the HID backend is unavailable.
    pub fn open() -> Result<Kpod, HidError> {
        let api = HidApi::new()?;
        let dev = api.open(VENDOR_ID, PRODUCT_ID)?;
        Ok(Kpod { dev })
    }

    /// Send an 8-byte command packet and read the 8-byte reply report. The reply
    /// is parsed; for `poll` its `is_event()` distinguishes a real event from an
    /// idle poll. A read that times out is treated as an idle report (`cmd = 0`).
    fn exchange(&self, packet: [u8; 8]) -> Result<Report, HidError> {
        // Write an OUT report: hidapi prepends the report-ID byte (0 = unnumbered).
        let mut out = [0u8; 9];
        out[1..].copy_from_slice(&packet);
        self.dev.write(&out)?;
        // Read the reply. The K-Pod uses report ID 0, so `read` returns the 8-byte
        // report body directly (no leading report-ID byte).
        let mut buf = [0u8; 8];
        let n = self.dev.read_timeout(&mut buf, READ_TIMEOUT_MS)?;
        if n == 0 {
            return Ok(Report::parse(&[0u8; 8])); // no reply yet → idle
        }
        Ok(Report::parse(&buf))
    }

    /// Poll the K-Pod once for a new event (encoder / button / rocker).
    pub fn poll(&self) -> Result<Report, HidError> {
        self.exchange(poll_packet())
    }

    /// Send a command packet that returns no meaningful data (Configure, LED/Aux,
    /// Beep) — a read is still performed per the spec.
    pub fn command(&self, packet: [u8; 8]) -> Result<(), HidError> {
        self.exchange(packet).map(|_| ())
    }
}
