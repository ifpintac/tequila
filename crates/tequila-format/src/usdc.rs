//! USD's binary form, the "crate" file (`.usdc`): written as the stage of a
//! USDZ package, and read by the USD importer.
//!
//! A crate file is a layer as a table of *specs*: each a path (`/Scene/Body`,
//! `/Scene/Body.points`), a kind (prim, attribute, relationship) and a set of
//! *fields* (`typeName`, `default`, `specifier`, ...), every name a token. The
//! file is a header, the field values one after another, and then six
//! structural sections - the tokens, the strings, the fields, the field
//! sets, the paths and the specs - found through a table of contents at the
//! end:
//!
//! ```text
//! "PXR-USDC" version(8) tocOffset(8) reserved(64)   the 88-byte header
//! values...                                          arrays, matrices, list ops
//! TOKENS STRINGS FIELDS FIELDSETS PATHS SPECS        structural sections
//! count(8) { name(16) start(8) size(8) }...          the table of contents
//! ```
//!
//! A value is referred to by a 64-bit *rep*: an array bit, an inline bit, a
//! compressed bit, an 8-bit type, and 48 bits that are either the value
//! itself (a token's index, a float's bits) or where in the file it starts.
//!
//! Written as version 0.8.0, what every current USD writes by default.
//! From 0.4.0 the structural sections are compressed: integers as deltas
//! coded in two bits each, then the whole through LZ4. This writer emits
//! LZ4 blocks of literals only - valid LZ4, which any decoder reads - since
//! the sections are small next to the geometry, which is stored plain. The reader decodes real LZ4, the
//! compressed integer and float arrays, and versions from 0.4.0, which is
//! everything written since 2017; older files are refused by name.
//!
//! Checked against Pixar's library as Blender 5.2 bundles it: a file it
//! writes reads here field for field, and a file written here opens in it
//! and passes its USDZ checks (`tests` and `docs/03` §6.3).

use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// `def`, `over` or `class`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Specifier {
    Def,
    Over,
    Class,
}

/// What a spec describes. The numbers are `SdfSpecType`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecKind {
    Attribute,
    Connection,
    Prim,
    PseudoRoot,
    Relationship,
    RelationshipTarget,
    Variant,
    VariantSet,
    Other(u32),
}

impl SpecKind {
    fn code(self) -> u32 {
        match self {
            SpecKind::Attribute => 1,
            SpecKind::Connection => 2,
            SpecKind::Prim => 6,
            SpecKind::PseudoRoot => 7,
            SpecKind::Relationship => 8,
            SpecKind::RelationshipTarget => 9,
            SpecKind::Variant => 10,
            SpecKind::VariantSet => 11,
            SpecKind::Other(n) => n,
        }
    }

    fn from_code(code: u32) -> SpecKind {
        match code {
            1 => SpecKind::Attribute,
            2 => SpecKind::Connection,
            6 => SpecKind::Prim,
            7 => SpecKind::PseudoRoot,
            8 => SpecKind::Relationship,
            9 => SpecKind::RelationshipTarget,
            10 => SpecKind::Variant,
            11 => SpecKind::VariantSet,
            n => SpecKind::Other(n),
        }
    }
}

/// A list edit, as USD writes a relationship's targets, an attribute's
/// connections, a prim's references and applied schemas: an explicit list,
/// or items prepended to (or appended to) what is below. Read, the added,
/// prepended and appended items are kept in that order; deletions and
/// reorderings, which only mean something across layers, are dropped.
#[derive(Clone, Debug, PartialEq)]
pub struct ListOp<T> {
    pub explicit: bool,
    pub items: Vec<T>,
}

impl<T> ListOp<T> {
    pub fn explicit(items: Vec<T>) -> ListOp<T> {
        ListOp { explicit: true, items }
    }

    pub fn prepend(items: Vec<T>) -> ListOp<T> {
        ListOp { explicit: false, items }
    }
}

/// A reference: another layer (`asset`, empty for this one) and the prim in
/// it whose opinions this prim takes.
#[derive(Clone, Debug, PartialEq)]
pub struct Reference {
    pub asset: String,
    pub prim: String,
}

/// A field's value. Floating-point vectors and matrices keep their width in
/// their length: a `float3` is three numbers, a `matrix4d` sixteen, rows of
/// a row-vector matrix as USD writes them.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i32),
    Float(f32),
    Double(f64),
    Token(String),
    String(String),
    Asset(String),
    /// `float2` to `float4`, `color3f`, `normal3f`, ...
    Vecf(Vec<f32>),
    /// `double2` to `double4`, and the matrices.
    Vecd(Vec<f64>),
    Ints(Vec<i32>),
    Floats(Vec<f32>),
    Doubles(Vec<f64>),
    /// An array of float vectors, `width` numbers each: `point3f[]`,
    /// `texCoord2f[]`.
    Vecfs(usize, Vec<f32>),
    /// An array of double vectors, `width` numbers each.
    Vecds(usize, Vec<f64>),
    /// A `token[]` attribute's value.
    Tokens(Vec<String>),
    /// A `string[]` or `asset[]` attribute's value.
    Strings(Vec<String>),
    /// A spec's list of names: `primChildren`, `properties`.
    TokenVector(Vec<String>),
    Specifier(Specifier),
    /// Whether an attribute is `uniform`.
    Variability(bool),
    Paths(ListOp<String>),
    TokenList(ListOp<String>),
    References(ListOp<Reference>),
    /// A value of a kind this reader does not decode (time samples,
    /// dictionaries, variant selections), by its type number.
    Other(u8),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub path: String,
    pub kind: SpecKind,
    pub fields: Vec<(String, Value)>,
}

impl Spec {
    pub fn field(&self, name: &str) -> Option<&Value> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// Set a field, replacing it if it is there.
    pub fn set(&mut self, name: &str, value: Value) -> &mut Spec {
        match self.fields.iter_mut().find(|(n, _)| n == name) {
            Some(field) => field.1 = value,
            None => self.fields.push((name.to_string(), value)),
        }
        self
    }
}

/// A layer: the stage's own metadata on the pseudo-root `/`, and a spec for
/// each prim and property. Written, `primChildren` and `properties` are
/// made from the specs' paths, in the order the specs were added, so a
/// builder adds prims and properties and nothing else.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub specs: Vec<Spec>,
}

impl Default for Layer {
    fn default() -> Self {
        Layer::new()
    }
}

impl Layer {
    pub fn new() -> Layer {
        Layer { specs: vec![Spec { path: "/".into(), kind: SpecKind::PseudoRoot, fields: Vec::new() }] }
    }

    /// The pseudo-root, where the stage's metadata goes.
    pub fn root(&mut self) -> &mut Spec {
        if !self.specs.iter().any(|s| s.kind == SpecKind::PseudoRoot) {
            self.specs.insert(0, Spec { path: "/".into(), kind: SpecKind::PseudoRoot, fields: Vec::new() });
        }
        self.specs.iter_mut().find(|s| s.kind == SpecKind::PseudoRoot).expect("just made")
    }

    pub fn spec(&self, path: &str) -> Option<&Spec> {
        self.specs.iter().find(|s| s.path == path)
    }

    /// A prim, `def`ined (or `class` for a prototype) with a type.
    pub fn prim(&mut self, path: &str, specifier: Specifier, type_name: &str) -> &mut Spec {
        let mut fields = vec![("specifier".to_string(), Value::Specifier(specifier))];
        if !type_name.is_empty() {
            fields.push(("typeName".into(), Value::Token(type_name.into())));
        }
        self.specs.push(Spec { path: path.into(), kind: SpecKind::Prim, fields });
        self.specs.last_mut().expect("just pushed")
    }

