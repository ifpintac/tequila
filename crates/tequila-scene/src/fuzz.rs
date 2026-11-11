//! The Fuzz geometry node (`docs/02` §4.4): real fibres standing out of a
//! part's surface - a peach's skin, felt's halo, a tennis ball's nap, a
//! towel's loops - for the renders where a sheen alone is too smooth.
//!
//! Asked for by the material's own fibre settings (`fuzz_density` and the
//! rest), or by a Fuzz node in its graph, which documents from before those
//! settings carry and which wins where both are set.
//!
//! Each fibre is a curve of [`Fibres::POINTS`] points, rooted at a point
//! spread evenly over the surface by area and tapering from the thickness
//! asked to a fifth of it at the tip. It leans away from the normal by up to
//! `lean` and bends along its length by `curl`, each fibre its own way, every
//! length a little different, and draws a random number its colour is
//! darkened by. The fibres are one mesh in the part's own space, so they
//! move with the part, and they wear the part's material: the renderers
//! shade them with a fibre's BSDF in the fuzz colour and roughness.
//!
//! Where the material's Sheen Color or Sheen Roughness varies over the part
//! - a texture, a pattern, anything its graph computes - each fibre takes
//! them from where it is rooted ([`Paint`]): the graph is run at the root,
//! as the renderer runs it on the surface there, and the fibre keeps what
//! it gave. A velvet's print and a fur's stripes are in its fibres, not
//! only on the surface under them.
//!
//! Two settings make them a pile rather than a fur. `clump` gathers
//! neighbours into locks, as a hairy suede's nap mats or a sheepskin's wool
//! tufts: clumps are grown first, each with a fibre of its own, and every
//! fibre is drawn toward its nearest clump's shape, carried over to its own
//! root and narrowing evenly to the clump's tip, taking most of its shade
//! too. `comb` lays fibres the
//! same way as their neighbours, along a smooth field of directions over
//! the part that holds over a few centimetres and turns between: a little
//! for a loose pile, and for fibres gathered into locks far enough that
//! the locks fan one way and another, as a fur's do. Both draw
//! from random streams of their own, so fibres asked for without them come
//! out exactly as they did before either existed.
//!
//! Fibres are a lot of geometry, so a scene grows at most [`BUDGET`] of
//! them, all its parts together: a large part gets the density its material
//! asks for as long as the scene's fibres fit, and past that every part
//! gives up the same share of what it asked ([`shares`]), its fibres
//! widened to cover what the missing ones would have.

use rayon::prelude::*;
use tequila_core::math::{Transform, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{CompiledGraph, FuzzRequest, Instruction};
use tequila_materials::material_type::Param;
use tequila_render_core::material::Material;
use tequila_render_core::scene::{Fibres, Mesh as RenderMesh, SceneSnapshot};
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_texture::projection::{PointWorld, SurfacePoint};

/// The most fibres a scene grows, all its parts together.
///
/// Memory decides it, and Cycles most. A fibre is 84 bytes in the scene
/// (five points with a radius each, and its random number; 16 more with a
/// colour and roughness of its own), and each renderer builds its own
/// structures over it. Measured on a sofa cushion's 4.4 million against
/// the 250,000 it had before, a whole render's process held about 310
/// bytes a fibre more at its height in Tequila's renderer (the fibres, its
/// copy for Embree, and Embree's BVH over four segments) and about 820 in
/// Cycles on the processor (its own copies, packed for its kernels, and its
/// BVH). Six million are about 1.9 GB in Tequila's renderer and 4.9 GB in
/// Cycles - a third of a 16 GB computer - and grow in about a third of a
/// second, Embree's BVH over them in under three.
///
/// The old cap, 250,000 a part, was what tubes of triangles could afford
/// when fibres were first built as those (150,000 tubes of 18 triangles),
/// raised a little when they became curves; Embree has no limit near either
/// (2^32 segments a mesh).
pub const BUDGET: usize = 6_000_000;

/// What the tip's radius is of the root's.
const TIP: f32 = 0.2;

/// How many fibres `request` asks for over `area` square scene units
/// (metres).
pub fn wanted(area: f32, request: &FuzzRequest) -> f64 {
    (area.max(0.0) as f64) * 1.0e4 * (request.density().max(0.0) as f64)
}

/// How many fibres `request` grows over `area` square scene units
/// (metres) on a part alone in the scene: as many as it asks, within the
/// scene's [`BUDGET`].
pub fn count(area: f32, request: &FuzzRequest) -> usize {
    (wanted(area, request).round() as usize).min(BUDGET)
}

/// How many fibres each of a scene's parts grows, for parts asking for
/// `wanted` each: all they ask while the scene's fibres fit its [`BUDGET`],
/// and past it each the same share of what it asked, so a large part
/// thins no more than a small one.
pub fn shares(wanted: &[f64]) -> Vec<usize> {
    let total: f64 = wanted.iter().map(|w| w.max(0.0)).sum();
    let kept = if total > BUDGET as f64 { BUDGET as f64 / total } else { 1.0 };
    wanted.iter().map(|w| ((w.max(0.0) * kept).round() as usize).min(BUDGET)).collect()
}

/// A mesh's area in its own units, as fibres are spread over it: its
/// triangles' that index real vertices.
pub fn area(mesh: &RenderMesh) -> f64 {
    let mut total = 0.0f64;
    for t in mesh.indices.chunks_exact(3) {
        if t.iter().all(|&i| (i as usize) < mesh.positions.len()) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
            total += ((b - a).cross(c - a).length() * 0.5) as f64;
        }
    }
    total
}

/// A mesh's area in square scene units, for a part `scale` scene units to
/// a unit of the mesh: `area` (in the mesh's own units) measured as fibres
/// are counted.
pub fn scene_area(area: f64, scale: f32) -> f32 {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    area as f32 * scale * scale
}

/// How a part's fibres grow beyond what its material asks.
#[derive(Clone, Copy, Debug, Default)]
pub struct Growth<'a> {
    /// How many: the part's share of the scene's [`BUDGET`] ([`shares`]).
    /// `None` is as many as the material asks, as on a part alone.
    pub count: Option<usize>,
    /// What each takes from the surface where it is rooted, where the
    /// material's sheen varies over the part.
    pub paint: Option<&'a Paint<'a>>,
}

/// The fibres grown on `mesh`, in its own space; `scale` is scene units per
/// mesh unit, so lengths and densities are in scene units however the part
/// was scaled. `None` when the mesh has no area or the request no fibres.
pub fn grow(mesh: &RenderMesh, scale: f32, request: &FuzzRequest) -> Option<RenderMesh> {
    grow_with(mesh, scale, request, Growth::default())
}

/// How many random numbers one fibre draws from the part's stream: three
/// for its root, five for its shape, one for its shade. Always the same, so
/// fibre `i` starts `9 i` numbers in, and fibres can be grown apart, in
/// parallel, exactly as one after another.
const DRAWS: u64 = 9;

/// Fibres grown together, each batch from its own place in the stream.
const BATCH: usize = 4096;

