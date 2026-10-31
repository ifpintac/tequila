//! KeyShot scene files (`.bip`), read into a tree of tokens.
//!
//! A `.bip` is a line of text (`//---luxion:dal:020---`), a fixed header
//! that ends at byte 128, then tokens to the end of the file. A token is a
//! tag byte, its payload's size and the payload:
//!
//! ```text
//! token := tag:u8 size payload[size]
//! size  := u8 when the tag is odd, else u32 little-endian
//! type  := tag >> 1
//! ```
//!
//! Some types open a container (a group, a part, a material, a shader node,
//! a camera, a project object) and a matching type closes it; the rest are
//! values. Most values are keyed: the value first, at the size its type
//! gives, and the key is what is left of the payload. Arrays name their key
//! first, with its length. One container, a compressed chunk, holds a zlib
//! stream its size does not cover: it is inflated and read as tokens of its
//! own, then reading carries on after the compressed bytes.
//!
//! Every token says how long it is, so one of a type not known here is
//! skipped by its size and said in a warning, never guessed at; a file cut
//! short or with its containers out of step is read as far as it goes. The
//! grammar was learned from scenes people shared, for opening their own
//! files; nothing of KeyShot's is used.

use std::io::Read;

/// What a KeyShot scene starts with.
pub(crate) const MAGIC: &[u8] = b"//---luxion:dal:";

/// Where the tokens start: after the first line and the fixed header.
const TOKENS: usize = 0x80;

/// Containers deeper than this are not a scene: KeyShot's nest six or seven
/// deep. A file that nests further is refused there rather than walked by
/// code that recurses.
const DEEPEST: usize = 256;

/// The most one compressed chunk may inflate to. The project objects
/// KeyShot compresses are tens of kilobytes; a chunk that grows past this is
/// a zip bomb or a mistake.
const MOST_INFLATED: u64 = 256 << 20;

/// How many warnings of one kind are listed before the rest are counted.
const LISTED: usize = 8;

/// The kinds of container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Camera,
    /// A shader-graph node: a number and a type.
    Node,
    Material,
    SettingsA,
    /// A project object: a number, a class and a name.
    Object,
    SettingsB,
    Part,
    Shape,
    Group,
    Geometry,
    Plugin,
    Chunk,
}

impl Kind {
    /// The container a type opens, and the type that closes it: not always
    /// the next one (a material closes with 7, a part and a shape both with
    /// 0x27).
    fn of(id: u8) -> Option<(Kind, u8)> {
        Some(match id {
            0x01 => (Kind::Camera, 0x02),
            0x03 => (Kind::Node, 0x04),
            0x05 => (Kind::Material, 0x07),
            0x09 => (Kind::SettingsA, 0x0a),
            0x1f => (Kind::Object, 0x20),
            0x23 => (Kind::SettingsB, 0x24),
            0x25 => (Kind::Part, 0x27),
            0x26 => (Kind::Shape, 0x27),
            0x2a => (Kind::Group, 0x2c),
            0x2d => (Kind::Geometry, 0x2e),
            0x35 => (Kind::Plugin, 0x36),
            0x43 => (Kind::Chunk, 0x44),
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Camera => "camera",
            Kind::Node => "shader node",
            Kind::Material => "material",
            Kind::SettingsA | Kind::SettingsB => "settings block",
            Kind::Object => "project object",
            Kind::Part => "part",
            Kind::Shape => "shape",
            Kind::Group => "group",
            Kind::Geometry => "geometry",
            Kind::Plugin => "plugin section",
            Kind::Chunk => "chunk",
        }
    }
}

const ENDS: [u8; 11] = [0x02, 0x04, 0x07, 0x0a, 0x20, 0x24, 0x27, 0x2c, 0x2e, 0x36, 0x44];

/// A value, as its type gives it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    F32(f32),
    F64(f64),
    I32(i32),
    Bool(bool),
    Vec3f([f32; 3]),
    /// Four floats: a camera's window, a colour with alpha.
    Vec4f([f32; 4]),
    Vec3d([f64; 3]),
    /// Sixteen doubles, row by row, for points multiplied on the left.
    Mat4d(Box<[f64; 16]>),
    /// Text; `attr` (metadata) and `str` alike.
    Text(String),
    /// An input driven by a node: `link`.
    Link(u32),
    /// A parameter driven by a texture node: `param_link`.
    ParamLink(u32),
    /// A project object referring to another.
    ObjLink(u32),
    F32s(Vec<f32>),
    F32Pairs(Vec<[f32; 2]>),
    F64s(Vec<f64>),
    Bools(Vec<bool>),
    I32s(Vec<i32>),
    /// The keyless ones.
    MatRoot(u32, u32),
    Id(u32),
    Flags(u32),
    MaterialRef(u32),
    ShapeRef(u32),
    Xform(Box<[f64; 16]>),
    PropName(String),
    PropMatrix(Box<[f64; 16]>),
    Progress,
    /// Raw bytes: the preview picture.
    Blob(Vec<u8>),
}

