//! Real USB HID I/O for the K-Pod (feature `hidapi`, L4 hardware).
//!
//! The K-Pod exchanges fixed 8-byte packets over the control endpoint (EP0). We
//! drive it as HID feature reports: send the command (`SET_REPORT`) then read the
//! reply (`GET_REPORT`). The 1-byte report-ID prefix hidapi requires is `0` (the
//! K-Pod has no report IDs). **Untested against hardware** — see
//! `docs/test/hil-runs/`; the wire framing here is validated on a real K-Pod.

use hidapi::{HidApi, HidDevice, HidError};

use crate::{poll_packet, Report, PRODUCT_ID, VENDOR_ID};

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
    /// idle poll.
    fn exchange(&self, packet: [u8; 8]) -> Result<Report, HidError> {
        // hidapi wants a leading report-ID byte (0 = no report ID).
        let mut out = [0u8; 9];
        out[1..].copy_from_slice(&packet);
        self.dev.send_feature_report(&out)?;
        let mut buf = [0u8; 9];
        buf[0] = 0; // report ID to read
        let n = self.dev.get_feature_report(&mut buf)?;
        // Report is the 8 bytes after the report-ID byte.
        let mut rpt = [0u8; 8];
        let take = n.saturating_sub(1).min(8);
        rpt[..take].copy_from_slice(&buf[1..1 + take]);
        Ok(Report::parse(&rpt))
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
