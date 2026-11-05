//! FBX, through ufbx.
//!
//! FBX is Autodesk's format and has no published specification; what makes
//! it readable without Autodesk's SDK is ufbx (<https://github.com/ufbx/ufbx>),
//! an open-source reader in one C file, MIT or public domain, that reads
//! binary and ASCII FBX from 2010 on (6.1 to 7.7). The `ufbx` crate is its
//! Rust binding. It does the parts of FBX that are hard to get right and
//! undocumented besides - pivots, pre- and post-rotations, Maya's segment
//! scale compensation, geometric transforms, the axis system and unit scale
//! - and this module turns what it gives into a [`RawScene`]:
//!
//! - The node hierarchy, each node where the file put it relative to its
//!   parent, under one group named after the file when there are several
//!   top nodes, as a glTF's are. A node whose subtree holds no mesh - a
//!   camera, a light, a bone, an empty with nothing under it - is left out.
//! - Meshes, their polygons divided into triangles by ufbx, with their
//!   normals, first UV set and vertex colours. A mesh with several
//!   materials becomes a group of one part per material. A mesh placed by
//!   several nodes (an instance) is one mesh in the document, placed by
//!   each.
//! - Y up, as ufbx converts any axis system the file declares; the file's
//!   unit (`UnitScaleFactor`: centimetres unless it says otherwise) is the
//!   source unit, scaled into metres by the pipeline like any other.
//! - Materials (`docs/03` §6.1 point 8): the classic Lambert and Phong by
//!   the documented mapping - diffuse colour times its factor as the base
//!   colour, `roughness = sqrt(2 / (exponent + 2))`, a Lambert as diffuse
//!   alone - and the physically based ones exactly where ufbx reads them:
//!   Autodesk Standard Surface and Arnold's, 3ds Max's Physical and PBR
//!   materials, Stingray's PBS, and Blender's export, whose Phong carries
//!   its Principled BSDF's roughness and metalness recoverably.
//! - Textures: a picture packed inside the file is written out beside the
//!   model, as a glTF's is; one named by path is looked for relative to the
//!   model, then at its absolute path, then by name beside the model.
//! - Each node's user properties, as attributes (`docs/03` §6.5).
//!
//! Left out, and said so: animation, skinning (a skinned mesh comes in at
//! its bind pose), shape keys, cameras, lights, and NURBS surfaces.

use crate::raw::{RawMesh, RawNode, RawScene, RawTexture};
use crate::ImportError;
use tequila_core::math::{vec2, vec3, vec4, Mat4, Transform};
use tequila_core::spectrum::Spectrum;
use tequila_core::units::LengthUnit;
use tequila_materials::graph::Value;
use tequila_materials::material_type::{MaterialType, Param};
use tequila_materials::openpbr::Material;
use tequila_scene::material::MaterialDefinition;
use tequila_texture::texture::MapRole;
use std::collections::HashMap;