/// [`grow`], at the count and with the paint `growth` gives.
pub fn grow_with(mesh: &RenderMesh, scale: f32, request: &FuzzRequest, growth: Growth) -> Option<RenderMesh> {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let triangles: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .filter(|t| t.iter().all(|&i| (i as usize) < mesh.positions.len()))
        .map(|t| [t[0], t[1], t[2]])
        .collect();
    // Each triangle's share of the area, as a running total to pick from.
    let mut running = Vec::with_capacity(triangles.len());
    let mut total = 0.0f64;
    for t in &triangles {
        let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
        total += ((b - a).cross(c - a).length() * 0.5) as f64;
        running.push(total);
    }
    // Which way is out. A closed part wound inside out, as CAD exports and
    // some glTFs are, encloses a negative volume by its winding, and an
    // import generates its normals from that winding, pointing in: taken at
    // their word, either would grow the fur on the inside. So on a closed
    // part the volume decides, and the normals and the winding are turned
    // round where they disagree with it; an open one - a panel, a sheet -
    // has no inside, and goes by its normals, or its winding.
    let (flip_winding, flip_normals) = {
        let has_normals = mesh.normals.len() == mesh.positions.len();
        let (mut volume, mut agreement) = (0.0f64, 0.0f64);
        for t in &triangles {
            let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
            volume += a.dot(b.cross(c)) as f64;
            if has_normals {
                let n = t.iter().fold(Vec3::ZERO, |n, &i| n + mesh.normals[i as usize]);
                agreement += (b - a).cross(c - a).dot(n) as f64;
            }
        }
        // Six times the enclosed volume; a closed part's is a fair share of
        // its box's, an open one's an accident of where the origin is.
        let size = mesh.bounds().diagonal();
        let closed = volume.abs() / 6.0 > 0.01 * (size.x as f64 * size.y as f64 * size.z as f64).max(1e-30);
        let inside_out = closed && volume < 0.0;
        (inside_out, closed && has_normals && (agreement < 0.0) != inside_out)
    };
    let area = scene_area(total, scale);
    let asked = wanted(area, request).round();
    let n = growth.count.unwrap_or_else(|| count(area, request));
    let length = request.length() / scale;
    let radius = request.thickness() / scale;
    if n == 0 || total <= 0.0 || !(length > 0.0) || !(radius > 0.0) {
        return None;
    }
    // Fewer than asked, where the scene's budget is spent: each is widened
    // by as many times as it is fewer, so the pile covers the surface and
    // stops the light as the fibres asked for would. Cook et al.'s
    // stochastic simplification of aggregate detail (2007), which keeps the
    // total area of what is left.
    let radius = if (n as f64) < asked { radius * (asked / n as f64) as f32 } else { radius };

    let start = SplitMix(u64::from(request.seed) ^ 0x5eed_f022);
    let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let clump = unit(request.clump());
    let comb = unit(request.comb());
    let surface = Surface { mesh, triangles: &triangles, running: &running, total, flip_winding, flip_normals };
    let shape = Shape {
        lean: request.lean().clamp(0.0, 1.0),
        curl: request.curl().clamp(0.0, 1.0),
        length,
        comb,
        field: (comb > 0.0).then(|| CombField::new(request.seed, COMB_PATCH / scale, clump)),
    };
    // The clumps' own fibres, each the one its clump gathers to. Drawn from
    // a stream of their own, so fibres asked for without clumping, or
    // without combing, come out exactly as they always have.
    let clumps = (clump > 0.0).then(|| Clumps::grow(&surface, &shape, n, request.seed));
    let paint = growth.paint;
    let mut fibres = Fibres {
        points: vec![Vec3::ZERO; n * Fibres::POINTS],
        radii: vec![0.0; n * Fibres::POINTS],
        random: vec![0.0; n],
        colors: if paint.is_some_and(|p| p.colour) { vec![Spectrum::ZERO; n] } else { Vec::new() },
        roughness: if paint.is_some_and(|p| p.roughness) { vec![0.0; n] } else { Vec::new() },
    };
    let batches = n.div_ceil(BATCH);
    let colours = in_batches(&mut fibres.colors, batches);
    let roughnesses = in_batches(&mut fibres.roughness, batches);
    fibres
        .points
        .par_chunks_mut(BATCH * Fibres::POINTS)
        .zip(fibres.radii.par_chunks_mut(BATCH * Fibres::POINTS))
        .zip(fibres.random.par_chunks_mut(BATCH))
        .zip(colours.into_par_iter().zip(roughnesses.into_par_iter()))
        .enumerate()
        .for_each(|(batch, (((points, radii), shades), (mut colours, mut roughnesses)))| {
            let mut random = start.skipped(DRAWS * (batch * BATCH) as u64);
            let mut shading = paint.map(|_| (Registers::new(), Material::default()));
            for (index, slot) in shades.iter_mut().enumerate() {
                let root = surface.sample(&mut random);
                let mut fibre = shape.fibre(&mut random, root.at, root.normal);
                // Gathered into the nearest clump: its own fibre's shape,
                // carried over to this root and narrowing evenly to its tip,
                // so a clump is a lock that tapers, not a tuft pinched in at
                // its base. The root stays where it grew.
                let mut theirs = None;
                if let Some(guide) = clumps.as_ref().and_then(|c| c.nearest(root.at)) {
                    let offset = root.at - guide.points[0];
                    for (point, at) in fibre.iter_mut().enumerate().skip(1) {
                        let along = point as f32 / Fibres::SEGMENTS as f32;
                        let lock = guide.points[point] + offset * (1.0 - along);
                        *at = *at + (lock - *at) * clump;
                    }
                    theirs = Some(guide.random);
                }
                for (point, at) in fibre.iter().enumerate() {
                    let along = point as f32 / Fibres::SEGMENTS as f32;
                    points[index * Fibres::POINTS + point] = *at;
                    radii[index * Fibres::POINTS + point] = radius * (1.0 - (1.0 - TIP) * along);
                }
                // A clump's fibres share most of its shade, so a clump reads
                // as one lock, lighter or darker than the next.
                let own = random.unit();
                *slot = match theirs {
                    Some(theirs) => own + (theirs - own) * clump * 0.75,
                    None => own,
                };
                if let (Some(paint), Some((registers, scratch))) = (paint, shading.as_mut()) {
                    let (colour, roughness) = paint.at(&surface, &root, scale, radius, registers, scratch);
                    if let Some(c) = colours.as_deref_mut() {
                        c[index] = colour;
                    }
                    if let Some(r) = roughnesses.as_deref_mut() {
                        r[index] = roughness;
                    }
                }
            }
        });
    Some(RenderMesh::of_fibres(fibres))
}

/// A batch's share of `values` - a colour or a roughness each fibre - or
/// none for every batch where the fibres keep none.
fn in_batches<T>(values: &mut [T], batches: usize) -> Vec<Option<&mut [T]>> {
    if values.is_empty() {
        (0..batches).map(|_| None).collect()
    } else {
        values.chunks_mut(BATCH).map(Some).collect()
    }
}

/// What fibres take from the surface where they are rooted, where the
/// material's Sheen Color or Sheen Roughness varies over the part: its
/// graph cut down to those two, run at each root as the renderer runs it on
/// the surface there - at the root's UV and position, its normal, vertex
/// colour, curvature and occlusion, the part's place in the world - so a
/// fibre has the colour, in the rendering space, and the roughness the
/// surface has under it, a texture's colour converted from its picture's as
/// the surface's own lookups convert it.
///
/// The graph is read as the camera sees the surface straight on: a sheen
/// that changes with the angle it is seen at (Layer Weight, Fresnel, the
/// Light Path) is fixed at a fibre to what it is seen square on.
pub struct Paint<'a> {
    program: CompiledGraph,
    colour: bool,
    roughness: bool,
    textures: &'a TextureTable,
    transform: Transform,
    object_random: f32,
    /// The graph reads where the part is - its world position or normal,
    /// its origin, its random number - so its fibres depend on that too.
    reads_world: bool,
}

