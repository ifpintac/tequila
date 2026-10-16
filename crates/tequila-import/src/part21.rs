//! ISO 10303-21, the text a STEP file is written in, read into ruststep's
//! syntax tree.
//!
//! ruststep's own parser builds the same tree, and truck reads it from
//! there. It is also where a STEP import spent most of its time: a
//! combinator parser at six or seven megabytes a second, which on the
//! hundreds of megabytes of a large assembly is a minute before any
//! geometry is looked at. This is a single pass over the bytes instead.
//!
//! It also reads two things ruststep refused, failing the whole file over
//! either: a doubled quote inside a string, which is how Part 21 writes an
//! apostrophe (`'Sam''s bracket'`), and an empty list, `()`.

use ruststep::ast::{DataSection, EntityInstance, Name, Parameter, Record, SubSuperRecord};

/// The file's first DATA section.
pub fn data_section(text: &str) -> Result<DataSection, String> {
    let mut reader = Reader { bytes: text.as_bytes(), text, at: 0, depth: 0 };
    loop {
        reader.space()?;
        if reader.at >= reader.bytes.len() {
            return Err("the file has no DATA section".into());
        }
        let word = reader.word();
        if word == "DATA" {
            return reader.data();
        }
        if word.is_empty() && !matches!(reader.peek(), Some(b'#' | b'(' | b'\'' | b';')) {
            return Err(reader.error("a keyword"));
        }
        // HEADER, its entries, ENDSEC, the ISO-10303-21 line: skipped to
        // the end of the statement.
        reader.skip_statement()?;
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    text: &'a str,
    at: usize,
    /// How deep in lists and typed values the reader is. A real file nests a
    /// few levels; a corrupt or hostile one nested tens of thousands deep
    /// ran the reading thread out of stack, which ends the application.
    depth: usize,
}

/// Deeper than any file a CAD package writes, and well inside the stack.
const DEEPEST: usize = 256;

impl<'a> Reader<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn error(&self, wanted: &str) -> String {
        let line = self.bytes[..self.at.min(self.bytes.len())].iter().filter(|&&b| b == b'\n').count() + 1;
        let found: String = self.text.get(self.at..).unwrap_or("").chars().take(24).collect();
        if found.is_empty() {
            format!("line {line}: expected {wanted}, found the end of the file")
        } else {
            format!("line {line}: expected {wanted}, found `{found}`")
        }
    }

    /// Whitespace and `/* comments */`.
    fn space(&mut self) -> Result<(), String> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\r' | b'\n') => self.at += 1,
                Some(b'/') if self.bytes.get(self.at + 1) == Some(&b'*') => {
                    let end = find(&self.bytes[self.at + 2..], b"*/").ok_or_else(|| self.error("the end of a comment"))?;
                    self.at += 2 + end + 2;
                }
                _ => return Ok(()),
            }
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        self.space()?;
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(&format!("`{}`", byte as char)))
        }
    }

    /// Letters, digits, `_` and `-`: a keyword, or a section's name.
    fn word(&mut self) -> &'a str {
        let start = self.at;
        while matches!(self.peek(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-')) {
            self.at += 1;
        }
        &self.text[start..self.at]
    }

    fn skip_statement(&mut self) -> Result<(), String> {
        loop {
            match self.peek() {
                None => return Err(self.error("`;`")),
                Some(b';') => {
                    self.at += 1;
                    return Ok(());
                }
                Some(b'\'') => {
                    self.string()?;
                }
                Some(b'/') if self.bytes.get(self.at + 1) == Some(&b'*') => self.space()?,
                Some(_) => self.at += 1,
            }
        }
    }

    fn data(&mut self) -> Result<DataSection, String> {
        self.space()?;
        let meta = if self.peek() == Some(b'(') {
            self.at += 1;
            self.parameters(b')')?
        } else {
            Vec::new()
        };
        self.expect(b';')?;
        let mut entities = Vec::new();
        loop {
            self.space()?;
            match self.peek() {
                Some(b'#') => entities.push(self.instance()?),
                Some(b'E') if self.word() == "ENDSEC" => {
                    self.expect(b';')?;
                    return Ok(DataSection { meta, entities });
                }
                _ => return Err(self.error("an entity instance or ENDSEC")),
            }
        }
    }

    fn instance(&mut self) -> Result<EntityInstance, String> {
        self.at += 1;
        let id = self.digits().ok_or_else(|| self.error("an entity number"))?;
        self.expect(b'=')?;
        self.space()?;
        let instance = if self.peek() == Some(b'(') {
            self.at += 1;
            let mut records = Vec::new();
            loop {
                self.space()?;
                if self.peek() == Some(b')') {
                    self.at += 1;
                    break;
                }
                records.push(self.record()?);
            }
            EntityInstance::Complex { id, subsuper: SubSuperRecord(records) }
        } else {
            EntityInstance::Simple { id, record: self.record()? }
        };
        self.expect(b';')?;
        Ok(instance)
    }

    fn keyword(&mut self) -> Result<String, String> {
        self.space()?;
        // A user-defined keyword is marked with `!`; ruststep drops the
        // mark, and so does this.
        if self.peek() == Some(b'!') {
            self.at += 1;
        }
        let start = self.at;
        while matches!(self.peek(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')) {
            self.at += 1;
        }
        if start == self.at {
            return Err(self.error("a keyword"));
        }
        Ok(self.text[start..self.at].to_string())
    }

    fn record(&mut self) -> Result<Record, String> {
        let name = self.keyword()?;
        self.expect(b'(')?;
        let parameters = self.parameters(b')')?;
        Ok(Record { name, parameter: Parameter::List(parameters) })
    }

    /// A comma-separated list up to and including `close`.
    fn parameters(&mut self, close: u8) -> Result<Vec<Parameter>, String> {
        let mut list = Vec::new();
        self.space()?;
        if self.peek() == Some(close) {
            self.at += 1;
            return Ok(list);
        }
        loop {
            list.push(self.parameter()?);
            self.space()?;
            match self.peek() {
                Some(b',') => self.at += 1,
                Some(c) if c == close => {
                    self.at += 1;
                    return Ok(list);
                }
                _ => return Err(self.error("`,` or the end of a list")),
            }
        }
    }

    fn parameter(&mut self) -> Result<Parameter, String> {
        if self.depth >= DEEPEST {
            return Err(self.error(&format!("a value nested fewer than {DEEPEST} deep")));
        }
        self.depth += 1;
        let parameter = self.parameter_within();
        self.depth -= 1;
        parameter
    }

    fn parameter_within(&mut self) -> Result<Parameter, String> {
        self.space()?;
        let Some(first) = self.peek() else { return Err(self.error("a parameter")) };
        Ok(match first {
            b'$' => {
                self.at += 1;
                Parameter::NotProvided
            }
            b'*' => {
                self.at += 1;
                Parameter::Omitted
            }
            b'\'' => Parameter::String(self.string()?),
            b'#' | b'@' => {
                self.at += 1;
                let number = self.digits().ok_or_else(|| self.error("an instance number"))?;
                Parameter::Ref(if first == b'#' { Name::Entity(number) } else { Name::Value(number) })
            }
            b'.' => {
                self.at += 1;
                let start = self.at;
                while matches!(self.peek(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_')) {
                    self.at += 1;
                }
                let name = self.text[start..self.at].to_string();
                if name.is_empty() || self.peek() != Some(b'.') {
                    return Err(self.error("an enumeration"));
                }
                self.at += 1;
                Parameter::Enumeration(name)
            }
            b'(' => {
                self.at += 1;
                Parameter::List(self.parameters(b')')?)
            }
            b'"' => {
                // Binary: a hex string. Nothing a mesh needs reads one,
                // and ruststep refuses the file; kept as its digits.
                self.at += 1;
                let start = self.at;
                while !matches!(self.peek(), Some(b'"') | None) {
                    self.at += 1;
                }
                let digits = self.text[start..self.at].to_string();
                self.expect(b'"')?;
                Parameter::String(digits)
            }
            b'+' | b'-' | b'0'..=b'9' => self.number()?,
            b'A'..=b'Z' | b'a'..=b'z' | b'!' => {
                let keyword = self.keyword()?;
                self.expect(b'(')?;
                let parameter = self.parameter()?;
                self.expect(b')')?;
                Parameter::Typed { keyword, parameter: Box::new(parameter) }
            }
            _ => return Err(self.error("a parameter")),
        })
    }

    fn digits(&mut self) -> Option<u64> {
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        self.text[start..self.at].parse().ok()
    }

    /// An integer, or a real: `-12`, `1.`, `1.5E-07`.
    fn number(&mut self) -> Result<Parameter, String> {
        let negative = self.peek() == Some(b'-');
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.at += 1;
            self.space()?;
        }
        let start = self.at;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
        if start == self.at {
            return Err(self.error("a number"));
        }
        let mut real = false;
        if self.peek() == Some(b'.') {
            real = true;
            self.at += 1;
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        if real && matches!(self.peek(), Some(b'E' | b'e')) {
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.at += 1;
            }
        }
        let digits = &self.text[start..self.at];
        let parsed = if real {
            digits.parse::<f64>().map(|v| Parameter::Real(if negative { -v } else { v })).ok()
        } else {
            digits.parse::<i64>().map(|v| Parameter::Integer(if negative { -v } else { v })).ok()
        };
        parsed.ok_or_else(|| self.error("a number"))
    }

    /// A quoted string, a doubled quote inside it standing for one. Other
    /// escapes (`\X2\` and the like) are left for the reader of the name.
    fn string(&mut self) -> Result<String, String> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let rest = &self.bytes[self.at..];
            let Some(quote) = rest.iter().position(|&b| b == b'\'') else {
                return Err(self.error("the end of a string"));
            };
            out.push_str(&self.text[self.at..self.at + quote]);
            self.at += quote + 1;
            if self.peek() == Some(b'\'') {
                out.push('\'');
                self.at += 1;
            } else {
                return Ok(out);
            }
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file nested far deeper than any CAD package writes is refused,
    /// not followed until the stack runs out.
    #[test]
    fn a_value_nested_too_deep_is_an_error_not_a_crash() {
        let deep = format!("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=IFCX({}1{});\nENDSEC;\nEND-ISO-10303-21;\n", "(".repeat(100_000), ")".repeat(100_000));
        let read = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || data_section(&deep).map(|_| ()))
            .unwrap()
            .join()
            .expect("the reader overflowed its stack");
        assert!(read.is_err());
    }

    /// The same tree ruststep builds, for every fixture — which is what
    /// truck reads, so anything else would change the geometry.
    #[test]
    fn the_fixtures_read_as_ruststep_reads_them() {
        for fixture in [
            include_str!("../tests/data/two_bodies.step"),
            include_str!("../tests/data/inches.step"),
            include_str!("../tests/data/assembly.step"),
        ] {
            let theirs = ruststep::parser::parse(fixture).expect("ruststep reads the fixture");
            let ours = data_section(fixture).expect("this reads the fixture");
            assert_eq!(ours, theirs.data[0]);
        }
    }

    #[test]
    fn every_kind_of_parameter() {
        let text = "ISO-10303-21;\nHEADER;\nFILE_NAME('DATA;',/* DATA; */'');\nENDSEC;\n\
                    DATA;\n#1 = A('Sam''s', $, *, .T., #22, @3, -4, 1., -1.5E-07, 2.e3, (1,(2)), \
                    LENGTH_MEASURE(0.5), ());\n\
                    /* between */ #2=( B() !C(#1) );\nENDSEC;\nEND-ISO-10303-21;\n";
        let data = data_section(text).unwrap();
        let EntityInstance::Simple { id: 1, record } = &data.entities[0] else { panic!("{:?}", data.entities[0]) };
        assert_eq!(record.name, "A");
        let Parameter::List(p) = &record.parameter else { panic!() };
        assert_eq!(p[0], Parameter::String("Sam's".into()));
        assert_eq!(p[1], Parameter::NotProvided);
        assert_eq!(p[2], Parameter::Omitted);
        assert_eq!(p[3], Parameter::Enumeration("T".into()));
        assert_eq!(p[4], Parameter::Ref(Name::Entity(22)));
        assert_eq!(p[5], Parameter::Ref(Name::Value(3)));
        assert_eq!(p[6], Parameter::Integer(-4));
        assert_eq!(p[7], Parameter::Real(1.0));
        assert_eq!(p[8], Parameter::Real(-1.5e-7));
        assert_eq!(p[9], Parameter::Real(2000.0));
        assert_eq!(
            p[10],
            Parameter::List(vec![Parameter::Integer(1), Parameter::List(vec![Parameter::Integer(2)])])
        );
        assert_eq!(p[11], Parameter::Typed { keyword: "LENGTH_MEASURE".into(), parameter: Box::new(Parameter::Real(0.5)) });
        assert_eq!(p[12], Parameter::List(vec![]));
        let EntityInstance::Complex { id: 2, subsuper } = &data.entities[1] else { panic!() };
        assert_eq!(subsuper.0.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["B", "C"]);
    }

    #[test]
    fn a_broken_file_says_where() {
        let error = data_section("ISO-10303-21;\nHEADER;\nENDSEC;\nDATA;\n#1=A(1,;\nENDSEC;\n").unwrap_err();
        assert!(error.starts_with("line 5:"), "{error}");
        assert!(data_section("ISO-10303-21;\nHEADER;\nENDSEC;\n").is_err());
        assert!(data_section("DATA;\n#1=A('unterminated);\n").is_err());
    }
}
