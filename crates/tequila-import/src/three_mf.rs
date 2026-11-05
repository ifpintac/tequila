//! 3MF, the 3D Manufacturing Format: a ZIP package with the model inside it
//! as XML, which is what 3D printers' slicers and CAD tools write for print.
//!
//! What is read (3MF Core 1.3 and the Materials and Properties extension):
//!
//! - The model the package's relationships name as its start part
//!   (`/3D/3dmodel.model` by convention), in the unit it declares -
//!   micron, millimetre (the default), centimetre, inch, foot or metre.
//!   Microns are read as millimetres a thousandth the size, since the
//!   document has no unit that small to show.
//! - Mesh objects, and objects made of components: other objects placed
//!   by a transform, which may themselves be made of components. An object
//!   placed twice is one mesh in the document, placed twice.
//! - The build: the items to print, each an object and where it sits. Each
//!   becomes a part, or a group of parts, under its object's name.
//! - Base materials (named, with a display colour) and colour groups as
//!   materials; a triangle whose three corners take different colours from
//!   a group gets vertex colours instead, which the pipeline shows. Texture
//!   groups as a material whose base colour is the package's picture, with
//!   the triangles' texture coordinates.
//! - The production extension's `path`, by which slicers (Bambu Studio,
//!   Orca, PrusaSlicer) keep each object in a model file of its own.
//! - Support objects (`type="support"`) are left out: they are what a slicer
//!   prints under the part, not the part.
//!
//! What is not: the package thumbnail, composite materials and
//! multi-properties (their triangles come in without a material, and the
//! import says so), beam lattices, slices, and a slicer's own settings.
//!
//! 3MF is Z up, as a printer's bed is. The whole model is turned Y up under
//! one group named after the file, as an IFC or a Z-up USD stage is, so each
//! part keeps its own Z as the direction it was built along.

use crate::raw::{RawMesh, RawNode, RawScene, RawTexture};
use crate::ImportError;
use tequila_core::math::{vec2, vec3, vec4, Mat4, Transform, Vec2, Vec3, Vec4};
use tequila_core::units::LengthUnit;
use std::collections::HashMap;

/// The relationship type that names a package's start part.
const START_PART: &str = "http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel";

/// How deep components may nest. Real files go two or three levels; a
/// deeper chain is a cycle or a file built to hang the reader.
const DEEPEST: usize = 64;

/// Read a 3MF package.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    let members = crate::texture_set::read_zip(bytes)
        .map_err(|e| ImportError::Malformed(format!("a 3MF file is a ZIP package, and this one cannot be opened: {e}")))?;
    let package = Package::new(members);
    let start = package.start_part()?;

    let root_text = package.text(&start)?;
    let root_unit = model_unit(&root_text)?;
    // The unit the document is told, and what the start part's numbers are
    // multiplied by to be in it.
    let (declared, root_factor) = match root_unit {
        Unit::Micron => (LengthUnit::Millimetre, 0.001),
        Unit::Length(unit) => (unit, 1.0),
    };
    let root = parse_model(&root_text, root_factor)?;

    let mut builder = Builder {
        package: &package,
        declared,
        models: HashMap::new(),
        scene: RawScene { source_unit: Some(declared), ..RawScene::default() },
        materials: HashMap::new(),
        meshes: HashMap::new(),
        unread_properties: false,
        supports: 0,
    };

    // Z up to Y up, around everything, under the file's name.
    let mut top = RawNode::group(name);
    let mut turn = Mat4::IDENTITY;
    turn.m[1] = [0.0, 0.0, 1.0, 0.0];
    turn.m[2] = [0.0, -1.0, 0.0, 0.0];
    top.transform = Transform::from_matrix(turn).unwrap_or(Transform::IDENTITY);
    for (key, value) in &root.metadata {
        top.metadata.insert(key.clone(), value.clone());
    }
    builder.scene.nodes.push(top);
    builder.scene.roots.push(0);

    let mut items = root.build.clone();
    if items.is_empty() {
        // Nothing marked to print. Every object that is not a piece of
        // another, rather than nothing at all.
        let used: std::collections::HashSet<u32> = root
            .resources
            .values()
            .filter_map(|r| match r {
                Resource::Object(Object { content: Content::Components(list), .. }) => Some(list.iter().filter(|c| c.path.is_none()).map(|c| c.object)),
                _ => None,
            })
            .flatten()
            .collect();
        let mut ids: Vec<u32> = root.resources.iter().filter(|(id, r)| matches!(r, Resource::Object(_)) && !used.contains(id)).map(|(&id, _)| id).collect();
        ids.sort_unstable();
        items = ids.into_iter().map(|object| Item { object, transform: Mat4::IDENTITY, path: None, part_number: None }).collect();
        builder.scene.warnings.push("the package lists nothing to build, so every object in it was brought in".into());
    }
    builder.models.insert(start.clone(), root);

    for item in &items {
        let file = item.path.as_deref().map(Package::key).unwrap_or_else(|| start.clone());
        if let Some(node) = builder.object(&file, item.object, Some(0), item.transform, &mut Vec::new())? {
            if let Some(number) = &item.part_number {
                builder.scene.nodes[node].metadata.insert("Part number".to_string(), number.clone());
            }
        }
    }

    if builder.scene.meshes.is_empty() {
        return Err(ImportError::Malformed("the package holds no triangles to show".into()));
    }
    if builder.supports > 0 {
        let n = builder.supports;
        builder.scene.warnings.push(format!("{n} support object{} left out: what a slicer prints under the part, not the part", if n == 1 { " was" } else { "s were" }));
    }
    if builder.unread_properties {
        builder.scene.warnings.push(
            "some triangles take their look from composite materials or multi-properties, which are not read; they came in without a material".into(),
        );
    }
    Ok(builder.scene)
}