    /// An attribute on `prim`: its USD type (`point3f[]`) and, perhaps, a
    /// value.
    pub fn attribute(&mut self, prim: &str, name: &str, type_name: &str, value: Option<Value>) -> &mut Spec {
        let mut fields = vec![("typeName".to_string(), Value::Token(type_name.into()))];
        if let Some(value) = value {
            fields.push(("default".into(), value));
        }
        self.specs.push(Spec { path: format!("{prim}.{name}"), kind: SpecKind::Attribute, fields });
        self.specs.last_mut().expect("just pushed")
    }

    /// An attribute connected to another's output: a shader input fed by a
    /// texture, a material's surface by its shader.
    pub fn connection(&mut self, prim: &str, name: &str, type_name: &str, source: &str) -> &mut Spec {
        let spec = self.attribute(prim, name, type_name, None);
        spec.set("connectionPaths", Value::Paths(ListOp::explicit(vec![source.to_string()])));
        spec
    }

    /// A relationship, `material:binding` say, to `targets`.
    pub fn relationship(&mut self, prim: &str, name: &str, targets: Vec<String>) -> &mut Spec {
        self.specs.push(Spec {
            path: format!("{prim}.{name}"),
            kind: SpecKind::Relationship,
            fields: vec![("variability".into(), Value::Variability(true)), ("targetPaths".into(), Value::Paths(ListOp::explicit(targets)))],
        });
        self.specs.last_mut().expect("just pushed")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CrateError(pub String);

impl fmt::Display for CrateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CrateError {}

fn bad(message: impl Into<String>) -> CrateError {
    CrateError(message.into())
}

// ---- the file's numbers -------------------------------------------------------

const MAGIC: &[u8; 8] = b"PXR-USDC";
const VERSION: [u8; 3] = [0, 8, 0];
const HEADER: usize = 88;
const SECTIONS: [&str; 6] = ["TOKENS", "STRINGS", "FIELDS", "FIELDSETS", "PATHS", "SPECS"];

const ARRAY_BIT: u64 = 1 << 63;
const INLINE_BIT: u64 = 1 << 62;
const COMPRESSED_BIT: u64 = 1 << 61;
const PAYLOAD: u64 = (1 << 48) - 1;

// `Usd_CrateDataTypes`' numbers, the ones read or written here.
mod ty {
    pub const BOOL: u8 = 1;
    pub const UCHAR: u8 = 2;
    pub const INT: u8 = 3;
    pub const UINT: u8 = 4;
    pub const INT64: u8 = 5;
    pub const UINT64: u8 = 6;
    pub const HALF: u8 = 7;
    pub const FLOAT: u8 = 8;
    pub const DOUBLE: u8 = 9;
    pub const STRING: u8 = 10;
    pub const TOKEN: u8 = 11;
    pub const ASSET: u8 = 12;
    pub const MATRIX2D: u8 = 13;
    pub const MATRIX3D: u8 = 14;
    pub const MATRIX4D: u8 = 15;
    pub const QUATD: u8 = 16;
    pub const QUATF: u8 = 17;
    pub const QUATH: u8 = 18;
    pub const VEC2D: u8 = 19;
    pub const VEC2F: u8 = 20;
    pub const VEC2H: u8 = 21;
    pub const VEC2I: u8 = 22;
    pub const VEC3D: u8 = 23;
    pub const VEC3F: u8 = 24;
    pub const VEC3H: u8 = 25;
    pub const VEC3I: u8 = 26;
    pub const VEC4D: u8 = 27;
    pub const VEC4F: u8 = 28;
    pub const VEC4H: u8 = 29;
    pub const VEC4I: u8 = 30;
    pub const TOKEN_LIST_OP: u8 = 32;
    pub const PATH_LIST_OP: u8 = 34;
    pub const REFERENCE_LIST_OP: u8 = 35;
    pub const PATH_VECTOR: u8 = 40;
    pub const TOKEN_VECTOR: u8 = 41;
    pub const SPECIFIER: u8 = 42;
    pub const PERMISSION: u8 = 43;
    pub const VARIABILITY: u8 = 44;
    pub const DOUBLE_VECTOR: u8 = 48;
    pub const STRING_VECTOR: u8 = 50;
}

// A list op's header bits.
const LIST_EXPLICIT: u8 = 1;
const LIST_HAS_EXPLICIT: u8 = 1 << 1;
const LIST_HAS_ADDED: u8 = 1 << 2;
const LIST_HAS_DELETED: u8 = 1 << 3;
const LIST_HAS_ORDERED: u8 = 1 << 4;
const LIST_HAS_PREPENDED: u8 = 1 << 5;
const LIST_HAS_APPENDED: u8 = 1 << 6;

// ---- LZ4 and the integer coding -----------------------------------------------

/// `TfFastCompression` of `data`: a zero byte (one chunk) and an LZ4 block
/// of literals only.
fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 255 + 16);
    out.push(0);
    let n = data.len();
    if n < 15 {
        out.push((n as u8) << 4);
    } else {
        out.push(0xF0);
        let mut rest = n - 15;
        while rest >= 255 {
            out.push(255);
            rest -= 255;
        }
        out.push(rest as u8);
    }
    out.extend_from_slice(data);
    out
}

/// One LZ4 block, decoded.
fn lz4_block(src: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<(), CrateError> {
    let start = out.len();
    let mut i = 0;
    let byte = |i: usize| src.get(i).copied().ok_or_else(|| bad("an LZ4 block ends early"));
    while i < src.len() {
        let token = byte(i)?;
        i += 1;
        let mut literals = (token >> 4) as usize;
        if literals == 15 {
            loop {
                let b = byte(i)?;
                i += 1;
                literals += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        let end = i.checked_add(literals).filter(|&e| e <= src.len()).ok_or_else(|| bad("an LZ4 block's literals run past it"))?;
        out.extend_from_slice(&src[i..end]);
        i = end;
        if i >= src.len() {
            break;
        }
        let offset = byte(i)? as usize | (byte(i + 1)? as usize) << 8;
        i += 2;
        let mut length = (token & 15) as usize;
        if length == 15 {
            loop {
                let b = byte(i)?;
                i += 1;
                length += b as usize;
                if b != 255 {
                    break;
                }
            }
        }
        length += 4;
        if offset == 0 || offset > out.len() - start {
            return Err(bad("an LZ4 match reaches before its block"));
        }
        if out.len() - start + length > limit {
            return Err(bad("an LZ4 block decodes past its stated size"));
        }
        let from = out.len() - offset;
        for k in 0..length {
            let b = out[from + k];
            out.push(b);
        }
    }
    if out.len() - start > limit {
        return Err(bad("an LZ4 block decodes past its stated size"));
    }
    Ok(())
}

/// The inverse of `TfFastCompression`, to at most `limit` bytes.
fn decompress(data: &[u8], limit: usize) -> Result<Vec<u8>, CrateError> {
    let (&chunks, mut rest) = data.split_first().ok_or_else(|| bad("an empty compressed block"))?;
    let mut out = Vec::with_capacity(limit.min(1 << 26));
    if chunks == 0 {
        lz4_block(rest, &mut out, limit)?;
    } else {
        for _ in 0..chunks {
            if rest.len() < 4 {
                return Err(bad("a compressed chunk ends early"));
            }
            let size = i32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]).max(0) as usize;
            let chunk = rest.get(4..4 + size).ok_or_else(|| bad("a compressed chunk runs past its block"))?;
            let room = limit.saturating_sub(out.len());
            lz4_block(chunk, &mut out, room)?;
            rest = &rest[4 + size..];
        }
    }
    Ok(out)
}

/// `Usd_IntegerCompression` of 32-bit integers: the commonest difference
/// from the one before, two bits for each saying whether it is that or how
/// many bytes it takes, then those bytes.
fn encode_ints(values: &[i32]) -> Vec<u8> {
    let mut deltas = Vec::with_capacity(values.len());
    let mut previous = 0i32;
    for &v in values {
        deltas.push(v.wrapping_sub(previous));
        previous = v;
    }
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for &d in &deltas {
        *counts.entry(d).or_default() += 1;
    }
    // The commonest; among equals, the smallest, so the output is fixed.
    let common = counts.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(d, _)| *d).unwrap_or(0);
    let mut codes = vec![0u8; (values.len() * 2).div_ceil(8)];
    let mut tail = Vec::new();
    for (i, &d) in deltas.iter().enumerate() {
        let code = if d == common {
            0
        } else if i8::try_from(d).is_ok() {
            tail.push(d as i8 as u8);
            1
        } else if i16::try_from(d).is_ok() {
            tail.extend_from_slice(&(d as i16).to_le_bytes());
            2
        } else {
            tail.extend_from_slice(&d.to_le_bytes());
            3
        };
        codes[i / 4] |= code << (2 * (i % 4));
    }
    let mut out = common.to_le_bytes().to_vec();
    out.extend_from_slice(&codes);
    out.extend_from_slice(&tail);
    out
}

