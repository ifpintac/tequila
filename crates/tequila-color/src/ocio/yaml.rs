//! A YAML subset, enough to read an OpenColorIO config.
//!
//! Hand-written for the same reason `tequila-format` hand-writes its JSON: the
//! file being read is one this project fully specifies the shape of, the subset
//! it uses is small, and a general YAML implementation would bring a dependency
//! tree larger than the renderer for the sake of features no config uses.
//!
//! What a config actually needs:
//!
//! - block mappings and sequences, by indentation;
//! - flow mappings and sequences on one line — `{name: ACES, colorspace: srgb}`
//!   and `[0.9525, 0.0, 0.0]`, both of which OCIO writes constantly;
//! - `!<Tag>` type tags, which are how a transform says what kind it is and
//!   therefore the one piece of YAML syntax this parser cannot skip;
//! - quoted and bare scalars, and block scalars (`|`, `>`) for descriptions.
//!
//! What it deliberately does not support: anchors and aliases, multiple
//! documents, complex keys, and merge keys. None appears in a config written by
//! OCIO itself, and a file using them is reported rather than half-read — see
//! [`Yaml::parse`].

use std::collections::BTreeMap;

/// A parsed YAML value.
#[derive(Clone, Debug, PartialEq)]
pub enum Yaml {
    Scalar(String),
    Sequence(Vec<Yaml>),
    Mapping(BTreeMap<String, Yaml>),
    /// A value with a `!<Tag>` in front of it.
    Tagged(String, Box<Yaml>),
    Null,
}

impl Yaml {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Yaml::Scalar(s) => Some(s),
            Yaml::Tagged(_, inner) => inner.as_str(),
            _ => None,
        }
    }

    pub fn as_f32(&self) -> Option<f32> {
        self.as_str()?.trim().parse().ok()
    }

    pub fn as_sequence(&self) -> &[Yaml] {
        match self {
            Yaml::Sequence(items) => items,
            Yaml::Tagged(_, inner) => inner.as_sequence(),
            _ => &[],
        }
    }

    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self {
            Yaml::Mapping(map) => map.get(key),
            Yaml::Tagged(_, inner) => inner.get(key),
            _ => None,
        }
    }

    /// The `!<Tag>` on this value, if it has one.
    pub fn tag(&self) -> Option<&str> {
        match self {
            Yaml::Tagged(tag, _) => Some(tag),
            _ => None,
        }
    }

    /// A list of numbers, however it was written: `[1, 2, 3]` or a block
    /// sequence. OCIO uses both for matrices in the same file.
    pub fn as_numbers(&self) -> Vec<f32> {
        match self {
            Yaml::Sequence(items) => items.iter().filter_map(|i| i.as_f32()).collect(),
            Yaml::Tagged(_, inner) => inner.as_numbers(),
            Yaml::Scalar(text) => text
                .split([',', ' '])
                .filter(|s| !s.trim().is_empty())
                .filter_map(|s| s.trim().parse().ok())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Parse a whole document.
    pub fn parse(text: &str) -> Result<Yaml, String> {
        let mut lines = Vec::new();
        for (number, raw) in text.lines().enumerate() {
            if raw.contains(" &") || raw.trim_start().starts_with('*') {
                return Err(format!(
                    "line {}: anchors and aliases are not supported",
                    number + 1
                ));
            }
            let trimmed = strip_comment(raw);
            if trimmed.trim().is_empty() {
                continue;
            }
            let indent = trimmed.len() - trimmed.trim_start().len();
            lines.push(Line {
                indent,
                text: trimmed.trim().to_string(),
                number: number + 1,
            });
        }
        let mut cursor = 0usize;
        let value = parse_block(&lines, &mut cursor, 0)?;
        Ok(value)
    }
}

struct Line {
    indent: usize,
    text: String,
    number: usize,
}

/// Strip a trailing `#` comment, respecting quotes.
fn strip_comment(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    for c in line.chars() {
        match quote {
            Some(q) => {
                out.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' {
                    quote = Some(c);
                    out.push(c);
                } else if c == '#' && (previous == ' ' || out.is_empty()) {
                    break;
                } else {
                    out.push(c);
                }
            }
        }
        previous = c;
    }
    out
}