impl std::fmt::Debug for Paint<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Paint")
            .field("colour", &self.colour)
            .field("roughness", &self.roughness)
            .field("reads_world", &self.reads_world)
            .finish_non_exhaustive()
    }
}

impl<'a> Paint<'a> {
    /// What the fibres of a part wearing `program` take from it, the part
    /// instance `instance` of its scene, placed by `transform`, its textures
    /// in `textures`. `None` when the graph leaves both the Sheen Color and
    /// the Sheen Roughness constant, and every fibre wears the material's.
    pub fn of(program: &CompiledGraph, textures: &'a TextureTable, transform: Transform, instance: usize) -> Option<Paint<'a>> {
        let drives = |param: Param| program.driven.iter().any(|(p, _)| *p == param);
        let (colour, roughness) = (drives(Param::FuzzColor), drives(Param::FuzzRoughness));
        if !colour && !roughness {
            return None;
        }
        let wanted: Vec<Param> = [(colour, Param::FuzzColor), (roughness, Param::FuzzRoughness)]
            .into_iter()
            .filter_map(|(driven, param)| driven.then_some(param))
            .collect();
        let mut program = program.restricted_to(&wanted);
        // Labels print on the base colour, and a mixed-in surface is a
        // layer of the surface, not of the fibres, which wear the
        // material's own.
        program.decals.clear();
        program.layer = None;
        use tequila_materials::nodes::PointInput;
        let reads_world = program.instructions.iter().any(|instruction| {
            matches!(instruction, Instruction::Input { input, .. }
                if !matches!(input, PointInput::Object | PointInput::Generated | PointInput::Uv | PointInput::ObjectNormal))
        });
        Some(Paint { program, colour, roughness, textures, transform, object_random: SceneSnapshot::object_random(instance), reads_world })
    }

    /// Whether the fibres take their colour, and their roughness.
    pub fn takes(&self) -> (bool, bool) {
        (self.colour, self.roughness)
    }

    /// What decides the fibres' colours: the graph's instructions for them,
    /// the pictures they read, and, if the graph reads it, where the part
    /// is. Two paints alike here paint alike.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", self.program.instructions).hash(&mut hasher);
        format!("{:?}", self.program.driven).hash(&mut hasher);
        for instruction in &self.program.instructions {
            if let Instruction::Sample { texture, .. } | Instruction::SampleAlpha { texture, .. } = instruction {
                // The picture itself, as loaded: a texture read again from a
                // changed file is another.
                self.textures.get(*texture).map(|t| std::sync::Arc::as_ptr(t) as usize).hash(&mut hasher);
            }
        }
        if self.reads_world {
            for row in self.transform.matrix.m {
                for v in row {
                    v.to_bits().hash(&mut hasher);
                }
            }
            self.object_random.to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// The colour and roughness at `root`, on a part `scale` scene units to
    /// a unit of its mesh, sampled across `footprint` mesh units: a fibre's
    /// own width, so it takes the colour where it stands rather than an
    /// average of its neighbourhood, and many fibres together average the
    /// surface's.
    fn at(&self, surface: &Surface, root: &Root, scale: f32, footprint: f32, registers: &mut Registers, scratch: &mut Material) -> (Spectrum, f32) {
        let mesh = surface.mesh;
        let [ia, ib, ic] = surface.triangles[root.triangle].map(|i| i as usize);
        let (u, v) = (root.u, root.v);
        let w = 1.0 - u - v;
        let has = |len: usize| len == mesh.positions.len();
        // The surface's own coordinates at the root, as a hit there reads
        // them: its UVs interpolated, or the triangle's own without.
        let uv = if has(mesh.uvs.len()) { mesh.uvs[ia] * w + mesh.uvs[ib] * u + mesh.uvs[ic] * v } else { Vec2 { x: u, y: v } };
        let uv_footprint = has(mesh.uvs.len()).then(|| {
            let (p0, p1, p2) = (mesh.positions[ia], mesh.positions[ib], mesh.positions[ic]);
            let (d1, d2) = (mesh.uvs[ib] - mesh.uvs[ia], mesh.uvs[ic] - mesh.uvs[ia]);
            let det = d1.x * d2.y - d1.y * d2.x;
            let (e1, e2) = (p1 - p0, p2 - p0);
            let (lu, lv) = if det.abs() < 1e-20 || !det.is_finite() {
                (0.0, 0.0)
            } else {
                (((e1 * d2.y - e2 * d1.y) / det).length(), ((e2 * d1.x - e1 * d2.x) / det).length())
            };
            (lu > 1e-12 && lv > 1e-12).then(|| [footprint / lu, footprint / lv])
        });
        let mixed = |values: &[f32]| if has(values.len()) { values[ia] * w + values[ib] * u + values[ic] * v } else { 0.0 };
        let vertex_color = if has(mesh.colors.len()) {
            let c = mesh.colors[ia] * w + mesh.colors[ib] * u + mesh.colors[ic] * v;
            Spectrum::from_linear_srgb(tequila_core::math::vec3(c.x, c.y, c.z))
        } else {
            Spectrum::ONE
        };
        let world_normal = self.transform.normal(root.normal).try_normalize().unwrap_or(root.normal);
        let bounds = mesh.bounds();
        let centre = (bounds.min + bounds.max) * 0.5;
        let half = (bounds.max - bounds.min) * 0.5;
        let size = |h: f32| if h == 0.0 { 1.0 } else { h.max(1e-5) };
        let point = SurfacePoint {
            position: root.at,
            normal: root.normal,
            uv,
            footprint,
            scale,
            uv_footprint: uv_footprint.flatten(),
            // In inverse scene units, as a hit's.
            curvature: mixed(&mesh.curvature) / scale,
            occlusion: mixed(&mesh.occlusion),
            vertex_color,
            world: PointWorld {
                position: self.transform.point(root.at),
                normal: world_normal,
                true_normal: self.transform.normal(root.face).try_normalize().unwrap_or(world_normal),
                incoming: world_normal,
                backfacing: false,
                generated: (root.at - centre) / tequila_core::math::vec3(size(half.x), size(half.y), size(half.z)) * 0.5 + Vec3::splat(0.5),
                object_location: self.transform.point(Vec3::ZERO),
                object_random: self.object_random,
                closeness: 0.0,
                object_color: tequila_core::spectrum::Spectrum::ONE,
                object_number: 0.0,
                ray: Default::default(),
            },
        };
        let shaded = shade(&self.program, self.textures, &point, registers, scratch);
        (shaded.fuzz_color, shaded.fuzz_roughness)
    }
}

/// How wide a clump is, as a share of the fibres' length: a lock of
/// hairy suede about half as wide as it is long.
const CLUMP_RADIUS: f32 = 0.45;

/// How wide a combed patch is, in scene units (metres): the nap of a hide
/// lies one way over a few centimetres, then turns.
const COMB_PATCH: f32 = 0.04;

/// The most clumps one part has, past which they are larger than asked:
/// for up to 250,000 fibres, and in proportion past that.
const MOST_CLUMPS: usize = 60_000;

/// The mesh, for spreading roots over it evenly by area.
struct Surface<'a> {
    mesh: &'a RenderMesh,
    triangles: &'a [[u32; 3]],
    running: &'a [f64],
    total: f64,
    flip_winding: bool,
    flip_normals: bool,
}

/// Where a fibre is rooted: the point, which way is out there, the
/// triangle it is on and where on it.
struct Root {
    at: Vec3,
    normal: Vec3,
    /// The triangle's own normal, out.
    face: Vec3,
    triangle: usize,
    u: f32,
    v: f32,
}

impl Surface<'_> {
    /// A point on the surface, evenly by area, and which way is out there.
    fn sample(&self, random: &mut SplitMix) -> Root {
        let mesh = self.mesh;
        let pick = random.unit() as f64 * self.total;
        let index = self.running.partition_point(|&r| r < pick).min(self.triangles.len() - 1);
        let [ia, ib, ic] = self.triangles[index];
        let (a, b, c) = (mesh.positions[ia as usize], mesh.positions[ib as usize], mesh.positions[ic as usize]);
        let (mut u, mut v) = (random.unit(), random.unit());
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        let root = a + (b - a) * u + (c - a) * v;
        let face = if self.flip_winding { (c - a).cross(b - a) } else { (b - a).cross(c - a) };
        let normal = if mesh.normals.len() == mesh.positions.len() {
            let n = mesh.normals[ia as usize] * (1.0 - u - v) + mesh.normals[ib as usize] * u + mesh.normals[ic as usize] * v;
            let n = if self.flip_normals { -n } else { n };
            n.try_normalize().unwrap_or_else(|| face.normalize())
        } else {
            face.try_normalize().unwrap_or(Vec3::Y)
        };
        Root { at: root, normal, face: face.try_normalize().unwrap_or(normal), triangle: index, u, v }
    }
}

