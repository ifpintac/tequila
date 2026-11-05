//! USDZ export: what is shown, as the one-file USD package Apple's devices
//! open - AR Quick Look on an iPhone or iPad, and Vision Pro - to stand the
//! product in a room at its real size (`docs/03` §6.3, §6.6).
//!
//! A USDZ is a ZIP, stored not compressed, every member's data starting on
//! a 64-byte boundary, the stage first and its pictures after it as PNG.
//! The stage here is binary USD (`usdc.rs`): what Apple's own tools write
//! and the form AR Quick Look was first made to read, and a fraction of the
//! text form's size for a dense product mesh. (A text stage opens on a
//! current device too: Pixar dropped the checker's rule against one in 2023
//! as no longer true of RealityKit. Binary is the one with no exceptions.)
//!
//! What the stage holds:
//!
//! * **Scale and place.** Metres (`metersPerUnit = 1`), Y up, and the
//!   product moved so its box stands on the ground with its middle over the
//!   origin: "full size" on Vision Pro is the real size, standing on the
//!   floor where it is put.
//! * **Geometry.** A `Mesh` for each part shown, named as the part is,
//!   with its normals and UVs, in `Xform`s for its groups with their
//!   transforms. A mesh several parts share is written once, as a class
//!   prim under `/Prototypes`, and each part is an instanceable `Xform`
//!   referencing it - USD's own instancing. The default prim is one `Xform`
//!   over everything, named for the product, of kind `component`.
//! * **Materials.** AR Quick Look and RealityKit draw `UsdPreviewSurface`
//!   and nothing else, so that is all a material is here: Tequila's
//!   OpenPBR mapped onto it (a metal's colour from its measured
//!   reflectance; glass as a faint, glossy, mostly transparent surface,
//!   since a preview surface does not refract). A material whose graph
//!   varies - a procedural, an image, a label, a bump - is baked by
//!   `bake.rs` into PNGs on the part's own UVs or an atlas of its own:
//!   colour and opacity, roughness and metalness, emission, coat, occlusion
//!   and a tangent-space normal map, each written only where it varies and
//!   as a plain value where it does not. A material without images also
//!   carries its whole Tequila definition, as the `.usda` export does, so
//!   Tequila reads it back as it was.
//!
//! Not written: lights and cameras (AR viewers light and frame the product
//! themselves), hidden parts and other model sets, and Multi-Material
//! variants beyond the one shown.

use crate::bake::{self, srgb, Atlas, Extras, Part, Source};
use crate::usd::identifier;
use crate::usdc::{Layer, ListOp, Reference, Specifier, Value};
use crate::zip::ZipWriter;
use tequila_core::math::{Mat4, Vec3};
use tequila_materials::graph::CompiledGraph;
use tequila_materials::openpbr::Material;
use tequila_render_core::shade::TextureTable;
use tequila_scene::document::SceneDocument;
use tequila_scene::material::MaterialDefinition;
use tequila_scene::tree::{MaterialKey, MeshKey, NodeId, NodeKind};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct UsdzOptions {
    /// Pixels on a side of a baked material's images.
    pub bake_size: usize,
    /// What the product is called: the default prim's name and the stage's
    /// file name inside the package. The project's name when empty.
    pub name: String,
}

impl Default for UsdzOptions {
    fn default() -> Self {
        UsdzOptions { bake_size: 1024, name: String::new() }
    }
}

/// What was written.
#[derive(Debug, Default)]
pub struct UsdzExport {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
    /// The package's members in order, the stage first.
    pub members: Vec<String>,
    /// What was exported, in the package's space: metres, standing on the
    /// ground at the origin.
    pub bounds: Option<(Vec3, Vec3)>,
}

/// How much of a clear glass's surface a preview surface keeps: none would
/// leave a bottle invisible; this much, dark, shows its shape and carries
/// its reflections.
const GLASS_SEEN: f32 = 0.15;