/// Parse the block starting at `cursor`, consuming every line indented at least
/// `indent`.
fn parse_block(lines: &[Line], cursor: &mut usize, indent: usize) -> Result<Yaml, String> {
    if *cursor >= lines.len() {
        return Ok(Yaml::Null);
    }
    if lines[*cursor].text.starts_with("- ") || lines[*cursor].text == "-" {
        parse_sequence(lines, cursor, indent)
    } else {
        parse_mapping(lines, cursor, indent)
    }
}

fn parse_sequence(lines: &[Line], cursor: &mut usize, indent: usize) -> Result<Yaml, String> {
    let mut items = Vec::new();
    while *cursor < lines.len() && lines[*cursor].indent >= indent {
        let line = &lines[*cursor];
        if line.indent > indent {
            return Err(format!("line {}: unexpected indentation", line.number));
        }
        if !(line.text.starts_with("- ") || line.text == "-") {
            break;
        }
        let rest = line.text[1..].trim().to_string();
        *cursor += 1;

        if rest.is_empty() {
            // The item's contents are on the following lines.
            let child_indent = lines.get(*cursor).map(|l| l.indent).unwrap_or(0);
            if child_indent > indent {
                items.push(parse_block(lines, cursor, child_indent)?);
            } else {
                items.push(Yaml::Null);
            }
            continue;
        }

        // `- key: value` starts a mapping whose first key is on this line. The
        // rest of it is indented past the dash, which in YAML is the dash's own
        // column plus two — that offset is why this cannot simply recurse on
        // the line's indent.
        let inline = parse_inline(&rest)?;
        let continues = lines
            .get(*cursor)
            .is_some_and(|l| l.indent > indent && !l.text.starts_with("- "));
        if let Some((tag, first)) = as_pending_mapping(&inline, &rest) {
            let mut map = first;
            if continues {
                let child_indent = lines[*cursor].indent;
                if let Yaml::Mapping(rest_map) = parse_mapping(lines, cursor, child_indent)? {
                    map.extend(rest_map);
                }
            }
            let value = Yaml::Mapping(map);
            items.push(match tag {
                Some(tag) => Yaml::Tagged(tag, Box::new(value)),
                None => value,
            });
        } else {
            items.push(inline);
        }
    }
    Ok(Yaml::Sequence(items))
}

/// When a sequence item begins a mapping, the tag and its first entry.
fn as_pending_mapping(
    value: &Yaml,
    raw: &str,
) -> Option<(Option<String>, BTreeMap<String, Yaml>)> {
    // A bare `!<Tag>` with nothing after it, or `key: value`.
    match value {
        Yaml::Mapping(map) => Some((None, map.clone())),
        Yaml::Tagged(tag, inner) => match &**inner {
            Yaml::Mapping(map) => Some((Some(tag.clone()), map.clone())),
            Yaml::Null => Some((Some(tag.clone()), BTreeMap::new())),
            _ => None,
        },
        _ => {
            let _ = raw;
            None
        }
    }
}