/// How a fibre stands, leans and bends, in mesh units.
struct Shape {
    lean: f32,
    curl: f32,
    length: f32,
    comb: f32,
    field: Option<CombField>,
}

impl Shape {
    /// One fibre's points, root to tip.
    fn fibre(&self, random: &mut SplitMix, root: Vec3, normal: Vec3) -> [Vec3; Fibres::POINTS] {
        let segments = Fibres::SEGMENTS;
        let (t, s) = perpendiculars(normal);
        // Leaning a random way, by up to `lean` of a near-flat 80 degrees;
        // combed, mostly the way the nap lies here, and laid down evenly,
        // nearly as far as `lean` says rather than anywhere up to it.
        let azimuth = random.unit() * std::f32::consts::TAU;
        let spread = 0.35 + 0.65 * random.unit();
        let spread = spread + (1.0 - spread) * self.comb * 0.8;
        let tilt = self.lean * spread * 80f32.to_radians();
        let mut sideways = t * azimuth.cos() + s * azimuth.sin();
        if let Some(along) = self.field.as_ref().and_then(|f| f.along(root, normal)) {
            sideways = (sideways * (1.0 - self.comb) + along * self.comb).try_normalize().unwrap_or(along);
        }
        let mut direction = (normal * tilt.cos() + sideways * tilt.sin()).normalize();
        // Bending about an axis across the fibre, a little more each step,
        // the axis itself turning a little so a curl is not flat.
        let mut bend_axis = normal.cross(sideways).try_normalize().unwrap_or(t);
        let bend = self.curl * (0.4 + 0.6 * random.unit()) * 35f32.to_radians();
        let twist = self.curl * (random.unit() * 2.0 - 1.0) * 40f32.to_radians();
        let step = self.length * (0.7 + 0.6 * random.unit()) / segments as f32;

        let mut points = [Vec3::ZERO; Fibres::POINTS];
        let mut at = root;
        for point in points.iter_mut() {
            *point = at;
            at = at + direction * step;
            direction = rotate(direction, bend_axis, bend).normalize();
            bend_axis = rotate(bend_axis, direction, twist).normalize();
        }
        points
    }
}

/// Which way the nap lies across a part: a smooth field of directions, a
/// steady drift with a few long waves over it, so it holds one way over a
/// patch and turns between patches. How far it turns follows the clumping.
///
/// A loose pile's nap turns only a little. Its drift runs along the part's
/// diagonal. Where it stands straight out of the surface the waves alone
/// are left, and turn the nap round in whorls; along the diagonal, that is
/// at a corner of a part modelled along its axes, never across a face - not
/// a top, whichever way is up, and not a side. On a suede's short nap a
/// sharper turn reads as scales.
///
/// Locks splay. Gathered into them, the nap drifts a way of its own and the
/// waves are as strong as the drift, so the locks fan one way over a patch
/// and another over the next, and whorl where the drift stands out of the
/// surface: a fur's coat. Between [`LOOSE`] and [`LOCKED`] clumping the two
/// are mixed; outside it each is exactly itself.
struct CombField {
    /// The loose pile's waves, about [`DRIFT`].
    waves: [Wave; 4],
    /// The locks' drift and waves, and how much of the nap is theirs.
    locks: Option<(Vec3, [Wave; 4], f32)>,
}

/// One of the field's waves: which way and how fast it runs, its phase,
/// and which way and how far it turns the nap.
type Wave = (Vec3, f32, Vec3);

/// A loose pile's drift, a unit vector along the diagonal.
const DRIFT: Vec3 = tequila_core::math::vec3(0.577_350_3, 0.577_350_3, 0.577_350_3);

/// How far a loose pile's waves turn its nap from the drift: a third of it,
/// so a patch's nap wanders by a few tens of degrees and never doubles back.
const WANDER: f32 = 0.3;

/// Clumping up to which the nap is a loose pile's: a hairy suede's.
const LOOSE: f32 = 0.4;

/// Clumping from which the nap is all locks: a fur's.
const LOCKED: f32 = 0.7;

impl CombField {
    /// Patches `patch` mesh units across, for fibres clumped by `clump`.
    fn new(seed: u32, patch: f32, clump: f32) -> CombField {
        // Each from the start of the same stream, so a loose pile combs as it
        // did before locks were told apart, and locks as they did before.
        let stream = || SplitMix(u64::from(seed) ^ 0xc0_4b_f1e1d);
        let frequency = std::f32::consts::TAU / patch.max(1e-9);
        let waves = Self::waves(&mut stream(), frequency, WANDER);
        let t = ((clump - LOOSE) / (LOCKED - LOOSE)).clamp(0.0, 1.0);
        let share = t * t * (3.0 - 2.0 * t);
        let locks = (share > 0.0).then(|| {
            let mut random = stream();
            let drift = Self::direction(&mut random) * 1.2;
            (drift, Self::waves(&mut random, frequency, 1.0), share)
        });
        CombField { waves, locks }
    }

    /// A direction, uniform on the sphere.
    fn direction(random: &mut SplitMix) -> Vec3 {
        let z = random.unit() * 2.0 - 1.0;
        let angle = random.unit() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        tequila_core::math::vec3(r * angle.cos(), r * angle.sin(), z)
    }

    /// Four waves running random ways `frequency` radians a mesh unit, each
    /// turning the nap a random way by `wander` of a unit drift.
    fn waves(random: &mut SplitMix, frequency: f32, wander: f32) -> [Wave; 4] {
        std::array::from_fn(|_| {
            let k = Self::direction(random) * frequency;
            let phase = random.unit() * std::f32::consts::TAU;
            (k, phase, Self::direction(random) * wander)
        })
    }

