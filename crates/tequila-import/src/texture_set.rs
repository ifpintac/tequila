//! PBR texture sets: a folder or ZIP of maps, made into a material.
//!
//! The material packs people download — ambientCG, Poly Haven, Quixel
//! Megascans, CGBookcase, textures.com, a Substance export — are a handful of
//! image files, one per map, with the map's role written in the file name:
//! `Wood049_2K-JPG_Color.jpg`, `brown_planks_09_nor_gl_2k.png`,
//! `T_Metal_BaseColor.png`. None of them agree on the words, and several ship
//! the same maps at two or three resolutions and both normal-map conventions.
//! This reads the names, picks one file per role, and wires a material graph
//! the way each map is meant to be read.
//!
//! What decides a file's role is the *last* role word in its name, reading
//! from the end and skipping resolution and format words. The material's own
//! name comes first and can contain role words itself — Poly Haven's
//! `metal_plate_diff_2k` is a base colour, not a metalness map — and the
//! suffix is where every pack puts the role.
//!
//! Every map shares one Mapping node, so moving or scaling the material
//! moves every map together. The mapping is tri-planar at a real-world size
//! by default, because the parts this renders are most often CAD bodies whose
//! UVs are nothing a texture was painted for; a model with real UVs can be
//! switched to them in the Material tab.

use crate::ImportError;
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{Channel, MaterialGraph, Node, NodeKind, Socket, TextureId, Value};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_render_core::material::Material;
use tequila_scene::material::MaterialDefinition;
use tequila_scene::textures::TextureSource;
use tequila_texture::projection::{Mapping, Projection};
use tequila_texture::texture::MapRole;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a map in a set is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Slot {
    BaseColor,
    Roughness,
    /// Glossiness or smoothness: roughness, inverted.
    Glossiness,
    Metalness,
    /// A tangent-space normal map; `directx` when its green channel points
    /// down, as DirectX and Unreal expect.
    Normal { directx: bool },
    /// Height or displacement, read as a bump where there is no normal map.
    Height,
    Occlusion,
    Opacity,
    Emission,
    /// Occlusion, roughness and metalness packed into red, green and blue:
    /// Poly Haven's `arm`, Unreal's `ORM`.
    Packed,
}

impl Slot {
    pub fn label(self) -> &'static str {
        match self {
            Slot::BaseColor => "base colour",
            Slot::Roughness => "roughness",
            Slot::Glossiness => "glossiness",
            Slot::Metalness => "metalness",
            Slot::Normal { directx: false } => "normal (OpenGL)",
            Slot::Normal { directx: true } => "normal (DirectX)",
            Slot::Height => "height",
            Slot::Occlusion => "ambient occlusion",
            Slot::Opacity => "opacity",
            Slot::Emission => "emission",
            Slot::Packed => "packed occlusion/roughness/metalness",
        }
    }

    /// Two normal maps are the same slot whichever convention they are in:
    /// a set carries one, in whichever convention it prefers.
    fn kind(self) -> Slot {
        match self {
            Slot::Normal { .. } => Slot::Normal { directx: false },
            other => other,
        }
    }
}

/// One file of a set.
#[derive(Clone, Debug, PartialEq)]
pub struct Map {
    pub path: PathBuf,
    pub slot: Slot,
    /// Pixels along the long side, when the name says (`2K`, `4096`); zero
    /// when it does not.
    pub resolution: u32,
}

/// A material's worth of maps.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TextureSet {
    /// From the file names, before the role word: `Wood049`,
    /// `brown planks 09`.
    pub name: String,
    /// One per slot, the best of what the set offered.
    pub maps: Vec<Map>,
    /// Files that were not used, and why, for the report.
    pub skipped: Vec<(PathBuf, String)>,
}

impl TextureSet {
    pub fn map(&self, slot: Slot) -> Option<&Map> {
        self.maps.iter().find(|m| m.slot.kind() == slot.kind())
    }
}

/// Words that name a role, and the role. Compared against whole words of
/// the lower-cased name.
fn role(word: &str) -> Option<Slot> {
    Some(match word {
        "basecolor" | "basecolour" | "albedo" | "diffuse" | "diff" | "color" | "colour" | "col" | "alb"
        | "basecolormap" | "albedomap" | "diffusemap" => Slot::BaseColor,
        "roughness" | "rough" | "rgh" | "roughnessmap" => Slot::Roughness,
        "gloss" | "glossiness" | "smoothness" => Slot::Glossiness,
        "metalness" | "metallic" | "metal" | "met" | "metallicmap" | "metalnessmap" => Slot::Metalness,
        "normal" | "normals" | "nor" | "nrm" | "nrml" | "norm" | "normalmap" | "normalgl" | "normalogl" => {
            Slot::Normal { directx: false }
        }
        "normaldx" => Slot::Normal { directx: true },
        "height" | "heightmap" | "displacement" | "disp" | "displace" | "bump" | "bumpmap" => Slot::Height,
        "ao" | "ambientocclusion" | "occlusion" | "occ" | "mixedao" => Slot::Occlusion,
        "opacity" | "alpha" | "mask" | "transparency" | "cutout" => Slot::Opacity,
        "emission" | "emissive" | "emit" | "glow" | "emissivemap" => Slot::Emission,
        "arm" | "orm" | "occlusionroughnessmetallic" => Slot::Packed,
        _ => return None,
    })
}

