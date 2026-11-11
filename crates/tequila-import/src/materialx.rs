//! MaterialX documents (`.mtlx`) as library materials.
//!
//! MaterialX is the file the film and visualisation renderers have agreed
//! to exchange a material in: V-Ray, Arnold, Karma, Redshift and Blender
//! read and write it, and the free libraries (ambientCG, Poly Haven, the
//! AMD GPUOpen MaterialX library) publish in it. A material in one is a
//! *surface shader* node of one of a few models, its inputs either values
//! or wired to a small graph, most often of pictures.
//!
//! What comes across: the four models everything is written in - Autodesk
//! Standard Surface, OpenPBR Surface, glTF PBR and USD Preview Surface -
//! onto Tequila's own parameters, which are OpenPBR's, and each input that
//! is a picture (an `image` or `tiledimage`, straight or through a
//! `normalmap`, an `extract` or a graph's output) as an Image node on that
//! parameter, by the mesh's UVs with the picture's own tiling. What does
//! not is said, input by input ([`Converted::notes`]): an input driven by
//! a network of other nodes keeps the value the model gives it.

use crate::ImportError;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tequila_core::math::vec3;
use tequila_core::spectrum::Spectrum;
use tequila_format::library::{LibraryEntry, Source};
use tequila_materials::graph::{Channel, MaterialGraph, Node, NodeKind, Socket, TextureId, Value};
use tequila_materials::material_type::{MaterialType, Param, ParamUi, ParamValue};
use tequila_materials::openpbr::Material;
use tequila_scene::textures::TextureSource;
use tequila_texture::projection::{Mapping, Projection};
use tequila_texture::texture::MapRole;

/// A material from a document, and what of it did not come across.
#[derive(Clone, Debug)]
pub struct Converted {
    pub entry: LibraryEntry,
    pub notes: Vec<String>,
}

/// Whether `path` is a MaterialX document, by its name.
pub fn is_mtlx(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("mtlx"))
}

/// Every material in the document at `path`, its pictures where they are:
/// beside the document, as its `fileprefix` and file names say.
pub fn read_mtlx(path: &Path) -> Result<Vec<Converted>, ImportError> {
    let text = std::fs::read_to_string(path).map_err(|e| ImportError::Io(format!("{}: {e}", path.display())))?;
    let hint = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Material".into());
    convert(&text, path.parent().unwrap_or(Path::new(".")), &hint)
}

/// Bring every material of the document at `path` into the library at
/// `directory`, its pictures copied in with it. Returns where each went and
/// what of it did not come across.
pub fn import_to_library(path: &Path, directory: &Path, adjust: impl Fn(&mut LibraryEntry)) -> Result<Vec<(PathBuf, Vec<String>)>, ImportError> {
    read_mtlx(path)?
        .into_iter()
        .map(|mut converted| {
            adjust(&mut converted.entry);
            let saved = tequila_format::library::save_entry(directory, &converted.entry).map_err(|e| ImportError::Io(e.to_string()))?;
            Ok((saved, converted.notes))
        })
        .collect()
}

/// How a model's input is taken.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Take {
    /// As the parameter it is.
    Plain(Param),
    /// One minus it: glossiness, transparency.
    Inverted(Param),
    /// A normal map (or a `normalmap` node over one), onto Bump.
    Normal,
    /// A colour whose strength is another input: glTF's emissive.
    Emission,
}