/// A value and its key (empty for the keyless ones).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Leaf {
    pub key: String,
    pub value: Value,
}

/// A container and what it holds.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Container {
    pub kind: Kind,
    /// Its name: a group's, a part's, a node's type, an object's name.
    pub name: String,
    /// A node's or an object's number.
    pub number: u32,
    /// An object's class (`lux.env.pin`).
    pub class: String,
    pub children: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Item {
    Container(Container),
    Leaf(Leaf),
}

/// The header's fields a reader has use for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Header {
    /// `020`: the serializer's version.
    pub version: String,
    /// The KeyShot that saved it: 11, 3.
    pub application: (u16, u16),
}

/// A file read: its header, its top-level items, and what was wrong.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub header: Header,
    pub items: Vec<Item>,
    pub warnings: Vec<String>,
    /// Tokens read, all told.
    pub tokens: usize,
}

/// Whether the bytes are a KeyShot scene.
pub(crate) fn looks_like(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

/// Read a scene. Refused only when it is not one; anything wrong further in
/// is a warning, and what could be read is kept.
pub(crate) fn parse(bytes: &[u8]) -> Result<Parsed, String> {
    let newline = bytes.iter().take(64).position(|&b| b == b'\n');
    let Some(newline) = newline.filter(|_| looks_like(bytes)) else {
        return Err("it does not start as a KeyShot scene does (//---luxion:dal:…)".into());
    };
    let mut parsed = Parsed::default();
    let line = String::from_utf8_lossy(&bytes[MAGIC.len()..newline]);
    parsed.header.version = line.trim_end_matches('-').to_string();
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    parsed.header.application = (u16_at(0x28).unwrap_or(0), u16_at(0x2a).unwrap_or(0));
    if bytes.len() < TOKENS {
        return Err("the file ends inside its header".into());
    }
    // The header gives the file's length: a shorter file was cut off, which
    // is worth saying before the tokens run out somewhere in the middle.
    if let Some(stated) = u32_at(0x20).filter(|&n| n as usize != bytes.len() && n != 0) {
        parsed.warnings.push(format!("the file is {} bytes where its header says {stated}: it may have been cut short", bytes.len()));
    }
    let mut reader = Reader { warnings: Warnings::default(), tokens: 0 };
    parsed.items = reader.tokens(bytes, TOKENS, 0, "");
    parsed.tokens = reader.tokens;
    parsed.warnings.extend(reader.warnings.finish());
    Ok(parsed)
}

/// Warnings, each kind listed a few times and then counted, so a corrupt
/// file says what is wrong with it in a paragraph rather than a book.
#[derive(Default)]
struct Warnings {
    kinds: Vec<(&'static str, Vec<String>, usize)>,
}

impl Warnings {
    fn push(&mut self, kind: &'static str, text: String) {
        match self.kinds.iter_mut().find(|(k, ..)| *k == kind) {
            Some((_, listed, count)) => {
                *count += 1;
                if listed.len() < LISTED {
                    listed.push(text);
                }
            }
            None => self.kinds.push((kind, vec![text], 1)),
        }
    }

    fn finish(self) -> Vec<String> {
        let mut out = Vec::new();
        for (kind, listed, count) in self.kinds {
            let shown = listed.len();
            out.extend(listed);
            if count > shown {
                out.push(format!("{} more of these ({kind})", count - shown));
            }
        }
        out
    }
}

struct Reader {
    warnings: Warnings,
    tokens: usize,
}

/// An open container, and where a raw chunk's body ends.
struct Open {
    container: Container,
    end: u8,
    body_end: Option<usize>,
}

impl Reader {
    /// The tokens of `buf` from `start` to its end, as items. `inflation` is
    /// how many compressed chunks this is inside, `place` how to say where.
    fn tokens(&mut self, buf: &[u8], start: usize, inflation: usize, place: &str) -> Vec<Item> {
        let mut top: Vec<Item> = Vec::new();
        let mut stack: Vec<Open> = Vec::new();
        let mut at = start;
        let end = buf.len();
        while at < end {
            let offset = at;
            let tag = buf[at];
            let (payload_at, size) = if tag & 1 == 1 {
                match buf.get(at + 1) {
                    Some(&n) => (at + 2, n as usize),
                    None => {
                        self.warnings.push("cut short", format!("the file ends inside a token at {place}{offset:#x}"));
                        break;
                    }
                }
            } else {
                match buf.get(at + 1..at + 5) {
                    Some(b) => (at + 5, u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize),
                    None => {
                        self.warnings.push("cut short", format!("the file ends inside a token at {place}{offset:#x}"));
                        break;
                    }
                }
            };
            let Some(payload) = payload_at.checked_add(size).and_then(|e| buf.get(payload_at..e)) else {
                self.warnings.push(
                    "cut short",
                    format!("a token at {place}{offset:#x} says it is {size} bytes long and the file ends {} bytes after it", end - payload_at.min(end)),
                );
                break;
            };
            at = payload_at + size;
            self.tokens += 1;
            let id = tag >> 1;

            if let Some((kind, closing)) = Kind::of(id) {
                if stack.len() >= DEEPEST {
                    self.warnings.push("too deep", format!("containers nest more than {DEEPEST} deep at {place}{offset:#x}; the rest of the file was not read"));
                    break;
                }
                let mut container = Container { kind, name: String::new(), number: 0, class: String::new(), children: Vec::new() };
                let mut body_end = None;
                match kind {
                    Kind::Node => match payload.get(..4) {
                        Some(n) => {
                            container.number = u32::from_le_bytes([n[0], n[1], n[2], n[3]]);
                            container.name = text(&payload[4..]);
                        }
                        None => self.warnings.push("short header", format!("a shader node at {place}{offset:#x} has no number")),
                    },
                    Kind::Object => {
                        let number = payload.get(..4).map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]));
                        let length = payload.get(4..8).map(|n| u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize);
                        match (number, length) {
                            (Some(number), Some(length)) => {
                                container.number = number;
                                let class_end = (8 + length).min(payload.len());
                                container.class = text(&payload[8..class_end]);
                                container.name = text(&payload[class_end..]);
                            }
                            _ => self.warnings.push("short header", format!("a project object at {place}{offset:#x} has no number or class")),
                        }
                    }
                    Kind::SettingsA | Kind::SettingsB => {}
                    Kind::Chunk => {
                        let header = (payload.len() >= 16).then(|| {
                            let length = u64::from_le_bytes(payload[0..8].try_into().unwrap_or([0; 8]));
                            let compression = u16::from_le_bytes([payload[8], payload[9]]);
                            (length, compression)
                        });
                        let Some((length, compression)) = header else {
                            self.warnings.push("short header", format!("a chunk at {place}{offset:#x} has a {}-byte header, not 16", payload.len()));
                            stack.push(Open { container, end: closing, body_end: None });
                            continue;
                        };
                        let body = usize::try_from(length).ok().and_then(|l| at.checked_add(l));
                        match body {
                            Some(body) if body <= end => {
                                if compression & 1 == 1 {
                                    container.children = self.inflate(&buf[at..body], inflation, offset, place);
                                    at = body;
                                } else {
                                    body_end = Some(body);
                                }
                            }
                            _ => self.warnings.push("cut short", format!("a chunk at {place}{offset:#x} says its body is {length} bytes, past the end of the file")),
                        }
                        if compression & !1 != 0 {
                            self.warnings.push("unknown chunk", format!("a chunk at {place}{offset:#x} uses compression {compression}, not one Tequila knows; it was read as plain"));
                        }
                    }
                    _ => container.name = text(payload),
                }
                stack.push(Open { container, end: closing, body_end });
                continue;
            }

            if ENDS.contains(&id) {
                // The container it closes, wherever it is open: those above
                // it on the stack were never closed.
                let Some(depth) = stack.iter().rposition(|o| o.end == id) else {
                    self.warnings.push("stray end", format!("an end of a container at {place}{offset:#x} closes nothing that is open"));
                    continue;
                };
                while stack.len() > depth + 1 {
                    let open = stack.pop().expect("deeper than depth");
                    self.warnings.push("unclosed", format!("a {} opened inside another was not closed before it (at {place}{offset:#x})", open.container.kind.label()));
                    attach(&mut stack, &mut top, open.container);
                }
                let open = stack.pop().expect("found at depth");
                if let Some(body_end) = open.body_end.filter(|&b| b != offset) {
                    self.warnings.push("chunk length", format!("a chunk ends at {place}{offset:#x} where its header said {body_end:#x}"));
                }
                attach(&mut stack, &mut top, open.container);
                continue;
            }

            let leaf = match leaf(id, payload) {
                Ok(leaf) => leaf,
                Err(why) => {
                    let kind = if why.starts_with("unknown") { "unknown token" } else { "bad token" };
                    self.warnings.push(kind, format!("{why} at {place}{offset:#x}; skipped"));
                    continue;
                }
            };
            match stack.last_mut() {
                Some(open) => open.container.children.push(Item::Leaf(leaf)),
                None => top.push(Item::Leaf(leaf)),
            }
        }
        if !stack.is_empty() {
            self.warnings.push("unclosed", format!("{} containers were still open at the end of {}", stack.len(), if place.is_empty() { "the file" } else { "a compressed chunk" }));
        }
        while let Some(open) = stack.pop() {
            attach(&mut stack, &mut top, open.container);
        }
        top
    }

    /// A compressed chunk's tokens.
    fn inflate(&mut self, compressed: &[u8], inflation: usize, offset: usize, place: &str) -> Vec<Item> {
        if inflation >= 4 {
            self.warnings.push("too deep", format!("a compressed chunk at {place}{offset:#x} is inside four others; it was skipped"));
            return Vec::new();
        }
        let mut raw = Vec::new();
        let read = flate2::read::ZlibDecoder::new(compressed).take(MOST_INFLATED + 1).read_to_end(&mut raw);
        if let Err(e) = read {
            self.warnings.push("bad chunk", format!("a compressed chunk at {place}{offset:#x} did not inflate ({e}); what it held is missing"));
            return Vec::new();
        }
        if raw.len() as u64 > MOST_INFLATED {
            self.warnings.push("bad chunk", format!("a compressed chunk at {place}{offset:#x} inflates past {} MB; it was skipped", MOST_INFLATED >> 20));
            return Vec::new();
        }
        let inner = format!("{place}{offset:#x}+");
        self.tokens(&raw, 0, inflation + 1, &inner)
    }
}