/// The colour (linear sRGB) and opacity a preview surface shows for
/// OpenPBR's parameters. A metal's colour is its measured reflectance. A
/// preview surface cannot tint what is behind it, only cover it, so what
/// light passes through is transparency: clear glass is a faint dark film
/// that reflects (white, it went milky), and a tinted one - a scent, a
/// smoked lens - covers more of what is behind it in its own colour, the
/// more the deeper its tint.
pub(crate) fn preview(m: &Material) -> ([f32; 3], f32) {
    let t = m.transmission_weight.clamp(0.0, 1.0);
    let surface = m.solid_color() * m.base_weight;
    // What it shows of what is behind it: for a body of it, a scent in its
    // bottle, the colour it reaches at its own depth, which is a part's
    // size; for a pane, what a thin wall lets through.
    let behind = if m.has_volume() && m.transmission_depth > 0.0 { m.transmission_color * m.transmission_tint } else { m.see_through_tint() };
    let tint = srgb(behind).map(|c| c.clamp(0.0, 1.0));
    let clarity = tint[0].min(tint[1]).min(tint[2]);
    let through = tint.map(|c| c * (1.0 - clarity));
    let covered = GLASS_SEEN + (1.0 - GLASS_SEEN) * (1.0 - (tint[0] + tint[1] + tint[2]) / 3.0);
    let surface = srgb(surface);
    let colour = [0, 1, 2].map(|k| (surface[k] * (1.0 - t) + through[k] * t).min(1.0));
    let opacity = m.geometry_opacity.clamp(0.0, 1.0) * (1.0 - t + covered * t);
    (colour, opacity)
}

fn decode_srgb(v: u8) -> f32 {
    let e = v as f32 / 255.0;
    if e <= 0.04045 {
        e / 12.92
    } else {
        ((e + 0.055) / 1.055).powf(2.4)
    }
}

/// A matrix as USD stores one: rows of a row-vector transform, which is
/// Tequila's column-vector matrix transposed.
fn rows(m: &Mat4) -> Value {
    let mut v = Vec::with_capacity(16);
    for r in 0..4 {
        for c in 0..4 {
            v.push(m.m[c][r] as f64);
        }
    }
    Value::Vecd(v)
}

fn transform(layer: &mut Layer, prim: &str, m: &Mat4) {
    if *m != Mat4::IDENTITY {
        layer.attribute(prim, "xformOp:transform", "matrix4d", Some(rows(m)));
        layer.attribute(prim, "xformOpOrder", "token[]", Some(Value::Tokens(vec!["xformOp:transform".into()]))).set("variability", Value::Variability(true));
    }
}

fn bind(layer: &mut Layer, prim: &str, material: &str) {
    if let Some(spec) = layer.specs.iter_mut().rev().find(|s| s.path == prim) {
        spec.set("apiSchemas", Value::TokenList(ListOp::prepend(vec!["MaterialBindingAPI".into()])));
    }
    layer.relationship(prim, "material:binding", vec![material.to_string()]);
}

/// A mesh prim's geometry: its triangles, normals, the UVs its baked
/// images use (or its own), and baked vertex colours for a dense one.
fn write_mesh(layer: &mut Layer, path: &str, source: &Source, layout: Option<&Atlas>, colours: Option<&[f32]>) {
    let (lo, hi) = source.bounds();
    layer.attribute(path, "extent", "float3[]", Some(Value::Vecfs(3, vec![lo.x, lo.y, lo.z, hi.x, hi.y, hi.z])));
    layer.attribute(path, "points", "point3f[]", Some(Value::Vecfs(3, source.positions.iter().flat_map(|p| [p.x, p.y, p.z]).collect())));
    layer.attribute(path, "faceVertexCounts", "int[]", Some(Value::Ints(vec![3; source.triangles.len()])));
    layer.attribute(path, "faceVertexIndices", "int[]", Some(Value::Ints(source.triangles.iter().flatten().map(|&i| i as i32).collect())));
    layer
        .attribute(path, "normals", "normal3f[]", Some(Value::Vecfs(3, source.normals.iter().flat_map(|n| [n.x, n.y, n.z]).collect())))
        .set("interpolation", Value::Token("vertex".into()));
    // USD's t runs up the image; Tequila's v, as glTF's, down it.
    match layout {
        Some(atlas) if !atlas.own => {
            let size = atlas.size as f32;
            let st: Vec<f32> = atlas.corners.iter().flat_map(|c| c.iter().flat_map(|p| [p[0] / size, 1.0 - p[1] / size])).collect();
            layer.attribute(path, "primvars:st", "texCoord2f[]", Some(Value::Vecfs(2, st))).set("interpolation", Value::Token("faceVarying".into()));
        }
        _ => {
            if let Some(uvs) = source.uvs {
                let st: Vec<f32> = uvs.iter().flat_map(|uv| [uv.x, 1.0 - uv.y]).collect();
                layer.attribute(path, "primvars:st", "texCoord2f[]", Some(Value::Vecfs(2, st))).set("interpolation", Value::Token("vertex".into()));
            }
        }
    }
    if let Some(colours) = colours {
        let rgb: Vec<f32> = colours.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
        layer.attribute(path, "primvars:displayColor", "color3f[]", Some(Value::Vecfs(3, rgb))).set("interpolation", Value::Token("vertex".into()));
    }
    layer.attribute(path, "subdivisionScheme", "token", Some(Value::Token("none".into()))).set("variability", Value::Variability(true));
    layer.attribute(path, "doubleSided", "bool", Some(Value::Bool(true))).set("variability", Value::Variability(true));
}

