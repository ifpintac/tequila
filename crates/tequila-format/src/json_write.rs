//! Writing JSON back out.
//!
//! Pretty-printed, because §4.1's whole rationale for splitting materials and
//! geometry into separate members is that "a scene with 500 materials stays
//! diffable" — which minified JSON is not.
//!
//! Deterministic in both key order and number formatting. Keys come from a
//! [`BTreeMap`](std::collections::BTreeMap), so they sort; numbers use the
//! shortest representation that round-trips, so a value read and written
//! unchanged produces the same bytes. Without both, M1's byte-identical
//! save/reopen criterion cannot hold.

use crate::json::Value;

/// A builder for a JSON object, keeping the call sites readable.
#[derive(Default)]
pub struct Object(std::collections::BTreeMap<String, Value>);

impl Object {
    pub fn new() -> Object {
        Object::default()
    }

    pub fn set(&mut self, key: &str, value: impl Into<Value>) -> &mut Object {
        self.0.insert(key.to_string(), value.into());
        self
    }

    /// Set only when there is something to set, so absent data leaves no key
    /// rather than a `null`.
    pub fn set_some(&mut self, key: &str, value: Option<impl Into<Value>>) -> &mut Object {
        if let Some(value) = value {
            self.0.insert(key.to_string(), value.into());
        }
        self
    }

    pub fn build(self) -> Value {
        Value::Object(self.0)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Value {
        Value::String(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Value {
        Value::String(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Value {
        Value::Bool(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Value {
        // Widened through the shortest decimal that round-trips as `f32`, not
        // through `v as f64` — the latter turns 0.1f32 into
        // 0.10000000149011612, which is exactly right and completely unreadable
        // in a file meant to be diffed.
        Value::Number(v.to_string().parse().unwrap_or(v as f64))
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Value {
        Value::Number(v)
    }
}

impl From<u64> for Value {
    fn from(v: u64) -> Value {
        Value::Number(v as f64)
    }
}

impl From<usize> for Value {
    fn from(v: usize) -> Value {
        Value::Number(v as f64)
    }
}

impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Value {
        Value::Array(v)
    }
}

impl From<Vec<f32>> for Value {
    fn from(v: Vec<f32>) -> Value {
        Value::Array(v.into_iter().map(Value::from).collect())
    }
}

/// Serialize with two-space indentation and a trailing newline.
pub fn to_string(value: &Value) -> String {
    let mut out = String::new();
    write_value(&mut out, value, 0);
    out.push('\n');
    out
}

fn write_value(out: &mut String, value: &Value, depth: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(n) => out.push_str(&number(*n)),
        Value::String(s) => write_string(out, s),
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Array(items) => {
            // Arrays of plain numbers stay on one line: a transform written as
            // sixteen lines is unreadable, and these are the bulk of a scene.
            if items.iter().all(|v| matches!(v, Value::Number(_))) {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    write_value(out, item, depth);
                }
                out.push(']');
                return;
            }
            out.push_str("[\n");
            for (i, item) in items.iter().enumerate() {
                indent(out, depth + 1);
                write_value(out, item, depth + 1);
                if i + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, depth);
            out.push(']');
        }
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Object(map) => {
            out.push_str("{\n");
            for (i, (key, item)) in map.iter().enumerate() {
                indent(out, depth + 1);
                write_string(out, key);
                out.push_str(": ");
                write_value(out, item, depth + 1);
                if i + 1 < map.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, depth);
            out.push('}');
        }
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Integers print without a decimal point, so a node ID reads as `12` rather
/// than `12.0` and a hand-edited file stays natural.
fn number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 9e15 {
        format!("{}", n as i64)
    } else {
        // Rust's shortest round-tripping form. `1e300` prints in full rather
        // than in exponent form, which is legal JSON either way.
        let text = format!("{n}");
        if text.contains(['e', 'E']) {
            // JSON has no `inf`/`NaN`, and the parser rejects them, so an
            // exponent is the only special form that reaches here.
            text
        } else {
            text
        }
    }
}

fn write_string(out: &mut String, s: &str) {
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
            // Everything else goes through as UTF-8. Escaping non-ASCII would
            // turn a part named "Gehäuse" into "Gehäuse" in a file whose
            // point is being readable.
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    #[test]
    fn what_is_written_reads_back_the_same() {
        let mut root = Object::new();
        root.set("name", "Housing")
            .set("id", 4u64)
            .set("visible", true)
            .set("scale", vec![1.0f32, 0.5, 0.25])
            .set_some("material", Some("mat_004"))
            .set_some("brep", None::<String>);

        let value = root.build();
        let text = to_string(&value);
        assert_eq!(json::parse(text.as_bytes()).unwrap(), value);
        assert!(!text.contains("brep"), "an absent field left a key behind");
    }

    /// The round trip has to be byte-identical, so parsing and re-writing a
    /// document must reproduce it exactly.
    #[test]
    fn parsing_and_rewriting_is_idempotent() {
        let text = to_string(
            &json::parse(br#"{"a":[1,2,3],"b":{"c":"x"},"d":0.25,"e":[],"f":{},"g":null,"h":-7}"#)
                .unwrap(),
        );
        let again = to_string(&json::parse(text.as_bytes()).unwrap());
        assert_eq!(text, again);
    }

    /// Whole numbers must not gain a `.0`, or every node ID in the file reads
    /// as a float.
    #[test]
    fn integers_print_without_a_decimal_point() {
        assert_eq!(number(12.0), "12");
        assert_eq!(number(-3.0), "-3");
        assert_eq!(number(0.0), "0");
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(-0.25), "-0.25");
    }

    /// An `f32` must not print as its exact `f64` widening. 0.1f32 is really
    /// 0.100000001490116119384765625, and writing that is unreadable and
    /// pointlessly long.
    #[test]
    fn floats_print_at_their_own_precision() {
        let value: Value = 0.1f32.into();
        assert_eq!(to_string(&value).trim(), "0.1");
        let value: Value = 0.3f32.into();
        assert_eq!(to_string(&value).trim(), "0.3");
    }

    #[test]
    fn strings_are_escaped_but_not_over_escaped() {
        let value = Value::String("a\"b\\c\nGehäuse\u{1}".into());
        let text = to_string(&value);
        // The control character must come out as the six-character escape,
        // and the umlaut must not: escaping it would make the file
        // unreadable for no gain.
        assert_eq!(text.trim(), r#""a\"b\\c\nGehäuse\u0001""#);
        assert_eq!(json::parse(text.as_bytes()).unwrap(), value);
    }

    #[test]
    fn number_arrays_stay_on_one_line() {
        let mut root = Object::new();
        root.set("t", vec![0.0f32, 1.0, 2.0]);
        let text = to_string(&root.build());
        assert!(text.contains("[0, 1, 2]"), "{text}");
    }

    /// Keys come out sorted regardless of insertion order, so two writes of the
    /// same content agree.
    #[test]
    fn key_order_does_not_depend_on_insertion_order() {
        let mut a = Object::new();
        a.set("z", 1u64).set("a", 2u64).set("m", 3u64);
        let mut b = Object::new();
        b.set("m", 3u64).set("z", 1u64).set("a", 2u64);
        assert_eq!(to_string(&a.build()), to_string(&b.build()));
    }
}
