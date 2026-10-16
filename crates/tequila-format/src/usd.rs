//! USD export, as `.usda` text (`docs/03` §6.3).
//!
//! What is shown goes out as a USD stage, Y up, metres: an `Xform` for each
//! group and a `Mesh` for each part, at their places in the hierarchy, a
//! `Camera` for the active camera, and a `Material` for each material used.
//!
//! A material carries two shaders. A `UsdPreviewSurface`, which every USD
//! viewer draws - Apple's Quick Look, Blender, Omniverse, usdview. And an
//! OpenPBR surface as MaterialX names it (`ND_open_pbr_surface_surfaceshader`),
//! every parameter written, which is Tequila's own model: a renderer that
//! knows MaterialX reads the material as it is, and so does Tequila
//! importing the file again - the round trip loses nothing. A material with
//! a graph and no images carries its whole definition as well
//! (`tequila:definition`), so a procedural wood comes back as the wood.
//!
//! Colours go out in linear Rec.709, as both shaders expect, from the
//! rendering space (ACEScg). Matrices are written row-vector, as USD has
//! them; UVs with `v` up, as USD has them.

use crate::json_write;
use tequila_core::math::Mat4;
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::{Param, ParamValue};
use tequila_scene::document::SceneDocument;
use tequila_scene::tree::{MaterialKey, NodeId, NodeKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;

/// The info:id of OpenPBR's MaterialX surface shader.
pub const OPEN_PBR: &str = "ND_open_pbr_surface_surfaceshader";

/// A USD identifier from a name: letters, digits and underscores, not
/// starting with a digit, unique among `taken`.
fn identifier(name: &str, taken: &mut BTreeSet<String>) -> String {
    let mut out: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    let mut unique = out.clone();
    let mut n = 1;
    while !taken.insert(unique.clone()) {
        n += 1;
        unique = format!("{out}_{n}");
    }
    unique
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn number(v: f32) -> String {
    if v.is_finite() {
        let s = format!("{v}");
        if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") }
    } else {
        "0.0".into()
    }
}

fn rec709(c: Spectrum) -> String {
    let v = c.to_linear_srgb();
    format!("({}, {}, {})", number(v.x), number(v.y), number(v.z))
}

/// A matrix as USD writes one: rows of a row-vector transform, which is
/// Tequila's column-vector matrix transposed.
fn matrix(m: &Mat4) -> String {
    let rows: Vec<String> = (0..4)
        .map(|r| format!("({}, {}, {}, {})", number(m.m[0][r]), number(m.m[1][r]), number(m.m[2][r]), number(m.m[3][r])))
        .collect();
    format!("( {} )", rows.join(", "))
}

/// Write what is shown in `document` as a `.usda` stage.
pub fn export(document: &SceneDocument) -> String {
    let tree = &document.tree;
    let mut out = String::new();
    let _ = writeln!(out, "#usda 1.0\n(\n    defaultPrim = \"Scene\"\n    doc = \"Written by Tequila\"\n    metersPerUnit = 1\n    upAxis = \"Y\"\n)\n");
    let _ = writeln!(out, "def Xform \"Scene\" (\n    kind = \"assembly\"\n)\n{{");

    // The materials the shown parts wear, named once each.
    let mut used: BTreeMap<MaterialKey, String> = BTreeMap::new();
    let mut names = BTreeSet::new();
    for node in tree.iter() {
        if matches!(node.kind, NodeKind::Part { .. }) && document.is_shown(node.id) {
            if let Some(key) = node.material {
                if !used.contains_key(&key) {
                    let name = document.materials.get(key).map(|m| m.name.clone()).unwrap_or_else(|| "Material".into());
                    used.insert(key, identifier(&name, &mut names));
                }
            }
        }
    }

    let mut taken = BTreeSet::new();
    taken.insert("Materials".to_string());
    taken.insert("Camera".to_string());
    for &root in tree.roots() {
        node(document, root, 1, &used, &mut taken, &mut out);
    }

    // The camera.
    let saved = document.cameras.active();
    let camera = &saved.camera;
    let focal = match camera.projection {
        tequila_render_core::camera::Projection::Perspective { focal_length_mm }
        | tequila_render_core::camera::Projection::Shift { focal_length_mm, .. } => focal_length_mm,
        _ => 50.0,
    };
    let aspect = document.render.width.max(1) as f32 / document.render.height.max(1) as f32;
    let _ = writeln!(out, "\n    def Camera \"Camera\"\n    {{");
    // USD measures lenses in tenths of a scene unit: at a metre a unit, a
    // tenth is 100 mm, so 50 mm is 0.5. Written in millimetres, Blender
    // reads a 50 mm lens as five metres long.
    let tenths = |mm: f32| mm / 100.0;
    let _ = writeln!(out, "        float focalLength = {}", number(tenths(focal)));
    let _ = writeln!(out, "        float horizontalAperture = {}", number(tenths(camera.sensor_width_mm)));
    let _ = writeln!(out, "        float verticalAperture = {}", number(tenths(camera.sensor_width_mm / aspect)));
    let _ = writeln!(out, "        float focusDistance = {}", number(camera.focus_distance));
    let _ = writeln!(out, "        matrix4d xformOp:transform = {}", matrix(&camera.camera_to_world));
    let _ = writeln!(out, "        uniform token[] xformOpOrder = [\"xformOp:transform\"]\n    }}");

    // The materials.
    let _ = writeln!(out, "\n    def Scope \"Materials\"\n    {{");
    for (key, name) in &used {
        let Some(definition) = document.materials.get(*key) else { continue };
        material(definition, name, &mut out);
    }
    let _ = writeln!(out, "    }}\n}}");
    out
}

fn node(document: &SceneDocument, id: NodeId, depth: usize, materials: &BTreeMap<MaterialKey, String>, taken: &mut BTreeSet<String>, out: &mut String) {
    let tree = &document.tree;
    let Some(n) = tree.get(id) else { return };
    let pad = "    ".repeat(depth);
    let name = identifier(&n.name, taken);
    let mut children_taken = BTreeSet::new();
    match &n.kind {
        NodeKind::Group => {
            let _ = writeln!(out, "\n{pad}def Xform \"{name}\"\n{pad}{{");
        }
        NodeKind::Part { mesh } => {
            let Some(geometry) = document.geometry.get(*mesh) else { return };
            let _ = writeln!(out, "\n{pad}def Mesh \"{name}\" (\n{pad}    prepend apiSchemas = [\"MaterialBindingAPI\"]\n{pad})\n{pad}{{");
            let inner = format!("{pad}    ");
            let points: Vec<String> = geometry.positions.iter().map(|p| format!("({}, {}, {})", number(p.x), number(p.y), number(p.z))).collect();
            let _ = writeln!(out, "{inner}point3f[] points = [{}]", points.join(", "));
            let triangles = geometry.indices.len() / 3;
            let _ = writeln!(out, "{inner}int[] faceVertexCounts = [{}]", vec!["3"; triangles].join(", "));
            let indices: Vec<String> = geometry.indices.iter().map(|i| i.to_string()).collect();
            let _ = writeln!(out, "{inner}int[] faceVertexIndices = [{}]", indices.join(", "));
            if geometry.normals.len() == geometry.positions.len() {
                let normals: Vec<String> = geometry.normals.iter().map(|v| format!("({}, {}, {})", number(v.x), number(v.y), number(v.z))).collect();
                let _ = writeln!(out, "{inner}normal3f[] normals = [{}] (\n{inner}    interpolation = \"vertex\"\n{inner})", normals.join(", "));
            }
            if let Some(uvs) = geometry.uv_sets.first().filter(|uv| uv.len() == geometry.positions.len()) {
                // USD's v runs up the image; Tequila's, as glTF's, down.
                let st: Vec<String> = uvs.iter().map(|uv| format!("({}, {})", number(uv.x), number(1.0 - uv.y))).collect();
                let _ = writeln!(out, "{inner}texCoord2f[] primvars:st = [{}] (\n{inner}    interpolation = \"vertex\"\n{inner})", st.join(", "));
            }
            let _ = writeln!(out, "{inner}uniform token subdivisionScheme = \"none\"");
            let _ = writeln!(out, "{inner}uniform bool doubleSided = 1");
            if let Some(material) = n.material.and_then(|k| materials.get(&k)) {
                let _ = writeln!(out, "{inner}rel material:binding = </Scene/Materials/{material}>");
            }
        }
        // Lights and cutaways have no USD form here.
        _ => return,
    }
    let inner = format!("{pad}    ");
    let _ = writeln!(out, "{inner}matrix4d xformOp:transform = {}", matrix(&n.transform.matrix));
    let _ = writeln!(out, "{inner}uniform token[] xformOpOrder = [\"xformOp:transform\"]");
    if !n.visible {
        let _ = writeln!(out, "{inner}token visibility = \"invisible\"");
    }
    if !n.metadata.is_empty() {
        let fields: Vec<String> = n.metadata.iter().map(|(k, v)| format!("{inner}    string {} = {}", quote(k), quote(v))).collect();
        let _ = writeln!(out, "{inner}custom dictionary tequila:metadata = {{\n{}\n{inner}}}", fields.join("\n"));
    }
    for &child in &n.children {
        if let Some(c) = tree.get(child) {
            if !document.model_sets.allows(tree, c.id) && !matches!(c.kind, NodeKind::Group) {
                continue;
            }
        }
        node(document, child, depth + 1, materials, &mut children_taken, out);
    }
    let _ = writeln!(out, "{pad}}}");
}

fn material(definition: &tequila_scene::material::MaterialDefinition, name: &str, out: &mut String) {
    let m = definition.material();
    let pad = "        ";
    let _ = writeln!(out, "\n{pad}def Material \"{name}\"\n{pad}{{");
    let _ = writeln!(out, "{pad}    token outputs:surface.connect = </Scene/Materials/{name}/PreviewSurface.outputs:surface>");
    let _ = writeln!(out, "{pad}    token outputs:mtlx:surface.connect = </Scene/Materials/{name}/OpenPBR.outputs:surface>");
    let _ = writeln!(out, "{pad}    custom string tequila:name = {}", quote(&definition.name));
    let _ = writeln!(out, "{pad}    custom string tequila:materialType = {}", quote(definition.material_type.id()));
    let textured = definition.graph.compile().map(|p| !p.requirements.textures.is_empty()).unwrap_or(true);
    if definition.has_graph() && !textured {
        let json = json_write::to_string(&crate::material_to_json(MaterialKey(0), definition));
        let _ = writeln!(out, "{pad}    custom string tequila:definition = {}", quote(&json));
    }

    // What every viewer draws.
    let base = m.base_color * m.base_weight;
    let _ = writeln!(out, "\n{pad}    def Shader \"PreviewSurface\"\n{pad}    {{");
    let inner = format!("{pad}        ");
    let _ = writeln!(out, "{inner}uniform token info:id = \"UsdPreviewSurface\"");
    let _ = writeln!(out, "{inner}color3f inputs:diffuseColor = {}", rec709(base));
    let _ = writeln!(out, "{inner}float inputs:metallic = {}", number(m.base_metalness));
    let _ = writeln!(out, "{inner}float inputs:roughness = {}", number(m.specular_roughness));
    let _ = writeln!(out, "{inner}float inputs:ior = {}", number(m.specular_ior));
    let _ = writeln!(out, "{inner}float inputs:opacity = {}", number((m.geometry_opacity * (1.0 - m.transmission_weight)).clamp(0.0, 1.0)));
    let _ = writeln!(out, "{inner}float inputs:clearcoat = {}", number(m.coat_weight));
    let _ = writeln!(out, "{inner}float inputs:clearcoatRoughness = {}", number(m.coat_roughness));
    let glow = m.emission_color * (m.emission_luminance / 100.0);
    let _ = writeln!(out, "{inner}color3f inputs:emissiveColor = {}", rec709(glow));
    let _ = writeln!(out, "{inner}token outputs:surface\n{pad}    }}");

    // Tequila's own model, every parameter.
    let _ = writeln!(out, "\n{pad}    def Shader \"OpenPBR\"\n{pad}    {{");
    let _ = writeln!(out, "{inner}uniform token info:id = \"{OPEN_PBR}\"");
    for param in Param::ALL {
        match param.read(m) {
            Some(ParamValue::Scalar(v)) => {
                let _ = writeln!(out, "{inner}float inputs:{} = {}", param.key(), number(v));
            }
            Some(ParamValue::Color(c)) => {
                let _ = writeln!(out, "{inner}color3f inputs:{} = {}", param.key(), rec709(c));
            }
            Some(ParamValue::Flag(b)) => {
                let _ = writeln!(out, "{inner}bool inputs:{} = {}", param.key(), if b { 1 } else { 0 });
            }
            Some(ParamValue::Metal(ior)) => {
                // Not OpenPBR's: a measured metal, kept for the round trip.
                let n = ior.n.to_linear_srgb();
                let k = ior.k.to_linear_srgb();
                let _ = writeln!(out, "{inner}custom float3 tequila:metal_ior_n = ({}, {}, {})", number(n.x), number(n.y), number(n.z));
                let _ = writeln!(out, "{inner}custom float3 tequila:metal_ior_k = ({}, {}, {})", number(k.x), number(k.y), number(k.z));
            }
            None => {}
        }
    }
    let _ = writeln!(out, "{inner}token outputs:surface\n{pad}    }}\n{pad}}}");
}
