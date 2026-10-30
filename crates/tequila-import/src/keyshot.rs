//! KeyShot material packages (`.kmp`), into Tequila's library.
//!
//! A `.kmp` is a ZIP: one or more `.mtl` files, the pictures they read, and
//! a `Configuration.xml` listing both. The `.mtl` is text: numbered shader
//! nodes, each a type and its parameters, then the material, which names the
//! node that is its surface:
//!
//! ```text
//! #define shader 2 texture_rgb { "texture" "C:/…/wood.jpg", "scale_u" 2.2, … }
//! #define shader 3 lux_metal { "color" 0.35 0.34 0.33, "color" ! 2, "roughness" 0.075, }
//! #define shader 4 lux_bumpmap { "bumpmap" 1, "shader" 3, "bump_scale" 1, }
//! #define material "Weathered Metal" { icon bX89504e…, surface 4 }
//! ```
//!
//! `"key" ! N` drives the input from node N. A list of nodes is written as
//! bytes between brackets: a four-byte tag ending in its element type, a
//! little-endian count, then the elements.
//!
//! KeyShot's material types are the ones Tequila's are modelled on, so a
//! surface becomes the same type with its numbers carried over, and its
//! pictures drive the same inputs. What has no counterpart is said in the
//! notes rather than dropped silently: procedural patterns (a flat colour in
//! their place), measured AxF materials (approximated from the package's own
//! preview), and textures in KeyShot's own protected format (Mold-Tech's).
//!
//! Read for interoperability, as a person's own files; nothing of KeyShot's
//! is used beyond what a package carries.

use crate::texture_set::flip_green;
use crate::ImportError;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tequila_core::math::vec3;
use tequila_core::spectrum::Spectrum;
use tequila_format::library::{LibraryEntry, Source};
use tequila_materials::fresnel::ComplexIor;
use tequila_materials::graph::{Channel, MaterialGraph, Node, NodeId, NodeKind, Socket, TextureId, Value};
use tequila_materials::nodes::{MixData, ShaderNode};
use tequila_texture::cycles::ops::{BlendMode, MapRangeKind, MathOp};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_materials::openpbr::Material;
use tequila_scene::textures::TextureSource;
use tequila_texture::mip::Wrap;
use tequila_texture::projection::{Axis, Mapping, Projection};
use tequila_texture::texture::MapRole;

/// A material read out of a package, and what did not come across.
#[derive(Debug)]
pub struct Converted {
    pub entry: LibraryEntry,
    pub notes: Vec<String>,
}

/// Metres per unit of the oldest packages' texture sizes (`scale_u`), which
/// say no unit: measured against the 100 mm disc KeyShot's own preview shows
/// a wood grain on. Newer packages give sizes in metres.
const OLD_UNIT: f32 = 0.22;

/// Scene units of bump per unit of a procedural's `bump_height`: White
/// pearl's noise, at 5, a gentle undulation as in KeyShot's preview.
const PROCEDURAL_BUMP: f32 = 0.00002;

/// What stands in for a texture in KeyShot's protected format (Mold-Tech's):
/// a fine, even moulded texture, its features this wide and this deep.
/// Mold-Tech's patterns are anything from sand grains to cross-hatching,
/// and an encrypted one cannot be told apart; a fine stipple reads as a
/// textured moulding whichever it was. Sized from the texture's tile, six
/// to one, the features of a 10 mm tile were lumps 1.7 mm across, which
/// no moulding has.
const GRAIN_SIZE: f32 = 0.000_35;
const GRAIN_DEPTH: f32 = 0.000_03;

/// Metres between Marble's bands at a `scale` of 1: measured off the 100 mm
/// disc KeyShot's preview shows a 3D-print material's layer lines on.
const MARBLE_PERIOD: f32 = 0.0014;

/// The first material in a `.kmp`, its pictures extracted into `scratch`.
pub fn read_kmp(path: &Path, scratch: &Path) -> Result<Converted, ImportError> {
    let bytes = std::fs::read(path).map_err(|e| ImportError::Io(format!("{}: {e}", path.display())))?;
    let members = crate::texture_set::read_zip(&bytes)?;
    let mut mtls: Vec<&(String, Vec<u8>)> = members.iter().filter(|(n, _)| n.to_ascii_lowercase().ends_with(".mtl")).collect();
    mtls.sort_by(|a, b| a.0.cmp(&b.0));
    let Some((mtl_name, text)) = mtls.first().map(|m| (&m.0, &m.1)) else {
        return Err(ImportError::Malformed(format!("{} holds no KeyShot material (.mtl)", path.display())));
    };
    let mtl = parse(text).map_err(|e| ImportError::Malformed(format!("{}: {mtl_name}: {e}", path.display())))?;
    std::fs::create_dir_all(scratch).map_err(|e| ImportError::Io(format!("{}: {e}", scratch.display())))?;
    // The pictures by bare name, as the package keeps them: the paths in
    // the .mtl are wherever they were on the author's computer.
    let pictures: BTreeMap<String, &[u8]> = members
        .iter()
        .filter(|(n, _)| !n.to_ascii_lowercase().ends_with(".mtl") && !n.to_ascii_lowercase().ends_with(".xml"))
        .map(|(n, d)| (base_name(n).to_ascii_lowercase(), d.as_slice()))
        .collect();
    let mut written: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut picture = |wanted: &str| -> Result<Option<PathBuf>, String> {
        let key = base_name(wanted).to_ascii_lowercase();
        if let Some(path) = written.get(&key) {
            return Ok(Some(path.clone()));
        }
        let Some(data) = pictures.get(&key) else { return Ok(None) };
        // Only a plain file name lands in the scratch folder, whatever the
        // package calls it.
        let name: String = base_name(wanted).chars().map(|c| if c.is_alphanumeric() || " ._-".contains(c) { c } else { '_' }).collect();
        let file = scratch.join(if name.is_empty() { "texture".into() } else { name });
        std::fs::write(&file, data).map_err(|e| format!("{}: {e}", file.display()))?;
        written.insert(key, file.clone());
        Ok(Some(file))
    };
    let mut converted = convert(&mtl, &mut picture, scratch)?;
    if mtls.len() > 1 {
        converted.notes.push(format!("the package holds {} materials; the first, {}, was imported", mtls.len(), mtl.name));
    }
    Ok(converted)
}

/// Whether `path` is a KeyShot material package, by its extension.
pub fn is_kmp(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("kmp"))
}

/// A `.kmp` into the library at `directory`, `adjust`ed first: the file the
/// entry was saved to, and what did not come across.
pub fn import_to_library(
    path: &Path,
    directory: &Path,
    adjust: impl FnOnce(&mut LibraryEntry),
) -> Result<(PathBuf, Vec<String>), ImportError> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let scratch = std::env::temp_dir().join(format!("tequila-kmp-{}-{stamp}", std::process::id()));
    let result = read_kmp(path, &scratch).and_then(|mut converted| {
        adjust(&mut converted.entry);
        let saved = tequila_format::library::save_entry(directory, &converted.entry).map_err(|e| ImportError::Io(e.to_string()))?;
        Ok((saved, converted.notes))
    });
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

// ---- reading the .mtl -----------------------------------------------------------

/// A parameter's value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Val {
    Numbers(Vec<f32>),
    Text(String),
    Flag(bool),
    /// Driven by another node.
    Link(u32),
    /// A list of nodes.
    List(Vec<u32>),
}

/// A shader node: its type and parameters. A scene's (`crate::bip_scene`)
/// are the same nodes a package's `.mtl` holds.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shader {
    pub kind: String,
    pub params: Vec<(String, Val)>,
}