/// Read an FBX file, binary or ASCII.
pub fn read(bytes: &[u8], name: &str) -> Result<RawScene, ImportError> {
    let options = ufbx::LoadOpts {
        // Y up whatever the file says; the unit is left as the file has it
        // and handed to the pipeline as the source unit.
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        space_conversion: ufbx::SpaceConversion::AdjustTransforms,
        // A geometric transform (3ds Max's object offset) moves a node's
        // mesh and not its children; baked into the mesh it needs nothing
        // the scene tree does not have.
        geometry_transform_handling: ufbx::GeometryTransformHandling::ModifyGeometry,
        // Maya's segment scale compensation, which a plain parent-child
        // product gets wrong.
        inherit_mode_handling: ufbx::InheritModeHandling::Compensate,
        // Blender writes its Principled BSDF as a Phong in a set way, and
        // ufbx can read it back as the roughness and metalness it was.
        use_blender_pbr_material: true,
        ignore_animation: true,
        load_external_files: false,
        file_format: ufbx::FileFormat::Fbx,
        ..Default::default()
    };
    let loaded = ufbx::load_memory(bytes, options).map_err(|e| {
        let what = e.description.to_string();
        match e.type_ {
            ufbx::ErrorType::UnsupportedVersion => ImportError::Unsupported(format!("this FBX's version is not one ufbx reads: {what}")),
            _ => ImportError::Malformed(format!("{what} ({})", e.info())),
        }
    })?;
    let fbx: &ufbx::Scene = &loaded;

    let mut reader = Reader { scene: RawScene::default(), meshes: HashMap::new(), materials: HashMap::new(), buffer: Vec::new() };

    // Which nodes have a mesh at or below them, so the rest can be left out.
    let mut keeps = vec![false; fbx.nodes.len()];
    let mut by_depth: Vec<&ufbx::Node> = fbx.nodes.iter().map(|n| &**n).collect();
    by_depth.sort_by_key(|n| std::cmp::Reverse(n.node_depth));
    for node in by_depth {
        let index = node.element.typed_id as usize;
        if node.mesh.is_some() {
            keeps[index] = true;
        }
        if keeps[index] {
            if let Some(parent) = &node.parent {
                keeps[parent.element.typed_id as usize] = true;
            }
        }
    }

    // Parents before children, depth first, in the file's order.
    let mut roots = Vec::new();
    let mut stack: Vec<(&ufbx::Node, Option<usize>)> = fbx.root_node.children.iter().rev().map(|c| (&**c, None)).collect();
    while let Some((node, parent)) = stack.pop() {
        if !keeps[node.element.typed_id as usize] {
            continue;
        }
        // A top node is placed in the world, the axis conversion with it.
        let matrix = matrix_of(if parent.is_none() { &node.node_to_world } else { &node.node_to_parent });
        let index = reader.node(node, parent, matrix, name)?;
        if parent.is_none() {
            roots.push(index);
        }
        for child in node.children.iter().rev() {
            stack.push((&**child, Some(index)));
        }
    }
    if reader.scene.meshes.is_empty() {
        return Err(ImportError::Malformed("the file holds no meshes".into()));
    }

    // Several top nodes: one group, so the import is one thing to move.
    if roots.len() > 1 {
        for node in reader.scene.nodes.iter_mut() {
            node.parent = Some(node.parent.map(|p| p + 1).unwrap_or(0));
        }
        reader.scene.nodes.insert(0, RawNode::group(name));
        roots = vec![0];
    }
    reader.scene.roots = roots;

    // What the file held that is not taken, said rather than dropped.
    let mut left = Vec::new();
    let count = |n: usize, one: &str, many: &str| match n {
        0 => None,
        1 => Some(format!("1 {one}")),
        n => Some(format!("{n} {many}")),
    };
    left.extend(count(fbx.cameras.len(), "camera", "cameras"));
    left.extend(count(fbx.lights.len(), "light", "lights"));
    left.extend(count(fbx.nurbs_surfaces.len(), "NURBS surface", "NURBS surfaces"));
    if !fbx.anim_curves.is_empty() {
        left.push("its animation".to_string());
    }
    if !fbx.skin_deformers.is_empty() {
        left.push("its skinning (skinned meshes are in their bind pose)".to_string());
    }
    if !fbx.blend_deformers.is_empty() {
        left.push("its shape keys".to_string());
    }
    if !left.is_empty() {
        reader.scene.warnings.push(format!("left out of the FBX: {}", left.join(", ")));
    }

    // The file's unit, which the pipeline scales into metres. A unit the
    // document has no name for is applied here instead, into metres.
    let unit = fbx.settings.unit_meters;
    let known = [
        (0.001, LengthUnit::Millimetre),
        (0.01, LengthUnit::Centimetre),
        (1.0, LengthUnit::Metre),
        (0.0254, LengthUnit::Inch),
        (0.3048, LengthUnit::Foot),
    ];
    reader.scene.source_unit = if let Some(&(_, unit)) = known.iter().find(|(m, _)| (unit / m - 1.0).abs() < 1e-4) {
        Some(unit)
    } else if unit.is_finite() && unit > 0.0 {
        crate::rescale(&mut reader.scene, unit);
        Some(LengthUnit::Metre)
    } else {
        None
    };
    Ok(reader.scene)
}

/// ufbx's affine matrix, columns named `m<row><column>`, as a `Mat4`.
fn matrix_of(m: &ufbx::Matrix) -> Mat4 {
    let mut out = Mat4::IDENTITY;
    out.m[0] = [m.m00 as f32, m.m01 as f32, m.m02 as f32, m.m03 as f32];
    out.m[1] = [m.m10 as f32, m.m11 as f32, m.m12 as f32, m.m13 as f32];
    out.m[2] = [m.m20 as f32, m.m21 as f32, m.m22 as f32, m.m23 as f32];
    out
}

