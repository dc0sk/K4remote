//! K4 protocol core: binary framing, authentication hashing, and CAT command
//! encoding/decoding.
//!
//! Hardware- and UI-independent (NFR-MAINT-01): this crate has no transport,
//! audio, or GUI dependency so it can be unit-tested without a radio
//! (NFR-TEST-02). It implements the K4/0 wire protocol documented in
//! `docs/references/external-references.md` (R-EXT-01) as a clean-room
//! reimplementation (CON-09) and the CAT commands from the K4 Programmer's
//! Reference.

pub mod auth;
pub mod cat;
pub mod cw;
pub mod frame;
pub mod state;

pub use cat::LineDecoder;
pub use cw::{encode_kz, KeyElement};
pub use frame::{encode_frame, FrameDecoder, PayloadType, END_MARKER, START_MARKER};
pub use state::{connect_state_seed, s_unit_label, Mode, RadioState};
