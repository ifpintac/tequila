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
use tequila_materials::material_type::{MaterialType, Param};
use tequila_materials::openpbr::Material;
use tequila_scene::textures::TextureSource;
use tequila_texture::mip::Wrap;
use tequila_texture::projection::{Mapping, Projection};
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
const OLD_UNIT: f32 = 0.09;

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
enum Val {
    Numbers(Vec<f32>),
    Text(String),
    Flag(bool),
    /// Driven by another node.
    Link(u32),
    /// A list of nodes.
    List(Vec<u32>),
}

#[derive(Clone, Debug, Default)]
struct Shader {
    kind: String,
    params: Vec<(String, Val)>,
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
struct Mtl {
    name: String,
    shaders: BTreeMap<u32, Shader>,
    surface: Option<u32>,
    /// A displacement, when the material has one.
    geometry: Option<u32>,
    icon: Option<Vec<u8>>,
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
}

fn convert(mtl: &Mtl, picture: &mut dyn FnMut(&str) -> Result<Option<PathBuf>, String>, scratch: &Path) -> Result<Converted, ImportError> {
    let mut build = Build {
        mtl,
        graph: MaterialGraph::from_material(MaterialType::Plastic.defaults()),
        textures: Vec::new(),
        notes: Vec::new(),
        picture,
        scratch,
        made: BTreeMap::new(),
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
            "lux_glass" => (MaterialType::Glass, MaterialType::Glass.defaults()),
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
            k if k.starts_with("bsdf_axf") => {
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
                self.note(format!(
                    "{} is a measured material (X-Rite AxF), which Tequila cannot read; it is approximated from the package's preview",
                    self.mtl.name
                ));
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
                if let Some(size) = s.number("metal_flake_size").filter(|v| *v > 0.0) {
                    m.flake_size = size * 0.001;
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
                // as pale a yellow here.
                if let Some(d) = s.number("transparency").filter(|d| *d > 0.0) {
                    m.transmission_depth = d * 0.03;
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
        let inputs: &[(&str, Param, Want, MapRole)] = &[
            ("color", if matches!(material_type, MaterialType::Glass | MaterialType::GlassSolid | MaterialType::Liquid | MaterialType::Dielectric) { Param::TransmissionColor } else { Param::BaseColor }, Want::Colour, MapRole::Color),
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
            // A picture of a metal's colour is its reflectance as the file
            // stores it, not decoded as a photograph: a mid-grey weathered
            // steel renders as bright metal in KeyShot.
            let role = if param == Param::BaseColor && matches!(material_type, MaterialType::Metal | MaterialType::Anisotropic) {
                MapRole::Data
            } else {
                role
            };
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
        if kind.starts_with("edgeshader_") || kind == "lux_color_adjust" || kind == "convert_rgba_to_float" {
            if kind == "lux_color_adjust" {
                self.note("a colour adjustment (hue, saturation, colorize) was left out".into());
            }
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
        // DirectX-convention normal maps, turned to the renderer's.
        if role == MapRole::Normal && base_name(file).to_ascii_lowercase().contains("normaldx") {
            let flipped = self.scratch.join(format!("{}-gl.png", path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()));
            flip_green(&path, &flipped)?;
            path = flipped;
        }
        let id = TextureId(self.textures.len() as u32 + 1);
        self.textures.push((id, TextureSource::new(path, role)));
        let mapping = self.mapping(s);
        let coords = self.graph.add_node(Node::new(NodeKind::Mapping2d { mapping }));
        let fallback = match role {
            MapRole::Color => Value::Color(Spectrum::splat(0.5)),
            _ => Value::Float(0.5),
        };
        let image = self.graph.add_node(Node::new(NodeKind::ImageTexture { texture: id, fallback }));
        self.graph.connect(coords, image, Socket::Input(0));
        Ok(Some(image))
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

    /// A bump or normal map from node `n`.
    fn bump(&mut self, n: u32, scale: f32) -> Result<(), ImportError> {
        let s = self.shader(n)?.clone();
        if s.kind == "lux_bumpmap_add" {
            self.note("two bump maps added together were reduced to the first".into());
            if let Some(first) = s.number("bump_1") {
                return self.bump(first as u32, scale);
            }
            return Ok(());
        }
        let normal = s.kind.contains("normal")
            || s.number("bump_mode") == Some(1.0)
            || s.text("texture").is_some_and(|t| {
                let t = base_name(t).to_ascii_lowercase();
                t.contains("normal") || t.contains("_nrm") || t.contains("_nm") || t.contains("-nm")
            });
        let output = self.graph.output();
        if !(s.kind.starts_with("texture") || s.kind == "triplanar") {
            self.note(format!("the procedural bump `{}` was left out", s.kind));
            return Ok(());
        }
        let Some(image) = self.image(&s, if normal { MapRole::Normal } else { MapRole::Height })? else { return Ok(()) };
        if normal {
            self.graph.connect(image, output, Socket::Param(Param::Bump));
        } else {
            let height = s.number("bump_height").unwrap_or(1.0) * scale;
            let red = self.graph.add_node(Node::new(NodeKind::Channel { channel: Channel::Red }));
            self.graph.connect(image, red, Socket::Input(0));
            let bump = self.graph.add_node(Node::new(NodeKind::Bump { depth: 0.0005 * height }));
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
        // A metal's colour picture is read as stored, not decoded.
        assert!(roles.contains(&MapRole::Data) && roles.contains(&MapRole::Normal), "{roles:?}");
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

    /// What cannot come across is said, not dropped silently.
    #[test]
    fn what_has_no_counterpart_is_noted() {
        let text = "#define shader 1 lux_noise {\n\"color1\" 1 1 1,\n\"color2\" 0 0 0,\n}\n#define shader 2 lux_plastic {\n\"diffuse\" 0.5 0.5 0.5,\n\"diffuse\" ! 1,\n}\n#define material \"Speckle\" {\nsurface 2\n}\n";
        let mtl = parse(text.as_bytes()).unwrap();
        let converted = convert(&mtl, &mut |_| Ok(None), Path::new(".")).unwrap();
        assert!(converted.notes.iter().any(|n| n.contains("lux_noise")), "{:?}", converted.notes);
        assert!(converted.entry.graph.is_connected(Param::BaseColor));
    }
}