/// The coding above read back, for 32-bit (`wide` false) or 64-bit integers.
fn decode_ints(buffer: &[u8], count: usize, wide: bool) -> Result<Vec<i64>, CrateError> {
    let short = || bad("a run of compressed integers ends early");
    let common_size = if wide { 8 } else { 4 };
    let codes_size = (count * 2).div_ceil(8);
    if buffer.len() < common_size + codes_size {
        return Err(short());
    }
    let common = if wide {
        i64::from_le_bytes(buffer[0..8].try_into().expect("eight bytes"))
    } else {
        i32::from_le_bytes(buffer[0..4].try_into().expect("four bytes")) as i64
    };
    let codes = &buffer[common_size..common_size + codes_size];
    let mut at = common_size + codes_size;
    let mut out = Vec::with_capacity(count);
    let mut previous = 0i64;
    for i in 0..count {
        let code = (codes[i / 4] >> (2 * (i % 4))) & 3;
        let take = |at: &mut usize, n: usize| -> Result<&[u8], CrateError> {
            let s = buffer.get(*at..*at + n).ok_or_else(short)?;
            *at += n;
            Ok(s)
        };
        let delta = match (code, wide) {
            (0, _) => common,
            (1, false) => take(&mut at, 1)?[0] as i8 as i64,
            (2, false) => i16::from_le_bytes(take(&mut at, 2)?.try_into().expect("two")) as i64,
            (3, false) => i32::from_le_bytes(take(&mut at, 4)?.try_into().expect("four")) as i64,
            (1, true) => i16::from_le_bytes(take(&mut at, 2)?.try_into().expect("two")) as i64,
            (2, true) => i32::from_le_bytes(take(&mut at, 4)?.try_into().expect("four")) as i64,
            _ => i64::from_le_bytes(take(&mut at, 8)?.try_into().expect("eight")),
        };
        previous = if wide { previous.wrapping_add(delta) } else { (previous as i32).wrapping_add(delta as i32) as i64 };
        out.push(previous);
    }
    Ok(out)
}

// ---- writing --------------------------------------------------------------------

/// Paths as a tree: each node an element under its parent.
#[derive(Default)]
struct PathTree {
    /// (parent, element, is a property) for each path after the root.
    nodes: Vec<(usize, String, bool)>,
    children: Vec<Vec<usize>>,
    index: HashMap<String, usize>,
}

impl PathTree {
    fn new() -> PathTree {
        let mut tree = PathTree::default();
        tree.nodes.push((0, String::new(), false));
        tree.children.push(Vec::new());
        tree.index.insert("/".into(), 0);
        tree
    }

    /// The index of `path`, adding it and its ancestors.
    fn add(&mut self, path: &str) -> usize {
        if let Some(&i) = self.index.get(path) {
            return i;
        }
        // A property is after the last prim element's dot.
        let (parent, element, property) = match path.rfind(['/', '.']) {
            Some(at) if path.as_bytes()[at] == b'.' && path[..at].rfind('/').is_some() => (&path[..at], &path[at + 1..], true),
            Some(0) => ("/", &path[1..], false),
            Some(at) => (&path[..at], &path[at + 1..], false),
            None => ("/", path, false),
        };
        let parent = self.add(parent);
        let i = self.nodes.len();
        self.nodes.push((parent, element.to_string(), property));
        self.children.push(Vec::new());
        self.children[parent].push(i);
        self.index.insert(path.to_string(), i);
        i
    }
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
    tokens: Vec<String>,
    token_index: HashMap<String, u32>,
    strings: Vec<u32>,
    string_index: HashMap<String, u32>,
    paths: PathTree,
}

impl Writer {
    fn token(&mut self, text: &str) -> u32 {
        if let Some(&i) = self.token_index.get(text) {
            return i;
        }
        let i = self.tokens.len() as u32;
        self.tokens.push(text.to_string());
        self.token_index.insert(text.to_string(), i);
        i
    }

    fn string(&mut self, text: &str) -> u32 {
        if let Some(&i) = self.string_index.get(text) {
            return i;
        }
        let token = self.token(text);
        let i = self.strings.len() as u32;
        self.strings.push(token);
        self.string_index.insert(text.to_string(), i);
        i
    }

    fn path(&mut self, path: &str) -> u32 {
        self.paths.add(path) as u32
    }

    fn u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    fn rep(kind: u8, array: bool, inline: bool, payload: u64) -> u64 {
        (kind as u64) << 48 | if array { ARRAY_BIT } else { 0 } | if inline { INLINE_BIT } else { 0 } | (payload & PAYLOAD)
    }

    /// Where the next value goes.
    fn here(&self) -> u64 {
        self.out.len() as u64
    }

    fn list_op<T>(&mut self, kind: u8, op: &ListOp<T>, mut item: impl FnMut(&mut Writer, &T)) -> u64 {
        let at = self.here();
        let bits = if op.explicit { LIST_EXPLICIT | LIST_HAS_EXPLICIT } else { LIST_HAS_PREPENDED };
        self.out.push(bits);
        self.u64(op.items.len() as u64);
        for i in &op.items {
            item(self, i);
        }
        Writer::rep(kind, false, false, at)
    }

