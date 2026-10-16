//! glTF 2.0 export: the scene as one binary `.glb` (`docs/03` §8, §9).
//!
//! For the web configurator's live view and for any other tool. A part is a
//! node at its world transform with one mesh; its material maps from
//! OpenPBR onto glTF's metallic-roughness model and the extensions three.js,
//! Blender and model-viewer read: transmission and IOR for glass, clearcoat,
//! sheen, specular and emissive strength. Colours go from the rendering
//! space, ACEScg, to glTF's linear sRGB.
//!
//! A material whose graph drives anything - a procedural wood, an image on
//! a triplanar mapping, a label - has no glTF equivalent, so it is **baked**:
//! evaluated by the renderer's own `shade` at every texel of an atlas laid
//! out over the part's triangles, into a base-colour and a
//! metallic-roughness texture. The atlas gives every triangle its own cell,
//! sized by its area, so patterns in object space, triplanar images and
//! labels all come out right whatever UVs the model has or lacks. A mesh
//! too dense for that (over [`ATLAS_TRIANGLES`]) is baked to vertex colours
//! instead, which at that density is the finer of the two.
//!
//! Multi-Material variants are written as `KHR_materials_variants`, one
//! variant per Multi-Material choice ("Body: Red"), so a viewer that knows
//! the extension can switch them. [`Exported`] says which glTF node and
//! material stands for what, for the configurator's own player.

use crate::json::Value;
use crate::json_write::Object;
use tequila_core::math::{Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::CompiledGraph;
use tequila_materials::openpbr::Material;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_scene::document::SceneDocument;
use tequila_scene::tree::{MaterialKey, MeshKey, NodeId, NodeKind};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Past this many triangles a mesh is baked to vertex colours.
pub const ATLAS_TRIANGLES: usize = 40_000;

#[derive(Clone, Debug)]
pub struct GltfOptions {
    /// Every part, whichever model set is shown - the configurator's player
    /// switches them itself - rather than only those shown now.
    pub every_model_set: bool,
    /// Pixels on a side of a baked atlas.
    pub bake_size: usize,
}

impl Default for GltfOptions {
    fn default() -> Self {
        GltfOptions { every_model_set: true, bake_size: 1024 }
    }
}

/// What was written, and where each thing went.
#[derive(Debug, Default)]
pub struct Exported {
    pub glb: Vec<u8>,
    /// The glTF node each exported part became.
    pub nodes: BTreeMap<NodeId, usize>,
    /// The glTF material for a material on a part: the same material baked
    /// onto two meshes is two glTF materials.
    pub materials: BTreeMap<(NodeId, MaterialKey), usize>,
    /// The `KHR_materials_variants` index of each Multi-Material's variant.
    pub variants: BTreeMap<(u64, usize), usize>,
    /// Bounds of what was exported, in world space.
    pub bounds: Option<(Vec3, Vec3)>,
    pub warnings: Vec<String>,
}

/// The binary chunk being assembled, with its views and accessors.
#[derive(Default)]
struct Builder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    images: Vec<Value>,
    textures: Vec<Value>,
    samplers: Vec<Value>,
}

impl Builder {
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let mut view = Object::new();
        view.set("buffer", 0usize).set("byteOffset", self.bin.len()).set("byteLength", bytes.len());
        if let Some(target) = target {
            view.set("target", target as usize);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view.build());
        self.views.len() - 1
    }

    /// A float accessor of `width` components a row; positions get their
    /// bounds, which glTF requires.
    fn floats(&mut self, data: &[f32], width: usize, bounded: bool) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34962));
        let kind = match width {
            2 => "VEC2",
            3 => "VEC3",
            4 => "VEC4",
            _ => "SCALAR",
        };
        let mut accessor = Object::new();
        accessor.set("bufferView", view).set("componentType", 5126usize).set("count", data.len() / width).set("type", kind);
        if bounded {
            let mut lo = vec![f32::INFINITY; width];
            let mut hi = vec![f32::NEG_INFINITY; width];
            for row in data.chunks_exact(width) {
                for k in 0..width {
                    lo[k] = lo[k].min(row[k]);
                    hi[k] = hi[k].max(row[k]);
                }
            }
            accessor.set("min", lo).set("max", hi);
        }
        self.accessors.push(accessor.build());
        self.accessors.len() - 1
    }

    fn indices(&mut self, data: &[u32]) -> usize {
        let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(34963));
        let mut accessor = Object::new();
        accessor.set("bufferView", view).set("componentType", 5125usize).set("count", data.len()).set("type", "SCALAR");
        self.accessors.push(accessor.build());
        self.accessors.len() - 1
    }

    /// A PNG as a texture; `nearest` atlases are read without filtering
    /// across cells at a distance.
    fn texture(&mut self, png: &[u8], atlas: bool) -> usize {
        let view = self.view(png, None);
        let mut image = Object::new();
        image.set("bufferView", view).set("mimeType", "image/png");
        self.images.push(image.build());
        if self.samplers.is_empty() {
            // 0: ordinary, mipmapped and repeating. 1: an atlas, filtered
            // but not mipmapped, so far cells do not bleed into near ones.
            let mut repeat = Object::new();
            repeat.set("magFilter", 9729usize).set("minFilter", 9987usize).set("wrapS", 10497usize).set("wrapT", 10497usize);
            self.samplers.push(repeat.build());
            let mut clamp = Object::new();
            clamp.set("magFilter", 9729usize).set("minFilter", 9729usize).set("wrapS", 33071usize).set("wrapT", 33071usize);
            self.samplers.push(clamp.build());
        }
        let mut texture = Object::new();
        texture.set("source", self.images.len() - 1).set("sampler", if atlas { 1usize } else { 0 });
        self.textures.push(texture.build());
        self.textures.len() - 1
    }
}