/// Words a pack puts in names that say nothing about the role.
fn resolution(word: &str) -> Option<u32> {
    if let Some(k) = word.strip_suffix('k') {
        return k.parse::<u32>().ok().filter(|k| (1..=32).contains(k)).map(|k| k * 1024);
    }
    word.parse::<u32>().ok().filter(|v| [256, 512, 1024, 2048, 4096, 8192, 16384].contains(v))
}

fn is_noise(word: &str) -> bool {
    resolution(word).is_some()
        || matches!(word, "jpg" | "jpeg" | "png" | "exr" | "hdr" | "tif" | "tiff" | "tga" | "bmp" | "webp" | "dds" | "jxl" | "gl" | "ogl" | "opengl" | "dx"
            | "directx" | "16bit" | "8bit" | "32bit" | "lod0" | "map" | "tex" | "texture" | "t" | "mixed")
}

/// Maps a render cannot use, named so the report can say why.
fn unused(word: &str) -> Option<&'static str> {
    Some(match word {
        "preview" | "thumbnail" | "thumb" | "sphere" | "cube" | "render" | "swatch" | "icon" => "a preview image",
        "specular" | "spec" | "reflection" | "refl" => "a specular map (this is a metal/roughness workflow)",
        "cavity" => "a cavity map",
        "translucency" | "sss" | "subsurface" => "a translucency map",
        "fuzz" | "sheen" => "a sheen map",
        "id" | "idmap" | "matid" => "an ID mask",
        "curvature" | "curv" => "a curvature map",
        _ => return None,
    })
}

/// The words of a file stem, lower-cased, split on `_ - . space` and on
/// lower-to-upper case changes (`BaseColor` stays one word; `woodNormal`
/// becomes two).
fn words(stem: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out = Vec::new();
    let chars: Vec<(usize, char)> = stem.char_indices().collect();
    let mut start: Option<usize> = None;
    for (i, &(at, ch)) in chars.iter().enumerate() {
        let separator = matches!(ch, '_' | '-' | '.' | ' ');
        let boundary = i > 0 && chars[i - 1].1.is_ascii_lowercase() && ch.is_ascii_uppercase() && {
            // Only split where the rest is a role word and the whole is not:
            // `woodNormal` splits, `BaseColor` and `AmbientOcclusion` stay
            // whole.
            let rest: String = stem[at..].chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            let whole = format!("{}{}", start.map(|s| &stem[s..at]).unwrap_or(""), rest);
            role(&rest.to_ascii_lowercase()).is_some() && role(&whole.to_ascii_lowercase()).is_none()
        };
        if separator || boundary {
            if let Some(s) = start.take() {
                out.push((stem[s..at].to_ascii_lowercase(), s..at));
            }
        }
        if !separator && start.is_none() {
            start = Some(at);
        }
    }
    if let Some(s) = start {
        out.push((stem[s..].to_ascii_lowercase(), s..stem.len()));
    }
    out
}

/// How a file name reads: its role, its resolution, and the name of the
/// material it belongs to. `Err` with the reason when it is not a map.
pub fn classify(path: &Path) -> Result<(Slot, u32, String), String> {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let words = words(&stem);
    let resolution = words.iter().filter_map(|(w, _)| resolution(w)).max().unwrap_or(0);
    let directx = words.iter().any(|(w, _)| w == "dx" || w == "directx" || w == "normaldx");
    for (index, (word, _)) in words.iter().enumerate().rev() {
        if let Some(reason) = unused(word) {
            return Err(reason.to_string());
        }
        if is_noise(word) {
            continue;
        }
        if let Some(mut slot) = role(word) {
            // `base color` as two words.
            if word == "color" || word == "colour" {
                slot = Slot::BaseColor;
            }
            if let Slot::Normal { .. } = slot {
                slot = Slot::Normal { directx };
            }
            // The material's name is what came before the role, less any
            // words that are only noise or a `base` of `base_color`.
            let mut end = if index > 0 { words[index - 1].1.end } else { 0 };
            let mut before = index;
            while before > 0 && (is_noise(&words[before - 1].0) || words[before - 1].0 == "base") {
                before -= 1;
                end = if before > 0 { words[before - 1].1.end } else { 0 };
            }
            let name = stem[..end].trim_matches(|c: char| matches!(c, '_' | '-' | '.' | ' ')).to_string();
            return Ok((slot, resolution, name));
        }
        // The first unknown word from the end is the material's name, not a
        // role: nothing further left can be a role either.
        break;
    }
    Err("no map role in its name".to_string())
}

/// Which format to prefer when a set offers the same map twice at one
/// resolution: the lossless ones (PNG, TIFF, TGA, BMP) keep every level, the
/// ones that may not (JPEG, WebP, JPEG XL, block-compressed DDS) are next,
/// and EXR last because its float data is rarely what an 8-bit map was
/// authored as.
fn format_rank(path: &Path) -> u32 {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("png" | "tif" | "tiff" | "tga" | "bmp") => 3,
        Some("jpg" | "jpeg" | "webp" | "jxl" | "dds") => 2,
        Some("exr") | Some("hdr") => 1,
        _ => 0,
    }
}