fn is_identity(m: &Mat4) -> bool {
    (0..4).all(|r| (0..4).all(|c| (m.m[r][c] - if r == c { 1.0 } else { 0.0 }).abs() < 1e-6))
}

struct Reader {
    scene: RawScene,
    /// (mesh, material slot, material) to the scene's mesh: an instance
    /// with the same material shares it.
    meshes: HashMap<(u32, u32, Option<u32>), usize>,
    /// ufbx's material to the scene's.
    materials: HashMap<u32, usize>,
    /// Room for one polygon's triangles.
    buffer: Vec<u32>,
}

impl Reader {
    /// The node, and the parts its mesh adds: itself when the mesh has one
    /// material, a part per material under it when it has several.
    fn node(&mut self, node: &ufbx::Node, parent: Option<usize>, matrix: Mat4, file: &str) -> Result<usize, ImportError> {
        let name = match node.element.name.trim() {
            "" => format!("{file} {}", node.element.typed_id),
            name => name.to_string(),
        };
        let transform = Transform::from_matrix(matrix).unwrap_or_else(|| {
            self.scene.warnings.push(format!("`{name}` is placed by a transform that flattens it; it was placed without one"));
            Transform::IDENTITY
        });
        let mut raw = RawNode { parent, transform, hidden: !node.visible, ..RawNode::group(name.clone()) };
        for prop in node.element.props.props.iter() {
            if prop.flags.has_any(ufbx::PropFlags::USER_DEFINED) {
                if let Some(value) = prop_text(prop) {
                    raw.metadata.insert(prop.name.to_string(), value);
                }
            }
        }
        self.scene.nodes.push(raw);
        let index = self.scene.nodes.len() - 1;

        let Some(mesh) = &node.mesh else {
            return Ok(index);
        };
        let parts = self.parts(node, mesh);
        let geometry = matrix_of(&node.geometry_to_node);
        if parts.len() == 1 && is_identity(&geometry) {
            self.scene.nodes[index].mesh = Some(parts[0].0);
        } else {
            let placed = Transform::from_matrix(geometry).unwrap_or(Transform::IDENTITY);
            for (mesh, label) in parts {
                let child = RawNode { parent: Some(index), transform: placed, ..RawNode::part(if label.is_empty() { name.clone() } else { format!("{name} ({label})") }, mesh) };
                self.scene.nodes.push(child);
            }
        }
        Ok(index)
    }

    /// A mesh's triangles as the scene's meshes, one per material, each
    /// with what to call it.
    fn parts(&mut self, node: &ufbx::Node, mesh: &ufbx::Mesh) -> Vec<(usize, String)> {
        let mut out = Vec::new();
        let all: Vec<u32>;
        let parts: Vec<(u32, &[u32])> = if mesh.material_parts.is_empty() {
            all = (0..mesh.num_faces as u32).collect();
            vec![(u32::MAX, &all[..])]
        } else {
            mesh.material_parts.iter().filter(|p| p.num_triangles > 0).map(|p| (p.index, &*p.face_indices)).collect()
        };
        let several = parts.len() > 1;
        for (slot, faces) in parts {
            // A node's own materials win over its mesh's: the same mesh
            // placed twice can wear different ones.
            let material = (slot != u32::MAX)
                .then(|| node.materials.get(slot as usize).or_else(|| mesh.materials.get(slot as usize)))
                .flatten();
            let material_id = material.map(|m| m.element.typed_id);
            let key = (mesh.element.typed_id, slot, material_id);
            let index = match self.meshes.get(&key) {
                Some(&index) => index,
                None => {
                    let mut raw = self.triangles(mesh, faces);
                    raw.material = material.map(|m| self.material(m));
                    raw.name = mesh.element.name.to_string();
                    self.scene.meshes.push(raw);
                    self.meshes.insert(key, self.scene.meshes.len() - 1);
                    self.scene.meshes.len() - 1
                }
            };
            let label = match material {
                Some(m) if several => m.element.name.to_string(),
                _ => String::new(),
            };
            out.push((index, label));
        }
        out
    }

