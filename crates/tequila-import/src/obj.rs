//! Wavefront OBJ, with its companion MTL.
//!
//! OBJ's awkward parts, all of which show up in real exports:
//!
//! - Indices are 1-based and may be **negative**, counting back from the end
//!   of what has been read so far.
//! - Position, normal and UV each have their own index, so a "vertex" is a
//!   triple. The renderer needs one flat vertex buffer, so each distinct
//!   triple becomes one vertex.
//! - Faces are arbitrary polygons and need triangulating (§6.1 point 1).
//! - `vt` has its origin at the bottom left, where glTF and this renderer put
//!   it at the top left, so v is flipped on the way in.
//! - `g`/`o` groups and `usemtl` cut across each other; a change of either
//!   starts a new part.

use crate::raw::{RawMesh, RawNode, RawScene};
use crate::ImportError;
use tequila_core::math::{vec2, vec3, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_render_core::material::Material;
use tequila_scene::material::MaterialDefinition;
use std::collections::BTreeMap;

/// Read an OBJ. `mtl` supplies the contents of any referenced `.mtl` files,
/// keyed by the name in the `mtllib` line — resolving those is the caller's
/// job, because it needs the filesystem and this does not.
pub fn read(
    bytes: &[u8],
    name: &str,
    mtl: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<RawScene, ImportError> {
    // Lossy: an OBJ written in a Windows code page, with an accented
    // material or group name, is still an OBJ, and its geometry is ASCII.
    let text = String::from_utf8_lossy(bytes);

    let mut positions: Vec<Vec3> = Vec::new();
    let mut vertex_colors: Vec<Option<tequila_core::math::Vec4>> = Vec::new();
    let mut normals: Vec<Vec3> = Vec::new();
    let mut uvs: Vec<Vec2> = Vec::new();

    let mut scene = RawScene::default();
    let mut warnings = Vec::new();
    let mut material_indices: BTreeMap<String, usize> = BTreeMap::new();

    // The part currently being accumulated, and the (group, material) it
    // belongs to. A change of either closes it.
    let mut builder = PartBuilder::new(name.to_string(), None);
    let mut group_name = name.to_string();
    let mut current_material: Option<usize> = None;

    for (number, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let keyword = words.next().unwrap_or("");

        match keyword {
            "v" => {
                let values = read_floats(&mut words, number)?;
                if values.len() < 3 {
                    return Err(ImportError::Malformed(format!(
                        "line {}: a vertex needs three coordinates",
                        number + 1
                    )));
                }
                positions.push(vec3(values[0], values[1], values[2]));
                // The common extension — MeshLab's, ZBrush's — puts a colour
                // after the position, as display values from 0 to 1.
                vertex_colors.push((values.len() >= 6).then(|| {
                    let linear = |v: f32| {
                        let v = v.clamp(0.0, 1.0);
                        if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
                    };
                    tequila_core::math::vec4(linear(values[3]), linear(values[4]), linear(values[5]), 1.0)
                }));
            }
            "vn" => normals.push(read_vec3(&mut words, number)?),
            "vt" => {
                let values: Vec<f32> = read_floats(&mut words, number)?;
                // A `vt` may carry a third (w) coordinate, which is unused.
                if values.len() < 2 {
                    return Err(ImportError::Malformed(format!(
                        "line {}: a texture coordinate needs at least two values",
                        number + 1
                    )));
                }
                // OBJ puts v=0 at the *bottom* of the image; glTF, and this
                // renderer with it, put it at the top. The two conventions
                // disagree by a flip, and taking `vt` verbatim imports every
                // textured OBJ vertically mirrored — which on a checker is
                // obvious and on a wood grain or a logo is just subtly wrong.
                //
                // Converted here, at the one place OBJ coordinates enter, so
                // that nothing downstream has to know which format a mesh came
                // from.
                uvs.push(vec2(values[0], 1.0 - values[1]));
            }
            "f" => {
                let corners: Vec<&str> = words.collect();
                if corners.len() < 3 {
                    return Err(ImportError::Malformed(format!(
                        "line {}: a face needs at least three corners",
                        number + 1
                    )));
                }
                let resolved: Result<Vec<Corner>, ImportError> = corners
                    .iter()
                    .map(|c| Corner::parse(c, positions.len(), uvs.len(), normals.len(), number))
                    .collect();
                let resolved = resolved?;

                // Fan triangulation. Correct for convex polygons, which is what
                // OBJ faces are in every exporter that writes them; a concave
                // face would need ear clipping and is vanishingly rare here.
                for i in 1..resolved.len() - 1 {
                    for corner in [resolved[0], resolved[i], resolved[i + 1]] {
                        builder.push(corner, &positions, &uvs, &normals, &vertex_colors);
                    }
                }
            }
            "g" | "o" => {
                let new_name = words.collect::<Vec<_>>().join(" ");
                let new_name = if new_name.is_empty() {
                    name.to_string()
                } else {
                    new_name
                };
                if new_name != group_name {
                    builder.finish_into(&mut scene);
                    group_name = new_name.clone();
                    builder = PartBuilder::new(new_name, current_material);
                }
            }
            "usemtl" => {
                let material_name = words.collect::<Vec<_>>().join(" ");
                let index = material_indices.get(&material_name).copied();
                if index.is_none() && !material_name.is_empty() {
                    warnings.push(format!(
                        "line {}: material `{material_name}` was used but never defined",
                        number + 1
                    ));
                }
                if index != current_material {
                    builder.finish_into(&mut scene);
                    current_material = index;
                    builder = PartBuilder::new(group_name.clone(), current_material);
                }
            }
            "mtllib" => {
                // The rest of the line as one name first: Blender writes
                // `mtllib Office Chair.mtl` for a model saved as "Office
                // Chair". The format's own reading, a file per word, is the
                // fallback.
                let whole = line["mtllib".len()..].trim();
                let libraries: Vec<(&str, Option<Vec<u8>>)> = match mtl(whole) {
                    Some(bytes) => vec![(whole, Some(bytes))],
                    None => words.map(|word| (word, mtl(word))).collect(),
                };
                for (library, bytes) in libraries {
                    let before = scene.textures.len();
                    match bytes {
                        Some(bytes) => match read_mtl(
                            &bytes,
                            &mut scene.textures,
                            &mut warnings,
                        ) {
                            Ok(materials) => {
                                // A map named in an MTL is beside the MTL, which
                                // need not be beside the OBJ.
                                if let Some(folder) = library.rfind(['/', '\\']).map(|at| &library[..=at]) {
                                    for texture in &mut scene.textures[before..] {
                                        if !is_absolute(&texture.uri) {
                                            texture.uri = format!("{folder}{}", texture.uri);
                                        }
                                    }
                                }
                                for (material_name, definition) in materials {
                                    let index = scene.materials.len();
                                    material_indices.entry(material_name).or_insert_with(|| {
                                        scene.materials.push(definition);
                                        index
                                    });
                                }
                            }
                            Err(error) => warnings.push(format!("`{library}`: {error}")),
                        },
                        // Not fatal: a missing MTL costs colours, not geometry,
                        // and refusing the whole import over it would be worse
                        // than importing grey.
                        None => warnings.push(format!("`{library}` could not be read")),
                    }
                }
            }
            _ => {}
        }
    }
    builder.finish_into(&mut scene);

    if scene.meshes.is_empty() {
        return Err(ImportError::Malformed("the file contains no faces".into()));
    }

    // One node per part, all under a group named for the file when there is
    // more than one — a flat list of parts at the document root would bury
    // everything else in the scene.
    if scene.meshes.len() == 1 {
        scene.nodes = vec![RawNode::part(scene.meshes[0].name.clone(), 0)];
        scene.roots = vec![0];
    } else {
        scene.nodes = vec![RawNode::group(name)];
        for (i, mesh) in scene.meshes.iter().enumerate() {
            let mut node = RawNode::part(mesh.name.clone(), i);
            node.parent = Some(0);
            scene.nodes.push(node);
        }
        scene.roots = vec![0];
    }

    scene.warnings = warnings;
    // OBJ carries no units (§3.4); the caller guesses from the bounding box.
    scene.source_unit = None;
    Ok(scene)
}

/// A path from another machine, or rooted: not relative to anything here.
fn is_absolute(path: &str) -> bool {
    path.starts_with(['/', '\\']) || path.as_bytes().get(1) == Some(&b':')
}

/// One `(position, uv, normal)` triple from a face corner, already resolved to
/// zero-based indices.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Corner {
    position: usize,
    uv: Option<usize>,
    normal: Option<usize>,
}

impl Corner {
    fn parse(
        text: &str,
        positions: usize,
        uvs: usize,
        normals: usize,
        line: usize,
    ) -> Result<Corner, ImportError> {
        let mut parts = text.split('/');
        let position =
            resolve(parts.next().unwrap_or(""), positions, line, "vertex")?.ok_or_else(|| {
                ImportError::Malformed(format!("line {}: a face corner needs a vertex", line + 1))
            })?;
        let uv = resolve(parts.next().unwrap_or(""), uvs, line, "texture coordinate")?;
        let normal = resolve(parts.next().unwrap_or(""), normals, line, "normal")?;
        Ok(Corner {
            position,
            uv,
            normal,
        })
    }
}

/// OBJ indices are 1-based, and negative ones count back from the end of what
/// has been read so far. Both are resolved here so nothing downstream has to
/// know.
fn resolve(
    text: &str,
    available: usize,
    line: usize,
    what: &str,
) -> Result<Option<usize>, ImportError> {
    if text.is_empty() {
        return Ok(None);
    }
    let raw: i64 = text.parse().map_err(|_| {
        ImportError::Malformed(format!("line {}: `{text}` is not an index", line + 1))
    })?;
    let zero_based = if raw > 0 {
        raw - 1
    } else if raw < 0 {
        available as i64 + raw
    } else {
        return Err(ImportError::Malformed(format!(
            "line {}: index 0 is not valid in OBJ, which counts from 1",
            line + 1
        )));
    };
    if zero_based < 0 || zero_based >= available as i64 {
        return Err(ImportError::Malformed(format!(
            "line {}: {what} index {raw} is out of range (the file has {available} so far)",
            line + 1
        )));
    }
    Ok(Some(zero_based as usize))
}

/// Accumulates one part, deduplicating corners into a flat vertex buffer.
struct PartBuilder {
    name: String,
    material: Option<usize>,
    mesh: RawMesh,
    seen: BTreeMap<Corner, u32>,
    /// Whether every corner so far carried a normal. A part where only some
    /// faces have them cannot present a normal channel, so the pipeline
    /// regenerates the lot rather than mixing authored and generated normals.
    all_have_normals: bool,
    all_have_uvs: bool,
    all_have_colors: bool,
}

impl PartBuilder {
    fn new(name: String, material: Option<usize>) -> PartBuilder {
        PartBuilder {
            name,
            material,
            mesh: RawMesh::default(),
            seen: BTreeMap::new(),
            all_have_normals: true,
            all_have_uvs: true,
            all_have_colors: true,
        }
    }

    fn push(
        &mut self,
        corner: Corner,
        positions: &[Vec3],
        uvs: &[Vec2],
        normals: &[Vec3],
        colors: &[Option<tequila_core::math::Vec4>],
    ) {
        self.all_have_normals &= corner.normal.is_some();
        self.all_have_uvs &= corner.uv.is_some();
        let color = colors.get(corner.position).copied().flatten();
        self.all_have_colors &= color.is_some();

        let next = self.mesh.positions.len() as u32;
        let index = *self.seen.entry(corner).or_insert(next);
        if index == next {
            self.mesh.positions.push(positions[corner.position]);
            self.mesh
                .normals
                .push(corner.normal.map(|n| normals[n]).unwrap_or(Vec3::Y));
            self.mesh
                .uvs
                .push(corner.uv.map(|t| uvs[t]).unwrap_or(Vec2::ZERO));
            self.mesh
                .colors
                .push(color.unwrap_or(tequila_core::math::vec4(1.0, 1.0, 1.0, 1.0)));
        }
        self.mesh.indices.push(index);
    }

    fn finish_into(mut self, scene: &mut RawScene) {
        if self.mesh.indices.is_empty() {
            return;
        }
        if !self.all_have_normals {
            self.mesh.normals.clear();
        }
        if !self.all_have_uvs {
            self.mesh.uvs.clear();
        }
        // As with normals: a part only some of whose vertices are coloured
        // has no colour channel it can honestly present.
        if !self.all_have_colors {
            self.mesh.colors.clear();
        }
        self.mesh.name = self.name;
        self.mesh.material = self.material;
        scene.meshes.push(self.mesh);
    }
}

fn read_vec3<'a>(
    words: &mut impl Iterator<Item = &'a str>,
    line: usize,
) -> Result<Vec3, ImportError> {
    let values = read_floats(words, line)?;
    if values.len() < 3 {
        return Err(ImportError::Malformed(format!(
            "line {}: expected three numbers, found {}",
            line + 1,
            values.len()
        )));
    }
    Ok(vec3(values[0], values[1], values[2]))
}