fn resolution_label(pixels: u32) -> String {
    if pixels >= 1024 && pixels % 1024 == 0 { format!("{}K", pixels / 1024) } else { format!("{pixels}-pixel") }
}

/// Sort files into sets: one per material name, each with one map per role —
/// the highest resolution, then the best format, and for normal maps the
/// OpenGL convention where both are offered, since it needs no conversion.
pub fn scan(paths: &[PathBuf]) -> Vec<TextureSet> {
    let mut sets: BTreeMap<String, TextureSet> = BTreeMap::new();
    let mut loose = Vec::new();
    for path in paths {
        let decodable = tequila_texture::image::Image::is_decodable(path);
        match classify(path) {
            Ok((slot, resolution, name)) if decodable => {
                let key = name.to_ascii_lowercase();
                let set = sets.entry(key).or_insert_with(|| TextureSet { name: display_name(&name), ..Default::default() });
                let candidate = Map { path: path.clone(), slot, resolution };
                match set.maps.iter_mut().find(|m| m.slot.kind() == slot.kind()) {
                    None => set.maps.push(candidate),
                    Some(existing) => {
                        let better = |m: &Map| {
                            (m.resolution, !matches!(m.slot, Slot::Normal { directx: true }), format_rank(&m.path))
                        };
                        // Why the loser lost, in the words a person would use.
                        let why = |loser: &Map, winner: &Map| -> String {
                            if loser.resolution < winner.resolution {
                                format!("a {} copy is used instead", resolution_label(winner.resolution))
                            } else if matches!(loser.slot, Slot::Normal { directx: true }) {
                                "the OpenGL version of this normal map is used instead".into()
                            } else {
                                "the same map in a better-suited format is used instead".into()
                            }
                        };
                        if better(&candidate) > better(existing) {
                            let reason = why(existing, &candidate);
                            let replaced = std::mem::replace(existing, candidate);
                            set.skipped.push((replaced.path, reason));
                        } else {
                            let reason = why(&candidate, existing);
                            set.skipped.push((candidate.path, reason));
                        }
                    }
                }
            }
            Ok(_) => loose.push((
                path.clone(),
                format!("not a format Tequila can read ({})", tequila_texture::image::Image::DECODABLE.join(", ")),
            )),
            Err(reason) => loose.push((path.clone(), reason)),
        }
    }
    let mut sets: Vec<TextureSet> = sets.into_values().filter(|s| !s.maps.is_empty()).collect();
    // A set with no colour is still a material; one with nothing but a
    // colour is too. Skipped files go to the first set's report.
    if let Some(first) = sets.first_mut() {
        first.skipped.extend(loose);
    }
    for set in &mut sets {
        set.maps.sort_by_key(|m| m.slot);
    }
    sets
}

/// `brown_planks_09` as `Brown Planks 09`; `Wood049` as it is.
fn display_name(name: &str) -> String {
    let spaced = name.replace(['_', '-'], " ");
    let spaced = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    if spaced.chars().any(|c| c.is_ascii_uppercase()) {
        return spaced;
    }
    spaced
        .split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// How a set becomes a material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SetOptions {
    /// How wide one repeat of the texture is, in scene units, for a
    /// projection that measures in them.
    pub size: f32,
    /// Tri-planar (the default, for CAD parts) or the mesh's own UVs.
    pub projection: Projection,
    /// How many times the texture repeats across and down the UVs, for
    /// [`Projection::Uv`]: a sphere's UVs run twice as far around as from
    /// pole to pole, so square bricks need two repeats around it.
    pub uv_repeats: [f32; 2],
    /// How deep a height map's bump is, in scene units, where the set has
    /// no normal map.
    pub bump_depth: f32,
}

impl Default for SetOptions {
    fn default() -> Self {
        SetOptions { size: 0.5, projection: Projection::TriPlanar, uv_repeats: [1.0, 1.0], bump_depth: 0.001 }
    }
}

