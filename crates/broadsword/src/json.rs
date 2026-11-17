//! Hand-written JSON output helpers (std only). Only what the tool list needs: string escaping.

/// A JSON string literal, quotes included. Escapes `"`, `\` and every control character below
/// U+0020; everything else (including non-ASCII) is passed through as UTF-8, which JSON allows.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
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
    out
}

/// A small strict JSON parser, used only by tests to prove the hand-written output is valid.
#[cfg(test)]
pub mod check {
    #[derive(Debug, Clone, PartialEq)]
    pub enum J {
        Null,
        Bool(bool),
        Num(f64),
        Str(String),
        Arr(Vec<J>),
        Obj(Vec<(String, J)>),
    }

    impl J {
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
    }

    pub fn parse(text: &str) -> Result<J, String> {
        let mut p = P { b: text.as_bytes(), i: 0 };
        p.ws();
        let v = p.value()?;
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
        fn value(&mut self) -> Result<J, String> {
            match self.b.get(self.i) {
                Some(b'{') => {
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
                        m.push((k, self.value()?));
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
                    self.i += 1;
                    let mut a = Vec::new();
                    self.ws();
                    if self.b.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Ok(J::Arr(a));
                    }
                    loop {
                        self.ws();
                        a.push(self.value()?);
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
                Some(b'-' | b'0'..=b'9') => {
                    let s = self.i;
                    while matches!(self.b.get(self.i), Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) {
                        self.i += 1;
                    }
                    std::str::from_utf8(&self.b[s..self.i]).unwrap().parse().map(J::Num).map_err(|_| format!("bad number at byte {s}"))
                }
                _ => Err(format!("unexpected input at byte {}", self.i)),
            }
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
                                let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4).ok_or("short \\u escape")?).map_err(|_| "bad \\u escape")?;
                                self.i += 4;
                                let n = u32::from_str_radix(hex, 16).map_err(|_| "bad \\u escape")?;
                                char::from_u32(n).ok_or("lone surrogate")?
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

    #[test]
    fn parser_accepts_and_rejects() {
        assert!(parse(r#"{"a":[1,2.5,-3e2,true,false,null,"xé\n"],"b":{}}"#).is_ok());
        for bad in ["{", "[1,]", r#"{"a":1,}"#, "\"a\nb\"", r#""\x""#, "01x", "{} {}", ""] {
            assert!(parse(bad).is_err(), "should reject {bad:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        assert_eq!(string("a\"b\\c\nd\té\u{1}"), "\"a\\\"b\\\\c\\nd\\té\\u0001\"");
        assert_eq!(check::parse(&string("q\"\\\n\r\t\u{8}\u{c}\u{1f}é日本")).unwrap(), check::J::Str("q\"\\\n\r\t\u{8}\u{c}\u{1f}é日本".into()));
    }
}
