//! R6 — the **reachability ratchet** (NFR-TEST-01): every public function in `crates/*/src` and
//! `app/src` must be referenced from production code somewhere other than its own definition.
//!
//! Why: coverage and traceability both certify test-only code — a function exercised only by tests
//! reads as covered, and a requirement citing it reads as evidenced. `KZF` (#222), `KZL` and
//! `send_cw` (#227) were all "covered" while nothing in the product ever called them. R5 watched
//! only one module's CAT encoders; this watches every public function.
//!
//! A heuristic, hence a ratchet: names are matched as words across all production code (test code
//! removed per item, comments and strings masked — `regions`), so a common name (`new`) is never
//! flagged and dynamic dispatch is invisible. Today's unreferenced set lives in a baseline file,
//! each entry with a reason; a **new** unreferenced function fails the build, and so does a
//! **stale** baseline entry (now referenced, or gone), so the list can only shrink.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::regions;

/// Where the baseline lives, relative to the workspace root.
pub const BASELINE: &str = "docs/test/r6-reachability-baseline.md";

/// The source trees scanned (production code only; `tests/` directories are skipped).
const TREES: &[&str] = &["crates", "app"];

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if path.is_dir() {
            if name != "tests" && name != "target" && name != "benches" && name != "examples" {
                rust_sources(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Production code with test regions removed and comments/strings blanked to spaces.
fn production_code(text: &str) -> String {
    let live = regions::without_tests(text);
    let mask = regions::code_mask(&live);
    live.char_indices()
        .map(|(i, c)| if mask[i] || c == '\n' { c } else { ' ' })
        .collect()
}

/// The name of a public function defined on this (masked) line, if any: `pub`, an optional
/// visibility scope, optional `const`/`async`/`unsafe`/`extern`, then `fn <name>`.
fn pub_fn_name(line: &str) -> Option<&str> {
    let mut rest = line.trim_start().strip_prefix("pub")?;
    if let Some(r) = rest.strip_prefix('(') {
        rest = &r[r.find(')')? + 1..];
    } else if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    rest = rest.trim_start();
    loop {
        let before = rest;
        for kw in ["const ", "async ", "unsafe ", "extern "] {
            if let Some(r) = rest.strip_prefix(kw) {
                rest = r.trim_start();
            }
        }
        if rest == before {
            break;
        }
    }
    let rest = rest.strip_prefix("fn ")?.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// How many times `name` occurs as a whole word in `code`.
fn word_count(code: &str, name: &str) -> usize {
    code.match_indices(name)
        .filter(|(i, _)| {
            let before = code[..*i].chars().next_back();
            let after = code[i + name.len()..].chars().next();
            !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
        })
        .count()
}

/// Every public function (as `path:name`, path relative to `root`) whose name occurs nowhere in
/// production code but its own definition.
pub fn unreferenced(root: &Path) -> BTreeSet<String> {
    let mut files = Vec::new();
    for tree in TREES {
        rust_sources(&root.join(tree), &mut files);
    }
    let code: Vec<(String, String)> = files
        .iter()
        .filter_map(|p| {
            let text = fs::read_to_string(p).ok()?;
            let rel = p
                .strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            Some((rel, production_code(&text)))
        })
        .collect();
    let all: String = code
        .iter()
        .map(|(_, c)| c.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut out = BTreeSet::new();
    for (rel, text) in &code {
        for line in text.lines() {
            if let Some(name) = pub_fn_name(line) {
                if word_count(&all, name) <= 1 {
                    out.insert(format!("{rel}:{name}"));
                }
            }
        }
    }
    out
}

/// The baseline's entries: each list line `- `path:name` — reason`.
pub fn load_baseline(root: &Path) -> BTreeSet<String> {
    let Ok(text) = fs::read_to_string(root.join(BASELINE)) else {
        return BTreeSet::new();
    };
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix("- `"))
        .filter_map(|l| l.split('`').next())
        .map(str::to_string)
        .collect()
}

/// The ratchet's verdict: unreferenced functions not in the baseline (new), and baseline entries
/// that are no longer unreferenced (stale — referenced now, or gone).
pub fn check(root: &Path) -> (Vec<String>, Vec<String>, usize) {
    let found = unreferenced(root);
    let base = load_baseline(root);
    let new = found.difference(&base).cloned().collect();
    let stale = base.difference(&found).cloned().collect();
    (new, stale, found.intersection(&base).count())
}

#[cfg(test)]
mod tests {
    use super::{check, pub_fn_name, word_count, BASELINE};
    use std::fs;

    /// NFR-TEST-01: public function definitions are recognised in their forms, private ones not.
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_pub_fn_definitions_are_recognised() {
        for (line, want) in [
            ("pub fn a() {}", Some("a")),
            ("    pub(crate) fn b_2(x: u8) -> u8 {", Some("b_2")),
            ("pub const fn c() {}", Some("c")),
            ("pub async unsafe fn d() {}", Some("d")),
            ("pub(super) extern fn e() {}", Some("e")),
            ("fn private() {}", None),
            ("pub struct S;", None),
            ("pub fnord: u8,", None),
            ("publish(x);", None),
        ] {
            assert_eq!(pub_fn_name(line), want, "{line:?}");
        }
        assert_eq!(word_count("a(x); ab(); a_b(); a;", "a"), 2);
    }

    /// NFR-TEST-01, in a scratch workspace: a function used only by tests or named only in a
    /// comment is unreferenced; one called from production is not. The ratchet fails on a new
    /// unreferenced function and on a stale baseline entry, and passes a baselined one.
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_the_ratchet_flags_new_and_stale_entries() {
        let root = std::env::temp_dir().join(format!("xtask-reach-{}", std::process::id()));
        let src = root.join("crates/demo/src");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(root.join("docs/test")).unwrap();
        fs::write(
            src.join("lib.rs"),
            "pub fn used() {}\npub fn wired_later() {}\npub fn orphan() {}\npub fn baselined() {}\n\
             /// see orphan() and wired_later()\npub fn caller() { used(); wired_later(); }\n\
             fn main_path() { caller(); }\n\
             #[cfg(test)]\nmod tests { #[test] fn t() { super::orphan(); super::baselined(); } }\n",
        )
        .unwrap();
        fs::write(
            root.join(BASELINE),
            "# baseline\n- `crates/demo/src/lib.rs:baselined` — kept for a reason\n\
             - `crates/demo/src/lib.rs:wired_later` — was unreferenced, now called\n",
        )
        .unwrap();
        let (new, stale, kept) = check(&root);
        let _ = fs::remove_dir_all(&root);
        assert_eq!(
            new,
            vec!["crates/demo/src/lib.rs:orphan".to_string()],
            "new"
        );
        assert_eq!(
            stale,
            vec!["crates/demo/src/lib.rs:wired_later".to_string()],
            "stale"
        );
        assert_eq!(kept, 1, "the baselined one is accepted");
    }
}