/// What a material's preview surface takes, each input a value or a
/// connection to an image's channel.
#[derive(Default)]
struct Surface {
    inputs: Vec<(&'static str, &'static str, Input)>,
    /// The images: (shader name, file in the package, colour space, outputs
    /// used, whether it is a normal map).
    images: Vec<(String, String, &'static str, Vec<(&'static str, &'static str)>, bool)>,
    /// A vertex-colour reader feeding the colour, for a dense mesh.
    vertex_colour: bool,
}

enum Input {
    Value(Value),
    /// A channel of an image: its shader and output.
    Image(String, &'static str),
    /// The vertex colours, with a value for viewers that do not read them.
    Vertex(Value),
}

impl Surface {
    fn value(&mut self, name: &'static str, kind: &'static str, value: Value) {
        self.inputs.push((name, kind, Input::Value(value)));
    }

    /// The constants every preview surface takes, from `m`.
    fn constants(m: &Material) -> Surface {
        let mut s = Surface::default();
        let (colour, opacity) = preview(m);
        s.value("diffuseColor", "color3f", Value::Vecf(colour.to_vec()));
        s.value("metallic", "float", Value::Float(m.base_metalness.clamp(0.0, 1.0)));
        s.value("roughness", "float", Value::Float(m.specular_roughness.clamp(0.0, 1.0)));
        s.value("ior", "float", Value::Float(m.specular_ior));
        s.value("opacity", "float", Value::Float(opacity));
        s.value("clearcoat", "float", Value::Float(m.coat_weight.clamp(0.0, 1.0)));
        s.value("clearcoatRoughness", "float", Value::Float(m.coat_roughness.clamp(0.0, 1.0)));
        let glow = srgb(m.emission_color * (m.emission_luminance / 100.0));
        s.value("emissiveColor", "color3f", Value::Vecf(glow.to_vec()));
        s
    }

    fn replace(&mut self, name: &'static str, input: Input) {
        match self.inputs.iter_mut().find(|i| i.0 == name) {
            Some(slot) => slot.2 = input,
            None => self.inputs.push((name, if name == "normal" { "normal3f" } else { "float" }, input)),
        }
    }

