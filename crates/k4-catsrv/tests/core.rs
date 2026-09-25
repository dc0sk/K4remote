//! The CAT server core against the command sequences real clients send (FR-CATSRV-02/03/05/06/07).
//!
//! The Hamlib and flrig sequences and the reply checks below were read from their source (Hamlib
//! `rigs/kenwood/{k3,elecraft,kenwood}.c`, flrig `src/rigs/elecraft/K4.cxx`, 2026-09-25; see
//! `docs/concept/cat-server-plan.md` §0/§0.1): Hamlib fails its open on any missing or
//! wrong-length reply, so each reply here is held to the length Hamlib checks.
//! trace: FR-CATSRV-02, FR-CATSRV-03, FR-CATSRV-05, FR-CATSRV-06, FR-CATSRV-07

use k4_catsrv::{handle, Action, Cache, Client};
use k4_protocol::state::{Mode, RadioState};

fn state() -> RadioState {
    RadioState {
        vfo_a_hz: Some(14_074_000),
        vfo_b_hz: Some(14_076_500),
        mode_a: Some(Mode::Data),
        mode_b: Some(Mode::Usb),
        split: Some(false),
        scanning: Some(false),
        transmitting: Some(false),
        bandwidth_hz: Some(2_800),
        sub_bandwidth_hz: Some(2_400),
        data_submode: Some(0),
        sub_data_submode: Some(0),
        rit_offset: Some(0),
        rit_on: Some(false),
        xit_on: Some(false),
        ..RadioState::default()
    }
}

fn cache(s: &RadioState) -> Cache<'_> {
    Cache {
        state: s,
        om: Some("OM AP-S----4---"),
        rvm: Some("RVM01.23"),
        rvd: Some("RVD02.34"),
        id_text: Some("DC0SK"),
        link_up: true,
    }
}

/// The single reply a command gets, or `None` if it gets none.
fn reply(c: &mut Client, cmd: &str, k: &Cache) -> Option<String> {
    let acts = handle(c, cmd, k);
    let replies: Vec<String> = acts
        .iter()
        .filter_map(|a| match a {
            Action::Reply(r) => Some(r.clone()),
            _ => None,
        })
        .collect();
    assert!(
        replies.len() <= 1,
        "{cmd}: more than one reply: {replies:?}"
    );
    replies.into_iter().next()
}

/// Hamlib's K4 open, command by command, each reply held to the length and form Hamlib checks
/// (lengths exclude the `;`): `PS1`; `ID…` ≥ 5 (`ID017`); `K2n`/`K3n` 3; `OM…` exactly 15;
/// `RVM`/`RVD` present (RVM is fatal if missing); `AIn` 3; `FR/FT/TQ` exactly 3; `FA/FB` 13;
/// `IF` exactly 37; `MD` 3; `BW` 6; `DT` 3 (asked because the mode is DATA). Nothing is sent
/// to the radio for any of it.
/// trace: FR-CATSRV-03, FR-CATSRV-06
#[test]
fn fr_catsrv_06_hamlibs_k4_open_is_answered_locally_in_the_forms_it_checks() {
    let s = state();
    let k = cache(&s);
    let mut c = Client::new();
    let script: &[(&str, Option<usize>, &str)] = &[
        ("PS;", Some(3), "PS1;"),
        ("K40;", None, ""),
        ("ID;", Some(5), "ID017;"),
        ("K2;", Some(3), "K20;"),
        ("K2;", Some(3), "K20;"),
        ("K22;", None, ""),
        ("ID;", Some(5), "ID017;"),
        ("OM;", Some(15), "OM AP-S----4---;"),
        ("K3;", Some(3), "K30;"),
        ("RVM;", None, "RVM01.23;"),
        ("RVD;", None, "RVD02.34;"),
        ("AI;", Some(3), "AI0;"),
        ("AI0;", None, ""),
        ("FR;", Some(3), "FR0;"),
        ("FT;", Some(3), "FT0;"),
        ("TQ;", Some(3), "TQ0;"),
        ("FA;", Some(13), "FA00014074000;"),
        ("FB;", Some(13), "FB00014076500;"),
        ("IF;", Some(37), "IF00014074000     +000000 0006000001 ;"),
        ("MD;", Some(3), "MD6;"),
        ("BW;", Some(6), "BW0280;"),
        ("DT;", Some(3), "DT0;"),
        ("BW$;", Some(7), "BW$0240;"),
    ];
    for (cmd, len, want) in script {
        let acts = handle(&mut c, cmd, &k);
        assert!(
            !acts
                .iter()
                .any(|a| matches!(a, Action::Forward(_) | Action::Stop(_))),
            "{cmd} reached the radio: {acts:?}"
        );
        let got = reply(&mut Client::clone(&c), cmd, &k);
        if want.is_empty() {
            assert_eq!(got, None, "{cmd} is a SET and gets no reply");
        } else {
            assert_eq!(got.as_deref(), Some(*want), "{cmd}");
            if let Some(n) = len {
                assert_eq!(
                    want.trim_end_matches(';').len(),
                    *n,
                    "{cmd} length Hamlib checks"
                );
            }
        }
    }
}