/// The material a set describes, and the textures its graph refers to.
///
/// `ids` are the texture ids to use, one per map, reserved from the document
/// by the caller (`reserve_texture_ids`). A normal map in the DirectX
/// convention must already have been converted (see [`flip_green`]); its
/// path is what is given here.
pub fn material(set: &TextureSet, ids: &[TextureId], options: SetOptions) -> (MaterialDefinition, Vec<(TextureId, TextureSource)>) {
    let material = Material { specular_roughness: 0.5, ..MaterialType::Generic.defaults() };
    let mut graph = MaterialGraph::from_material(material.clone());
    let output = graph.output();
    let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
        mapping: Mapping {
            projection: options.projection,
            real_world_units: options.projection != Projection::Uv,
            scale: if options.projection == Projection::Uv {
                tequila_core::math::vec3(options.uv_repeats[0], options.uv_repeats[1], 1.0)
            } else {
                tequila_core::math::vec3(options.size, options.size, 1.0)
            },
            ..Mapping::default()
        },
    }));
    let mut textures = Vec::new();
    let mut ids = ids.iter().copied();
    let mut image = |graph: &mut MaterialGraph, map: &Map, role: MapRole, fallback: Value| {
        let id = ids.next().expect("an id per map");
        textures.push((id, TextureSource::new(map.path.clone(), role)));
        let node = graph.add_node(Node::new(NodeKind::ImageTexture { texture: id, fallback }));
        graph.connect(mapping, node, Socket::Input(0));
        node
    };
    let channel = |graph: &mut MaterialGraph, from, channel: Channel| {
        let node = graph.add_node(Node::new(NodeKind::Channel { channel }));
        graph.connect(from, node, Socket::Input(0));
        node
    };

    let has_normal = set.map(Slot::Normal { directx: false }).is_some();
    for map in &set.maps {
        match map.slot {
            Slot::BaseColor => {
                let node = image(&mut graph, map, MapRole::Color, Value::Color(Spectrum::splat(0.5)));
                graph.connect(node, output, Socket::Param(Param::BaseColor));
            }
            Slot::Roughness => {
                let node = image(&mut graph, map, MapRole::Data, Value::Float(0.5));
                let red = channel(&mut graph, node, Channel::Red);
                graph.connect(red, output, Socket::Param(Param::SpecularRoughness));
            }
            Slot::Glossiness => {
                let node = image(&mut graph, map, MapRole::Data, Value::Float(0.5));
                let inverted = graph.add_node(Node::new(NodeKind::Invert));
                graph.connect(node, inverted, Socket::Input(0));
                let red = channel(&mut graph, inverted, Channel::Red);
                graph.connect(red, output, Socket::Param(Param::SpecularRoughness));
            }
            Slot::Metalness => {
                let node = image(&mut graph, map, MapRole::Data, Value::Float(0.0));
                let red = channel(&mut graph, node, Channel::Red);
                graph.connect(red, output, Socket::Param(Param::BaseMetalness));
            }
            Slot::Normal { .. } => {
                let node = image(&mut graph, map, MapRole::Normal, Value::Float(0.0));
                graph.connect(node, output, Socket::Param(Param::Bump));
            }
            Slot::Height if !has_normal => {
                let node = image(&mut graph, map, MapRole::Height, Value::Float(0.0));
                let red = channel(&mut graph, node, Channel::Red);
                let bump = graph.add_node(Node::new(NodeKind::Bump { depth: options.bump_depth }));
                graph.connect(red, bump, Socket::Input(0));
                graph.connect(bump, output, Socket::Param(Param::Bump));
            }
            Slot::Height => {}
            Slot::Occlusion => {
                let node = image(&mut graph, map, MapRole::Data, Value::Float(1.0));
                let red = channel(&mut graph, node, Channel::Red);
                graph.connect(red, output, Socket::Param(Param::Occlusion));
            }
            Slot::Opacity => {
                let node = image(&mut graph, map, MapRole::Data, Value::Float(1.0));
                let red = channel(&mut graph, node, Channel::Red);
                graph.connect(red, output, Socket::Param(Param::Opacity));
            }
            Slot::Emission => {
                let node = image(&mut graph, map, MapRole::Color, Value::Color(Spectrum::ZERO));
                graph.connect(node, output, Socket::Param(Param::EmissionColor));
                graph.material_mut().emission_luminance = 100.0;
            }
            Slot::Packed => {
                let node = image(&mut graph, map, MapRole::Data, Value::Float(1.0));
                let (ao, rough, metalness) =
                    (channel(&mut graph, node, Channel::Red), channel(&mut graph, node, Channel::Green), channel(&mut graph, node, Channel::Blue));
                if set.map(Slot::Occlusion).is_none() {
                    graph.connect(ao, output, Socket::Param(Param::Occlusion));
                }
                if set.map(Slot::Roughness).is_none() && set.map(Slot::Glossiness).is_none() {
                    graph.connect(rough, output, Socket::Param(Param::SpecularRoughness));
                }
                if set.map(Slot::Metalness).is_none() {
                    graph.connect(metalness, output, Socket::Param(Param::BaseMetalness));
                }
            }
        }
    }
    let definition = MaterialDefinition::from_graph(set.name.clone(), MaterialType::Generic, graph);
    (definition, textures)
}

/// How many texture ids [`material`] will use for a set.
pub fn texture_count(set: &TextureSet) -> usize {
    let has_normal = set.map(Slot::Normal { directx: false }).is_some();
    set.maps.iter().filter(|m| !(m.slot == Slot::Height && has_normal)).count()
}