fn parse_mapping(lines: &[Line], cursor: &mut usize, indent: usize) -> Result<Yaml, String> {
    let mut map = BTreeMap::new();
    while *cursor < lines.len() && lines[*cursor].indent >= indent {
        let line = &lines[*cursor];
        if line.indent > indent {
            return Err(format!("line {}: unexpected indentation", line.number));
        }
        if line.text.starts_with("- ") {
            break;
        }
        let Some((key, rest)) = split_key(&line.text) else {
            return Err(format!("line {}: expected `key: value`", line.number));
        };
        let number = line.number;
        *cursor += 1;

        if rest.is_empty() {
            // Value on the following lines, or an empty mapping entry.
            let child_indent = lines.get(*cursor).map(|l| l.indent).unwrap_or(0);
            // A sequence may be written at the *same* indent as its key, which
            // is legal YAML and which OCIO uses for `displays`.
            let child_is_sequence = lines
                .get(*cursor)
                .is_some_and(|l| l.text.starts_with("- ") || l.text == "-");
            if child_indent > indent || (child_is_sequence && child_indent == indent) {
                map.insert(key, parse_block(lines, cursor, child_indent)?);
            } else {
                map.insert(key, Yaml::Null);
            }
            continue;
        }

        if rest == "|" || rest == ">" {
            // A block scalar. Only descriptions use these, so the exact folding
            // rules do not matter — joining the lines preserves the meaning.
            let mut text = String::new();
            let child_indent = lines.get(*cursor).map(|l| l.indent).unwrap_or(0);
            while *cursor < lines.len() && lines[*cursor].indent >= child_indent && child_indent > indent {
                if !text.is_empty() {
                    text.push(if rest == "|" { '\n' } else { ' ' });
                }
                text.push_str(&lines[*cursor].text);
                *cursor += 1;
            }
            map.insert(key, Yaml::Scalar(text));
            continue;
        }

        let inline = parse_inline(&rest)
            .map_err(|e| format!("line {number}: {e}"))?;

        // `key: !<Tag>` with the tagged value's body on the following lines.
        if let Yaml::Tagged(tag, inner) = &inline {
            if **inner == Yaml::Null {
                let child_indent = lines.get(*cursor).map(|l| l.indent).unwrap_or(0);
                let value = if child_indent > indent {
                    parse_block(lines, cursor, child_indent)?
                } else {
                    Yaml::Null
                };
                map.insert(key, Yaml::Tagged(tag.clone(), Box::new(value)));
                continue;
            }
        }
        map.insert(key, inline);
    }
    Ok(Yaml::Mapping(map))
}

/// Split `key: value`, respecting quotes and flow brackets.
fn split_key(text: &str) -> Option<(String, String)> {
    let bytes = text.as_bytes();
    let mut depth = 0i32;
    let mut quote: Option<u8> = None;
    for (i, c) in bytes.iter().enumerate() {
        match quote {
            Some(q) => {
                if *c == q {
                    quote = None;
                }
            }
            None => match c {
                b'"' | b'\'' => quote = Some(*c),
                b'{' | b'[' => depth += 1,
                b'}' | b']' => depth -= 1,
                b':' if depth == 0 => {
                    let after = bytes.get(i + 1);
                    if after.is_none() || after == Some(&b' ') {
                        let key = unquote(text[..i].trim());
                        let rest = text[i + 1..].trim().to_string();
                        return Some((key, rest));
                    }
                }
                _ => {}
            },
        }
    }
    None
}

/// Parse a value written on one line.
fn parse_inline(text: &str) -> Result<Yaml, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Yaml::Null);
    }
    if let Some(rest) = text.strip_prefix("!<") {
        let Some(end) = rest.find('>') else {
            return Err("an unterminated `!<` tag".into());
        };
        let tag = rest[..end].to_string();
        let inner = parse_inline(rest[end + 1..].trim())?;
        return Ok(Yaml::Tagged(tag, Box::new(inner)));
    }
    if let Some(inner) = text.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
        let mut map = BTreeMap::new();
        for part in split_flow(inner) {
            if let Some((key, value)) = split_key(&part) {
                map.insert(key, parse_inline(&value)?);
            }
        }
        return Ok(Yaml::Mapping(map));
    }
    if let Some(inner) = text.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
        let mut items = Vec::new();
        for part in split_flow(inner) {
            items.push(parse_inline(&part)?);
        }
        return Ok(Yaml::Sequence(items));
    }
    Ok(Yaml::Scalar(unquote(text)))
}

