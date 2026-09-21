//! A small, strict, bounded JSON parser for the networks whose messages nest (FreeDV Reporter's
//! `bulk_update` is a list of `[name, {..}]` pairs), where the flat scanners of [`crate::psk`] and
//! [`crate::pota`] do not reach.
//!
//! The text is **untrusted**, so this parser refuses what a lenient one would repair: a trailing
//! comma, a duplicate key, a lone surrogate, a control character in a string, a number with a
//! leading zero, anything after the value. It is bounded three ways so no input can make it
//! allocate or recurse without limit: [`MAX_DEPTH`] levels of nesting, [`MAX_NODES`] values in
//! all, and [`MAX_STRING`] bytes in any one string or key.

/// Deepest nesting read.
pub const MAX_DEPTH: usize = 8;
/// Most values (of any kind) read from one text.
pub const MAX_NODES: usize = 20_000;
/// Longest string or key read, bytes after decoding.
pub const MAX_STRING: usize = 4096;

/// A JSON value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    /// Members in order; keys are unique (a duplicate is a parse error).
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// The member `key` of an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Element `i` of an array.
    pub fn at(&self, i: usize) -> Option<&Value> {
        match self {
            Value::Arr(a) => a.get(i),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) if n.is_finite() => Some(*n),
            _ => None,
        }
    }

    /// A non-negative whole number, exactly representable (at most 2^53).
    pub fn as_u64(&self) -> Option<u64> {
        let n = self.as_f64()?;
        (n >= 0.0 && n.fract() == 0.0 && n <= 9_007_199_254_740_992.0).then_some(n as u64)
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }
}

struct Parser<'a> {
    /// The text, kept as a `&str` so slicing it to read a character costs nothing and cannot
    /// split one (the input is valid UTF-8 by construction).
    text: &'a str,
    s: &'a [u8],
    i: usize,
    nodes: usize,
}

type R<T> = Result<T, String>;

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self
            .s
            .get(self.i)
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.i += 1;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        let hit = self.peek() == Some(b);
        if hit {
            self.i += 1;
        }
        hit
    }

    fn node(&mut self) -> R<()> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(format!("more than {MAX_NODES} values"));
        }
        Ok(())
    }

    fn value(&mut self, depth: usize) -> R<Value> {
        self.node()?;
        self.ws();
        match self.peek().ok_or("unexpected end")? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(Value::Str),
            b'-' | b'0'..=b'9' => self.number(),
            b't' => self.literal("true", Value::Bool(true)),
            b'f' => self.literal("false", Value::Bool(false)),
            b'n' => self.literal("null", Value::Null),
            other => Err(format!("unexpected byte 0x{other:02x}")),
        }
    }

    fn literal(&mut self, word: &str, v: Value) -> R<Value> {
        if self.s[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(format!("expected {word}"))
        }
    }

    fn object(&mut self, depth: usize) -> R<Value> {
        if depth >= MAX_DEPTH {
            return Err(format!("nested deeper than {MAX_DEPTH}"));
        }
        self.i += 1; // {
        let mut members: Vec<(String, Value)> = Vec::new();
        self.ws();
        if self.eat(b'}') {
            return Ok(Value::Obj(members));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err("expected a string key".into());
            }
            let key = self.string()?;
            if members.iter().any(|(k, _)| *k == key) {
                return Err(format!("duplicate key {key:?}"));
            }
            self.ws();
            if !self.eat(b':') {
                return Err("expected ':'".into());
            }
            let v = self.value(depth + 1)?;
            members.push((key, v));
            self.ws();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b'}') {
                return Ok(Value::Obj(members));
            }
            return Err("expected ',' or '}'".into());
        }
    }

    fn array(&mut self, depth: usize) -> R<Value> {
        if depth >= MAX_DEPTH {
            return Err(format!("nested deeper than {MAX_DEPTH}"));
        }
        self.i += 1; // [
        let mut items = Vec::new();
        self.ws();
        if self.eat(b']') {
            return Ok(Value::Arr(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(b',') {
                continue;
            }
            if self.eat(b']') {
                return Ok(Value::Arr(items));
            }
            return Err("expected ',' or ']'".into());
        }
    }

    fn hex4(&mut self) -> R<u32> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = self
                .peek()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or("bad \\u escape")?;
            v = v * 16 + d;
            self.i += 1;
        }
        Ok(v)
    }

    fn string(&mut self) -> R<String> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let b = self.peek().ok_or("unterminated string")?;
            match b {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.i += 1;
                    let e = self.peek().ok_or("unterminated escape")?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                // A high surrogate must be followed by a low one.
                                if !(self.eat(b'\\') && self.eat(b'u')) {
                                    return Err("lone surrogate".into());
                                }
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err("lone surrogate".into());
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err("lone surrogate".into());
                            } else {
                                hi
                            };
                            out.push(char::from_u32(cp).ok_or("bad code point")?);
                        }
                        _ => return Err("bad escape".into()),
                    }
                }
                0..=0x1f => return Err("control character in a string".into()),
                _ => {
                    // Copy one whole UTF-8 character. `self.i` is always on a character boundary:
                    // it only ever advances by whole characters or over ASCII.
                    let c = self.text[self.i..]
                        .chars()
                        .next()
                        .ok_or("unterminated string")?;
                    out.push(c);
                    self.i += c.len_utf8();
                }
            }
            if out.len() > MAX_STRING {
                return Err(format!("a string longer than {MAX_STRING} bytes"));
            }
        }
    }

    fn number(&mut self) -> R<Value> {
        let start = self.i;
        self.eat(b'-');
        match self.peek() {
            // A leading zero needs no check of its own: `01` reads as `0` and leaves a `1` that
            // nothing accepts (pinned by the tests), so a second guard would only mask the first.
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    self.i += 1;
                }
            }
            _ => return Err("bad number".into()),
        }
        if self.eat(b'.') {
            let d0 = self.i;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == d0 {
                return Err("bad number".into());
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.i += 1;
            }
            let d0 = self.i;
            while self.peek().is_some_and(|b| b.is_ascii_digit()) {
                self.i += 1;
            }
            if self.i == d0 {
                return Err("bad number".into());
            }
        }
        if self.i - start > 32 {
            return Err("a number of more than 32 characters".into());
        }
        let text = std::str::from_utf8(&self.s[start..self.i]).map_err(|_| "bad number")?;
        let n: f64 = text.parse().map_err(|_| "bad number".to_string())?;
        if n.is_finite() {
            Ok(Value::Num(n))
        } else {
            Err("a number out of range".into())
        }
    }
}