    fn value(&mut self, value: &Value) -> u64 {
        match value {
            Value::Bool(b) => Writer::rep(ty::BOOL, false, true, *b as u64),
            Value::Int(i) => Writer::rep(ty::INT, false, true, *i as u32 as u64),
            Value::Float(f) => Writer::rep(ty::FLOAT, false, true, f.to_bits() as u64),
            Value::Double(d) => {
                if (*d as f32) as f64 == *d {
                    Writer::rep(ty::DOUBLE, false, true, (*d as f32).to_bits() as u64)
                } else {
                    let at = self.here();
                    self.out.extend_from_slice(&d.to_le_bytes());
                    Writer::rep(ty::DOUBLE, false, false, at)
                }
            }
            Value::Token(t) => {
                let i = self.token(t);
                Writer::rep(ty::TOKEN, false, true, i as u64)
            }
            Value::String(s) => {
                let i = self.string(s);
                Writer::rep(ty::STRING, false, true, i as u64)
            }
            Value::Asset(a) => {
                let i = self.token(a);
                Writer::rep(ty::ASSET, false, true, i as u64)
            }
            Value::Vecf(v) => {
                let kind = match v.len() {
                    2 => ty::VEC2F,
                    3 => ty::VEC3F,
                    _ => ty::VEC4F,
                };
                let at = self.here();
                for k in 0..v.len().clamp(2, 4) {
                    self.out.extend_from_slice(&v.get(k).copied().unwrap_or(0.0).to_le_bytes());
                }
                Writer::rep(kind, false, false, at)
            }
            Value::Vecd(v) => {
                let (kind, n) = match v.len() {
                    2 => (ty::VEC2D, 2),
                    3 => (ty::VEC3D, 3),
                    4 => (ty::VEC4D, 4),
                    9 => (ty::MATRIX3D, 9),
                    _ => (ty::MATRIX4D, 16),
                };
                let at = self.here();
                for k in 0..n {
                    self.out.extend_from_slice(&v.get(k).copied().unwrap_or(0.0).to_le_bytes());
                }
                Writer::rep(kind, false, false, at)
            }
            Value::Ints(v) => {
                let at = self.here();
                self.u64(v.len() as u64);
                for i in v {
                    self.out.extend_from_slice(&i.to_le_bytes());
                }
                Writer::rep(ty::INT, true, false, at)
            }
            Value::Floats(v) => {
                let at = self.here();
                self.u64(v.len() as u64);
                for f in v {
                    self.out.extend_from_slice(&f.to_le_bytes());
                }
                Writer::rep(ty::FLOAT, true, false, at)
            }
            Value::Doubles(v) => {
                let at = self.here();
                self.u64(v.len() as u64);
                for f in v {
                    self.out.extend_from_slice(&f.to_le_bytes());
                }
                Writer::rep(ty::DOUBLE, true, false, at)
            }
            Value::Vecfs(width, v) => {
                let kind = match width {
                    2 => ty::VEC2F,
                    3 => ty::VEC3F,
                    _ => ty::VEC4F,
                };
                let at = self.here();
                self.u64((v.len() / (*width).max(1)) as u64);
                for f in v {
                    self.out.extend_from_slice(&f.to_le_bytes());
                }
                Writer::rep(kind, true, false, at)
            }
            Value::Vecds(width, v) => {
                let kind = match width {
                    2 => ty::VEC2D,
                    3 => ty::VEC3D,
                    _ => ty::VEC4D,
                };
                let at = self.here();
                self.u64((v.len() / (*width).max(1)) as u64);
                for f in v {
                    self.out.extend_from_slice(&f.to_le_bytes());
                }
                Writer::rep(kind, true, false, at)
            }
            Value::Tokens(v) => {
                let indices: Vec<u32> = v.iter().map(|t| self.token(t)).collect();
                let at = self.here();
                self.u64(indices.len() as u64);
                for i in indices {
                    self.u32(i);
                }
                Writer::rep(ty::TOKEN, true, false, at)
            }
            Value::Strings(v) => {
                let indices: Vec<u32> = v.iter().map(|s| self.string(s)).collect();
                let at = self.here();
                self.u64(indices.len() as u64);
                for i in indices {
                    self.u32(i);
                }
                Writer::rep(ty::STRING, true, false, at)
            }
            Value::TokenVector(v) => {
                let indices: Vec<u32> = v.iter().map(|t| self.token(t)).collect();
                let at = self.here();
                self.u64(indices.len() as u64);
                for i in indices {
                    self.u32(i);
                }
                Writer::rep(ty::TOKEN_VECTOR, false, false, at)
            }
            Value::Specifier(s) => Writer::rep(
                ty::SPECIFIER,
                false,
                true,
                match s {
                    Specifier::Def => 0,
                    Specifier::Over => 1,
                    Specifier::Class => 2,
                },
            ),
            Value::Variability(uniform) => Writer::rep(ty::VARIABILITY, false, true, *uniform as u64),
            Value::Paths(op) => {
                let indices: Vec<u32> = op.items.iter().map(|p| self.path(p)).collect();
                let op = ListOp { explicit: op.explicit, items: indices };
                self.list_op(ty::PATH_LIST_OP, &op, |w, i| w.u32(*i))
            }
            Value::TokenList(op) => {
                let indices: Vec<u32> = op.items.iter().map(|t| self.token(t)).collect();
                let op = ListOp { explicit: op.explicit, items: indices };
                self.list_op(ty::TOKEN_LIST_OP, &op, |w, i| w.u32(*i))
            }
            Value::References(op) => {
                let items: Vec<(u32, u32)> = op.items.iter().map(|r| (self.string(&r.asset), self.path(&r.prim))).collect();
                let op = ListOp { explicit: op.explicit, items };
                self.list_op(ty::REFERENCE_LIST_OP, &op, |w, (asset, prim)| {
                    w.u32(*asset);
                    w.u32(*prim);
                    // No layer offset (an offset of 0, a scale of 1), no
                    // custom data (an empty dictionary).
                    w.out.extend_from_slice(&0f64.to_le_bytes());
                    w.out.extend_from_slice(&1f64.to_le_bytes());
                    w.u64(0);
                })
            }
            Value::Other(_) => Writer::rep(ty::BOOL, false, true, 0),
        }
    }

    /// A compressed run of integers as a section holds them: its size, then
    /// it.
    fn ints(&mut self, values: &[i32]) {
        let packed = compress(&encode_ints(values));
        self.u64(packed.len() as u64);
        self.out.extend_from_slice(&packed);
    }
}

/// The parent of a spec's path, and whether the spec is a property of it.
fn parent_of(path: &str) -> Option<(&str, bool)> {
    if path == "/" {
        return None;
    }
    let slash = path.rfind('/')?;
    match path[slash..].find('.') {
        Some(dot) => Some((&path[..slash + dot], true)),
        None => Some((if slash == 0 { "/" } else { &path[..slash] }, false)),
    }
}

