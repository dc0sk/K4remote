//! The network-neutral spot model and the interface every spotting network
//! implements (FR-SPOT-06).
//!
//! A spot's text comes off a network, so it is **untrusted**: it is validated
//! here, before it can reach the store or the screen. The rule throughout is
//! *reject, don't repair* — a callsign that is not a callsign is refused rather
//! than trimmed into a different one.

/// The spotting networks. Adding a network adds a variant here and a
/// [`SpotSource`] impl; the store and the overlay do not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Network {
    PskReporter,
    Rbn,
    DxCluster,
}

/// One report of a station on a frequency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    /// The station reported, normalised by [`normalise_callsign`].
    pub call: String,
    /// Frequency, Hz. Never 0.
    pub freq_hz: u64,
    /// Mode as the network names it (`"CW"`, `"FT8"`, …), if it says.
    pub mode: Option<String>,
    /// When the report was made, Unix seconds.
    pub time: u64,
    /// Which network it came from.
    pub network: Network,
    /// Signal-to-noise as reported, dB.
    pub snr_db: Option<i16>,
    /// The station that heard it (a skimmer, a receiver, a person).
    pub spotter: Option<String>,
    /// Free text the network attached.
    pub comment: Option<String>,
}

impl Spot {
    /// Build a spot from the fields every network has, or `None` if the callsign
    /// is not acceptable or the frequency is 0. The optional fields are added
    /// with the `with_*` methods.
    pub fn new(call: &str, freq_hz: u64, time: u64, network: Network) -> Option<Self> {
        if freq_hz == 0 {
            return None;
        }
        Some(Self {
            call: normalise_callsign(call)?,
            freq_hz,
            mode: None,
            time,
            network,
            snr_db: None,
            spotter: None,
            comment: None,
        })
    }

    /// Set the mode. Text that fails [`sanitise_text`] is **dropped**: a bad
    /// comment must not cost the operator a good callsign.
    pub fn with_mode(mut self, mode: &str) -> Self {
        self.mode = sanitise_text(mode);
        self
    }

    pub fn with_snr(mut self, snr_db: i16) -> Self {
        self.snr_db = Some(snr_db);
        self
    }

    /// Set the spotter; dropped if it fails [`sanitise_text`].
    pub fn with_spotter(mut self, spotter: &str) -> Self {
        self.spotter = sanitise_text(spotter);
        self
    }

    /// Set the comment; dropped if it fails [`sanitise_text`].
    pub fn with_comment(mut self, comment: &str) -> Self {
        self.comment = sanitise_text(comment);
        self
    }
}

/// Longest callsign accepted, in characters. Portable prefixes and suffixes
/// (`DL/DC0SK/P`) fit well inside it.
const MAX_CALL_LEN: usize = 16;
/// Shortest callsign accepted.
const MIN_CALL_LEN: usize = 3;

/// Normalise a callsign from a network: trimmed at the ends and upper-cased, or
/// `None`.
///
/// Accepted: ASCII letters and digits with `/` between them, 3–16 characters,
/// at least one letter **and** one digit (which every real callsign has and
/// which rejects `CQ`, `73`, `TEST`). Anything else — an inner space, a control
/// character, a bidi override, a non-ASCII look-alike, a leading, trailing or
/// doubled `/` — is rejected, never repaired.
pub fn normalise_callsign(raw: &str) -> Option<String> {
    let call = raw.trim().to_ascii_uppercase();
    let len = call.chars().count();
    if !(MIN_CALL_LEN..=MAX_CALL_LEN).contains(&len) {
        return None;
    }
    if !call.chars().all(|c| c.is_ascii_alphanumeric() || c == '/') {
        return None;
    }
    if call.starts_with('/') || call.ends_with('/') || call.contains("//") {
        return None;
    }
    let has_letter = call.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = call.chars().any(|c| c.is_ascii_digit());
    (has_letter && has_digit).then_some(call)
}

/// Longest free-text field kept (mode, spotter, comment), in characters.
pub const MAX_TEXT_LEN: usize = 64;

/// Accept a free-text field from a network: printable ASCII only (space through
/// `~`), trimmed, non-empty and at most [`MAX_TEXT_LEN`] characters — else `None`.
/// Over-long text is rejected, not cut: a cut comment can read as something the
/// spotter never wrote.
pub fn sanitise_text(raw: &str) -> Option<String> {
    let text = raw.trim();
    if text.is_empty() || text.chars().count() > MAX_TEXT_LEN {
        return None;
    }
    text.chars()
        .all(|c| (' '..='~').contains(&c))
        .then(|| text.to_string())
}

/// Why a source could not deliver — surfaced per network in Settings, never
/// swallowed (FR-SPOT-09).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError(pub String);

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SourceError {}

/// What every spotting network implements. A source hands the spots it has
/// obtained to `sink` and reports failure as a [`SourceError`]; it knows nothing
/// of the store or the overlay, and they know nothing of it.
///
/// One `poll` means "deliver what has arrived": a polled network (PSK Reporter)
/// makes a request; a streamed one (RBN, a DX cluster) drains its socket. The
/// shape is revisited when the first real sources land (FR-SPOT-05/-07).
pub trait SpotSource {
    /// Which network this is.
    fn network(&self) -> Network;

    /// Deliver every spot obtained since the last call to `sink`.
    fn poll(&mut self, sink: &mut dyn FnMut(Spot)) -> Result<(), SourceError>;
}