impl Shader {
    fn value(&self, key: &str) -> Option<&Val> {
        self.params.iter().find(|(k, v)| k == key && !matches!(v, Val::Link(_))).map(|(_, v)| v)
    }
    fn link(&self, key: &str) -> Option<u32> {
        self.params.iter().find_map(|(k, v)| match v {
            Val::Link(n) if k == key => Some(*n),
            _ => None,
        })
    }
    fn number(&self, key: &str) -> Option<f32> {
        match self.value(key)? {
            Val::Numbers(v) => v.first().copied(),
            _ => None,
        }
    }
    fn colour(&self, key: &str) -> Option<Spectrum> {
        match self.value(key)? {
            Val::Numbers(v) if v.len() >= 3 => Some(Spectrum::from_linear_srgb(vec3(v[0], v[1], v[2]))),
            Val::Numbers(v) if v.len() == 1 => Some(Spectrum::splat(v[0])),
            _ => None,
        }
    }
    fn text(&self, key: &str) -> Option<&str> {
        match self.value(key)? {
            Val::Text(t) => Some(t),
            _ => None,
        }
    }
    fn flag(&self, key: &str) -> Option<bool> {
        match self.value(key)? {
            Val::Flag(f) => Some(*f),
            Val::Numbers(v) => v.first().map(|n| *n != 0.0),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Mtl {
    pub name: String,
    pub shaders: BTreeMap<u32, Shader>,
    pub surface: Option<u32>,
    /// A displacement, when the material has one.
    pub geometry: Option<u32>,
    pub icon: Option<Vec<u8>>,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }
    fn skip_space(&mut self) {
        while let Some(b) = self.peek() {
            if b == b'/' && self.bytes.get(self.at + 1) == Some(&b'/') {
                while self.peek().is_some_and(|b| b != b'\n') {
                    self.at += 1;
                }
            } else if b.is_ascii_whitespace() || b == b',' {
                self.at += 1;
            } else {
                break;
            }
        }
    }
    fn word(&mut self) -> String {
        self.skip_space();
        let start = self.at;
        while self.peek().is_some_and(|b| !b.is_ascii_whitespace() && b != b',' && b != b'{' && b != b'}') {
            self.at += 1;
        }
        String::from_utf8_lossy(&self.bytes[start..self.at]).into_owned()
    }
    /// A quoted string, taken as written: KeyShot does not escape its
    /// Windows paths' backslashes.
    fn quoted(&mut self) -> Result<String, String> {
        self.skip_space();
        if self.peek() != Some(b'"') {
            return Err(format!("expected a quoted name at byte {}", self.at));
        }
        self.at += 1;
        let start = self.at;
        while self.peek().is_some_and(|b| b != b'"') {
            self.at += 1;
        }
        let text = self.bytes[start..self.at].iter().map(|&b| b as char).collect();
        self.at += 1;
        Ok(text)
    }
    fn expect(&mut self, byte: u8) -> Result<(), String> {
        self.skip_space();
        if self.peek() == Some(byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("expected `{}` at byte {}", byte as char, self.at))
        }
    }
    fn hex(&mut self) -> Vec<u8> {
        // `bX` then hex digits; a stray leading digit pads them to an odd
        // count, and the picture is what starts at its PNG signature.
        let word = self.word();
        let digits = word.strip_prefix("bX").unwrap_or(&word);
        let bytes: Vec<u8> = digits.as_bytes().chunks(2).filter_map(|p| u8::from_str_radix(std::str::from_utf8(p).ok()?, 16).ok()).collect();
        let png = b"\x89PNG";
        match bytes.windows(4).position(|w| w == png) {
            Some(at) if at < 4 => bytes[at..].to_vec(),
            _ => bytes,
        }
    }
}

fn parse(bytes: &[u8]) -> Result<Mtl, String> {
    if is_binary(bytes) {
        return parse_binary(bytes);
    }
    let mut c = Cursor { bytes, at: 0 };
    let mut mtl = Mtl::default();
    loop {
        c.skip_space();
        let Some(b) = c.peek() else { break };
        if b == b'#' {
            let define = c.word();
            if define != "#define" {
                return Err(format!("unexpected `{define}`"));
            }
            match c.word().as_str() {
                "shader" => {
                    let number: u32 = c.word().parse().map_err(|_| "a shader without a number".to_string())?;
                    let kind = c.word();
                    c.expect(b'{')?;
                    let params = parse_params(&mut c)?;
                    mtl.shaders.insert(number, Shader { kind, params });
                }
                "material" => {
                    mtl.name = c.quoted()?;
                    c.expect(b'{')?;
                    loop {
                        c.skip_space();
                        match c.peek() {
                            Some(b'}') => {
                                c.at += 1;
                                break;
                            }
                            None => return Err("the material is not closed".into()),
                            _ => {}
                        }
                        match c.word().as_str() {
                            "icon" => mtl.icon = Some(c.hex()),
                            "surface" => mtl.surface = c.word().parse().ok(),
                            "geometry" => mtl.geometry = c.word().parse().ok(),
                            "attribute" => {
                                c.quoted()?;
                                c.quoted()?;
                            }
                            // id, flags, geometry and the rest: a number each.
                            _ => {
                                c.word();
                            }
                        }
                    }
                }
                other => return Err(format!("unknown definition `{other}`")),
            }
        } else if c.bytes[c.at..].starts_with(b"icon") {
            c.word();
            mtl.icon = Some(c.hex());
        } else {
            return Err(format!("unexpected text at byte {}", c.at));
        }
    }
    if mtl.surface.is_none() {
        return Err("the material names no surface".into());
    }
    Ok(mtl)
}

// ---- KeyShot 11 and later: the same nodes, as binary records -------------------
//
// The newer `.mtl` keeps its first line (`//--lux:mat:001--`), then a
// fixed header, then a stream of records. A record is a tag byte, its
// payload's length - one byte, or four little-endian when the tag's low
// bits are 2, none when they are 0 - and the payload. The nodes are the
// text format's, record for record:
//
// ```text
// 07  id, type name            a node: `#define shader <id> <type> {`
// 09                           the node's end
// 17/1d/25  f32/i32/bool, key  a number, a whole number, on/off
// 27/29  3 or 4 f32, key       a colour, without or with alpha
// 2f  length, text, key        text: a picture's file name
// 31  id, key                  a node by number, as `"shader" 3`
// 9b  id, key                  driven by a node, as `"color" ! 3`
// a1  id, key                  one of the file's named colours
// 7f  key length, key, ids     a list of nodes
// 0b  name                     the material; `11` (1, id) names its surface
// 9d … 9f                      a named colour: `39` its id, `29` the colour
// 36  …PNG                     the preview
// ```
//
// Read from the packages people share, as the text format is.

/// Where the records start, after the first line and the fixed header.
const BINARY_RECORDS: usize = 128;

fn is_binary(bytes: &[u8]) -> bool {
    let Some(newline) = bytes.iter().take(64).position(|&b| b == b'\n') else { return false };
    bytes.starts_with(b"//--lux:mat:") && bytes[newline..].iter().take(32).any(|&b| b == 0)
}

fn parse_binary(bytes: &[u8]) -> Result<Mtl, String> {
    let u32_at = |p: &[u8], at: usize| p.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let f32_at = |p: &[u8], at: usize| u32_at(p, at).map(f32::from_bits);
    let text = |p: &[u8]| String::from_utf8_lossy(p).into_owned();

    let mut mtl = Mtl::default();
    let mut colours: BTreeMap<u32, Vec<f32>> = BTreeMap::new();
    // A named colour being read: its id and its value.
    let mut colour: Option<(Option<u32>, Option<Vec<f32>>)> = None;
    let mut shader: Option<(u32, Shader)> = None;
    let mut in_material = false;
    let mut at = BINARY_RECORDS;
    while at < bytes.len() {
        let tag = bytes[at];
        let (start, length) = match tag & 3 {
            0 => (at + 1, 0),
            2 => (at + 5, u32_at(bytes, at + 1).ok_or("a record is cut short")? as usize),
            _ => (at + 2, *bytes.get(at + 1).ok_or("a record is cut short")? as usize),
        };
        let payload = bytes.get(start..start + length).ok_or_else(|| format!("a record at byte {at} runs past the end"))?;
        at = start + length;
        match tag {
            0x07 | 0x09 | 0x0b => {
                if let Some((id, s)) = shader.take() {
                    mtl.shaders.insert(id, s);
                }
                if tag == 0x07 {
                    let id = u32_at(payload, 0).ok_or("a node without a number")?;
                    shader = Some((id, Shader { kind: text(&payload[4..]), params: Vec::new() }));
                } else if tag == 0x0b {
                    mtl.name = text(payload);
                    in_material = true;
                }
            }
            0x36 => {
                let png = b"\x89PNG";
                if let Some(p) = payload.windows(4).position(|w| w == png) {
                    mtl.icon = Some(payload[p..].to_vec());
                }
            }
            0x9d => colour = Some((None, None)),
            0x9f => {
                if let Some((Some(id), Some(value))) = colour.take() {
                    colours.insert(id, value);
                }
            }
            0x39 if colour.is_some() => {
                if let Some((id, _)) = colour.as_mut() {
                    *id = u32_at(payload, 0);
                }
            }
            0x29 if colour.is_some() => {
                if let Some((_, value)) = colour.as_mut() {
                    *value = (0..4).map(|i| f32_at(payload, 4 * i)).collect();
                }
            }
            // The material's surface (1) or displacement (2), by node.
            0x11 if in_material && shader.is_none() => {
                if let (Some(role), Some(id)) = (u32_at(payload, 0), u32_at(payload, 4)) {
                    match role {
                        1 => mtl.surface = Some(id),
                        2 => mtl.geometry = Some(id),
                        _ => {}
                    }
                }
            }
            _ => {
                if let (Some((_, s)), Some(param)) = (shader.as_mut(), binary_param(tag, payload, &colours)) {
                    s.params.push(param);
                }
            }
        }
    }
    if let Some((id, s)) = shader.take() {
        mtl.shaders.insert(id, s);
    }
    if mtl.surface.is_none() {
        return Err("the material names no surface".into());
    }
    Ok(mtl)
}

/// A node's parameter from its record: the value comes first, the name
/// after it (a list's name comes first). `None` for what carries nothing a
/// material needs: attributes, the editor's notes.
fn binary_param(tag: u8, payload: &[u8], colours: &BTreeMap<u32, Vec<f32>>) -> Option<(String, Val)> {
    let word = |at: usize| payload.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let name = |at: usize| payload.get(at..).map(|k| String::from_utf8_lossy(k).into_owned());
    let numbers = |count: usize| -> Option<(String, Val)> {
        let values: Option<Vec<f32>> = (0..count).map(|i| word(4 * i).map(f32::from_bits)).collect();
        Some((name(4 * count)?, Val::Numbers(values?)))
    };
    match tag {
        0x17 => numbers(1),
        0x27 => numbers(3),
        0x29 => numbers(4),
        0x1d => Some((name(4)?, Val::Numbers(vec![word(0)? as i32 as f32]))),
        0x25 => Some((name(4)?, Val::Flag(word(0)? != 0))),
        0x31 => Some((name(4)?, Val::Numbers(vec![word(0)? as f32]))),
        0x9b => Some((name(4)?, Val::Link(word(0)?))),
        0xa1 => Some((name(4)?, Val::Numbers(colours.get(&word(0)?)?.clone()))),
        0x2f => {
            let n = word(0)? as usize;
            let value = String::from_utf8_lossy(payload.get(4..4 + n)?).into_owned();
            Some((name(4 + n)?, Val::Text(value)))
        }
        0x7f => {
            let n = word(0)? as usize;
            let key = String::from_utf8_lossy(payload.get(4..4 + n)?).into_owned();
            let rest = payload.get(4 + n..)?;
            let ids = rest.chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
            Some((key, Val::List(ids)))
        }
        _ => None,
    }
}

fn parse_params(c: &mut Cursor) -> Result<Vec<(String, Val)>, String> {
    let mut params = Vec::new();
    loop {
        c.skip_space();
        match c.peek() {
            Some(b'}') => {
                c.at += 1;
                return Ok(params);
            }
            None => return Err("a shader is not closed".into()),
            _ => {}
        }
        let key = c.quoted()?;
        c.skip_space();
        let value = match c.peek() {
            Some(b'"') => Val::Text(c.quoted()?),
            Some(b'!') => {
                c.at += 1;
                Val::Link(c.word().parse().map_err(|_| format!("`{key}` links to no node"))?)
            }
            Some(b'[') => {
                c.at += 1;
                // Bytes: a tag, its last byte the element type; a count; the
                // elements, four bytes each.
                let read = |at: usize| c.bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                let start = c.at + c.bytes[c.at..].iter().take_while(|b| b.is_ascii_whitespace()).count();
                let count = read(start + 4).ok_or("a list is cut short")? as usize;
                if count > 4096 {
                    return Err(format!("`{key}` lists {count} nodes"));
                }
                let items: Option<Vec<u32>> = (0..count).map(|i| read(start + 8 + 4 * i)).collect();
                c.at = start + 8 + 4 * count;
                c.expect(b']')?;
                Val::List(items.ok_or("a list is cut short")?)
            }
            _ => {
                // Numbers, or on/off, up to the comma.
                let mut numbers = Vec::new();
                let mut flag = None;
                loop {
                    c.skip_space_inline();
                    match c.peek() {
                        Some(b',') | Some(b'\n') | Some(b'}') | None => break,
                        _ => {}
                    }
                    let word = c.word();
                    match word.as_str() {
                        "on" => flag = Some(true),
                        "off" => flag = Some(false),
                        _ => numbers.push(word.parse::<f32>().map_err(|_| format!("`{key}`: `{word}` is not a number"))?),
                    }
                }
                match flag {
                    Some(f) if numbers.is_empty() => Val::Flag(f),
                    _ => Val::Numbers(numbers),
                }
            }
        };
        params.push((key, value));
    }
}

impl Cursor<'_> {
    fn skip_space_inline(&mut self) {
        while self.peek().is_some_and(|b| b == b' ' || b == b'\t' || b == b'\r') {
            self.at += 1;
        }
    }
}

// ---- into Tequila -----------------------------------------------------------------

/// What an input wants from the node driving it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Want {
    Colour,
    Number,
}

struct Build<'a> {
    mtl: &'a Mtl,
    graph: MaterialGraph,
    textures: Vec<(TextureId, TextureSource)>,
    notes: Vec<String>,
    picture: &'a mut dyn FnMut(&str) -> Result<Option<PathBuf>, String>,
    scratch: &'a Path,
    /// Nodes already made, by KeyShot node and what was wanted of it.
    made: BTreeMap<(u32, bool), Option<(NodeId, u8)>>,
    /// Read from a scene (`.bip`) rather than a package: textures placed by
    /// their own frames, labels, measured metals by number, and an
    /// emissive's power. See [`convert_scene`].
    scene: bool,
}

fn convert(mtl: &Mtl, picture: &mut dyn FnMut(&str) -> Result<Option<PathBuf>, String>, scratch: &Path) -> Result<Converted, ImportError> {
    convert_with(mtl, picture, scratch, false)
}

/// A material of a KeyShot scene, its nodes already in the units this
/// converter reads: colours linear, roughness as the slider shows it
/// (`crate::bip_scene` does both). Beyond a package's conversion, a scene's
/// textures are placed by the frames they carry, its labels come over as
/// labels, a metal's measured preset is read by number, and an emissive
/// keeps its power.
pub(crate) fn convert_scene(mtl: &Mtl, picture: &mut dyn FnMut(&str) -> Result<Option<PathBuf>, String>, scratch: &Path) -> Result<Converted, ImportError> {
    convert_with(mtl, picture, scratch, true)
}

fn convert_with(mtl: &Mtl, picture: &mut dyn FnMut(&str) -> Result<Option<PathBuf>, String>, scratch: &Path, scene: bool) -> Result<Converted, ImportError> {
    let mut build = Build {
        mtl,
        graph: MaterialGraph::from_material(MaterialType::Plastic.defaults()),
        textures: Vec::new(),
        notes: Vec::new(),
        picture,
        scratch,
        made: BTreeMap::new(),
        scene,
    };
    let surface = mtl.surface.ok_or_else(|| ImportError::Malformed("no surface".into()))?;
    let material_type = build.surface(surface, 0)?;
    if mtl.geometry.and_then(|g| mtl.shaders.get(&g)).is_some_and(|g| g.kind != "edgeshader_disabled" || g.number("input").is_some()) {
        build.note("its displacement was left out; its bump or normal map carries the detail".into());
    }
    let category = category(material_type, &mtl.name);
    let mut graph = build.graph;
    let entry = LibraryEntry {
        name: mtl.name.clone(),
        category,
        tags: vec!["KeyShot".into()],
        material_type,
        material: graph.material().clone(),
        graph: std::mem::replace(&mut graph, MaterialGraph::from_material(Material::default())),
        textures: build.textures,
        source: Source::User,
        path: String::new(),
    };
    Ok(Converted { entry, notes: build.notes })
}

fn category(material_type: MaterialType, name: &str) -> String {
    let name = name.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    if has(&["wood", "oak", "walnut", "maple", "zebrawood", "ebony", "cherry", "bamboo"]) {
        return "Wood".into();
    }
    if has(&["fabric", "cloth", "weave", "textile", "mesh", "knit", "denim", "velvet", "felt"]) {
        return "Fabric".into();
    }
    if has(&["leather", "suede"]) {
        return "Leather".into();
    }
    if has(&["marble", "granite", "stone", "concrete", "terrazzo"]) {
        return "Stone".into();
    }
    match material_type {
        MaterialType::Metal | MaterialType::Anisotropic => "Metal",
        MaterialType::Paint | MaterialType::MetallicPaint => "Paint",
        MaterialType::Glass | MaterialType::GlassSolid | MaterialType::Dielectric => "Glass",
        MaterialType::Liquid => "Liquid",
        MaterialType::Gem => "Gem",
        MaterialType::Translucent | MaterialType::TranslucentMedium | MaterialType::ScatteringMedium => "Translucent",
        MaterialType::Velvet | MaterialType::RealCloth => "Fabric",
        MaterialType::Plastic | MaterialType::PlasticCloudy | MaterialType::PlasticTransparent => "Plastic",
        _ => "Other",
    }
    .into()
}