/// flrig runs in K41: `ID` then gives the radio's ID text, `K4;` reads 1, and the K41 SET turns
/// off K2 meta-mode (PRG). Meta mode and AI are per client: the flrig client's K41 does not change
/// a second client's K40 replies.
/// trace: FR-CATSRV-02, FR-CATSRV-06
#[test]
fn fr_catsrv_02_meta_mode_and_ai_are_per_client() {
    let s = state();
    let k = cache(&s);
    let mut flrig = Client::new();
    let mut hamlib = Client::new();
    for cmd in ["K22;", "AI2;"] {
        handle(&mut flrig, cmd, &k);
    }
    for cmd in ["AI0;", "K41;"] {
        handle(&mut flrig, cmd, &k);
    }
    assert_eq!(reply(&mut flrig, "K4;", &k).as_deref(), Some("K41;"));
    assert_eq!(
        reply(&mut flrig, "K2;", &k).as_deref(),
        Some("K20;"),
        "K4n turns off K2"
    );
    assert_eq!(reply(&mut flrig, "ID;", &k).as_deref(), Some("IDDC0SK;"));
    assert_eq!(reply(&mut flrig, "AI;", &k).as_deref(), Some("AI0;"));
    assert_eq!(reply(&mut hamlib, "K4;", &k).as_deref(), Some("K40;"));
    assert_eq!(reply(&mut hamlib, "ID;", &k).as_deref(), Some("ID017;"));
    // K41 with no ID text known: the radio's default "0".
    let unknown = Cache {
        id_text: None,
        ..cache(&s)
    };
    assert_eq!(reply(&mut flrig, "ID;", &unknown).as_deref(), Some("ID0;"));
    // An AI level the radio does not have (3 is reserved) is not taken.
    handle(&mut hamlib, "AI3;", &k);
    assert_eq!(reply(&mut hamlib, "AI;", &k).as_deref(), Some("AI0;"));
    // Commands are case-insensitive (PRG), so `k41;` is K41.
    handle(&mut hamlib, "k41;", &k);
    assert_eq!(reply(&mut hamlib, "K4;", &k).as_deref(), Some("K41;"));
}

/// The frequency, mode, bandwidth, DATA sub-mode, split and RIT/XIT SETs a logger sends are
/// forwarded to the radio unchanged (FR-CATSRV-05) — and only those: they are the allowlist.
/// trace: FR-CATSRV-05
#[test]
fn fr_catsrv_05_allowlisted_sets_are_forwarded_verbatim() {
    let s = state();
    let k = cache(&s);
    let mut c = Client::new();
    for cmd in [
        "FA00014075000",
        "FB00007074000",
        "MD2",
        "MD$6",
        "BW0300",
        "BW$0240",
        "DT0",
        "DT$1",
        "FT1",
        "FT0",
        "FR0",
        "RT1",
        "XT0",
        "RC",
        "fa00014075000",
    ] {
        let acts = handle(&mut c, &format!("{cmd};"), &k);
        assert_eq!(acts, vec![Action::Forward(format!("{cmd};"))], "{cmd}");
    }
}