/// Write `layer` as a crate file.
pub fn write(layer: &Layer) -> Vec<u8> {
    let mut w = Writer { out: vec![0u8; HEADER], paths: PathTree::new(), ..Writer::default() };
    // Token 0 is never a property's name, so a property path's negated
    // element index is never zero - Pixar's writer puts a smiley there.
    w.token(";-)");
    w.token("");

    // The children and properties of each prim, in the order the specs
    // came.
    let mut children: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    let mut properties: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for spec in &layer.specs {
        if let Some((parent, property)) = parent_of(&spec.path) {
            let name = spec.path[parent.len()..].trim_start_matches(['/', '.']).to_string();
            if property {
                properties.entry(parent).or_default().push(name);
            } else if spec.kind == SpecKind::Prim {
                children.entry(parent).or_default().push(name);
            }
        }
    }

    // The values, and the fields and field sets that point at them.
    let mut fields: Vec<(u32, u64)> = Vec::new();
    let mut field_index: HashMap<(u32, u64), u32> = HashMap::new();
    let mut field_sets: Vec<i32> = Vec::new();
    let mut set_index: HashMap<Vec<u32>, u32> = HashMap::new();
    let mut specs: Vec<(u32, u32, u32)> = Vec::new();
    for spec in &layer.specs {
        let mut set = Vec::new();
        let mut own: Vec<(String, Value)> = spec.fields.iter().filter(|(n, _)| n != "primChildren" && n != "properties").cloned().collect();
        if let Some(names) = children.get(spec.path.as_str()) {
            own.push(("primChildren".into(), Value::TokenVector(names.clone())));
        }
        if let Some(names) = properties.get(spec.path.as_str()) {
            own.push(("properties".into(), Value::TokenVector(names.clone())));
        }
        for (name, value) in &own {
            let token = w.token(name);
            let rep = w.value(value);
            let index = *field_index.entry((token, rep)).or_insert_with(|| {
                fields.push((token, rep));
                fields.len() as u32 - 1
            });
            set.push(index);
        }
        let set_at = match set_index.get(&set) {
            Some(&at) => at,
            None => {
                let at = field_sets.len() as u32;
                field_sets.extend(set.iter().map(|&i| i as i32));
                field_sets.push(-1);
                set_index.insert(set, at);
                at
            }
        };
        let path = w.path(&spec.path);
        specs.push((path, set_at, spec.kind.code()));
    }

    // Every path's element is a token, and the tokens are written first.
    for node in 1..w.paths.nodes.len() {
        let element = w.paths.nodes[node].1.clone();
        w.token(&element);
    }

    let mut toc: Vec<(&str, u64, u64)> = Vec::new();
    let mut section = |w: &mut Writer, name: &'static str, start: u64| toc.push((name, start, w.here() - start));

    // TOKENS: each followed by a zero byte, compressed.
    let start = w.here();
    let mut blob = Vec::new();
    for t in &w.tokens {
        blob.extend_from_slice(t.as_bytes());
        blob.push(0);
    }
    let packed = compress(&blob);
    w.u64(w.tokens.len() as u64);
    w.u64(blob.len() as u64);
    w.u64(packed.len() as u64);
    w.out.extend_from_slice(&packed);
    section(&mut w, SECTIONS[0], start);

    // STRINGS: the token each string is.
    let start = w.here();
    w.u64(w.strings.len() as u64);
    for i in w.strings.clone() {
        w.u32(i);
    }
    section(&mut w, SECTIONS[1], start);

    // FIELDS: the names as integers, then the reps compressed.
    let start = w.here();
    w.u64(fields.len() as u64);
    let names: Vec<i32> = fields.iter().map(|f| f.0 as i32).collect();
    w.ints(&names);
    let reps: Vec<u8> = fields.iter().flat_map(|f| f.1.to_le_bytes()).collect();
    let packed = compress(&reps);
    w.u64(packed.len() as u64);
    w.out.extend_from_slice(&packed);
    section(&mut w, SECTIONS[2], start);

    // FIELDSETS: field indices, each set ended by ~0.
    let start = w.here();
    w.u64(field_sets.len() as u64);
    w.ints(&field_sets);
    section(&mut w, SECTIONS[3], start);

    // PATHS: the tree depth first, as three runs - each node's path index,
    // its element's token (negative for a property), and where its next
    // sibling is: -2 neither child nor sibling, -1 a child (next) only, 0 a
    // sibling (next) only, n a child next and a sibling n on.
    let start = w.here();
    let count = w.paths.nodes.len();
    let mut order = Vec::with_capacity(count);
    let mut stack = vec![0usize];
    // Depth first, remembering for each node the node after it among its
    // parent's children.
    let mut next_sibling = vec![None; count];
    for node in 0..count {
        let kids = &w.paths.children[node];
        for pair in kids.windows(2) {
            next_sibling[pair[0]] = Some(pair[1]);
        }
    }
    let mut position = vec![0usize; count];
    while let Some(node) = stack.pop() {
        position[node] = order.len();
        order.push(node);
        // Children pushed in reverse so the first is visited first.
        for &child in w.paths.children[node].iter().rev() {
            stack.push(child);
        }
    }
    let mut element_tokens = Vec::with_capacity(count);
    for &node in &order {
        let (_, element, property) = w.paths.nodes[node].clone();
        let token = w.token(if node == 0 { "" } else { &element }) as i32;
        element_tokens.push(if property { -token } else { token });
    }
    let jumps: Vec<i32> = order
        .iter()
        .map(|&node| {
            let child = !w.paths.children[node].is_empty();
            match (child, next_sibling[node]) {
                (true, Some(sibling)) => (position[sibling] - position[node]) as i32,
                (true, None) => -1,
                (false, Some(_)) => 0,
                (false, None) => -2,
            }
        })
        .collect();
    let path_indexes: Vec<i32> = order.iter().map(|&n| n as i32).collect();
    w.u64(count as u64);
    w.u64(count as u64);
    w.ints(&path_indexes);
    w.ints(&element_tokens);
    w.ints(&jumps);
    section(&mut w, SECTIONS[4], start);

    // SPECS: path, field set and kind, as three runs.
    let start = w.here();
    w.u64(specs.len() as u64);
    let runs: [Vec<i32>; 3] = [
        specs.iter().map(|s| s.0 as i32).collect(),
        specs.iter().map(|s| s.1 as i32).collect(),
        specs.iter().map(|s| s.2 as i32).collect(),
    ];
    for run in &runs {
        w.ints(run);
    }
    section(&mut w, SECTIONS[5], start);

    // The table of contents, and the header that finds it.
    let toc_at = w.here();
    w.u64(toc.len() as u64);
    for (name, start, size) in &toc {
        let mut field = [0u8; 16];
        field[..name.len()].copy_from_slice(name.as_bytes());
        w.out.extend_from_slice(&field);
        w.u64(*start);
        w.u64(*size);
    }
    w.out[0..8].copy_from_slice(MAGIC);
    w.out[8..11].copy_from_slice(&VERSION);
    w.out[16..24].copy_from_slice(&toc_at.to_le_bytes());
    w.out
}