impl Build<'_> {
    fn shader(&self, n: u32) -> Result<&Shader, ImportError> {
        self.mtl.shaders.get(&n).ok_or_else(|| ImportError::Malformed(format!("the material refers to node {n}, which it does not define")))
    }

    fn note(&mut self, text: String) {
        if !self.notes.contains(&text) {
            self.notes.push(text);
        }
    }

    /// Make node `n` the surface, following the nodes that wrap one: a bump
    /// over a surface, a composite of layers. Returns the material type.
    fn surface(&mut self, n: u32, depth: u32) -> Result<MaterialType, ImportError> {
        if depth > 16 {
            return Err(ImportError::Malformed("the material's nodes loop".into()));
        }
        let shader = self.shader(n)?.clone();
        match shader.kind.as_str() {
            "lux_bumpmap" => {
                let inner = shader.number("shader").map(|v| v as u32);
                let material_type = match inner {
                    Some(inner) => self.surface(inner, depth + 1)?,
                    None => MaterialType::Plastic,
                };
                if let Some(bump) = shader.number("bumpmap").map(|v| v as u32) {
                    self.bump(bump, shader.number("bump_scale").unwrap_or(1.0))?;
                }
                Ok(material_type)
            }
            "lux_composite" | "lux_multi_layer" => {
                let layers = match shader.value("shaders") {
                    Some(Val::List(list)) => list.clone(),
                    _ => Vec::new(),
                };
                let Some(&top) = layers.last() else {
                    self.note("an empty composite became plain plastic".into());
                    return Ok(MaterialType::Plastic);
                };
                let material_type = self.surface(top, depth + 1)?;
                if self.scene && !shader.flag("opacity_map").unwrap_or(false) && layers.len() >= 2 {
                    // A scene's labels come first and the material they are
                    // printed on last; the first listed is the one on top,
                    // so they are laid down from the end.
                    for &layer in layers[..layers.len() - 1].iter().rev() {
                        self.label(layer, depth + 1)?;
                    }
                    return Ok(material_type);
                }
                if shader.flag("opacity_map").unwrap_or(false) && layers.len() >= 2 {
                    // The first layer's colour is the opacity map.
                    let mask = self.shader(layers[0])?.clone();
                    let input = ["diffuse", "color", "base_color", "base"].into_iter().find_map(|k| mask.link(k));
                    if let Some(input) = input {
                        if let Some(node) = self.drive(input, Want::Number, MapRole::Data)? {
                            let output = self.graph.output();
                            self.graph.connect_output(node.0, node.1, output, Socket::Param(Param::Opacity));
                        }
                    }
                } else if layers.len() >= 2 {
                    self.note("the material's layers were reduced to its top one".into());
                }
                Ok(material_type)
            }
            _ => self.base(&shader),
        }
    }

    /// A surface node's own parameters.
    fn base(&mut self, s: &Shader) -> Result<MaterialType, ImportError> {
        let kind = s.kind.as_str();
        let (material_type, mut m) = match kind {
            "lux_plastic" | "lux_moldtech_plastic" | "lux_plastic_simple" => (MaterialType::Plastic, MaterialType::Plastic.defaults()),
            "lux_plastic_transparent" | "lux_plastic_cloudy" => (MaterialType::PlasticTransparent, MaterialType::PlasticTransparent.defaults()),
            "lux_metal" | "lux_metal_coated" | "lux_metal_simple" => (MaterialType::Metal, MaterialType::Metal.defaults()),
            "lux_anisotropic" => (MaterialType::Anisotropic, MaterialType::Anisotropic.defaults()),
            "lux_paint" => (MaterialType::Paint, MaterialType::Paint.defaults()),
            "metallic_paint" | "lux_metallic_paint" => (MaterialType::MetallicPaint, MaterialType::MetallicPaint.defaults()),
            // A scene's glass is a solid one: light is coloured by how far it
            // travels through it.
            "lux_glass" | "lux_glass_simple" if self.scene => (MaterialType::GlassSolid, MaterialType::GlassSolid.defaults()),
            "lux_glass" | "lux_glass_simple" => (MaterialType::Glass, MaterialType::Glass.defaults()),
            "lux_solid_glass" | "lux_glass_solid" => (MaterialType::GlassSolid, MaterialType::GlassSolid.defaults()),
            "lux_dielectric" => (MaterialType::Dielectric, MaterialType::Dielectric.defaults()),
            "lux_liquid" => (MaterialType::Liquid, MaterialType::Liquid.defaults()),
            "lux_gem" => (MaterialType::Gem, MaterialType::Gem.defaults()),
            "lux_translucent" | "lux_translucent_medium" => (MaterialType::Translucent, MaterialType::Translucent.defaults()),
            "lux_velvet" => (MaterialType::Velvet, MaterialType::Velvet.defaults()),
            "lux_cloth" | "lux_realcloth" => (MaterialType::RealCloth, MaterialType::RealCloth.defaults()),
            "lux_thin_film" => (MaterialType::ThinFilm, MaterialType::ThinFilm.defaults()),
            "lux_diffuse" => (MaterialType::Diffuse, MaterialType::Diffuse.defaults()),
            "lux_emissive" | "lux_light_emissive" => (MaterialType::Emissive, MaterialType::Emissive.defaults()),
            "lux_flat" => (MaterialType::Flat, MaterialType::Flat.defaults()),
            "lux_generic" => (MaterialType::Generic, MaterialType::Generic.defaults()),
            "lux_advanced" => (MaterialType::Advanced, MaterialType::Advanced.defaults()),
            "lux_toon" => {
                self.toon(s);
                return Ok(MaterialType::Flat);
            }
            k if k.starts_with("bsdf_axf") => {
                // The measured material itself, from the AxF file the
                // package carries.
                let measured = s.text("bsdf").map(|file| (self.picture)(file).map_err(ImportError::Io)).transpose()?.flatten();
                let read = measured.as_deref().map(|path| std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| crate::axf::read(&bytes)));
                match read {
                    Some(Ok(svbrdf)) => return self.measured(s, &svbrdf),
                    Some(Err(e)) => self.note(format!("its measured AxF data could not be read ({e})")),
                    None => {}
                }
                let carpaint = k.contains("carpaint");
                let mut m = if carpaint { MaterialType::MetallicPaint.defaults() } else { MaterialType::Plastic.defaults() };
                if let Some(colour) = self.mtl.icon.as_deref().and_then(icon_colour) {
                    m.base_color = colour;
                    if carpaint {
                        // A sparkle of the paint's own colour, not grey.
                        m.flake_color = (colour * 4.0).clamp(0.0, 1.0);
                        m.flake_weight = 0.15;
                    }
                }
                m.specular_roughness = if carpaint { 0.3 } else { 0.45 };
                self.note(format!("{} is a measured material (X-Rite AxF); it is approximated from the package's preview", self.mtl.name));
                self.graph.material_mut().clone_from(&m);
                let t = if carpaint { MaterialType::MetallicPaint } else { MaterialType::Plastic };
                return Ok(t);
            }
            other => {
                self.note(format!("its KeyShot surface `{other}` has no counterpart; it came in as plastic"));
                (MaterialType::Plastic, MaterialType::Plastic.defaults())
            }
        };

        // The numbers.
        let rough = |s: &Shader| s.number("roughness");
        match material_type {
            MaterialType::Metal | MaterialType::Anisotropic if self.scene => self.scene_metal(s, &mut m),
            MaterialType::MetallicPaint if self.scene => {
                // A scene's metallic paint: its roughness and index are the
                // clear coat's, over a base and metal flakes.
                if let Some(c) = s.colour("base") {
                    m.base_color = c;
                }
                if let Some(c) = s.colour("metal") {
                    m.flake_color = c;
                }
                if let Some(c) = s.number("metal_coverage") {
                    m.flake_weight = c.clamp(0.0, 1.0);
                }
                if let Some(r) = s.number("metal_roughness") {
                    m.flake_roughness = r.clamp(0.0, 1.0);
                    m.specular_roughness = r.clamp(0.0, 1.0);
                }
                // Flake size 0 is KeyShot's "too fine to see one by one":
                // the metal colour as an even sheen, which is what a fine
                // flake gives. A size is taken as millimetres.
                match s.number("metal_flake_size") {
                    Some(size) if size > 0.0 => m.flake_size = size * 0.001,
                    _ => m.flake_size = 0.000_005,
                }
                if s.number("metal_flake_visibility").is_some() {
                    self.note("a metallic paint's flake visibility has no counterpart; the flakes show as Tequila's do".into());
                }
                m.coat_weight = 1.0;
                if let Some(c) = s.colour("clearcoat") {
                    m.coat_color = c;
                }
                if let Some(r) = rough(s) {
                    m.coat_roughness = r.clamp(0.0, 1.0);
                }
                if let Some(ior) = s.number("ior") {
                    m.coat_ior = ior;
                }
                if s.number("thickness multiplier").is_some_and(|t| (t - 1.0).abs() > 1e-3) {
                    self.note(format!("{}'s clear-coat thickness has no counterpart; the coat is Tequila's own", self.mtl.name));
                }
            }
            MaterialType::Emissive if self.scene => {
                // The power is KeyShot's Intensity, as radiance beside the
                // environment's: an intensity of 1 glows as bright as a
                // white environment, which is how Tequila's emission and
                // its environment compare too.
                m.emission_color = s.colour("color").unwrap_or(Spectrum::ONE);
                m.emission_luminance = s.number("intensity").unwrap_or(1.0).max(0.0);
                // Tequila's glowing surfaces are seen directly and in
                // reflections and cast shadows like any other; KeyShot's
                // can each be switched off.
                let hidden: Vec<&str> = [("visible to the eye", "to the camera"), ("visible in reflections", "in reflections"), ("visible in shadows", "in shadows")]
                    .into_iter()
                    .filter(|(key, _)| s.flag(key) == Some(false))
                    .map(|(_, what)| what)
                    .collect();
                if !hidden.is_empty() {
                    self.note(format!("{} glows but is hidden {} in KeyShot; Tequila has no such switch, so it shows there", self.mtl.name, hidden.join(" and ")));
                }
            }
            MaterialType::Metal | MaterialType::Anisotropic => {
                if let Some(c) = s.colour("color") {
                    m.base_color = c;
                }
                // The colour is the reflectance: on a metal that is all
                // reflectance, so it is not dimmed by aluminium's again.
                m.metal_ior = ComplexIor { n: Spectrum::splat(0.01), k: Spectrum::splat(100.0) };
                if let Some(r) = rough(s) {
                    m.specular_roughness = r;
                }
                if kind == "lux_metal_coated" {
                    m.coat_weight = 1.0;
                }
            }
            MaterialType::MetallicPaint => {
                if let Some(c) = s.colour("base") {
                    m.base_color = c;
                }
                if let Some(c) = s.colour("metal") {
                    m.flake_color = c;
                }
                if let Some(c) = s.number("metal_coverage") {
                    m.flake_weight = c.clamp(0.0, 1.0);
                }
                if let Some(r) = s.number("metal_roughness") {
                    m.flake_roughness = r;
                }
                match s.number("metal_flake_size") {
                    Some(size) if size > 0.0 => m.flake_size = size * 0.001,
                    // KeyShot's 0: flakes too fine to see one by one, a
                    // pearly sheen over the paint (White pearl's) rather
                    // than glitter. At its full coverage the flakes' colour
                    // swamped the paint's.
                    Some(_) => {
                        m.flake_size = 0.000005;
                        m.flake_weight *= 0.25;
                    }
                    None => {}
                }
                if let Some(r) = rough(s) {
                    m.specular_roughness = r;
                }
                if let Some(ior) = s.number("ior") {
                    m.coat_ior = ior;
                }
            }
            MaterialType::Glass | MaterialType::GlassSolid | MaterialType::Dielectric | MaterialType::Liquid | MaterialType::PlasticTransparent => {
                if let Some(c) = s.colour("color").or_else(|| s.colour("transmission")) {
                    m.transmission_color = c;
                }
                if let Some(ior) = s.number("ior").or_else(|| s.number("refractive_index")) {
                    m.specular_ior = ior;
                }
                // The distance the colour is reached at, its unit unstated:
                // scaled so the wine KeyShot previews through 8 cm of it is
                // as pale a yellow here. A scene's is in metres: read so, a
                // thin smoked cover whose colour is nearly black lets through
                // what KeyShot's own render shows through it; at the
                // package's scale it is opaque.
                if let Some(d) = s.number("transparency").filter(|d| *d > 0.0) {
                    m.transmission_depth = if self.scene { d } else { d * 0.03 };
                }
                if let Some(r) = rough(s) {
                    m.specular_roughness = r;
                }
            }
            MaterialType::Generic => {
                if let Some(c) = s.colour("base_color") {
                    m.base_color = c;
                }
                if let Some(v) = s.number("metallic") {
                    m.base_metalness = v;
                }
                if let Some(r) = rough(s) {
                    m.specular_roughness = r;
                }
                if let Some(v) = s.number("specular") {
                    m.specular_weight = (v * 2.0).clamp(0.0, 1.0);
                }
                if let Some(v) = s.number("sheen") {
                    m.fuzz_weight = v;
                }
                if let Some(v) = s.number("clearcoat") {
                    m.coat_weight = v;
                }
                if let Some(v) = s.number("clearcoat_roughness") {
                    m.coat_roughness = v;
                }
                if let Some(v) = s.number("anisotropic") {
                    m.specular_roughness_anisotropy = v;
                }
            }
            _ => {
                let diffuse = s.colour("diffuse").or_else(|| s.colour("color")).or_else(|| s.colour("base"));
                if let Some(c) = diffuse {
                    m.base_color = c;
                }
                if let Some(c) = s.colour("specular") {
                    // Advanced's specular is a colour standing for strength
                    // and tint at once.
                    let peak = c.max_component();
                    m.specular_weight = peak.clamp(0.0, 1.0);
                    if peak > 1e-4 {
                        m.specular_color = c * (1.0 / peak);
                    }
                }
                if let Some(t) = s.colour("specular_transmission") {
                    let peak = t.max_component();
                    if peak > 0.01 {
                        m.transmission_weight = peak.clamp(0.0, 1.0);
                        m.transmission_color = t * (1.0 / peak);
                    }
                }
                if let Some(r) = rough(s) {
                    m.specular_roughness = r;
                }
                if let Some(ior) = s.number("ior").or_else(|| s.number("refractive_index")) {
                    m.specular_ior = ior;
                }
            }
        }
        if let Some(a) = s.number("alpha") {
            m.geometry_opacity = a.clamp(0.0, 1.0);
        }
        *self.graph.material_mut() = m;

        // The inputs driven by other nodes.
        let output = self.graph.output();
        let colour_param = if matches!(material_type, MaterialType::Glass | MaterialType::GlassSolid | MaterialType::Liquid | MaterialType::Dielectric) {
            Param::TransmissionColor
        } else if self.scene && material_type == MaterialType::Emissive {
            Param::EmissionColor
        } else {
            Param::BaseColor
        };
        let inputs: &[(&str, Param, Want, MapRole)] = &[
            ("color", colour_param, Want::Colour, MapRole::Color),
            ("diffuse", Param::BaseColor, Want::Colour, MapRole::Color),
            ("base_color", Param::BaseColor, Want::Colour, MapRole::Color),
            ("base", Param::BaseColor, Want::Colour, MapRole::Color),
            ("metal", Param::FlakeColor, Want::Colour, MapRole::Color),
            ("specular", Param::SpecularColor, Want::Colour, MapRole::Color),
            ("roughness", Param::SpecularRoughness, Want::Number, MapRole::Data),
            ("metallic", Param::BaseMetalness, Want::Number, MapRole::Data),
            ("alpha", Param::Opacity, Want::Number, MapRole::Data),
        ];
        for (key, param, want, role) in inputs.iter().copied() {
            let Some(from) = s.link(key) else { continue };
            // A metal's colour picture is decoded as the photograph it is,
            // as KeyShot decodes it. Read as raw numbers, Weathered Metal's
            // mid-grey steel came in nearly white and mirror-bright, where
            // KeyShot's own preview shows it dark and patchy.
            if let Some((node, socket)) = self.drive(from, want, role)? {
                self.graph.connect_output(node, socket, output, Socket::Param(param));
                if param == Param::SpecularColor {
                    self.graph.material_mut().specular_weight = 1.0;
                }
            }
        }
        Ok(material_type)
    }

    /// The node driving an input, made from KeyShot node `n`: a picture,
    /// or what a converter between types passes on.
    fn drive(&mut self, n: u32, want: Want, role: MapRole) -> Result<Option<(NodeId, u8)>, ImportError> {
        let key = (n, want == Want::Colour);
        if let Some(made) = self.made.get(&key) {
            return Ok(*made);
        }
        let made = self.make(n, want, role, 0)?;
        self.made.insert(key, made);
        Ok(made)
    }

    fn make(&mut self, n: u32, want: Want, role: MapRole, depth: u32) -> Result<Option<(NodeId, u8)>, ImportError> {
        if depth > 16 {
            return Err(ImportError::Malformed("the material's nodes loop".into()));
        }
        let s = self.shader(n)?.clone();
        let kind = s.kind.as_str();
        if kind.starts_with("texture") || kind == "triplanar" {
            let Some(image) = self.image(&s, role)? else { return Ok(None) };
            if want == Want::Number {
                let red = self.graph.add_node(Node::new(NodeKind::Channel { channel: Channel::Red }));
                self.graph.connect(image, red, Socket::Input(0));
                return Ok(Some((red, 0)));
            }
            return Ok(Some((image, 0)));
        }
        // Converters between kinds of value, which pass their input on.
        if kind.starts_with("edgeshader_") || kind == "convert_rgba_to_float" {
            let input = s.link("input").or_else(|| s.link("color")).or_else(|| s.value("input").and_then(|v| match v {
                Val::Numbers(v) if v.len() == 1 => Some(v[0] as u32),
                _ => None,
            }));
            let Some(input) = input else { return Ok(None) };
            let passed = self.make(input, want, role, depth + 1)?;
            if kind == "convert_rgba_to_float" {
                if let (Some((node, socket)), Some(a), Some(b), Some(c), Some(d)) =
                    (passed, s.number("input_from"), s.number("input_to"), s.number("output_from"), s.number("output_to"))
                {
                    let remap = self.graph.add_node(Node::new(NodeKind::Remap {
                        from: tequila_core::math::vec2(a, b),
                        to: tequila_core::math::vec2(c, d),
                    }));
                    self.graph.connect_output(node, socket, remap, Socket::Input(0));
                    return Ok(Some((remap, 0)));
                }
            }
            return Ok(passed);
        }
        // A colour on its own, which KeyShot 11 and later put in a node of
        // its own to name it.
        if matches!(kind, "lux_const_color_extended" | "lux_const_color" | "lux_color") {
            let c = s.colour("color").unwrap_or(Spectrum::splat(0.5));
            let value = match want {
                Want::Colour => Value::Color(c),
                Want::Number => Value::Float(c.luminance()),
            };
            return Ok(Some((self.graph.add_node(Node::new(NodeKind::Constant(value))), 0)));
        }
        if matches!(kind, "lux_noise" | "lux_fractal_noise") {
            let share = self.noise(&s);
            if want == Want::Number {
                return Ok(Some(share));
            }
            return Ok(Some(self.between(&s, share)));
        }
        if kind == "lux_color_adjust" {
            let input = s.link("color").or_else(|| s.link("input"));
            let Some(input) = input else { return Ok(None) };
            let Some(passed) = self.make(input, Want::Colour, role, depth + 1)? else { return Ok(None) };
            // Wired to a number, a colour is read as its luminance.
            return Ok(Some(self.adjust(&s, passed)));
        }
        if kind == "lux_marble" {
            let marble = self.marble(&s);
            if want == Want::Number {
                return Ok(Some((marble, 0)));
            }
            // Between its two colours, the second in the veins.
            let first = self.graph.add_node(Node::new(NodeKind::Constant(Value::Color(s.colour("color1").unwrap_or(Spectrum::splat(0.7))))));
            let second = self.graph.add_node(Node::new(NodeKind::Constant(Value::Color(s.colour("color2").unwrap_or(Spectrum::splat(0.2))))));
            let mix = self.graph.add_node(Node::new(NodeKind::Mix));
            self.graph.connect(first, mix, Socket::Input(0));
            self.graph.connect(second, mix, Socket::Input(1));
            self.graph.connect(marble, mix, Socket::Input(2));
            return Ok(Some((mix, 0)));
        }
        // A pattern: the colour it mostly shows, in its place - its first,
        // moved towards its second as far as its magnitude takes it.
        let pattern = ["color1", "color2"].iter().filter_map(|k| s.colour(k)).collect::<Vec<_>>();
        self.note(format!("the procedural `{kind}` has no counterpart; a flat colour stands in for it"));
        let value = if pattern.is_empty() {
            Value::Float(0.5)
        } else {
            let share = s.number("magnitude").map(|m| (m * 0.5).clamp(0.0, 1.0)).unwrap_or(0.5);
            let mean = match pattern.as_slice() {
                [a, b] => *a * (1.0 - share) + *b * share,
                _ => pattern[0],
            };
            match want {
                Want::Colour => Value::Color(mean),
                Want::Number => Value::Float(mean.luminance()),
            }
        };
        let constant = self.graph.add_node(Node::new(NodeKind::Constant(value)));
        Ok(Some((constant, 0)))
    }

    /// A picture node, mapped as the texture node says.
    fn image(&mut self, s: &Shader, role: MapRole) -> Result<Option<NodeId>, ImportError> {
        let Some((id, mapping)) = self.texture(s, role)? else { return Ok(None) };
        let coords = self.graph.add_node(Node::new(NodeKind::Mapping2d { mapping }));
        let fallback = match role {
            MapRole::Color => Value::Color(Spectrum::splat(0.5)),
            _ => Value::Float(0.5),
        };
        let image = self.graph.add_node(Node::new(NodeKind::ImageTexture { texture: id, fallback }));
        self.graph.connect(coords, image, Socket::Input(0));
        Ok(Some(image))
    }

    /// A texture node's picture, kept with the material, and where it goes.
    fn texture(&mut self, s: &Shader, role: MapRole) -> Result<Option<(TextureId, Mapping)>, ImportError> {
        let file = s.text("texture").or_else(|| s.text("textureX")).or_else(|| s.text("textureY")).or_else(|| s.text("textureZ"));
        let Some(file) = file else { return Ok(None) };
        if file.to_ascii_lowercase().ends_with(".ktx") {
            self.note(format!(
                "its texture {} is in KeyShot's own protected format (as Mold-Tech's are), which Tequila does not read; the finish came in without it",
                base_name(file)
            ));
            return Ok(None);
        }
        let Some(mut path) = (self.picture)(file).map_err(ImportError::Io)? else {
            self.note(format!("its texture {} is not in the package", base_name(file)));
            return Ok(None);
        };
        // A floating-point normal map with its normals as they are, -1 to 1,
        // rather than encoded 0 to 1 as normal maps are: encoded.
        if role == MapRole::Normal {
            if let Some(encoded) = self.encode_signed_normals(&path)? {
                path = encoded;
            }
        }
        // DirectX-convention normal maps, turned to the renderer's.
        if role == MapRole::Normal && base_name(file).to_ascii_lowercase().contains("normaldx") {
            let flipped = self.scratch.join(format!("{}-gl.png", path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()));
            flip_green(&path, &flipped)?;
            path = flipped;
        }
        // The same picture for the same use is kept once: a label and the
        // emission it drives read one file.
        let id = match self.textures.iter().find(|(_, t)| t.path == path && t.role == role) {
            Some((id, _)) => *id,
            None => {
                let id = TextureId(self.textures.len() as u32 + 1);
                self.textures.push((id, TextureSource::new(path, role)));
                id
            }
        };
        let mapping = self.mapping(s);
        Ok(Some((id, mapping)))
    }

    /// The normal map at `path`, encoded as normal maps are, when it is a
    /// floating-point picture of normals as they are: `None` for one that
    /// is not.
    fn encode_signed_normals(&mut self, path: &std::path::Path) -> Result<Option<PathBuf>, ImportError> {
        let extension = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        // The formats that can hold floats; a TIFF only sometimes does, and
        // an integer one never has a negative value to find below.
        if !matches!(extension.as_str(), "exr" | "hdr" | "tif" | "tiff") {
            return Ok(None);
        }
        let Ok(image) = tequila_texture::Image::load_raw(path) else { return Ok(None) };
        if image.channels < 3 || !image.data.iter().any(|v| *v < -0.01) {
            return Ok(None);
        }
        let map = crate::axf::Map::Image { width: image.width, height: image.height, channels: image.channels, data: image.data };
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let encoded = crate::axf::write_png(&map, crate::axf::Encoding::Normal, &self.scratch.join(format!("{stem}-encoded.png"))).map_err(ImportError::Io)?;
        Ok(Some(encoded))
    }

    /// Where a texture node puts its picture.
    fn mapping(&mut self, s: &Shader) -> Mapping {
        let box_of = |u: f32, v: f32| Mapping {
            projection: Projection::TriPlanar,
            real_world_units: true,
            scale: vec3(u.abs().max(1e-5), v.abs().max(1e-5), 1.0),
            ..Mapping::default()
        };
        if s.kind == "triplanar" {
            let u = s.number("uScaleY").or_else(|| s.number("uScaleX")).unwrap_or(0.1);
            let v = s.number("vScaleY").or_else(|| s.number("vScaleX")).unwrap_or(u);
            return box_of(u, v);
        }
        if self.scene {
            return self.scene_mapping(s);
        }
        // A separate mapping node, or the texture's own.
        let m = s.number("uv_shader").and_then(|n| self.mtl.shaders.get(&(n as u32))).cloned().unwrap_or_else(|| s.clone());
        if let Some(projection) = m.number("uv_projection") {
            let (u, v) = (m.number("uv_scale_u").unwrap_or(1.0), m.number("uv_scale_v").unwrap_or(1.0));
            let mut mapping = match projection as i32 {
                // The mesh's UVs: KeyShot's scale is the texture's size,
                // so repeats are its inverse.
                0 => Mapping {
                    projection: Projection::Uv,
                    scale: vec3(1.0 / u.abs().max(1e-5), 1.0 / v.abs().max(1e-5), 1.0),
                    ..Mapping::default()
                },
                1 => box_of(u, v),
                other => {
                    self.note(format!("a texture's projection ({other}) was taken as box mapping"));
                    box_of(u, v)
                }
            };
            mapping.rotation = m.number("uv_angle").unwrap_or(0.0) / 360.0;
            mapping.offset = tequila_core::math::vec2(m.number("uv_shift_u").unwrap_or(0.0), m.number("uv_shift_v").unwrap_or(0.0));
            mapping.wrap_u = Wrap::Repeat;
            mapping.wrap_v = Wrap::Repeat;
            return mapping;
        }
        // The oldest packages: sizes in units they do not state.
        let (u, v) = (s.number("scale_u").unwrap_or(1.0), s.number("scale_v").unwrap_or(1.0));
        let mut mapping = box_of(u * OLD_UNIT, v * OLD_UNIT);
        mapping.flip_u = u < 0.0;
        mapping.flip_v = v < 0.0;
        mapping.rotation = s.number("angle").unwrap_or(0.0) / 360.0;
        self.note("its texture size was estimated (the package is from an older KeyShot that does not say its units)".into());
        mapping
    }

    /// A scene's Metal: a colour (preset 0) or one of KeyShot's measured
    /// metals, numbered in the order its list gives them, and an optional
    /// anodised film. The numbering is read from the list as KeyShot's
    /// manual gives it, with 0 its colour mode: a scene's files say only the
    /// number.
    fn scene_metal(&mut self, s: &Shader, m: &mut Material) {
        use tequila_materials::fresnel::metals;
        use tequila_core::spectrum::spectrum;
        const LIST: [&str; 13] = ["Aluminum", "Brass", "Chromium", "Copper", "Gold", "Iron", "Magnesium", "Nickel", "Niobium", "Platinum", "Silver", "Titanium", "Zinc"];
        let preset = s.number("preset").map(|p| p as i32).unwrap_or(0);
        let colour = s.colour("color").unwrap_or(Spectrum::splat(0.8));
        // Reflectance at normal incidence and at grazing for the metals
        // Tequila has not measured: handbook values, near enough to tell
        // brass from zinc.
        let fitted = |r: [f32; 3], g: [f32; 3]| ComplexIor::from_reflectivity_and_edge_tint(spectrum(r[0], r[1], r[2]), spectrum(g[0], g[1], g[2]));
        let named = usize::try_from(preset).ok().and_then(|p| p.checked_sub(1)).and_then(|i| LIST.get(i)).copied();
        match named {
            Some(name) => {
                m.base_color = Spectrum::ONE;
                m.metal_ior = match name {
                    "Aluminum" => metals::ALUMINIUM,
                    "Chromium" => metals::CHROMIUM,
                    "Copper" => metals::COPPER,
                    "Gold" => metals::GOLD,
                    "Iron" => metals::IRON,
                    "Nickel" => metals::NICKEL,
                    "Platinum" => metals::PLATINUM,
                    "Silver" => metals::SILVER,
                    "Titanium" => metals::TITANIUM,
                    "Brass" => fitted([0.91, 0.78, 0.42], [0.96, 0.90, 0.70]),
                    "Magnesium" => fitted([0.84, 0.83, 0.82], [0.95, 0.94, 0.93]),
                    "Niobium" => fitted([0.62, 0.60, 0.56], [0.85, 0.83, 0.80]),
                    _ => fitted([0.66, 0.82, 0.85], [0.90, 0.95, 0.96]),
                };
                if matches!(name, "Brass" | "Magnesium" | "Niobium" | "Zinc") {
                    self.note(format!("{name} is not one of Tequila's measured metals; it was fitted from its reflectance"));
                }
                self.note(format!("a Metal's preset number was read as KeyShot's list (preset {preset} as {name})"));
            }
            None => {
                if preset != 0 {
                    self.note(format!("a Metal's preset {preset} is past KeyShot's list; its colour was used"));
                }
                // Its colour is its reflectance, as for a package's metal.
                m.base_color = colour;
                m.metal_ior = ComplexIor { n: Spectrum::splat(0.01), k: Spectrum::splat(100.0) };
            }
        }
        if let Some(r) = s.number("roughness") {
            m.specular_roughness = r.clamp(0.0, 1.0);
        }
        // Anodised: a film on the metal, its thickness in nanometres as
        // Tequila's is.
        if s.flag("coated") == Some(true) {
            m.thin_film_weight = 1.0;
            m.thin_film_thickness = s.number("film_thickness").unwrap_or(100.0).clamp(0.0, 2000.0);
            m.thin_film_ior = s.number("film_ior").unwrap_or(1.5).max(1.0);
            if s.number("film_extinction").is_some_and(|k| k > 0.0) {
                self.note("an anodised film's extinction has no counterpart; the film is clear".into());
            }
        }
    }

    /// A label of a scene's composite: a material printed through its
    /// picture. A glowing one becomes the glow of the material it is
    /// printed on, coloured by its picture where it is placed: Tequila's
    /// labels print and reflect but do not glow. Any other becomes one of
    /// Tequila's labels of its picture, with its placement.
    fn label(&mut self, n: u32, depth: u32) -> Result<(), ImportError> {
        if depth > 16 {
            return Err(ImportError::Malformed("the material's nodes loop".into()));
        }
        let layer = self.shader(n)?.clone();
        let inner = if layer.kind == "lux_label" { layer.number("shader").map(|v| v as u32) } else { Some(n) };
        let Some(inner) = inner else {
            self.note("a label printing nothing was left out".into());
            return Ok(());
        };
        let s = self.shader(inner)?.clone();
        let texture = ["color", "diffuse", "base", "base_color"]
            .iter()
            .find_map(|k| s.link(k))
            .and_then(|t| self.mtl.shaders.get(&t))
            .filter(|t| t.kind.starts_with("texture"))
            .cloned();
        if matches!(s.kind.as_str(), "lux_emissive" | "lux_light_emissive") {
            let output = self.graph.output();
            {
                let m = self.graph.material_mut();
                m.emission_luminance = s.number("intensity").unwrap_or(1.0).max(0.0);
                m.emission_color = s.colour("color").unwrap_or(Spectrum::ONE);
            }
            if let Some(texture) = &texture {
                if let Some(image) = self.image(texture, MapRole::Color)? {
                    self.graph.connect(image, output, Socket::Param(Param::EmissionColor));
                }
            }
            self.note(format!("{}'s glowing label became the glow of the material it is printed on, coloured by its picture", self.mtl.name));
            return Ok(());
        }
        let Some(texture) = texture else {
            self.note(format!("a label of {} has no picture and was left out", self.mtl.name));
            return Ok(());
        };
        let Some((id, mapping)) = self.texture(&texture, MapRole::Color)? else { return Ok(()) };
        let name = texture.text("texture").map(|t| {
            let file = base_name(t);
            file.rsplit_once('.').map_or(file, |(stem, _)| stem).to_string()
        });
        let mut decal = tequila_materials::decal::Decal::new(name.unwrap_or_else(|| "Label".into()), id);
        decal.mapping = mapping;
        decal.roughness = s.number("roughness").unwrap_or(0.35).clamp(0.0, 1.0);
        decal.metallic = s.kind.contains("metal");
        if !decal.is_valid() {
            self.note(format!("a label of {} could not be placed and was left out", self.mtl.name));
            return Ok(());
        }
        self.graph.decals.push(decal);
        Ok(())
    }

    /// Where a scene's texture goes. KeyShot's projections are numbered as
    /// its Mapping list gives them - planar, box, cylindrical, spherical,
    /// UV - and each carries the frame it was placed in: object
    /// coordinates in metres, multiplied on the left by
    /// `transform_obj_to_uv`, are the texture's space, in which one picture
    /// is `uv_scale` across. A planar picture is centred on that frame's
    /// origin, looking down its second axis, its width along the third and
    /// its height along the first: how a scene's label lands on the part it
    /// was made for.
    fn scene_mapping(&mut self, s: &Shader) -> Mapping {
        let positive = |v: Option<f32>| v.map(f32::abs).filter(|v| *v > 1e-6);
        let su = positive(s.number("uv_scale_u")).unwrap_or(1.0);
        let sv = positive(s.number("uv_scale_v")).unwrap_or(su);
        let frame: Option<[f32; 16]> = match s.value("transform_obj_to_uv") {
            Some(Val::Numbers(m)) if m.len() == 16 && m.iter().all(|v| v.is_finite()) => m.as_slice().try_into().ok(),
            _ => None,
        };
        // Bit 0x400 is set on a label and not on a tiling texture.
        let repeat = s.number("uv_flags").map(|f| (f as i32) & 0x400 == 0).unwrap_or(true);
        let wrap = if repeat { Wrap::Repeat } else { Wrap::Clamp };
        let shift = tequila_core::math::vec2(s.number("uv_shift_u").unwrap_or(0.0), s.number("uv_shift_v").unwrap_or(0.0));
        let angle = s.number("uv_angle").unwrap_or(0.0).to_radians();
        let projection = s.number("uv_projection").map(|p| p as i32).unwrap_or(0);
        let identity = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        let m = frame.unwrap_or(identity);
        // The object axis most nearly along each of the frame's.
        let column = |j: usize| [m[j], m[4 + j], m[8 + j]];
        let along = |c: [f32; 3]| (0..3).max_by(|&a, &b| c[a].abs().total_cmp(&c[b].abs())).unwrap_or(2);
        let axis_of = |i: usize| match i {
            0 => Axis::X,
            1 => Axis::Y,
            _ => Axis::Z,
        };
        let normal = column(1);
        match projection {
            0 => {
                let axis = along(normal);
                let (ia, ib) = match axis {
                    0 => (1, 2),
                    1 => (0, 2),
                    _ => (0, 1),
                };
                // u and v as KeyShot has them, from the two object axes
                // across the projection: [u v] = L [a b] + c.
                let (cu, cv) = (column(2), column(0));
                let mut l = [[cu[ia] / su, cu[ib] / su], [cv[ia] / sv, cv[ib] / sv]];
                let mut c = [m[14] / su + 0.5, m[12] / sv + 0.5];
                // Its own turn about the picture's centre, then its shift.
                if angle != 0.0 {
                    let (sin, cos) = angle.sin_cos();
                    let turn = |p: [f32; 2]| [p[0] * cos - p[1] * sin, p[0] * sin + p[1] * cos];
                    let (c0, c1) = (turn([l[0][0], l[1][0]]), turn([l[0][1], l[1][1]]));
                    l = [[c0[0], c1[0]], [c0[1], c1[1]]];
                    let centred = turn([c[0] - 0.5, c[1] - 0.5]);
                    c = [centred[0] + 0.5, centred[1] + 0.5];
                }
                c = [c[0] + shift.x, c[1] + shift.y];
                // Tequila's planar placement is a scale, a turn and a
                // mirror: L = F R S, so S is its columns' lengths, and what
                // is left a rotation or a rotation mirrored.
                let s1 = (l[0][0] * l[0][0] + l[1][0] * l[1][0]).sqrt();
                let s2 = (l[0][1] * l[0][1] + l[1][1] * l[1][1]).sqrt();
                if !(s1 > 1e-9 && s2 > 1e-9) {
                    self.note("a texture's planar frame is degenerate; it was placed square to the part".into());
                    return Mapping { projection: Projection::Planar, axis: axis_of(axis), scale: vec3(1.0 / su, 1.0 / sv, 1.0), wrap_u: wrap, wrap_v: wrap, ..Mapping::default() };
                }
                let q = [[l[0][0] / s1, l[0][1] / s2], [l[1][0] / s1, l[1][1] / s2]];
                let mirrored = q[0][0] * q[1][1] - q[0][1] * q[1][0] < 0.0;
                let r = if mirrored { [q[0], [-q[1][0], -q[1][1]]] } else { q };
                if (q[0][0] * q[0][1] + q[1][0] * q[1][1]).abs() > 0.05 {
                    self.note("a texture's planar frame is sheared; it was placed without the shear".into());
                }
                let rotation = r[1][0].atan2(r[0][0]) / std::f32::consts::TAU;
                let offset = tequila_core::math::vec2(c[0], if mirrored { -c[1] } else { c[1] });
                Mapping {
                    projection: Projection::Planar,
                    axis: axis_of(axis),
                    scale: vec3(s1, s2, 1.0),
                    rotation,
                    offset,
                    flip_v: mirrored,
                    wrap_u: wrap,
                    wrap_v: wrap,
                    ..Mapping::default()
                }
            }
            4 => Mapping {
                projection: Projection::Uv,
                scale: vec3(1.0 / su, 1.0 / sv, 1.0),
                offset: shift,
                rotation: angle / std::f32::consts::TAU,
                wrap_u: wrap,
                wrap_v: wrap,
                ..Mapping::default()
            },
            2 | 3 => {
                self.note(format!("a {} texture was wrapped about its frame's axis; its size round the part was estimated", if projection == 2 { "cylindrical" } else { "spherical" }));
                Mapping {
                    projection: if projection == 2 { Projection::Cylindrical } else { Projection::Spherical },
                    axis: axis_of(along(normal)),
                    scale: vec3(1.0, 1.0 / sv, 1.0),
                    offset: shift,
                    wrap_u: wrap,
                    wrap_v: wrap,
                    ..Mapping::default()
                }
            }
            other => {
                if other != 1 {
                    self.note(format!("a texture's projection ({other}) was taken as box mapping"));
                }
                Mapping {
                    projection: Projection::TriPlanar,
                    real_world_units: true,
                    scale: vec3(su, sv, 1.0),
                    rotation: angle / std::f32::consts::TAU,
                    offset: shift,
                    ..Mapping::default()
                }
            }
        }
    }

    /// A measured AxF material: its diffuse colour, GGX lobe, Fresnel and
    /// normals, maps of the scanned sample tiled at the sample's size, or
    /// single values. A car paint's flakes, which its SVBRDF leaves out,
    /// are Tequila's own over it.
    fn measured(&mut self, s: &Shader, svbrdf: &crate::axf::Svbrdf) -> Result<MaterialType, ImportError> {
        use crate::axf::{write_png, Encoding, Map};
        let car_paint = svbrdf.car_paint || s.kind.contains("carpaint");
        let material_type = if car_paint { MaterialType::MetallicPaint } else { MaterialType::Plastic };
        let mut m = material_type.defaults();
        let linear = |v: &[f32]| Spectrum::from_linear_srgb(vec3(v[0], *v.get(1).unwrap_or(&v[0]), *v.get(2).unwrap_or(&v[0])));
        // The tile: as the package places it, else the sample's own size.
        let size = (
            s.number("uv_scale_u").filter(|v| *v > 0.0).unwrap_or(svbrdf.size.0),
            s.number("uv_scale_v").filter(|v| *v > 0.0).unwrap_or(svbrdf.size.1),
        );
        let mapping = Mapping {
            projection: Projection::TriPlanar,
            real_world_units: true,
            scale: vec3(size.0.max(1e-5), size.1.max(1e-5), 1.0),
            ..Mapping::default()
        };
        let stem: String = svbrdf.name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
        let output = self.graph.output();

        // Fresnel's reflectance at normal incidence as an index, and the
        // specular colour's strength as the weight that scales it.
        let f0 = svbrdf.fresnel.clamp(0.001, 0.5).sqrt();
        m.specular_ior = ((1.0 + f0) / (1.0 - f0)).clamp(1.0, 3.0);
        let specular = svbrdf.specular.mean();
        let strength = specular.iter().copied().fold(0.0f32, f32::max);
        m.specular_weight = strength.clamp(0.0, 1.0);
        if strength > 1e-4 {
            m.specular_color = linear(&specular.iter().map(|v| v / strength).collect::<Vec<_>>());
        }
        match &svbrdf.diffuse {
            Map::Constant(v) => m.base_color = linear(v),
            image => {
                m.base_color = linear(&image.mean());
                let path = write_png(image, Encoding::Colour, &self.scratch.join(format!("{stem}-diffuse.png"))).map_err(ImportError::Io)?;
                let node = self.picture_node(path, MapRole::Color, mapping);
                self.graph.connect(node, output, Socket::Param(Param::BaseColor));
            }
        }
        match &svbrdf.lobes {
            Map::Constant(v) => m.specular_roughness = v.first().copied().unwrap_or(0.1).max(0.0).sqrt(),
            image => {
                m.specular_roughness = image.mean().first().copied().unwrap_or(0.1).max(0.0).sqrt();
                let path = write_png(image, Encoding::Roughness, &self.scratch.join(format!("{stem}-roughness.png"))).map_err(ImportError::Io)?;
                let node = self.picture_node(path, MapRole::Data, mapping);
                let red = self.graph.add_node(Node::new(NodeKind::Channel { channel: Channel::Red }));
                self.graph.connect(node, red, Socket::Input(0));
                self.graph.connect(red, output, Socket::Param(Param::SpecularRoughness));
            }
        }
        if let Some(normal) = &svbrdf.normal {
            let path = write_png(normal, Encoding::Normal, &self.scratch.join(format!("{stem}-normal.png"))).map_err(ImportError::Io)?;
            let node = self.picture_node(path, MapRole::Normal, mapping);
            self.graph.connect(node, output, Socket::Param(Param::Bump));
        }
        if car_paint {
            // A powder or car paint's sparkle over the measured coat, which
            // has no clear coat of its own: fitted to the flakes the file
            // measured (`axf::Flakes`), else fine silver ones of Tequila's.
            // Sparse either way: at a quarter's coverage flakes alone turned
            // a black paint satin grey.
            m.coat_weight = 0.0;
            m.flake_size = 0.000025;
            m.flake_roughness = 0.2;
            match &svbrdf.flakes {
                Some(flakes) => {
                    m.flake_color = linear(&flakes.colour);
                    let reflectance = m.flake_color.luminance().max(0.05);
                    m.flake_weight = (flakes.coverage_reflectance / reflectance).clamp(0.002, 0.5);
                    m.flake_spread = (flakes.cone_degrees / 60.0).clamp(0.02, 1.0);
                    m.flake_size = flakes.size;
                }
                None => {
                    m.flake_weight = 0.06;
                    m.flake_color = Spectrum::splat(0.5);
                    self.note(format!("{}'s flakes are Tequila's own: the measured ones in its AxF file did not read", self.mtl.name));
                }
            }
        }
        m.geometry_opacity = 1.0;
        *self.graph.material_mut() = m;
        Ok(material_type)
    }

    /// A picture node for a file written here, placed by `mapping`.
    fn picture_node(&mut self, path: PathBuf, role: MapRole, mapping: Mapping) -> NodeId {
        let id = TextureId(self.textures.len() as u32 + 1);
        self.textures.push((id, TextureSource::new(path, role)));
        let coords = self.graph.add_node(Node::new(NodeKind::Mapping2d { mapping }));
        let fallback = match role {
            MapRole::Color => Value::Color(Spectrum::splat(0.5)),
            _ => Value::Float(0.5),
        };
        let image = self.graph.add_node(Node::new(NodeKind::ImageTexture { texture: id, fallback }));
        self.graph.connect(coords, image, Socket::Input(0));
        image
    }

    /// A height in scene units from node `n`, for adding to another: a
    /// procedural's or a height map's, not a normal map's.
    fn height(&mut self, n: u32, scale: f32) -> Result<Option<(NodeId, u8)>, ImportError> {
        let s = self.shader(n)?.clone();
        let depth = match s.kind.as_str() {
            // A scene's: as deep as its features are wide, at a height of
            // 1, so a millimetre-wide noise at a height of 0.1 is a tenth of
            // a millimetre deep. At a package's depth a moulding KeyShot
            // renders speckled came out smooth.
            "lux_noise" | "lux_fractal_noise" if self.scene => s.number("scale").filter(|v| *v > 0.0).unwrap_or(0.01),
            "lux_noise" | "lux_fractal_noise" => PROCEDURAL_BUMP,
            k if k.starts_with("texture") && !k.contains("normal") && s.number("bump_mode") != Some(1.0) => self.bump_depth(&s),
            _ => return Ok(None),
        };
        let share = match s.kind.as_str() {
            "lux_noise" | "lux_fractal_noise" => self.noise(&s),
            _ => {
                let Some(image) = self.image(&s, MapRole::Height)? else { return Ok(None) };
                let red = self.graph.add_node(Node::new(NodeKind::Channel { channel: Channel::Red }));
                self.graph.connect(image, red, Socket::Input(0));
                (red, 0)
            }
        };
        let height = s.number("bump_height").unwrap_or(1.0) * scale;
        let bump = self.graph.add_node(Node::new(NodeKind::Bump { depth: depth * height }));
        self.graph.connect_output(share.0, share.1, bump, Socket::Input(0));
        Ok(Some((bump, 0)))
    }

    /// KeyShot's Noise and Fractal Noise, as Blender's noise: a share from
    /// 0 to 1 of the way from its first colour to its second. Its `scale` is
    /// the size of a feature in metres; Fractal Noise's levels are octaves,
    /// and its fall-off how much each is weaker than the last. Noise's
    /// magnitude is how far towards the second colour it goes.
    fn noise(&mut self, s: &Shader) -> (NodeId, u8) {
        let size = s.number("scale").filter(|v| *v > 0.0).unwrap_or(0.01);
        let fractal = s.kind == "lux_fractal_noise";
        let detail = if fractal { (s.number("levels").unwrap_or(3.0) - 1.0).clamp(0.0, 15.0) } else { 1.0 };
        let roughness = if fractal { s.number("fall-off").unwrap_or(0.5).clamp(0.0, 1.0) } else { 0.5 };
        let noise = self.graph.add_node(
            Node::new(NodeKind::Node(ShaderNode::noise()))
                .with_literal(Socket::Input(2), Value::Float(1.0 / size))
                .with_literal(Socket::Input(3), Value::Float(detail))
                .with_literal(Socket::Input(4), Value::Float(roughness)),
        );
        if fractal {
            return (noise, 0);
        }
        let magnitude = s.number("magnitude").unwrap_or(1.0);
        let scaled = self.graph.add_node(
            Node::new(NodeKind::Node(ShaderNode::Math { op: MathOp::Multiply, clamp: true })).with_literal(Socket::Input(1), Value::Float(magnitude)),
        );
        self.graph.connect(noise, scaled, Socket::Input(0));
        (scaled, 0)
    }

    /// A procedural's two colours, `share` of the way from the first.
    fn between(&mut self, s: &Shader, share: (NodeId, u8)) -> (NodeId, u8) {
        let first = self.graph.add_node(Node::new(NodeKind::Constant(Value::Color(s.colour("color1").unwrap_or(Spectrum::ONE)))));
        let second = self.graph.add_node(Node::new(NodeKind::Constant(Value::Color(s.colour("color2").unwrap_or(Spectrum::ZERO)))));
        let mix = self.graph.add_node(Node::new(NodeKind::Mix));
        self.graph.connect(first, mix, Socket::Input(0));
        self.graph.connect(second, mix, Socket::Input(1));
        self.graph.connect_output(share.0, share.1, mix, Socket::Input(2));
        (mix, 0)
    }

    /// KeyShot's Color Adjust: the colour tinted by its colorize colour,
    /// its hue turned (a whole turn is 1), its saturation and value scaled,
    /// and its contrast.
    fn adjust(&mut self, s: &Shader, input: (NodeId, u8)) -> (NodeId, u8) {
        let mut colour = input;
        if let Some(tint) = s.colour("colorize").filter(|c| (c.r - 1.0).abs() + (c.g - 1.0).abs() + (c.b - 1.0).abs() > 1e-3) {
            let tint = self.graph.add_node(Node::new(NodeKind::Constant(Value::Color(tint))));
            let multiply = self.graph.add_node(Node::new(NodeKind::Multiply));
            self.graph.connect_output(colour.0, colour.1, multiply, Socket::Input(0));
            self.graph.connect(tint, multiply, Socket::Input(1));
            colour = (multiply, 0);
        }
        let hue = s.number("hue").unwrap_or(0.0);
        let saturation = s.number("saturation").unwrap_or(1.0);
        let value = s.number("value").unwrap_or(1.0);
        if hue != 0.0 || saturation != 1.0 || value != 1.0 {
            let hsv = self.graph.add_node(
                Node::new(NodeKind::Node(ShaderNode::HueSaturationValue))
                    .with_literal(Socket::Input(0), Value::Float((0.5 + hue).rem_euclid(1.0)))
                    .with_literal(Socket::Input(1), Value::Float(saturation))
                    .with_literal(Socket::Input(2), Value::Float(value))
                    .with_literal(Socket::Input(3), Value::Float(1.0)),
            );
            self.graph.connect_output(colour.0, colour.1, hsv, Socket::Input(4));
            colour = (hsv, 0);
        }
        let contrast = s.number("contrast").unwrap_or(1.0);
        if (contrast - 1.0).abs() > 1e-3 {
            let node = self.graph.add_node(
                Node::new(NodeKind::Node(ShaderNode::BrightContrast)).with_literal(Socket::Input(2), Value::Float((contrast - 1.0).clamp(-1.0, 1.0))),
            );
            self.graph.connect_output(colour.0, colour.1, node, Socket::Input(0));
            colour = (node, 0);
        }
        colour
    }

    /// KeyShot's Toon: flat tones and outlines. Tequila has no cel shading,
    /// so the tones are one flat colour halfway between lit and shadowed,
    /// as see-through as the material is, and the outline is drawn where
    /// the surface turns away from the viewer - a silhouette's contour,
    /// though not the edges inside it.
    fn toon(&mut self, s: &Shader) {
        let mut m = MaterialType::Flat.defaults();
        let lit = s.colour("color").unwrap_or(Spectrum::splat(0.5));
        let shadow = s.colour("shadow color").unwrap_or(lit);
        let fill = lit * 0.5 + shadow * 0.5;
        let clear = (1.0 - s.number("transparency").unwrap_or(0.0)).clamp(0.0, 1.0);
        m.emission_color = fill;
        m.geometry_opacity = clear;
        *self.graph.material_mut() = m;
        let outline = s.flag("outline contour").unwrap_or(true) || s.flag("part contour").unwrap_or(true);
        let contour = s.colour("contour color");
        if let (true, Some(contour)) = (outline, contour) {
            // How far the surface faces away, 0 facing the viewer to 1 edge
            // on; the outline is the last stretch of it, the wider the
            // thicker the contour asked for.
            let width = s.number("contour width").unwrap_or(1.0).clamp(0.2, 8.0) * s.number("outline width multiplier").unwrap_or(1.0).clamp(0.5, 4.0);
            let start = (1.0 - 0.05 * width).clamp(0.5, 0.97);
            let facing = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::LayerWeight)).with_literal(Socket::Input(0), Value::Float(0.5)));
            let silhouette = self.graph.add_node(
                Node::new(NodeKind::Node(ShaderNode::MapRange { kind: MapRangeKind::Smoothstep, clamp: true }))
                    .with_literal(Socket::Input(1), Value::Float(start))
                    .with_literal(Socket::Input(2), Value::Float((start + 0.04).min(1.0)))
                    .with_literal(Socket::Input(3), Value::Float(0.0))
                    .with_literal(Socket::Input(4), Value::Float(1.0)),
            );
            self.graph.connect_output(facing, 1, silhouette, Socket::Input(0));
            // And the creases KeyShot draws inside it: wherever the part
            // bends sharply, either way, within a millimetre.
            let mut edge = silhouette;
            if s.flag("interior edge contour").unwrap_or(true) || s.flag("material contour").unwrap_or(true) {
                let convex = self.graph.add_node(Node::new(NodeKind::Curvature {
                    mode: tequila_materials::graph::CurvatureMode::Convex,
                    radius: 0.001 * width.max(1.0),
                    contrast: 3.0,
                }));
                let concave = self.graph.add_node(Node::new(NodeKind::Curvature {
                    mode: tequila_materials::graph::CurvatureMode::Concave,
                    radius: 0.001 * width.max(1.0),
                    contrast: 3.0,
                }));
                let creases = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::Math { op: MathOp::Add, clamp: true })));
                self.graph.connect(convex, creases, Socket::Input(0));
                self.graph.connect(concave, creases, Socket::Input(1));
                let both = self.graph.add_node(Node::new(NodeKind::Node(ShaderNode::Math { op: MathOp::Maximum, clamp: true })));
                self.graph.connect(silhouette, both, Socket::Input(0));
                self.graph.connect(creases, both, Socket::Input(1));
                edge = both;
            }
            let (fill_node, line) = (
                self.graph.add_node(Node::new(NodeKind::Constant(Value::Color(fill)))),
                self.graph.add_node(Node::new(NodeKind::Constant(Value::Color(contour)))),
            );
            let mix = self.graph.add_node(Node::new(NodeKind::Mix));
            self.graph.connect(fill_node, mix, Socket::Input(0));
            self.graph.connect(line, mix, Socket::Input(1));
            self.graph.connect(edge, mix, Socket::Input(2));
            let output = self.graph.output();
            self.graph.connect(mix, output, Socket::Param(Param::EmissionColor));
            // The outline opaque, the fill as clear as it is.
            let opacity = self.graph.add_node(
                Node::new(NodeKind::Node(ShaderNode::MapRange { kind: MapRangeKind::Linear, clamp: true }))
                    .with_literal(Socket::Input(1), Value::Float(0.0))
                    .with_literal(Socket::Input(2), Value::Float(1.0))
                    .with_literal(Socket::Input(3), Value::Float(clear))
                    .with_literal(Socket::Input(4), Value::Float(1.0)),
            );
            self.graph.connect(edge, opacity, Socket::Input(0));
            self.graph.connect(opacity, output, Socket::Param(Param::Opacity));
            self.note(format!(
                "{} is a toon (cel-shaded) material: Tequila has no cel shading, so its tones came in as one flat colour; its outline is drawn round the silhouette and along sharp creases, which the Solid view does not show",
                self.mtl.name
            ));
        } else {
            self.note(format!("{} is a toon (cel-shaded) material; it came in as a flat, see-through colour", self.mtl.name));
        }
    }

    /// KeyShot's Marble as Tequila's: parallel bands bent by noise, across
    /// the axis its placement turns them to.
    fn marble(&mut self, s: &Shader) -> NodeId {
        use tequila_texture::procedural::{Axis, Procedural};
        // The placement: a 4×4, row by row, points multiplied on its left.
        // The bands run across the pattern's first coordinate, so they
        // vary along its first column.
        let axis = match s.value("transform_obj_to_uv") {
            Some(Val::Numbers(m)) if m.len() >= 9 => {
                let column = [m[0].abs(), m[4].abs(), m[8].abs()];
                if column[1] > column[0] && column[1] >= column[2] {
                    Axis::Y
                } else if column[2] > column[0] {
                    Axis::Z
                } else {
                    Axis::X
                }
            }
            _ => Axis::X,
        };
        let scale = s.number("scale").filter(|v| *v > 0.0).unwrap_or(1.0);
        let marble = Procedural::Marble {
            axis,
            frequency: std::f32::consts::TAU / (MARBLE_PERIOD * scale),
            distortion: s.number("vein_noise").unwrap_or(0.1) * std::f32::consts::TAU,
            fractal: tequila_texture::noise::Fractal { octaves: 3, ..Default::default() },
        };
        self.note("its marble pattern's band spacing was estimated from KeyShot's own preview of it".into());
        self.graph.add_node(Node::new(NodeKind::Procedural { procedural: Box::new(marble), seed: 1 }))
    }

    /// How deep a picture's bump is per unit of its height. A package's is
    /// half a millimetre, measured against KeyShot's previews. A scene's is
    /// a thousandth of the picture's repeat, about a texel for a picture a
    /// thousand across: a slope rather than a depth, so a fine finish on a
    /// small part stays a finish (at half a millimetre a glossy one came out
    /// matte grey, where KeyShot's render keeps it glossy).
    fn bump_depth(&self, s: &Shader) -> f32 {
        if self.scene {
            0.001 * s.number("uv_scale_u").map(f32::abs).filter(|v| *v > 1e-6).unwrap_or(0.5)
        } else {
            0.0005
        }
    }

    /// A bump or normal map from node `n`.
    fn bump(&mut self, n: u32, scale: f32) -> Result<(), ImportError> {
        let s = self.shader(n)?.clone();
        let output = self.graph.output();
        if s.kind == "lux_bumpmap_add" {
            // Two heights, mixed by its ratio: none of the second at 0, all
            // of it at 1. A normal map has no height to add, so it stands
            // alone.
            let (first, second) = (s.number("bump_1").map(|v| v as u32), s.number("bump_2").map(|v| v as u32));
            let ratio = s.number("ratio").unwrap_or(0.5).clamp(0.0, 1.0);
            let a = match first {
                Some(f) => self.height(f, scale * s.number("weight_1").unwrap_or(1.0))?,
                None => None,
            };
            let b = match second {
                Some(f) => self.height(f, scale * s.number("weight_2").unwrap_or(1.0))?,
                None => None,
            };
            let mixed = match (a, b) {
                (Some(a), Some(b)) => {
                    // a + (b - a) * ratio, as one Mix of numbers.
                    let mix = self.graph.add_node(
                        Node::new(NodeKind::Node(ShaderNode::Mix { data: MixData::Float, blend: BlendMode::Mix, clamp_factor: true, clamp_result: false }))
                            .with_literal(Socket::Input(0), Value::Float(ratio)),
                    );
                    self.graph.connect_output(a.0, a.1, mix, Socket::Input(1));
                    self.graph.connect_output(b.0, b.1, mix, Socket::Input(2));
                    Some((mix, 0))
                }
                (one, other) => one.or(other),
            };
            match mixed {
                Some((node, socket)) => self.graph.connect_output(node, socket, output, Socket::Param(Param::Bump)),
                None => {
                    if let Some(first) = first {
                        return self.bump(first, scale);
                    }
                }
            }
            return Ok(());
        }
        if matches!(s.kind.as_str(), "lux_noise" | "lux_fractal_noise") {
            if let Some((node, socket)) = self.height(n, scale)? {
                self.graph.connect_output(node, socket, output, Socket::Param(Param::Bump));
            }
            return Ok(());
        }
        let normal = s.kind.contains("normal")
            || s.number("bump_mode") == Some(1.0)
            || s.text("texture").is_some_and(|t| {
                let t = base_name(t).to_ascii_lowercase();
                // `_n` closing the name as well: X-Rite's scans name their
                // normal maps `..._n.exr`, and one read as heights left a
                // powder coat's grain flat.
                let stem = t.rsplit_once('.').map_or(t.as_str(), |(stem, _)| stem);
                t.contains("normal") || t.contains("_nrm") || t.contains("_nm") || t.contains("-nm") || stem.ends_with("_n") || stem.ends_with("-n")
            });
        if s.kind == "lux_marble" {
            // Marble's bands as a bump are what 3D-print materials draw
            // their layer lines with.
            let marble = self.marble(&s);
            let height = s.number("bump_height").unwrap_or(1.0) * scale;
            let bump = self.graph.add_node(Node::new(NodeKind::Bump { depth: 0.0005 * height }));
            self.graph.connect(marble, bump, Socket::Input(0));
            self.graph.connect(bump, output, Socket::Param(Param::Bump));
            return Ok(());
        }
        if !(s.kind.starts_with("texture") || s.kind == "triplanar") {
            self.note(format!("the procedural bump `{}` was left out", s.kind));
            return Ok(());
        }
        let Some(image) = self.image(&s, if normal { MapRole::Normal } else { MapRole::Height })? else {
            // A protected Mold-Tech texture: a moulded grain at its size
            // stands in, so the finish is textured and matte as moulded
            // plastic is, not glass-smooth.
            let file = s.text("texture").or_else(|| s.text("textureX")).unwrap_or("");
            if file.to_ascii_lowercase().ends_with(".ktx") {
                let grain = self.graph.add_node(
                    Node::new(NodeKind::Node(ShaderNode::noise()))
                        .with_literal(Socket::Input(2), Value::Float(1.0 / GRAIN_SIZE))
                        .with_literal(Socket::Input(3), Value::Float(2.0))
                        .with_literal(Socket::Input(4), Value::Float(0.5)),
                );
                let bump = self.graph.add_node(Node::new(NodeKind::Bump { depth: GRAIN_DEPTH * scale }));
                self.graph.connect(grain, bump, Socket::Input(0));
                self.graph.connect(bump, output, Socket::Param(Param::Bump));
                self.note("a fine moulded texture stands in for it".into());
            }
            return Ok(());
        };
        if normal {
            self.graph.connect(image, output, Socket::Param(Param::Bump));
        } else {
            let height = s.number("bump_height").unwrap_or(1.0) * scale;
            let red = self.graph.add_node(Node::new(NodeKind::Channel { channel: Channel::Red }));
            self.graph.connect(image, red, Socket::Input(0));
            let bump = self.graph.add_node(Node::new(NodeKind::Bump { depth: self.bump_depth(&s) * height }));
            self.graph.connect(red, bump, Socket::Input(0));
            self.graph.connect(bump, output, Socket::Param(Param::Bump));
        }
        Ok(())
    }
}