/// Linear sRGB, from the rendering space.
fn srgb(c: Spectrum) -> [f32; 3] {
    let v = c.to_linear_srgb();
    [v.x.max(0.0), v.y.max(0.0), v.z.max(0.0)]
}

/// The colour a glTF base colour stands for: base, weighted, with what
/// subsurface scattering mixes in.
fn base_colour(m: &Material) -> [f32; 3] {
    let base = m.base_color * m.base_weight;
    let w = m.subsurface_weight.clamp(0.0, 1.0);
    let mixed = base * (1.0 - w) + m.subsurface_color * w;
    srgb(mixed)
}

fn encode_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let e = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (e * 255.0 + 0.5) as u8
}

fn png(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("writing to memory");
        writer.write_image_data(rgba).expect("writing to memory");
    }
    out
}

/// The glTF material for constant parameters, with `textures` when baked:
/// (base colour, metallic-roughness, emissive, which texture coordinate).
fn material_json(name: &str, m: &Material, baked: Option<(usize, usize, Option<usize>, usize)>, vertex_colours: bool) -> Value {
    let mut pbr = Object::new();
    let base = base_colour(m);
    let alpha = m.geometry_opacity.clamp(0.0, 1.0);
    let texture = |index: usize, coord: usize| {
        let mut t = Object::new();
        t.set("index", index);
        if coord != 0 {
            t.set("texCoord", coord);
        }
        t.build()
    };
    match baked {
        Some((colour, metal_rough, _, coord)) => {
            pbr.set("baseColorTexture", texture(colour, coord)).set("metallicRoughnessTexture", texture(metal_rough, coord));
        }
        None if vertex_colours => {
            pbr.set("baseColorFactor", vec![1.0f32, 1.0, 1.0, alpha]);
        }
        None => {
            pbr.set("baseColorFactor", vec![base[0], base[1], base[2], alpha]);
        }
    }
    if baked.is_none() {
        pbr.set("metallicFactor", m.base_metalness.clamp(0.0, 1.0)).set("roughnessFactor", m.specular_roughness.clamp(0.0, 1.0));
    }
    let mut object = Object::new();
    object.set("name", name).set("pbrMetallicRoughness", pbr.build()).set("doubleSided", true);
    if alpha < 0.999 || m.geometry_opacity < 0.999 {
        object.set("alphaMode", "BLEND");
    }
    let mut extensions = Object::new();
    let mut any = false;
    if m.is_emissive() {
        let e = srgb(m.emission_color);
        let peak = e[0].max(e[1]).max(e[2]).max(1e-6);
        match baked.and_then(|b| b.2) {
            Some(index) => {
                object.set("emissiveTexture", texture(index, baked.map_or(0, |b| b.3)));
                object.set("emissiveFactor", vec![1.0f32, 1.0, 1.0]);
            }
            None => {
                object.set("emissiveFactor", vec![e[0] / peak, e[1] / peak, e[2] / peak]);
            }
        }
        // Nits over a hundred: a display's white is about a hundred nits,
        // and a viewer's exposure of one shows it as white.
        let strength = m.emission_luminance * peak / 100.0;
        if strength > 1.0 {
            let mut s = Object::new();
            s.set("emissiveStrength", strength);
            extensions.set("KHR_materials_emissive_strength", s.build());
            any = true;
        }
    }
    if m.transmission_weight > 0.0 {
        let mut t = Object::new();
        t.set("transmissionFactor", m.transmission_weight.clamp(0.0, 1.0));
        extensions.set("KHR_materials_transmission", t.build());
        let tint = srgb(m.transmission_color);
        if !m.geometry_thin_walled && (tint[0] < 0.999 || tint[1] < 0.999 || tint[2] < 0.999) {
            let mut v = Object::new();
            v.set("thicknessFactor", 0.01f32)
                .set("attenuationColor", tint.to_vec())
                .set("attenuationDistance", if m.transmission_depth > 0.0 { m.transmission_depth } else { 0.01 });
            extensions.set("KHR_materials_volume", v.build());
        }
        any = true;
    }
    if (m.specular_ior - 1.5).abs() > 1e-3 {
        let mut i = Object::new();
        i.set("ior", m.specular_ior);
        extensions.set("KHR_materials_ior", i.build());
        any = true;
    }
    if m.coat_weight > 0.0 {
        let mut c = Object::new();
        c.set("clearcoatFactor", m.coat_weight.clamp(0.0, 1.0)).set("clearcoatRoughnessFactor", m.coat_roughness.clamp(0.0, 1.0));
        extensions.set("KHR_materials_clearcoat", c.build());
        any = true;
    }
    if m.fuzz_weight > 0.0 {
        let f = srgb(m.fuzz_color * m.fuzz_weight);
        let mut s = Object::new();
        s.set("sheenColorFactor", f.to_vec()).set("sheenRoughnessFactor", m.fuzz_roughness.clamp(0.0, 1.0));
        extensions.set("KHR_materials_sheen", s.build());
        any = true;
    }
    let spec = srgb(m.specular_color);
    if (m.specular_weight - 1.0).abs() > 1e-3 || spec.iter().any(|c| (c - 1.0).abs() > 1e-3) {
        let mut s = Object::new();
        s.set("specularFactor", m.specular_weight.clamp(0.0, 1.0)).set("specularColorFactor", spec.to_vec());
        extensions.set("KHR_materials_specular", s.build());
        any = true;
    }
    if any {
        object.set("extensions", extensions.build());
    }
    object.build()
}