fn read_floats<'a>(
    words: &mut impl Iterator<Item = &'a str>,
    line: usize,
) -> Result<Vec<f32>, ImportError> {
    words
        .map(|w| {
            w.parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| {
                    ImportError::Malformed(format!(
                        "line {}: `{w}` is not a finite number",
                        line + 1
                    ))
                })
        })
        .collect()
}

/// Read an MTL, mapping Phong parameters onto the OpenPBR model.
///
/// §6.1 point 8 specifies the conversion: `roughness = sqrt(2 / (Ns + 2))`.
/// That is the standard inversion of the Blinn-Phong exponent, and it is a
/// genuine approximation — Phong's lobe is not a GGX lobe, so the match is
/// close at the extremes and loosest in the middle. It is applied here rather
/// than left to the user because the alternative is every OBJ importing dead
/// flat.
///
/// # Maps
///
/// `map_Kd` is the one that matters — a downloaded OBJ almost always has one,
/// and without it the model arrives as flat colour however good its textures
/// are. `bump`/`map_Bump`, `norm`, `map_Ks` and the PBR extensions `map_Pr`
/// and `map_Pm` come too.
///
/// `bump` and `norm` are not the same map and are not treated as such: OBJ's
/// original `bump` is a height field, while `norm` is the later convention for
/// a tangent-space normal map. Reading one as the other turns a surface inside
/// out, so the role is taken from the keyword.
///
/// `d` (and `Tr`, which is the same number inverted) becomes
/// `geometry_opacity`, and `map_d` drives it per texel.
///
/// `map_Ns` is recognised and refused with a reason rather than ignored: it is
/// a Phong exponent per texel, which needs the same inversion as `Ns` applied
/// through the texture rather than to a number.
pub fn read_mtl(
    bytes: &[u8],
    textures: &mut Vec<crate::raw::RawTexture>,
    warnings: &mut Vec<String>,
) -> Result<Vec<(String, MaterialDefinition)>, ImportError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ImportError::Malformed("the material library is not valid UTF-8".into()))?;

    let mut out: Vec<(String, MaterialDefinition)> = Vec::new();
    let mut name = String::new();
    let mut diffuse = Spectrum::splat(0.5);
    let mut emission = Spectrum::ZERO;
    let mut exponent = 32.0f32;
    let mut ior = 1.5f32;
    let mut dissolve = 1.0f32;
    let mut maps = Maps::default();
    let mut open = false;

    let flush = |out: &mut Vec<(String, MaterialDefinition)>,
                 textures: &mut Vec<crate::raw::RawTexture>,
                 name: &str,
                 diffuse: Spectrum,
                 emission: Spectrum,
                 exponent: f32,
                 ior: f32,
                 dissolve: f32,
                 maps: &Maps| {
        // sqrt(2 / (Ns + 2)), clamped: a zero exponent means "fully rough" and
        // an enormous one a mirror, and both appear in real files.
        let roughness = (2.0 / (exponent.max(0.0) + 2.0)).sqrt().clamp(0.0, 1.0);
        let emissive = emission != Spectrum::ZERO && diffuse == Spectrum::ZERO;
        let material = if emissive {
            Material {
                emission_color: emission,
                emission_luminance: 1.0,
                ..Material::diffuse(diffuse)
            }
        } else {
            Material {
                specular_ior: ior,
                emission_color: emission,
                emission_luminance: if emission == Spectrum::ZERO { 0.0 } else { 1.0 },
                geometry_opacity: dissolve,
                ..Material::plastic(diffuse, roughness)
            }
        };
        let material_type = if emissive {
            MaterialType::Emissive
        } else {
            MaterialType::Plastic
        };
        let mut definition = MaterialDefinition::new(name, material_type, material);

        // §5.2: the role decides the colour space, and it is the only thing
        // that can — `roughness.png` and `colour.png` are the same kind of
        // file. The keyword the map arrived under is where the role comes
        // from, which is why `bump` and `norm` are kept apart above.
        use tequila_materials::graph::Value;
        use tequila_materials::material_type::Param;
        use tequila_texture::texture::MapRole;
        for (uri, role, param, fallback) in [
            (
                &maps.base_color,
                MapRole::Color,
                Param::BaseColor,
                Value::Color(diffuse),
            ),
            (
                &maps.specular,
                MapRole::Color,
                Param::SpecularColor,
                Value::Color(Spectrum::ONE),
            ),
            (
                &maps.height,
                MapRole::Height,
                Param::Bump,
                Value::Float(0.0),
            ),
            (
                &maps.normal,
                MapRole::Normal,
                Param::Bump,
                Value::Float(0.0),
            ),
            (
                &maps.roughness,
                MapRole::Data,
                Param::SpecularRoughness,
                Value::Float(roughness),
            ),
            (
                &maps.metalness,
                MapRole::Data,
                Param::BaseMetalness,
                Value::Float(0.0),
            ),
            // The material's own dissolve is the fallback, so a mask that
            // fails to load leaves the surface as the file described it
            // rather than punching it out entirely.
            (
                &maps.opacity,
                MapRole::Data,
                Param::Opacity,
                Value::Float(dissolve),
            ),
        ] {
            if let Some(uri) = uri {
                attach_map(&mut definition, textures, uri, role, param, fallback);
            }
        }

        out.push((name.to_string(), definition));
    };

    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut words = line.split_whitespace();
        let Some(keyword) = words.next() else {
            continue;
        };
        let rest: Vec<&str> = words.collect();

        match keyword {
            "newmtl" => {
                if open {
                    flush(
                        &mut out, textures, &name, diffuse, emission, exponent, ior, dissolve,
                        &maps,
                    );
                }
                name = rest.join(" ");
                diffuse = Spectrum::splat(0.5);
                emission = Spectrum::ZERO;
                exponent = 32.0;
                ior = 1.5;
                dissolve = 1.0;
                maps = Maps::default();
                open = true;
            }
            "Kd" => diffuse = read_color(&rest).unwrap_or(diffuse),
            "Ke" => emission = read_color(&rest).unwrap_or(emission),
            "Ns" => exponent = read_scalar(&rest).unwrap_or(exponent),
            "Ni" => ior = read_scalar(&rest).unwrap_or(ior).clamp(1.0, 3.0),
            // Dissolve: 1 is opaque. `Tr` is the same number the other way up,
            // and files in the wild use both, sometimes in the same library.
            "d" => dissolve = read_scalar(&rest).unwrap_or(dissolve).clamp(0.0, 1.0),
            "Tr" => {
                dissolve = 1.0 - read_scalar(&rest).unwrap_or(1.0 - dissolve).clamp(0.0, 1.0)
            }

            "map_Kd" => maps.base_color = map_filename(&rest).or(maps.base_color),
            "map_Ks" => maps.specular = map_filename(&rest).or(maps.specular),
            // OBJ's own bump keyword, either spelling. A height field.
            "map_Bump" | "map_bump" | "bump" => maps.height = map_filename(&rest).or(maps.height),
            // The later convention for a real tangent-space normal map.
            "norm" | "map_Kn" => maps.normal = map_filename(&rest).or(maps.normal),
            // The PBR extension, which exporters emit increasingly often.
            "map_Pr" => maps.roughness = map_filename(&rest).or(maps.roughness),
            "map_Pm" => maps.metalness = map_filename(&rest).or(maps.metalness),

            // A cutout mask: white keeps the surface, black punches a hole.
            "map_d" => maps.opacity = map_filename(&rest).or(maps.opacity),

            // Recognised and declined, with the reason. Silence here reads as
            // the renderer having lost the map rather than never having had
            // it, which is the distinction this codebase keeps having to
            // restore.
            "map_Ns" => warnings.push(format!(
                "`{name}` has a specular exponent map; only the single `Ns` \
                 value is read, because converting an exponent to roughness \
                 per texel needs a node the graph does not have yet"
            )),
            _ => {}
        }
    }
    if open {
        flush(
            &mut out, textures, &name, diffuse, emission, exponent, ior, dissolve, &maps,
        );
    }
    Ok(out)
}