/// Transmit safety in phase A (FR-CATSRV-07): **no client command keys the transmitter**. Every
/// keying form the review found — including the ones a deny-list misses (`SW17` is both KEYPAD 1
/// and "play M1"; `SWT16` is Hamlib's tuner; `TS1` keys downstream gear; `KY` text; `<` in `KY`
/// is TX TEST) — is refused, never forwarded. The stop direction is never refused: `RX` unkeys
/// through the session, and `KY @`, `KY |`, `TU0`, `PB0`, `DA0` are forwarded as stops. `KY0` is
/// *not* a stop (the PRG's `KY*[text]` sends "0").
/// trace: FR-CATSRV-07
#[test]
fn fr_catsrv_07_no_client_command_keys_in_phase_a_and_stops_always_pass() {
    let s = state();
    let k = cache(&s);
    let mut c = Client::new();
    for cmd in [
        "TX",
        "tx",
        "KY CQ TEST",
        "KY0",
        "KYR CQ",
        "KY <",
        "KZ1",
        "SW17",
        "SW51",
        "SW18",
        "SW52",
        "SW162",
        "SW50",
        "SW132",
        "SW40",
        "SWT16",
        "SWH11",
        "TS1",
        "TU1",
        "TU2",
        "PB1",
        "PB8",
        "DAPM",
        "DAMP100000",
        "DA1",
        "VX1",
    ] {
        let acts = handle(&mut c, &format!("{cmd};"), &k);
        assert!(
            !acts
                .iter()
                .any(|a| matches!(a, Action::Forward(_) | Action::Stop(_) | Action::Unkey)),
            "{cmd} reached the radio: {acts:?}"
        );
        assert!(
            acts.iter()
                .any(|a| matches!(a, Action::Refused(_) | Action::Dropped(..))),
            "{cmd} was neither refused nor dropped: {acts:?}"
        );
    }
    assert_eq!(handle(&mut c, "RX;", &k), vec![Action::Unkey]);
    assert_eq!(handle(&mut c, "rx;", &k), vec![Action::Unkey]);
    for stop in ["KY @", "KY |", "TU0", "PB0", "DA0"] {
        assert_eq!(
            handle(&mut c, &format!("{stop};"), &k),
            vec![Action::Stop(format!("{stop};"))],
            "{stop}"
        );
    }
}

/// Session-owned, hazardous and unknown commands never reach the radio (FR-CATSRV-06): the
/// `RR*` family (e.g. `RRC0` disables remote connections, `RRP…` changes the password), `EC`,
/// `LB1`, `PS0`/`PS8`, `EM…`, `SL…`, `ER1`, `RDY`, and anything not on the allowlist — they are
/// dropped with a reason, never forwarded. A GET the cache cannot answer gets no reply (never a
/// K4 `<cmd>?;`, which Hamlib mis-parses).
/// trace: FR-CATSRV-06
#[test]
fn fr_catsrv_06_hazardous_and_unknown_commands_never_reach_the_radio() {
    let s = state();
    let k = cache(&s);
    let mut c = Client::new();
    for cmd in [
        "RRC0",
        "RRP secret",
        "RRN",
        "RRT",
        "EC1",
        "LB1",
        "PS0",
        "PS8",
        "EM3",
        "SL2",
        "ER1",
        "RDY",
        "AG30",
        "PC050H",
        "XY123",
        "ZZ",
        "SM",
        "KY",
    ] {
        let acts = handle(&mut c, &format!("{cmd};"), &k);
        assert!(
            !acts
                .iter()
                .any(|a| matches!(a, Action::Forward(_) | Action::Stop(_) | Action::Unkey)),
            "{cmd} reached the radio: {acts:?}"
        );
        assert!(
            !acts
                .iter()
                .any(|a| matches!(a, Action::Reply(r) if r.contains('?'))),
            "{cmd} got an error-form reply: {acts:?}"
        );
    }
}