// ---- reading --------------------------------------------------------------------

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    version: (u8, u8, u8),
    tokens: Vec<String>,
    strings: Vec<u32>,
    paths: Vec<String>,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CrateError> {
        let s = self.bytes.get(self.at..self.at.checked_add(n).ok_or_else(|| bad("a size overflows"))?).ok_or_else(|| bad("the file ends early"))?;
        self.at += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8, CrateError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, CrateError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("four")))
    }

    fn u64(&mut self) -> Result<u64, CrateError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("eight")))
    }

    fn f32(&mut self) -> Result<f32, CrateError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().expect("four")))
    }

    fn f64(&mut self) -> Result<f64, CrateError> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().expect("eight")))
    }

    /// A count no larger than the bytes left could hold, `each` bytes an
    /// item: a damaged count is caught before it allocates.
    fn count(&mut self, each: usize) -> Result<usize, CrateError> {
        let n = self.u64()? as usize;
        if n.saturating_mul(each.max(1)) > self.bytes.len() {
            return Err(bad("a count is larger than the file"));
        }
        Ok(n)
    }

    fn ints(&mut self, count: usize, wide: bool) -> Result<Vec<i64>, CrateError> {
        let size = self.count(1)?;
        let packed = self.take(size)?;
        let limit = (if wide { 8 } else { 4 }) * (count + 1) + count.div_ceil(4) + 16;
        let buffer = decompress(packed, limit)?;
        decode_ints(&buffer, count, wide)
    }

    fn token(&self, i: u64) -> Result<String, CrateError> {
        self.tokens.get(i as usize).cloned().ok_or_else(|| bad(format!("token {i} is not in the file")))
    }

    fn string(&self, i: u64) -> Result<String, CrateError> {
        let token = *self.strings.get(i as usize).ok_or_else(|| bad(format!("string {i} is not in the file")))?;
        self.token(token as u64)
    }

    fn path(&self, i: u64) -> Result<String, CrateError> {
        self.paths.get(i as usize).cloned().ok_or_else(|| bad(format!("path {i} is not in the file")))
    }

    fn at_least(&self, minor: u8) -> bool {
        self.version.0 > 0 || self.version.1 >= minor
    }

    /// An array's length, as each version wrote it.
    fn array_length(&mut self) -> Result<usize, CrateError> {
        if !self.at_least(5) {
            let _rank = self.u32()?;
        }
        let n = if self.at_least(7) { self.u64()? as usize } else { self.u32()? as usize };
        if n > self.bytes.len() {
            return Err(bad("an array is longer than the file"));
        }
        Ok(n)
    }

    fn list_op<T>(&mut self, mut item: impl FnMut(&mut Reader<'a>) -> Result<T, CrateError>) -> Result<ListOp<T>, CrateError> {
        let bits = self.u8()?;
        let mut out = ListOp { explicit: bits & LIST_EXPLICIT != 0, items: Vec::new() };
        // Written in this order, each when its bit is set.
        for (bit, keep) in [
            (LIST_HAS_EXPLICIT, true),
            (LIST_HAS_ADDED, true),
            (LIST_HAS_PREPENDED, true),
            (LIST_HAS_APPENDED, true),
            (LIST_HAS_DELETED, false),
            (LIST_HAS_ORDERED, false),
        ] {
            if bits & bit != 0 {
                let n = self.count(4)?;
                for _ in 0..n {
                    let v = item(self)?;
                    if keep {
                        out.items.push(v);
                    }
                }
            }
        }
        Ok(out)
    }

    /// The scalars of one element of a numeric type, as numbers.
    fn element(&mut self, kind: u8) -> Result<Vec<f64>, CrateError> {
        let (scalar, n) = shape(kind).ok_or_else(|| bad(format!("type {kind} is not numeric")))?;
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(match scalar {
                Scalar::F16 => half::f16::from_bits(u16::from_le_bytes(self.take(2)?.try_into().expect("two"))).to_f64(),
                Scalar::F32 => self.f32()? as f64,
                Scalar::F64 => self.f64()?,
                Scalar::I32 => i32::from_le_bytes(self.take(4)?.try_into().expect("four")) as f64,
                Scalar::U32 => self.u32()? as f64,
                Scalar::I64 => self.u64()? as i64 as f64,
                Scalar::U64 => self.u64()? as f64,
                Scalar::U8 => self.u8()? as f64,
                Scalar::Bool => self.u8()? as f64,
            });
        }
        Ok(out)
    }

    fn value(&mut self, rep: u64) -> Result<Value, CrateError> {
        let kind = ((rep >> 48) & 0xFF) as u8;
        let array = rep & ARRAY_BIT != 0;
        let inline = rep & INLINE_BIT != 0;
        let compressed = rep & COMPRESSED_BIT != 0;
        let payload = rep & PAYLOAD;
        if inline {
            return inline_value(self, kind, payload);
        }
        self.at = payload as usize;
        if self.at > self.bytes.len() {
            return Err(bad("a value is past the end of the file"));
        }
        if array {
            return self.array(kind, compressed, payload);
        }
        Ok(match kind {
            ty::TOKEN_VECTOR => {
                let n = self.count(4)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let i = self.u32()?;
                    v.push(self.token(i as u64)?);
                }
                Value::TokenVector(v)
            }
            ty::PATH_VECTOR => {
                let n = self.count(4)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let i = self.u32()?;
                    v.push(self.path(i as u64)?);
                }
                Value::Paths(ListOp::explicit(v))
            }
            ty::STRING_VECTOR => {
                let n = self.count(4)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let i = self.u32()?;
                    v.push(self.string(i as u64)?);
                }
                Value::Strings(v)
            }
            ty::DOUBLE_VECTOR => {
                let n = self.count(8)?;
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    v.push(self.f64()?);
                }
                Value::Doubles(v)
            }
            ty::PATH_LIST_OP => Value::Paths(self.list_op(|r| {
                let i = r.u32()?;
                r.path(i as u64)
            })?),
            ty::TOKEN_LIST_OP => Value::TokenList(self.list_op(|r| {
                let i = r.u32()?;
                r.token(i as u64)
            })?),
            ty::REFERENCE_LIST_OP => Value::References(self.list_op(|r| {
                let asset = r.u32()?;
                let prim = r.u32()?;
                let _offset = r.f64()?;
                let _scale = r.f64()?;
                // Custom data: a dictionary, which is not followed; one
                // with entries ends the list here.
                if r.u64()? != 0 {
                    return Err(bad("a reference carries custom data, which is not read"));
                }
                Ok(Reference { asset: r.string(asset as u64)?, prim: r.path(prim as u64)? })
            })?),
            ty::STRING => {
                let i = self.u32()?;
                Value::String(self.string(i as u64)?)
            }
            ty::TOKEN => {
                let i = self.u32()?;
                Value::Token(self.token(i as u64)?)
            }
            ty::ASSET => {
                let i = self.u32()?;
                Value::Asset(self.token(i as u64)?)
            }
            ty::SPECIFIER => Value::Specifier(specifier(self.u32()? as u64)),
            ty::VARIABILITY => Value::Variability(self.u32()? == 1),
            k if shape(k).is_some() => numeric(k, self.element(k)?),
            other => Value::Other(other),
        })
    }

    fn array(&mut self, kind: u8, compressed: bool, payload: u64) -> Result<Value, CrateError> {
        // An empty array may be written as no data at all.
        if payload == 0 {
            return Ok(empty_array(kind));
        }
        let n = self.array_length()?;
        match kind {
            ty::TOKEN | ty::STRING | ty::ASSET => {
                let mut v = Vec::with_capacity(n);
                for _ in 0..n {
                    let i = self.u32()? as u64;
                    v.push(if kind == ty::STRING { self.string(i)? } else { self.token(i)? });
                }
                return Ok(if kind == ty::TOKEN { Value::Tokens(v) } else { Value::Strings(v) });
            }
            _ => {}
        }
        let (scalar, width) = shape(kind).ok_or_else(|| bad(format!("arrays of type {kind} are not read")))?;
        // Arrays shorter than sixteen are written plain whatever their bit.
        let numbers: Vec<f64> = if compressed && matches!(scalar, Scalar::I32 | Scalar::U32 | Scalar::I64 | Scalar::U64) && n >= 16 {
            self.ints(n, matches!(scalar, Scalar::I64 | Scalar::U64))?.into_iter().map(|v| v as f64).collect()
        } else if compressed && matches!(scalar, Scalar::F16 | Scalar::F32 | Scalar::F64) && n >= 16 {
            // Floats that are all whole numbers, or few distinct values.
            match self.u8()? {
                b'i' => self.ints(n, false)?.into_iter().map(|v| v as f64).collect(),
                b't' => {
                    let size = self.u32()? as usize;
                    if size > self.bytes.len() {
                        return Err(bad("a lookup table is larger than the file"));
                    }
                    let mut table = Vec::with_capacity(size);
                    for _ in 0..size {
                        table.push(self.element(kind)?[0]);
                    }
                    let indexes = self.ints(n, false)?;
                    indexes.into_iter().map(|i| table.get(i as usize).copied().unwrap_or(0.0)).collect()
                }
                code => return Err(bad(format!("an array of floats is compressed with an unknown code {code}"))),
            }
        } else {
            let mut v = Vec::with_capacity(n * width);
            for _ in 0..n {
                v.extend(self.element(kind)?);
            }
            v
        };
        Ok(match (scalar, width) {
            (Scalar::F64, 1) => Value::Doubles(numbers),
            (Scalar::F16 | Scalar::F32, 1) => Value::Floats(numbers.into_iter().map(|v| v as f32).collect()),
            (Scalar::F64, w) => Value::Vecds(w, numbers),
            (Scalar::F16 | Scalar::F32, w) => Value::Vecfs(w, numbers.into_iter().map(|v| v as f32).collect()),
            (_, 1) => Value::Ints(numbers.into_iter().map(|v| v as i32).collect()),
            (_, w) => Value::Vecfs(w, numbers.into_iter().map(|v| v as f32).collect()),
        })
    }
}

#[derive(Clone, Copy)]
enum Scalar {
    Bool,
    U8,
    I32,
    U32,
    I64,
    U64,
    F16,
    F32,
    F64,
}