    /// The way the nap lies at `p`, along the surface; `None` where the
    /// field stands straight out of it.
    fn along(&self, p: Vec3, normal: Vec3) -> Option<Vec3> {
        let field = |drift: Vec3, waves: &[Wave; 4]| {
            let mut f = drift;
            for (k, phase, amplitude) in waves {
                f = f + *amplitude * (k.dot(p) + phase).sin();
            }
            f
        };
        let f = match &self.locks {
            None => field(DRIFT, &self.waves),
            Some((drift, waves, share)) if *share >= 1.0 => field(*drift, waves),
            Some((drift, waves, share)) => field(DRIFT, &self.waves) * (1.0 - share) + field(*drift, waves) * *share,
        };
        let flat = f - normal * normal.dot(f);
        (flat.length() > 1e-3 * f.length()).then(|| flat.normalize())
    }
}

/// A clump's own fibre, which the fibres around it gather to.
struct Guide {
    points: [Vec3; Fibres::POINTS],
    random: f32,
}

/// The clumps over a part, and a grid to find the nearest by.
struct Clumps {
    cell: f32,
    guides: Vec<Guide>,
    grid: std::collections::HashMap<(i32, i32, i32), Vec<u32>>,
}

impl Clumps {
    /// Clumps [`CLUMP_RADIUS`] of a fibre's length across, spread evenly
    /// by area; at least four fibres each and at most [`MOST_CLUMPS`] for
    /// every 250,000 fibres.
    fn grow(surface: &Surface, shape: &Shape, fibres: usize, seed: u32) -> Clumps {
        let radius = (CLUMP_RADIUS * shape.length).max(1e-9);
        let wanted = surface.total / (std::f64::consts::PI * (radius as f64).powi(2));
        let most = MOST_CLUMPS.max(fibres * 6 / 25);
        let count = (wanted.round() as usize).clamp(1, (fibres / 4).max(1)).min(most);
        // Fewer clumps than wanted are larger: found within their own size.
        let radius = radius * (wanted / count as f64).max(1.0).sqrt() as f32;
        let mut random = SplitMix(u64::from(seed) ^ 0xc10b_5eed);
        let mut clumps = Clumps { cell: 2.0 * radius, guides: Vec::with_capacity(count), grid: Default::default() };
        for index in 0..count {
            let root = surface.sample(&mut random);
            let points = shape.fibre(&mut random, root.at, root.normal);
            clumps.guides.push(Guide { points, random: random.unit() });
            clumps.grid.entry(clumps.key(root.at)).or_default().push(index as u32);
        }
        clumps
    }

    fn key(&self, p: Vec3) -> (i32, i32, i32) {
        let k = |x: f32| (x / self.cell).floor() as i32;
        (k(p.x), k(p.y), k(p.z))
    }

    /// The clump whose root is nearest `p`, if one is within a cell of it.
    fn nearest(&self, p: Vec3) -> Option<&Guide> {
        let (x, y, z) = self.key(p);
        let mut best: Option<(f32, u32)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &index in self.grid.get(&(x + dx, y + dy, z + dz)).into_iter().flatten() {
                        let d = (self.guides[index as usize].points[0] - p).length_squared();
                        if best.is_none_or(|(b, _)| d < b) {
                            best = Some((d, index));
                        }
                    }
                }
            }
        }
        best.map(|(_, index)| &self.guides[index as usize])
    }
}

/// Two unit vectors perpendicular to `n` and to each other.
fn perpendiculars(n: Vec3) -> (Vec3, Vec3) {
    let helper = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let t = n.cross(helper).normalize();
    (t, n.cross(t))
}

/// `v` turned `angle` radians about the unit `axis` (Rodrigues).
fn rotate(v: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let (s, c) = angle.sin_cos();
    v * c + axis.cross(v) * s + axis * (axis.dot(v) * (1.0 - c))
}

/// The fibres' random numbers: SplitMix64, whose state only counts up, so
/// a stream can be entered anywhere along it.
#[derive(Clone, Copy)]
struct SplitMix(u64);

/// What SplitMix64 adds to its state for each number.
const GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;