/// The package's members, found by name as OPC finds them: without the
/// leading slash, and ignoring case.
struct Package {
    members: HashMap<String, Vec<u8>>,
}

impl Package {
    fn new(members: Vec<(String, Vec<u8>)>) -> Package {
        Package { members: members.into_iter().map(|(name, data)| (Package::key(&name), data)).collect() }
    }

    /// A part name as a lookup key: `/3D/Textures/Wood%20grain.png` and
    /// `3d/textures/wood grain.png` are the same part.
    fn key(name: &str) -> String {
        crate::gltf::percent_decode(name.trim_start_matches(['/', '\\'])).replace('\\', "/").to_lowercase()
    }

    fn get(&self, name: &str) -> Option<&[u8]> {
        self.members.get(&Package::key(name)).map(|v| v.as_slice())
    }

    fn text(&self, name: &str) -> Result<String, ImportError> {
        let bytes = self.get(name).ok_or_else(|| ImportError::Malformed(format!("the package names `{name}` and does not hold it")))?;
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
        String::from_utf8(bytes.to_vec()).map_err(|_| ImportError::Malformed(format!("`{name}` is not UTF-8")))
    }

    /// The model part the package starts from: the one its relationships
    /// name, or failing that the conventional one, or any model at all.
    fn start_part(&self) -> Result<String, ImportError> {
        if let Some(rels) = self.get("_rels/.rels") {
            let text = String::from_utf8_lossy(rels.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(rels)).into_owned();
            if let Ok(doc) = roxmltree::Document::parse(&text) {
                let target = doc
                    .descendants()
                    .filter(|n| n.tag_name().name() == "Relationship")
                    .find(|n| n.attribute("Type").is_some_and(|t| t.eq_ignore_ascii_case(START_PART)))
                    .and_then(|n| n.attribute("Target"));
                if let Some(target) = target {
                    let key = Package::key(target);
                    if self.members.contains_key(&key) {
                        return Ok(key);
                    }
                }
            }
        }
        if self.members.contains_key("3d/3dmodel.model") {
            return Ok("3d/3dmodel.model".to_string());
        }
        let mut models: Vec<&String> = self.members.keys().filter(|k| k.ends_with(".model")).collect();
        models.sort();
        models.first().map(|k| k.to_string()).ok_or_else(|| ImportError::Malformed("the package holds no 3D model part".into()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Unit {
    Micron,
    Length(LengthUnit),
}

impl Unit {
    fn metres(self) -> f64 {
        match self {
            Unit::Micron => 1e-6,
            Unit::Length(unit) => unit.to_metres(),
        }
    }
}

/// The unit a model part declares, without reading the rest of it.
fn model_unit(text: &str) -> Result<Unit, ImportError> {
    let doc = parse_xml(text)?;
    Ok(unit_of(doc.root_element()))
}

fn unit_of(model: roxmltree::Node) -> Unit {
    match model.attribute("unit").map(|u| u.trim().to_ascii_lowercase()).as_deref() {
        Some("micron") => Unit::Micron,
        Some("centimeter") => Unit::Length(LengthUnit::Centimetre),
        Some("meter") => Unit::Length(LengthUnit::Metre),
        Some("inch") => Unit::Length(LengthUnit::Inch),
        Some("foot") => Unit::Length(LengthUnit::Foot),
        // The specification's default.
        _ => Unit::Length(LengthUnit::Millimetre),
    }
}

fn parse_xml(text: &str) -> Result<roxmltree::Document<'_>, ImportError> {
    roxmltree::Document::parse(text).map_err(|e| ImportError::Malformed(format!("the model is not well-formed XML: {e}")))
}

/// One model part, read out of its XML.
#[derive(Debug)]
struct Model {
    metadata: Vec<(String, String)>,
    resources: HashMap<u32, Resource>,
    build: Vec<Item>,
}

#[derive(Debug)]
enum Resource {
    /// Each base material's name and display colour, sRGB with alpha.
    Base(Vec<(String, [f32; 4])>),
    Colours(Vec<[f32; 4]>),
    Texture { path: String },
    TextureCoordinates { texture: u32, coordinates: Vec<Vec2> },
    Object(Object),
    /// A property resource that is not read: composite materials,
    /// multi-properties.
    Unread,
}

#[derive(Debug)]
struct Object {
    name: Option<String>,
    support: bool,
    pid: Option<u32>,
    pindex: Option<u32>,
    content: Content,
}

#[derive(Debug)]
enum Content {
    Mesh { vertices: Vec<Vec3>, triangles: Vec<Triangle> },
    Components(Vec<Component>),
    Empty,
}

#[derive(Clone, Copy, Debug)]
struct Triangle {
    v: [u32; 3],
    pid: Option<u32>,
    p: [Option<u32>; 3],
}

#[derive(Clone, Debug)]
struct Component {
    object: u32,
    transform: Mat4,
    path: Option<String>,
}

#[derive(Clone, Debug)]
struct Item {
    object: u32,
    transform: Mat4,
    path: Option<String>,
    part_number: Option<String>,
}

/// Read a model part. `factor` brings its lengths into the declared unit.
fn parse_model(text: &str, factor: f32) -> Result<Model, ImportError> {
    let doc = parse_xml(text)?;
    let model = doc.root_element();
    if model.tag_name().name() != "model" {
        return Err(ImportError::Malformed(format!("the model part's root is <{}>, not <model>", model.tag_name().name())));
    }
    let mut out = Model { metadata: Vec::new(), resources: HashMap::new(), build: Vec::new() };
    for child in model.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "metadata" => {
                if let (Some(key), Some(value)) = (child.attribute("name"), child.text()) {
                    let value = value.trim();
                    if !value.is_empty() {
                        // `Title`, `Designer`, `Application`; a namespaced
                        // name without its prefix.
                        let key = key.rsplit(':').next().unwrap_or(key);
                        out.metadata.push((key.to_string(), value.to_string()));
                    }
                }
            }
            "resources" => {
                for resource in child.children().filter(|n| n.is_element()) {
                    let Some(id) = resource.attribute("id").and_then(|v| v.trim().parse::<u32>().ok()) else {
                        continue;
                    };
                    if let Some(read) = parse_resource(resource, factor)? {
                        out.resources.insert(id, read);
                    }
                }
            }
            "build" => {
                for item in child.children().filter(|n| n.is_element() && n.tag_name().name() == "item") {
                    let Some(object) = item.attribute("objectid").and_then(|v| v.trim().parse().ok()) else {
                        continue;
                    };
                    out.build.push(Item {
                        object,
                        transform: transform_of(item.attribute("transform"), factor)?,
                        path: path_of(item),
                        part_number: item.attribute("partnumber").map(str::to_string).filter(|s| !s.is_empty()),
                    });
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The production extension's `p:path`: the model part an item or a
/// component's object is in, when it is not this one.
fn path_of(node: roxmltree::Node) -> Option<String> {
    node.attributes().find(|a| a.name() == "path").map(|a| a.value().to_string()).filter(|s| !s.is_empty())
}

fn parse_resource(node: roxmltree::Node, factor: f32) -> Result<Option<Resource>, ImportError> {
    let elements = |name: &'static str| node.children().filter(move |n| n.is_element() && n.tag_name().name() == name);
    Ok(Some(match node.tag_name().name() {
        "basematerials" => Resource::Base(
            elements("base")
                .enumerate()
                .map(|(i, base)| {
                    let name = base.attribute("name").map(str::to_string).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| format!("Material {}", i + 1));
                    (name, base.attribute("displaycolor").and_then(hex_colour).unwrap_or([0.8, 0.8, 0.8, 1.0]))
                })
                .collect(),
        ),
        "colorgroup" => Resource::Colours(elements("color").map(|c| c.attribute("color").and_then(hex_colour).unwrap_or([0.8, 0.8, 0.8, 1.0])).collect()),
        "texture2d" => Resource::Texture { path: node.attribute("path").unwrap_or("").to_string() },
        "texture2dgroup" => Resource::TextureCoordinates {
            texture: node.attribute("texid").and_then(|v| v.trim().parse().ok()).unwrap_or(u32::MAX),
            coordinates: elements("tex2coord")
                .map(|c| {
                    let number = |key: &str| c.attribute(key).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite()).unwrap_or(0.0);
                    // The texture's origin is its bottom left, as OBJ's
                    // is; the renderer's is the top left.
                    vec2(number("u"), 1.0 - number("v"))
                })
                .collect(),
        },
        "compositematerials" | "multiproperties" => Resource::Unread,
        "object" => Resource::Object(parse_object(node, factor)?),
        _ => return Ok(None),
    }))
}

fn parse_object(node: roxmltree::Node, factor: f32) -> Result<Object, ImportError> {
    let index = |n: roxmltree::Node, key: &str| n.attribute(key).and_then(|v| v.trim().parse::<u32>().ok());
    let kind = node.attribute("type").unwrap_or("model");
    let mut content = Content::Empty;
    for child in node.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "mesh" => {
                let mut vertices = Vec::new();
                let mut triangles = Vec::new();
                for part in child.children().filter(|n| n.is_element()) {
                    match part.tag_name().name() {
                        "vertices" => {
                            for v in part.children().filter(|n| n.is_element() && n.tag_name().name() == "vertex") {
                                let coordinate = |key: &str| -> Result<f32, ImportError> {
                                    v.attribute(key)
                                        .and_then(|s| s.trim().parse::<f32>().ok())
                                        .filter(|x| x.is_finite())
                                        .ok_or_else(|| ImportError::Malformed(format!("vertex {} has no number for {key}", vertices.len())))
                                };
                                vertices.push(vec3(coordinate("x")? * factor, coordinate("y")? * factor, coordinate("z")? * factor));
                            }
                        }
                        "triangles" => {
                            for t in part.children().filter(|n| n.is_element() && n.tag_name().name() == "triangle") {
                                let corner = |key: &str| index(t, key).ok_or_else(|| ImportError::Malformed(format!("triangle {} has no {key}", triangles.len())));
                                triangles.push(Triangle {
                                    v: [corner("v1")?, corner("v2")?, corner("v3")?],
                                    pid: index(t, "pid"),
                                    p: [index(t, "p1"), index(t, "p2"), index(t, "p3")],
                                });
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(bad) = triangles.iter().flat_map(|t| t.v).find(|&v| v as usize >= vertices.len()) {
                    return Err(ImportError::Malformed(format!("a triangle names vertex {bad}, and the object has {}", vertices.len())));
                }
                content = Content::Mesh { vertices, triangles };
            }
            "components" => {
                let mut list = Vec::new();
                for c in child.children().filter(|n| n.is_element() && n.tag_name().name() == "component") {
                    let Some(object) = index(c, "objectid") else { continue };
                    list.push(Component { object, transform: transform_of(c.attribute("transform"), factor)?, path: path_of(c) });
                }
                content = Content::Components(list);
            }
            _ => {}
        }
    }
    Ok(Object {
        name: node.attribute("name").map(str::to_string).filter(|s| !s.trim().is_empty()),
        support: kind == "support" || kind == "solidsupport",
        pid: index(node, "pid"),
        pindex: index(node, "pindex"),
        content,
    })
}

/// A 3MF transform: twelve numbers, a 4x3 matrix applied to a row vector,
/// its last row the translation. Absent, it is the identity.
fn transform_of(text: Option<&str>, factor: f32) -> Result<Mat4, ImportError> {
    let Some(text) = text else {
        return Ok(Mat4::IDENTITY);
    };
    let values: Vec<f32> = text.split_whitespace().filter_map(|w| w.parse::<f32>().ok()).filter(|v| v.is_finite()).collect();
    let [m00, m01, m02, m10, m11, m12, m20, m21, m22, m30, m31, m32] = values[..] else {
        return Err(ImportError::Malformed(format!("`{text}` is not a 3MF transform of twelve numbers")));
    };
    let mut m = Mat4::IDENTITY;
    m.m[0] = [m00, m10, m20, m30 * factor];
    m.m[1] = [m01, m11, m21, m31 * factor];
    m.m[2] = [m02, m12, m22, m32 * factor];
    Ok(m)
}

/// `#RRGGBB` or `#RRGGBBAA`, as display values 0 to 1.
fn hex_colour(text: &str) -> Option<[f32; 4]> {
    let hex = text.trim().strip_prefix('#')?;
    if !(hex.len() == 6 || hex.len() == 8) || !hex.is_ascii() {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok().map(|b| b as f32 / 255.0);
    Some([byte(0)?, byte(2)?, byte(4)?, if hex.len() == 8 { byte(6)? } else { 1.0 }])
}

/// A display colour as the linear one the renderer holds.
fn linear(colour: [f32; 4]) -> Vec4 {
    let f = crate::ply::srgb_to_linear;
    vec4(f(colour[0]), f(colour[1]), f(colour[2]), colour[3])
}

/// What a triangle's look comes from, which decides the mesh it lands in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Look {
    /// A material, or none.
    Material(Option<usize>),
    /// A picture, with texture coordinates at the corners.
    Textured(usize),
    /// Colours that differ from corner to corner.
    VertexColours,
}

/// One object's triangles of one look, gathered.
#[derive(Default)]
struct Gather {
    mesh: RawMesh,
    /// (the object's vertex, what the corner adds to it) to the gathered
    /// vertex: a vertex whose corners take different coordinates or colours
    /// is split.
    made: HashMap<(u32, u64), u32>,
}

struct Builder<'a> {
    package: &'a Package,
    declared: LengthUnit,
    /// Model parts read so far, by key.
    models: HashMap<String, Model>,
    scene: RawScene,
    /// (model part, resource, index) to the scene's material.
    materials: HashMap<(String, u32, u32), usize>,
    /// (model part, object) to its meshes and what to call each.
    meshes: HashMap<(String, u32), Vec<(usize, String)>>,
    unread_properties: bool,
    supports: usize,
}

impl Builder<'_> {
    /// A model part, read the first time it is needed.
    fn model(&mut self, file: &str) -> Result<&Model, ImportError> {
        if !self.models.contains_key(file) {
            let text = self.package.text(file)?;
            // Another part's numbers into the declared unit.
            let factor = (model_unit(&text)?.metres() / self.declared.to_metres()) as f32;
            let model = parse_model(&text, factor)?;
            self.models.insert(file.to_string(), model);
        }
        Ok(&self.models[file])
    }

    /// The node for an object placed by `transform` under `parent`: a part,
    /// a group of parts when its triangles have several looks, or a group
    /// of its components. `None` for something with nothing to show.
    fn object(&mut self, file: &str, id: u32, parent: Option<usize>, transform: Mat4, stack: &mut Vec<(String, u32)>) -> Result<Option<usize>, ImportError> {
        if stack.len() >= DEEPEST || stack.iter().any(|(f, i)| f == file && *i == id) {
            return Err(ImportError::Malformed(format!("object {id} is made of itself, through its components")));
        }
        let (name, support, components) = {
            let model = self.model(file)?;
            let Some(Resource::Object(object)) = model.resources.get(&id) else {
                return Err(ImportError::Malformed(format!("object {id} is placed and not defined")));
            };
            let components = match &object.content {
                Content::Components(list) => Some(list.clone()),
                _ => None,
            };
            (object.name.clone().unwrap_or_else(|| format!("Object {id}")), object.support, components)
        };
        if support {
            self.supports += 1;
            return Ok(None);
        }
        let transform = Transform::from_matrix(transform).unwrap_or_else(|| {
            self.scene.warnings.push(format!("`{name}` is placed by a transform that flattens it; it was placed without one"));
            Transform::IDENTITY
        });

        if let Some(components) = components {
            let node = self.push(RawNode { parent, transform, ..RawNode::group(name) });
            stack.push((file.to_string(), id));
            for component in components {
                let child_file = component.path.as_deref().map(Package::key).unwrap_or_else(|| file.to_string());
                self.object(&child_file, component.object, Some(node), component.transform, stack)?;
            }
            stack.pop();
            return Ok(Some(node));
        }

        let meshes = self.meshes_of(file, id)?;
        Ok(match meshes.as_slice() {
            [] => None,
            [(mesh, _)] => Some(self.push(RawNode { parent, transform, ..RawNode::part(name, *mesh) })),
            several => {
                let node = self.push(RawNode { parent, transform, ..RawNode::group(name) });
                for (mesh, label) in several {
                    self.push(RawNode { parent: Some(node), ..RawNode::part(label.clone(), *mesh) });
                }
                Some(node)
            }
        })
    }

    fn push(&mut self, node: RawNode) -> usize {
        self.scene.nodes.push(node);
        self.scene.nodes.len() - 1
    }

    /// A mesh object's triangles as the scene's meshes, one per look, made
    /// once however many times the object is placed.
    fn meshes_of(&mut self, file: &str, id: u32) -> Result<Vec<(usize, String)>, ImportError> {
        let key = (file.to_string(), id);
        if let Some(made) = self.meshes.get(&key) {
            return Ok(made.clone());
        }
        // Taken out while its triangles are gathered, since that makes
        // materials and so needs the builder; put back after.
        let model = self.models.remove(file).expect("read by `object`");
        let result = self.gather(file, &model, id);
        self.models.insert(file.to_string(), model);
        let made = result?;
        self.meshes.insert(key, made.clone());
        Ok(made)
    }

    fn gather(&mut self, file: &str, model: &Model, id: u32) -> Result<Vec<(usize, String)>, ImportError> {
        let Some(Resource::Object(object)) = model.resources.get(&id) else {
            return Ok(Vec::new());
        };
        let Content::Mesh { vertices, triangles } = &object.content else {
            return Ok(Vec::new());
        };
        let name = object.name.clone().unwrap_or_else(|| format!("Object {id}"));
        let mut looks: Vec<(Look, Gather)> = Vec::new();
        for triangle in triangles {
            let pid = triangle.pid.or(object.pid);
            let p1 = triangle.p[0].or(object.pindex).unwrap_or(0);
            let p = [p1, triangle.p[1].unwrap_or(p1), triangle.p[2].unwrap_or(p1)];
            // What each corner adds to its vertex, and the look.
            let (look, extra): (Look, [Option<(u64, Vec2, Vec4)>; 3]) = match pid.and_then(|pid| model.resources.get(&pid).map(|r| (pid, r))) {
                None => (Look::Material(None), [None; 3]),
                Some((pid, Resource::Base(list))) => (Look::Material(self.base_material(file, pid, p1, list)), [None; 3]),
                Some((pid, Resource::Colours(list))) if p[0] == p[1] && p[1] == p[2] => (Look::Material(self.colour_material(file, pid, p1, list)), [None; 3]),
                Some((pid, Resource::Colours(list))) => {
                    let corner = |k: usize| list.get(p[k] as usize).map(|&c| (((pid as u64) << 32) | p[k] as u64, vec2(0.0, 0.0), linear(c)));
                    (Look::VertexColours, [corner(0), corner(1), corner(2)])
                }
                Some((pid, Resource::TextureCoordinates { texture, coordinates })) => {
                    let material = self.texture_material(file, *texture, model);
                    let corner = |k: usize| Some((((pid as u64) << 32) | p[k] as u64, coordinates.get(p[k] as usize).copied().unwrap_or(vec2(0.0, 0.0)), vec4(1.0, 1.0, 1.0, 1.0)));
                    (Look::Textured(material), [corner(0), corner(1), corner(2)])
                }
                Some(_) => {
                    self.unread_properties = true;
                    (Look::Material(None), [None; 3])
                }
            };
            let at = match looks.iter().position(|(l, _)| *l == look) {
                Some(at) => at,
                None => {
                    looks.push((look, Gather::default()));
                    looks.len() - 1
                }
            };
            let gather = &mut looks[at].1;
            for k in 0..3 {
                let v = triangle.v[k];
                let key = (v, extra[k].map_or(u64::MAX, |e| e.0));
                let index = *gather.made.entry(key).or_insert_with(|| {
                    gather.mesh.positions.push(vertices[v as usize]);
                    match (look, extra[k]) {
                        (Look::Textured(_), Some((_, uv, _))) => gather.mesh.uvs.push(uv),
                        (Look::VertexColours, Some((_, _, colour))) => gather.mesh.colors.push(colour),
                        (Look::VertexColours, None) => gather.mesh.colors.push(vec4(0.8, 0.8, 0.8, 1.0)),
                        _ => {}
                    }
                    (gather.mesh.positions.len() - 1) as u32
                });
                gather.mesh.indices.push(index);
            }
        }

        let several = looks.len() > 1;
        let mut out = Vec::new();
        for (look, gather) in looks {
            let mut mesh = gather.mesh;
            let (material, label) = match look {
                Look::Material(Some(m)) | Look::Textured(m) => (Some(m), self.scene.materials[m].name.clone()),
                Look::Material(None) => (None, name.clone()),
                Look::VertexColours => (None, "Vertex colours".to_string()),
            };
            mesh.material = material;
            mesh.name = if several { format!("{name} ({label})") } else { name.clone() };
            self.scene.meshes.push(mesh);
            out.push((self.scene.meshes.len() - 1, label));
        }
        Ok(out)
    }

    fn base_material(&mut self, file: &str, pid: u32, index: u32, list: &[(String, [f32; 4])]) -> Option<usize> {
        let (name, colour) = list.get(index as usize)?.clone();
        Some(self.material((file.to_string(), pid, index), || plastic(&name, colour)))
    }

    fn colour_material(&mut self, file: &str, pid: u32, index: u32, list: &[[f32; 4]]) -> Option<usize> {
        let colour = *list.get(index as usize)?;
        let name = format!("Colour #{:02X}{:02X}{:02X}", (colour[0] * 255.0).round() as u8, (colour[1] * 255.0).round() as u8, (colour[2] * 255.0).round() as u8);
        Some(self.material((file.to_string(), pid, index), || plastic(&name, colour)))
    }

    /// A material whose base colour is a texture of the package's.
    fn texture_material(&mut self, file: &str, texture: u32, model: &Model) -> usize {
        let key = (file.to_string(), texture, u32::MAX);
        if let Some(&m) = self.materials.get(&key) {
            return m;
        }
        use tequila_materials::graph::Value;
        use tequila_materials::material_type::Param;
        let path = match model.resources.get(&texture) {
            Some(Resource::Texture { path }) => path.clone(),
            _ => String::new(),
        };
        let file_name = path.rsplit(['/', '\\']).next().unwrap_or("").to_string();
        let mut definition = plastic(if file_name.is_empty() { "Texture" } else { &file_name }, [1.0, 1.0, 1.0, 1.0]);
        match self.package.get(&path) {
            Some(bytes) if !file_name.is_empty() => {
                // A plain name: it becomes a file beside the model.
                let safe: String = file_name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
                let wanted = RawTexture { uri: safe, role: tequila_texture::texture::MapRole::Color, embedded: Some(std::sync::Arc::new(bytes.to_vec())), elsewhere: None };
                crate::obj::attach_texture(&mut definition, &mut self.scene.textures, wanted, Param::BaseColor, Value::Color(tequila_core::spectrum::Spectrum::ONE));
            }
            _ => self.scene.warnings.push(format!("texture {texture} names `{path}`, which the package does not hold; its parts came in untextured")),
        }
        self.scene.materials.push(definition);
        let m = self.scene.materials.len() - 1;
        self.materials.insert(key, m);
        m
    }

    fn material(&mut self, key: (String, u32, u32), make: impl FnOnce() -> tequila_scene::material::MaterialDefinition) -> usize {
        if let Some(&m) = self.materials.get(&key) {
            return m;
        }
        self.scene.materials.push(make());
        let m = self.scene.materials.len() - 1;
        self.materials.insert(key, m);
        m
    }
}

/// A printed part's material: plastic of the display colour, a little
/// rough, as a print is; an alpha below one is how see-through it is.
fn plastic(name: &str, colour: [f32; 4]) -> tequila_scene::material::MaterialDefinition {
    use tequila_materials::material_type::MaterialType;
    let c = linear(colour);
    let base = tequila_core::spectrum::Spectrum::from_linear_srgb(vec3(c.x, c.y, c.z));
    let material = tequila_materials::openpbr::Material {
        geometry_opacity: colour[3].clamp(0.0, 1.0),
        ..tequila_materials::openpbr::Material::plastic(base, 0.45)
    };
    tequila_scene::material::MaterialDefinition::new(name, MaterialType::Plastic, material)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_format::zip::ZipWriter;

    fn package(model: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new();
        zip.add("[Content_Types].xml", br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#.to_vec());
        zip.add("_rels/.rels", br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#.to_vec());
        zip.add("3D/3dmodel.model", model.as_bytes().to_vec());
        for (name, bytes) in extra {
            zip.add(*name, bytes.to_vec());
        }
        zip.finish()
    }

    const TETRA: &str = r#"<mesh><vertices><vertex x="0" y="0" z="0"/><vertex x="10" y="0" z="0"/><vertex x="0" y="10" z="0"/><vertex x="0" y="0" z="10"/></vertices>
        <triangles><triangle v1="0" v2="2" v3="1"/><triangle v1="0" v2="1" v3="3"/><triangle v1="0" v2="3" v3="2"/><triangle v1="1" v2="2" v3="3"/></triangles></mesh>"#;

    #[test]
    fn a_single_object_reads_in_its_unit_turned_y_up() {
        let model = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><model unit="inch" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02">
            <metadata name="Title">Bracket</metadata><resources><object id="1" type="model" name="Tetra">{TETRA}</object></resources>
            <build><item objectid="1" partnumber="BR-7"/></build></model>"#
        );
        let scene = read(&package(&model, &[]), "bracket").unwrap();
        assert_eq!(scene.source_unit, Some(LengthUnit::Inch));
        assert_eq!(scene.nodes.len(), 2);
        assert_eq!(scene.nodes[0].name, "bracket");
        assert_eq!(scene.nodes[0].metadata.get("Title").map(String::as_str), Some("Bracket"));
        assert_eq!(scene.nodes[1].name, "Tetra");
        assert_eq!(scene.nodes[1].metadata.get("Part number").map(String::as_str), Some("BR-7"));
        // Z up turned Y up by the top group.
        let up = scene.nodes[0].transform.vector(vec3(0.0, 0.0, 1.0));
        assert!((up - vec3(0.0, 1.0, 0.0)).length() < 1e-6, "{up:?}");
        assert_eq!(scene.meshes[0].positions.len(), 4);
        assert_eq!(scene.meshes[0].material, None);
    }

    #[test]
    fn microns_come_in_as_millimetres() {
        let model = format!(r#"<model unit="micron" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources><object id="1">{TETRA}</object></resources><build><item objectid="1" transform="1 0 0 0 1 0 0 0 1 5000 0 0"/></build></model>"#);
        let scene = read(&package(&model, &[]), "tiny").unwrap();
        assert_eq!(scene.source_unit, Some(LengthUnit::Millimetre));
        let widest = scene.meshes[0].positions.iter().map(|p| p.x).fold(0.0f32, f32::max);
        assert!((widest - 0.01).abs() < 1e-7, "10 microns along X is {widest} mm");
        assert!((scene.nodes[1].transform.point(Vec3::ZERO).x - 5.0).abs() < 1e-5);
    }

    #[test]
    fn transforms_are_row_vectors_with_the_translation_last() {
        // Turned 90° about Z, then moved 7 along X.
        let m = transform_of(Some("0 1 0 -1 0 0 0 0 1 7 0 0"), 1.0).unwrap();
        let p = m.transform_point(vec3(1.0, 0.0, 0.0));
        assert!((p - vec3(7.0, 1.0, 0.0)).length() < 1e-6, "{p:?}");
        assert!(transform_of(Some("1 0 0"), 1.0).is_err());
    }

    #[test]
    fn colours_parse_with_and_without_alpha() {
        assert_eq!(hex_colour("#FF0000"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(hex_colour("#00ff0080").map(|c| (c[1], (c[3] * 255.0).round())), Some((1.0, 128.0)));
        assert_eq!(hex_colour("red"), None);
        assert_eq!(hex_colour("#12345"), None);
    }

    #[test]
    fn a_cycle_of_components_is_an_error_not_a_hang() {
        let model = r#"<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02"><resources>
            <object id="1"><components><component objectid="2"/></components></object>
            <object id="2"><components><component objectid="1"/></components></object>
            </resources><build><item objectid="1"/></build></model>"#;
        assert!(read(&package(model, &[]), "loop").is_err());
    }

    #[test]
    fn broken_packages_are_errors_not_panics() {
        assert!(read(b"not a zip", "x").is_err());
        assert!(read(&package("<model><resources>", &[]), "x").is_err());
        let bad_index = r#"<model><resources><object id="1"><mesh><vertices><vertex x="0" y="0" z="0"/></vertices><triangles><triangle v1="0" v2="1" v3="2"/></triangles></mesh></object></resources><build><item objectid="1"/></build></model>"#;
        assert!(read(&package(bad_index, &[]), "x").is_err());
        let undefined = r#"<model><resources></resources><build><item objectid="9"/></build></model>"#;
        assert!(read(&package(undefined, &[]), "x").is_err());
    }
}