/// A numeric type's scalar and how many there are of it.
fn shape(kind: u8) -> Option<(Scalar, usize)> {
    Some(match kind {
        ty::BOOL => (Scalar::Bool, 1),
        ty::UCHAR => (Scalar::U8, 1),
        ty::INT => (Scalar::I32, 1),
        ty::UINT => (Scalar::U32, 1),
        ty::INT64 => (Scalar::I64, 1),
        ty::UINT64 => (Scalar::U64, 1),
        ty::HALF => (Scalar::F16, 1),
        ty::FLOAT => (Scalar::F32, 1),
        ty::DOUBLE => (Scalar::F64, 1),
        ty::MATRIX2D => (Scalar::F64, 4),
        ty::MATRIX3D => (Scalar::F64, 9),
        ty::MATRIX4D => (Scalar::F64, 16),
        ty::QUATD => (Scalar::F64, 4),
        ty::QUATF => (Scalar::F32, 4),
        ty::QUATH => (Scalar::F16, 4),
        ty::VEC2D => (Scalar::F64, 2),
        ty::VEC2F => (Scalar::F32, 2),
        ty::VEC2H => (Scalar::F16, 2),
        ty::VEC2I => (Scalar::I32, 2),
        ty::VEC3D => (Scalar::F64, 3),
        ty::VEC3F => (Scalar::F32, 3),
        ty::VEC3H => (Scalar::F16, 3),
        ty::VEC3I => (Scalar::I32, 3),
        ty::VEC4D => (Scalar::F64, 4),
        ty::VEC4F => (Scalar::F32, 4),
        ty::VEC4H => (Scalar::F16, 4),
        ty::VEC4I => (Scalar::I32, 4),
        _ => return None,
    })
}

/// A numeric value from its numbers.
fn numeric(kind: u8, v: Vec<f64>) -> Value {
    let (scalar, width) = shape(kind).unwrap_or((Scalar::F64, v.len()));
    match (scalar, width) {
        (Scalar::Bool, _) => Value::Bool(v.first().is_some_and(|b| *b != 0.0)),
        (Scalar::F32 | Scalar::F16, 1) => Value::Float(v[0] as f32),
        (Scalar::F64, 1) => Value::Double(v[0]),
        (Scalar::F32 | Scalar::F16, _) => Value::Vecf(v.into_iter().map(|x| x as f32).collect()),
        (Scalar::F64, _) => Value::Vecd(v),
        (_, 1) => Value::Int(v[0] as i32),
        (_, _) => Value::Vecf(v.into_iter().map(|x| x as f32).collect()),
    }
}

fn empty_array(kind: u8) -> Value {
    match shape(kind) {
        Some((Scalar::F64, 1)) => Value::Doubles(Vec::new()),
        Some((Scalar::F16 | Scalar::F32, 1)) => Value::Floats(Vec::new()),
        Some((Scalar::F64, w)) => Value::Vecds(w, Vec::new()),
        Some((Scalar::F16 | Scalar::F32, w)) => Value::Vecfs(w, Vec::new()),
        Some(_) => Value::Ints(Vec::new()),
        None if kind == ty::TOKEN => Value::Tokens(Vec::new()),
        None => Value::Strings(Vec::new()),
    }
}

fn specifier(code: u64) -> Specifier {
    match code {
        1 => Specifier::Over,
        2 => Specifier::Class,
        _ => Specifier::Def,
    }
}

/// A value kept in its rep: small enough, or a vector or matrix of small
/// whole numbers, a byte each.
fn inline_value(r: &Reader, kind: u8, payload: u64) -> Result<Value, CrateError> {
    let bytes = (payload as u32).to_le_bytes();
    let small = |n: usize| -> Vec<f64> { bytes[..n].iter().map(|b| *b as i8 as f64).collect() };
    Ok(match kind {
        ty::BOOL => Value::Bool(payload != 0),
        ty::UCHAR | ty::INT | ty::UINT | ty::INT64 | ty::UINT64 => Value::Int(payload as u32 as i32),
        ty::HALF => Value::Float(half::f16::from_bits(payload as u16).to_f32()),
        ty::FLOAT => Value::Float(f32::from_bits(payload as u32)),
        // A double inlined is one a float holds exactly.
        ty::DOUBLE => Value::Double(f32::from_bits(payload as u32) as f64),
        ty::STRING => Value::String(r.string(payload)?),
        ty::TOKEN => Value::Token(r.token(payload)?),
        ty::ASSET => Value::Asset(r.token(payload)?),
        ty::SPECIFIER => Value::Specifier(specifier(payload)),
        ty::VARIABILITY => Value::Variability(payload == 1),
        ty::PERMISSION => Value::Int(payload as i32),
        ty::MATRIX2D | ty::MATRIX3D | ty::MATRIX4D => {
            // The diagonal, a byte each.
            let n = match kind {
                ty::MATRIX2D => 2,
                ty::MATRIX3D => 3,
                _ => 4,
            };
            let diagonal = small(n);
            let mut m = vec![0.0; n * n];
            for (k, d) in diagonal.into_iter().enumerate() {
                m[k * n + k] = d;
            }
            Value::Vecd(m)
        }
        k if shape(k).is_some_and(|(_, n)| (2..=4).contains(&n)) => {
            let (_, n) = shape(k).expect("checked");
            numeric(k, small(n))
        }
        other => Value::Other(other),
    })
}