/// A mesh's triangles, with normals: the authored ones, or smooth ones
/// made from the faces when it has none.
struct Source<'a> {
    positions: &'a [Vec3],
    normals: Vec<Vec3>,
    uvs: Option<&'a [Vec2]>,
    triangles: Vec<[u32; 3]>,
}

impl<'a> Source<'a> {
    fn of(mesh: &'a tequila_scene::geometry::Mesh) -> Source<'a> {
        let triangles: Vec<[u32; 3]> = mesh
            .indices
            .chunks_exact(3)
            .filter(|t| t.iter().all(|&i| (i as usize) < mesh.positions.len()))
            .map(|t| [t[0], t[1], t[2]])
            .collect();
        let normals = if mesh.normals.len() == mesh.positions.len() {
            mesh.normals.clone()
        } else {
            let mut n = vec![Vec3::ZERO; mesh.positions.len()];
            for t in &triangles {
                let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
                let face = (b - a).cross(c - a);
                for &i in t {
                    n[i as usize] = n[i as usize] + face;
                }
            }
            n.into_iter().map(|v| v.try_normalize().unwrap_or(Vec3::Y)).collect()
        };
        let uvs = mesh.uv_sets.first().filter(|uv| uv.len() == mesh.positions.len()).map(|v| v.as_slice());
        Source { positions: &mesh.positions, normals, uvs, triangles }
    }
}

/// Where each triangle is baked: its three corners in texels of a square
/// image, and whether those are the mesh's own UVs (shared, so written
/// indexed) or an atlas's (one cell each, so every corner its own vertex).
struct Atlas {
    size: usize,
    corners: Vec<[[f32; 2]; 3]>,
    own: bool,
}

/// The mesh's own UVs, when they can hold a bake: inside the unit square
/// and not folded over themselves - a model unwrapped for texturing. Tiled
/// or overlapping UVs, or none, cannot.
fn own_uvs(source: &Source, size: usize) -> Option<Atlas> {
    let uvs = source.uvs?;
    if uvs.iter().any(|uv| !(-1e-3..=1.001).contains(&uv.x) || !(-1e-3..=1.001).contains(&uv.y)) {
        return None;
    }
    // Rasterize the layout coarsely and count texels claimed twice.
    let n = 256usize;
    let mut claimed = vec![false; n * n];
    let (mut covered, mut twice) = (0usize, 0usize);
    let corners: Vec<[[f32; 2]; 3]> = source.triangles.iter().map(|t| t.map(|i| [uvs[i as usize].x, uvs[i as usize].y])).collect();
    for c in &corners {
        let s = c.map(|p| [p[0] * n as f32, p[1] * n as f32]);
        raster(&s, n, |x, y, _| {
            covered += 1;
            if std::mem::replace(&mut claimed[y * n + x], true) {
                twice += 1;
            }
        });
    }
    if covered < n * n / 20 || twice * 50 > covered {
        return None;
    }
    Some(Atlas { size, corners: corners.iter().map(|c| c.map(|p| [p[0] * size as f32, p[1] * size as f32])).collect(), own: true })
}

/// Lay the triangles out two to a square cell, cells sized by the pair's
/// area in powers of two and packed in rows, largest first. Each half is
/// inset from the cell's edges and from the diagonal, so filtering never
/// reaches the neighbour; the gaps are filled by dilation after baking.
fn atlas(source: &Source, size: usize) -> Atlas {
    let areas: Vec<f32> = source
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| source.positions[i as usize]);
            (b - a).cross(c - a).length() * 0.5
        })
        .collect();
    let total: f32 = areas.iter().sum::<f32>().max(1e-20);
    let mut order: Vec<usize> = (0..areas.len()).collect();
    order.sort_by(|&a, &b| areas[b].total_cmp(&areas[a]));
    let pairs: Vec<(usize, Option<usize>)> = order.chunks(2).map(|p| (p[0], p.get(1).copied())).collect();
    let mut fill = 0.85f32;
    loop {
        let side = |area: f32| -> usize {
            let ideal = (area / total * fill).sqrt() * size as f32;
            let p = (ideal.max(8.0) as usize).next_power_of_two();
            let p = if p as f32 > ideal * 1.5 { p / 2 } else { p };
            p.clamp(8, size / 2)
        };
        let mut corners = vec![[[0.0f32; 2]; 3]; areas.len()];
        let (mut x, mut y, mut row) = (0usize, 0usize, 0usize);
        let mut fits = true;
        for &(a, b) in &pairs {
            let s = side(areas[a] + b.map_or(0.0, |b| areas[b]));
            if x + s > size {
                x = 0;
                y += row;
                row = 0;
            }
            if y + s > size {
                fits = false;
                break;
            }
            let (x0, y0, x1, y1) = (x as f32, y as f32, (x + s) as f32, (y + s) as f32);
            corners[a] = [[x0 + 1.5, y0 + 1.5], [x1 - 3.5, y0 + 1.5], [x0 + 1.5, y1 - 3.5]];
            if let Some(b) = b {
                corners[b] = [[x1 - 1.5, y1 - 1.5], [x0 + 3.5, y1 - 1.5], [x1 - 1.5, y0 + 3.5]];
            }
            x += s;
            row = row.max(s);
        }
        if fits || fill < 0.01 {
            return Atlas { size, corners, own: false };
        }
        fill *= 0.8;
    }
}

