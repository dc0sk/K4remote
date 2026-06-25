//! StreamCodec seam (ARC-10): codecs for the payload bodies above the frame
//! layer (`k4-protocol::frame`).
//!
//! - [`audio`] — audio packet (`0x01`), v1 (FR-AUD-04).
//! - [`pan`] — panadapter/spectrum packet (`0x02`), Phase-2 groundwork (FR-PAN-01).
//!
//! Isolating these codecs here localises any correction to the
//! reverse-engineered protocol (ADR-12). Decoding the audio *bytes* (Opus/PCM)
//! and the device I/O live in `k4-audio`.

pub mod audio;
pub mod pan;
pub mod render;

pub use audio::{AudioPacket, EncodeMode};
pub use pan::PanFrame;
pub use render::{dbm_to_color, dbm_to_y};