    /// Some of a mesh's faces as triangles. A vertex is a corner's
    /// position, normal, coordinate and colour together, made once.
    fn triangles(&mut self, mesh: &ufbx::Mesh, faces: &[u32]) -> RawMesh {
        let mut out = RawMesh::default();
        let mut made: HashMap<[u32; 4], u32> = HashMap::new();
        let index_of = |attribute_exists: bool, indices: &ufbx::List<u32>, corner: usize| if attribute_exists { indices.get(corner).copied().unwrap_or(u32::MAX) } else { u32::MAX };
        let normals = mesh.vertex_normal.exists;
        let uvs = mesh.vertex_uv.exists;
        let colours = mesh.vertex_color.exists;
        self.buffer.resize(mesh.max_face_triangles.max(1) * 3, 0);
        for &f in faces {
            let Some(&face) = mesh.faces.get(f as usize) else { continue };
            if face.num_indices < 3 {
                continue;
            }
            let n = mesh.triangulate_face(&mut self.buffer, face) as usize;
            for &corner in &self.buffer[..n * 3] {
                let corner = corner as usize;
                let key = [
                    index_of(true, &mesh.vertex_position.indices, corner),
                    index_of(normals, &mesh.vertex_normal.indices, corner),
                    index_of(uvs, &mesh.vertex_uv.indices, corner),
                    index_of(colours, &mesh.vertex_color.indices, corner),
                ];
                if key[0] as usize >= mesh.vertex_position.values.len() {
                    continue;
                }
                let vertex = *made.entry(key).or_insert_with(|| {
                    let p = mesh.vertex_position.values[key[0] as usize];
                    out.positions.push(vec3(p.x as f32, p.y as f32, p.z as f32));
                    if normals {
                        let n = mesh.vertex_normal.values.get(key[1] as usize).map_or(vec3(0.0, 1.0, 0.0), |n| vec3(n.x as f32, n.y as f32, n.z as f32));
                        out.normals.push(n);
                    }
                    if uvs {
                        // FBX's coordinates start at the bottom left, as
                        // OBJ's do; the renderer's at the top left.
                        let t = mesh.vertex_uv.values.get(key[2] as usize).map_or(vec2(0.0, 0.0), |t| vec2(t.x as f32, 1.0 - t.y as f32));
                        out.uvs.push(t);
                    }
                    if colours {
                        let f = crate::ply::srgb_to_linear;
                        let c = mesh.vertex_color.values.get(key[3] as usize).map_or(vec4(1.0, 1.0, 1.0, 1.0), |c| vec4(f(c.x as f32), f(c.y as f32), f(c.z as f32), c.w.clamp(0.0, 1.0) as f32));
                        out.colors.push(c);
                    }
                    (out.positions.len() - 1) as u32
                });
                out.indices.push(vertex);
            }
        }
        // A layer of white is what several exporters write when there is no
        // painting at all.
        if out.colors.iter().all(|c| c.x > 0.999 && c.y > 0.999 && c.z > 0.999) {
            out.colors.clear();
        }
        out
    }

    fn material(&mut self, material: &ufbx::Material) -> usize {
        if let Some(&index) = self.materials.get(&material.element.typed_id) {
            return index;
        }
        let definition = convert_material(material, &mut self.scene.textures, &mut self.scene.warnings);
        self.scene.materials.push(definition);
        let index = self.scene.materials.len() - 1;
        self.materials.insert(material.element.typed_id, index);
        index
    }
}

/// A user property as text, for the node's attributes.
fn prop_text(prop: &ufbx::Prop) -> Option<String> {
    use ufbx::PropType;
    let v = &prop.value_vec4;
    Some(match prop.type_ {
        PropType::String => prop.value_str.to_string(),
        PropType::Boolean => (prop.value_int != 0).to_string(),
        PropType::Integer => prop.value_int.to_string(),
        PropType::Number | PropType::Distance => format!("{}", v.x),
        PropType::Vector | PropType::Color | PropType::Translation | PropType::Rotation | PropType::Scaling => format!("{}, {}, {}", v.x, v.y, v.z),
        PropType::ColorWithAlpha => format!("{}, {}, {}, {}", v.x, v.y, v.z, v.w),
        _ if !prop.value_str.is_empty() => prop.value_str.to_string(),
        _ => return None,
    })
}

/// Whether ufbx reads a material's physically based parameters straight
/// from the file, rather than deriving them from a classic one.
fn physically_based(shader: ufbx::ShaderType) -> bool {
    use ufbx::ShaderType::*;
    matches!(
        shader,
        OslStandardSurface | ArnoldStandardSurface | E3DsMaxPhysicalMaterial | E3DsMaxPbrMetalRough | E3DsMaxPbrSpecGloss | GltfMaterial | OpenpbrMaterial | ShaderfxGraph | BlenderPhong
    )
}