/// Call `f(x, y, barycentric)` for every texel whose centre lies inside the
/// triangle with corners `c` in texels.
fn raster(c: &[[f32; 2]; 3], size: usize, mut f: impl FnMut(usize, usize, [f32; 3])) {
    let area = (c[1][0] - c[0][0]) * (c[2][1] - c[0][1]) - (c[2][0] - c[0][0]) * (c[1][1] - c[0][1]);
    if area.abs() < 1e-9 {
        return;
    }
    let lo_x = c.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
    let hi_x = (c.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min(size);
    let lo_y = c.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min).floor().max(0.0) as usize;
    let hi_y = (c.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max).ceil() as usize).min(size);
    for y in lo_y..hi_y {
        for x in lo_x..hi_x {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let edge = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]) * (py - a[1]) - (px - a[0]) * (b[1] - a[1])) / area;
            let (w0, w1, w2) = (edge(c[1], c[2]), edge(c[2], c[0]), edge(c[0], c[1]));
            if w0 >= -1e-4 && w1 >= -1e-4 && w2 >= -1e-4 {
                f(x, y, [w0.max(0.0), w1.max(0.0), w2.max(0.0)]);
            }
        }
    }
}

/// Grow covered texels outwards into empty ones, `passes` texels deep, so
/// filtering at a chart's edge reads its own colour and not black.
fn dilate(images: &mut [&mut Vec<u8>], covered: &mut [bool], size: usize, passes: usize) {
    for _ in 0..passes {
        let before = covered.to_vec();
        for y in 0..size {
            for x in 0..size {
                if before[y * size + x] {
                    continue;
                }
                let mut from = None;
                for (dx, dy) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1), (-1, -1), (1, 1), (-1, 1), (1, -1)] {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx >= 0 && ny >= 0 && (nx as usize) < size && (ny as usize) < size && before[ny as usize * size + nx as usize] {
                        from = Some(ny as usize * size + nx as usize);
                        break;
                    }
                }
                if let Some(from) = from {
                    let to = y * size + x;
                    for image in images.iter_mut() {
                        let pixel: [u8; 4] = image[from * 4..from * 4 + 4].try_into().unwrap();
                        image[to * 4..to * 4 + 4].copy_from_slice(&pixel);
                    }
                    covered[to] = true;
                }
            }
        }
    }
}

/// What shading needs to know of a part.
struct Part<'a> {
    source: &'a Source<'a>,
    world: tequila_core::math::Transform,
    scale: f32,
    bounds: (Vec3, Vec3),
}