/// The map filenames one `newmtl` block named.
#[derive(Clone, Default)]
struct Maps {
    base_color: Option<String>,
    /// OBJ's original `bump`: a height field.
    height: Option<String>,
    /// `norm`: a tangent-space normal map.
    normal: Option<String>,
    specular: Option<String>,
    roughness: Option<String>,
    metalness: Option<String>,
    /// `map_d`, a cutout mask: white keeps the surface, black makes a hole.
    opacity: Option<String>,
}

/// The filename out of a `map_*` line, past its options.
///
/// A map line may carry options before the filename — `-bm 0.5`, `-s 1 1 1`,
/// `-clamp on` — and a filename may contain spaces, so neither "the last word"
/// nor "the first word" is right. Options begin with `-`; what follows the
/// last of them, joined back up, is the name.
///
/// The numeric options take between one and three values, and which it is
/// cannot be told from the option alone (`-s u`, `-s u v` and `-s u v w` are
/// all legal), so their arguments are taken as far as the words keep parsing
/// as numbers. A filename that is itself a number would defeat that, and is
/// not a thing.
fn map_filename(words: &[&str]) -> Option<String> {
    let mut i = 0;
    while i < words.len() && words[i].starts_with('-') {
        let option = words[i];
        i += 1;
        match option {
            // One argument, and it is not a number, so it has to be named.
            "-blendu" | "-blendv" | "-cc" | "-clamp" | "-imfchan" | "-type" => i += 1,
            // Numeric, of variable length.
            _ => {
                while i < words.len() && words[i].parse::<f32>().is_ok() {
                    i += 1;
                }
            }
        }
    }
    let name = words.get(i..)?.join(" ");
    (!name.trim().is_empty()).then(|| name.trim().to_string())
}