/// Write `source`, a DirectX-convention normal map, as an OpenGL one at
/// `destination` (a PNG): the green channel inverted. The renderer reads
/// OpenGL's convention, and read the other way every bump lights from the
/// wrong side — dents look raised.
pub fn flip_green(source: &Path, destination: &Path) -> Result<(), ImportError> {
    // Raw: a normal map's values are vector components, and decoding them
    // through the sRGB curve would bend every one before the flip.
    let image = tequila_texture::image::Image::load_raw(source)
        .map_err(|e| ImportError::Malformed(format!("{}: {e}", source.display())))?;
    let channels = image.channels.max(1);
    let mut bytes = Vec::with_capacity(image.width * image.height * 3);
    for pixel in image.data.chunks(channels) {
        let get = |i: usize| pixel.get(i).copied().unwrap_or(pixel[0]);
        for v in [get(0), 1.0 - get(1), get(2)] {
            bytes.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    let file = std::fs::File::create(destination)
        .map_err(|e| ImportError::Io(format!("{}: {e}", destination.display())))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width as u32, image.height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(&bytes))
        .map_err(|e| ImportError::Io(format!("{}: {e}", destination.display())))?;
    Ok(())
}

/// The image files in a ZIP, extracted into `directory` (created if need
/// be), flattened: a pack's `2K-JPG/` subfolder is only packaging. Deflated
/// and stored members are both read — every pack on the web is deflated.
/// Returns the paths written.
pub fn extract_zip(bytes: &[u8], directory: &Path) -> Result<Vec<PathBuf>, ImportError> {
    let members = read_zip(bytes)?;
    std::fs::create_dir_all(directory).map_err(|e| ImportError::Io(format!("{}: {e}", directory.display())))?;
    let mut written = Vec::new();
    for (name, data) in members {
        let file = name.rsplit('/').next().unwrap_or(&name).to_string();
        if file.is_empty() || !tequila_texture::image::Image::is_decodable(&file) {
            continue;
        }
        // Only a plain file name reaches the disk.
        if file.contains("..") || file.contains('\\') || file.contains(':') {
            continue;
        }
        let path = directory.join(&file);
        std::fs::write(&path, data).map_err(|e| ImportError::Io(format!("{}: {e}", path.display())))?;
        written.push(path);
    }
    Ok(written)
}

/// A ZIP's members, stored or deflated, read through the central directory.
pub(crate) fn read_zip(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, ImportError> {
    use std::io::Read;
    let u16_at = |at: usize| -> Option<u16> { bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])) };
    let u32_at = |at: usize| -> Option<u32> { bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
    let malformed = |what: &str| ImportError::Malformed(format!("not a ZIP Tequila can read: {what}"));
    // The end-of-directory record is in the last 64 KiB + 22 bytes.
    let floor = bytes.len().saturating_sub(65_557);
    let end = (floor..bytes.len().saturating_sub(21))
        .rev()
        .find(|&at| u32_at(at) == Some(0x0605_4b50))
        .ok_or_else(|| malformed("no end of central directory"))?;
    let count = u16_at(end + 10).ok_or_else(|| malformed("truncated"))? as usize;
    let mut at = u32_at(end + 16).ok_or_else(|| malformed("truncated"))? as usize;
    let mut members = Vec::new();
    for _ in 0..count {
        if u32_at(at) != Some(0x0201_4b50) {
            return Err(malformed("a directory entry is missing"));
        }
        let method = u16_at(at + 10).ok_or_else(|| malformed("truncated"))?;
        let compressed = u32_at(at + 20).ok_or_else(|| malformed("truncated"))? as usize;
        let uncompressed = u32_at(at + 24).ok_or_else(|| malformed("truncated"))? as usize;
        let name_len = u16_at(at + 28).ok_or_else(|| malformed("truncated"))? as usize;
        let extra_len = u16_at(at + 30).ok_or_else(|| malformed("truncated"))? as usize;
        let comment_len = u16_at(at + 32).ok_or_else(|| malformed("truncated"))? as usize;
        let local = u32_at(at + 42).ok_or_else(|| malformed("truncated"))? as usize;
        let name = bytes.get(at + 46..at + 46 + name_len).ok_or_else(|| malformed("truncated"))?;
        let name = String::from_utf8_lossy(name).into_owned();
        at += 46 + name_len + extra_len + comment_len;
        if name.ends_with('/') {
            continue;
        }
        if u32_at(local) != Some(0x0403_4b50) {
            return Err(malformed("a member has no local header"));
        }
        let start = local + 30 + u16_at(local + 26).unwrap_or(0) as usize + u16_at(local + 28).unwrap_or(0) as usize;
        let payload = bytes.get(start..start + compressed).ok_or_else(|| malformed("a member runs past the end"))?;
        // A pack's map is tens of megabytes at most; a member claiming more
        // than a gigabyte is a ZIP bomb or a mistake.
        if uncompressed > 1 << 30 {
            return Err(malformed("a member is implausibly large"));
        }
        let data = match method {
            0 => payload.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(uncompressed);
                flate2::read::DeflateDecoder::new(payload)
                    .take(uncompressed as u64 + 1)
                    .read_to_end(&mut out)
                    .map_err(|e| malformed(&format!("`{name}`: {e}")))?;
                if out.len() != uncompressed {
                    return Err(malformed(&format!("`{name}` did not inflate to its stated size")));
                }
                out
            }
            other => return Err(ImportError::Unsupported(format!("`{name}` is compressed with method {other}"))),
        };
        members.push((name, data));
    }
    Ok(members)
}

