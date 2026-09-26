//! CAT server **core** for third-party software (FR-CATSRV): per-client state, the command
//! policy, and replies from the cached radio state. Pure — no sockets, no threads; the listener
//! and the worker wiring live elsewhere and act on the [`Action`]s returned by [`handle`].
//!
//! Logging, contest and digital-mode software (WSJT-X/JTDX via Hamlib, fldigi/flrig, Log4OM,
//! CQRLOG, N1MM Logger+) talks to the app as if it were a K4's own raw-CAT TCP service. The
//! design, its sources and an adversarial review are in `docs/concept/cat-server-plan.md` v0.2
//! (§0, §0.1). The rules this core enforces, in order:
//!
//! 1. **Per-client meta mode and AI** (`K2n`/`K3n`/`K4n`, `AIn`) are answered and changed per
//!    client, never forwarded: our own link to the radio runs in K41, while Hamlib uses K40/K22
//!    and flrig K41, and a K41 reply handed to a K40 client breaks it (`ID`, `IF`, …).
//! 2. **The stop direction is never refused:** `RX` unkeys through the session (so the app does
//!    not keep believing it transmits); `KY @`, `KY |`, `TU0`, `PB0`, `DA0` are forwarded.
//! 3. **No client command keys the transmitter** in this phase: anything that keys or can key —
//!    including every `SW` code, since a deny-list cannot be complete (`SW17` is both KEYPAD 1
//!    and "play M1") — is refused. Keying behind arm + opt-in is a later phase.
//! 4. **GETs are answered locally or from the cache, in the forms real clients check** (Hamlib
//!    fails its open on any missing or wrong-length reply, with no retries). A GET the cache
//!    cannot answer gets **no** reply — never a K4 `<cmd>?;`, which Hamlib mis-parses.
//! 5. **SETs are forwarded only from an allowlist** (frequency, mode, bandwidth, DATA sub-mode,
//!    split, RIT/XIT); everything else — session-owned (`RR*`, `PS`, `EM`, `SL`, `ER`, `RDY`),
//!    hazardous (`EC`, `LB`) or merely unknown — is dropped with a reason, never forwarded.
//! 6. **While the radio link is down** GETs still come from the last cache (a short outage must
//!    not throw a logger into its error dialog), `TQ` reads 0, `PS` gets no reply, and SETs are
//!    dropped rather than queued.

use k4_protocol::cat::keys_transmitter;
use k4_protocol::cat_resp as resp;
use k4_protocol::state::RadioState;

/// One client's own state: its meta modes and Auto-Info level. A fresh client is a legacy one —
/// K20, K30, K40, AI0 — as on a new connection to the radio.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Client {
    k2: u8,
    k3: u8,
    k4: u8,
    ai: u8,
}

impl Client {
    pub fn new() -> Self {
        Self::default()
    }

    /// The client's Auto-Info level (0 = none).
    pub fn ai(&self) -> u8 {
        self.ai
    }

    /// The client is in K31 meta mode (the K3 extended responses, e.g. `IF`'s DATA sub-mode).
    pub fn k31(&self) -> bool {
        self.k3 == 1
    }

    /// The client is in K41 meta mode (the K4 advanced responses, e.g. `ID`'s text form).
    pub fn k41(&self) -> bool {
        self.k4 == 1
    }
}

/// What the core answers from: the cached radio state and the replies the app fetched from the
/// radio once per connect (`OM`, `RVM`, `RVD`, and `ID`'s text), each without its `;`.
#[derive(Debug, Clone, Copy)]
pub struct Cache<'a> {
    pub state: &'a RadioState,
    pub om: Option<&'a str>,
    pub rvm: Option<&'a str>,
    pub rvd: Option<&'a str>,
    /// The radio's ID text (the K41 form of `ID`), e.g. a callsign; `None` = the default "0".
    pub id_text: Option<&'a str>,
    /// The app's link to the radio is up.
    pub link_up: bool,
}

/// What to do for one client command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Send this line back to the client.
    Reply(String),
    /// Send this command to the radio (an allowlisted SET), `;` included.
    Forward(String),
    /// Send this stop command to the radio, whatever the arm state, `;` included.
    Stop(String),
    /// Unkey through the session (`end_tx`), for a client's `RX`.
    Unkey,
    /// A keying command, refused (logged; the client gets no reply, as for any SET).
    Refused(String),
    /// Not acted on, with the reason (logged).
    Dropped(String, &'static str),
}

/// The SETs a client may send to the radio (FR-CATSRV-05): the ones a logger or digital-mode
/// program needs to follow and set frequency and mode, none of which keys the transmitter.
pub const ALLOWED_SETS: &[&str] = &[
    "FA", "FB", "MD", "MD$", "BW", "BW$", "DT", "DT$", "FT", "FR", "RT", "XT", "RC",
];

/// Stop commands (besides `RX`): forwarded whatever the arm state (FR-CATSRV-07).
const STOPS: &[&str] = &["TU0", "PB0", "DA0", "KY @", "KY |"];