/// The inputs of the surface shader models, by the parameter each is.
fn take(model: &str, input: &str) -> Option<Take> {
    use Param::*;
    use Take::*;
    Some(match (model, input) {
        // OpenPBR Surface: Tequila's parameters are its, by its names.
        ("open_pbr_surface", "geometry_normal") => Normal,
        ("open_pbr_surface", "transmission_dispersion_abbe_number") => Plain(Abbe),
        ("open_pbr_surface", name) => Plain(Param::from_key(name).filter(|p| !matches!(p.ui(), ParamUi::Slot | ParamUi::Metal))?),

        ("standard_surface", "base") => Plain(BaseWeight),
        ("standard_surface", "base_color") => Plain(BaseColor),
        ("standard_surface", "diffuse_roughness") => Plain(DiffuseRoughness),
        ("standard_surface", "metalness") => Plain(BaseMetalness),
        ("standard_surface", "specular") => Plain(SpecularWeight),
        ("standard_surface", "specular_color") => Plain(SpecularColor),
        ("standard_surface", "specular_roughness") => Plain(SpecularRoughness),
        ("standard_surface", "specular_IOR") => Plain(SpecularIor),
        ("standard_surface", "specular_anisotropy") => Plain(Anisotropy),
        ("standard_surface", "specular_rotation") => Plain(AnisotropyRotation),
        ("standard_surface", "transmission") => Plain(TransmissionWeight),
        ("standard_surface", "transmission_color") => Plain(TransmissionColor),
        ("standard_surface", "transmission_depth") => Plain(TransmissionDepth),
        ("standard_surface", "transmission_scatter") => Plain(TransmissionScatter),
        ("standard_surface", "transmission_scatter_anisotropy") => Plain(ScatterAnisotropy),
        ("standard_surface", "transmission_dispersion") => Plain(Abbe),
        ("standard_surface", "subsurface") => Plain(SubsurfaceWeight),
        ("standard_surface", "subsurface_color") => Plain(SubsurfaceColor),
        ("standard_surface", "subsurface_radius") => Plain(SubsurfaceRadius),
        ("standard_surface", "subsurface_anisotropy") => Plain(SubsurfaceAnisotropy),
        ("standard_surface", "sheen") => Plain(FuzzWeight),
        ("standard_surface", "sheen_color") => Plain(FuzzColor),
        ("standard_surface", "sheen_roughness") => Plain(FuzzRoughness),
        ("standard_surface", "coat") => Plain(CoatWeight),
        ("standard_surface", "coat_color") => Plain(CoatColor),
        ("standard_surface", "coat_roughness") => Plain(CoatRoughness),
        ("standard_surface", "coat_IOR") => Plain(CoatIor),
        ("standard_surface", "thin_film_thickness") => Plain(ThinFilmThickness),
        ("standard_surface", "thin_film_IOR") => Plain(ThinFilmIor),
        ("standard_surface", "emission") => Plain(EmissionLuminance),
        ("standard_surface", "emission_color") => Plain(EmissionColor),
        ("standard_surface", "opacity") => Plain(Opacity),
        ("standard_surface", "thin_walled") => Plain(ThinWalled),
        ("standard_surface", "normal") => Normal,

        ("gltf_pbr", "base_color") => Plain(BaseColor),
        ("gltf_pbr", "metallic") => Plain(BaseMetalness),
        ("gltf_pbr", "roughness") => Plain(SpecularRoughness),
        ("gltf_pbr", "ior") => Plain(SpecularIor),
        ("gltf_pbr", "specular") => Plain(SpecularWeight),
        ("gltf_pbr", "specular_color") => Plain(SpecularColor),
        ("gltf_pbr", "transmission") => Plain(TransmissionWeight),
        ("gltf_pbr", "attenuation_color") => Plain(TransmissionColor),
        ("gltf_pbr", "clearcoat") => Plain(CoatWeight),
        ("gltf_pbr", "clearcoat_roughness") => Plain(CoatRoughness),
        ("gltf_pbr", "sheen_color") => Plain(FuzzColor),
        ("gltf_pbr", "sheen_roughness") => Plain(FuzzRoughness),
        ("gltf_pbr", "emissive") => Emission,
        ("gltf_pbr", "alpha") => Plain(Opacity),
        ("gltf_pbr", "occlusion") => Plain(Occlusion),
        ("gltf_pbr", "normal") => Normal,

        ("UsdPreviewSurface", "diffuseColor") => Plain(BaseColor),
        ("UsdPreviewSurface", "metallic") => Plain(BaseMetalness),
        ("UsdPreviewSurface", "roughness") => Plain(SpecularRoughness),
        ("UsdPreviewSurface", "ior") => Plain(SpecularIor),
        ("UsdPreviewSurface", "clearcoat") => Plain(CoatWeight),
        ("UsdPreviewSurface", "clearcoatRoughness") => Plain(CoatRoughness),
        ("UsdPreviewSurface", "emissiveColor") => Emission,
        ("UsdPreviewSurface", "opacity") => Plain(Opacity),
        ("UsdPreviewSurface", "occlusion") => Plain(Occlusion),
        ("UsdPreviewSurface", "normal") => Normal,
        // Glossiness, where a document has it.
        (_, "glossiness") => Inverted(SpecularRoughness),
        _ => return None,
    })
}