fn attach(stack: &mut [Open], top: &mut Vec<Item>, container: Container) {
    match stack.last_mut() {
        Some(open) => open.container.children.push(Item::Container(container)),
        None => top.push(Item::Container(container)),
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn f32_at(p: &[u8], at: usize) -> f32 {
    f32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}

fn f64_at(p: &[u8], at: usize) -> f64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&p[at..at + 8]);
    f64::from_le_bytes(b)
}

fn u32_at(p: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([p[at], p[at + 1], p[at + 2], p[at + 3]])
}

fn matrix(p: &[u8]) -> Box<[f64; 16]> {
    let mut m = [0.0; 16];
    for (i, v) in m.iter_mut().enumerate() {
        *v = f64_at(p, 8 * i);
    }
    Box::new(m)
}

/// A value token's payload.
fn leaf(id: u8, p: &[u8]) -> Result<Leaf, String> {
    let need = |n: usize, what: &str| -> Result<(), String> {
        if p.len() < n {
            Err(format!("a {what} of {} bytes, which needs {n}", p.len()))
        } else {
            Ok(())
        }
    };
    let exactly = |n: usize, what: &str| -> Result<(), String> {
        if p.len() != n {
            Err(format!("a {what} of {} bytes, which should be {n}", p.len()))
        } else {
            Ok(())
        }
    };
    let keyed = |size: usize, value: Value| Leaf { key: text(&p[size..]), value };
    let bare = |value: Value| Leaf { key: String::new(), value };
    Ok(match id {
        0x0b => {
            need(4, "number")?;
            keyed(4, Value::F32(f32_at(p, 0)))
        }
        0x0d => {
            need(16, "vector")?;
            keyed(16, Value::Vec4f([f32_at(p, 0), f32_at(p, 4), f32_at(p, 8), f32_at(p, 12)]))
        }
        0x0e => {
            need(4, "whole number")?;
            keyed(4, Value::I32(u32_at(p, 0) as i32))
        }
        0x12 => {
            need(4, "switch")?;
            keyed(4, Value::Bool(u32_at(p, 0) != 0))
        }
        0x13 => {
            need(12, "colour")?;
            keyed(12, Value::Vec3f([f32_at(p, 0), f32_at(p, 4), f32_at(p, 8)]))
        }
        0x14 => {
            need(16, "colour")?;
            keyed(16, Value::Vec4f([f32_at(p, 0), f32_at(p, 4), f32_at(p, 8), f32_at(p, 12)]))
        }
        0x15 => {
            need(24, "vector")?;
            keyed(24, Value::Vec3d([f64_at(p, 0), f64_at(p, 8), f64_at(p, 16)]))
        }
        0x16 => {
            need(128, "matrix")?;
            keyed(128, Value::Mat4d(matrix(p)))
        }
        0x1a => {
            need(8, "number")?;
            keyed(8, Value::F64(f64_at(p, 0)))
        }
        0x17 | 0x1e => {
            need(4, "text")?;
            let n = u32_at(p, 0) as usize;
            let end = 4usize.checked_add(n).filter(|&e| e <= p.len()).ok_or_else(|| format!("text of {n} bytes in a {}-byte token", p.len()))?;
            Leaf { key: text(&p[end..]), value: Value::Text(text(&p[4..end])) }
        }
        0x18 | 0x19 | 0x47 => {
            need(4, "link")?;
            let target = u32_at(p, 0);
            keyed(4, match id {
                0x18 => Value::Link(target),
                0x19 => Value::ParamLink(target),
                _ => Value::ObjLink(target),
            })
        }
        0x3a | 0x3c | 0x3d | 0x3e | 0x3f => {
            need(4, "list")?;
            let n = u32_at(p, 0) as usize;
            let start = 4usize.checked_add(n).filter(|&e| e <= p.len()).ok_or_else(|| format!("a list's name of {n} bytes in a {}-byte token", p.len()))?;
            let key = text(&p[4..start]);
            let data = &p[start..];
            let size = match id {
                0x3c | 0x3d => 8,
                _ => 4,
            };
            if data.len() % size != 0 {
                return Err(format!("a list `{key}` of {} bytes, not a whole number of {size}-byte elements", data.len()));
            }
            let value = match id {
                0x3a => Value::F32s(data.chunks_exact(4).map(|c| f32_at(c, 0)).collect()),
                0x3c => Value::F32Pairs(data.chunks_exact(8).map(|c| [f32_at(c, 0), f32_at(c, 4)]).collect()),
                0x3d => Value::F64s(data.chunks_exact(8).map(|c| f64_at(c, 0)).collect()),
                0x3e => Value::Bools(data.chunks_exact(4).map(|c| u32_at(c, 0) != 0).collect()),
                _ => Value::I32s(data.chunks_exact(4).map(|c| u32_at(c, 0) as i32).collect()),
            };
            Leaf { key, value }
        }
        0x08 => {
            exactly(8, "material's root")?;
            bare(Value::MatRoot(u32_at(p, 0), u32_at(p, 4)))
        }
        0x1c | 0x1d | 0x37 | 0x38 => {
            exactly(4, "number")?;
            let n = u32_at(p, 0);
            bare(match id {
                0x1c => Value::Id(n),
                0x1d => Value::Flags(n),
                0x37 => Value::MaterialRef(n),
                _ => Value::ShapeRef(n),
            })
        }
        0x39 | 0x30 => {
            exactly(128, "matrix")?;
            bare(if id == 0x39 { Value::Xform(matrix(p)) } else { Value::PropMatrix(matrix(p)) })
        }
        0x2f => bare(Value::PropName(text(p))),
        // Loading progress: a part's name and how far through the file it
        // is. Nothing of the scene's.
        0x48 => bare(Value::Progress),
        0x1b => {
            need(1, "picture")?;
            bare(Value::Blob(p[1..].to_vec()))
        }
        other => return Err(format!("unknown token type {other:#04x} ({} bytes)", p.len())),
    })
}

