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
//! Two settings make them a pile rather than a fur. `clump` gathers
//! neighbours into locks, as a hairy suede's nap mats or a sheepskin's wool
//! tufts: clumps are grown first, each with a fibre of its own, and every
//! fibre is drawn toward its nearest clump's shape, carried over to its own
//! root and narrowing evenly to the clump's tip, taking most of its shade
//! too. `comb` lays fibres the
//! same way as their neighbours, along a smooth field of directions over
//! the part that holds over a few centimetres and turns between. Both draw
//! from random streams of their own, so fibres asked for without them come
//! out exactly as they did before either existed.
//!
//! Fibres are a lot of geometry, so the count is capped at [`MOST`]: past
//! that, a large part gets fewer, sparser fibres rather than a scene that
//! will not build.

use tequila_core::math::Vec3;
use tequila_materials::graph::FuzzRequest;
use tequila_render_core::scene::{Fibres, Mesh as RenderMesh};

/// The most fibres one part grows.
pub const MOST: usize = 250_000;

/// What the tip's radius is of the root's.
const TIP: f32 = 0.2;

/// How many fibres `request` grows over `area` square scene units
/// (metres), capped.
pub fn count(area: f32, request: &FuzzRequest) -> usize {
    let wanted = (area.max(0.0) as f64) * 1.0e4 * (request.density().max(0.0) as f64);
    (wanted.round() as usize).min(MOST)
}

/// The fibres grown on `mesh`, in its own space; `scale` is scene units per
/// mesh unit, so lengths and densities are in scene units however the part
/// was scaled. `None` when the mesh has no area or the request no fibres.
pub fn grow(mesh: &RenderMesh, scale: f32, request: &FuzzRequest) -> Option<RenderMesh> {
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
    let area = total as f32 * scale * scale;
    let n = count(area, request);
    let length = request.length() / scale;
    let radius = request.thickness() / scale;
    if n == 0 || total <= 0.0 || !(length > 0.0) || !(radius > 0.0) {
        return None;
    }

    let mut random = SplitMix(u64::from(request.seed) ^ 0x5eed_f022);
    let unit = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let clump = unit(request.clump());
    let comb = unit(request.comb());
    let surface = Surface { mesh, triangles: &triangles, running: &running, total, flip_winding, flip_normals };
    let shape = Shape {
        lean: request.lean().clamp(0.0, 1.0),
        curl: request.curl().clamp(0.0, 1.0),
        length,
        comb,
        field: (comb > 0.0).then(|| CombField::new(request.seed, COMB_PATCH / scale)),
    };
    // The clumps' own fibres, each the one its clump gathers to. Drawn from
    // a stream of their own, so fibres asked for without clumping, or
    // without combing, come out exactly as they always have.
    let clumps = (clump > 0.0).then(|| Clumps::grow(&surface, &shape, n, request.seed));
    let segments = Fibres::SEGMENTS;
    let mut fibres = Fibres {
        points: Vec::with_capacity(n * Fibres::POINTS),
        radii: Vec::with_capacity(n * Fibres::POINTS),
        random: Vec::with_capacity(n),
    };
    for _ in 0..n {
        let (root, normal) = surface.sample(&mut random);
        let mut points = shape.fibre(&mut random, root, normal);
        // Gathered into the nearest clump: its own fibre's shape, carried
        // over to this root and narrowing evenly to its tip, so a clump is a
        // lock that tapers, not a tuft pinched in at its base. The root
        // stays where it grew.
        let mut shade = None;
        if let Some(guide) = clumps.as_ref().and_then(|c| c.nearest(root)) {
            let offset = root - guide.points[0];
            for (point, at) in points.iter_mut().enumerate().skip(1) {
                let along = point as f32 / segments as f32;
                let lock = guide.points[point] + offset * (1.0 - along);
                *at = *at + (lock - *at) * clump;
            }
            shade = Some(guide.random);
        }
        for (point, at) in points.iter().enumerate() {
            let along = point as f32 / segments as f32;
            fibres.points.push(*at);
            fibres.radii.push(radius * (1.0 - (1.0 - TIP) * along));
        }
        // A clump's fibres share most of its shade, so a clump reads as one
        // lock, lighter or darker than the next.
        let own = random.unit();
        fibres.random.push(match shade {
            Some(theirs) => own + (theirs - own) * clump * 0.75,
            None => own,
        });
    }
    Some(RenderMesh::of_fibres(fibres))
}

/// How wide a clump is, as a share of the fibres' length: a lock of
/// hairy suede about half as wide as it is long.
const CLUMP_RADIUS: f32 = 0.45;

/// How wide a combed patch is, in scene units (metres): the nap of a hide
/// lies one way over a few centimetres, then turns.
const COMB_PATCH: f32 = 0.04;