/// Parse `text` as exactly one JSON value.
pub fn parse(text: &str) -> Result<Value, String> {
    let mut p = Parser {
        text,
        s: text.as_bytes(),
        i: 0,
        nodes: 0,
    };
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err("text after the value".into());
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each bound is pinned once, as a number (a test built from the constant under test would move
    // with it and never notice a change); the fixtures are derived from these.
    const DEPTH: usize = 8;
    const NODES: usize = 20_000;
    const STRING: usize = 4096;

    /// FR-SPOT-08: real-shaped messages parse to the right tree, and the accessors read what they
    /// should — exactly, and no more.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_json_parses_and_reads() {
        let v = parse(r#" {"sid":"a1","freq":14236000,"transmitting":false,"snr":-7.5,"x":null,"l":[1,"b",[true]]} "#)
            .unwrap();
        assert_eq!(v.get("sid").and_then(Value::as_str), Some("a1"));
        assert_eq!(v.get("freq").and_then(Value::as_u64), Some(14_236_000));
        assert_eq!(v.get("transmitting").and_then(Value::as_bool), Some(false));
        assert_eq!(v.get("snr").and_then(Value::as_f64), Some(-7.5));
        assert_eq!(v.get("x"), Some(&Value::Null));
        assert_eq!(v.get("missing"), None);
        let l = v.get("l").unwrap();
        assert_eq!(l.at(0).and_then(Value::as_u64), Some(1));
        assert_eq!(l.at(1).and_then(Value::as_str), Some("b"));
        assert_eq!(
            l.at(2).and_then(|a| a.at(0)).and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(l.at(3), None);
        assert_eq!(l.as_array().map(<[Value]>::len), Some(3));
        // Accessors do not coerce: a string is not a number, a float is not a whole number.
        assert_eq!(v.get("sid").and_then(Value::as_f64), None);
        assert_eq!(
            v.get("snr").and_then(Value::as_u64),
            None,
            "-7.5 is not a u64"
        );
        assert_eq!(parse("-1").unwrap().as_u64(), None);
        assert_eq!(parse("1.0").unwrap().as_u64(), Some(1));
        // A fraction is not a whole number, positive or not (the sign must not be what stops it).
        assert_eq!(parse("1.5").unwrap().as_u64(), None);
        assert_eq!(parse("14236000.5").unwrap().as_u64(), None);
        assert_eq!(parse("0.5").unwrap().as_u64(), None);
        // A hand-built value can hold what the parser never produces; the accessors still refuse it.
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(Value::Num(bad).as_f64(), None);
            assert_eq!(Value::Num(bad).as_u64(), None);
        }
        assert_eq!(parse("1e3").unwrap().as_u64(), Some(1000));
        assert_eq!(
            parse("9007199254740992").unwrap().as_u64(),
            Some(9_007_199_254_740_992)
        );
        assert_eq!(
            parse("9007199254740994").unwrap().as_u64(),
            None,
            "beyond 2^53"
        );
        assert_eq!(parse("\"1\"").unwrap().as_u64(), None);
        assert_eq!(parse("true").unwrap().as_str(), None);
        assert_eq!(parse("{}").unwrap(), Value::Obj(Vec::new()));
        assert_eq!(parse("[]").unwrap(), Value::Arr(Vec::new()));
        // Escapes decode, including a surrogate pair and multi-byte text.
        let bs = '\\';
        let text =
            format!("\"a{bs}n{bs}\"{bs}{bs}{bs}/{bs}u00e9{bs}ud83d{bs}ude00 \u{e9}\u{4e2d}\"");
        assert_eq!(
            parse(&text).unwrap().as_str(),
            Some("a\n\"\\/\u{e9}\u{1f600} \u{e9}\u{4e2d}")
        );
    }

    /// FR-SPOT-08: what a lenient parser would repair is refused, and every bound holds.
    /// trace: FR-SPOT-08
    #[test]
    fn fr_spot_08_json_refuses_what_it_would_repair_and_is_bounded() {
        let bs = '\\';
        let bad: Vec<(&str, String)> = vec![
            ("empty", String::new()),
            ("whitespace only", "  ".into()),
            ("trailing comma array", "[1,]".into()),
            ("trailing comma object", r#"{"a":1,}"#.into()),
            ("leading comma", "[,1]".into()),
            ("double comma", "[1,,2]".into()),
            ("duplicate key", r#"{"a":1,"a":2}"#.into()),
            ("text after", "1 2".into()),
            ("two values", "[] []".into()),
            ("unquoted key", "{a:1}".into()),
            ("single quotes", "{'a':1}".into()),
            ("missing colon", r#"{"a" 1}"#.into()),
            ("missing value", r#"{"a":}"#.into()),
            ("unterminated string", "\"abc".into()),
            ("unterminated array", "[1,2".into()),
            ("unterminated object", r#"{"a":1"#.into()),
            ("control char", "\"a\tb\"".into()),
            ("newline in string", "\"a\nb\"".into()),
            ("bad escape", format!("\"{bs}q\"")),
            ("short unicode", format!("\"{bs}u12\"")),
            ("non-hex unicode", format!("\"{bs}u12g4\"")),
            ("lone high surrogate", format!("\"{bs}ud83d\"")),
            ("lone low surrogate", format!("\"{bs}ude00\"")),
            ("high then non-low", format!("\"{bs}ud83d{bs}u0041\"")),
            ("leading zero", "01".into()),
            ("negative leading zero", "-01".into()),
            ("plus sign", "+1".into()),
            ("bare minus", "-".into()),
            ("dot without digits", "1.".into()),
            ("leading dot", ".5".into()),
            ("exponent without digits", "1e".into()),
            ("hex number", "0x10".into()),
            ("NaN", "NaN".into()),
            ("Infinity", "Infinity".into()),
            ("overflowing number", "1e999".into()),
            ("too-long number", format!("1{}", "0".repeat(40))),
            ("bad literal", "tru".into()),
            ("capital literal", "True".into()),
            ("comment", "[1] // x".into()),
        ];
        for (name, text) in &bad {
            assert!(parse(text).is_err(), "{name} must be refused: {text:?}");
        }
        // The bounds first: pinned to the numbers above. Then depth: DEPTH levels are read, one more is not.
        assert_eq!((MAX_DEPTH, MAX_NODES, MAX_STRING), (DEPTH, NODES, STRING));
        let nest = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(parse(&nest(DEPTH)).is_ok());
        assert!(parse(&nest(DEPTH + 1)).is_err());
        let objects = |n: usize| format!("{}1{}", r#"{"a":"#.repeat(n), "}".repeat(n));
        assert!(parse(&objects(DEPTH)).is_ok());
        assert!(parse(&objects(DEPTH + 1)).is_err());
        assert!(
            parse(&nest(100_000)).is_err(),
            "a deep bomb must not recurse without limit"
        );
        // Nodes: exactly MAX_NODES values are read; one more is not.
        let flat = |n: usize| format!("[{}]", vec!["0"; n - 1].join(","));
        assert!(
            parse(&flat(NODES)).is_ok(),
            "the array and its elements make MAX_NODES"
        );
        assert!(parse(&flat(NODES + 1)).is_err());
        // Strings: MAX_STRING bytes are read, one more is not — keys as well as values.
        let s = |n: usize| format!("\"{}\"", "a".repeat(n));
        assert!(parse(&s(STRING)).is_ok());
        assert!(parse(&s(STRING + 1)).is_err());
        assert!(parse(&format!("{{{}:1}}", s(STRING + 1))).is_err());
        assert!(parse(&format!("{{{}:1}}", s(STRING))).is_ok());
        // Time is linear in the text: 256 KB of short strings is read at once, not in seconds.
        let big = format!("[{}]", vec!["\"abcdefghij\u{4e2d}\""; 15_000].join(","));
        let t = std::time::Instant::now();
        assert!(parse(&big).is_ok());
        assert!(
            t.elapsed() < std::time::Duration::from_secs(2),
            "parsing took {:?}",
            t.elapsed()
        );
        // Multi-byte characters count as their bytes.
        let wide = format!("\"{}\"", "\u{4e2d}".repeat(STRING / 3 + 1));
        assert!(parse(&wide).is_err());
    }
}