/// The material's colour as its preview shows it: the middle of the disc
/// KeyShot renders it on, without the label at its centre or the floor.
fn icon_colour(png: &[u8]) -> Option<Spectrum> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).ok()?;
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        _ => return None,
    };
    if info.bit_depth != png::BitDepth::Eight {
        return None;
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let mut samples: [Vec<f32>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    // Either side of the disc, clear of its label and of the floor.
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32 / w as f32, y as f32 / h as f32);
            let sides = (0.42..0.62).contains(&fy) && ((0.1..0.26).contains(&fx) || (0.74..0.9).contains(&fx));
            let top = (0.26..0.34).contains(&fy) && (0.3..0.7).contains(&fx);
            if !(sides || top) {
                continue;
            }
            let i = (y * w + x) * channels;
            let to_linear = |b: u8| {
                let v = b as f32 / 255.0;
                if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            for (c, list) in samples.iter_mut().enumerate() {
                list.push(to_linear(buffer[i + c]));
            }
        }
    }
    // The median of each channel: a carpaint's sparkle would pull a mean
    // up to grey.
    let median = |list: &mut Vec<f32>| -> Option<f32> {
        list.sort_by(f32::total_cmp);
        list.get(list.len() / 2).copied()
    };
    let [r, g, b] = &mut samples;
    Some(Spectrum::from_linear_srgb(vec3(median(r)?, median(g)?, median(b)?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const WEATHERED: &str = r#"//--lux:mat:3--
// KeyShot material file
#define shader 1 texture_normalmap_normal {
"texture" "C:\Users\Someone\Textures\weathered-metal-NM.png",
"bump_scale" 0.25,
"scale_u" 1,
"scale_v" 1,
"two_sided" on,
}
#define shader 2 texture_rgb {
"texture" "C:\Users\Someone\Textures\weathered-metal.jpg",
"default_color" 0.352941 0.337255 0.32549,
"scale_u" -2,
"scale_v" 1,
}
#define shader 3 lux_metal {
"color" 0.352941 0.337255 0.32549,
"color" ! 2,
"roughness" 0.075,
}
#define shader 4 lux_bumpmap {
"bumpmap" 1,
"shader" 3,
"bump_scale" 1,
}