impl SplitMix {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(GAMMA);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }

    /// The stream `draws` numbers further on, as if they had been drawn.
    fn skipped(self, draws: u64) -> SplitMix {
        SplitMix(self.0.wrapping_add(draws.wrapping_mul(GAMMA)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(length: f32, density: f32, lean: f32) -> FuzzRequest {
        request_with(length, density, lean, 0.3)
    }

    fn request_with(length: f32, density: f32, lean: f32, curl: f32) -> FuzzRequest {
        FuzzRequest {
            length_bits: length.to_bits(),
            density_bits: density.to_bits(),
            thickness_bits: 0.00002f32.to_bits(),
            lean_bits: lean.to_bits(),
            curl_bits: curl.to_bits(),
            clump_bits: 0f32.to_bits(),
            comb_bits: 0f32.to_bits(),
            seed: 3,
        }
    }

    fn clumped(length: f32, density: f32, lean: f32, clump: f32, comb: f32) -> FuzzRequest {
        FuzzRequest { clump_bits: clump.to_bits(), comb_bits: comb.to_bits(), ..request(length, density, lean) }
    }

    /// For a sample of fibres, the fibre whose root is nearest each one's.
    fn neighbours(fibres: &Fibres, sample: usize) -> Vec<(usize, usize)> {
        let roots: Vec<Vec3> = fibres.points.chunks_exact(Fibres::POINTS).map(|f| f[0]).collect();
        let step = (roots.len() / sample).max(1);
        (0..roots.len())
            .step_by(step)
            .map(|i| {
                let j = (0..roots.len())
                    .filter(|&j| j != i)
                    .min_by(|&a, &b| (roots[a] - roots[i]).length_squared().total_cmp(&(roots[b] - roots[i]).length_squared()))
                    .unwrap();
                (i, j)
            })
            .collect()
    }

    /// Clumping gathers neighbours' tips together, more the more it is
    /// asked, while their roots stay where they grew and nothing dips into
    /// the part: the tips of fibres side by side end far apart unclumped,
    /// close together clumped.
    #[test]
    fn fibres_gather_into_clumps_as_asked() {
        let sphere = crate::demo::uv_sphere(1.0, 64, 32);
        // Tips' spread over roots' spread, between neighbours.
        let spread = |clump: f32| {
            let fur = grow(&sphere, 0.02, &clumped(0.002, 300.0, 0.7, clump, 0.0)).expect("fibres");
            let fibres = fibres_of(&fur);
            let at = |i: usize, k: usize| fibres.points[i * Fibres::POINTS + k];
            let (mut tips, mut roots) = (0.0f32, 0.0f32);
            for (i, j) in neighbours(fibres, 400) {
                roots += (at(i, 0) - at(j, 0)).length();
                tips += (at(i, Fibres::POINTS - 1) - at(j, Fibres::POINTS - 1)).length();
            }
            for fibre in fibres.points.chunks_exact(Fibres::POINTS) {
                assert!((fibre[0].length() - 1.0).abs() < 0.01, "a root off the surface: {:?}", fibre[0]);
                assert!(fibre.iter().all(|p| p.length() >= fibre[0].length() - 1e-4), "a fibre dips into the part");
            }
            tips / roots
        };
        let (apart, loose, tight) = (spread(0.0), spread(0.5), spread(1.0));
        assert!(apart > 2.0, "unclumped neighbours' tips {apart} times as far apart as their roots");
        assert!(loose < 0.75 * apart && tight < 0.6 * loose, "clumping 0, 0.5, 1: tips {apart}, {loose}, {tight} times their roots");
    }

    /// Combed fibres lie the same way as their neighbours; uncombed, each
    /// its own way.
    #[test]
    fn combed_fibres_lie_the_same_way() {
        let sphere = crate::demo::uv_sphere(1.0, 64, 32);
        // How alike neighbours' leans are along the surface, 1 the same way.
        let agreement = |comb: f32| {
            let fur = grow(&sphere, 0.05, &clumped(0.002, 30.0, 0.8, 0.0, comb)).expect("fibres");
            let fibres = fibres_of(&fur);
            let lean = |i: usize| {
                let (root, next) = (fibres.points[i * Fibres::POINTS], fibres.points[i * Fibres::POINTS + 1]);
                let n = root.normalize();
                let d = next - root;
                (d - n * n.dot(d)).normalize()
            };
            let pairs = neighbours(fibres, 400);
            pairs.iter().map(|&(i, j)| lean(i).dot(lean(j))).sum::<f32>() / pairs.len() as f32
        };
        let (free, combed) = (agreement(0.0), agreement(1.0));
        assert!(free.abs() < 0.15, "uncombed neighbours agree by {free}");
        assert!(combed > 0.85, "combed neighbours agree by only {combed}");
    }

    /// A loose pile's nap holds one way across a top several patches wide;
    /// locks fan one way and another over it.
    #[test]
    fn locks_fan_out_where_a_loose_pile_lies_one_way() {
        // How well the nap over a flat top six patches across agrees with
        // itself, as the length of its mean direction, over a few seeds.
        let agreement = |clump: f32| {
            let mut total = 0.0;
            for seed in 0..8 {
                let field = CombField::new(seed, 1.0, clump);
                let (mut sum, mut n) = (Vec3::ZERO, 0);
                for i in 0..24 {
                    for j in 0..24 {
                        let p = tequila_core::math::vec3(i as f32 * 0.25, 0.0, j as f32 * 0.25);
                        if let Some(along) = field.along(p, Vec3::Y) {
                            sum = sum + along;
                            n += 1;
                        }
                    }
                }
                total += sum.length() / n.max(1) as f32;
            }
            total / 8.0
        };
        let (loose, locked) = (agreement(0.3), agreement(0.7));
        assert!(loose > 0.9, "a loose pile's nap agrees by only {loose}");
        assert!(locked < loose - 0.2, "locks agree by {locked}, a loose pile by {loose}");
    }

    /// Up to `LOOSE` clumping the nap is exactly a loose pile's, as it was
    /// before locks were told apart, so a hairy suede's renders keep; from
    /// `LOCKED` it is exactly the locks', so a fur's do.
    #[test]
    fn a_loose_pile_and_locks_each_keep_their_own_nap() {
        let nap = |clump: f32| {
            let field = CombField::new(3, 0.5, clump);
            (0..64)
                .map(|i| {
                    let p = tequila_core::math::vec3(i as f32 * 0.37, (i % 5) as f32 * 0.11, i as f32 * -0.21);
                    field.along(p, Vec3::Y)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(nap(0.0), nap(LOOSE));
        assert_eq!(nap(LOCKED), nap(1.0));
        assert_ne!(nap(LOOSE), nap(LOCKED));
        assert_ne!(nap(0.55), nap(LOOSE));
        assert_ne!(nap(0.55), nap(LOCKED));
    }

    /// Fibres asked for without clumping or combing grow exactly as they did
    /// before either existed: the same roots, the same shapes, the same
    /// shades, so no saved render changes.
    #[test]
    fn unclumped_uncombed_fibres_are_unchanged() {
        let sphere = crate::demo::uv_sphere(1.0, 32, 16);
        let fur = grow(&sphere, 1.0, &request_with(0.05, 0.5, 0.6, 0.4)).expect("fibres");
        let fibres = fibres_of(&fur);
        // The first fibre and shade, as the generator drew them before
        // clumping: its own stream, root, lean and bend, in that order.
        let mut random = SplitMix(3u64 ^ 0x5eed_f022);
        let mut draws: Vec<f32> = (0..9).map(|_| random.unit()).collect();
        let shade = draws.pop().unwrap();
        assert_eq!(fibres.random[0], shade);
        let root = fibres.points[0];
        assert!((root.length() - 1.0).abs() < 0.01);
    }

    fn fibres_of(mesh: &RenderMesh) -> &Fibres {
        mesh.fibres.as_deref().expect("a mesh of fibres")
    }

    /// As many fibres as the density asks over the area, in scene units -
    /// the same on a part scaled down as on one modelled small - and never
    /// more than the scene's budget.
    #[test]
    fn fibres_follow_the_density_and_the_part_scale() {
        let sphere = crate::demo::uv_sphere(1.0, 48, 24);
        let area_m2 = 4.0 * std::f32::consts::PI * 0.025 * 0.025;
        // A 2.5 cm sphere modelled at 1 unit, scaled by 0.025.
        let fur = grow(&sphere, 0.025, &request(0.002, 200.0, 0.3)).expect("fibres");
        let fibres = fibres_of(&fur);
        assert_eq!(fibres.points.len(), fibres.count() * Fibres::POINTS);
        let expected = area_m2 * 1.0e4 * 200.0;
        assert!((fibres.count() as f32 / expected - 1.0).abs() < 0.03, "{} fibres for {expected}", fibres.count());
        assert_eq!(count(1000.0, &request(0.002, 400.0, 0.3)), BUDGET);
        // The old cap's worth and more, as asked: a sofa's cushion of
        // velvet, three quarters of a square metre at 600 a square
        // centimetre.
        assert_eq!(count(0.75, &request(0.002, 600.0, 0.3)), 4_500_000);
        // Fibres in place of triangles: nothing that reads triangles sees
        // any.
        assert_eq!(fur.triangle_count(), 0);
        assert!(!fur.bounds().is_empty());
    }

    /// Fibres stand out of the surface, rooted on it, about as long as
    /// asked: upright with no lean, never into the part.
    #[test]
    fn fibres_stand_out_of_the_surface() {
        let sphere = crate::demo::uv_sphere(1.0, 48, 24);
        let fur = grow(&sphere, 1.0, &request(0.05, 2.0, 0.0)).expect("fibres");
        for fibre in fibres_of(&fur).points.chunks_exact(Fibres::POINTS).take(500) {
            let (root, tip) = (fibre[0], fibre[Fibres::POINTS - 1]);
            assert!((root.length() - 1.0).abs() < 0.01, "a root off the surface: {root:?}");
            let rise = tip.length() - root.length();
            assert!(rise > 0.03 && rise < 0.07, "a fibre rising {rise}");
            assert!(fibre.iter().all(|p| p.length() >= root.length() - 1e-4), "a fibre dips into the part");
        }
    }

    /// The thickness asked is the fibre's width at its root, in scene
    /// units however the part is scaled, tapering to a fifth at the tip.
    #[test]
    fn fibres_taper_from_the_thickness_asked() {
        let sphere = crate::demo::uv_sphere(1.0, 24, 12);
        let fur = grow(&sphere, 0.5, &request(0.05, 2.0, 0.3)).expect("fibres");
        let fibres = fibres_of(&fur);
        // 20 microns at the root in scene units, 40 in the part's own half
        // scale.
        let radii = &fibres.radii[..Fibres::POINTS];
        assert!((radii[0] - 0.00004).abs() < 1e-9, "root radius {}", radii[0]);
        assert!((radii[Fibres::POINTS - 1] - 0.00004 * TIP).abs() < 1e-9);
        assert!(radii.windows(2).all(|w| w[1] < w[0]), "not tapering: {radii:?}");
        assert!(fibres.random.iter().all(|r| (0.0..1.0).contains(r)));
    }

    /// Lean tips fibres over from the normal, and curl bends them, each by
    /// about what it says and not at all without it.
    #[test]
    fn fibres_lean_and_curl_as_asked() {
        let sphere = crate::demo::uv_sphere(1.0, 48, 24);
        // The mean angle a fibre's first segment makes with the surface
        // normal at its root, and how much its direction turns root to tip.
        let measure = |lean: f32, curl: f32| {
            let fur = grow(&sphere, 1.0, &request_with(0.05, 2.0, lean, curl)).expect("fibres");
            let (mut tilt, mut turn, mut n) = (0.0f32, 0.0f32, 0.0f32);
            for fibre in fibres_of(&fur).points.chunks_exact(Fibres::POINTS) {
                let first = (fibre[1] - fibre[0]).normalize();
                let last = (fibre[Fibres::POINTS - 1] - fibre[Fibres::POINTS - 2]).normalize();
                tilt += first.dot(fibre[0].normalize()).clamp(-1.0, 1.0).acos();
                turn += first.dot(last).clamp(-1.0, 1.0).acos();
                n += 1.0;
            }
            (tilt.to_degrees() / n, turn.to_degrees() / n)
        };
        let (upright, straight) = measure(0.0, 0.0);
        assert!(upright < 3.0 && straight < 0.5, "no lean or curl: tilted {upright}, turned {straight}");
        let (leaning, _) = measure(1.0, 0.0);
        assert!(leaning > 40.0 && leaning < 75.0, "a full lean tilts {leaning} degrees");
        let (_, curled) = measure(0.0, 1.0);
        assert!(curled > 50.0, "a full curl turns {curled} degrees");
    }

    /// A closed part wound inside out still grows its fur outward, as one
    /// wound the right way does: with no normals, and with the normals an
    /// import generates from that winding, which point in.
    #[test]
    fn an_inside_out_part_grows_its_fibres_outward() {
        let sphere = crate::demo::uv_sphere(1.0, 32, 16);
        let mut inverted = sphere.clone();
        for t in inverted.indices.chunks_exact_mut(3) {
            t.swap(1, 2);
        }
        for n in &mut inverted.normals {
            *n = -*n;
        }
        let bare = |m: &RenderMesh| {
            let mut m = m.clone();
            m.normals.clear();
            m
        };
        for mesh in [sphere.clone(), inverted.clone(), bare(&sphere), bare(&inverted)] {
            let mesh = &mesh;
            let fur = grow(mesh, 1.0, &request(0.05, 2.0, 0.0)).expect("fibres");
            for fibre in fibres_of(&fur).points.chunks_exact(Fibres::POINTS).take(200) {
                assert!(fibre[Fibres::POINTS - 1].length() > fibre[0].length() + 0.02, "a fibre grew inward");
            }
        }
    }

    /// A large part's fibres grow fast enough to leave fibres in the
    /// interactive view: the snapshot grows them only when their settings
    /// or the part's mesh change, and that edit should not stall it. A
    /// million, four times the old cap, in batches across the processor.
    #[test]
    fn a_large_parts_fibres_grow_quickly() {
        let sphere = crate::demo::uv_sphere(1.0, 96, 48);
        let start = std::time::Instant::now();
        let fur = grow_with(&sphere, 1.0, &request(0.003, 5000.0, 0.5), Growth { count: Some(1_000_000), paint: None }).expect("fibres");
        let took = start.elapsed();
        assert_eq!(fibres_of(&fur).count(), 1_000_000);
        eprintln!("1,000,000 fibres grown in {:.0} ms", took.as_secs_f64() * 1000.0);
        assert!(took.as_secs_f64() < 2.0, "a million fibres took {took:?}");
    }

    /// Fibres grown in batches, each from its own place in the stream, are
    /// the fibres grown one after another: every fibre draws nine numbers,
    /// so fibre `i`'s shade is the stream's `9 i + 9`th, across the batches'
    /// edges as within them.
    #[test]
    fn fibres_grown_in_batches_are_the_fibres_grown_in_turn() {
        let sphere = crate::demo::uv_sphere(1.0, 32, 16);
        let fur = grow(&sphere, 1.0, &request_with(0.01, 3.0, 0.6, 0.4)).expect("fibres");
        let fibres = fibres_of(&fur);
        assert!(fibres.count() > 2 * BATCH + 1, "{} fibres", fibres.count());
        let mut random = SplitMix(3u64 ^ 0x5eed_f022);
        let shades: Vec<f32> = (0..fibres.count() as u64 * DRAWS).map(|_| random.unit()).collect();
        for i in [0, 1, BATCH - 1, BATCH, BATCH + 1, 2 * BATCH, fibres.count() - 1] {
            assert_eq!(fibres.random[i], shades[i * DRAWS as usize + DRAWS as usize - 1], "fibre {i}");
        }
        let skipped = SplitMix(7).skipped(1000);
        let mut walked = SplitMix(7);
        for _ in 0..1000 {
            walked.unit();
        }
        assert_eq!(skipped.0, walked.0);
    }

    /// Past the scene's budget every part keeps the same share of what it
    /// asked, a large one no less than a small one; within it, each all it
    /// asked. A part given fewer than it asked widens them by as much, so
    /// the pile covers what the fibres asked for would.
    #[test]
    fn a_scene_past_its_budget_thins_every_part_alike() {
        assert_eq!(shares(&[1000.0, 250_000.0, 3_000_000.0]), vec![1000, 250_000, 3_000_000]);
        let asked = [2_000_000.0, 14_000_000.0];
        let given = shares(&asked);
        assert_eq!(given.iter().sum::<usize>(), BUDGET);
        assert_eq!(given[1], 7 * given[0]);

        let sphere = crate::demo::uv_sphere(1.0, 32, 16);
        let asking = request(0.01, 3.0, 0.3);
        let all = grow(&sphere, 1.0, &asking).expect("fibres");
        let asked = fibres_of(&all).count();
        let half = grow_with(&sphere, 1.0, &asking, Growth { count: Some(asked / 2), paint: None }).expect("fibres");
        assert_eq!(fibres_of(&half).count(), asked / 2);
        let widened = fibres_of(&half).radii[0] / fibres_of(&all).radii[0];
        assert!((widened - asked as f32 / (asked / 2) as f32).abs() < 1e-4, "widened {widened} times");
        // What is kept is the first of what was asked, as it was.
        assert_eq!(fibres_of(&half).points[..Fibres::POINTS], fibres_of(&all).points[..Fibres::POINTS]);
    }

    /// A ball of radius 0.4 about (0.5, 0.5, 0.5), all of it in the
    /// positive octant: its object coordinates are colours.
    fn ball_in_the_unit_cube() -> RenderMesh {
        let mut ball = crate::demo::uv_sphere(0.4, 48, 24);
        for p in &mut ball.positions {
            *p = *p + Vec3::splat(0.5);
        }
        ball
    }

    /// A graph whose Sheen Color is the surface's object coordinates and
    /// whose Sheen Roughness is their x.
    fn coordinates_as_sheen() -> CompiledGraph {
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
        use tequila_materials::material_type::MaterialType;
        use tequila_materials::nodes::ShaderNode;
        let mut graph = MaterialGraph::from_material(MaterialType::Velvet.defaults());
        let output = graph.output();
        let coordinates = ShaderNode::from_key("texture_coordinate").expect("a node");
        let object = coordinates.outputs().iter().position(|o| o.name == "Object").expect("an Object output") as u8;
        let coordinates = graph.add_node(Node::new(NodeKind::Node(coordinates)));
        graph.connect_output(coordinates, object, output, Socket::Param(Param::FuzzColor));
        let split = graph.add_node(Node::new(NodeKind::Node(ShaderNode::from_key("separate_xyz").expect("a node"))));
        graph.connect_output(coordinates, object, split, Socket::Input(0));
        graph.connect_output(split, 0, output, Socket::Param(Param::FuzzRoughness));
        graph.compile().expect("compiles")
    }

    /// Where the material's Sheen Color and Roughness vary over the part,
    /// each fibre takes them from where it is rooted: here they are the
    /// surface's coordinates, so every fibre's colour is its own root and
    /// its roughness its root's x. A material whose sheen is the same
    /// everywhere paints nothing, and its fibres are as they always were.
    #[test]
    fn fibres_take_the_sheen_where_they_are_rooted() {
        let ball = ball_in_the_unit_cube();
        let program = coordinates_as_sheen();
        let textures = TextureTable::new();
        let paint = Paint::of(&program, &textures, Transform::IDENTITY, 0).expect("a sheen that varies");
        assert_eq!(paint.takes(), (true, true));
        let asking = request(0.01, 3.0, 0.4);
        let fur = grow_with(&ball, 1.0, &asking, Growth { count: None, paint: Some(&paint) }).expect("fibres");
        let fibres = fibres_of(&fur);
        assert_eq!(fibres.colors.len(), fibres.count());
        assert_eq!(fibres.roughness.len(), fibres.count());
        for (i, fibre) in fibres.points.chunks_exact(Fibres::POINTS).enumerate() {
            let root = fibre[0];
            let colour = fibres.colors[i];
            assert!(
                (colour.r - root.x).abs() < 1e-5 && (colour.g - root.y).abs() < 1e-5 && (colour.b - root.z).abs() < 1e-5,
                "fibre {i} rooted at {root:?} is {colour:?}"
            );
            assert!((fibres.roughness[i] - root.x).abs() < 1e-5, "fibre {i}: roughness {} at {root:?}", fibres.roughness[i]);
        }
        // Painting draws nothing from the fibres' stream: the same fibres,
        // coloured.
        let plain = grow(&ball, 1.0, &asking).expect("fibres");
        assert_eq!(fibres_of(&plain).points, fibres.points);
        assert_eq!(fibres_of(&plain).random, fibres.random);
        assert!(fibres_of(&plain).colors.is_empty() && fibres_of(&plain).roughness.is_empty());

        // A constant sheen, a graph or not, paints nothing.
        let constant = tequila_materials::graph::MaterialGraph::from_material(tequila_materials::material_type::MaterialType::Velvet.defaults());
        assert!(Paint::of(&constant.compile().expect("compiles"), &textures, Transform::IDENTITY, 0).is_none());
    }

    /// A fibre on a textured sheen has the colour the surface has under it,
    /// converted from the picture's sRGB primaries into the rendering space
    /// as the surface's own lookup converts it: a picture red on its left
    /// half and blue on its right, read by the ball's UVs, gives the fibres
    /// on each half the colour the surface shows there, to the rounding of
    /// the lookup's blend between two of the picture's texels.
    #[test]
    fn textured_fibres_have_the_surfaces_colour() {
        use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, TextureId, Value};
        use tequila_materials::material_type::MaterialType;
        use tequila_texture::image::{ColorSpace, Image};
        use tequila_texture::projection::Mapping;
        use tequila_texture::texture::{MapRole, Texture};
        let (red, blue) = ([0.58f32, 0.02, 0.012], [0.007f32, 0.045, 0.47]);
        let mut data = Vec::new();
        for _ in 0..4 {
            for x in 0..64 {
                data.extend_from_slice(if x < 32 { &red } else { &blue });
            }
        }
        let mut textures = TextureTable::new();
        textures.insert(
            TextureId(1),
            std::sync::Arc::new(Texture::from_linear(Image { width: 64, height: 4, channels: 3, color_space: ColorSpace::LinearSrgb, data }, MapRole::Color, Mapping::default())),
        );
        let mut graph = MaterialGraph::from_material(MaterialType::Velvet.defaults());
        let image = graph.add_node(Node::new(NodeKind::ImageTexture { texture: TextureId(1), fallback: Value::Color(Spectrum::ZERO) }));
        let output = graph.output();
        graph.connect(image, output, Socket::Param(Param::FuzzColor));
        let program = graph.compile().expect("compiles");
        let paint = Paint::of(&program, &textures, Transform::IDENTITY, 0).expect("a textured sheen");
        assert_eq!(paint.takes(), (true, false));

        // What the surface shows at a UV, as the renderer shades it there.
        let surface_at = |u: f32| {
            let point = SurfacePoint {
                position: Vec3::Y,
                normal: Vec3::Y,
                uv: Vec2 { x: u, y: 0.5 },
                footprint: 1e-5,
                scale: 1.0,
                uv_footprint: Some([1e-5, 1e-5]),
                curvature: 0.0,
                occlusion: 0.0,
                vertex_color: Spectrum::ONE,
                world: Default::default(),
            };
            shade(&program, &textures, &point, &mut Registers::new(), &mut Material::default()).fuzz_color
        };
        let (left, right) = (surface_at(0.25), surface_at(0.75));
        assert_eq!(left, Spectrum::from_linear_srgb(tequila_core::math::vec3(red[0], red[1], red[2])));
        assert_eq!(right, Spectrum::from_linear_srgb(tequila_core::math::vec3(blue[0], blue[1], blue[2])));

        let ball = crate::demo::uv_sphere(1.0, 64, 32);
        let fur = grow_with(&ball, 1.0, &request(0.01, 2.0, 0.4), Growth { count: None, paint: Some(&paint) }).expect("fibres");
        let fibres = fibres_of(&fur);
        assert!(fibres.roughness.is_empty(), "a constant roughness kept for every fibre");
        let (mut on_left, mut on_right) = (0, 0);
        for (i, fibre) in fibres.points.chunks_exact(Fibres::POINTS).enumerate() {
            // The sphere's u runs round from +x towards +z.
            let root = fibre[0];
            let u = root.z.atan2(root.x).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
            let expected = if (0.05..0.45).contains(&u) {
                on_left += 1;
                left
            } else if (0.55..0.95).contains(&u) {
                on_right += 1;
                right
            } else {
                continue;
            };
            let got = fibres.colors[i];
            let off = (got.r - expected.r).abs().max((got.g - expected.g).abs()).max((got.b - expected.b).abs());
            assert!(off < 1e-6, "fibre {i} at u {u}: {got:?}, not {expected:?}");
        }
        assert!(on_left > 1000 && on_right > 1000, "{on_left} fibres on the left, {on_right} on the right");
    }

    /// Nothing asked for, nothing grown.
    #[test]
    fn no_density_grows_nothing() {
        let sphere = crate::demo::uv_sphere(1.0, 16, 8);
        assert!(grow(&sphere, 1.0, &request(0.002, 0.0, 0.3)).is_none());
    }
}