// ---- reading what was read ------------------------------------------------------

impl Container {
    pub fn containers(&self) -> impl Iterator<Item = &Container> {
        self.children.iter().filter_map(|i| match i {
            Item::Container(c) => Some(c),
            Item::Leaf(_) => None,
        })
    }

    pub fn leaves(&self) -> impl Iterator<Item = &Leaf> {
        self.children.iter().filter_map(|i| match i {
            Item::Leaf(l) => Some(l),
            Item::Container(_) => None,
        })
    }

    /// The first value under `key` that is not a link: a parameter can be
    /// both a value and driven by a texture, under one key.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.leaves().find(|l| l.key == key && !matches!(l.value, Value::Link(_) | Value::ParamLink(_) | Value::ObjLink(_))).map(|l| &l.value)
    }

    pub fn number(&self, key: &str) -> Option<f64> {
        match self.value(key)? {
            Value::F32(v) => Some(*v as f64),
            Value::F64(v) => Some(*v),
            Value::I32(v) => Some(*v as f64),
            Value::Bool(v) => Some(*v as u8 as f64),
            _ => None,
        }
        .filter(|v| v.is_finite())
    }

    pub fn f32(&self, key: &str) -> Option<f32> {
        self.number(key).map(|v| v as f32)
    }

    pub fn int(&self, key: &str) -> Option<i32> {
        match self.value(key)? {
            Value::I32(v) => Some(*v),
            Value::Bool(v) => Some(*v as i32),
            _ => None,
        }
    }

    pub fn flag(&self, key: &str) -> Option<bool> {
        match self.value(key)? {
            Value::Bool(v) => Some(*v),
            Value::I32(v) => Some(*v != 0),
            _ => None,
        }
    }

    pub fn vec3f(&self, key: &str) -> Option<[f32; 3]> {
        match self.value(key)? {
            Value::Vec3f(v) => Some(*v),
            Value::Vec4f(v) => Some([v[0], v[1], v[2]]),
            _ => None,
        }
        .filter(|v| v.iter().all(|c| c.is_finite()))
    }

    pub fn vec4f(&self, key: &str) -> Option<[f32; 4]> {
        match self.value(key)? {
            Value::Vec4f(v) => Some(*v),
            _ => None,
        }
        .filter(|v| v.iter().all(|c| c.is_finite()))
    }

    pub fn vec3d(&self, key: &str) -> Option<[f64; 3]> {
        match self.value(key)? {
            Value::Vec3d(v) => Some(*v),
            Value::Vec3f(v) => Some(v.map(|c| c as f64)),
            _ => None,
        }
        .filter(|v| v.iter().all(|c| c.is_finite()))
    }

    pub fn text(&self, key: &str) -> Option<&str> {
        match self.value(key)? {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn link(&self, key: &str) -> Option<u32> {
        self.leaves().find_map(|l| match l.value {
            Value::Link(n) if l.key == key => Some(n),
            _ => None,
        })
    }

    pub fn object_link(&self, key: &str) -> Option<u32> {
        self.leaves().find_map(|l| match l.value {
            Value::ObjLink(n) if l.key == key => Some(n),
            _ => None,
        })
    }

    pub fn f64s(&self, key: &str) -> Option<&[f64]> {
        match self.value(key)? {
            Value::F64s(v) => Some(v),
            _ => None,
        }
    }

    pub fn f32s(&self, key: &str) -> Option<&[f32]> {
        match self.value(key)? {
            Value::F32s(v) => Some(v),
            _ => None,
        }
    }

    pub fn pairs(&self, key: &str) -> Option<&[[f32; 2]]> {
        match self.value(key)? {
            Value::F32Pairs(v) => Some(v),
            _ => None,
        }
    }

    pub fn i32s(&self, key: &str) -> Option<&[i32]> {
        match self.value(key)? {
            Value::I32s(v) => Some(v),
            _ => None,
        }
    }

    /// Its keyless number: a part's, a group's, a material's, a camera's.
    pub fn id(&self) -> Option<u32> {
        self.leaves().find_map(|l| match l.value {
            Value::Id(n) => Some(n),
            _ => None,
        })
    }

    pub fn flags(&self) -> Option<u32> {
        self.leaves().find_map(|l| match l.value {
            Value::Flags(n) => Some(n),
            _ => None,
        })
    }

    /// A named matrix property: a group's `group_local_transform`.
    pub fn property(&self, name: &str) -> Option<[f64; 16]> {
        let mut named = false;
        for leaf in self.leaves() {
            match &leaf.value {
                Value::PropName(n) => named = n == name,
                Value::PropMatrix(m) if named => return Some(**m).filter(|m| m.iter().all(|c| c.is_finite())),
                _ => named = false,
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        if tag & 1 == 1 {
            out.push(payload.len() as u8);
        } else {
            out.extend((payload.len() as u32).to_le_bytes());
        }
        out.extend_from_slice(payload);
        out
    }

    fn file(tokens: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"//---luxion:dal:020---\n".to_vec();
        out.resize(TOKENS, 0);
        for t in tokens {
            out.extend(t);
        }
        let length = out.len() as u32;
        out[0x20..0x24].copy_from_slice(&length.to_le_bytes());
        out
    }

    fn keyed(value: &[u8], key: &str) -> Vec<u8> {
        let mut p = value.to_vec();
        p.extend(key.as_bytes());
        p
    }

    /// Both sizes of every value, a group around them, and the keys after
    /// the values; an array's key before its elements.
    #[test]
    fn values_read_in_both_sizes() {
        let mut big = 7.5f32.to_le_bytes().to_vec();
        big.extend(vec![b'k'; 300]);
        let mut list = (4u32).to_le_bytes().to_vec();
        list.extend(b"knot");
        for v in [0.0f64, 0.5, 1.0] {
            list.extend(v.to_le_bytes());
        }
        let bytes = file(&[
            token(0x55, b"Top"),
            token(0x17, &keyed(&2.5f32.to_le_bytes(), "gain")),
            token(0x16, &big),
            token(0x7b, &list),
            token(0x39, &9u32.to_le_bytes()),
            token(0x59, b""),
        ]);
        let parsed = parse(&bytes).unwrap();
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        let Item::Container(group) = &parsed.items[0] else { panic!("a group") };
        assert_eq!((group.kind, group.name.as_str()), (Kind::Group, "Top"));
        assert_eq!(group.f32("gain"), Some(2.5));
        assert_eq!(group.f32(&"k".repeat(300)), Some(7.5));
        assert_eq!(group.f64s("knot"), Some(&[0.0, 0.5, 1.0][..]));
        assert_eq!(group.id(), Some(9));
    }

    /// A type the reader does not know is skipped by its size, said in a
    /// warning, and what follows still reads.
    #[test]
    fn an_unknown_token_is_skipped_by_its_size() {
        let bytes = file(&[token(0x55, b"G"), token(0xf3, b"whatever it is"), token(0x39, &3u32.to_le_bytes()), token(0x59, b"")]);
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.warnings.len(), 1, "{:?}", parsed.warnings);
        assert!(parsed.warnings[0].contains("unknown token type 0x79"), "{}", parsed.warnings[0]);
        let Item::Container(group) = &parsed.items[0] else { panic!("a group") };
        assert_eq!(group.id(), Some(3));
    }

    /// A container closed out of turn closes those opened inside it, and
    /// an end that closes nothing is ignored; both are said.
    #[test]
    fn unbalanced_containers_are_mended_and_said() {
        let bytes = file(&[token(0x55, b"Outer"), token(0x4b, b"Part"), token(0x59, b""), token(0x59, b"")]);
        let parsed = parse(&bytes).unwrap();
        assert_eq!(parsed.items.len(), 1);
        let Item::Container(group) = &parsed.items[0] else { panic!("a group") };
        assert_eq!(group.containers().next().map(|c| c.kind), Some(Kind::Part));
        assert!(parsed.warnings.iter().any(|w| w.contains("not closed")), "{:?}", parsed.warnings);
        assert!(parsed.warnings.iter().any(|w| w.contains("closes nothing")), "{:?}", parsed.warnings);
    }

    /// Every way of cutting a file short, and every byte of it changed in
    /// turn: the reader never panics, and never reads past what it has.
    #[test]
    fn truncated_and_corrupted_files_never_panic() {
        let mut chunk = vec![];
        let objects = {
            use std::io::Write;
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            let mut object = 5u32.to_le_bytes().to_vec();
            object.extend(3u32.to_le_bytes());
            object.extend(b"pinLight");
            z.write_all(&token(0x3f, &object)).unwrap();
            z.write_all(&token(0x17, &keyed(&1.0f32.to_le_bytes(), "gain"))).unwrap();
            z.write_all(&token(0x41, b"")).unwrap();
            z.finish().unwrap()
        };
        chunk.extend((objects.len() as u64).to_le_bytes());
        chunk.extend(1u16.to_le_bytes());
        chunk.extend(0u16.to_le_bytes());
        chunk.extend(0u32.to_le_bytes());
        let mut tokens = vec![token(0x87, &chunk)];
        tokens.push(objects);
        tokens.push(token(0x89, b""));
        tokens.push(token(0x55, b"G"));
        tokens.push(token(0x7b, &{
            let mut l = 2u32.to_le_bytes().to_vec();
            l.extend(b"Pw");
            l.extend(1.0f64.to_le_bytes());
            l
        }));
        tokens.push(token(0x59, b""));
        let bytes = file(&tokens);
        let whole = parse(&bytes).unwrap();
        assert!(whole.warnings.is_empty(), "{:?}", whole.warnings);
        let Item::Container(chunk) = &whole.items[0] else { panic!("a chunk") };
        let Some(object) = chunk.containers().next() else { panic!("the compressed object") };
        assert_eq!((object.class.as_str(), object.name.as_str(), object.f32("gain")), ("pin", "Light", Some(1.0)));
        for cut in 0..bytes.len() {
            let _ = parse(&bytes[..cut]);
        }
        for at in TOKENS..bytes.len() {
            for flip in [0x01u8, 0x80, 0xff] {
                let mut corrupt = bytes.clone();
                corrupt[at] ^= flip;
                let _ = parse(&corrupt);
            }
        }
    }

    #[test]
    fn a_file_that_is_not_a_scene_is_refused() {
        assert!(parse(b"PK\x03\x04 a zip").is_err());
        assert!(parse(b"//---luxion:dal:020---\nshort").is_err());
    }
}