/// The models read, by their node's name.
const MODELS: [&str; 4] = ["standard_surface", "open_pbr_surface", "gltf_pbr", "UsdPreviewSurface"];

/// A picture an input is wired to.
#[derive(Clone, Debug, PartialEq)]
struct Picture {
    file: PathBuf,
    /// Repeats across and down.
    tiling: [f32; 2],
    /// Which channel a number is read from.
    channel: Channel,
    /// Through a `normalmap` node.
    normal: bool,
}

type XmlNode<'a> = roxmltree::Node<'a, 'a>;

fn numbers(text: &str) -> Vec<f32> {
    text.split(',').filter_map(|v| v.trim().parse().ok()).collect()
}

/// The `fileprefix` in force at `node`: its ancestors' joined, outermost
/// first.
fn prefix(node: XmlNode) -> String {
    let mut parts: Vec<&str> = node.ancestors().filter_map(|a| a.attribute("fileprefix")).collect();
    parts.reverse();
    parts.concat()
}

/// A node's input of that name.
fn input<'a>(node: XmlNode<'a>, name: &str) -> Option<XmlNode<'a>> {
    node.children().find(|c| c.has_tag_name("input") && c.attribute("name") == Some(name))
}

struct Document<'a> {
    /// Every node by its name: top-level ones as they are, a graph's as
    /// `graph/name`.
    nodes: HashMap<String, XmlNode<'a>>,
    /// A graph's outputs, `graph/output`, by the node each is.
    outputs: HashMap<String, XmlNode<'a>>,
    folder: &'a Path,
}

impl<'a> Document<'a> {
    fn new(root: XmlNode<'a>, folder: &'a Path) -> Document<'a> {
        let mut nodes = HashMap::new();
        let mut outputs = HashMap::new();
        for child in root.children().filter(|c| c.is_element()) {
            let Some(name) = child.attribute("name") else { continue };
            if child.has_tag_name("nodegraph") {
                for inner in child.children().filter(|c| c.is_element()) {
                    let Some(inner_name) = inner.attribute("name") else { continue };
                    let key = format!("{name}/{inner_name}");
                    if inner.has_tag_name("output") {
                        outputs.insert(key, inner);
                    } else {
                        nodes.insert(key, inner);
                    }
                }
            } else {
                nodes.insert(name.to_string(), child);
            }
        }
        Document { nodes, outputs, folder }
    }

    /// The node an input, or a graph's output, is wired to: in the graph
    /// `within` unless it names another.
    fn source(&self, wire: XmlNode<'a>, within: Option<&str>) -> Option<(XmlNode<'a>, Option<String>)> {
        if let Some(graph) = wire.attribute("nodegraph") {
            let output = wire.attribute("output").unwrap_or("out");
            let out = self.outputs.get(&format!("{graph}/{output}")).or_else(|| self.outputs.iter().find(|(k, _)| k.starts_with(&format!("{graph}/"))).map(|(_, v)| v))?;
            return self.source(*out, Some(graph));
        }
        let name = wire.attribute("nodename")?;
        let key = match within {
            Some(graph) => format!("{graph}/{name}"),
            None => name.to_string(),
        };
        self.nodes.get(&key).map(|node| (*node, within.map(str::to_string)))
    }

    /// The picture a wire leads to, through the nodes that only pass one
    /// on: or the kind of node that stood in the way.
    fn picture(&self, wire: XmlNode<'a>, within: Option<&str>) -> Result<Picture, String> {
        let mut channel = wire.attribute("channels").and_then(|c| c.chars().next()).map(channel_of).unwrap_or(Channel::Red);
        let mut normal = false;
        let (mut node, mut graph) = self.source(wire, within).ok_or_else(|| "a node that is not in the document".to_string())?;
        for _ in 0..16 {
            let kind = node.tag_name().name();
            match kind {
                "image" | "tiledimage" | "gltf_image" | "gltf_colorimage" | "gltf_normalmap" | "UsdUVTexture" => {
                    let file = input(node, "file").ok_or_else(|| format!("{kind} with no file"))?;
                    let name = file.attribute("value").filter(|v| !v.is_empty()).ok_or_else(|| format!("{kind} with no file"))?;
                    let tiling = input(node, "uvtiling").and_then(|t| t.attribute("value")).map(numbers).filter(|t| t.len() == 2).map_or([1.0, 1.0], |t| [t[0], t[1]]);
                    let relative = format!("{}{name}", prefix(file)).replace('\\', "/");
                    return Ok(Picture {
                        file: self.folder.join(relative),
                        tiling,
                        channel,
                        normal: normal || kind == "gltf_normalmap",
                    });
                }
                "normalmap" => normal = true,
                "extract" => {
                    let index = input(node, "index").and_then(|i| i.attribute("value")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                    channel = Channel::ALL[index.min(2)];
                }
                "separate3" | "separate4" => {
                    if let Some(out) = wire.attribute("output") {
                        channel = channel_of(out.chars().last().unwrap_or('r'));
                    }
                }
                "dot" | "convert" | "swizzle" => {
                    if let Some(c) = input(node, "channels").and_then(|c| c.attribute("value")).and_then(|c| c.chars().next()) {
                        channel = channel_of(c);
                    }
                }
                other => return Err(format!("a {other} node")),
            }
            let next = input(node, "in").ok_or_else(|| format!("a {kind} node with nothing wired in"))?;
            (node, graph) = self.source(next, graph.as_deref()).ok_or_else(|| "a node that is not in the document".to_string())?;
        }
        Err("a chain of nodes too long to follow".into())
    }
}

