//! R6 — the **reachability ratchet** (NFR-TEST-01): every public function in `crates/*/src` and
//! `app/src` must be referenced from production code somewhere other than its own definition.
//!
//! Why: coverage and traceability both certify test-only code — a function exercised only by tests
//! reads as covered, and a requirement citing it reads as evidenced. `KZF` (#222), `KZL` and
//! `send_cw` (#227) were all "covered" while nothing in the product ever called them. R5 watched
//! only one module's CAT encoders; this watches every public function.
//!
//! How: a **liveness pass over a name-level call graph** of the production code (test code removed
//! per item and per out-of-line test module, comments and strings masked — `regions`). Live
//! contexts are code outside any function, `main`, a method of an `impl Trait for Type` block
//! (the framework calls it), and the strings of an attribute (`deserialize_with = "…"`); a
//! function is live when its name occurs in a live context, and its own body is then live too.
//! So a definition is not a use, a call from a dead function does not keep its callee alive (an
//! orphan chain is caught whole), recursion does not keep a uniquely named function alive (a
//! same-named call may be delegation, `fn len(&self) { self.0.len() }`, so it counts), several functions of
//! one name with no caller are all caught, and a `use` line is not a call.
//!
//! Still a heuristic, hence a ratchet: names, not types — two functions of one name share a fate
//! when either is called (a trait method and an inherent one of the same name), a field or variant
//! of a function's name keeps it alive, and functions a macro defines from a metavariable are not
//! seen. Today's unreferenced set lives in a baseline file, each entry with a reason; a **new**
//! unreferenced function fails the build, and so does a **stale** baseline entry (now referenced,
//! or gone), so the list can only shrink.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::regions;

/// Where the baseline lives, relative to the workspace root.
pub const BASELINE: &str = "docs/test/r6-reachability-baseline.md";

/// Where R7's exemptions live, relative to the workspace root.
pub const EXEMPTIONS: &str = "docs/test/r7-acceptance-exemptions.md";

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

/// The sources scanned: production files under the trees, minus the files of out-of-line test
/// modules (`#[cfg(test)] mod tests;` → `tests.rs` or `tests/mod.rs` beside the declaring module)
/// and everything below them.
fn production_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for tree in TREES {
        rust_sources(&root.join(tree), &mut files);
    }
    let mut test_dirs: Vec<PathBuf> = Vec::new();
    let mut test_files: HashSet<PathBuf> = HashSet::new();
    for f in &files {
        let Ok(text) = fs::read_to_string(f) else {
            continue;
        };
        let stem = f.file_stem().unwrap_or_default().to_string_lossy();
        let parent = f.parent().unwrap_or(root);
        // A module's children live beside it (`lib.rs`, `main.rs`, `mod.rs`) or in its own
        // directory (`foo.rs` → `foo/`).
        let dir = if ["lib", "main", "mod"].contains(&stem.as_ref()) {
            parent.to_path_buf()
        } else {
            parent.join(stem.as_ref())
        };
        for name in regions::out_of_line_test_mods(&text) {
            test_files.insert(dir.join(format!("{name}.rs")));
            test_dirs.push(dir.join(&name));
        }
    }
    files.retain(|f| !test_files.contains(f) && !test_dirs.iter().any(|d| f.starts_with(d)));
    files
}