/// Split a flow collection's body on commas at depth zero.
fn split_flow(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    for c in text.chars() {
        match quote {
            Some(q) => {
                current.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    current.push(c);
                }
                '{' | '[' => {
                    depth += 1;
                    current.push(c);
                }
                '}' | ']' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 => {
                    parts.push(current.trim().to_string());
                    current = String::new();
                }
                _ => current.push(c),
            },
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts
}

fn unquote(text: &str) -> String {
    let text = text.trim();
    for quote in ['"', '\''] {
        if text.len() >= 2 && text.starts_with(quote) && text.ends_with(quote) {
            return text[1..text.len() - 1].to_string();
        }
    }
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_block_mapping() {
        let y = Yaml::parse("a: 1\nb: hello\n").unwrap();
        assert_eq!(y.get("a").unwrap().as_f32(), Some(1.0));
        assert_eq!(y.get("b").unwrap().as_str(), Some("hello"));
    }

    /// The two forms OCIO writes a list of numbers in, which have to read the
    /// same or a matrix means different things in different files.
    #[test]
    fn a_matrix_reads_the_same_in_either_form() {
        let flow = Yaml::parse("matrix: [1, 0, 0.5, 2]").unwrap();
        let block = Yaml::parse("matrix:\n  - 1\n  - 0\n  - 0.5\n  - 2\n").unwrap();
        assert_eq!(
            flow.get("matrix").unwrap().as_numbers(),
            block.get("matrix").unwrap().as_numbers()
        );
        assert_eq!(flow.get("matrix").unwrap().as_numbers(), vec![1.0, 0.0, 0.5, 2.0]);
    }

    /// `!<Tag>` is the one piece of YAML syntax an OCIO config cannot be read
    /// without: it is how a transform says what kind it is.
    #[test]
    fn tags_survive_in_both_positions() {
        let inline = Yaml::parse("from_reference: !<MatrixTransform> {matrix: [1, 2]}").unwrap();
        let t = inline.get("from_reference").unwrap();
        assert_eq!(t.tag(), Some("MatrixTransform"));
        assert_eq!(t.get("matrix").unwrap().as_numbers(), vec![1.0, 2.0]);

        let block = Yaml::parse(
            "from_reference: !<GroupTransform>\n  children:\n    - !<FileTransform> {src: a.spi1d}\n",
        )
        .unwrap();
        let g = block.get("from_reference").unwrap();
        assert_eq!(g.tag(), Some("GroupTransform"));
        let children = g.get("children").unwrap().as_sequence();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].tag(), Some("FileTransform"));
        assert_eq!(children[0].get("src").unwrap().as_str(), Some("a.spi1d"));
    }

    /// A sequence written at its key's own indent, which is legal YAML and
    /// which OCIO uses for `displays`. Requiring the extra indent would reject
    /// every real config.
    #[test]
    fn a_sequence_may_share_its_keys_indent() {
        let y = Yaml::parse("displays:\n  sRGB:\n  - !<View> {name: ACES, colorspace: out}\n")
            .unwrap();
        let views = y.get("displays").unwrap().get("sRGB").unwrap().as_sequence();
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].get("name").unwrap().as_str(), Some("ACES"));
    }

    /// A sequence of mappings whose first key shares the dash's line — the
    /// shape every `colorspaces` entry has.
    #[test]
    fn a_sequence_item_may_start_its_mapping_on_the_dash() {
        let y = Yaml::parse(
            "colorspaces:\n  - !<ColorSpace>\n    name: ACEScg\n    family: ACES\n  - !<ColorSpace>\n    name: sRGB\n",
        )
        .unwrap();
        let spaces = y.get("colorspaces").unwrap().as_sequence();
        assert_eq!(spaces.len(), 2);
        assert_eq!(spaces[0].get("name").unwrap().as_str(), Some("ACEScg"));
        assert_eq!(spaces[0].get("family").unwrap().as_str(), Some("ACES"));
        assert_eq!(spaces[1].get("name").unwrap().as_str(), Some("sRGB"));
    }

    /// Comments must not eat a `#` inside a quoted string, and must not be
    /// mistaken for one in the middle of a word.
    #[test]
    fn comments_stop_at_quotes() {
        let y = Yaml::parse("a: \"x # y\" # trailing\nb: c#d\n").unwrap();
        assert_eq!(y.get("a").unwrap().as_str(), Some("x # y"));
        assert_eq!(y.get("b").unwrap().as_str(), Some("c#d"));
    }

    /// Refused rather than half-read. A config using anchors would otherwise
    /// load with silently missing colour spaces.
    #[test]
    fn anchors_are_refused_rather_than_ignored() {
        assert!(Yaml::parse("a: &anchor 1\nb: *anchor\n").is_err());
    }
}