/// The most clumps one part has, past which they are larger than asked.
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

impl Surface<'_> {
    /// A point on the surface, evenly by area, and which way is out there.
    fn sample(&self, random: &mut SplitMix) -> (Vec3, Vec3) {
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
        (root, normal)
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
/// patch and turns a little between patches.
///
/// The drift runs along the part's diagonal. Where it stands straight out
/// of the surface the waves alone are left, and turn the nap round in
/// whorls; along the diagonal, that is at a corner of a part modelled along
/// its axes, never across a face - not a top, whichever way is up, and not
/// a side.
struct CombField {
    waves: [(Vec3, f32, Vec3); 4],
}

/// The drift, a unit vector along the diagonal.
const DRIFT: Vec3 = tequila_core::math::vec3(0.577_350_3, 0.577_350_3, 0.577_350_3);

/// How far the waves turn the nap from the drift: a third of it, so a
/// patch's nap wanders by a few tens of degrees and never doubles back.
const WANDER: f32 = 0.3;

impl CombField {
    /// Patches `patch` mesh units across.
    fn new(seed: u32, patch: f32) -> CombField {
        let mut random = SplitMix(u64::from(seed) ^ 0xc0_4b_f1e1d);
        let direction = |random: &mut SplitMix| {
            // Uniform on the sphere.
            let z = random.unit() * 2.0 - 1.0;
            let angle = random.unit() * std::f32::consts::TAU;
            let r = (1.0 - z * z).max(0.0).sqrt();
            tequila_core::math::vec3(r * angle.cos(), r * angle.sin(), z)
        };
        let frequency = std::f32::consts::TAU / patch.max(1e-9);
        let waves = std::array::from_fn(|_| {
            let k = direction(&mut random) * frequency;
            let phase = random.unit() * std::f32::consts::TAU;
            (k, phase, direction(&mut random) * WANDER)
        });
        CombField { waves }
    }

    /// The way the nap lies at `p`, along the surface; `None` where the
    /// field stands straight out of it.
    fn along(&self, p: Vec3, normal: Vec3) -> Option<Vec3> {
        let mut f = DRIFT;
        for (k, phase, amplitude) in &self.waves {
            f = f + *amplitude * (k.dot(p) + phase).sin();
        }
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
    /// by area; at least four fibres each and at most [`MOST_CLUMPS`].
    fn grow(surface: &Surface, shape: &Shape, fibres: usize, seed: u32) -> Clumps {
        let radius = (CLUMP_RADIUS * shape.length).max(1e-9);
        let wanted = surface.total / (std::f64::consts::PI * (radius as f64).powi(2));
        let count = (wanted.round() as usize).clamp(1, (fibres / 4).max(1)).min(MOST_CLUMPS);
        // Fewer clumps than wanted are larger: found within their own size.
        let radius = radius * (wanted / count as f64).max(1.0).sqrt() as f32;
        let mut random = SplitMix(u64::from(seed) ^ 0xc10b_5eed);
        let mut clumps = Clumps { cell: 2.0 * radius, guides: Vec::with_capacity(count), grid: Default::default() };
        for index in 0..count {
            let (root, normal) = surface.sample(&mut random);
            let points = shape.fibre(&mut random, root, normal);
            clumps.guides.push(Guide { points, random: random.unit() });
            clumps.grid.entry(clumps.key(root)).or_default().push(index as u32);
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

struct SplitMix(u64);

impl SplitMix {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
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
    /// more than the cap.
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
        assert_eq!(count(1000.0, &request(0.002, 400.0, 0.3)), MOST);
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

    /// A part's whole budget grows fast enough to leave fibres in the
    /// interactive view: the snapshot grows them only when their settings
    /// or the part's mesh change, and that edit should not stall it.
    #[test]
    fn a_parts_whole_budget_grows_quickly() {
        let sphere = crate::demo::uv_sphere(1.0, 96, 48);
        let start = std::time::Instant::now();
        let fur = grow(&sphere, 1.0, &request(0.003, 5000.0, 0.5)).expect("fibres");
        let took = start.elapsed();
        assert_eq!(fibres_of(&fur).count(), MOST);
        eprintln!("{MOST} fibres grown in {:.0} ms", took.as_secs_f64() * 1000.0);
        assert!(took.as_secs_f64() < 2.0, "{MOST} fibres took {took:?}");
    }

    /// Nothing asked for, nothing grown.
    #[test]
    fn no_density_grows_nothing() {
        let sphere = crate::demo::uv_sphere(1.0, 16, 8);
        assert!(grow(&sphere, 1.0, &request(0.002, 0.0, 0.3)).is_none());
    }
}
