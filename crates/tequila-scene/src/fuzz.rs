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
    let lean = request.lean().clamp(0.0, 1.0);
    let curl = request.curl().clamp(0.0, 1.0);
    let segments = Fibres::SEGMENTS;
    let mut fibres = Fibres {
        points: Vec::with_capacity(n * Fibres::POINTS),
        radii: Vec::with_capacity(n * Fibres::POINTS),
        random: Vec::with_capacity(n),
    };
    for _ in 0..n {
        // A root, evenly by area.
        let pick = random.unit() as f64 * total;
        let index = running.partition_point(|&r| r < pick).min(triangles.len() - 1);
        let [ia, ib, ic] = triangles[index];
        let (a, b, c) = (mesh.positions[ia as usize], mesh.positions[ib as usize], mesh.positions[ic as usize]);
        let (mut u, mut v) = (random.unit(), random.unit());
        if u + v > 1.0 {
            u = 1.0 - u;
            v = 1.0 - v;
        }
        let root = a + (b - a) * u + (c - a) * v;
        let face = if flip_winding { (c - a).cross(b - a) } else { (b - a).cross(c - a) };
        let normal = if mesh.normals.len() == mesh.positions.len() {
            let n = mesh.normals[ia as usize] * (1.0 - u - v) + mesh.normals[ib as usize] * u + mesh.normals[ic as usize] * v;
            let n = if flip_normals { -n } else { n };
            n.try_normalize().unwrap_or_else(|| face.normalize())
        } else {
            face.try_normalize().unwrap_or(Vec3::Y)
        };
        let (t, s) = perpendiculars(normal);
        // Leaning a random way, by up to `lean` of a near-flat 80 degrees.
        let azimuth = random.unit() * std::f32::consts::TAU;
        let tilt = lean * (0.35 + 0.65 * random.unit()) * 80f32.to_radians();
        let sideways = t * azimuth.cos() + s * azimuth.sin();
        let mut direction = (normal * tilt.cos() + sideways * tilt.sin()).normalize();
        // Bending about an axis across the fibre, a little more each step,
        // the axis itself turning a little so a curl is not flat.
        let mut bend_axis = normal.cross(sideways).try_normalize().unwrap_or(t);
        let bend = curl * (0.4 + 0.6 * random.unit()) * 35f32.to_radians();
        let twist = curl * (random.unit() * 2.0 - 1.0) * 40f32.to_radians();
        let step = length * (0.7 + 0.6 * random.unit()) / segments as f32;

        let mut at = root;
        for point in 0..Fibres::POINTS {
            let along = point as f32 / segments as f32;
            fibres.points.push(at);
            fibres.radii.push(radius * (1.0 - (1.0 - TIP) * along));
            at = at + direction * step;
            direction = rotate(direction, bend_axis, bend).normalize();
            bend_axis = rotate(bend_axis, direction, twist).normalize();
        }
        fibres.random.push(random.unit());
    }
    Some(RenderMesh::of_fibres(fibres))
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
            seed: 3,
        }
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
