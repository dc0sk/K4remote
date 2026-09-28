//! Which parts of a Rust source file are **test code** (NFR-TEST-01), for the gate's two checks
//! that care: requirement traces count as tests only inside test code (R3), and an encoder
//! counts as reachable only if called outside it (R5).
//!
//! The rule used to be "everything after the file's first test-cfg attribute" — which let a
//! test-only item in the middle of a file, or merely a *comment quoting the attribute*, turn the
//! rest of the file into "test code": production `trace:` lines were credited as tests (fail-open)
//! and production callers vanished (fail-closed). Now only a real attribute counts (not one inside
//! a comment or a string), and it covers only the item it is attached to: a `mod … { … }`, a
//! braced item, or an item ended by `;` or `,` (a variant, a field).

use std::ops::Range;

/// A byte-per-byte mask of `text`: `true` where the byte is code, `false` inside a comment, a
/// string (plain, byte, raw) or a character literal. Lifetimes and labels are code.
pub fn code_mask(text: &str) -> Vec<bool> {
    let b = text.as_bytes();
    let mut mask = vec![true; b.len()];
    let mut i = 0;
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    while i < b.len() {
        let c = b[i];
        // Line comment.
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            let end = b[i..]
                .iter()
                .position(|&x| x == b'\n')
                .map_or(b.len(), |p| i + p);
            mask[i..end].fill(false);
            i = end;
            continue;
        }
        // Block comment (nested).
        if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let (mut j, mut depth) = (i + 2, 1);
            while j < b.len() && depth > 0 {
                if b[j] == b'/' && b.get(j + 1) == Some(&b'*') {
                    depth += 1;
                    j += 2;
                } else if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
                    depth -= 1;
                    j += 2;
                } else {
                    j += 1;
                }
            }
            mask[i..j].fill(false);
            i = j;
            continue;
        }
        // Raw string: r"…", r#"…"#, br#"…"# — `r` not preceded by an identifier character.
        if (c == b'r' || (c == b'b' && b.get(i + 1) == Some(&b'r'))) && (i == 0 || !ident(b[i - 1]))
        {
            let start = i;
            let mut j = i + if c == b'b' { 2 } else { 1 };
            let mut hashes = 0;
            while b.get(j) == Some(&b'#') {
                hashes += 1;
                j += 1;
            }
            if b.get(j) == Some(&b'"') {
                j += 1;
                let close: Vec<u8> = std::iter::once(b'"')
                    .chain(std::iter::repeat_n(b'#', hashes))
                    .collect();
                let end = b[j..]
                    .windows(close.len())
                    .position(|w| w == close.as_slice())
                    .map_or(b.len(), |p| j + p + close.len());
                mask[start..end].fill(false);
                i = end;
                continue;
            }
        }
        // String (and byte string): "…" with escapes.
        if c == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(b.len());
            mask[i..end].fill(false);
            i = end;
            continue;
        }
        // Character literal vs lifetime: `'\…'` or `'x'` (x one char, maybe multi-byte) is a
        // literal; anything else (`'a`, `'static`, a label) is code.
        if c == b'\'' {
            if b.get(i + 1) == Some(&b'\\') {
                let mut j = i + 2;
                while j < b.len() && b[j] != b'\'' {
                    j += 1;
                }
                let end = (j + 1).min(b.len());
                mask[i..end].fill(false);
                i = end;
                continue;
            }
            if let Some(ch) = text[i + 1..].chars().next() {
                let after = i + 1 + ch.len_utf8();
                if b.get(after) == Some(&b'\'') {
                    mask[i..=after].fill(false);
                    i = after + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    mask
}

/// The byte ranges of `text` that are test code: each real test-cfg attribute and the item it
/// is attached to.
pub fn test_regions(text: &str) -> Vec<Range<usize>> {
    const ATTR: &str = "#[cfg(test)]";
    let b = text.as_bytes();
    let mask = code_mask(text);
    let code = |i: usize| mask.get(i).copied().unwrap_or(false);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(p) = text[from..].find(ATTR) {
        let start = from + p;
        from = start + ATTR.len();
        if !(start..start + ATTR.len()).all(code) {
            continue; // in a comment or a string
        }
        // The item: the first `{` (brace-matched, then a following `;` if any), or `;` / `,`
        // at depth 0, whichever comes first — counting code bytes only.
        let (mut j, mut depth) = (from, 0i32);
        let mut end = b.len();
        while j < b.len() {
            if !code(j) {
                j += 1;
                continue;
            }
            match b[j] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                b';' | b',' if depth == 0 => {
                    end = j + 1;
                    break;
                }
                b'{' if depth == 0 => {
                    let mut k = j;
                    let mut braces = 0i32;
                    while k < b.len() {
                        if code(k) {
                            match b[k] {
                                b'{' => braces += 1,
                                b'}' => {
                                    braces -= 1;
                                    if braces == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                        k += 1;
                    }
                    end = (k + 1).min(b.len());
                    let rest = text[end..].trim_start();
                    if rest.starts_with(';') {
                        end = b.len() - rest.len() + 1;
                    }
                    break;
                }
                _ => {}
            }
            j += 1;
        }
        out.push(start..end);
        from = end.max(from);
    }
    out
}

/// `text` with its test regions removed.
pub fn without_tests(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for r in test_regions(text) {
        out.push_str(&text[at..r.start]);
        at = r.end;
    }
    out.push_str(&text[at..]);
    out
}

/// Whether byte offset `at` of `text` lies in test code, given its regions.
pub fn in_regions(regions: &[Range<usize>], at: usize) -> bool {
    regions.iter().any(|r| r.contains(&at))
}

#[cfg(test)]
mod tests {
    use super::{code_mask, in_regions, test_regions, without_tests};

    /// NFR-TEST-01: comments, strings (plain, byte, raw), and character literals are not code;
    /// lifetimes and labels are.
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_the_mask_knows_comments_strings_and_chars() {
        let t = r####"a // c { "x"
b /* n /* e */ } */ c "s{\"}" br#"r{"# 'x' '\'' '{' 'é' <'a> 'l: loop {}"####;
        let m = code_mask(t);
        let code: String = t
            .char_indices()
            .filter(|(i, _)| m[*i])
            .map(|(_, c)| c)
            .collect();
        for gone in ["// c", "/* n", "s{", "r{", "'x'", "'{'", "'é'"] {
            assert!(!code.contains(gone), "{gone} counted as code: {code:?}");
        }
        for kept in ["a ", "b ", " c ", "<'a>", "'l: loop {}"] {
            assert!(code.contains(kept), "{kept} not counted as code: {code:?}");
        }
    }

    /// NFR-TEST-01: a test module is test code up to its matching brace — not beyond, and not cut
    /// short by an unbalanced brace inside a string or a char literal — and the production code
    /// after it is not test code.
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_a_test_module_ends_at_its_brace() {
        let t = "fn live() {}\n#[cfg(test)]\nmod tests {\n    fn t() { let _ = \"}\\n}\"; let _ = '}'; }\n}\nfn after() { call_me(); }\n";
        let r = test_regions(t);
        assert_eq!(r.len(), 1);
        let region = &t[r[0].clone()];
        assert!(
            region.starts_with("#[cfg(test)]") && region.ends_with("}\n}"),
            "{region:?}"
        );
        assert!(without_tests(t).contains("fn after() { call_me(); }"));
        assert!(without_tests(t).contains("fn live() {}"));
    }

    /// NFR-TEST-01: the attribute quoted in a comment or a string is not an attribute — the case
    /// that used to turn a whole file into "test code".
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_a_quoted_attribute_is_not_one() {
        let t = "/// Deliberately not behind #[cfg(test)] …\nfn a() {}\nconst S: &str = \"#[cfg(test)]\";\nfn b() { call_me(); }\n";
        assert!(test_regions(t).is_empty());
        assert_eq!(without_tests(t), t);
    }

    /// NFR-TEST-01: an item-level attribute covers only its item — an enum variant, a match arm's
    /// worth of braced item, a struct, an impl, a `use` — and production code after it stays live.
    /// trace: NFR-TEST-01
    #[test]
    fn nfr_test_01_an_item_level_attribute_covers_only_its_item() {
        let t = "enum E {\n    A,\n    #[cfg(test)]\n    B,\n    C,\n}\n#[cfg(test)]\nstruct S;\n#[cfg(test)]\nimpl S { fn f() {} }\n#[cfg(test)]\nuse std::{fmt, io};\nfn live() { rx_eq_flat(); }\n";
        let cut = without_tests(t);
        assert!(cut.contains("A,") && cut.contains("C,"), "{cut}");
        assert!(!cut.contains("B,"), "{cut}");
        assert!(
            !cut.contains("struct S;") && !cut.contains("impl S"),
            "{cut}"
        );
        assert!(!cut.contains("use std::{fmt, io};"), "{cut}");
        assert!(cut.contains("fn live() { rx_eq_flat(); }"), "{cut}");
        // A trace in production after the item-level attributes is not in a test region.
        let at = t.find("fn live").unwrap();
        assert!(!in_regions(&test_regions(t), at));
    }
}
