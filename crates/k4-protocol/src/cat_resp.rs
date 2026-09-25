//! CAT **replies** from the cached radio state (FR-CATSRV-03): the mirror image of
//! [`RadioState::apply_cat`], for a CAT server that answers a client's GET from the app's cache
//! instead of asking the radio. Each formatter gives the K4 RESP wire form (PRG D12), `;`
//! included, or `None` when the radio has not reported that field yet — a reply is never made
//! up. Meta-mode differences are the caller's to pass in (only `IF` depends on one here).

use crate::state::RadioState;

/// `FAnnnnnnnnnnn;` — VFO A, 11 digits.
pub fn fa(s: &RadioState) -> Option<String> {
    s.vfo_a_hz.map(|hz| format!("FA{hz:011};"))
}

/// `FBnnnnnnnnnnn;` — VFO B, 11 digits.
pub fn fb(s: &RadioState) -> Option<String> {
    s.vfo_b_hz.map(|hz| format!("FB{hz:011};"))
}

/// `MDn;` (main) / `MD$n;` (sub).
pub fn md(s: &RadioState, sub: bool) -> Option<String> {
    let m = if sub { s.mode_b } else { s.mode_a }?;
    Some(format!(
        "MD{}{};",
        if sub { "$" } else { "" },
        m.md_digit() as char
    ))
}

/// `BWnnnn;` / `BW$nnnn;` — bandwidth in 10 Hz units, 4 digits.
pub fn bw(s: &RadioState, sub: bool) -> Option<String> {
    let hz = if sub {
        s.sub_bandwidth_hz
    } else {
        s.bandwidth_hz
    }?;
    Some(format!(
        "BW{}{:04};",
        if sub { "$" } else { "" },
        (hz / 10).min(9999)
    ))
}

/// `DTn;` / `DT$n;` — DATA sub-mode (0 DATA A, 1 AFSK A, 2 FSK D, 3 PSK D).
pub fn dt(s: &RadioState, sub: bool) -> Option<String> {
    let n = if sub {
        s.sub_data_submode
    } else {
        s.data_submode
    }?;
    Some(format!("DT{}{};", if sub { "$" } else { "" }, n.min(9)))
}

/// `FTn;` — split.
pub fn ft(s: &RadioState) -> Option<String> {
    s.split.map(|on| format!("FT{};", u8::from(on)))
}

/// `FR0;` — receive VFO. "Equivalent to FT0" (PRG): the K4 always receives on VFO A.
pub fn fr() -> &'static str {
    "FR0;"
}

/// `TQn;` — transmit state.
pub fn tq(s: &RadioState) -> Option<String> {
    s.transmitting.map(|tx| format!("TQ{};", u8::from(tx)))
}

/// `IF[f]*****+yyyyrx*00tm0spbd1*;` (PRG D12 p16), 37 characters before the `;`. `b` is always
/// `0` (its K22 meaning, "sent because of a band change", does not apply to a poll); `d` is the
/// DATA sub-mode when the client is in K31 meta mode, else `0`. Needs the frequency and mode;
/// flags not yet reported read as off, and an unknown RIT/XIT offset as zero.
pub fn if_(s: &RadioState, k31: bool) -> Option<String> {
    let hz = s.vfo_a_hz?;
    let mode = s.mode_a?;
    let off = s.rit_offset.unwrap_or(0);
    let flag = |b: Option<bool>| if b.unwrap_or(false) { '1' } else { '0' };
    let d = if k31 {
        s.data_submode.unwrap_or(0).min(9)
    } else {
        0
    };
    Some(format!(
        "IF{hz:011}     {}{:04}{}{} 00{}{}0{}{}0{}1 ;",
        if off < 0 { '-' } else { '+' },
        off.unsigned_abs().min(9999),
        flag(s.rit_on),
        flag(s.xit_on),
        flag(s.transmitting),
        mode.md_digit() as char,
        flag(s.scanning),
        flag(s.split),
        d,
    ))
}
