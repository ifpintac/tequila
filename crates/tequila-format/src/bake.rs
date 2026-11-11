//! Baking a material onto a part, into images: for the export formats whose
//! materials are a few numbers and pictures (glTF, USDZ), which a procedural
//! wood, an image on a triplanar mapping or a label has no equivalent in.
//!
//! The material is evaluated by the renderer's own `shade` at every texel of
//! an image laid out over the part's triangles: the part's own UVs when they
//! can hold a bake - inside the unit square and not folded over themselves -
//! and otherwise an atlas that gives every triangle its own cell, sized by
//! its area, so patterns in object space, triplanar images and labels all
//! come out right whatever UVs the model has or lacks. Each texel gives:
//!
//! * the colour and opacity (sRGB, opacity in alpha), as the format asks for
//!   them from the shaded parameters;
//! * metalness and roughness (glTF's packing: roughness in green, metalness
//!   in blue), with baked occlusion in red when asked;
//! * the emission, when the material drives it;
//! * the coat's weight and roughness (red, green), when asked;
//! * the bumped normal, when asked and the material bumps: a tangent-space
//!   normal map in the frame the layout's UVs run in, as a viewer builds it
//!   from them (green up the image, as OpenGL and USD have it).
//!
//! A mesh too dense for an atlas (over [`ATLAS_TRIANGLES`]) and without UVs
//! of its own is baked to vertex colours instead, which at that density is
//! the finer of the two.

use tequila_core::math::{Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::CompiledGraph;
use tequila_materials::material_type::Param;
use tequila_materials::openpbr::Material;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_render_core::surface_bump::BumpSource;

/// Past this many triangles a mesh without usable UVs is baked to vertex
/// colours.
pub const ATLAS_TRIANGLES: usize = 40_000;

/// Linear sRGB, from the rendering space.
pub(crate) fn srgb(c: Spectrum) -> [f32; 3] {
    let v = c.to_linear_srgb();
    [v.x.max(0.0), v.y.max(0.0), v.z.max(0.0)]
}

pub(crate) fn encode_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let e = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (e * 255.0 + 0.5) as u8
}

pub(crate) fn unit(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// An 8-bit image as a PNG, with `channels` of each texel's four kept: 3
/// drops the alpha.
pub(crate) fn png(width: usize, height: usize, rgba: &[u8], channels: usize) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width as u32, height as u32);
        encoder.set_color(if channels == 3 { png::ColorType::Rgb } else { png::ColorType::Rgba });
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("writing to memory");
        if channels == 3 {
            let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            writer.write_image_data(&rgb).expect("writing to memory");
        } else {
            writer.write_image_data(rgba).expect("writing to memory");
        }
    }
    out
}

/// A mesh's triangles, with normals: the authored ones, or smooth ones
/// made from the faces when it has none.
pub(crate) struct Source<'a> {
    pub positions: &'a [Vec3],
    pub normals: Vec<Vec3>,
    pub uvs: Option<&'a [Vec2]>,
    pub triangles: Vec<[u32; 3]>,
}