fn channel_of(letter: char) -> Channel {
    match letter {
        'g' | 'y' | 'G' | 'Y' => Channel::Green,
        'b' | 'z' | 'B' | 'Z' => Channel::Blue,
        _ => Channel::Red,
    }
}

fn convert(text: &str, folder: &Path, hint: &str) -> Result<Vec<Converted>, ImportError> {
    let parsed = roxmltree::Document::parse(text).map_err(|e| ImportError::Malformed(format!("not a MaterialX document: {e}")))?;
    let root = parsed.root_element();
    if !root.has_tag_name("materialx") {
        return Err(ImportError::Malformed("not a MaterialX document: no <materialx> in it".into()));
    }
    let document = Document::new(root, folder);
    // A material names its surface shader; a document with shaders and no
    // materials is taken as a material for each.
    let mut found: Vec<(String, XmlNode)> = Vec::new();
    for material in root.children().filter(|c| c.has_tag_name("surfacematerial")) {
        let shader = input(material, "surfaceshader").and_then(|wire| document.source(wire, None)).map(|(node, _)| node);
        if let (Some(name), Some(shader)) = (material.attribute("name"), shader) {
            found.push((name.to_string(), shader));
        }
    }
    if found.is_empty() {
        found = root.children().filter(|c| MODELS.contains(&c.tag_name().name())).filter_map(|c| Some((c.attribute("name")?.to_string(), c))).collect();
    }
    if found.is_empty() {
        let old = root.children().any(|c| c.has_tag_name("material"));
        return Err(ImportError::Unsupported(if old {
            "this MaterialX document is in the format before 1.38 (shaderref); save it from a current application".into()
        } else {
            "this MaterialX document has no surface material in it".into()
        }));
    }
    let several = found.len() > 1;
    found
        .into_iter()
        .map(|(name, shader)| {
            let name = if several || name.len() > 2 { tidy_name(&name) } else { hint.to_string() };
            material(&document, shader, name)
        })
        .collect()
}

/// `SR_brushed_metal` and `M_Wood` as a person would write them.
fn tidy_name(name: &str) -> String {
    let bare = ["SR_", "M_", "MAT_", "Mat_", "mtl_", "SG_"].iter().find_map(|p| name.strip_prefix(p)).unwrap_or(name);
    let words: Vec<String> = bare
        .split(['_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut letters = w.chars();
            letters.next().map(|first| first.to_uppercase().chain(letters).collect()).unwrap_or_default()
        })
        .collect();
    if words.is_empty() { name.to_string() } else { words.join(" ") }
}