/// Production code with test regions removed and comments/strings blanked to spaces, byte for
/// byte (a blanked multi-byte character becomes as many spaces), plus the identifiers inside the
/// strings of its attributes.
fn production_code(text: &str) -> (String, Vec<String>) {
    let live = regions::without_tests(text);
    let mask = regions::code_mask(&live);
    let code: String = live
        .char_indices()
        .flat_map(|(i, c)| {
            let keep = mask[i] || c == '\n';
            let n = if keep { 1 } else { c.len_utf8() };
            std::iter::repeat_n(if keep { c } else { ' ' }, n)
        })
        .collect();
    let b = live.as_bytes();
    let mut attr_names = Vec::new();
    let mut from = 0;
    while let Some(p) = live[from..].find("#[") {
        let start = from + p;
        from = start + 2;
        if !mask[start] {
            continue;
        }
        let (mut j, mut depth) = (start + 1, 0i32);
        while j < b.len() {
            if mask[j] {
                match b[j] {
                    b'[' => depth += 1,
                    b']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }
        let strings: String = (start..j.min(b.len()))
            .map(|k| if mask[k] { ' ' } else { b[k] as char })
            .collect();
        attr_names.extend(idents(&strings).map(|(_, w)| w.to_string()));
        from = j.max(from);
    }
    (code, attr_names)
}

fn is_ident(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The identifier tokens of `code` with their byte offsets.
fn idents(code: &str) -> impl Iterator<Item = (usize, &str)> {
    let b = code.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < b.len() {
            let c = b[i] as char;
            if c.is_ascii_alphabetic() || c == '_' {
                let start = i;
                while i < b.len() && is_ident(b[i] as char) {
                    i += 1;
                }
                return Some((start, &code[start..i]));
            }
            if c.is_ascii_digit() {
                // A number (with a suffix such as `1u8`): not an identifier.
                while i < b.len() && is_ident(b[i] as char) {
                    i += 1;
                }
                continue;
            }
            i += 1;
        }
        None
    })
}

/// The previous token before `at` in `code`, past whitespace: an identifier, or one character.
fn prev_token(code: &str, at: usize) -> Option<(usize, &str)> {
    let head = code[..at].trim_end();
    let end = head.len();
    let last = head.chars().next_back()?;
    if is_ident(last) {
        let start = head.trim_end_matches(is_ident).len();
        Some((start, &code[start..end]))
    } else {
        Some((end - last.len_utf8(), &code[end - last.len_utf8()..end]))
    }
}

/// Whether the `fn` at `at` is public: `pub` or `pub(…)` before it, past `const`, `async`,
/// `unsafe` and `extern` (an ABI string is already blank) — across lines and after attributes.
fn is_pub_fn(code: &str, at: usize) -> bool {
    let mut at = at;
    loop {
        match prev_token(code, at) {
            Some((p, "const" | "async" | "unsafe" | "extern")) => at = p,
            Some((_, "pub")) => return true,
            Some((p, ")")) => {
                // `pub(crate)`, `pub(super)`, `pub(in path)`.
                let Some(open) = code[..p].rfind('(') else {
                    return false;
                };
                return matches!(prev_token(code, open), Some((_, "pub")));
            }
            _ => return false,
        }
    }
}

/// The matching `}` of the `{` at `open` in masked code, or the end.
fn brace_end(code: &str, open: usize) -> usize {
    let mut depth = 0i32;
    for (k, c) in code[open..].bytes().enumerate() {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return open + k + 1;
                }
            }
            _ => {}
        }
    }
    code.len()
}

/// The first `{` or `;` after `from` outside parentheses and brackets: an item's body or end.
fn body_or_end(code: &str, from: usize) -> Option<(usize, u8)> {
    let mut depth = 0i32;
    for (k, c) in code[from..].bytes().enumerate() {
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'{' | b';' if depth == 0 => return Some((from + k, c)),
            _ => {}
        }
    }
    None
}

/// A function definition in the scanned code.
struct FnDef {
    file: usize,
    name: String,
    public: bool,
    /// Called from outside the code: `main`, or a method of an `impl Trait for Type` block.
    entry: bool,
}

