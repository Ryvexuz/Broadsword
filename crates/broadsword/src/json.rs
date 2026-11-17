//! Hand-written JSON (std only): output helpers (`string`, `J::to_json`) and a small strict parser
//! (`parse`) used by the MCP server and by the tests that prove the hand-written output is valid.

/// Deepest nesting of arrays/objects `parse` accepts. Bounds the recursion, so hostile input cannot overflow the stack.
pub const MAX_DEPTH: usize = 64;

/// A JSON string literal, quotes included. Escapes `"`, `\` and every control character below
/// U+0020; everything else (including non-ASCII) is passed through as UTF-8, which JSON allows.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    push_string(&mut out, s);
    out
}

fn push_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A parsed JSON value. Objects keep their key order (and any duplicate keys); numbers keep their
/// original, grammar-checked text so a request `id` can be echoed back exactly.
#[derive(Debug, Clone, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    /// The first member of an object with this key.
    pub fn get(&self, key: &str) -> Option<&J> {
        match self {
            J::Obj(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn str(&self) -> Option<&str> {
        match self {
            J::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Compact serialisation: one line, no spaces, so it is safe as one line of a line-delimited protocol.
    pub fn to_json(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            J::Null => out.push_str("null"),
            J::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            J::Num(n) => out.push_str(n),
            J::Str(s) => push_string(out, s),
            J::Arr(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            J::Obj(m) => {
                out.push('{');
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    push_string(out, k);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// Parse exactly one JSON value (RFC 8259, strict): no trailing data, no comments, no trailing commas,
/// no leading zeros, no raw control characters in strings, no lone surrogates, nesting at most `MAX_DEPTH`.
pub fn parse(text: &str) -> Result<J, String> {
    let mut p = P { b: text.as_bytes(), i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.b.len() {
        return Err(format!("trailing data at byte {}", p.i));
    }
    Ok(v)
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> Result<(), String> {
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected `{}` at byte {}", c as char, self.i))
        }
    }

    fn lit(&mut self, word: &str, v: J) -> Result<J, String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(format!("bad literal at byte {}", self.i))
        }
    }

    fn digits(&mut self) -> usize {
        let s = self.i;
        while matches!(self.b.get(self.i), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        self.i - s
    }

    fn number(&mut self) -> Result<J, String> {
        let s = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        match self.b.get(self.i) {
            Some(b'0') => self.i += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(format!("bad number at byte {s}")),
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            if self.digits() == 0 {
                return Err(format!("bad number at byte {s}"));
            }
        }
        if matches!(self.b.get(self.i), Some(b'e' | b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if self.digits() == 0 {
                return Err(format!("bad number at byte {s}"));
            }
        }
        // Only ASCII bytes were consumed, so this slice is valid UTF-8.
        Ok(J::Num(String::from_utf8_lossy(&self.b[s..self.i]).into_owned()))
    }

    fn value(&mut self, depth: usize) -> Result<J, String> {
        match self.b.get(self.i) {
            Some(b'{') => {
                if depth >= MAX_DEPTH {
                    return Err(format!("nesting deeper than {MAX_DEPTH}"));
                }
                self.i += 1;
                let mut m = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(J::Obj(m));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    self.eat(b':')?;
                    self.ws();
                    m.push((k, self.value(depth + 1)?));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(J::Obj(m));
                        }
                        _ => return Err(format!("expected `,` or `}}` at byte {}", self.i)),
                    }
                }
            }
            Some(b'[') => {
                if depth >= MAX_DEPTH {
                    return Err(format!("nesting deeper than {MAX_DEPTH}"));
                }
                self.i += 1;
                let mut a = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(J::Arr(a));
                }
                loop {
                    self.ws();
                    a.push(self.value(depth + 1)?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(J::Arr(a));
                        }
                        _ => return Err(format!("expected `,` or `]` at byte {}", self.i)),
                    }
                }
            }
            Some(b'"') => Ok(J::Str(self.string()?)),
            Some(b't') => self.lit("true", J::Bool(true)),
            Some(b'f') => self.lit("false", J::Bool(false)),
            Some(b'n') => self.lit("null", J::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(format!("unexpected input at byte {}", self.i)),
            None => Err("unexpected end of input".to_string()),
        }
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.b.get(self.i..self.i + 4).ok_or("short \\u escape")?;
        if !h.iter().all(u8::is_ascii_hexdigit) {
            return Err("bad \\u escape".to_string());
        }
        self.i += 4;
        // All four bytes are ASCII hex digits.
        u32::from_str_radix(std::str::from_utf8(h).map_err(|_| "bad \\u escape")?, 16).map_err(|_| "bad \\u escape".to_string())
    }

    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = *self.b.get(self.i).ok_or("unterminated string")?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| "invalid UTF-8 in string".to_string()),
                0..=0x1f => return Err(format!("raw control character in string at byte {}", self.i - 1)),
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or("unterminated escape")?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let n = self.hex4()?;
                            match n {
                                0xD800..=0xDBFF => {
                                    // High surrogate: must be followed by `\u` and a low surrogate.
                                    if self.b.get(self.i..self.i + 2) != Some(b"\\u") {
                                        return Err("lone high surrogate".to_string());
                                    }
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..=0xDFFF).contains(&lo) {
                                        return Err("high surrogate not followed by a low surrogate".to_string());
                                    }
                                    char::from_u32(0x10000 + ((n - 0xD800) << 10) + (lo - 0xDC00)).ok_or("bad surrogate pair")?
                                }
                                0xDC00..=0xDFFF => return Err("lone low surrogate".to_string()),
                                _ => char::from_u32(n).ok_or("bad \\u escape")?,
                            }
                        }
                        _ => return Err(format!("bad escape at byte {}", self.i - 1)),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                c => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> J {
        J::Str(x.to_string())
    }

    #[test]
    fn escapes() {
        assert_eq!(string("a\"b\\c\nd\té\u{1}"), "\"a\\\"b\\\\c\\nd\\té\\u0001\"");
        assert_eq!(parse(&string("q\"\\\n\r\t\u{8}\u{c}\u{1f}é日本")).unwrap(), s("q\"\\\n\r\t\u{8}\u{c}\u{1f}é日本"));
    }

    #[test]
    fn scalars_and_containers() {
        assert_eq!(parse(" null ").unwrap(), J::Null);
        assert_eq!(parse("true").unwrap(), J::Bool(true));
        assert_eq!(parse("false").unwrap(), J::Bool(false));
        assert_eq!(parse("[]").unwrap(), J::Arr(vec![]));
        assert_eq!(parse("{}").unwrap(), J::Obj(vec![]));
        let v = parse(r#"{"a":[1,2.5,-3e2,true,null,"xé\n"],"b":{"c":{}}}"#).unwrap();
        assert_eq!(v.get("a"), Some(&J::Arr(vec![J::Num("1".into()), J::Num("2.5".into()), J::Num("-3e2".into()), J::Bool(true), J::Null, s("xé\n")])));
        assert_eq!(v.get("b").and_then(|b| b.get("c")), Some(&J::Obj(vec![])));
        assert_eq!(v.get("zzz"), None);
        // key order and duplicate keys are preserved; get returns the first
        let d = parse(r#"{"k":1,"j":2,"k":3}"#).unwrap();
        assert_eq!(d.get("k"), Some(&J::Num("1".into())));
        assert_eq!(d.to_json(), r#"{"k":1,"j":2,"k":3}"#);
    }

    #[test]
    fn numbers_keep_their_text_and_follow_the_grammar() {
        for good in ["0", "-0", "7", "-12", "0.5", "10.25", "1e5", "1E+5", "2.5e-3", "-0.0e0", "123456789012345678901234567890"] {
            assert_eq!(parse(good).unwrap(), J::Num(good.to_string()), "{good}");
        }
        for bad in ["01", "-", "-a", "1.", ".5", "1e", "1e+", "+1", "1.2.3", "0x10", "--1", "1 2", "NaN", "Infinity", "-Infinity"] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn string_escapes() {
        assert_eq!(parse(r#""\"\\\/\b\f\n\r\t""#).unwrap(), s("\"\\/\u{8}\u{c}\n\r\t"));
        assert_eq!(parse(r#""\u0041\u00e9\u65E5""#).unwrap(), s("Aé日"));
        assert_eq!(parse(r#""\u0000""#).unwrap(), s("\0"));
        // surrogate pair -> one supplementary-plane scalar
        assert_eq!(parse(r#""\ud83d\ude00""#).unwrap(), s("\u{1F600}"));
        assert_eq!(parse(r#""a\uD834\uDD1Eb""#).unwrap(), s("a\u{1D11E}b"));
        // raw UTF-8 passes through
        assert_eq!(parse("\"日本😀\"").unwrap(), s("日本😀"));
    }

    #[test]
    fn rejects_bad_strings() {
        for bad in [
            r#""\x""#, r#""\u12""#, r#""\u12G4""#, r#""\ud83d""#, r#""\ud83dx""#, r#""\ud83d\u0041""#, r#""\ude00""#, r#""\ud83d\ud83d""#, "\"a\nb\"", "\"a\tb\"", "\"abc", "\"abc\\", "'a'",
            r#""\U0041""#,
        ] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn rejects_malformed_documents() {
        for bad in [
            "", "   ", "{", "}", "[", "]", "[1,]", "[,1]", "[1 2]", r#"{"a":1,}"#, r#"{"a"}"#, r#"{"a":}"#, r#"{a:1}"#, r#"{"a" 1}"#, r#"{1:2}"#, "{} {}", "[] x", "1 2", "nul", "tru", "truee", "True", "// c\n1", "/* c */ 1", "[1,2",
            r#"{"a":[1,2}"#, "\u{feff}1",
        ] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn whitespace_rules() {
        assert!(parse(" \t\r\n[ 1 ,\n2 ]\n").is_ok());
        assert!(parse("\u{a0}1").is_err(), "non-breaking space is not JSON whitespace");
        assert!(parse("[1,\u{b}2]").is_err(), "vertical tab is not JSON whitespace");
    }

    #[test]
    fn depth_limit() {
        let ok = format!("{}{}", "[".repeat(MAX_DEPTH), "]".repeat(MAX_DEPTH));
        assert!(parse(&ok).is_ok());
        let deep = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));
        assert!(parse(&deep).unwrap_err().contains("nesting"));
        let ok_obj = format!("{}1{}", "{\"a\":".repeat(MAX_DEPTH), "}".repeat(MAX_DEPTH));
        assert!(parse(&ok_obj).is_ok());
        let deep_obj = format!("{}1{}", "{\"a\":".repeat(MAX_DEPTH + 1), "}".repeat(MAX_DEPTH + 1));
        assert!(parse(&deep_obj).is_err());
        // a hostile megabyte of open brackets fails fast instead of overflowing the stack
        assert!(parse(&"[".repeat(1_000_000)).is_err());
        assert!(parse(&"{\"a\":".repeat(200_000)).is_err());
    }

    #[test]
    fn to_json_is_compact_and_round_trips() {
        let src = r#" { "id" : "abc" , "n" : [ 1 , -2.50e+3 , true , null ] , "s" : "q\"\u0001é\ud83d\ude00" } "#;
        let v = parse(src).unwrap();
        let out = v.to_json();
        assert!(!out.contains('\n') && !out.contains(": ") && !out.contains(", "));
        assert_eq!(parse(&out).unwrap(), v);
        assert_eq!(out, "{\"id\":\"abc\",\"n\":[1,-2.50e+3,true,null],\"s\":\"q\\\"\\u0001é\u{1F600}\"}");
    }
}