/// Session-owned or hazardous command families, dropped with their own reason (FR-CATSRV-06).
const BLOCKED: &[&str] = &["RR", "PS", "EM", "SL", "ER", "RD", "EC", "LB"];

/// Handle one client command (with or without its `;`), updating the client's own state, and say
/// what should happen. See the module documentation for the rules, in order.
pub fn handle(client: &mut Client, raw: &str, cache: &Cache) -> Vec<Action> {
    let cmd = raw.trim().trim_end_matches(';').trim_end();
    let wire = format!("{cmd};");
    let drop = |why: &'static str| vec![Action::Dropped(wire.clone(), why)];
    if cmd.len() < 2
        || !cmd.is_ascii()
        || cmd.chars().any(|c| c.is_ascii_control())
        || !cmd.as_bytes()[0].is_ascii_alphabetic()
    {
        return drop("not a command");
    }
    let up = cmd.to_ascii_uppercase();

    // 1. Meta modes: `K2n` (0–3), `K3n`, `K4n` (0/1), per client.
    if let [b'K', m @ b'2'..=b'4', rest @ ..] = up.as_bytes() {
        let arg = std::str::from_utf8(rest).unwrap_or("");
        let slot = match m {
            b'2' => client.k2,
            b'3' => client.k3,
            _ => client.k4,
        };
        if arg.is_empty() {
            return vec![Action::Reply(format!("K{}{slot};", *m as char))];
        }
        let max = if *m == b'2' { 3 } else { 1 };
        return match arg.parse::<u8>() {
            Ok(n) if n <= max => {
                match m {
                    b'2' => client.k2 = n,
                    b'3' => client.k3 = n,
                    _ => {
                        // PRG: "K4n; SET command turns off K2; meta-mode and changes K3
                        // meta-mode". Taken as K3 following K4 — to confirm by capture.
                        client.k4 = n;
                        client.k2 = 0;
                        client.k3 = n;
                    }
                }
                Vec::new()
            }
            _ => drop("meta mode out of range"),
        };
    }

    let head = &up[..2];
    let (mnemonic, arg) = match up.as_bytes().get(2) {
        Some(b'$') => (&up[..3], &up[3..]),
        _ => (head, &up[2..]),
    };

    // 2. The stop direction.
    if up == "RX" {
        return vec![Action::Unkey];
    }
    if STOPS.contains(&up.as_str()) {
        return vec![Action::Stop(wire)];
    }

    // 3. Keying — refused in this phase, however it is spelled.
    let active = !arg.is_empty() && arg != "0";
    let keys = keys_transmitter(&up)
        || head == "SW"
        || (head == "TX" && arg.is_empty())
        || (matches!(head, "KY" | "KZ") && !arg.is_empty())
        || (matches!(head, "TS" | "TU" | "PB" | "DA" | "VX") && active);
    if keys {
        return vec![Action::Refused(wire)];
    }

    // 4. GETs: answered locally or from the cache, or not at all.
    let is_get = arg.is_empty() && mnemonic != "RC" || (head == "RV" && matches!(arg, "M" | "D"));
    if is_get {
        let s = cache.state;
        let local = match mnemonic {
            "PS" => cache.link_up.then(|| "PS1;".to_string()),
            "ID" => Some(if client.k41() {
                format!("ID{};", cache.id_text.unwrap_or("0"))
            } else {
                "ID017;".to_string()
            }),
            "AI" => Some(format!("AI{};", client.ai)),
            "OM" => cache.om.map(|o| format!("{o};")),
            "RV" if arg == "M" => cache.rvm.map(|r| format!("{r};")),
            "RV" if arg == "D" => cache.rvd.map(|r| format!("{r};")),
            "FA" => resp::fa(s),
            "FB" => resp::fb(s),
            "MD" => resp::md(s, false),
            "MD$" => resp::md(s, true),
            "BW" => resp::bw(s, false),
            "BW$" => resp::bw(s, true),
            "DT" => resp::dt(s, false),
            "DT$" => resp::dt(s, true),
            "FT" => resp::ft(s),
            "FR" => Some(resp::fr().to_string()),
            "TQ" if !cache.link_up => Some("TQ0;".to_string()),
            "TQ" => resp::tq(s),
            "IF" => resp::if_(s, client.k31()),
            _ => return drop("GET not answered by the server"),
        };
        return local.map(Action::Reply).into_iter().collect();
    }

    // 5. SETs. AI is the client's own; the rest go to the radio only from the allowlist.
    if mnemonic == "AI" {
        return match arg.parse::<u8>() {
            Ok(n @ (0 | 1 | 2 | 4 | 5)) => {
                client.ai = n;
                Vec::new()
            }
            _ => drop("AI level the radio does not have"),
        };
    }
    if BLOCKED.contains(&head) {
        return drop("session-owned or hazardous");
    }
    if !cache.link_up {
        return drop("radio link down");
    }
    if ALLOWED_SETS.contains(&mnemonic) {
        return vec![Action::Forward(wire)];
    }
    drop("SET not on the allowlist")
}