/// Wire one map into a material's graph, reserving its slot in the scene's
/// texture list.
fn attach_map(
    definition: &mut MaterialDefinition,
    textures: &mut Vec<crate::raw::RawTexture>,
    uri: &str,
    role: tequila_texture::texture::MapRole,
    param: tequila_materials::material_type::Param,
    fallback: tequila_materials::graph::Value,
) {
    use tequila_materials::graph::{Node, NodeKind, Socket, TextureId};

    let wanted = crate::raw::RawTexture {
        uri: uri.to_string(),
        role,
        embedded: None,
    };
    let index = textures
        .iter()
        .position(|existing| existing.same_image(&wanted))
        .unwrap_or_else(|| {
            textures.push(wanted);
            textures.len() - 1
        });

    let graph = &mut definition.graph;
    let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
        mapping: tequila_texture::projection::Mapping::default(),
    }));
    let node = graph.add_node(Node::new(NodeKind::ImageTexture {
        texture: TextureId(index as u32),
        fallback,
    }));
    let output = graph.output();
    graph.connect(mapping, node, Socket::Input(0));
    // A height map (`bump`, `map_Bump`) is 0 to 1 and needs a depth to be a
    // bump; a normal map is read as one straight off the socket.
    if param == tequila_materials::material_type::Param::Bump
        && role != tequila_texture::texture::MapRole::Normal
    {
        let bump = graph.add_node(Node::new(NodeKind::Bump {
            depth: tequila_materials::graph::DEFAULT_BUMP_DEPTH,
        }));
        graph.connect(node, bump, Socket::Input(0));
        graph.connect(bump, output, Socket::Param(param));
        return;
    }
    graph.connect(node, output, Socket::Param(param));
}