impl<'a> Source<'a> {
    pub fn of(mesh: &'a tequila_scene::geometry::Mesh) -> Source<'a> {
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

    /// The bounding box of the positions.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        self.positions.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(l, h), p| (l.min(*p), h.max(*p)))
    }
}

/// Where each triangle is baked: its three corners in texels of a square
/// image, and whether those are the mesh's own UVs (shared, so written
/// indexed) or an atlas's (one cell each, so every corner its own vertex).
pub(crate) struct Atlas {
    pub size: usize,
    pub corners: Vec<[[f32; 2]; 3]>,
    pub own: bool,
}

/// The mesh's own UVs, when they can hold a bake: inside the unit square
/// and not folded over themselves - a model unwrapped for texturing. Tiled
/// or overlapping UVs, or none, cannot.
pub(crate) fn own_uvs(source: &Source, size: usize) -> Option<Atlas> {
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
pub(crate) fn atlas(source: &Source, size: usize) -> Atlas {
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

/// The layout a part is baked into: its own UVs when they hold a bake,
/// else an atlas, or `None` for a mesh too dense for one.
pub(crate) fn layout(source: &Source, size: usize) -> Option<Atlas> {
    own_uvs(source, size).or_else(|| (source.triangles.len() <= ATLAS_TRIANGLES).then(|| atlas(source, size)))
}

/// Call `f(x, y, barycentric)` for every texel whose centre lies inside the
/// triangle with corners `c` in texels.
pub(crate) fn raster(c: &[[f32; 2]; 3], size: usize, mut f: impl FnMut(usize, usize, [f32; 3])) {
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
pub(crate) fn dilate(images: &mut [&mut Vec<u8>], covered: &mut [bool], size: usize, passes: usize) {
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
pub(crate) struct Part<'a> {
    pub source: &'a Source<'a>,
    pub world: tequila_core::math::Transform,
    pub scale: f32,
    pub bounds: (Vec3, Vec3),
}

impl Part<'_> {
    /// The shading point at a place on triangle `t`.
    pub fn point(&self, t: usize, b: [f32; 3], footprint: f32) -> tequila_texture::projection::SurfacePoint {
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
                closeness: 0.0,
                object_color: tequila_core::spectrum::Spectrum::ONE,
                object_number: 0.0,
                ray: Default::default(),
            },
        }
    }

    /// A triangle's change in position along its UVs, `dP/du` and `dP/dv`,
    /// in the part's own space, for corners `uv` in any units.
    fn derivatives(&self, t: usize, uv: [[f32; 2]; 3]) -> Option<(Vec3, Vec3)> {
        let [a, b, c] = self.source.triangles[t].map(|i| self.source.positions[i as usize]);
        let (e1, e2) = (b - a, c - a);
        let (du1, dv1) = (uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]);
        let (du2, dv2) = (uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]);
        let det = du1 * dv2 - du2 * dv1;
        if det.abs() < 1e-20 {
            return None;
        }
        let r = 1.0 / det;
        Some(((e1 * dv2 - e2 * dv1) * r, (e2 * du1 - e1 * du2) * r))
    }
}

/// What, beyond colour and metal-roughness, a bake should make.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Extras {
    /// The bumped normal, as a tangent-space map in the layout's frame.
    pub normal: bool,
    /// The coat's weight and roughness.
    pub coat: bool,
    /// Baked occlusion, in the metal-roughness image's red.
    pub occlusion: bool,
}

impl Extras {
    /// Everything `program` drives, of what can be baked.
    pub fn driven_by(program: &CompiledGraph, textures: &TextureTable) -> Extras {
        let drives = |p: Param| program.driven.iter().any(|(q, _)| *q == p);
        Extras {
            normal: BumpSource::of(program, textures).is_some(),
            coat: drives(Param::CoatWeight) || drives(Param::CoatRoughness),
            occlusion: drives(Param::Occlusion),
        }
    }
}

/// A bake's images, `size` texels square, four bytes a texel.
pub(crate) struct Baked {
    pub size: usize,
    /// The colour (sRGB) and opacity (alpha), as the format's `look` gave
    /// them.
    pub colour: Vec<u8>,
    /// Occlusion (red, when baked, else 0), roughness (green), metalness
    /// (blue).
    pub metal_rough: Vec<u8>,
    /// The emission (sRGB), when the material drives it.
    pub glow: Option<Vec<u8>>,
    /// The coat's weight (red) and roughness (green).
    pub coat: Option<Vec<u8>>,
    /// A tangent-space normal (linear, `n * 0.5 + 0.5`).
    pub normal: Option<Vec<u8>>,
    /// Which texels a triangle covers; the rest are dilation.
    pub covered: Vec<bool>,
}

/// The texels of an image the triangles cover, if they all agree: the
/// channels `mask` names, within `tolerance`. A material that varies in
/// roughness but not colour then writes a colour, not a picture of one.
pub(crate) fn uniform(image: &[u8], covered: &[bool], mask: [bool; 4], tolerance: u8) -> Option<[u8; 4]> {
    let mut first: Option<[u8; 4]> = None;
    let (mut lo, mut hi) = ([255u8; 4], [0u8; 4]);
    for (i, _) in covered.iter().enumerate().filter(|(_, c)| **c) {
        let p: [u8; 4] = image[i * 4..i * 4 + 4].try_into().expect("four");
        first.get_or_insert(p);
        for k in 0..4 {
            if mask[k] {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
                if hi[k] - lo[k] > tolerance {
                    return None;
                }
            }
        }
    }
    first
}