fn value(map: &ufbx::MaterialMap, default: f64) -> f32 {
    if map.has_value { map.value_vec4.x as f32 } else { default as f32 }
}

/// A colour property as stated: linear, with sRGB's primaries, as Blender,
/// Maya and 3ds Max write them and as OBJ's are read.
fn colour(map: &ufbx::MaterialMap, default: f64) -> Spectrum {
    let v = &map.value_vec4;
    if map.has_value {
        Spectrum::from_linear_srgb(vec3(v.x as f32, v.y as f32, v.z as f32))
    } else {
        Spectrum::splat(default as f32)
    }
}

/// An FBX material as an OpenPBR one (`docs/03` §6.1 point 8).
pub(crate) fn convert_material(material: &ufbx::Material, textures: &mut Vec<RawTexture>, warnings: &mut Vec<String>) -> MaterialDefinition {
    let name = match material.element.name.trim() {
        "" => format!("Material {}", material.element.typed_id + 1),
        name => name.to_string(),
    };
    if physically_based(material.shader_type) {
        physical(&name, material, textures)
    } else {
        classic(&name, material, textures, warnings)
    }
}

/// Lambert and Phong, by the mapping §6.1 documents:
///
/// - base colour: `DiffuseColor × DiffuseFactor`;
/// - roughness: `sqrt(2 / (ShininessExponent + 2))`, the inversion of the
///   Blinn-Phong exponent the OBJ reader uses for `Ns`; a Lambert has no
///   highlight and is diffuse alone;
/// - no highlight either where `SpecularColor × SpecularFactor` is black;
/// - emission: `EmissiveColor`, as bright as `EmissiveFactor`;
/// - opacity: the file's `Opacity` where it writes one, else one less
///   `TransparentColor × TransparencyFactor`, as the FBX SDK defines it.
///
/// Maps: the diffuse colour's, the normal map, a bump map (a normal map
/// when its name says so, since exporters put them there), the specular
/// colour's and the emission's. Others are named in a warning.
fn classic(name: &str, material: &ufbx::Material, textures: &mut Vec<RawTexture>, warnings: &mut Vec<String>) -> MaterialDefinition {
    let maps = &material.fbx;
    let lambert = material.shader_type == ufbx::ShaderType::FbxLambert;
    let base = colour(&maps.diffuse_color, 0.8) * value(&maps.diffuse_factor, 1.0).max(0.0);
    let exponent = value(&maps.specular_exponent, 20.0).max(0.0);
    let roughness = (2.0 / (exponent + 2.0)).sqrt().clamp(0.0, 1.0);
    let specular = colour(&maps.specular_color, 0.2) * value(&maps.specular_factor, 1.0).max(0.0);
    let emission = colour(&maps.emission_color, 0.0);
    let emission_strength = value(&maps.emission_factor, 1.0).max(0.0);
    let opacity = match ufbx::find_prop(&material.element.props, "Opacity") {
        Some(p) => p.value_vec4.x as f32,
        None => {
            let t = colour(&maps.transparency_color, 0.0).to_linear_srgb() * value(&maps.transparency_factor, 0.0);
            1.0 - (t.x + t.y + t.z) / 3.0
        }
    }
    .clamp(0.0, 1.0);

    let glows = !emission.is_black() && emission_strength > 0.0;
    let mut m = if lambert { Material::diffuse(base) } else { Material::plastic(base, roughness) };
    if !lambert && specular.is_black() {
        m.specular_weight = 0.0;
    }
    m.emission_color = if glows { emission } else { Spectrum::ZERO };
    m.emission_luminance = if glows { emission_strength } else { 0.0 };
    m.geometry_opacity = if opacity > 0.999 { 1.0 } else { opacity };
    let kind = if lambert { MaterialType::Diffuse } else { MaterialType::Plastic };
    let mut definition = MaterialDefinition::new(name, kind, m);

    if let Some(t) = texture(&maps.diffuse_color, MapRole::Color) {
        crate::obj::attach_texture(&mut definition, textures, t, Param::BaseColor, Value::Color(base));
    }
    if let Some(t) = texture(&maps.normal_map, MapRole::Normal) {
        crate::obj::attach_texture(&mut definition, textures, t, Param::Bump, Value::Float(0.0));
    } else if let Some(mut t) = texture(&maps.bump, MapRole::Height) {
        let lower = t.uri.to_lowercase();
        if ["normal", "_nrm", "_nor.", "_n."].iter().any(|s| lower.contains(s)) {
            t.role = MapRole::Normal;
        }
        crate::obj::attach_texture(&mut definition, textures, t, Param::Bump, Value::Float(0.0));
    }
    if let Some(t) = texture(&maps.specular_color, MapRole::Color) {
        crate::obj::attach_texture(&mut definition, textures, t, Param::SpecularColor, Value::Color(Spectrum::ONE));
    }
    if let Some(t) = texture(&maps.emission_color, MapRole::Color) {
        crate::obj::attach_texture(&mut definition, textures, t, Param::EmissionColor, Value::Color(emission));
    }
    let unread: Vec<&str> = [
        (&maps.transparency_color, "transparency"),
        (&maps.transparency_factor, "transparency"),
        (&maps.specular_exponent, "shininess"),
        (&maps.reflection_color, "reflection"),
        (&maps.displacement, "displacement"),
    ]
    .iter()
    .filter(|(map, _)| texture(map, MapRole::Data).is_some())
    .map(|(_, what)| *what)
    .collect();
    if !unread.is_empty() {
        let mut unread = unread;
        unread.dedup();
        warnings.push(format!(
            "`{name}` has {} map{} that {} not read: only the diffuse, normal, bump, specular colour and emission maps of a Lambert or Phong are",
            unread.join(" and "),
            if unread.len() == 1 { "" } else { "s" },
            if unread.len() == 1 { "is" } else { "are" }
        ));
    }
    definition
}