impl Part<'_> {
    /// The shading point at a place on triangle `t`.
    fn point(&self, t: usize, b: [f32; 3], footprint: f32) -> tequila_texture::projection::SurfacePoint {
        let tri = self.source.triangles[t];
        let p = (0..3).fold(Vec3::ZERO, |acc, k| acc + self.source.positions[tri[k] as usize] * b[k]);
        let n = (0..3).fold(Vec3::ZERO, |acc, k| acc + self.source.normals[tri[k] as usize] * b[k]).try_normalize().unwrap_or(Vec3::Y);
        let uv = match self.source.uvs {
            Some(uvs) => (0..3).fold(Vec2::ZERO, |acc, k| acc + uvs[tri[k] as usize] * b[k]),
            None => Vec2::ZERO,
        };
        let (lo, hi) = self.bounds;
        let half = (hi - lo) * 0.5;
        let safe = |h: f32| if h == 0.0 { 1.0 } else { h.max(1e-5) };
        let generated = tequila_core::math::vec3(
            (p.x - (lo.x + half.x)) / safe(half.x) * 0.5 + 0.5,
            (p.y - (lo.y + half.y)) / safe(half.y) * 0.5 + 0.5,
            (p.z - (lo.z + half.z)) / safe(half.z) * 0.5 + 0.5,
        );
        let world_normal = self.world.normal(n).try_normalize().unwrap_or(n);
        tequila_texture::projection::SurfacePoint {
            position: p,
            normal: n,
            uv,
            footprint,
            scale: self.scale,
            uv_footprint: None,
            curvature: 0.0,
            occlusion: 0.0,
            vertex_color: Spectrum::ONE,
            world: tequila_texture::projection::PointWorld {
                position: self.world.point(p),
                normal: world_normal,
                true_normal: world_normal,
                incoming: world_normal,
                backfacing: false,
                generated,
                object_location: self.world.point(Vec3::ZERO),
                object_random: 0.0,
            },
        }
    }
}

/// Bake `program` over `part` into the atlas: base colour (sRGB, with
/// opacity), metallic-roughness, and emission when the program drives it.
fn bake(part: &Part, atlas: &Atlas, program: &CompiledGraph, textures: &TextureTable) -> (Vec<u8>, Vec<u8>, Option<Vec<u8>>) {
    use rayon::prelude::*;
    let size = atlas.size;
    let emissive = program.driven.iter().any(|(p, _)| {
        matches!(p, tequila_materials::material_type::Param::EmissionColor | tequila_materials::material_type::Param::EmissionLuminance)
    });
    // A texel's width on the part, for the lookups that filter by it.
    let texel_world = {
        let area: f32 = part.source.triangles.iter().map(|t| {
            let [a, b, c] = t.map(|i| part.source.positions[i as usize]);
            (b - a).cross(c - a).length() * 0.5
        }).sum();
        (area / (size * size) as f32).sqrt() * part.scale
    };
    // Each triangle's texels, shaded where they fall; a texel two
    // triangles claim (on a shared edge of the mesh's own UVs) is written
    // by either.
    let shaded: Vec<Vec<(usize, [u8; 4], [u8; 4], [u8; 4])>> = (0..part.source.triangles.len())
        .into_par_iter()
        .map_init(
            || (Registers::default(), Material::default()),
            |(registers, scratch), t| {
                let mut out = Vec::new();
                raster(&atlas.corners[t], size, |x, y, b| {
                    let point = part.point(t, b, texel_world);
                    let m = shade(program, textures, &point, registers, scratch);
                    let base = base_colour(m);
                    let colour = [encode_srgb(base[0]), encode_srgb(base[1]), encode_srgb(base[2]), (m.geometry_opacity.clamp(0.0, 1.0) * 255.0 + 0.5) as u8];
                    let mr = [0, (m.specular_roughness.clamp(0.0, 1.0) * 255.0 + 0.5) as u8, (m.base_metalness.clamp(0.0, 1.0) * 255.0 + 0.5) as u8, 255];
                    let e = srgb(m.emission_color * (m.emission_luminance / 100.0));
                    let glow = [encode_srgb(e[0]), encode_srgb(e[1]), encode_srgb(e[2]), 255];
                    out.push((y * size + x, colour, mr, glow));
                });
                out
            },
        )
        .collect();
    let mut colour = vec![0u8; size * size * 4];
    let mut metal_rough = vec![0u8; size * size * 4];
    let mut glow = vec![0u8; if emissive { size * size * 4 } else { 0 }];
    let mut covered = vec![false; size * size];
    for (i, c, mr, g) in shaded.into_iter().flatten() {
        colour[i * 4..i * 4 + 4].copy_from_slice(&c);
        metal_rough[i * 4..i * 4 + 4].copy_from_slice(&mr);
        if emissive {
            glow[i * 4..i * 4 + 4].copy_from_slice(&g);
        }
        covered[i] = true;
    }
    {
        let mut images: Vec<&mut Vec<u8>> = vec![&mut colour, &mut metal_rough];
        if emissive {
            images.push(&mut glow);
        }
        dilate(&mut images, &mut covered, size, 4);
    }
    (png(size, size, &colour), png(size, size, &metal_rough), emissive.then(|| png(size, size, &glow)))
}

