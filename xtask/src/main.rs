//! Traceability gate (rules R3/R4 from docs/README.md).
//!
//! Cross-checks requirement IDs declared in the SRS against the `trace:` IDs
//! annotated in the test suite:
//!   * R4 (hard error): every `trace:` ID must name a declared requirement — a
//!     dangling trace fails the build.
//!   * R3 (hard error): every **Must/Should** requirement whose verification
//!     method includes **Test** must have at least one trace **in a test
//!     context** (a `tests/` file or a `#[cfg(test)]` module) — unless it is
//!     listed, with a reason, in `docs/test/r3-waivers.md`. Source-comment
//!     `trace:` annotations document intent but do NOT satisfy R3 on their own.
//!   * Duplicate declared IDs fail the build (SRS hygiene).
//!
//! A coverage report is written to `docs/test/coverage.generated.md`.
//!
//! Run with `cargo run -p xtask` (alias: `cargo xtask`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest has a parent")
        .to_path_buf()
}

fn is_req_id(tok: &str) -> bool {
    (tok.starts_with("FR-") || tok.starts_with("NFR-"))
        && tok.len() > 4
        && tok
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-')
}

/// Collect `.rs` files under `dir`, skipping `target/`.
fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            out.push(path);
        }
    }
}

/// One declared requirement: priority (`M`/`S`/`C`) and verification methods.
#[derive(Debug, Clone)]
struct Req {
    priority: char,
    /// Verification-method letters, e.g. `"T"`, `"T/D"`.
    verification: String,
}

impl Req {
    /// Must/Should priority verified (at least partly) by Test → needs a test.
    fn needs_test(&self) -> bool {
        matches!(self.priority, 'M' | 'S') && self.verification.contains('T')
    }
}

/// Parse the SRS requirement rows: returns each ID's `Req` plus any IDs that are
/// declared more than once (a hygiene error).
fn declared_requirements(root: &Path) -> (BTreeMap<String, Req>, BTreeSet<String>) {
    let srs = root.join("docs/requirements/system-requirements.md");
    let mut reqs = BTreeMap::new();
    let mut duplicates = BTreeSet::new();
    let Ok(text) = fs::read_to_string(srs) else {
        return (reqs, duplicates);
    };
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("| `") {
            continue;
        }
        // Columns: | `ID` | statement | stakeholder | Pri | Ver | acceptance |
        let cols: Vec<&str> = trimmed.split('|').map(str::trim).collect();
        if cols.len() < 6 {
            continue;
        }
        let id = cols[1].trim_matches('`');
        if !is_req_id(id) {
            continue;
        }
        let priority = cols[4].chars().next().unwrap_or('?');
        let req = Req {
            priority,
            verification: cols[5].to_string(),
        };
        if reqs.insert(id.to_string(), req).is_some() {
            duplicates.insert(id.to_string());
        }
    }
    (reqs, duplicates)
}

/// Requirement IDs referenced via `trace:` annotations, split by context: every
/// trace, and only those in a test context (a `tests/` file or after a
/// `#[cfg(test)]` marker in the file).
fn traced_requirements(root: &Path) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut files = Vec::new();
    for sub in ["crates", "app", "xtask"] {
        collect_rs(&root.join(sub), &mut files);
    }
    let mut all = BTreeSet::new();
    let mut test_ctx = BTreeSet::new();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        let in_tests_dir = file.components().any(|c| c.as_os_str() == "tests");
        let mut cfg_test_seen = false;
        for line in text.lines() {
            if line.contains("#[cfg(test)]") {
                cfg_test_seen = true;
            }
            let Some(idx) = line.find("trace:") else {
                continue;
            };
            let is_test = in_tests_dir || cfg_test_seen;
            let after = &line[idx + "trace:".len()..];
            for raw in after.split([',', ' ', '\t']) {
                let tok = raw.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
                if is_req_id(tok) {
                    all.insert(tok.to_string());
                    if is_test {
                        test_ctx.insert(tok.to_string());
                    }
                }
            }
        }
    }
    (all, test_ctx)
}

