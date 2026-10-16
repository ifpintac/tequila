//! A small JSON reader, for glTF.
//!
//! Written rather than pulled in because the workspace carries no third-party
//! Rust crates (see `crates/tequila-texture` for the same choice on HDR and
//! EXR), and because a glTF file is *untrusted input*: it arrives from whoever
//! sent the user a model. That makes the parser's failure modes part of its
//! job, not an afterthought —
//!
//! - recursion is capped, so a file of 100,000 open brackets returns an error
//!   instead of overflowing the stack and aborting the process;
//! - every number is finite, since a NaN that reaches a vertex position
//!   poisons the BVH and every ray that touches it;
//! - no input can panic. Malformed bytes are an `Err`.

use std::collections::BTreeMap;
use std::fmt;

/// How deep nesting may go. glTF's own structure is shallow — a handful of
/// levels — so anything beyond this is either a broken writer or an attempt to
/// exhaust the stack.
const MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(map) => map.get(key),
            _ => None,
        }
    }

    pub fn index(&self, i: usize) -> Option<&Value> {
        match self {
            Value::Array(items) => items.get(i),
            _ => None,
        }
    }

    pub fn as_array(&self) -> &[Value] {
        match self {
            Value::Array(items) => items,
            _ => &[],
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        self.as_f64().map(|n| n as f32)
    }

    /// A non-negative integer, which is what every glTF index and count is.
    /// Rejects negatives and fractions rather than truncating them, because a
    /// truncated index silently reads the wrong accessor.
    pub fn as_index(&self) -> Option<usize> {
        let n = self.as_f64()?;
        if n < 0.0 || n.fract() != 0.0 || n > usize::MAX as f64 {
            return None;
        }
        Some(n as usize)
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Convenience for the many `"key": [a, b, c]` number arrays in glTF.
    pub fn as_f32_array(&self) -> Option<Vec<f32>> {
        match self {
            Value::Array(items) => items.iter().map(|v| v.as_f32()).collect(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    pub message: String,
    /// Byte offset where parsing stopped, for a message that points at the
    /// problem rather than just naming it.
    pub offset: usize,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for JsonError {}

pub fn parse(input: &[u8]) -> Result<Value, JsonError> {
    let mut parser = Parser { input, at: 0 };
    parser.skip_whitespace();
    let value = parser.value(0)?;
    parser.skip_whitespace();
    if parser.at != input.len() {
        return Err(parser.error("trailing data after the JSON document"));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a [u8],
    at: usize,
}

impl<'a> Parser<'a> {
    fn error(&self, message: impl Into<String>) -> JsonError {
        JsonError {
            message: message.into(),
            offset: self.at,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.at).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), JsonError> {
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(format!("expected `{}`", byte as char)))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.error(format!("nesting deeper than {MAX_DEPTH} levels")));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal(b"true", Value::Bool(true)),
            Some(b'f') => self.literal(b"false", Value::Bool(false)),
            Some(b'n') => self.literal(b"null", Value::Null),
            Some(_) => self.number(),
            None => Err(self.error("unexpected end of input")),
        }
    }

    fn literal(&mut self, word: &[u8], value: Value) -> Result<Value, JsonError> {
        if self.input[self.at..].starts_with(word) {
            self.at += word.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal"))
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.expect(b'{')?;
        let mut map = BTreeMap::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(Value::Object(map));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            let value = self.value(depth + 1)?;
            map.insert(key, value);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Value::Object(map));
                }
                _ => return Err(self.error("expected `,` or `}`")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(b']') => {
                    self.at += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]`")),
            }
        }
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            let byte = self
                .peek()
                .ok_or_else(|| self.error("unterminated string"))?;
            self.at += 1;
            match byte {
                b'"' => return Ok(out),
                b'\\' => {
                    let escape = self
                        .peek()
                        .ok_or_else(|| self.error("unterminated escape"))?;
                    self.at += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error("unknown escape sequence")),
                    }
                }
                // Control characters are not legal unescaped, and letting them
                // through would put them into node names.
                0x00..=0x1f => return Err(self.error("unescaped control character")),
                _ => {
                    // Multi-byte UTF-8: take the whole sequence at once so the
                    // string stays valid.
                    let start = self.at - 1;
                    let length = utf8_length(byte);
                    if length == 0 || start + length > self.input.len() {
                        return Err(self.error("invalid UTF-8"));
                    }
                    let text = std::str::from_utf8(&self.input[start..start + length])
                        .map_err(|_| self.error("invalid UTF-8"))?;
                    out.push_str(text);
                    self.at = start + length;
                }
            }
        }
    }

    fn unicode_escape(&mut self) -> Result<char, JsonError> {
        let first = self.hex4()?;
        // Surrogate pairs: a high surrogate alone is not a character, and
        // `char::from_u32` would reject it. Astral-plane names (emoji in a
        // part name, say) arrive this way.
        if (0xd800..0xdc00).contains(&first) {
            if !self.input[self.at..].starts_with(b"\\u") {
                return Err(self.error("lone high surrogate"));
            }
            self.at += 2;
            let second = self.hex4()?;
            if !(0xdc00..0xe000).contains(&second) {
                return Err(self.error("invalid low surrogate"));
            }
            let combined = 0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00);
            return char::from_u32(combined).ok_or_else(|| self.error("invalid code point"));
        }
        char::from_u32(first).ok_or_else(|| self.error("invalid code point"))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        if self.at + 4 > self.input.len() {
            return Err(self.error("truncated \\u escape"));
        }
        let mut value = 0u32;
        for _ in 0..4 {
            let digit = (self.input[self.at] as char)
                .to_digit(16)
                .ok_or_else(|| self.error("invalid hex digit"))?;
            value = value * 16 + digit;
            self.at += 1;
        }
        Ok(value)
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.at;
        if self.peek() == Some(b'-') {
            self.at += 1;
        }
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
        ) {
            self.at += 1;
        }
        if start == self.at {
            return Err(self.error("expected a value"));
        }
        let text = std::str::from_utf8(&self.input[start..self.at])
            .map_err(|_| self.error("invalid number"))?;
        let value: f64 = text.parse().map_err(|_| JsonError {
            message: format!("`{text}` is not a number"),
            offset: start,
        })?;
        // JSON has no syntax for these, so they can only arrive from a writer
        // emitting `1e999`. Letting one through puts an infinity into a vertex.
        if !value.is_finite() {
            return Err(JsonError {
                message: format!("`{text}` is not finite"),
                offset: start,
            });
        }
        Ok(Value::Number(value))
    }
}

/// Length in bytes of the UTF-8 sequence starting with `first`, or 0 if it is
/// not a valid leading byte.
fn utf8_length(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_shape_of_a_gltf_header() {
        let value = parse(
            br#"{"asset":{"version":"2.0"},"scene":0,
                "nodes":[{"mesh":0,"name":"Body","translation":[1.0,-2.5,3e2]}]}"#,
        )
        .unwrap();

        assert_eq!(
            value.get("asset").unwrap().get("version").unwrap().as_str(),
            Some("2.0")
        );
        assert_eq!(value.get("scene").unwrap().as_index(), Some(0));
        let node = value.get("nodes").unwrap().index(0).unwrap();
        assert_eq!(node.get("name").unwrap().as_str(), Some("Body"));
        assert_eq!(
            node.get("translation").unwrap().as_f32_array().unwrap(),
            vec![1.0, -2.5, 300.0]
        );
    }

    #[test]
    fn parses_escapes_including_astral_characters() {
        let value = parse(br#"["a\"b\\c\n\u00e9\ud83d\ude00"]"#).unwrap();
        // The surrogate pair is the interesting one: a part named with an
        // emoji arrives as two \u escapes that have to be recombined.
        assert_eq!(
            value.index(0).unwrap().as_str(),
            Some("a\"b\\c\n\u{e9}\u{1f600}")
        );
    }

    #[test]
    fn keeps_multibyte_names_intact() {
        let value = parse("[\"Gehäuse 外殻\"]".as_bytes()).unwrap();
        assert_eq!(value.index(0).unwrap().as_str(), Some("Gehäuse 外殻"));
    }

    /// An index that is negative or fractional is a broken file, and reading it
    /// as a truncated `usize` would quietly address the wrong accessor.
    #[test]
    fn indices_reject_negatives_and_fractions() {
        let value = parse(b"[-1, 2.5, 3, 1e300]").unwrap();
        assert_eq!(value.index(0).unwrap().as_index(), None);
        assert_eq!(value.index(1).unwrap().as_index(), None);
        assert_eq!(value.index(2).unwrap().as_index(), Some(3));
        assert_eq!(value.index(3).unwrap().as_index(), None);
    }

    /// The parser sees files from strangers. None of these may panic, and the
    /// deep one may not overflow the stack either.
    #[test]
    fn malformed_input_is_an_error_and_never_a_panic() {
        let deep = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
        for bad in [
            "".to_string(),
            "{".to_string(),
            "[1,]".to_string(),
            "{\"a\"}".to_string(),
            "{\"a\":}".to_string(),
            "tru".to_string(),
            "\"unterminated".to_string(),
            "\"\\u00\"".to_string(),
            "\"\\ud83d\"".to_string(),
            "1e999".to_string(),
            "01x".to_string(),
            "{} trailing".to_string(),
            deep,
        ] {
            assert!(
                parse(bad.as_bytes()).is_err(),
                "{bad:?} should have been rejected"
            );
        }
    }

    #[test]
    fn accessors_do_not_panic_on_the_wrong_type() {
        let value = parse(b"{\"a\": 1}").unwrap();
        assert_eq!(value.index(0), None);
        assert_eq!(value.get("a").unwrap().as_str(), None);
        assert_eq!(value.get("a").unwrap().as_array(), &[]);
        assert_eq!(value.get("missing"), None);
    }
}