/// Every public function (as `path:name`, path relative to `root`) that the liveness pass does not
/// reach from production code.
pub fn unreferenced(root: &Path) -> BTreeSet<String> {
    let sources: Vec<(String, String, Vec<String>)> = production_files(root)
        .iter()
        .filter_map(|p| {
            let text = fs::read_to_string(p).ok()?;
            let rel = p
                .strip_prefix(root)
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/");
            let (code, attrs) = production_code(&text);
            Some((rel, code, attrs))
        })
        .collect();

    // Pass 1: definitions, their bodies, trait-impl spans and `use` spans, per file.
    let mut defs: Vec<FnDef> = Vec::new();
    let mut owners: Vec<Vec<u32>> = Vec::new(); // per file: innermost function per byte
    let mut skip: Vec<Vec<bool>> = Vec::new(); // per file: definition names and `use` lines
    for (file, (_, code, _)) in sources.iter().enumerate() {
        let toks: Vec<(usize, &str)> = idents(code).collect();
        let mut owner = vec![u32::MAX; code.len()];
        let mut skipped = vec![false; code.len()];
        let mut trait_impls: Vec<std::ops::Range<usize>> = Vec::new();
        for (t, &(at, word)) in toks.iter().enumerate() {
            match word {
                "use" => {
                    let end = code[at..].find(';').map_or(code.len(), |e| at + e);
                    skipped[at..end].fill(true);
                }
                "impl" => {
                    if let Some((open, b'{')) = body_or_end(code, at) {
                        let header = &code[at..open];
                        let is_trait = idents(header).any(|(p, w)| {
                            w == "for" && !header[p + 3..].trim_start().starts_with('<')
                        });
                        if is_trait {
                            trait_impls.push(open..brace_end(code, open));
                        }
                    }
                }
                "fn" => {
                    let Some(&(nat, name)) = toks.get(t + 1) else {
                        continue;
                    };
                    if !code[at + 2..nat].trim().is_empty() {
                        continue; // `fn(` — a pointer type, not a definition
                    }
                    skipped[nat..nat + name.len()].fill(true);
                    let idx = defs.len() as u32;
                    defs.push(FnDef {
                        file,
                        name: name.to_string(),
                        public: is_pub_fn(code, at),
                        entry: name == "main" || trait_impls.iter().any(|r| r.contains(&at)),
                    });
                    if let Some((open, b'{')) = body_or_end(code, nat + name.len()) {
                        // Bodies nest and are met in order, so an inner one overwrites its outer.
                        owner[open..brace_end(code, open)].fill(idx);
                    }
                }
                _ => {}
            }
        }
        owners.push(owner);
        skip.push(skipped);
    }

    // Pass 2: which names each function body (and the code outside functions) refers to.
    let names: HashSet<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    let mut def_count: HashMap<&str, usize> = HashMap::new();
    for d in &defs {
        *def_count.entry(d.name.as_str()).or_default() += 1;
    }
    let mut refs: Vec<HashSet<String>> = vec![HashSet::new(); defs.len()];
    let mut live_names: HashSet<String> = HashSet::new();
    for (file, (_, code, attrs)) in sources.iter().enumerate() {
        live_names.extend(attrs.iter().filter(|a| names.contains(a.as_str())).cloned());
        for (at, word) in idents(code) {
            if skip[file][at] || !names.contains(word) {
                continue;
            }
            match owners[file][at] {
                u32::MAX => {
                    live_names.insert(word.to_string());
                }
                // Its own name in its own body is recursion only when no other function has
                // that name; otherwise it may be a call to one (`fn len(&self) { self.0.len() }`).
                o if defs[o as usize].name == word && def_count[word] == 1 => {}
                o => {
                    refs[o as usize].insert(word.to_string());
                }
            }
        }
    }

    // Pass 3: liveness, from the entry points and the code outside functions.
    let mut by_name: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, d) in defs.iter().enumerate() {
        by_name.entry(d.name.as_str()).or_default().push(i);
    }
    let mut live = vec![false; defs.len()];
    let mut work: Vec<usize> = (0..defs.len()).filter(|&i| defs[i].entry).collect();
    for n in &live_names {
        work.extend(by_name.get(n.as_str()).into_iter().flatten().copied());
    }
    while let Some(i) = work.pop() {
        if std::mem::replace(&mut live[i], true) {
            continue;
        }
        for n in &refs[i] {
            work.extend(
                by_name
                    .get(n.as_str())
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&j| !live[j]),
            );
        }
    }
    defs.iter()
        .zip(&live)
        .filter(|(d, &l)| d.public && !l)
        .map(|(d, _)| format!("{}:{}", sources[d.file].0, d.name))
        .collect()
}

