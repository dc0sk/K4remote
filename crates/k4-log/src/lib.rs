//! QSO logging to **Wavelog** through its API v2 (FR-LOG; design
//! `docs/concept/wavelog-logging-plan.md` v0.3).
//!
//! Pure and hardware-free, like `k4-spot`: the QSO record and its mapping from the radio's state
//! ([`qso`], [`band`]), the lookup ([`lookup`]), the server URL rules ([`server`]), the token type ([`secret`]), the
//! classification of every reply ([`reply`]), the persisted queue ([`queue`]) and its ADIF exit
//! ([`adif`]). The HTTP transport is the app's.

pub mod adif;
pub mod band;
pub mod lookup;
pub mod qso;
pub mod queue;
pub mod reply;
pub mod secret;
pub mod server;
