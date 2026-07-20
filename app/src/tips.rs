//! Control tooltips (FR-UI-TIP-01).
//!
//! Each tip says what the control does **and** names the CAT command behind it,
//! so the panel doubles as live documentation: an operator reading the
//! diagnostics console can match what they see on the wire to the control that
//! sent it.
//!
//! Tips are keyed by a stable `&'static str` id rather than by position, so a
//! control can move between screens without losing its tip, and the id is what
//! the hover-delay state tracks.

/// How long the pointer must rest on a control before its tip appears.
///
/// The UI re-renders on a 100 ms tick, so the observed delay is 500–600 ms.
pub const TOOLTIP_DELAY: std::time::Duration = std::time::Duration::from_millis(500);

/// Look up the tip for a control id. `None` means "no tip written yet", which
/// renders as no tooltip rather than an empty bubble.
///
/// trace: FR-UI-TIP-01
pub fn tip(id: &str) -> Option<&'static str> {
    TIPS.iter().find(|(k, _)| *k == id).map(|(_, v)| *v)
}

/// Coverage: 55 of these are wired to a control. The rest are not simply
/// un-wired — they fall into three groups worth distinguishing:
///
/// - **No control exists yet.** `pan.nb` describes the panadapter noise
///   blanker (`#NB`/`#NBL`); the encoders exist in `k4-protocol` but nothing
///   in the UI calls them. The tip is written and waiting.
/// - **Composite widgets.** `vfo.a`/`vfo.b` are per-digit readouts and
///   `filter.locut`/`filter.hicut` are drag handles on the passband graphic.
///   One tooltip over the whole widget would fight the per-digit click and
///   drag behaviour, so these need a deliberate choice of hover target rather
///   than a wrapper.
/// - **Switch emulation.** `tx.tune`, `tx.tunelp`, `vfo.swap`, `vfo.a2b`,
///   `vfo.b2a` and the antenna selectors are driven through the generic
///   `Message::Switch(n)` front-panel emulation, which has no per-control call
///   site to attach a tip to.
///
/// Every control tip, as `(id, text)`.
///
/// Ids are grouped by the screen they appear on. Keep them stable: they are
/// referenced from the view code and are what the hover state compares.
pub const TIPS: &[(&str, &str)] = &[
    // --- VFO / frequency -------------------------------------------------
    ("vfo.a", "VFO A frequency (FA). Click a digit to step it; the mouse wheel tunes."),
    ("vfo.b", "VFO B frequency (FB). Click a digit to step it; the mouse wheel tunes."),
    ("vfo.swap", "Swap VFO A and B (AB1). A>B and B>A copy one to the other."),
    ("vfo.a2b", "Copy VFO A to VFO B (AB0)."),
    ("vfo.b2a", "Copy VFO B to VFO A (AB2)."),
    ("vfo.split", "Split: transmit on VFO B while receiving on A (FT)."),
    ("vfo.rit", "RIT — offset the receive frequency without moving the transmit frequency (RT)."),
    ("vfo.xit", "XIT — offset the transmit frequency without moving the receive frequency (XT)."),
    ("vfo.clr", "Clear the RIT/XIT offset back to zero (RC)."),
    ("vfo.band.up", "Next band up (BN+)."),
    ("vfo.band.down", "Next band down (BN-)."),
    // --- Mode / filter ---------------------------------------------------
    ("mode.cycle", "Step through the enabled modes (MD+). Each mode has its own filter and rate settings."),
    ("mode.select", "Operating mode (MD): LSB, USB, CW, FM, AM, DATA and the reversed variants."),
    ("filter.bw", "Receive bandwidth (BW). Narrower rejects more noise; wider sounds more natural."),
    ("filter.shift", "IF centre pitch (IS) — slides the passband without changing its width."),
    ("filter.preset", "Filter preset FL1–FL3 for this mode (FP). Hold to normalise (FP~)."),
    ("filter.locut", "Passband low cut. Sent as the BW + IS pair the K4 uses."),
    ("filter.hicut", "Passband high cut. Sent as the BW + IS pair the K4 uses."),
    // --- Receive DSP -----------------------------------------------------
    ("rx.agc", "AGC speed (GT): slow for SSB, fast for CW and pile-ups."),
    ("rx.nb", "Noise blanker on/off (NB) — for pulse noise such as ignition or power-line ticks."),
    ("rx.nr", "Noise reduction on/off (NR) — DSP smoothing of broadband hiss."),
    ("rx.notch", "Manual notch (NM) — tune it onto a carrier to null it out."),
    ("rx.autonotch", "Automatic notch (NA) — tracks and nulls steady carriers by itself."),
    ("rx.apf", "Audio peaking filter (AP) — a narrow CW peak around the sidetone pitch."),
    ("rx.preamp", "Preamp (PA). Use on the quiet bands; switch off when the band is noisy."),
    ("rx.atten", "Attenuator (RA). Backs off strong signals that would overload the front end."),
    ("rx.afgain", "AF gain — receiver volume (AG). Local only; does not change the radio's own audio."),
    ("rx.rfgain", "RF gain (RG). Reduces receiver gain ahead of the AGC."),
    ("rx.squelch", "Squelch threshold (SQ). 0 leaves the receiver open."),
    ("rx.subrx", "Sub receiver on/off (SB). Needed for dual-pan and for the sub-RX mini-pan."),
    ("rx.diversity", "Diversity receive (DV) — main and sub on separate antennas, one per ear."),
    // --- Transmit --------------------------------------------------------
    ("tx.arm", "Arm transmit. Nothing can key the radio until this is on — PTT, CW keying and TUNE are all gated by it."),
    ("tx.ptt", "Key the transmitter (TX / RX). Requires TX to be armed."),
    ("tx.estop", "Emergency stop: unkey, stop any tune, and disarm immediately (RX)."),
    ("tx.power", "Transmit power (PC). H is the QRO range, L is QRP, X is milliwatts."),
    ("tx.mic", "Microphone gain (MG)."),
    ("tx.comp", "Speech compression (CP). SSB modes only."),
    ("tx.vox", "VOX — key automatically from voice (VX). Set the threshold with VOX gain."),
    ("tx.mon", "Transmit monitor level (ML) — how loudly you hear your own audio."),
    // --- ATU / tune ------------------------------------------------------
    ("atu.mode", "ATU in line or bypassed (AT). Shown only when the radio reports a tuner fitted."),
    ("atu.tune", "Run an ATU match (TU3). Tap again within 5 s for an extended search. Requires TX to be armed."),
    ("tx.tune", "Emit a steady carrier at the current power (TU1) for tuning an external amplifier or ATU. Requires TX to be armed."),
    ("tx.tunelp", "Emit a carrier at the menu-set low power (TU2). Requires TX to be armed."),
    // --- CW --------------------------------------------------------------
    ("cw.wpm", "Keyer speed in words per minute (KS)."),
    ("cw.pitch", "CW sidetone pitch (CW). Multiples of 50 Hz line up with the DSP filters."),
    ("cw.qsk", "QSK / break-in delay (SD). Full break-in lets you hear between elements."),
    ("cw.spot", "Spot — tune your signal onto the received one (SP)."),
    // --- Panadapter ------------------------------------------------------
    ("pan.span", "Displayed span (#SPN), 6–368 kHz. Narrower gives finer resolution across the same width."),
    ("pan.ref", "Reference level (#REF) — the dBm at the bottom of the spectrum scale."),
    ("pan.scale", "Vertical scale (#SCL) — how many dB the spectrum window covers."),
    ("pan.avg", "Spectrum averaging (#AVG). More averaging is smoother but slower to react."),
    ("pan.peak", "Peak hold (#PKM) — keeps the highest level seen in each bin."),
    ("pan.freeze", "Freeze the spectrum and waterfall (#FRZ)."),
    ("pan.mode", "Panadapter display mode (#DPM): VFO A, VFO B, or both."),
    ("pan.minipan", "Mini-pan — a narrow zoomed strip for fine tuning (#MP$). Needs dual-pan off when the sub RX is disabled, or the radio refuses it."),
    ("pan.wfpalette", "Waterfall colour palette (#WFC)."),
    ("pan.wfheight", "Waterfall height as a percentage of the pan area (#WFH)."),
    ("pan.nb", "Panadapter noise blanker (#NB) — cleans the display without touching receive audio."),
    // --- Antenna ---------------------------------------------------------
    ("ant.tx", "Transmit antenna (AN)."),
    ("ant.rx", "Receive antenna (AR). Can differ from the transmit antenna."),
    // --- Connection / app ------------------------------------------------
    ("conn.connect", "Connect to the radio, or disconnect if already connected."),
    ("conn.tls", "Use TLS-PSK (port 9204) instead of the plaintext port (9205)."),
    ("conn.remember", "Store the password in the operating system keychain, never in the config file."),
    ("app.settings", "Connection, audio devices, levels and application preferences."),
    ("app.about", "Version, licence, project links, and the update check."),
    ("app.diag", "Diagnostics console: every CAT command sent and received, filterable and copyable."),
    ("app.tooltips", "Show these explanatory tips when the pointer rests on a control."),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every tip is present, non-trivial, and single-line.
    /// trace: FR-UI-TIP-01
    #[test]
    fn fr_ui_tip_01_tips_are_usable() {
        assert!(
            TIPS.len() >= 50,
            "expected broad coverage, got {}",
            TIPS.len()
        );
        for (id, text) in TIPS {
            assert!(!id.is_empty(), "empty id");
            assert!(
                text.len() > 15,
                "{id}: tip is too short to be worth showing: {text:?}"
            );
            assert!(!text.contains('\n'), "{id}: tips are single-line");
            assert!(
                text.ends_with('.'),
                "{id}: tips are sentences, so they end in a full stop"
            );
        }
    }

    /// Ids are unique — a duplicate would silently shadow the later tip.
    /// trace: FR-UI-TIP-01
    #[test]
    fn fr_ui_tip_01_ids_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for (id, _) in TIPS {
            assert!(seen.insert(*id), "duplicate tip id: {id}");
        }
    }

    /// A tip must not merely restate its control's label — the value is in
    /// naming the CAT command and saying what the control is *for*. Most tips
    /// therefore carry a mnemonic in parentheses.
    /// trace: FR-UI-TIP-01
    #[test]
    fn fr_ui_tip_01_tips_name_their_cat_command() {
        let with_cmd = TIPS.iter().filter(|(_, t)| t.contains('(')).count();
        assert!(
            with_cmd * 4 >= TIPS.len() * 3,
            "expected most tips to name a CAT command, got {with_cmd}/{}",
            TIPS.len()
        );
    }

    /// Lookup finds a known id and reports nothing for an unknown one, so an
    /// un-written tip renders as no tooltip rather than an empty bubble.
    /// trace: FR-UI-TIP-01
    #[test]
    fn fr_ui_tip_01_lookup() {
        assert!(tip("tx.arm").is_some());
        assert!(tip("atu.tune").unwrap().contains("TU3"));
        assert_eq!(tip("nope.not.a.control"), None);
        assert_eq!(tip(""), None);
    }

    /// The delay is the specified half second.
    /// trace: FR-UI-TIP-01
    #[test]
    fn fr_ui_tip_01_delay_is_500ms() {
        assert_eq!(TOOLTIP_DELAY, std::time::Duration::from_millis(500));
    }
}