fn material(document: &Document, shader: XmlNode, name: String) -> Result<Converted, ImportError> {
    let model = shader.tag_name().name();
    if !MODELS.contains(&model) {
        return Err(ImportError::Unsupported(format!("{name} is a {model} surface, which is not one of the models read (Standard Surface, OpenPBR Surface, glTF PBR, USD Preview Surface)")));
    }
    let mut notes = Vec::new();
    let mut constants = Material { ..MaterialType::Generic.defaults() };
    // The models' own defaults where they differ from a Generic material's.
    constants.specular_roughness = if model == "standard_surface" { 0.2 } else if model == "open_pbr_surface" { 0.3 } else { 0.5 };
    let mut pictures: Vec<(Take, Picture)> = Vec::new();
    let mut emission_strength: Option<f32> = None;
    for wire in shader.children().filter(|c| c.has_tag_name("input")) {
        let Some(input_name) = wire.attribute("name") else { continue };
        if matches!((model, input_name), ("gltf_pbr", "emissive_strength")) {
            emission_strength = wire.attribute("value").and_then(|v| v.trim().parse().ok());
            continue;
        }
        let Some(taken) = take(model, input_name) else {
            // Said only where it was set to something.
            if wire.attribute("nodename").is_some() || wire.attribute("nodegraph").is_some() {
                notes.push(format!("{input_name} is wired, and has no place here"));
            }
            continue;
        };
        if wire.attribute("nodename").is_some() || wire.attribute("nodegraph").is_some() {
            match document.picture(wire, None) {
                Ok(picture) if picture.file.is_file() => pictures.push((taken, picture)),
                Ok(picture) => notes.push(format!("{input_name}: its picture {} is missing", picture.file.display())),
                Err(why) => notes.push(format!("{input_name} is driven by {why}, which was not brought across")),
            }
            continue;
        }
        let Some(values) = wire.attribute("value").map(numbers).filter(|v| !v.is_empty()).or_else(|| wire.attribute("value").map(|v| vec![if v.trim() == "true" { 1.0 } else { 0.0 }])) else { continue };
        let colour = |v: &[f32]| if v.len() >= 3 { Spectrum { r: v[0], g: v[1], b: v[2] } } else { Spectrum::splat(v[0]) };
        let number = |v: &[f32]| if v.len() >= 3 { (v[0] + v[1] + v[2]) / 3.0 } else { v[0] };
        match taken {
            Take::Plain(param) | Take::Inverted(param) => {
                let inverted = matches!(taken, Take::Inverted(_));
                let value = match param.ui() {
                    ParamUi::Color | ParamUi::Vector => ParamValue::Color(colour(&values)),
                    ParamUi::Flag => ParamValue::Flag(values[0] != 0.0),
                    _ => ParamValue::Scalar(if inverted { 1.0 - number(&values) } else { number(&values) }),
                };
                if !param.write(&mut constants, value) {
                    notes.push(format!("{input_name} could not be set"));
                }
            }
            Take::Emission => constants.emission_color = colour(&values),
            Take::Normal => {}
        }
    }
    // A model whose emission is a colour alone glows by how bright it is.
    if matches!(model, "gltf_pbr" | "UsdPreviewSurface") {
        let lit = constants.emission_color.r.max(constants.emission_color.g).max(constants.emission_color.b) > 0.0 || pictures.iter().any(|(t, _)| *t == Take::Emission);
        constants.emission_luminance = if lit { 100.0 * emission_strength.unwrap_or(1.0) } else { 0.0 };
    }

    let mut graph = MaterialGraph::from_material(constants.clone());
    let output = graph.output();
    let mut textures: Vec<(TextureId, TextureSource)> = Vec::new();
    // One Projection for each tiling the pictures have: by the UVs.
    let mut mappings: Vec<([f32; 2], tequila_materials::graph::NodeId)> = Vec::new();
    for (taken, picture) in pictures {
        let mapping = match mappings.iter().find(|(tiling, _)| *tiling == picture.tiling) {
            Some((_, node)) => *node,
            None => {
                let node = graph.add_node(Node::new(NodeKind::Mapping2d {
                    mapping: Mapping { projection: Projection::Uv, scale: vec3(picture.tiling[0], picture.tiling[1], 1.0), ..Mapping::default() },
                }));
                mappings.push((picture.tiling, node));
                node
            }
        };
        let (param, role, number) = match taken {
            Take::Normal => (Param::Bump, MapRole::Normal, false),
            Take::Emission => (Param::EmissionColor, MapRole::Color, false),
            Take::Plain(param) | Take::Inverted(param) => {
                let colour = matches!(param.ui(), ParamUi::Color | ParamUi::Vector);
                (param, if colour { MapRole::Color } else { MapRole::Data }, !colour)
            }
        };
        // A picture read for two inputs is one picture.
        let id = match textures.iter().find(|(_, source)| source.path == picture.file && source.role == role) {
            Some((id, _)) => *id,
            None => {
                let id = TextureId(textures.len() as u32 + 1);
                textures.push((id, TextureSource::new(picture.file.clone(), role)));
                id
            }
        };
        let fallback = if number { Value::Float(0.5) } else { Value::Color(Spectrum::splat(0.5)) };
        let mut from = graph.add_node(Node::new(NodeKind::ImageTexture { texture: id, fallback }));
        graph.connect(mapping, from, Socket::Input(0));
        if matches!(taken, Take::Inverted(_)) {
            let inverted = graph.add_node(Node::new(NodeKind::Invert));
            graph.connect(from, inverted, Socket::Input(0));
            from = inverted;
        }
        if number {
            let one = graph.add_node(Node::new(NodeKind::Channel { channel: picture.channel }));
            graph.connect(from, one, Socket::Input(0));
            from = one;
        }
        graph.connect(from, output, Socket::Param(param));
    }
    let entry = LibraryEntry {
        name,
        category: "Other".into(),
        tags: vec!["MaterialX".into()],
        material_type: MaterialType::Generic,
        material: graph.material().clone(),
        graph,
        textures,
        preview_uv_unit: None,
        source: Source::User,
        path: String::new(),
    };
    Ok(Converted { entry, notes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("tequila-mtlx-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(folder.join("textures")).unwrap();
        folder
    }

    /// A Standard Surface as the libraries publish one: values on some
    /// inputs, and pictures in a node graph on others - a colour, a
    /// roughness read from a channel, a normal map through its node.
    #[test]
    fn a_standard_surface_comes_across_with_its_pictures() {
        let folder = scratch("standard");
        for name in ["wood_color.png", "wood_rough.png", "wood_normal.png"] {
            std::fs::write(folder.join("textures").join(name), b"png").unwrap();
        }
        let text = r#"<?xml version="1.0"?>
<materialx version="1.38" colorspace="lin_rec709" fileprefix="textures/">
  <nodegraph name="NG_wood">
    <tiledimage name="color" type="color3">
      <input name="file" type="filename" value="wood_color.png" colorspace="srgb_texture"/>
      <input name="uvtiling" type="vector2" value="4, 2"/>
    </tiledimage>
    <tiledimage name="rough" type="color3">
      <input name="file" type="filename" value="wood_rough.png"/>
      <input name="uvtiling" type="vector2" value="4, 2"/>
    </tiledimage>
    <extract name="rough_g" type="float">
      <input name="in" type="color3" nodename="rough"/>
      <input name="index" type="integer" value="1"/>
    </extract>
    <image name="nrm" type="vector3">
      <input name="file" type="filename" value="wood_normal.png"/>
    </image>
    <normalmap name="bent" type="vector3">
      <input name="in" type="vector3" nodename="nrm"/>
    </normalmap>
    <output name="out_color" type="color3" nodename="color"/>
    <output name="out_rough" type="float" nodename="rough_g"/>
    <output name="out_normal" type="vector3" nodename="bent"/>
  </nodegraph>
  <standard_surface name="SR_wood_tiled" type="surfaceshader">
    <input name="base" type="float" value="0.9"/>
    <input name="base_color" type="color3" nodegraph="NG_wood" output="out_color"/>
    <input name="specular_roughness" type="float" nodegraph="NG_wood" output="out_rough"/>
    <input name="specular_IOR" type="float" value="1.6"/>
    <input name="coat" type="float" value="0.4"/>
    <input name="coat_roughness" type="float" value="0.1"/>
    <input name="normal" type="vector3" nodegraph="NG_wood" output="out_normal"/>
    <input name="subsurface_color" type="color3" value="0.2, 0.4, 0.6"/>
  </standard_surface>
  <surfacematerial name="M_wood_tiled" type="material">
    <input name="surfaceshader" type="surfaceshader" nodename="SR_wood_tiled"/>
  </surfacematerial>
</materialx>"#;
        let converted = convert(text, &folder, "file").expect("reads");
        assert_eq!(converted.len(), 1);
        let Converted { entry, notes } = &converted[0];
        assert_eq!(entry.name, "Wood Tiled");
        assert!(notes.is_empty(), "{notes:?}");
        let m = &entry.material;
        assert_eq!((m.base_weight, m.specular_ior, m.coat_weight, m.coat_roughness), (0.9, 1.6, 0.4, 0.1));
        assert_eq!(m.subsurface_color, Spectrum { r: 0.2, g: 0.4, b: 0.6 });

        // Three pictures, by the roles the inputs give them.
        let role = |name: &str| entry.textures.iter().find(|(_, s)| s.path.ends_with(format!("textures/{name}"))).map(|(_, s)| s.role).unwrap_or_else(|| panic!("no {name}: {:?}", entry.textures));
        assert_eq!((role("wood_color.png"), role("wood_rough.png"), role("wood_normal.png")), (MapRole::Color, MapRole::Data, MapRole::Normal));
        let graph = &entry.graph;
        let into = |param: Param| graph.links().iter().find(|l| l.to == graph.output() && l.to_socket == Socket::Param(param)).map(|l| l.from).unwrap_or_else(|| panic!("nothing wired into {param:?}"));
        assert!(matches!(graph.node(into(Param::BaseColor)).unwrap().kind, NodeKind::ImageTexture { .. }));
        assert!(matches!(graph.node(into(Param::Bump)).unwrap().kind, NodeKind::ImageTexture { .. }));
        // Roughness from the green channel, as the extract's index says.
        assert_eq!(graph.node(into(Param::SpecularRoughness)).unwrap().kind, NodeKind::Channel { channel: Channel::Green });
        // By the UVs, four across and two down, where the pictures say so.
        let tilings: Vec<(f32, f32)> = graph
            .nodes()
            .filter_map(|(_, n)| match &n.kind {
                NodeKind::Mapping2d { mapping } => Some((mapping.projection == Projection::Uv, mapping.scale.x, mapping.scale.y)),
                _ => None,
            })
            .map(|(uv, x, y)| {
                assert!(uv);
                (x, y)
            })
            .collect();
        assert!(tilings.contains(&(4.0, 2.0)) && tilings.contains(&(1.0, 1.0)), "{tilings:?}");
        graph.compile().expect("the material compiles");
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// OpenPBR's inputs are Tequila's parameters by name; glTF's metallic,
    /// roughness and emissive are the ones they mean; and what is wired to
    /// something that is not a picture is said, and keeps its value.
    #[test]
    fn the_other_models_come_across_and_what_does_not_is_said() {
        let folder = scratch("models");
        let text = r#"<materialx version="1.39">
  <open_pbr_surface name="Pearl" type="surfaceshader">
    <input name="base_color" type="color3" value="0.9, 0.85, 0.8"/>
    <input name="specular_roughness" type="float" value="0.15"/>
    <input name="coat_weight" type="float" value="1"/>
    <input name="thin_film_weight" type="float" value="0.5"/>
    <input name="geometry_thin_walled" type="boolean" value="true"/>
  </open_pbr_surface>
  <noise3d name="grain" type="float"/>
  <gltf_pbr name="Lamp" type="surfaceshader">
    <input name="base_color" type="color3" value="0.1, 0.1, 0.1"/>
    <input name="metallic" type="float" value="1"/>
    <input name="roughness" type="float" nodename="grain"/>
    <input name="emissive" type="color3" value="1, 0.5, 0.2"/>
    <input name="emissive_strength" type="float" value="3"/>
  </gltf_pbr>
  <surfacematerial name="Pearl_Mat" type="material"><input name="surfaceshader" type="surfaceshader" nodename="Pearl"/></surfacematerial>
  <surfacematerial name="Lamp_Mat" type="material"><input name="surfaceshader" type="surfaceshader" nodename="Lamp"/></surfacematerial>
</materialx>"#;
        let converted = convert(text, &folder, "file").expect("reads");
        assert_eq!(converted.iter().map(|c| c.entry.name.as_str()).collect::<Vec<_>>(), ["Pearl Mat", "Lamp Mat"]);
        let pearl = &converted[0].entry.material;
        assert_eq!(pearl.base_color, Spectrum { r: 0.9, g: 0.85, b: 0.8 });
        assert_eq!((pearl.specular_roughness, pearl.coat_weight, pearl.thin_film_weight, pearl.geometry_thin_walled), (0.15, 1.0, 0.5, true));
        assert!(converted[0].notes.is_empty());

        let lamp = &converted[1];
        assert_eq!(lamp.entry.material.base_metalness, 1.0);
        assert_eq!(lamp.entry.material.emission_color, Spectrum { r: 1.0, g: 0.5, b: 0.2 });
        assert_eq!(lamp.entry.material.emission_luminance, 300.0);
        assert_eq!(lamp.entry.material.specular_roughness, 0.5, "the model's own value where the wire could not be followed");
        assert_eq!(lamp.notes.len(), 1);
        assert!(lamp.notes[0].contains("roughness") && lamp.notes[0].contains("noise3d"), "{:?}", lamp.notes);

        // Not MaterialX, the old format, and nothing to take: each said.
        assert!(matches!(convert("<svg/>", &folder, "x"), Err(ImportError::Malformed(_))));
        assert!(matches!(convert(r#"<materialx version="1.36"><material name="m"/></materialx>"#, &folder, "x"), Err(ImportError::Unsupported(why)) if why.contains("1.38")));
        assert!(matches!(convert(r#"<materialx version="1.39"/>"#, &folder, "x"), Err(ImportError::Unsupported(_))));
        let _ = std::fs::remove_dir_all(&folder);
    }

    /// Into a library: the entry saved, its pictures copied beside it, and
    /// read back as it was written.
    #[test]
    fn a_document_goes_into_a_library_with_its_pictures() {
        let folder = scratch("library");
        // A real picture: the library copies it and reads its size.
        let file = std::fs::File::create(folder.join("textures/tint.png")).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 2, 2);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[200, 120, 60, 200, 120, 60, 200, 120, 60, 200, 120, 60]).unwrap();
        std::fs::write(
            folder.join("tinted.mtlx"),
            r#"<materialx version="1.38">
  <image name="tint" type="color3"><input name="file" type="filename" value="textures/tint.png" colorspace="srgb_texture"/></image>
  <standard_surface name="SR_tinted" type="surfaceshader"><input name="base_color" type="color3" nodename="tint"/></standard_surface>
  <surfacematerial name="Tinted" type="material"><input name="surfaceshader" type="surfaceshader" nodename="SR_tinted"/></surfacematerial>
</materialx>"#,
        )
        .unwrap();
        assert!(is_mtlx(&folder.join("tinted.mtlx")) && !is_mtlx(&folder.join("tinted.json")));
        let library = folder.join("library");
        let saved = import_to_library(&folder.join("tinted.mtlx"), &library, |entry| entry.category = "Plastic".into()).expect("imports");
        assert_eq!(saved.len(), 1);
        let (entries, problems) = tequila_format::library::read_directory(&library);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].name.as_str(), entries[0].category.as_str()), ("Tinted", "Plastic"));
        assert_eq!(entries[0].textures.len(), 1);
        let copied = &entries[0].textures[0].1.path;
        assert!(copied.starts_with(&library) && copied.is_file(), "{} is not a copy in the library", copied.display());
        let _ = std::fs::remove_dir_all(&folder);
    }
}