/// R3 waivers: IDs explicitly exempted from needing a test, each with a reason.
/// Table rows `| `ID` | reason |` in `docs/test/r3-waivers.md`.
fn load_waivers(root: &Path) -> BTreeSet<String> {
    let path = root.join("docs/test/r3-waivers.md");
    let mut ids = BTreeSet::new();
    let Ok(text) = fs::read_to_string(path) else {
        return ids;
    };
    for line in text.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("| `") {
            if let Some(end) = rest.find('`') {
                let id = &rest[..end];
                if is_req_id(id) {
                    ids.insert(id.to_string());
                }
            }
        }
    }
    ids
}

fn main() {
    let root = workspace_root();
    let (declared, duplicates) = declared_requirements(&root);
    let (all_traces, test_traces) = traced_requirements(&root);
    let waivers = load_waivers(&root);

    let declared_ids: BTreeSet<String> = declared.keys().cloned().collect();
    let dangling: Vec<_> = all_traces.difference(&declared_ids).cloned().collect();
    let stale_waivers: Vec<_> = waivers.difference(&declared_ids).cloned().collect();

    // R3: Must/Should + Test requirements without a test-context trace, unless waived.
    let mut r3_missing = Vec::new();
    let mut needs_test = 0usize;
    for (id, req) in &declared {
        if req.needs_test() {
            needs_test += 1;
            if !test_traces.contains(id) && !waivers.contains(id) {
                r3_missing.push(id.clone());
            }
        }
    }

    write_coverage_report(&root, &declared, &test_traces, &waivers);

    println!("K4 Remote — traceability report");
    println!("  requirements declared (SRS): {}", declared.len());
    println!("  duplicate declared IDs:      {}", duplicates.len());
    println!("  Must/Should + Test:          {needs_test}");
    println!("  test-context traces:         {}", test_traces.len());
    println!("  R3 waivers:                  {}", waivers.len());
    println!("  R3 gaps (unwaived):          {}", r3_missing.len());
    for id in &r3_missing {
        println!("    - {id}");
    }

    let mut failed = false;
    if !duplicates.is_empty() {
        eprintln!(
            "\nerror (SRS hygiene): {} duplicate ID(s):",
            duplicates.len()
        );
        for id in &duplicates {
            eprintln!("    ! {id}");
        }
        failed = true;
    }
    if !dangling.is_empty() {
        eprintln!(
            "\nerror (R4): {} trace ID(s) reference unknown requirements:",
            dangling.len()
        );
        for id in &dangling {
            eprintln!("    ! {id}");
        }
        failed = true;
    }
    if !stale_waivers.is_empty() {
        eprintln!(
            "\nerror: {} waiver(s) name unknown requirements:",
            stale_waivers.len()
        );
        for id in &stale_waivers {
            eprintln!("    ! {id}");
        }
        failed = true;
    }
    if !r3_missing.is_empty() {
        eprintln!(
            "\nerror (R3): {} Must/Should+Test requirement(s) lack a test-context trace \
             and are not waived in docs/test/r3-waivers.md:",
            r3_missing.len()
        );
        for id in &r3_missing {
            eprintln!("    ! {id}");
        }
        failed = true;
    }

    if failed {
        std::process::exit(1);
    }
    println!("\nOK: R3 (Must/Should+Test covered or waived) and R4 (no dangling) satisfied.");
}

/// Write `docs/test/coverage.generated.md` — the promised coverage report.
fn write_coverage_report(
    root: &Path,
    declared: &BTreeMap<String, Req>,
    test_traces: &BTreeSet<String>,
    waivers: &BTreeSet<String>,
) {
    let mut out = String::from(
        "# Requirement coverage (generated by `cargo xtask` — do not edit)\n\n\
         Legend: ✅ test-traced · 🟡 waived (see r3-waivers.md) · ⚪ not test-required · ❌ gap\n\n\
         | Requirement | Pri | Ver | Status |\n|---|---|---|---|\n",
    );
    for (id, req) in declared {
        let status = if test_traces.contains(id) {
            "✅"
        } else if waivers.contains(id) {
            "🟡"
        } else if req.needs_test() {
            "❌"
        } else {
            "⚪"
        };
        out.push_str(&format!(
            "| `{id}` | {} | {} | {status} |\n",
            req.priority, req.verification
        ));
    }
    let _ = fs::write(root.join("docs/test/coverage.generated.md"), out);
}