/// The physically based materials, whose parameters ufbx gathers into one
/// set from whichever shader wrote them. Mapped one to one onto OpenPBR's,
/// which is the model Autodesk Standard Surface and Arnold's were the
/// ancestors of; a glossiness is turned back into roughness.
fn physical(name: &str, material: &ufbx::Material, textures: &mut Vec<RawTexture>) -> MaterialDefinition {
    let p = &material.pbr;
    let features = &material.features;
    let glossy = features.roughness_as_glossiness.enabled;
    let base = colour(&p.base_color, 0.8);
    let metalness = value(&p.metalness, 0.0).clamp(0.0, 1.0);
    let mut roughness = value(&p.roughness, 0.5).clamp(0.0, 1.0);
    if glossy {
        roughness = 1.0 - roughness;
    }
    let mut m = Material {
        base_weight: value(&p.base_factor, 1.0).clamp(0.0, 1.0),
        base_color: base,
        base_metalness: metalness,
        metal_ior: tequila_materials::fresnel::metals::ALUMINIUM,
        specular_weight: value(&p.specular_factor, 1.0).clamp(0.0, 1.0),
        specular_color: colour(&p.specular_color, 1.0),
        specular_ior: value(&p.specular_ior, 1.5).clamp(1.0, 3.0),
        specular_roughness_anisotropy: value(&p.specular_anisotropy, 0.0).clamp(0.0, 1.0),
        transmission_weight: value(&p.transmission_factor, 0.0).clamp(0.0, 1.0),
        transmission_color: colour(&p.transmission_color, 1.0),
        coat_weight: value(&p.coat_factor, 0.0).clamp(0.0, 1.0),
        coat_color: colour(&p.coat_color, 1.0),
        coat_ior: value(&p.coat_ior, 1.5).clamp(1.0, 3.0),
        fuzz_weight: value(&p.sheen_factor, 0.0).clamp(0.0, 1.0),
        fuzz_color: colour(&p.sheen_color, 1.0),
        fuzz_roughness: value(&p.sheen_roughness, 0.3).clamp(0.0, 1.0),
        thin_film_weight: value(&p.thin_film_factor, 0.0).clamp(0.0, 1.0),
        subsurface_weight: value(&p.subsurface_factor, 0.0).clamp(0.0, 1.0),
        subsurface_color: colour(&p.subsurface_color, 1.0),
        geometry_thin_walled: features.thin_walled.enabled,
        ..Material::plastic(base, roughness)
    };
    m.specular_roughness = roughness;
    let mut coat_roughness = value(&p.coat_roughness, 0.1).clamp(0.0, 1.0);
    if features.coat_roughness_as_glossiness.enabled {
        coat_roughness = 1.0 - coat_roughness;
    }
    m.coat_roughness = coat_roughness;
    if p.thin_film_thickness.has_value {
        m.thin_film_thickness = value(&p.thin_film_thickness, 400.0).clamp(0.0, 2000.0);
    }
    if p.thin_film_ior.has_value {
        m.thin_film_ior = value(&p.thin_film_ior, 1.5).clamp(1.0, 3.0);
    }
    let emission = colour(&p.emission_color, 0.0);
    let strength = value(&p.emission_factor, 1.0).max(0.0);
    if !emission.is_black() && strength > 0.0 {
        m.emission_color = emission;
        m.emission_luminance = strength;
    }
    let opacity = value(&p.opacity, 1.0).clamp(0.0, 1.0);
    m.geometry_opacity = if opacity > 0.999 { 1.0 } else { opacity };
    let kind = if metalness >= 0.5 { MaterialType::Metal } else { MaterialType::Plastic };
    let mut definition = MaterialDefinition::new(name, kind, m);

    if let Some(t) = texture(&p.base_color, MapRole::Color) {
        crate::obj::attach_texture(&mut definition, textures, t, Param::BaseColor, Value::Color(base));
    }
    if let Some(t) = texture(&p.roughness, MapRole::Data) {
        let node = crate::obj::attach_texture(&mut definition, textures, t, Param::SpecularRoughness, Value::Float(if glossy { 1.0 - roughness } else { roughness }));
        if glossy {
            use tequila_materials::graph::{Node, NodeKind, Socket};
            let graph = &mut definition.graph;
            let invert = graph.add_node(Node::new(NodeKind::Invert));
            let output = graph.output();
            graph.connect(node, invert, Socket::Input(0));
            graph.connect(invert, output, Socket::Param(Param::SpecularRoughness));
        }
    }
    if let Some(t) = texture(&p.metalness, MapRole::Data) {
        crate::obj::attach_texture(&mut definition, textures, t, Param::BaseMetalness, Value::Float(metalness));
    }
    if let Some(t) = texture(&p.normal_map, MapRole::Normal) {
        crate::obj::attach_texture(&mut definition, textures, t, Param::Bump, Value::Float(0.0));
    }
    if let Some(t) = texture(&p.emission_color, MapRole::Color) {
        crate::obj::attach_texture(&mut definition, textures, t, Param::EmissionColor, Value::Color(emission));
    }
    if let Some(t) = texture(&p.opacity, MapRole::Data) {
        crate::obj::attach_texture(&mut definition, textures, t, Param::Opacity, Value::Float(opacity));
    }
    definition
}