/// What importing a texture pack made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetReport {
    /// The materials, in the order of the command's payload.
    pub materials: Vec<(tequila_scene::tree::MaterialKey, String)>,
    /// Per material, the maps used: slot and file name.
    pub maps: Vec<Vec<(Slot, String)>>,
    /// Anything worth saying: files skipped and why, conversions made.
    pub notes: Vec<String>,
}

/// The files a source names: a folder's (and its subfolders', a level or
/// two down, since packs nest resolutions), a ZIP's once extracted beside
/// it, or — for a single map picked on its own — its folder's.
fn files_of(source: &Path, notes: &mut Vec<String>) -> Result<Vec<PathBuf>, ImportError> {
    fn walk(directory: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(directory) else { return };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if depth > 0 {
                    walk(&path, depth - 1, out);
                }
            } else {
                out.push(path);
            }
        }
    }
    let is_zip = source.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if is_zip {
        let bytes = std::fs::read(source).map_err(|e| ImportError::Io(format!("{}: {e}", source.display())))?;
        let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "textures".into());
        // Beside the archive, where a person would unpack it: the document
        // stores paths, so the maps need a home on disk that outlives this.
        let directory = source.with_file_name(&stem);
        let files = extract_zip(&bytes, &directory)?;
        notes.push(format!("unpacked {} maps into {}", files.len(), directory.display()));
        return Ok(files);
    }
    let directory = if source.is_dir() { source.to_path_buf() } else { source.parent().map(Path::to_path_buf).unwrap_or_default() };
    let mut out = Vec::new();
    walk(&directory, 2, &mut out);
    Ok(out)
}

/// A texture pack made into a library entry, without touching any open
/// project: read into a document of its own, its first material taken with
/// the pictures it reads, tri-planar with one repeat `size` metres across.
/// [`tequila_format::library::save_entry`] then copies the pictures into a
/// library.
pub fn library_entry(
    source: &Path,
    size: f32,
    name: String,
    category: String,
    tags: Vec<String>,
) -> Result<tequila_format::library::LibraryEntry, ImportError> {
    let scratch = tequila_scene::SceneDocument::new();
    let (command, _) = import_sets(&scratch, source, SetOptions { size, ..SetOptions::default() })?;
    let tequila_scene::Command::Import { payload } = command else {
        return Err(ImportError::Malformed("the pack made no material".into()));
    };
    let (_, definition) = payload
        .materials
        .into_iter()
        .next()
        .ok_or_else(|| ImportError::Malformed("the pack made no material".into()))?;
    let used: std::collections::BTreeSet<TextureId> = definition
        .graph
        .nodes()
        .filter_map(|(_, node)| match node.kind {
            NodeKind::ImageTexture { texture, .. } | NodeKind::ImageAlpha { texture, .. } => Some(texture),
            _ => None,
        })
        .collect();
    let textures = payload.textures.into_iter().filter(|(id, _)| used.contains(id)).collect();
    Ok(tequila_format::library::LibraryEntry {
        name,
        category,
        tags,
        material_type: definition.material_type,
        material: definition.material().clone(),
        graph: definition.graph.clone(),
        textures,
        source: tequila_format::library::Source::User,
        path: String::new(),
    })
}