/// Every mnemonic the Programmer's Reference defines, with a few argument shapes: the only ones
/// that ever reach the radio are the allowlist, the stops and `RX` — a sweep from the document,
/// not a hand list, so a command the policy forgot is dropped by default rather than forwarded.
/// trace: FR-CATSRV-05, FR-CATSRV-06, FR-CATSRV-07
#[test]
fn fr_catsrv_06_only_the_allowlist_reaches_the_radio_over_every_prg_mnemonic() {
    let prg = include_str!("../../../docs/references/external/K4ProgrammersReferencerev.D12.html");
    let text: String = {
        let mut out = String::new();
        let mut tag = false;
        for ch in prg.chars() {
            match ch {
                '<' => tag = true,
                '>' => {
                    tag = false;
                    out.push(' ');
                }
                c if !tag => out.push(c),
                _ => {}
            }
        }
        out.replace("&nbsp;", " ").replace("&amp;", "&")
    };
    // A command heading: 2–3 capitals (plus an optional `$`), then " (".
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut mnemonics: Vec<String> = Vec::new();
    for w in words.windows(2) {
        let (m, next) = (w[0], w[1]);
        let core = m.trim_end_matches('$');
        if next.starts_with('(')
            && (2..=3).contains(&core.len())
            && core
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && core.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && !mnemonics.iter().any(|x| x == m)
        {
            mnemonics.push(m.to_string());
        }
    }
    assert!(
        mnemonics.len() > 100,
        "the sweep found only {} mnemonics",
        mnemonics.len()
    );
    for must in [
        "FA", "TX", "SW", "KY", "RRC", "RRP", "TS", "PB", "DA", "AI", "PS",
    ] {
        assert!(
            mnemonics.iter().any(|m| m == must),
            "{must} not found in the PRG sweep"
        );
    }

    let s = state();
    let k = cache(&s);
    let allowed = [
        "FA", "FB", "MD", "MD$", "BW", "BW$", "DT", "DT$", "FT", "FR", "RT", "XT", "RC",
    ];
    for m in &mnemonics {
        for arg in ["", "0", "1", "2", "5", "00014074000", "T16", " TEXT"] {
            let cmd = format!("{m}{arg};");
            let mut c = Client::new();
            for a in handle(&mut c, &cmd, &k) {
                match a {
                    Action::Forward(_) => {
                        assert!(allowed.contains(&m.as_str()), "{cmd} was forwarded")
                    }
                    Action::Stop(ref x) => {
                        assert!(
                            ["TU0;", "PB0;", "DA0;"].contains(&x.as_str()),
                            "{cmd} was forwarded as a stop"
                        )
                    }
                    Action::Unkey => assert_eq!(cmd, "RX;", "{cmd} unkeyed"),
                    _ => {}
                }
            }
        }
    }
}

/// While the radio link is down (the grace period), GETs are still answered from the last cache
/// — so a logger is not thrown into an error by a short outage — but `TQ` reads 0, `PS` gets no
/// reply, and SETs are dropped rather than queued for a link that is not there.
/// trace: FR-CATSRV-08
#[test]
fn fr_catsrv_08_link_down_answers_from_cache_and_drops_sets() {
    let s = RadioState {
        transmitting: Some(true),
        ..state()
    };
    let down = Cache {
        link_up: false,
        ..cache(&s)
    };
    let mut c = Client::new();
    assert_eq!(
        reply(&mut c, "FA;", &down).as_deref(),
        Some("FA00014074000;")
    );
    assert_eq!(reply(&mut c, "TQ;", &down).as_deref(), Some("TQ0;"));
    assert_eq!(reply(&mut c, "PS;", &down), None);
    let acts = handle(&mut c, "FA00014075000;", &down);
    assert!(
        acts.iter().all(|a| matches!(a, Action::Dropped(..))),
        "{acts:?}"
    );
}

/// A client line holding several commands is handled command by command, and a malformed
/// fragment costs only itself (FR-CATSRV-02). `handle` takes one command; the decoder splits.
/// trace: FR-CATSRV-02
#[test]
fn fr_catsrv_02_empty_and_junk_commands_are_harmless() {
    let s = state();
    let k = cache(&s);
    let mut c = Client::new();
    for junk in ["", ";", "   ;", "\u{7f}\u{0}", "$$$;", "F", "9"] {
        let acts = handle(&mut c, junk, &k);
        assert!(
            !acts.iter().any(|a| matches!(
                a,
                Action::Forward(_) | Action::Stop(_) | Action::Unkey | Action::Reply(_)
            )),
            "{junk:?}: {acts:?}"
        );
    }
}