/// The tangent and bitangent of each triangle's corners in `atlas`'s frame,
/// in the part's own space: green up the image, and so along decreasing
/// texel rows. Shared vertices of the mesh's own UVs average their
/// triangles' frames, as a viewer's tangent generator does; an atlas's
/// corners are each their own.
fn layout_frames(part: &Part, atlas: &Atlas) -> Vec<[(Vec3, Vec3); 3]> {
    let per_triangle: Vec<(Vec3, Vec3)> = (0..part.source.triangles.len())
        .map(|t| match part.derivatives(t, atlas.corners[t]) {
            // Rows run down the image; up it is -v.
            Some((dpdu, dpdv)) => (dpdu, -dpdv),
            None => (Vec3::ZERO, Vec3::ZERO),
        })
        .collect();
    if !atlas.own {
        return per_triangle.into_iter().map(|f| [f; 3]).collect();
    }
    let mut sums = vec![(Vec3::ZERO, Vec3::ZERO); part.source.positions.len()];
    for (t, tri) in part.source.triangles.iter().enumerate() {
        let (dt, db) = per_triangle[t];
        let (dt, db) = (dt.try_normalize().unwrap_or(Vec3::ZERO), db.try_normalize().unwrap_or(Vec3::ZERO));
        for &i in tri {
            sums[i as usize].0 = sums[i as usize].0 + dt;
            sums[i as usize].1 = sums[i as usize].1 + db;
        }
    }
    part.source.triangles.iter().map(|tri| tri.map(|i| sums[i as usize])).collect()
}

/// The bumped normal at a shaded point, in the part's own space, or `None`
/// where the material leaves the surface as it is. `main` holds the
/// registers the material's program just filled there.
#[allow(clippy::too_many_arguments)]
fn bumped(
    source: &BumpSource,
    part: &Part,
    t: usize,
    point: &tequila_texture::projection::SurfacePoint,
    texel: f32,
    main: &Registers,
    textures: &TextureTable,
    registers: &mut Registers,
    scratch: &mut Material,
) -> Option<Vec3> {
    let n = point.normal;
    // The triangle's own UV derivatives, which the material's maps follow.
    let derivatives = part.source.uvs.and_then(|uvs| {
        let tri = part.source.triangles[t];
        part.derivatives(t, tri.map(|i| [uvs[i as usize].x, uvs[i as usize].y]))
    });
    match source {
        BumpSource::NormalMap { texture, mapping, .. } if mapping.projection != tequila_texture::projection::Projection::Uv => {
            textures.get(*texture)?.sample_normal_with(mapping, point)?.try_normalize()
        }
        BumpSource::NormalMap { register, .. } => {
            let (dpdu, dpdv) = derivatives?;
            let tangent = (dpdu - n * n.dot(dpdu)).try_normalize()?;
            let down = (dpdv - n * n.dot(dpdv)).try_normalize()?;
            let frame = tequila_texture::TangentFrame { normal: n, tangent, bitangent: -down };
            let sample = main.value(*register).as_color();
            Some(frame.decode_normal_map(tequila_core::math::vec3(sample.r, sample.g, sample.b), 1.0))
        }
        BumpSource::Height { program, register } => {
            // As the renderer measures it (`surface_bump`), in world units,
            // across half a texel.
            let world_n = point.world.normal;
            let world_derivatives = derivatives.map(|(u, v)| (part.world.vector(u), part.world.vector(v)));
            let tangent = world_derivatives
                .and_then(|(dpdu, _)| (dpdu - world_n * world_n.dot(dpdu)).try_normalize())
                .unwrap_or_else(|| tequila_texture::TangentFrame::from_normal(world_n).tangent);
            let bitangent = world_n.cross(tangent);
            let step = (texel * 0.5).max(1e-6);
            let mut height = |offset: Vec3| -> f32 {
                let (du, dv) = match world_derivatives {
                    Some((dpdu, dpdv)) => uv_offset(dpdu, dpdv, offset),
                    None => (0.0, 0.0),
                };
                let moved = tequila_texture::projection::SurfacePoint {
                    position: point.position + part.world.inv_vector(offset),
                    uv: point.uv + tequila_core::math::vec2(du, dv),
                    world: tequila_texture::projection::PointWorld { position: point.world.position + offset, ..point.world },
                    ..*point
                };
                shade(program, textures, &moved, registers, scratch);
                registers.value(*register).as_float()
            };
            let along_t = (height(tangent * step) - height(tangent * -step)) / (2.0 * step);
            let along_b = (height(bitangent * step) - height(bitangent * -step)) / (2.0 * step);
            let world = (world_n - tangent * along_t - bitangent * along_b).try_normalize()?;
            part.world.inv_normal(world).try_normalize()
        }
    }
}