fn read_color(words: &[&str]) -> Option<Spectrum> {
    let values: Vec<f32> = words.iter().filter_map(|w| w.parse().ok()).collect();
    match values.as_slice() {
        [v] if v.is_finite() => Some(Spectrum::splat(*v)),
        [r, g, b, ..] if r.is_finite() && g.is_finite() && b.is_finite() => {
            // Linear sRGB, as `map_Kd` is once decoded.
            Some(Spectrum::from_linear_srgb(tequila_core::math::vec3(*r, *g, *b)))
        }
        _ => None,
    }
}

fn read_scalar(words: &[&str]) -> Option<f32> {
    words.first()?.parse::<f32>().ok().filter(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_mtl(_: &str) -> Option<Vec<u8>> {
        None
    }

    #[test]
    fn reads_a_quad_and_triangulates_it() {
        let obj = b"v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1 2 3 4\n";
        let scene = read(obj, "quad", &no_mtl).unwrap();

        assert_eq!(scene.meshes.len(), 1);
        assert_eq!(scene.meshes[0].positions.len(), 4);
        assert_eq!(
            scene.meshes[0].indices,
            vec![0, 1, 2, 0, 2, 3],
            "the fan must keep the original winding"
        );
        scene.validate().unwrap();
    }

    /// Negative indices count back from the end. Getting this wrong reads the
    /// wrong vertices and produces a scrambled mesh rather than an error.
    #[test]
    fn negative_indices_count_back_from_the_end() {
        let obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n";
        let scene = read(obj, "t", &no_mtl).unwrap();
        assert_eq!(scene.meshes[0].indices, vec![0, 1, 2]);
        assert_eq!(scene.meshes[0].positions[2], vec3(0.0, 1.0, 0.0));
    }

    #[test]
    fn distinct_index_triples_become_distinct_vertices() {
        // Two faces sharing positions but with different UVs: a seam. Merging
        // them would wrap the texture across it.
        let obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\n\
                    vt 0 0\nvt 1 0\nvt 0 1\nvt 1 1\n\
                    f 1/1 2/2 3/3\nf 1/4 2/2 3/3\n";
        let scene = read(obj, "t", &no_mtl).unwrap();
        assert_eq!(
            scene.meshes[0].positions.len(),
            4,
            "vertex 1 appears with two different UVs and must be split"
        );
        assert_eq!(scene.meshes[0].uvs.len(), 4);
    }

    /// A part where only some corners carry normals cannot present a normal
    /// channel: half authored and half filler is worse than regenerating.
    #[test]
    fn a_partial_normal_channel_is_dropped_entirely() {
        let obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\nv 1 1 0\n\
                    vn 0 0 1\n\
                    f 1//1 2//1 3//1\nf 2 4 3\n";
        let scene = read(obj, "t", &no_mtl).unwrap();
        assert!(scene.meshes[0].normals.is_empty());
        scene.validate().unwrap();
    }

    #[test]
    fn groups_and_materials_each_start_a_new_part() {
        let obj = b"v 0 0 0\nv 1 0 0\nv 0 1 0\n\
                    g body\nf 1 2 3\n\
                    g lid\nf 1 2 3\n";
        let scene = read(obj, "thing", &no_mtl).unwrap();
        assert_eq!(scene.meshes.len(), 2);
        assert_eq!(scene.meshes[0].name, "body");
        assert_eq!(scene.meshes[1].name, "lid");
        // Multiple parts get a group to live under.
        assert_eq!(scene.nodes.len(), 3);
        assert_eq!(scene.roots, vec![0]);
        scene.validate().unwrap();
    }

    #[test]
    fn a_missing_material_library_warns_but_still_imports() {
        let obj = b"mtllib gone.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let scene = read(obj, "t", &no_mtl).unwrap();
        assert_eq!(scene.meshes.len(), 1, "geometry must survive");
        assert!(scene.warnings.iter().any(|w| w.contains("gone.mtl")));
    }

    #[test]
    fn materials_come_from_the_library_and_map_the_exponent() {
        let mtl: Vec<u8> = b"newmtl red\nKd 1 0 0\nNs 0\n\
                             newmtl shiny\nKd 0.5 0.5 0.5\nNs 1000\n"
            .to_vec();
        let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl red\nf 1 2 3\n";
        let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();

        assert_eq!(scene.materials.len(), 2);
        assert_eq!(scene.meshes[0].material, Some(0));
        // `Kd` is a linear sRGB colour, as the textures beside it are, and
        // renders as the red it states rather than as ACEScg's wider one.
        let red = scene.materials[0].material().base_color.to_linear_srgb();
        assert!(
            (red.x - 1.0).abs() < 1e-3 && red.y.abs() < 1e-3 && red.z.abs() < 1e-3,
            "Kd 1 0 0 renders as {red:?}"
        );
        // Ns = 0 is fully rough; Ns = 1000 is nearly a mirror.
        assert!((scene.materials[0].material().specular_roughness - 1.0).abs() < 1e-4);
        assert!(scene.materials[1].material().specular_roughness < 0.05);
        scene.validate().unwrap();
    }

    /// OBJ's texture coordinates are flipped on the way in.
    ///
    /// OBJ puts v=0 at the bottom of the image and glTF puts it at the top,
    /// and this renderer follows glTF — `Image::sample_bilinear` maps v=0 to
    /// row 0. Taking `vt` verbatim therefore imported every textured OBJ
    /// vertically mirrored. On a checker that is obvious; on a wood grain it
    /// is invisible, and on a logo it is upside down.
    ///
    /// The `vt` values here are the four corners in order, so a flip is the
    /// difference between v running 0,0,1,1 and 1,1,0,0 — not something a
    /// symmetric fixture could hide.
    #[test]
    fn obj_texture_coordinates_are_flipped_to_the_renderers_convention() {
        let obj = b"v 0 0 0\nv 1 0 0\nv 1 1 0\n\
                    vt 0 0\nvt 1 0.25\nvt 1 1\n\
                    f 1/1 2/2 3/3\n";
        let scene = read(obj, "t", &no_mtl).unwrap();
        let uvs = &scene.meshes[0].uvs;

        assert_eq!(uvs.len(), 3);
        assert!((uvs[0].y - 1.0).abs() < 1e-6, "v 0 became {}", uvs[0].y);
        assert!((uvs[1].y - 0.75).abs() < 1e-6, "v 0.25 became {}", uvs[1].y);
        assert!((uvs[2].y - 0.0).abs() < 1e-6, "v 1 became {}", uvs[2].y);
        // u is untouched: the two conventions agree on the horizontal axis.
        assert!((uvs[0].x - 0.0).abs() < 1e-6);
        assert!((uvs[1].x - 1.0).abs() < 1e-6);
    }

    /// Blender names the MTL after the model, spaces and all; a map named
    /// in an MTL in a subfolder is in that subfolder.
    #[test]
    fn an_mtl_with_a_space_in_its_name_and_a_folder_of_its_own_is_found() {
        let obj = b"mtllib materials/Office Chair.mtl\nusemtl wood\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let mtl = |name: &str| -> Option<Vec<u8>> {
            (name == "materials/Office Chair.mtl").then(|| b"newmtl wood\nKd 1 1 1\nmap_Kd oak.jpg\nmap_Bump C:\\maps\\bump.png\n".to_vec())
        };
        let scene = read(obj, "chair", &mtl).unwrap();
        assert!(scene.warnings.iter().all(|w| !w.contains("could not be read")), "{:?}", scene.warnings);
        let uris: Vec<&str> = scene.textures.iter().map(|t| t.uri.as_str()).collect();
        assert!(uris.contains(&"materials/oak.jpg"), "{uris:?}");
        assert!(uris.contains(&"C:\\maps\\bump.png"), "an absolute path is left as it is: {uris:?}");
    }

    /// A Windows code page in a name costs the name's accents, not the file.
    #[test]
    fn a_file_that_is_not_utf8_still_reads() {
        let obj = b"o Caf\xe9\nv 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n";
        let scene = read(obj, "t", &no_mtl).expect("the geometry is ASCII");
        assert_eq!(scene.meshes[0].indices.len(), 3);
    }

    /// An OBJ's maps arrive, in the roles their keywords give them.
    ///
    /// `map_Kd` is the one that matters: a downloaded OBJ almost always has
    /// one, and the reader ignored every `map_*` line, so the model imported
    /// as flat colour no matter how good its textures were.
    #[test]
    fn an_mtl_brings_its_maps_in() {
        let mtl: Vec<u8> = b"newmtl painted\nKd 0.8 0.2 0.2\n\
                             map_Kd colour.png\n\
                             bump height.png\n\
                             map_Pr rough.png\n\
                             map_Pm metal.png\n"
            .to_vec();
        let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\n\
                    vt 0 0\nvt 1 0\nvt 0 1\n\
                    usemtl painted\nf 1/1 2/2 3/3\n";
        let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();
        scene.validate().unwrap();

        let names: Vec<&str> = scene.textures.iter().map(|t| t.uri.as_str()).collect();
        assert_eq!(names.len(), 4, "the maps that came in were {names:?}");
        for wanted in ["colour.png", "height.png", "rough.png", "metal.png"] {
            assert!(names.contains(&wanted), "{wanted} is missing from {names:?}");
        }

        // §5.2: the role decides the colour space, and it can only come from
        // the keyword — these are all PNGs.
        use tequila_texture::texture::MapRole;
        let role_of = |uri: &str| {
            scene
                .textures
                .iter()
                .find(|t| t.uri == uri)
                .map(|t| t.role)
                .unwrap()
        };
        assert_eq!(role_of("colour.png"), MapRole::Color);
        assert_eq!(role_of("height.png"), MapRole::Height);
        assert_eq!(role_of("rough.png"), MapRole::Data);
        assert_eq!(role_of("metal.png"), MapRole::Data);

        // And the graph actually refers to them, or they would be listed in
        // the panel and used by nothing.
        let used = scene.materials[0]
            .graph
            .compile()
            .expect("compiles")
            .requirements
            .textures;
        assert_eq!(used.len(), 4, "the graph refers to {} of them", used.len());
    }

    /// `bump` and `norm` are different maps and must not be conflated.
    ///
    /// OBJ's original `bump` is a height field; `norm` is the later convention
    /// for a tangent-space normal map. Reading one as the other turns the
    /// surface inside out, and both spellings appear in real files.
    #[test]
    fn a_height_map_and_a_normal_map_are_told_apart() {
        use tequila_texture::texture::MapRole;
        for (keyword, expected) in [
            ("bump", MapRole::Height),
            ("map_Bump", MapRole::Height),
            ("norm", MapRole::Normal),
        ] {
            let mtl = format!("newmtl m\nKd 1 1 1\n{keyword} bumps.png\n").into_bytes();
            let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl m\nf 1 2 3\n";
            let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();
            assert_eq!(
                scene.textures.first().map(|t| t.role),
                Some(expected),
                "`{keyword}` gave the wrong role"
            );
        }
    }

    /// A map line's options are skipped and a spaced filename survives them.
    ///
    /// Neither "the first word" nor "the last word" is the filename: options
    /// come before it and it may contain spaces. Real exports carry `-bm` on
    /// bump lines constantly.
    #[test]
    fn map_options_are_skipped_and_spaces_survive() {
        let mtl: Vec<u8> =
            b"newmtl m\nKd 1 1 1\nmap_Kd -clamp on -s 1 1 1 -o 0 0 base colour.png\n\
              bump -bm 0.4 height map.png\n"
                .to_vec();
        let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl m\nf 1 2 3\n";
        let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();

        let names: Vec<&str> = scene.textures.iter().map(|t| t.uri.as_str()).collect();
        assert!(
            names.contains(&"base colour.png"),
            "the options ate the filename: {names:?}"
        );
        assert!(
            names.contains(&"height map.png"),
            "`-bm` ate the filename: {names:?}"
        );
    }

    /// One file named by two materials is one entry, and so one decode.
    #[test]
    fn two_materials_sharing_a_map_share_its_entry() {
        let mtl: Vec<u8> = b"newmtl a\nKd 1 0 0\nmap_Kd shared.png\n\
                             newmtl b\nKd 0 1 0\nmap_Kd shared.png\n"
            .to_vec();
        let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl a\nf 1 2 3\n";
        let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();
        assert_eq!(
            scene.textures.len(),
            1,
            "the shared map was brought in {} times",
            scene.textures.len()
        );
    }

    /// A map the renderer cannot honour is declined with a reason, not
    /// ignored.
    ///
    /// One entry left. `map_d` used to be here too, and came off when cutout
    /// opacity landed — the warning is checked for its whole text now, because
    /// the version of this test that only looked for the word "opacity" sat
    /// happily through a message with a broken line continuation in it.
    #[test]
    fn an_unsupported_map_says_why() {
        let mtl: Vec<u8> = b"newmtl m\nKd 1 1 1\nmap_Ns gloss.png\n".to_vec();
        let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl m\nf 1 2 3\n";
        let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();

        assert_eq!(
            scene.warnings,
            vec![
                "`m` has a specular exponent map; only the single `Ns` value \
                 is read, because converting an exponent to roughness per \
                 texel needs a node the graph does not have yet"
                    .to_string()
            ],
        );
        assert!(
            scene.textures.is_empty(),
            "a map that cannot be honoured was brought in anyway"
        );
    }

    /// `d` and `Tr` are the same number, written upside down from each other.
    ///
    /// Both appear in real files, sometimes in one library, and getting the
    /// sense backwards turns every solid surface into a hole. 0.25 rather than
    /// 0.5, because 0.5 is its own complement and would pass either way.
    #[test]
    fn dissolve_and_transparency_are_read_as_opacity() {
        let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\nusemtl m\nf 1 2 3\n";

        let mtl: Vec<u8> = b"newmtl m\nKd 1 1 1\nd 0.25\n".to_vec();
        let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();
        let opacity = scene.materials[0].material().geometry_opacity;
        assert!((opacity - 0.25).abs() < 1e-6, "`d 0.25` gave {opacity}");

        let mtl: Vec<u8> = b"newmtl m\nKd 1 1 1\nTr 0.25\n".to_vec();
        let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();
        let opacity = scene.materials[0].material().geometry_opacity;
        assert!(
            (opacity - 0.75).abs() < 1e-6,
            "`Tr 0.25` is three-quarters opaque, and gave {opacity}"
        );

        // Absent means solid, which is the state every material starts in.
        let mtl: Vec<u8> = b"newmtl m\nKd 1 1 1\n".to_vec();
        let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();
        assert_eq!(scene.materials[0].material().geometry_opacity, 1.0);
    }

    /// `map_d` drives opacity per texel, with `d` as its fallback.
    #[test]
    fn a_cutout_mask_drives_opacity() {
        use tequila_materials::material_type::Param;
        use tequila_texture::texture::MapRole;

        let mtl: Vec<u8> = b"newmtl m\nKd 1 1 1\nd 0.5\nmap_d holes.png\n".to_vec();
        let obj = b"mtllib m.mtl\nv 0 0 0\nv 1 0 0\nv 0 1 0\n\
                    vt 0 0\nvt 1 0\nvt 0 1\nusemtl m\nf 1/1 2/2 3/3\n";
        let scene = read(obj, "t", &|_| Some(mtl.clone())).unwrap();

        assert!(
            scene.warnings.is_empty(),
            "a cutout mask is implemented now: {:?}",
            scene.warnings
        );
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(scene.textures[0].uri, "holes.png");
        // Data, not colour: an sRGB curve on a mask would bend every edge.
        assert_eq!(scene.textures[0].role, MapRole::Data);

        // And it reaches opacity, which is what makes the surface a cutout at
        // all rather than a texture nothing samples.
        let program = scene.materials[0].graph.compile().expect("compiles");
        assert!(
            program.driven.iter().any(|(p, _)| *p == Param::Opacity),
            "the mask does not drive opacity: {:?}",
            program.driven
        );
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        for bad in [
            "v 0 0 0\nf 1 2 3\n",                   // indices past the end
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 0\n", // index 0 is illegal
            "v 0 0\nf 1 1 1\n",                     // short vertex
            "v a b c\n",                            // not numbers
            "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2\n",   // two-corner face
            "",                                     // no faces at all
        ] {
            assert!(read(bad.as_bytes(), "t", &no_mtl).is_err(), "{bad:?}");
        }
    }
}