#define material "Weathered Metal" {
surface 4
id 195
}
"#;

    /// The text reads into its nodes: numbers, on/off, quoted paths with
    /// their backslashes, links, and the material's surface.
    #[test]
    fn an_mtl_reads_into_its_nodes() {
        let mtl = parse(WEATHERED.as_bytes()).expect("reads");
        assert_eq!(mtl.name, "Weathered Metal");
        assert_eq!(mtl.surface, Some(4));
        let metal = &mtl.shaders[&3];
        assert_eq!(metal.kind, "lux_metal");
        assert_eq!(metal.link("color"), Some(2));
        assert_eq!(metal.number("roughness"), Some(0.075));
        assert_eq!(mtl.shaders[&1].flag("two_sided"), Some(true));
        assert_eq!(mtl.shaders[&2].text("texture"), Some(r"C:\Users\Someone\Textures\weathered-metal.jpg"));
    }

    /// A composite's list of layers is bytes between brackets.
    #[test]
    fn a_list_of_nodes_is_read_from_its_bytes() {
        let mut text = b"#define shader 9 lux_composite {\n\"shaders\" [\n".to_vec();
        text.extend_from_slice(&[1, 2, 3, b'i', 2, 0, 0, 0, 2, 0, 0, 0, 8, 0, 0, 0]);
        text.extend_from_slice(b" ],\n\"opacity_map\" on,\n}\n#define material \"M\" {\nsurface 9\n}\n");
        let mtl = parse(&text).expect("reads");
        assert_eq!(mtl.shaders[&9].value("shaders"), Some(&Val::List(vec![2, 8])));
        assert_eq!(mtl.shaders[&9].flag("opacity_map"), Some(true));
    }

    /// The metal comes in as a metal, its colour driven by its picture and
    /// its normal map on Bump; the pictures are found by name, wherever the
    /// author kept them.
    #[test]
    fn a_textured_metal_comes_in_as_one() {
        let mtl = parse(WEATHERED.as_bytes()).unwrap();
        let scratch = std::env::temp_dir().join(format!("tequila-kmp-test-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let mut asked = Vec::new();
        let mut picture = |name: &str| -> Result<Option<PathBuf>, String> {
            asked.push(base_name(name).to_string());
            Ok(Some(PathBuf::from(format!("C:/pictures/{}", base_name(name)))))
        };
        let converted = convert(&mtl, &mut picture, &scratch).expect("converts");
        let entry = converted.entry;
        assert_eq!(entry.material_type, MaterialType::Metal);
        assert_eq!(entry.category, "Metal");
        assert!((entry.material.specular_roughness - 0.075).abs() < 1e-6);
        assert!(entry.graph.is_connected(Param::BaseColor), "the colour picture is not wired");
        assert!(entry.graph.is_connected(Param::Bump), "the normal map is not wired");
        let roles: Vec<MapRole> = entry.textures.iter().map(|(_, t)| t.role).collect();
        // A metal's colour picture is a photograph, decoded as one.
        assert!(roles.contains(&MapRole::Color) && roles.contains(&MapRole::Normal), "{roles:?}");
        assert_eq!(asked, vec!["weathered-metal.jpg", "weathered-metal-NM.png"]);
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A liquid's colour is the colour light takes on through it.
    #[test]
    fn a_liquid_tints_what_passes_through() {
        let text = "#define shader 1 lux_liquid {\n\"color\" 1 0.98 0.5,\n\"ior\" 1.33,\n\"transparency\" 3.9,\n}\n#define material \"Wine\" {\nsurface 1\n}\n";
        let mtl = parse(text.as_bytes()).unwrap();
        let converted = convert(&mtl, &mut |_| Ok(None), Path::new(".")).unwrap();
        let m = &converted.entry.material;
        assert_eq!(converted.entry.material_type, MaterialType::Liquid);
        assert!((m.specular_ior - 1.33).abs() < 1e-6);
        assert!(m.transmission_color.to_linear_srgb().z < 0.6, "{:?}", m.transmission_color);
        assert!(converted.notes.is_empty(), "{:?}", converted.notes);
    }

    /// A 3D-print material draws its layer lines as a Marble bump; it was
    /// left out, and the material came in smooth. Its bands run across the
    /// axis its placement says, at the spacing KeyShot shows them.
    #[test]
    fn a_marble_bump_draws_layer_lines() {
        let text = "#define shader 1 lux_marble {\n\"color1\" 0.7 0.7 0.7,\n\"color2\" 0.2 0.1 0.1,\n\"vein_noise\" 0.075,\n\"scale\" 1,\n\"bump_height\" 0.25,\n\"transform_obj_to_uv\" 1 0 0 0 0 1 -1.8e-07 0 0 1.8e-07 1 0 2.55 0 -39.77 1,\n}\n#define shader 2 lux_plastic {\n\"diffuse\" 0.0007 0.27 0.72,\n\"roughness\" 0.01,\n}\n#define shader 3 lux_bumpmap {\n\"bumpmap\" 1,\n\"shader\" 2,\n\"bump_scale\" 1,\n}\n#define material \"True Blue PLA\" {\nsurface 3\n}\n";
        let converted = convert(&parse(text.as_bytes()).unwrap(), &mut |_| Ok(None), Path::new(".")).unwrap();
        let graph = &converted.entry.graph;
        assert!(graph.is_connected(Param::Bump), "{:?}", converted.notes);
        assert!(!converted.notes.iter().any(|n| n.contains("left out")), "{:?}", converted.notes);
        let marble = graph.nodes().find_map(|(_, n)| match &n.kind {
            NodeKind::Procedural { procedural, .. } => match **procedural {
                tequila_texture::procedural::Procedural::Marble { axis, frequency, .. } => Some((axis, frequency)),
                _ => None,
            },
            _ => None,
        });
        let (axis, frequency) = marble.expect("no marble in the graph");
        assert_eq!(axis, tequila_texture::procedural::Axis::X);
        let period_mm = std::f32::consts::TAU / frequency * 1000.0;
        assert!((1.0..2.0).contains(&period_mm), "bands every {period_mm} mm");
        assert_eq!(converted.entry.material_type, MaterialType::Plastic);
    }

    /// One record of KeyShot 11's binary `.mtl`: a tag, a one-byte length,
    /// the payload.
    fn record(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![tag, payload.len() as u8];
        out.extend_from_slice(payload);
        out
    }

    fn binary_mtl(records: &[Vec<u8>]) -> Vec<u8> {
        let mut bytes = b"//--lux:mat:001--\n     \0\x02".to_vec();
        bytes.resize(BINARY_RECORDS, 0);
        for r in records {
            bytes.extend_from_slice(r);
        }
        bytes
    }

    fn with_key(value: &[u8], key: &str) -> Vec<u8> {
        [value, key.as_bytes()].concat()
    }

    fn floats(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// KeyShot 11 and later write the `.mtl` as binary records, which were
    /// refused as "unexpected text": a toon material from KeyShot 13 would
    /// not import at all. Its named colours, links, on/off values and layer
    /// list all read, and the toon comes in as the flat, see-through colour
    /// Tequila can draw.
    #[test]
    fn a_keyshot_13_binary_material_reads() {
        let id = |n: u32| n.to_le_bytes().to_vec();
        let mut colour_block = vec![vec![0x9d, 0]];
        colour_block.push(record(0x39, &id(17)));
        colour_block.push(record(0x29, &with_key(&floats(&[0.25, 0.01, 0.01, 1.0]), "color")));
        colour_block.push(vec![0x9f, 0]);
        let mut records = vec![record(0x8d, b"//--lux:mat:12--\n")];
        let mut icon = vec![0x36];
        let png = b"0\x89PNG\r\n\x1a\nrest";
        icon.extend_from_slice(&(png.len() as u32).to_le_bytes());
        icon.extend_from_slice(png);
        records.push(icon);
        records.extend(colour_block);
        records.push(record(0x07, &with_key(&id(1), "lux_const_color_extended")));
        records.push(record(0xa1, &with_key(&id(17), "color")));
        records.push(vec![0x09, 0]);
        records.push(record(0x07, &with_key(&id(2), "lux_toon")));
        records.push(record(0x27, &with_key(&floats(&[0.0, 0.0, 0.0]), "color")));
        records.push(record(0x9b, &with_key(&id(1), "shadow color")));
        records.push(record(0x27, &with_key(&floats(&[0.25, 0.01, 0.01]), "shadow color")));
        records.push(record(0x17, &with_key(&floats(&[0.68]), "transparency")));
        records.push(record(0x25, &with_key(&id(1), "outline contour")));
        records.push(vec![0x09, 0]);
        records.push(record(0x07, &with_key(&id(3), "lux_composite")));
        records.push(record(0x7f, &[id(7), b"shaders".to_vec(), id(1), id(2)].concat()));
        records.push(vec![0x09, 0]);
        records.push(record(0x0b, b"evil toon"));
        records.push(record(0x11, &[id(1), id(3)].concat()));
        records.push(vec![0x0f, 0]);

        let mtl = parse(&binary_mtl(&records)).unwrap();
        assert_eq!(mtl.name, "evil toon");
        assert_eq!(mtl.surface, Some(3));
        assert!(mtl.icon.as_deref().is_some_and(|i| i.starts_with(b"\x89PNG")));
        let toon = &mtl.shaders[&2];
        assert_eq!(toon.kind, "lux_toon");
        assert_eq!(toon.link("shadow color"), Some(1));
        assert_eq!(toon.number("transparency"), Some(0.68));
        assert_eq!(toon.flag("outline contour"), Some(true));
        assert_eq!(mtl.shaders[&3].value("shaders"), Some(&Val::List(vec![1, 2])));
        // The named colour is resolved where it is used.
        assert_eq!(mtl.shaders[&1].colour("color"), Some(Spectrum::from_linear_srgb(vec3(0.25, 0.01, 0.01))));

        let converted = convert(&mtl, &mut |_| Ok(None), Path::new(".")).unwrap();
        let m = &converted.entry.material;
        assert_eq!(converted.entry.material_type, MaterialType::Flat);
        assert!((m.geometry_opacity - 0.32).abs() < 1e-4, "{}", m.geometry_opacity);
        let red = m.emission_color.to_linear_srgb();
        assert!(red.x > 0.1 && red.y < 0.02 && red.z < 0.02, "{red:?}");
        assert!(converted.notes.iter().any(|n| n.contains("toon")), "{:?}", converted.notes);
    }

    /// Packages on this computer, named in `TEQUILA_KMP` (separated by `;`):
    /// each reads, and says what did not come across.
    #[test]
    #[ignore = "reads packages named in TEQUILA_KMP"]
    fn packages_named_in_the_environment_read() {
        let Ok(list) = std::env::var("TEQUILA_KMP") else { return };
        for path in list.split(';').filter(|p| !p.is_empty()) {
            // TEQUILA_KMP_DUMP: every node of the package's material, as read.
            if std::env::var_os("TEQUILA_KMP_DUMP").is_some() {
                let bytes = std::fs::read(path).unwrap();
                let members = crate::texture_set::read_zip(&bytes).unwrap();
                for (name, data) in members.iter().filter(|(n, _)| n.to_ascii_lowercase().ends_with(".mtl")) {
                    let mtl = parse(data).unwrap();
                    println!("== {path} :: {name}: {} surface {:?} geometry {:?}", mtl.name, mtl.surface, mtl.geometry);
                    for (n, shader) in &mtl.shaders {
                        let params: Vec<String> = shader
                            .params
                            .iter()
                            .map(|(k, v)| match v {
                                Val::Numbers(n) if n.len() > 16 => format!("{k}=[{} numbers]", n.len()),
                                other => format!("{k}={other:?}"),
                            })
                            .collect();
                        println!("  {n} {}: {}", shader.kind, params.join(", "));
                    }
                }
                for (name, data) in &members {
                    println!("  member {name} ({} bytes)", data.len());
                }
            }
            let scratch = std::env::temp_dir().join(format!("tequila-kmp-check-{}", std::process::id()));
            let converted = read_kmp(Path::new(path), &scratch).unwrap_or_else(|e| panic!("{path}: {e}"));
            let _ = std::fs::remove_dir_all(&scratch);
            let m = &converted.entry.material;
            println!(
                "{path}: {} ({:?}) base {:?} emission {:?} opacity {} bump {}\n  notes: {:?}",
                converted.entry.name,
                converted.entry.material_type,
                m.base_color.to_linear_srgb(),
                m.emission_color.to_linear_srgb(),
                m.geometry_opacity,
                converted.entry.graph.is_connected(Param::Bump),
                converted.notes
            );
        }
    }

    /// What cannot come across is said, not dropped silently.
    #[test]
    fn what_has_no_counterpart_is_noted() {
        let text = "#define shader 1 lux_cellular {\n\"color1\" 1 1 1,\n\"color2\" 0 0 0,\n}\n#define shader 2 lux_plastic {\n\"diffuse\" 0.5 0.5 0.5,\n\"diffuse\" ! 1,\n}\n#define material \"Cells\" {\nsurface 2\n}\n";
        let mtl = parse(text.as_bytes()).unwrap();
        let converted = convert(&mtl, &mut |_| Ok(None), Path::new(".")).unwrap();
        assert!(converted.notes.iter().any(|n| n.contains("lux_cellular")), "{:?}", converted.notes);
        assert!(converted.entry.graph.is_connected(Param::BaseColor));
    }

    /// KeyShot's noise is Blender's noise between its two colours, a feature
    /// its `scale` across; and as a bump, added to another by its ratio.
    #[test]
    fn noise_comes_across_as_noise_colour_and_bump() {
        let text = "#define shader 1 lux_noise {\n\"color1\" 1 1 1,\n\"color2\" 1 0.3 0.15,\n\"scale\" 0.0125,\n\"magnitude\" 0.32,\n\"bump_height\" 5,\n}\n\
#define shader 2 lux_fractal_noise {\n\"levels\" 3,\n\"fall-off\" 0.25,\n\"scale\" 0.01,\n\"bump_height\" 0.04,\n}\n\
#define shader 3 lux_bumpmap_add {\n\"bump_1\" 1,\n\"bump_2\" 2,\n\"ratio\" 0.9,\n}\n\
#define shader 4 lux_color_adjust {\n\"color\" ! 1,\n\"hue\" 0.25,\n\"colorize\" 0.6 1 0.6,\n}\n\
#define shader 5 metallic_paint {\n\"base\" 0.9 0.8 0.75,\n\"base\" ! 1,\n\"metal\" ! 4,\n\"roughness\" 0.08,\n}\n\
#define shader 6 lux_bumpmap {\n\"bumpmap\" 3,\n\"shader\" 5,\n\"bump_scale\" 1,\n}\n#define material \"Pearl\" {\nsurface 6\n}\n";
        let mtl = parse(text.as_bytes()).unwrap();
        let converted = convert(&mtl, &mut |_| Ok(None), Path::new(".")).unwrap();
        let graph = &converted.entry.graph;
        assert!(converted.notes.is_empty(), "{:?}", converted.notes);
        for param in [Param::BaseColor, Param::FlakeColor, Param::Bump] {
            assert!(graph.is_connected(param), "{param:?} is not wired");
        }
        let noise = graph
            .nodes()
            .find_map(|(_, n)| match n.kind {
                NodeKind::Node(ShaderNode::Noise { .. }) => n.literals.get(&Socket::Input(2)).copied(),
                _ => None,
            })
            .expect("a noise node");
        assert_eq!(noise, Value::Float(80.0));
        assert!(graph.nodes().any(|(_, n)| matches!(n.kind, NodeKind::Node(ShaderNode::HueSaturationValue))));
        graph.compile().expect("compiles");
    }

    /// A toon's outline is drawn where the surface turns from view: opaque,
    /// in its contour colour, over a fill as clear as the toon is.
    #[test]
    fn a_toon_has_its_outline() {
        let text = "#define shader 1 lux_toon {\n\"color\" 0 0 0,\n\"contour color\" 0.64 0.03 0.03,\n\"shadow color\" 0.25 0.01 0.01,\n\"transparency\" 0.68,\n\"contour width\" 1,\n}\n#define material \"Toon\" {\nsurface 1\n}\n";
        let mtl = parse(text.as_bytes()).unwrap();
        let converted = convert(&mtl, &mut |_| Ok(None), Path::new(".")).unwrap();
        assert_eq!(converted.entry.material_type, MaterialType::Flat);
        let graph = &converted.entry.graph;
        assert!(graph.is_connected(Param::EmissionColor));
        assert!(graph.is_connected(Param::Opacity));
        assert!((converted.entry.material.geometry_opacity - 0.32).abs() < 1e-5);
        graph.compile().expect("compiles");
    }
}