/// The change in UV that moves a point by `offset` along the surface: the
/// least-squares solution of `dpdu * du + dpdv * dv = offset`.
fn uv_offset(dpdu: Vec3, dpdv: Vec3, offset: Vec3) -> (f32, f32) {
    let (a, b, c) = (dpdu.dot(dpdu), dpdu.dot(dpdv), dpdv.dot(dpdv));
    let det = a * c - b * b;
    if det.abs() < 1e-20 {
        return (0.0, 0.0);
    }
    let (pu, pv) = (dpdu.dot(offset), dpdv.dot(offset));
    ((c * pu - b * pv) / det, (a * pv - b * pu) / det)
}

/// One texel's worth of every image.
type Texel = (usize, [u8; 4], [u8; 4], [u8; 4], [u8; 4], [u8; 4]);

/// Bake `program` over `part` into `atlas`. `look` turns the shaded
/// parameters into the format's colour (linear sRGB) and opacity.
pub(crate) fn bake(
    part: &Part,
    atlas: &Atlas,
    program: &CompiledGraph,
    textures: &TextureTable,
    look: fn(&Material) -> ([f32; 3], f32),
    extras: Extras,
) -> Baked {
    use rayon::prelude::*;
    let size = atlas.size;
    let emissive = program.driven.iter().any(|(p, _)| matches!(p, Param::EmissionColor | Param::EmissionLuminance));
    let bump = if extras.normal { BumpSource::of(program, textures) } else { None };
    let frames = bump.as_ref().map(|_| layout_frames(part, atlas));
    // A texel's width on the part, for the lookups that filter by it.
    let texel_world = {
        let area: f32 = part
            .source
            .triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| part.source.positions[i as usize]);
                (b - a).cross(c - a).length() * 0.5
            })
            .sum();
        (area / (size * size) as f32).sqrt() * part.scale
    };
    // Each triangle's texels, shaded where they fall; a texel two
    // triangles claim (on a shared edge of the mesh's own UVs) is written
    // by either.
    let shaded: Vec<Vec<Texel>> = (0..part.source.triangles.len())
        .into_par_iter()
        .map_init(
            || (Registers::default(), Material::default(), Registers::default(), Material::default()),
            |(registers, scratch, bump_registers, bump_scratch), t| {
                let mut out = Vec::new();
                raster(&atlas.corners[t], size, |x, y, b| {
                    let point = part.point(t, b, texel_world);
                    let m = shade(program, textures, &point, registers, scratch);
                    let (colour, alpha) = look(m);
                    let colour = [encode_srgb(colour[0]), encode_srgb(colour[1]), encode_srgb(colour[2]), unit(alpha)];
                    let occlusion = if extras.occlusion { unit(m.geometry_occlusion) } else { 0 };
                    let mr = [occlusion, unit(m.specular_roughness), unit(m.base_metalness), 255];
                    let e = srgb(m.emission_color * (m.emission_luminance / 100.0));
                    let glow = [encode_srgb(e[0]), encode_srgb(e[1]), encode_srgb(e[2]), 255];
                    let coat = [unit(m.coat_weight), unit(m.coat_roughness), 0, 255];
                    let mut normal = [128, 128, 255, 255];
                    if let (Some(source), Some(frames)) = (&bump, &frames) {
                        if let Some(nb) = bumped(source, part, t, &point, texel_world, registers, textures, bump_registers, bump_scratch) {
                            // Encoded against the layout's frame at this
                            // texel, made orthonormal about the normal.
                            let n = point.normal;
                            let corner = frames[t];
                            let along = (0..3).fold(Vec3::ZERO, |acc, k| acc + corner[k].0 * b[k]);
                            let up = (0..3).fold(Vec3::ZERO, |acc, k| acc + corner[k].1 * b[k]);
                            if let Some(tangent) = (along - n * n.dot(along)).try_normalize() {
                                let side = n.cross(tangent);
                                let bitangent = if side.dot(up) < 0.0 { -side } else { side };
                                let local = [nb.dot(tangent), nb.dot(bitangent), nb.dot(n).max(0.0)];
                                normal = [unit(local[0] * 0.5 + 0.5), unit(local[1] * 0.5 + 0.5), unit(local[2] * 0.5 + 0.5), 255];
                            }
                        }
                    }
                    out.push((y * size + x, colour, mr, glow, coat, normal));
                });
                out
            },
        )
        .collect();
    let blank = |fill: [u8; 4]| -> Vec<u8> { fill.iter().copied().cycle().take(size * size * 4).collect() };
    let mut colour = vec![0u8; size * size * 4];
    let mut metal_rough = vec![0u8; size * size * 4];
    let mut glow = if emissive { blank([0, 0, 0, 255]) } else { Vec::new() };
    let mut coat = if extras.coat { blank([0, 0, 0, 255]) } else { Vec::new() };
    let mut normal = if bump.is_some() { blank([128, 128, 255, 255]) } else { Vec::new() };
    let mut covered = vec![false; size * size];
    for (i, c, mr, g, k, n) in shaded.into_iter().flatten() {
        colour[i * 4..i * 4 + 4].copy_from_slice(&c);
        metal_rough[i * 4..i * 4 + 4].copy_from_slice(&mr);
        if emissive {
            glow[i * 4..i * 4 + 4].copy_from_slice(&g);
        }
        if extras.coat {
            coat[i * 4..i * 4 + 4].copy_from_slice(&k);
        }
        if bump.is_some() {
            normal[i * 4..i * 4 + 4].copy_from_slice(&n);
        }
        covered[i] = true;
    }
    let shaded_texels = covered.clone();
    {
        let mut images: Vec<&mut Vec<u8>> = vec![&mut colour, &mut metal_rough];
        for extra in [&mut glow, &mut coat, &mut normal] {
            if !extra.is_empty() {
                images.push(extra);
            }
        }
        dilate(&mut images, &mut covered, size, 4);
    }
    Baked {
        size,
        colour,
        metal_rough,
        glow: emissive.then_some(glow),
        coat: extras.coat.then_some(coat),
        normal: bump.is_some().then_some(normal),
        covered: shaded_texels,
    }
}

/// Bake `program` at each vertex: the colour as `look` gives it (linear
/// RGBA), and the mean metalness and roughness.
pub(crate) fn bake_vertices(part: &Part, program: &CompiledGraph, textures: &TextureTable, look: fn(&Material) -> ([f32; 3], f32)) -> (Vec<f32>, f32, f32) {
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
        let (c, alpha) = look(m);
        colours.extend_from_slice(&[c[0], c[1], c[2], alpha]);
        rough += m.specular_roughness;
        metal += m.base_metalness;
    }
    let n = on.len().max(1) as f32;
    (colours, rough / n, metal / n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uniform_image_is_found_and_a_varied_one_is_not() {
        let mut image = vec![10u8, 20, 30, 255].repeat(16);
        let covered = vec![true; 16];
        assert_eq!(uniform(&image, &covered, [true; 4], 1), Some([10, 20, 30, 255]));
        image[5 * 4 + 1] = 90;
        assert_eq!(uniform(&image, &covered, [true; 4], 1), None);
        // Only red asked about: green may vary.
        assert!(uniform(&image, &covered, [true, false, false, false], 1).is_some());
    }
}