    /// A value a bake found the same all over the part: the material's own
    /// constant where the texels are it to their eight bits, so what the
    /// graph does not drive keeps its exact value; else what they are.
    fn settle(&mut self, name: &'static str, value: Value) {
        let same = match (self.inputs.iter().find(|i| i.0 == name).map(|i| &i.2), &value) {
            (Some(Input::Value(Value::Float(a))), Value::Float(b)) => (a - b).abs() <= 1.5 / 255.0,
            (Some(Input::Value(Value::Vecf(a))), Value::Vecf(b)) => {
                a.iter().zip(b).all(|(a, b)| (bake::encode_srgb(*a) as i32 - bake::encode_srgb(*b) as i32).abs() <= 1)
            }
            _ => false,
        };
        if !same {
            self.replace(name, Input::Value(value));
        }
    }
}

/// The images a bake made, each written where it varies and a value where
/// it does not, into `surface` and `files`.
fn baked_surface(material: &str, baked: &bake::Baked, extras: Extras, surface: &mut Surface, files: &mut Vec<(String, Vec<u8>)>) {
    let size = baked.size;
    let covered = &baked.covered;
    let mut file = |stem: &str, image: &[u8], channels: usize| -> String {
        let name = format!("textures/{material}_{stem}.png");
        files.push((name.clone(), bake::png(size, size, image, channels)));
        name
    };
    // Colour and opacity.
    let colour_flat = bake::uniform(&baked.colour, covered, [true, true, true, false], 1);
    let alpha_flat = bake::uniform(&baked.colour, covered, [false, false, false, true], 1);
    if colour_flat.is_none() || alpha_flat.is_none() {
        let path = file("colour", &baked.colour, if alpha_flat.is_some() { 3 } else { 4 });
        let mut outputs = Vec::new();
        if colour_flat.is_none() {
            surface.replace("diffuseColor", Input::Image("Colour".into(), "rgb"));
            outputs.push(("rgb", "float3"));
        }
        if alpha_flat.is_none() {
            surface.replace("opacity", Input::Image("Colour".into(), "a"));
            outputs.push(("a", "float"));
        }
        surface.images.push(("Colour".into(), path, "sRGB", outputs, false));
    }
    if let Some(c) = colour_flat {
        surface.settle("diffuseColor", Value::Vecf(vec![decode_srgb(c[0]), decode_srgb(c[1]), decode_srgb(c[2])]));
    }
    if let Some(a) = alpha_flat {
        surface.settle("opacity", Value::Float(a[3] as f32 / 255.0));
    }

    // Occlusion, roughness and metalness, packed as glTF packs them.
    let mut packed: Vec<(&'static str, &'static str, usize)> = vec![("roughness", "g", 1), ("metallic", "b", 2)];
    if extras.occlusion {
        packed.push(("occlusion", "r", 0));
    }
    let mut outputs = Vec::new();
    for (input, channel, k) in packed {
        let mut mask = [false; 4];
        mask[k] = true;
        match bake::uniform(&baked.metal_rough, covered, mask, 1) {
            Some(p) => surface.settle(input, Value::Float(p[k] as f32 / 255.0)),
            None => {
                surface.replace(input, Input::Image("MetalRough".into(), channel));
                outputs.push((channel, "float"));
            }
        }
    }
    if !outputs.is_empty() {
        let path = file("metal_rough", &baked.metal_rough, 3);
        surface.images.push(("MetalRough".into(), path, "raw", outputs, false));
    }

    if let Some(glow) = &baked.glow {
        match bake::uniform(glow, covered, [true, true, true, false], 1) {
            Some(c) => surface.settle("emissiveColor", Value::Vecf(vec![decode_srgb(c[0]), decode_srgb(c[1]), decode_srgb(c[2])])),
            None => {
                let path = file("emission", glow, 3);
                surface.replace("emissiveColor", Input::Image("Emission".into(), "rgb"));
                surface.images.push(("Emission".into(), path, "sRGB", vec![("rgb", "float3")], false));
            }
        }
    }

    if let Some(coat) = &baked.coat {
        let mut outputs = Vec::new();
        for (input, channel, k) in [("clearcoat", "r", 0), ("clearcoatRoughness", "g", 1)] {
            let mut mask = [false; 4];
            mask[k] = true;
            match bake::uniform(coat, covered, mask, 1) {
                Some(p) => surface.settle(input, Value::Float(p[k] as f32 / 255.0)),
                None => {
                    surface.replace(input, Input::Image("Coat".into(), channel));
                    outputs.push((channel, "float"));
                }
            }
        }
        if !outputs.is_empty() {
            let path = file("coat", coat, 3);
            surface.images.push(("Coat".into(), path, "raw", outputs, false));
        }
    }

    if let Some(normal) = &baked.normal {
        // A flat map is no map.
        let flat = bake::uniform(normal, covered, [true, true, true, false], 2).is_some_and(|p| p[0].abs_diff(128) <= 2 && p[1].abs_diff(128) <= 2);
        if !flat {
            let path = file("normal", normal, 3);
            surface.replace("normal", Input::Image("Normal".into(), "rgb"));
            surface.images.push(("Normal".into(), path, "raw", vec![("rgb", "float3")], true));
        }
    }
}

/// A material prim with its preview surface and the shaders feeding it.
fn material_prim(layer: &mut Layer, path: &str, definition: &MaterialDefinition, surface: &Surface) {
    layer.prim(path, Specifier::Def, "Material");
    let shader = format!("{path}/PreviewSurface");
    layer.connection(path, "outputs:surface", "token", &format!("{shader}.outputs:surface"));
    layer.attribute(path, "tequila:name", "string", Some(Value::String(definition.name.clone()))).set("custom", Value::Bool(true));
    layer.attribute(path, "tequila:materialType", "string", Some(Value::String(definition.material_type.id().to_string()))).set("custom", Value::Bool(true));
    // The whole material, for Tequila to read back, when it has no images
    // that would have to travel with it.
    let images = definition.graph.compile().map(|p| !p.requirements.textures.is_empty()).unwrap_or(true);
    if !images {
        let json = crate::json_write::to_string(&crate::material_to_json(MaterialKey(0), definition));
        layer.attribute(path, "tequila:definition", "string", Some(Value::String(json))).set("custom", Value::Bool(true));
    }

    layer.prim(&shader, Specifier::Def, "Shader");
    layer.attribute(&shader, "info:id", "token", Some(Value::Token("UsdPreviewSurface".into()))).set("variability", Value::Variability(true));
    for (name, kind, input) in &surface.inputs {
        let attribute = format!("inputs:{name}");
        match input {
            Input::Value(v) => {
                layer.attribute(&shader, &attribute, kind, Some(v.clone()));
            }
            Input::Image(image, output) => {
                layer.connection(&shader, &attribute, kind, &format!("{path}/{image}.outputs:{output}"));
            }
            Input::Vertex(v) => {
                layer.connection(&shader, &attribute, kind, &format!("{path}/VertexColour.outputs:result")).set("default", v.clone());
            }
        }
    }
    layer.attribute(&shader, "outputs:surface", "token", None);

    if !surface.images.is_empty() {
        let reader = format!("{path}/TexCoord");
        layer.prim(&reader, Specifier::Def, "Shader");
        layer.attribute(&reader, "info:id", "token", Some(Value::Token("UsdPrimvarReader_float2".into()))).set("variability", Value::Variability(true));
        layer.attribute(&reader, "inputs:varname", "string", Some(Value::String("st".into())));
        layer.attribute(&reader, "outputs:result", "float2", None);
    }
    for (name, file, space, outputs, normal) in &surface.images {
        let image = format!("{path}/{name}");
        layer.prim(&image, Specifier::Def, "Shader");
        layer.attribute(&image, "info:id", "token", Some(Value::Token("UsdUVTexture".into()))).set("variability", Value::Variability(true));
        layer.attribute(&image, "inputs:file", "asset", Some(Value::Asset(file.clone())));
        layer.connection(&image, "inputs:st", "float2", &format!("{path}/TexCoord.outputs:result"));
        layer.attribute(&image, "inputs:sourceColorSpace", "token", Some(Value::Token((*space).into())));
        layer.attribute(&image, "inputs:wrapS", "token", Some(Value::Token("clamp".into())));
        layer.attribute(&image, "inputs:wrapT", "token", Some(Value::Token("clamp".into())));
        if *normal {
            // Eight-bit texels to the -1..1 a normal is.
            layer.attribute(&image, "inputs:scale", "float4", Some(Value::Vecf(vec![2.0, 2.0, 2.0, 1.0])));
            layer.attribute(&image, "inputs:bias", "float4", Some(Value::Vecf(vec![-1.0, -1.0, -1.0, 0.0])));
        }
        for (output, kind) in outputs {
            layer.attribute(&image, &format!("outputs:{output}"), kind, None);
        }
    }
    if surface.vertex_colour {
        let reader = format!("{path}/VertexColour");
        layer.prim(&reader, Specifier::Def, "Shader");
        layer.attribute(&reader, "info:id", "token", Some(Value::Token("UsdPrimvarReader_float3".into()))).set("variability", Value::Variability(true));
        layer.attribute(&reader, "inputs:varname", "string", Some(Value::String("displayColor".into())));
        layer.attribute(&reader, "outputs:result", "float3", None);
    }
}

/// A mesh's place in the package: written inline at its one part, or once
/// as a prototype that several parts reference.
struct MeshPlan<'a> {
    source: Source<'a>,
    /// Where its baked images go, when one of its materials bakes.
    layout: Option<Atlas>,
    /// Baked vertex colours, for a dense mesh with no layout.
    colours: Option<Vec<f32>>,
    /// The prototype's path, for a mesh several parts share.
    prototype: Option<String>,
}

/// Write what is shown in `document` as a `.usdz`.
pub fn export(document: &SceneDocument, options: &UsdzOptions) -> UsdzExport {
    let mut out = UsdzExport::default();
    let tree = &document.tree;
    let size = options.bake_size.clamp(16, 8192);

    // The parts shown, and the mesh each is.
    let mut shown: Vec<NodeId> = Vec::new();
    let mut users: BTreeMap<MeshKey, Vec<NodeId>> = BTreeMap::new();
    for id in tree.depth_first() {
        let Some(node) = tree.get(id) else { continue };
        let NodeKind::Part { mesh } = node.kind else { continue };
        if !tree.is_effectively_visible(id) || !document.is_shown(id) {
            continue;
        }
        let Some(geometry) = document.geometry.get(mesh) else { continue };
        if geometry.indices.len() < 3 {
            continue;
        }
        shown.push(id);
        users.entry(mesh).or_default().push(id);
    }
    let shown_set: BTreeSet<NodeId> = shown.iter().copied().collect();

    // Where it all is, to stand it on the ground at the origin.
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for &id in &shown {
        let Some(NodeKind::Part { mesh }) = tree.get(id).map(|n| n.kind.clone()) else { continue };
        let world = tree.world_transform(id);
        if let Some(geometry) = document.geometry.get(mesh) {
            for p in &geometry.positions {
                let w = world.point(*p);
                lo = lo.min(w);
                hi = hi.max(w);
            }
        }
    }
    let offset = if lo.x <= hi.x { tequila_core::math::vec3(-(lo.x + hi.x) * 0.5, -lo.y, -(lo.z + hi.z) * 0.5) } else { Vec3::ZERO };
    if lo.x <= hi.x {
        out.bounds = Some((lo + offset, hi + offset));
    }

    // The programs of the materials that vary, and the images they read.
    let mut programs: BTreeMap<MaterialKey, Option<Arc<CompiledGraph>>> = BTreeMap::new();
    let mut textures = TextureTable::new();
    for &id in &shown {
        let Some(key) = tree.get(id).and_then(|n| n.material) else { continue };
        if programs.contains_key(&key) {
            continue;
        }
        let program = document
            .materials
            .get(key)
            .filter(|d| d.graph.len() > 1 || !d.graph.decals.is_empty())
            .and_then(|d| d.graph.compile().ok())
            .filter(|p| !p.driven.is_empty() || !p.decals.is_empty())
            .map(Arc::new);
        if let Some(program) = &program {
            for texture in &program.requirements.textures {
                if textures.get(*texture).is_none() {
                    match document.textures.get(*texture).and_then(tequila_scene::snapshot::load_texture) {
                        Some(loaded) => textures.insert(*texture, Arc::new(loaded)),
                        None => out.warnings.push(format!(
                            "an image of “{}” could not be read; its fallback is baked",
                            document.materials.get(key).map(|d| d.name.as_str()).unwrap_or("?")
                        )),
                    }
                }
            }
        }
        programs.insert(key, program);
    }

    let name = {
        let wanted = if !options.name.trim().is_empty() {
            options.name.clone()
        } else if !document.meta.name.trim().is_empty() {
            document.meta.name.clone()
        } else {
            "Product".to_string()
        };
        identifier(wanted.trim(), &mut BTreeSet::new())
    };
    let root = format!("/{name}");
    let mut layer = Layer::new();
    layer
        .root()
        .set("defaultPrim", Value::Token(name.clone()))
        .set("documentation", Value::String("Written by Tequila".into()))
        .set("metersPerUnit", Value::Double(1.0))
        .set("upAxis", Value::Token("Y".into()));
    layer.prim(&root, Specifier::Def, "Xform").set("kind", Value::Token("component".into()));
    if offset != Vec3::ZERO {
        layer.attribute(&root, "xformOp:translate", "double3", Some(Value::Vecd(vec![offset.x as f64, offset.y as f64, offset.z as f64])));
        layer.attribute(&root, "xformOpOrder", "token[]", Some(Value::Tokens(vec!["xformOp:translate".into()]))).set("variability", Value::Variability(true));
    }

    // Each mesh's layout, and its prototype when parts share it.
    let mut plans: BTreeMap<MeshKey, MeshPlan> = BTreeMap::new();
    let mut prototype_names = BTreeSet::new();
    for (&key, parts) in &users {
        let Some(geometry) = document.geometry.get(key) else { continue };
        let source = Source::of(geometry);
        let bakes = parts.iter().any(|id| tree.get(*id).and_then(|n| n.material).is_some_and(|m| programs.get(&m).is_some_and(|p| p.is_some())));
        let layout = if bakes { bake::layout(&source, size) } else { None };
        let prototype = (parts.len() > 1).then(|| {
            let first = tree.get(parts[0]).map(|n| n.name.clone()).unwrap_or_default();
            format!("/Prototypes/{}", identifier(&first, &mut prototype_names))
        });
        plans.insert(key, MeshPlan { source, layout, colours: None, prototype });
    }

    // The materials: a plain one once, a baked one for each mesh it is on.
    let materials_scope = format!("{root}/Materials");
    let mut material_names = BTreeSet::new();
    let mut plain: BTreeMap<MaterialKey, String> = BTreeMap::new();
    let mut baked: BTreeMap<(MeshKey, MaterialKey), String> = BTreeMap::new();
    let mut material_specs: Vec<(String, MaterialKey, Surface)> = Vec::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for &id in &shown {
        let Some(node) = tree.get(id) else { continue };
        let (NodeKind::Part { mesh }, Some(key)) = (node.kind.clone(), node.material) else { continue };
        let Some(definition) = document.materials.get(key) else { continue };
        match programs.get(&key).cloned().flatten() {
            None => {
                if !plain.contains_key(&key) {
                    let path = format!("{materials_scope}/{}", identifier(&definition.name, &mut material_names));
                    material_specs.push((path.clone(), key, Surface::constants(definition.material())));
                    plain.insert(key, path);
                }
            }
            Some(program) => {
                if baked.contains_key(&(mesh, key)) {
                    continue;
                }
                let Some(plan) = plans.get_mut(&mesh) else { continue };
                let world = tree.world_transform(id);
                let part = Part { source: &plan.source, world, scale: 1.0 / world.inv_length_scale(), bounds: plan.source.bounds() };
                let label = identifier(&format!("{}_{}", definition.name, node.name), &mut material_names);
                let path = format!("{materials_scope}/{label}");
                let mut surface = Surface::constants(&program.constants);
                match &plan.layout {
                    Some(atlas) => {
                        let extras = Extras::driven_by(&program, &textures);
                        let images = bake::bake(&part, atlas, &program, &textures, preview, extras);
                        baked_surface(&label, &images, extras, &mut surface, &mut files);
                    }
                    None => {
                        // Too dense for an atlas, and no UVs to bake into:
                        // its colours at its vertices.
                        let (colours, rough, metal) = bake::bake_vertices(&part, &program, &textures, preview);
                        let n = (colours.len() / 4).max(1) as f32;
                        let mean = |k: usize| colours.iter().skip(k).step_by(4).sum::<f32>() / n;
                        surface.replace("roughness", Input::Value(Value::Float(rough.clamp(0.0, 1.0))));
                        surface.replace("metallic", Input::Value(Value::Float(metal.clamp(0.0, 1.0))));
                        surface.replace("diffuseColor", Input::Vertex(Value::Vecf(vec![mean(0), mean(1), mean(2)])));
                        surface.vertex_colour = true;
                        if plan.colours.is_none() {
                            plan.colours = Some(colours);
                        } else {
                            out.warnings.push(format!("“{}” is too dense to bake each of its materials; “{}” shows the first one's colours", node.name, definition.name));
                        }
                        out.warnings.push(format!("“{}” has no UVs and is too dense to bake “{}” into images; its colours are at its vertices", node.name, definition.name));
                    }
                }
                material_specs.push((path.clone(), key, surface));
                baked.insert((mesh, key), path);
            }
        }
    }

    // The hierarchy, groups only where something shown is under them.
    fn has_shown(document: &SceneDocument, id: NodeId, shown: &BTreeSet<NodeId>) -> bool {
        shown.contains(&id) || document.tree.get(id).is_some_and(|n| n.children.iter().any(|c| has_shown(document, *c, shown)))
    }
    #[allow(clippy::too_many_arguments)]
    fn node(
        document: &SceneDocument,
        id: NodeId,
        parent: &str,
        taken: &mut BTreeSet<String>,
        shown: &BTreeSet<NodeId>,
        plans: &BTreeMap<MeshKey, MeshPlan>,
        plain: &BTreeMap<MaterialKey, String>,
        baked: &BTreeMap<(MeshKey, MaterialKey), String>,
        layer: &mut Layer,
    ) {
        let Some(n) = document.tree.get(id) else { return };
        if !has_shown(document, id, shown) {
            return;
        }
        let path = format!("{parent}/{}", identifier(&n.name, taken));
        let children: Vec<NodeId> = n.children.iter().copied().filter(|c| has_shown(document, *c, shown)).collect();
        let material = |mesh: MeshKey| n.material.and_then(|k| baked.get(&(mesh, k)).or_else(|| plain.get(&k)));
        let mut child_names = BTreeSet::new();
        match (&n.kind, shown.contains(&id)) {
            (NodeKind::Part { mesh }, true) => {
                let plan = plans.get(mesh);
                match plan.and_then(|p| p.prototype.as_ref()) {
                    Some(prototype) => {
                        // One of several parts sharing a mesh: an instance
                        // of it, wearing its own material.
                        layer
                            .prim(&path, Specifier::Def, "Xform")
                            .set("instanceable", Value::Bool(true))
                            .set("references", Value::References(ListOp::prepend(vec![Reference { asset: String::new(), prim: prototype.clone() }])));
                        transform(layer, &path, &n.transform.matrix);
                        if let Some(material) = material(*mesh) {
                            bind(layer, &path, material);
                        }
                    }
                    None if children.is_empty() => {
                        layer.prim(&path, Specifier::Def, "Mesh");
                        if let Some(plan) = plan {
                            write_mesh(layer, &path, &plan.source, plan.layout.as_ref(), plan.colours.as_deref());
                        }
                        transform(layer, &path, &n.transform.matrix);
                        if let Some(material) = material(*mesh) {
                            bind(layer, &path, material);
                        }
                    }
                    None => {
                        // A part with parts under it: a mesh cannot hold
                        // another, so its own is a child of an Xform.
                        layer.prim(&path, Specifier::Def, "Xform");
                        transform(layer, &path, &n.transform.matrix);
                        let own = format!("{path}/{}", identifier(&format!("{}_Mesh", n.name), &mut child_names));
                        layer.prim(&own, Specifier::Def, "Mesh");
                        if let Some(plan) = plan {
                            write_mesh(layer, &own, &plan.source, plan.layout.as_ref(), plan.colours.as_deref());
                        }
                        if let Some(material) = material(*mesh) {
                            bind(layer, &own, material);
                        }
                    }
                }
            }
            _ => {
                layer.prim(&path, Specifier::Def, "Xform");
                transform(layer, &path, &n.transform.matrix);
            }
        }
        for child in children {
            node(document, child, &path, &mut child_names, shown, plans, plain, baked, layer);
        }
    }
    let mut taken = BTreeSet::new();
    taken.insert("Materials".to_string());
    for &top in tree.roots() {
        node(document, top, &root, &mut taken, &shown_set, &plans, &plain, &baked, &mut layer);
    }

    if !material_specs.is_empty() {
        layer.prim(&materials_scope, Specifier::Def, "Scope");
        for (path, key, surface) in &material_specs {
            if let Some(definition) = document.materials.get(*key) {
                material_prim(&mut layer, path, definition, surface);
            }
        }
    }

    // The prototypes of shared meshes, outside the product: abstract, so
    // drawn only where a part references one.
    if plans.values().any(|p| p.prototype.is_some()) {
        layer.prim("/Prototypes", Specifier::Class, "");
        for plan in plans.values() {
            let Some(prototype) = &plan.prototype else { continue };
            layer.prim(prototype, Specifier::Def, "Xform");
            let geometry = format!("{prototype}/Mesh");
            layer.prim(&geometry, Specifier::Def, "Mesh");
            write_mesh(&mut layer, &geometry, &plan.source, plan.layout.as_ref(), plan.colours.as_deref());
        }
    }

    // The package: the stage first, then its images, each on 64 bytes.
    let stage = format!("{name}.usdc");
    let mut zip = ZipWriter::aligned(64);
    out.members.push(stage.clone());
    zip.add(stage, crate::usdc::write(&layer));
    for (file, bytes) in files {
        out.members.push(file.clone());
        zip.add(file, bytes);
    }
    out.bytes = zip.finish();
    out
}