/// A list file's entries: each line `- `key` — reason`.
fn load_list(root: &Path, rel: &str) -> BTreeSet<String> {
    let Ok(text) = fs::read_to_string(root.join(rel)) else {
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
    let base = load_list(root, BASELINE);
    let new = found.difference(&base).cloned().collect();
    let stale = base.difference(&found).cloned().collect();
    (new, stale, found.intersection(&base).count())
}

/// Whether `name` occurs as a word inside a backtick span of `cell` — how the SRS names code.
fn cites(cell: &str, name: &str) -> bool {
    cell.split('`')
        .skip(1)
        .step_by(2)
        .any(|span| word_count(span, name) > 0)
}

/// R7 (NFR-TEST-01): a requirement whose **acceptance** names, as its evidence, a function R6
/// finds unreferenced is evidenced by code the product never runs — `FR-PAN-06/07/08` (#229)
/// passed that way. Each such `ID:name` must be exempted with a reason; an exemption that no
/// longer applies is stale. Only the acceptance cell counts: a test that merely *uses* a test
/// seam while exercising production is not flagged. Returns (cited, stale, exempted count).
pub fn check_acceptance(
    root: &Path,
    rows: &[(String, String)],
) -> (Vec<String>, Vec<String>, usize) {
    let names: BTreeSet<String> = unreferenced(root)
        .iter()
        .filter_map(|k| k.rsplit(':').next().map(str::to_string))
        .collect();
    let found: BTreeSet<String> = rows
        .iter()
        .flat_map(|(id, cell)| {
            names
                .iter()
                .filter(|n| cites(cell, n))
                .map(move |n| format!("{id}:{n}"))
        })
        .collect();
    let exempt = load_list(root, EXEMPTIONS);
    let cited = found.difference(&exempt).cloned().collect();
    let stale = exempt.difference(&found).cloned().collect();
    (cited, stale, found.intersection(&exempt).count())
}

#[cfg(test)]
mod tests {
    use super::{
        check, check_acceptance, cites, is_pub_fn, unreferenced, word_count, BASELINE, EXEMPTIONS,
    };
    use std::collections::BTreeSet;
    use std::fs;

    /// NFR-TEST-01: a definition is public with `pub` or `pub(…)` before it, past `const`,
    /// `async`, `unsafe` and `extern`, across lines and after an attribute; private ones are not.
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_pub_fn_definitions_are_recognised() {
        for (code, want) in [
            ("pub fn a() {}", true),
            ("    pub(crate) fn b_2(x: u8) -> u8 {", true),
            ("pub const fn c() {}", true),
            ("pub async unsafe fn d() {}", true),
            ("pub(super) extern     fn e() {}", true),
            ("#[inline] pub fn f() {}", true),
            ("pub\n    fn g() {}", true),
            ("pub(in crate::x) fn h() {}", true),
            ("fn private() {}", false),
            ("#[inline]\nfn private2() {}", false),
            ("x.publish(); fn p3() {}", false),
        ] {
            let at = code.rfind("fn ").unwrap();
            assert_eq!(is_pub_fn(code, at), want, "{code:?}");
        }
        assert_eq!(word_count("a(x); ab(); a_b(); a;", "a"), 2);
    }

    /// A scratch workspace of `files` (path, text); returns what R6 finds unreferenced, as names.
    fn scan(tag: &str, files: &[(&str, &str)]) -> BTreeSet<String> {
        let root = std::env::temp_dir().join(format!("xtask-r6-{tag}-{}", std::process::id()));
        for (path, text) in files {
            let p = root.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        let found = unreferenced(&root);
        let _ = fs::remove_dir_all(&root);
        found
            .iter()
            .map(|k| k.rsplit(':').next().unwrap().to_string())
            .collect()
    }

    /// NFR-TEST-01 (audit T, U, X, 2026-10-10): each way the old name count let an unused public
    /// function through is flagged now — and each live context still keeps a function alive.
    /// The old rule ("the name occurs at least twice in production code") passed every one of the
    /// `flagged` cases.
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_r6_sees_what_the_name_count_missed() {
        let lib = "
            // T: an orphan chain — `chain_top` is never called, so neither is `chain_mid`.
            pub fn chain_top() { chain_mid(); }
            pub fn chain_mid() {}
            // T: one name, several definitions, no caller.
            pub struct A; pub struct B;
            impl A { pub fn twin(&self) {} }
            impl B { pub fn twin(&self) {} }
            // Recursion is not a caller.
            pub fn recurse(n: u8) { if n > 0 { recurse(n - 1) } }
            // A `use` line is not a caller.
            pub mod m { pub fn imported() {} }
            use crate::m::imported;
            // X: an attribute on the same line, and `pub` split from `fn`.
            #[inline] pub fn attributed() {}
            pub
            fn split() {}
            // U: an item behind `cfg(all(test, …))`, and one whose generics hold a comma.
            #[cfg(all(test, feature = \"x\"))]
            fn all_test() { only_in_all_test(); }
            pub fn only_in_all_test() {}
            #[cfg(test)]
            fn generic<A, B>() { only_in_generic(); }
            pub fn only_in_generic() {}
            // U: an out-of-line test module.
            #[cfg(test)]
            mod tests;
            pub fn only_in_test_file() {}

            // Live: called from main; through a live chain; from a trait impl the framework calls;
            // named in an attribute string; used outside any function; behind `any(test, …)`.
            // Each `cfg`-gated function is called, so its `cfg` alone decides what its body counts as.
            fn main() { live_top(); all_test(); generic::<u8, u8>(); any_test(); }
            pub fn live_top() { live_mid(); }
            pub fn live_mid() {}
            pub struct S;
            impl std::fmt::Display for S {
                fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { from_impl(); Ok(()) }
            }
            pub fn from_impl() {}
            #[derive(serde::Deserialize)]
            pub struct C { #[serde(deserialize_with = \"by_attribute\")] x: u8 }
            pub fn by_attribute<'de, D>(d: D) -> Result<u8, D::Error> { todo!() }
            pub const TABLE: [fn(); 1] = [in_const];
            pub fn in_const() {}
            #[cfg(any(test, feature = \"x\"))]
            fn any_test() { only_in_any_test(); }
            pub fn only_in_any_test() {}
            // Delegation to a same-named method from a live trait impl keeps the callee alive.
            pub struct Inner;
            impl Inner { pub fn draw(&self) {} }
            pub struct Outer(Inner);
            impl Drawable for Outer { fn draw(&self) { self.0.draw() } }
            pub fn fn_pointer_user(f: fn(u8) -> u8) -> u8 { f(1) }
            fn uses_pointer() { fn_pointer_user(|x| x); }
        ";
        let found = scan(
            "fixtures",
            &[
                ("crates/demo/src/main.rs", lib),
                (
                    "crates/demo/src/tests.rs",
                    "fn t() { super::only_in_test_file(); }",
                ),
            ],
        );
        let flagged = [
            "chain_top",
            "chain_mid",
            "twin",
            "recurse",
            "imported",
            "attributed",
            "split",
            "only_in_all_test",
            "only_in_generic",
            "only_in_test_file",
        ];
        for name in flagged {
            assert!(
                found.contains(name),
                "{name} should be unreferenced: {found:?}"
            );
        }
        for name in [
            "live_top",
            "live_mid",
            "from_impl",
            "by_attribute",
            "in_const",
            "only_in_any_test",
            "draw",
        ] {
            assert!(!found.contains(name), "{name} is live: {found:?}");
        }
        // `fn_pointer_user` is called by `uses_pointer`, which nothing calls: dead too, and the
        // `fn(u8) -> u8` pointer type in its signature is not a definition.
        assert!(found.contains("fn_pointer_user"), "{found:?}");
        assert!(!found.iter().any(|n| n == "u8"), "{found:?}");
        assert_eq!(found.len(), flagged.len() + 1, "{found:?}");
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
             fn main() { caller(); }\n\
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

    /// NFR-TEST-01: acceptance cites a function only by name in a backtick span, as a word.
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_acceptance_citations_are_backtick_words() {
        assert!(cites("`row_scroll_px`/`hz_to_x` shift a row", "hz_to_x"));
        assert!(cites("`hz_to_x(f)` maps", "hz_to_x"));
        assert!(
            !cites("hz_to_x maps", "hz_to_x"),
            "bare prose is not a citation"
        );
        assert!(
            !cites("`hz_to_x_2` maps", "hz_to_x"),
            "a longer name is not this one"
        );
    }

    /// NFR-TEST-01, in a scratch workspace: a requirement whose acceptance cites an unreferenced
    /// function fails unless exempted; one citing a production-called function does not; an
    /// exemption that no longer applies is stale.
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_acceptance_citing_unreachable_code_is_flagged() {
        let root = std::env::temp_dir().join(format!("xtask-r7-{}", std::process::id()));
        let src = root.join("crates/demo/src");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(root.join("docs/test")).unwrap();
        fs::write(
            src.join("lib.rs"),
            "pub fn live() {}\npub fn ghost() {}\npub fn gauge() {}\nfn main() { live(); }\n",
        )
        .unwrap();
        fs::write(
            root.join(EXEMPTIONS),
            "- `FR-B-01:gauge` — a measuring instrument\n- `FR-C-01:ghost` — no longer cited\n",
        )
        .unwrap();
        let rows = [
            ("FR-A-01".to_string(), "`ghost` does it (test)".to_string()),
            (
                "FR-B-01".to_string(),
                "`gauge` measures it (test)".to_string(),
            ),
            ("FR-D-01".to_string(), "`live` does it (test)".to_string()),
        ];
        let (cited, stale, kept) = check_acceptance(&root, &rows);
        let _ = fs::remove_dir_all(&root);
        assert_eq!(cited, vec!["FR-A-01:ghost".to_string()], "cited");
        assert_eq!(stale, vec!["FR-C-01:ghost".to_string()], "stale");
        assert_eq!(kept, 1, "the exempted one is accepted");
    }
}