/// The picture a material map is driven by, if any: packed inside the file,
/// or named by path.
fn texture(map: &ufbx::MaterialMap, role: MapRole) -> Option<RawTexture> {
    if map.feature_disabled {
        return None;
    }
    let texture = map.texture.as_ref()?;
    // A layered or procedural texture stands on file textures; the first of
    // them is the picture.
    let file: &ufbx::Texture = if texture.type_ == ufbx::TextureType::File { texture } else { &**texture.file_textures.first()? };
    let relative = file.relative_filename.replace('\\', "/");
    let absolute = file.absolute_filename.to_string();
    let named = [relative.as_str(), &*file.filename, absolute.as_str()].into_iter().find(|s| !s.is_empty()).unwrap_or("").to_string();
    if !file.content.is_empty() {
        // Packed: a plain name to write it out under, beside the model.
        let stem = named.rsplit(['/', '\\']).next().unwrap_or("");
        let mut safe: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' }).collect();
        if safe.trim_matches(['_', '.']).is_empty() {
            let content: &[u8] = &file.content;
            let extension = if content.starts_with(&[0x89, b'P', b'N', b'G']) { "png" } else if content.starts_with(&[0xFF, 0xD8, 0xFF]) { "jpg" } else { "bin" };
            safe = format!("texture_{}.{extension}", file.element.typed_id);
        }
        return Some(RawTexture { uri: safe, role, embedded: Some(std::sync::Arc::new(file.content.to_vec())), elsewhere: None });
    }
    if named.is_empty() {
        return None;
    }
    // Relative first: it is what survives the model and its pictures being
    // moved together. The absolute path is tried when it does not resolve.
    let (uri, elsewhere) = if !relative.is_empty() {
        (relative.clone(), (!absolute.is_empty() && absolute != relative).then_some(absolute))
    } else {
        (named, None)
    };
    Some(RawTexture { uri, role, embedded: None, elsewhere })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_broken_file_is_an_error_not_a_panic() {
        assert!(read(b"", "x").is_err());
        assert!(read(b"Kaydara FBX Binary  \x00\x1a\x00", "x").is_err());
        assert!(read(b"; FBX 7.4.0 project file\nnothing at all", "x").is_err());
    }

    /// The §6.1 mapping, on a Phong written as text.
    #[test]
    fn a_phong_maps_by_the_documented_formula() {
        let text = phong_cube("Phong", 98.0, [0.5, 0.25, 0.125], 1.0);
        let scene = read(text.as_bytes(), "cube").unwrap();
        let material = scene.materials[0].material();
        // sqrt(2 / 100)
        assert!((material.specular_roughness - 0.1414).abs() < 1e-3, "{}", material.specular_roughness);
        let base = material.base_color.to_linear_srgb();
        assert!((base.x - 0.5).abs() < 1e-3 && (base.y - 0.25).abs() < 1e-3 && (base.z - 0.125).abs() < 1e-3, "{base:?}");
        assert_eq!(scene.materials[0].material_type, MaterialType::Plastic);

        let text = phong_cube("Lambert", 0.0, [0.5, 0.5, 0.5], 0.5);
        let scene = read(text.as_bytes(), "cube").unwrap();
        assert_eq!(scene.materials[0].material_type, MaterialType::Diffuse);
        let base = scene.materials[0].material().base_color.to_linear_srgb();
        assert!((base.x - 0.25).abs() < 1e-3, "the diffuse factor scales the colour: {base:?}");
    }

    /// A one-cube ASCII FBX with one material, in centimetres, Y up.
    pub(crate) fn phong_cube(model: &str, exponent: f64, diffuse: [f64; 3], factor: f64) -> String {
        let [r, g, b] = diffuse;
        format!(
            r#"; FBX 7.4.0 project file
FBXHeaderExtension:  {{
	FBXHeaderVersion: 1003
	FBXVersion: 7400
	Creator: "Tequila test"
}}
GlobalSettings:  {{
	Version: 1000
	Properties70:  {{
		P: "UpAxis", "int", "Integer", "",1
		P: "UpAxisSign", "int", "Integer", "",1
		P: "FrontAxis", "int", "Integer", "",2
		P: "FrontAxisSign", "int", "Integer", "",1
		P: "CoordAxis", "int", "Integer", "",0
		P: "CoordAxisSign", "int", "Integer", "",1
		P: "UnitScaleFactor", "double", "Number", "",1
	}}
}}
Objects:  {{
	Geometry: 10, "Geometry::Cube", "Mesh" {{
		Vertices: *24 {{
			a: -1,-1,-1,1,-1,-1,1,1,-1,-1,1,-1,-1,-1,1,1,-1,1,1,1,1,-1,1,1
		}}
		PolygonVertexIndex: *24 {{
			a: 0,3,2,-2,4,5,6,-8,0,1,5,-5,2,3,7,-7,1,2,6,-6,3,0,4,-8
		}}
		GeometryVersion: 124
		LayerElementMaterial: 0 {{
			Version: 101
			Name: ""
			MappingInformationType: "AllSame"
			ReferenceInformationType: "IndexToDirect"
			Materials: *1 {{
				a: 0
			}}
		}}
		Layer: 0 {{
			Version: 100
			LayerElement:  {{
				Type: "LayerElementMaterial"
				TypedIndex: 0
			}}
		}}
	}}
	Model: 20, "Model::Cube", "Mesh" {{
		Version: 232
		Properties70:  {{
			P: "Lcl Translation", "Lcl Translation", "", "A",0,1,0
		}}
	}}
	Material: 30, "Material::Paint", "" {{
		Version: 102
		ShadingModel: "{model}"
		MultiLayer: 0
		Properties70:  {{
			P: "DiffuseColor", "Color", "", "A",{r},{g},{b}
			P: "DiffuseFactor", "Number", "", "A",{factor}
			P: "SpecularColor", "Color", "", "A",1,1,1
			P: "ShininessExponent", "Number", "", "A",{exponent}
		}}
	}}
}}
Connections:  {{
	C: "OO",20,0
	C: "OO",10,20
	C: "OO",30,20
}}
"#
        )
    }
}