/// Import a texture pack as materials: one per set it holds, as one
/// [`tequila_scene::Command::Import`] that adds the materials and their
/// textures and touches nothing else. Assigning them is the caller's.
pub fn import_sets(
    document: &tequila_scene::SceneDocument,
    source: &Path,
    options: SetOptions,
) -> Result<(tequila_scene::Command, SetReport), ImportError> {
    let mut report = SetReport::default();
    let files = files_of(source, &mut report.notes)?;
    let mut sets = scan(&files);
    if sets.is_empty() {
        return Err(ImportError::Unsupported(format!(
            "no texture maps found in {} — Tequila looks for files named like              `Wood_Color.jpg`, `wood_rough_2k.png` or `T_Wood_Normal.png`",
            source.display()
        )));
    }
    // A DirectX normal map becomes an OpenGL one beside it, once.
    for set in &mut sets {
        for map in &mut set.maps {
            if let Slot::Normal { directx: true } = map.slot {
                let stem = map.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let converted = map.path.with_file_name(format!("{stem}_opengl.png"));
                if !converted.exists() {
                    flip_green(&map.path, &converted)?;
                }
                report.notes.push(format!(
                    "{} is a DirectX normal map; its green channel was flipped into {}",
                    map.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    converted.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                ));
                map.path = converted;
                map.slot = Slot::Normal { directx: false };
            }
        }
    }
    let total: usize = sets.iter().map(texture_count).sum();
    let mut ids = tequila_scene::command::reserve_texture_ids(document, total).into_iter();
    let (_, keys, _) = tequila_scene::command::reserve_keys(document, 0, sets.len(), 0);
    let mut textures = Vec::new();
    let mut materials = Vec::new();
    for (set, key) in sets.iter().zip(keys) {
        let these: Vec<TextureId> = ids.by_ref().take(texture_count(set)).collect();
        let (definition, used) = material(set, &these, options);
        textures.extend(used);
        report.materials.push((key, definition.name.clone()));
        report.maps.push(
            set.maps
                .iter()
                .map(|m| (m.slot, m.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
                .collect(),
        );
        for (path, reason) in &set.skipped {
            report.notes.push(format!(
                "skipped {}: {reason}",
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            ));
        }
        materials.push((key, definition));
    }
    let command = tequila_scene::Command::Import {
        payload: Box::new(tequila_scene::command::ImportPayload {
            meshes: Vec::new(),
            textures,
            materials,
            nodes: Vec::new(),
            roots: Vec::new(),
        }),
    };
    Ok((command, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(name: &str) -> Result<Slot, String> {
        classify(Path::new(name)).map(|(s, _, _)| s)
    }

    /// The naming schemes of the packs people actually download.
    #[test]
    fn every_packs_naming_reads() {
        let cases: &[(&str, Slot)] = &[
            // ambientCG
            ("Wood049_2K-JPG_Color.jpg", Slot::BaseColor),
            ("Wood049_2K-JPG_Roughness.jpg", Slot::Roughness),
            ("Wood049_2K-JPG_NormalGL.jpg", Slot::Normal { directx: false }),
            ("Wood049_2K-JPG_NormalDX.jpg", Slot::Normal { directx: true }),
            ("Wood049_2K-JPG_Displacement.jpg", Slot::Height),
            ("Wood049_2K-JPG_AmbientOcclusion.jpg", Slot::Occlusion),
            ("Metal032_2K-PNG_Metalness.png", Slot::Metalness),
            ("Leaf001_1K-PNG_Opacity.png", Slot::Opacity),
            // Poly Haven
            ("brown_planks_09_diff_2k.jpg", Slot::BaseColor),
            ("brown_planks_09_rough_2k.jpg", Slot::Roughness),
            ("brown_planks_09_nor_gl_2k.png", Slot::Normal { directx: false }),
            ("brown_planks_09_nor_dx_2k.png", Slot::Normal { directx: true }),
            ("brown_planks_09_arm_2k.png", Slot::Packed),
            ("brown_planks_09_disp_2k.png", Slot::Height),
            ("metal_plate_diff_4k.jpg", Slot::BaseColor),
            ("metal_plate_metal_4k.jpg", Slot::Metalness),
            // Quixel Megascans
            ("vbhnbj3_4K_Albedo.jpg", Slot::BaseColor),
            ("vbhnbj3_4K_Gloss.jpg", Slot::Glossiness),
            ("vbhnbj3_4K_Normal_LOD0.jpg", Slot::Normal { directx: false }),
            // Substance / game-engine exports
            ("T_Crate_BaseColor.png", Slot::BaseColor),
            ("T_Crate_OcclusionRoughnessMetallic.png", Slot::Packed),
            ("Crate_Mixed_AO.png", Slot::Occlusion),
            ("crate_base_color.png", Slot::BaseColor),
            ("Screen_Emissive.png", Slot::Emission),
            // camelCase
            ("woodNormal.png", Slot::Normal { directx: false }),
        ];
        for (name, want) in cases {
            assert_eq!(slot(name), Ok(*want), "{name}");
        }
    }

    #[test]
    fn previews_and_unusable_maps_say_why() {
        assert!(slot("Wood049.png").is_err());
        assert!(slot("Wood049_PREVIEW.jpg").unwrap_err().contains("preview"));
        assert!(slot("vbhnbj3_4K_Specular.jpg").unwrap_err().contains("specular"));
    }

    #[test]
    fn names_and_resolutions_come_from_the_file_name() {
        assert_eq!(classify(Path::new("Wood049_2K-JPG_Color.jpg")).unwrap(), (Slot::BaseColor, 2048, "Wood049".to_string()));
        assert_eq!(
            classify(Path::new("brown_planks_09_nor_gl_4k.png")).unwrap(),
            (Slot::Normal { directx: false }, 4096, "brown_planks_09".to_string())
        );
        assert_eq!(classify(Path::new("crate_base_color.png")).unwrap().2, "crate");
    }

    /// A pack shipping every map at two resolutions and both normal
    /// conventions gives one material with the best of each.
    #[test]
    fn a_pack_is_one_set_with_the_best_of_each_map() {
        let paths: Vec<PathBuf> = [
            "brown_planks_09_diff_1k.jpg",
            "brown_planks_09_diff_4k.jpg",
            "brown_planks_09_nor_dx_4k.png",
            "brown_planks_09_nor_gl_4k.png",
            "brown_planks_09_rough_4k.jpg",
            "brown_planks_09_rough_4k.exr",
            "brown_planks_09_arm_4k.png",
            "brown_planks_09.blend",
            "brown_planks_09_preview.png",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        let sets = scan(&paths);
        assert_eq!(sets.len(), 1);
        let set = &sets[0];
        assert_eq!(set.name, "Brown Planks 09");
        let file = |s: Slot| set.map(s).map(|m| m.path.to_string_lossy().into_owned());
        assert_eq!(file(Slot::BaseColor).as_deref(), Some("brown_planks_09_diff_4k.jpg"));
        assert_eq!(file(Slot::Normal { directx: false }).as_deref(), Some("brown_planks_09_nor_gl_4k.png"));
        assert_eq!(file(Slot::Roughness).as_deref(), Some("brown_planks_09_rough_4k.jpg"));
        assert!(set.skipped.iter().any(|(p, r)| p.ends_with("brown_planks_09_preview.png") && r.contains("preview")));
        assert!(set.skipped.iter().any(|(p, r)| p.ends_with("brown_planks_09_nor_dx_4k.png") && r.contains("OpenGL")));
        assert!(set.skipped.iter().any(|(p, r)| p.ends_with("brown_planks_09_diff_1k.jpg") && r.contains("4K")));
        assert!(set.skipped.iter().any(|(p, _)| p.ends_with("brown_planks_09.blend")));
    }

    #[test]
    fn a_folder_of_two_materials_is_two_sets() {
        let paths: Vec<PathBuf> = ["Wood049_2K_Color.jpg", "Wood049_2K_Roughness.jpg", "Metal032_2K_Color.jpg", "Metal032_2K_Metalness.jpg"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let sets = scan(&paths);
        let names: Vec<&str> = sets.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Metal032", "Wood049"]);
    }

    /// Each map is read the way it is meant to be: colour as colour,
    /// numbers as data, gloss inverted, the packed map split into three, and
    /// every map through one shared mapping.
    #[test]
    fn the_material_wires_each_map_as_its_role_needs() {
        let paths: Vec<PathBuf> = ["m_Color.png", "m_Gloss.png", "m_NormalGL.png", "m_Displacement.png", "m_ARM.png"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let set = &scan(&paths)[0];
        let n = texture_count(set);
        assert_eq!(n, 4, "the height map is not used beside a normal map");
        let ids: Vec<TextureId> = (10..10 + n as u32).map(TextureId).collect();
        let (definition, textures) = material(set, &ids, SetOptions::default());
        let role_of = |name: &str| textures.iter().find(|(_, t)| t.path.ends_with(name)).map(|(_, t)| t.role);
        assert_eq!(role_of("m_Color.png"), Some(MapRole::Color));
        assert_eq!(role_of("m_Gloss.png"), Some(MapRole::Data));
        assert_eq!(role_of("m_NormalGL.png"), Some(MapRole::Normal));
        assert_eq!(role_of("m_ARM.png"), Some(MapRole::Data));
        let graph = &definition.graph;
        let mappings = graph.nodes().filter(|(_, n)| matches!(n.kind, NodeKind::Mapping2d { .. })).count();
        assert_eq!(mappings, 1, "one mapping for every map");
        assert!(graph.nodes().any(|(_, n)| matches!(n.kind, NodeKind::Invert)), "gloss is inverted");
        // Roughness comes from the gloss map, not the packed one; occlusion
        // and metalness from the packed one.
        for param in [Param::BaseColor, Param::SpecularRoughness, Param::BaseMetalness, Param::Occlusion, Param::Bump] {
            assert!(graph.driver_of(param).is_some(), "{param:?} is not driven");
        }
        graph.compile().expect("the graph compiles");
    }

    #[test]
    fn a_zip_is_read_whether_stored_or_deflated() {
        use std::io::Write;
        // A minimal archive with one deflated member, written by hand.
        let data = b"not really a png but a member all the same";
        let mut deflated = Vec::new();
        {
            let mut e = flate2::write::DeflateEncoder::new(&mut deflated, flate2::Compression::default());
            e.write_all(data).unwrap();
        }
        let name = b"pack/m_Color.png";
        let crc = tequila_format::zip::crc32(data);
        let mut zip = Vec::new();
        let local = zip.len() as u32;
        zip.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        zip.extend_from_slice(&[20, 0, 0, 0, 8, 0, 0, 0, 0, 0]);
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&(deflated.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&0u16.to_le_bytes());
        zip.extend_from_slice(name);
        zip.extend_from_slice(&deflated);
        let directory = zip.len() as u32;
        zip.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        zip.extend_from_slice(&[20, 0, 20, 0, 0, 0, 8, 0, 0, 0, 0, 0]);
        zip.extend_from_slice(&crc.to_le_bytes());
        zip.extend_from_slice(&(deflated.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes());
        zip.extend_from_slice(&(name.len() as u16).to_le_bytes());
        zip.extend_from_slice(&[0; 12]);
        zip.extend_from_slice(&local.to_le_bytes());
        zip.extend_from_slice(name);
        let directory_size = zip.len() as u32 - directory;
        zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        zip.extend_from_slice(&[0, 0, 0, 0, 1, 0, 1, 0]);
        zip.extend_from_slice(&directory_size.to_le_bytes());
        zip.extend_from_slice(&directory.to_le_bytes());
        zip.extend_from_slice(&[0, 0]);

        let members = read_zip(&zip).expect("reads");
        assert_eq!(members, vec![("pack/m_Color.png".to_string(), data.to_vec())]);
        assert!(read_zip(b"nothing like a zip").is_err());
        for cut in 0..zip.len() {
            let _ = read_zip(&zip[..cut]);
        }
    }
}