/// Read a crate file.
pub fn read(bytes: &[u8]) -> Result<Layer, CrateError> {
    if bytes.len() < HEADER || &bytes[0..8] != MAGIC {
        return Err(bad("this is not a binary USD (crate) file"));
    }
    let version = (bytes[8], bytes[9], bytes[10]);
    if version.0 == 0 && version.1 < 4 {
        return Err(bad(format!(
            "this binary USD is version {}.{}.{}, from before 2017; open it in a current USD tool and save it again",
            version.0, version.1, version.2
        )));
    }
    let mut r = Reader { bytes, at: 16, version, tokens: Vec::new(), strings: Vec::new(), paths: Vec::new() };
    let toc = r.u64()? as usize;
    r.at = toc;
    let count = r.count(32)?;
    let mut sections: HashMap<String, usize> = HashMap::new();
    for _ in 0..count {
        let name = r.take(16)?;
        let name = String::from_utf8_lossy(name.split(|b| *b == 0).next().unwrap_or(&[])).into_owned();
        let start = r.u64()? as usize;
        let _size = r.u64()?;
        sections.insert(name, start);
    }
    let seek = |r: &mut Reader, name: &str| -> Result<bool, CrateError> {
        match sections.get(name) {
            Some(&at) if at <= bytes.len() => {
                r.at = at;
                Ok(true)
            }
            Some(_) => Err(bad(format!("the {name} section is past the end of the file"))),
            None => Ok(false),
        }
    };

    if seek(&mut r, "TOKENS")? {
        let n = r.count(1)?;
        let size = r.count(1)?;
        let packed_size = r.count(1)?;
        let packed = r.take(packed_size)?;
        let blob = decompress(packed, size)?;
        r.tokens = blob.split(|b| *b == 0).take(n).map(|t| String::from_utf8_lossy(t).into_owned()).collect();
        if r.tokens.len() != n {
            return Err(bad("the tokens section holds fewer tokens than it says"));
        }
    }
    if seek(&mut r, "STRINGS")? {
        let n = r.count(4)?;
        for _ in 0..n {
            let i = r.u32()?;
            r.strings.push(i);
        }
    }
    let mut fields: Vec<(String, u64)> = Vec::new();
    if seek(&mut r, "FIELDS")? {
        let n = r.count(4)?;
        let names = r.ints(n, false)?;
        let size = r.count(1)?;
        let packed = r.take(size)?;
        let reps = decompress(packed, n * 8)?;
        if reps.len() != n * 8 {
            return Err(bad("the fields section is short"));
        }
        for (k, name) in names.into_iter().enumerate() {
            let rep = u64::from_le_bytes(reps[k * 8..k * 8 + 8].try_into().expect("eight"));
            fields.push((r.token(name as u32 as u64)?, rep));
        }
    }
    let mut sets: Vec<i64> = Vec::new();
    if seek(&mut r, "FIELDSETS")? {
        let n = r.count(4)?;
        sets = r.ints(n, false)?;
    }
    if seek(&mut r, "PATHS")? {
        let total = r.count(4)?;
        let n = r.count(4)?;
        let indexes = r.ints(n, false)?;
        let elements = r.ints(n, false)?;
        let jumps = r.ints(n, false)?;
        let mut paths = vec![String::new(); total];
        // Iteratively: a stack of (node, parent path) still to visit.
        let mut stack: Vec<(usize, Option<String>)> = vec![(0, None)];
        while let Some((mut i, mut parent)) = stack.pop() {
            loop {
                if i >= n {
                    return Err(bad("a path's sibling is past the end of the paths"));
                }
                let here = match &parent {
                    None => "/".to_string(),
                    Some(parent) => {
                        let element = elements[i];
                        let name = r.token(element.unsigned_abs())?;
                        if element < 0 {
                            format!("{parent}.{name}")
                        } else if parent == "/" {
                            format!("/{name}")
                        } else {
                            format!("{parent}/{name}")
                        }
                    }
                };
                let slot = indexes[i] as usize;
                *paths.get_mut(slot).ok_or_else(|| bad("a path index is out of range"))? = here.clone();
                let jump = jumps[i];
                let child = jump > 0 || jump == -1;
                let sibling = jump >= 0;
                if child && sibling {
                    stack.push((i + jump as usize, parent.clone()));
                }
                if child {
                    parent = Some(here);
                }
                if !child && !sibling {
                    break;
                }
                i += 1;
            }
        }
        r.paths = paths;
    }
    let mut layer = Layer { specs: Vec::new() };
    if seek(&mut r, "SPECS")? {
        let n = r.count(4)?;
        let paths = r.ints(n, false)?;
        let set_starts = r.ints(n, false)?;
        let kinds = r.ints(n, false)?;
        for k in 0..n {
            let path = r.path(paths[k] as u64)?;
            let mut spec = Spec { path, kind: SpecKind::from_code(kinds[k] as u32), fields: Vec::new() };
            let mut at = set_starts[k] as usize;
            while let Some(&field) = sets.get(at) {
                if field < 0 || field as u32 == u32::MAX {
                    break;
                }
                let (name, rep) = fields.get(field as usize).cloned().ok_or_else(|| bad("a field set names a field not in the file"))?;
                let value = r.value(rep)?;
                spec.fields.push((name, value));
                at += 1;
            }
            layer.specs.push(spec);
        }
    }
    Ok(layer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_coding_round_trips() {
        let values = vec![0, 1, 2, 3, 3, 3, -1, 70000, -70000, 5, i32::MAX, i32::MIN, 300, 301, -2];
        let coded = encode_ints(&values);
        let back = decode_ints(&coded, values.len(), false).unwrap();
        assert_eq!(back.into_iter().map(|v| v as i32).collect::<Vec<_>>(), values);
    }

    #[test]
    fn literal_lz4_decodes() {
        for n in [0usize, 1, 14, 15, 16, 269, 270, 271, 5000] {
            let data: Vec<u8> = (0..n).map(|i| (i * 7 % 251) as u8).collect();
            assert_eq!(decompress(&compress(&data), n).unwrap(), data, "{n} bytes");
        }
    }

    /// A block with a back-reference, as LZ4 itself writes: "abcabcabcabc".
    #[test]
    fn lz4_matches_decode() {
        // Three literals, then a match of nine at offset three.
        let block = [0x35, b'a', b'b', b'c', 3, 0];
        let mut out = Vec::new();
        lz4_block(&block, &mut out, 64).unwrap();
        assert_eq!(out, b"abcabcabcabc");
    }

    fn sample() -> Layer {
        let mut layer = Layer::new();
        layer.root().set("defaultPrim", Value::Token("Scene".into())).set("metersPerUnit", Value::Double(1.0)).set("upAxis", Value::Token("Y".into()));
        layer.prim("/Scene", Specifier::Def, "Xform").set("kind", Value::Token("component".into()));
        layer.prim("/Scene/Body", Specifier::Def, "Mesh").set("apiSchemas", Value::TokenList(ListOp::prepend(vec!["MaterialBindingAPI".into()])));
        layer.attribute("/Scene/Body", "points", "point3f[]", Some(Value::Vecfs(3, vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.25])));
        layer.attribute("/Scene/Body", "faceVertexIndices", "int[]", Some(Value::Ints(vec![0, 1, 2])));
        layer.attribute("/Scene/Body", "xformOp:transform", "matrix4d", Some(Value::Vecd((0..16).map(|i| i as f64 * 0.5).collect())));
        layer.attribute("/Scene/Body", "xformOpOrder", "token[]", Some(Value::Tokens(vec!["xformOp:transform".into()]))).set("variability", Value::Variability(true));
        layer.relationship("/Scene/Body", "material:binding", vec!["/Scene/Looks/Red".into()]);
        layer.prim("/Scene/Looks", Specifier::Def, "Scope");
        layer.prim("/Scene/Looks/Red", Specifier::Def, "Material");
        layer.connection("/Scene/Looks/Red", "outputs:surface", "token", "/Scene/Looks/Red/Surface.outputs:surface");
        layer.prim("/Scene/Looks/Red/Surface", Specifier::Def, "Shader");
        layer.attribute("/Scene/Looks/Red/Surface", "inputs:roughness", "float", Some(Value::Float(0.3)));
        layer.attribute("/Scene/Looks/Red/Surface", "inputs:ior", "float", Some(Value::Double(1.123456789)));
        layer.attribute("/Scene/Looks/Red/Surface", "inputs:file", "asset", Some(Value::Asset("textures/a.png".into())));
        layer.attribute("/Scene/Looks/Red/Surface", "outputs:surface", "token", None);
        layer.prim("/Prototypes", Specifier::Class, "");
        layer.prim("/Scene/Copy", Specifier::Def, "Xform")
            .set("instanceable", Value::Bool(true))
            .set("references", Value::References(ListOp::prepend(vec![Reference { asset: String::new(), prim: "/Prototypes".into() }])));
        layer
    }

    #[test]
    fn a_layer_round_trips() {
        let layer = sample();
        let bytes = write(&layer);
        assert_eq!(&bytes[0..8], b"PXR-USDC");
        let back = read(&bytes).expect("reads");
        for spec in &layer.specs {
            let read = back.spec(&spec.path).unwrap_or_else(|| panic!("{} is there", spec.path));
            assert_eq!(read.kind, spec.kind, "{}", spec.path);
            for (name, value) in &spec.fields {
                assert_eq!(read.field(name), Some(value), "{}.{name}", spec.path);
            }
        }
        // The children and properties, made from the paths.
        let scene = back.spec("/Scene").unwrap();
        assert_eq!(scene.field("primChildren"), Some(&Value::TokenVector(vec!["Body".into(), "Looks".into(), "Copy".into()])));
        let body = back.spec("/Scene/Body").unwrap();
        assert_eq!(
            body.field("properties"),
            Some(&Value::TokenVector(vec!["points".into(), "faceVertexIndices".into(), "xformOp:transform".into(), "xformOpOrder".into(), "material:binding".into()]))
        );
        let root = back.spec("/").unwrap();
        assert_eq!(root.field("primChildren"), Some(&Value::TokenVector(vec!["Scene".into(), "Prototypes".into()])));
    }

    #[test]
    fn damage_is_refused_not_followed() {
        let bytes = write(&sample());
        for cut in [10, 90, bytes.len() / 2, bytes.len() - 10] {
            let _ = read(&bytes[..cut]);
        }
        let mut flipped = bytes.clone();
        for i in (HEADER..flipped.len()).step_by(7) {
            flipped[i] ^= 0x5A;
            let _ = read(&flipped);
            flipped[i] ^= 0x5A;
        }
    }
}