/// Bake `program` at each vertex: the base colour as vertex colours, and
/// the mean metalness and roughness.
fn bake_vertices(part: &Part, program: &CompiledGraph, textures: &TextureTable) -> (Vec<f32>, f32, f32) {
    // One triangle each vertex sits on, for its shading point.
    let mut on = vec![(0usize, 0usize); part.source.positions.len()];
    for (t, tri) in part.source.triangles.iter().enumerate() {
        for (k, &i) in tri.iter().enumerate() {
            on[i as usize] = (t, k);
        }
    }
    let mut registers = Registers::default();
    let mut scratch = Material::default();
    let mut colours = Vec::with_capacity(on.len() * 4);
    let (mut rough, mut metal) = (0.0f32, 0.0f32);
    for &(t, k) in &on {
        let mut b = [0.0; 3];
        b[k] = 1.0;
        let m = shade(program, textures, &part.point(t, b, 0.0), &mut registers, &mut scratch);
        let c = base_colour(m);
        colours.extend_from_slice(&[c[0], c[1], c[2], m.geometry_opacity.clamp(0.0, 1.0)]);
        rough += m.specular_roughness;
        metal += m.base_metalness;
    }
    let n = on.len().max(1) as f32;
    (colours, rough / n, metal / n)
}

/// Write `document` as a `.glb`.
pub fn export(document: &SceneDocument, options: &GltfOptions) -> Exported {
    let mut out = Exported::default();
    let mut b = Builder::default();
    let tree = &document.tree;

    // The parts to write, with the materials each can wear.
    let mut wears: BTreeMap<NodeId, Vec<MaterialKey>> = BTreeMap::new();
    for id in tree.depth_first() {
        let Some(node) = tree.get(id) else { continue };
        if !matches!(node.kind, NodeKind::Part { .. }) || !tree.is_effectively_visible(id) {
            continue;
        }
        if !options.every_model_set && !document.is_shown(id) {
            continue;
        }
        let mut keys: Vec<MaterialKey> = node.material.into_iter().collect();
        for multi in &document.multi_materials {
            if multi.parts.contains(&id) {
                for key in &multi.variants {
                    if !keys.contains(key) {
                        keys.push(*key);
                    }
                }
            }
        }
        wears.insert(id, keys);
    }

    // Programs and the textures they read, once per material.
    let mut programs: BTreeMap<MaterialKey, Option<Arc<CompiledGraph>>> = BTreeMap::new();
    let mut textures = TextureTable::new();
    for keys in wears.values() {
        for key in keys {
            if programs.contains_key(key) {
                continue;
            }
            let program = document
                .materials
                .get(*key)
                .filter(|d| d.graph.len() > 1 || !d.graph.decals.is_empty())
                .and_then(|d| d.graph.compile().ok())
                .filter(|p| !p.driven.is_empty() || !p.decals.is_empty())
                .map(Arc::new);
            if let Some(program) = &program {
                for id in &program.requirements.textures {
                    if textures.get(*id).is_none() {
                        if let Some(loaded) = document.textures.get(*id).and_then(tequila_scene::snapshot::load_texture) {
                            textures.insert(*id, Arc::new(loaded));
                        } else {
                            out.warnings.push(format!("a texture of “{}” could not be read; its fallback is baked", document.materials.get(*key).map(|d| d.name.as_str()).unwrap_or("?")));
                        }
                    }
                }
            }
            programs.insert(*key, program);
        }
    }

    let mut materials: Vec<Value> = Vec::new();
    let mut meshes: Vec<Value> = Vec::new();
    let mut nodes: Vec<Value> = Vec::new();
    // Plain materials are shared; baked ones belong to a part.
    let mut plain: BTreeMap<MaterialKey, usize> = BTreeMap::new();
    let mut variant_names: Vec<Value> = Vec::new();
    for multi in &document.multi_materials {
        for (i, key) in multi.variants.iter().enumerate() {
            let label = document.materials.get(*key).map(|d| d.name.clone()).unwrap_or_else(|| format!("Variant {}", i + 1));
            let mut v = Object::new();
            v.set("name", format!("{}: {}", multi.name, label));
            variant_names.push(v.build());
            out.variants.insert((multi.id, i), variant_names.len() - 1);
        }
    }
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);

    for (&id, keys) in &wears {
        let Some(node) = tree.get(id) else { continue };
        let NodeKind::Part { mesh } = node.kind else { continue };
        let Some(geometry) = document.geometry.get(mesh) else { continue };
        let source = Source::of(geometry);
        if source.triangles.is_empty() {
            continue;
        }
        let world = tree.world_transform(id);
        for p in source.positions {
            let w = world.point(*p);
            lo = lo.min(w);
            hi = hi.max(w);
        }
        let bounds = source.positions.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(l, h), p| (l.min(*p), h.max(*p)));
        let part = Part { source: &source, world, scale: 1.0 / world.inv_length_scale(), bounds };
        let baking = keys.iter().any(|k| programs.get(k).is_some_and(|p| p.is_some()));
        let dense = source.triangles.len() > ATLAS_TRIANGLES;
        let name = node.name.clone();

        // The vertex streams: indexed as authored, or one vertex per
        // triangle corner when an atlas gives every corner its own place.
        let atlas = (baking && !dense).then(|| own_uvs(&source, options.bake_size).unwrap_or_else(|| atlas(&source, options.bake_size)));
        let coord = if atlas.as_ref().is_some_and(|a| a.own) { 0 } else { 1 };
        let (positions, normals, uv0, uv1, indices): (Vec<f32>, Vec<f32>, Option<Vec<f32>>, Option<Vec<f32>>, Vec<u32>) = match atlas.as_ref().filter(|a| !a.own) {
            Some(atlas) => {
                let mut p = Vec::new();
                let mut n = Vec::new();
                let mut u0 = source.uvs.map(|_| Vec::new());
                let mut u1 = Vec::new();
                for (t, tri) in source.triangles.iter().enumerate() {
                    for (k, &i) in tri.iter().enumerate() {
                        let (v, nn) = (source.positions[i as usize], source.normals[i as usize]);
                        p.extend_from_slice(&[v.x, v.y, v.z]);
                        n.extend_from_slice(&[nn.x, nn.y, nn.z]);
                        if let (Some(u0), Some(uvs)) = (u0.as_mut(), source.uvs) {
                            u0.extend_from_slice(&[uvs[i as usize].x, uvs[i as usize].y]);
                        }
                        let c = atlas.corners[t][k];
                        u1.extend_from_slice(&[c[0] / atlas.size as f32, c[1] / atlas.size as f32]);
                    }
                }
                let count = p.len() as u32 / 3;
                (p, n, u0, Some(u1), (0..count).collect())
            }
            None => (
                source.positions.iter().flat_map(|v| [v.x, v.y, v.z]).collect(),
                source.normals.iter().flat_map(|v| [v.x, v.y, v.z]).collect(),
                source.uvs.map(|uvs| uvs.iter().flat_map(|v| [v.x, v.y]).collect()),
                None,
                source.triangles.iter().flatten().copied().collect(),
            ),
        };
        let mut attributes = Object::new();
        attributes.set("POSITION", b.floats(&positions, 3, true)).set("NORMAL", b.floats(&normals, 3, false));
        if let Some(uv0) = &uv0 {
            attributes.set("TEXCOORD_0", b.floats(uv0, 2, false));
        }
        if let Some(uv1) = &uv1 {
            attributes.set("TEXCOORD_1", b.floats(uv1, 2, false));
        }

        // Each material the part can wear, baked for it when it must be.
        let mut part_materials: Vec<(MaterialKey, usize)> = Vec::new();
        let mut vertex_colours: Option<Vec<f32>> = None;
        for key in keys {
            let Some(definition) = document.materials.get(*key) else { continue };
            let constants = definition.material().clone();
            let index = match programs.get(key).cloned().flatten() {
                None => *plain.entry(*key).or_insert_with(|| {
                    materials.push(material_json(&definition.name, &constants, None, false));
                    materials.len() - 1
                }),
                Some(program) => match &atlas {
                    Some(atlas) => {
                        let (colour, metal_rough, glow) = bake(&part, atlas, &program, &textures);
                        let colour = b.texture(&colour, true);
                        let metal_rough = b.texture(&metal_rough, true);
                        let glow = glow.map(|g| b.texture(&g, true));
                        materials.push(material_json(&definition.name, &program.constants, Some((colour, metal_rough, glow, coord)), false));
                        materials.len() - 1
                    }
                    None => {
                        // Dense: the first material's colours go to the
                        // vertices; a variant would need its own, which
                        // glTF cannot switch, so later ones are averaged.
                        let (colours, rough, metal) = bake_vertices(&part, &program, &textures);
                        let mut constants = program.constants.clone();
                        constants.specular_roughness = rough;
                        constants.base_metalness = metal;
                        let use_vertices = vertex_colours.is_none();
                        if use_vertices {
                            vertex_colours = Some(colours.clone());
                        } else {
                            let n = (colours.len() / 4).max(1) as f32;
                            let mean = |k: usize| colours.iter().skip(k).step_by(4).sum::<f32>() / n;
                            constants.base_color = Spectrum::from_linear_srgb(tequila_core::math::vec3(mean(0), mean(1), mean(2)));
                            constants.base_weight = 1.0;
                            constants.subsurface_weight = 0.0;
                            out.warnings.push(format!("“{name}” is too dense to bake each variant; “{}” is its average colour", definition.name));
                        }
                        materials.push(material_json(&definition.name, &constants, None, use_vertices));
                        materials.len() - 1
                    }
                },
            };
            out.materials.insert((id, *key), index);
            part_materials.push((*key, index));
        }
        if let Some(colours) = &vertex_colours {
            attributes.set("COLOR_0", b.floats(colours, 4, false));
        }
        let indices = b.indices(&indices);
        let mut primitive = Object::new();
        primitive.set("attributes", attributes.build()).set("indices", indices).set("mode", 4usize);
        let current = node.material.and_then(|k| part_materials.iter().find(|(m, _)| *m == k)).or(part_materials.first());
        if let Some((_, index)) = current {
            primitive.set("material", *index);
        }
        // Variants: which material each Multi-Material choice puts here.
        let mut mappings: Vec<Value> = Vec::new();
        for multi in document.multi_materials.iter().filter(|m| m.parts.contains(&id)) {
            for (i, key) in multi.variants.iter().enumerate() {
                if let (Some((_, material)), Some(variant)) = (part_materials.iter().find(|(m, _)| m == key), out.variants.get(&(multi.id, i))) {
                    let mut mapping = Object::new();
                    mapping.set("material", *material).set("variants", vec![Value::from(*variant)]);
                    mappings.push(mapping.build());
                }
            }
        }
        if !mappings.is_empty() {
            let mut ext = Object::new();
            let mut v = Object::new();
            v.set("mappings", mappings);
            ext.set("KHR_materials_variants", v.build());
            primitive.set("extensions", ext.build());
        }
        let mut gltf_mesh = Object::new();
        gltf_mesh.set("name", name.clone()).set("primitives", vec![primitive.build()]);
        meshes.push(gltf_mesh.build());

        // Column-major, as glTF has it.
        let m = world.matrix.m;
        let mut matrix = Vec::with_capacity(16);
        for c in 0..4 {
            for r in 0..4 {
                matrix.push(m[r][c]);
            }
        }
        let mut gltf_node = Object::new();
        gltf_node.set("name", name).set("mesh", meshes.len() - 1);
        if matrix != [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
            gltf_node.set("matrix", matrix);
        }
        nodes.push(gltf_node.build());
        out.nodes.insert(id, nodes.len() - 1);
        let _ = mesh;
    }
    if lo.x <= hi.x {
        out.bounds = Some((lo, hi));
    }

    let mut root = Object::new();
    let mut asset = Object::new();
    asset.set("version", "2.0").set("generator", "Tequila");
    root.set("asset", asset.build());
    let mut scene = Object::new();
    scene.set("nodes", (0..nodes.len()).map(Value::from).collect::<Vec<_>>());
    root.set("scene", 0usize).set("scenes", vec![scene.build()]);
    root.set("nodes", nodes).set("meshes", meshes).set("materials", materials.clone());
    // The extensions in use, as glTF wants them declared.
    let mut used: Vec<&str> = Vec::new();
    let text = materials.iter().map(crate::json_write::to_string).collect::<String>();
    for ext in [
        "KHR_materials_emissive_strength",
        "KHR_materials_transmission",
        "KHR_materials_volume",
        "KHR_materials_ior",
        "KHR_materials_clearcoat",
        "KHR_materials_sheen",
        "KHR_materials_specular",
    ] {
        if text.contains(ext) {
            used.push(ext);
        }
    }
    if !variant_names.is_empty() {
        used.push("KHR_materials_variants");
        let mut ext = Object::new();
        let mut v = Object::new();
        v.set("variants", variant_names);
        ext.set("KHR_materials_variants", v.build());
        root.set("extensions", ext.build());
    }
    if !used.is_empty() {
        root.set("extensionsUsed", used.iter().map(|s| Value::from(*s)).collect::<Vec<_>>());
    }
    while b.bin.len() % 4 != 0 {
        b.bin.push(0);
    }
    if !b.bin.is_empty() {
        let mut buffer = Object::new();
        buffer.set("byteLength", b.bin.len());
        root.set("buffers", vec![buffer.build()]);
        root.set("bufferViews", b.views.clone()).set("accessors", b.accessors.clone());
    }
    if !b.images.is_empty() {
        root.set("images", b.images.clone()).set("textures", b.textures.clone()).set("samplers", b.samplers.clone());
    }

    let mut json = crate::json_write::to_string(&root.build()).into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let total = 12 + 8 + json.len() + if b.bin.is_empty() { 0 } else { 8 + b.bin.len() };
    let mut glb = Vec::with_capacity(total);
    glb.extend_from_slice(b"glTF");
    glb.extend_from_slice(&2u32.to_le_bytes());
    glb.extend_from_slice(&(total as u32).to_le_bytes());
    glb.extend_from_slice(&(json.len() as u32).to_le_bytes());
    glb.extend_from_slice(b"JSON");
    glb.extend_from_slice(&json);
    if !b.bin.is_empty() {
        glb.extend_from_slice(&(b.bin.len() as u32).to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend_from_slice(&b.bin);
    }
    out.glb = glb;
    let _ = MeshKey(0);
    out
}
