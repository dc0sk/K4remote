//! Traceability gate (rules R3/R4 from docs/README.md).
//!
//! Cross-checks requirement IDs declared in the SRS against the `trace:` IDs
//! annotated in test/source files:
//!   * R4 (hard error): every `trace:` ID must name a requirement in the SRS —
//!     a dangling trace fails the build.
//!   * R3 (reported): every SRS requirement should be covered by >=1 trace —
//!     uncovered requirements are listed (informational while scaffolding).
//!
//! Run with `cargo run -p xtask` (alias: `cargo xtask` once configured).

use std::collections::BTreeSet;
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

/// Requirement IDs declared in the SRS (leading backtick-quoted cell of a row).
fn declared_requirements(root: &Path) -> BTreeSet<String> {
    let srs = root.join("docs/requirements/system-requirements.md");
    let mut ids = BTreeSet::new();
    let Ok(text) = fs::read_to_string(srs) else {
        return ids;
    };
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("| `") {
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

/// Requirement IDs referenced via `trace:` annotations across the workspace.
fn traced_requirements(root: &Path) -> BTreeSet<String> {
    let mut files = Vec::new();
    for sub in ["crates", "app", "xtask"] {
        collect_rs(&root.join(sub), &mut files);
    }
    let mut ids = BTreeSet::new();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines() {
            let Some(idx) = line.find("trace:") else {
                continue;
            };
            let after = &line[idx + "trace:".len()..];
            for raw in after.split([',', ' ', '\t']) {
                let tok = raw.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '-');
                if is_req_id(tok) {
                    ids.insert(tok.to_string());
                }
            }
        }
    }
    ids
}

fn main() {
    let root = workspace_root();
    let declared = declared_requirements(&root);
    let traced = traced_requirements(&root);

    let uncovered: Vec<_> = declared.difference(&traced).collect();
    let dangling: Vec<_> = traced.difference(&declared).collect();

    println!("K4 Remote — traceability report");
    println!("  requirements declared (SRS): {}", declared.len());
    println!("  requirements traced (tests): {}", traced.len());
    println!("  uncovered (R3, informational): {}", uncovered.len());
    for id in &uncovered {
        println!("    - {id}");
    }

    if !dangling.is_empty() {
        eprintln!(
            "\nerror (R4): {} trace ID(s) reference unknown requirements:",
            dangling.len()
        );
        for id in &dangling {
            eprintln!("    ! {id}");
        }
        std::process::exit(1);
    }

    println!("\nOK: no dangling traces (R4 satisfied).");
}
