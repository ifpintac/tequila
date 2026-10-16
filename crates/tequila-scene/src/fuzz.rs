//! The Fuzz geometry node (`docs/02` §4.4): real fibres standing out of a
//! part's surface - a peach's skin, felt's halo, a tennis ball's nap, a
//! towel's loops - for the renders where a sheen alone is too smooth.
//!
//! Each fibre is a thin tube of three sides, tapering to a point over three
//! segments, rooted at a point spread evenly over the surface by area. It
//! leans away from the normal by up to `lean` and bends along its length by
//! `curl`, each fibre its own way, every length a little different. The
//! tubes are one mesh in the part's own space, so they move with the part,
//! and they wear the part's material.
//!
//! Fibres are a lot of geometry, so the count is capped at [`MOST`]: past
//! that, a large part gets fewer, sparser fibres rather than a scene that
//! will not build.

use tequila_core::math::Vec3;
use tequila_materials::graph::FuzzRequest;
use tequila_render_core::scene::Mesh as RenderMesh;

/// The most fibres one part grows.
pub const MOST: usize = 150_000;

/// Segments along a fibre and sides around it.
const SEGMENTS: usize = 3;
const SIDES: usize = 3;

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
    let area = total as f32 * scale * scale;
    let n = count(area, request);
    if n == 0 || total <= 0.0 {
        return None;
    }

    let mut random = SplitMix(u64::from(request.seed) ^ 0x5eed_f022);
    let length = request.length() / scale;
    let radius = request.thickness() / scale;
    let lean = request.lean().clamp(0.0, 1.0);
    let curl = request.curl().clamp(0.0, 1.0);
    let per_fibre = (SEGMENTS * SIDES) + 1;
    let mut positions = Vec::with_capacity(n * (per_fibre + SIDES));
    let mut normals = Vec::with_capacity(n * (per_fibre + SIDES));
    let mut indices: Vec<u32> = Vec::with_capacity(n * SEGMENTS * SIDES * 6);
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
        let face = (b - a).cross(c - a);
        let normal = if mesh.normals.len() == mesh.positions.len() {
            let n = mesh.normals[ia as usize] * (1.0 - u - v) + mesh.normals[ib as usize] * u + mesh.normals[ic as usize] * v;
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
        // Bending about an axis across the fibre, a little more each step.
        let bend_axis = normal.cross(sideways).try_normalize().unwrap_or(t);
        let bend = curl * (0.4 + 0.6 * random.unit()) * 35f32.to_radians();
        let step = length * (0.7 + 0.6 * random.unit()) / SEGMENTS as f32;

        let base = positions.len() as u32;
        let mut at = root;
        for ring in 0..SEGMENTS {
            let r = radius * (1.0 - ring as f32 / SEGMENTS as f32 * 0.8);
            let (p, q) = perpendiculars(direction);
            for side in 0..SIDES {
                let angle = side as f32 / SIDES as f32 * std::f32::consts::TAU;
                let out = p * angle.cos() + q * angle.sin();
                positions.push(at + out * r);
                normals.push(out);
            }
            at = at + direction * step;
            direction = rotate(direction, bend_axis, bend).normalize();
        }
        // The tip.
        positions.push(at);
        normals.push(direction);
        let tip = base + (SEGMENTS * SIDES) as u32;
        for ring in 0..SEGMENTS {
            let first = base + (ring * SIDES) as u32;
            for side in 0..SIDES as u32 {
                let next = (side + 1) % SIDES as u32;
                let (a0, a1) = (first + side, first + next);
                if ring + 1 < SEGMENTS {
                    let (b0, b1) = (a0 + SIDES as u32, a1 + SIDES as u32);
                    indices.extend_from_slice(&[a0, a1, b1, a0, b1, b0]);
                } else {
                    indices.extend_from_slice(&[a0, a1, tip]);
                }
            }
        }
    }
    Some(RenderMesh::new(positions, indices).with_normals(normals))
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
        FuzzRequest {
            length_bits: length.to_bits(),
            density_bits: density.to_bits(),
            thickness_bits: 0.00002f32.to_bits(),
            lean_bits: lean.to_bits(),
            curl_bits: 0.3f32.to_bits(),
            seed: 3,
        }
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
        let fibres = fur.positions.len() / (SEGMENTS * SIDES + 1);
        let expected = area_m2 * 1.0e4 * 200.0;
        assert!((fibres as f32 / expected - 1.0).abs() < 0.03, "{fibres} fibres for {expected}");
        assert_eq!(count(1000.0, &request(0.002, 400.0, 0.3)), MOST);
    }

    /// Fibres stand out of the surface, rooted on it, about as long as
    /// asked: upright with no lean, never into the part.
    #[test]
    fn fibres_stand_out_of_the_surface() {
        let sphere = crate::demo::uv_sphere(1.0, 48, 24);
        let fur = grow(&sphere, 1.0, &request(0.05, 2.0, 0.0)).expect("fibres");
        let per = SEGMENTS * SIDES + 1;
        for fibre in fur.positions.chunks_exact(per).take(500) {
            let root = fibre[..SIDES].iter().fold(Vec3::ZERO, |a, p| a + *p) * (1.0 / SIDES as f32);
            let tip = fibre[per - 1];
            assert!((root.length() - 1.0).abs() < 0.01, "a root off the surface: {root:?}");
            let rise = tip.length() - root.length();
            assert!(rise > 0.03 && rise < 0.07, "a fibre rising {rise}");
        }
    }

    /// Nothing asked for, nothing grown.
    #[test]
    fn no_density_grows_nothing() {
        let sphere = crate::demo::uv_sphere(1.0, 16, 8);
        assert!(grow(&sphere, 1.0, &request(0.002, 0.0, 0.3)).is_none());
    }

    /// The tubes are closed and wound outwards.
    #[test]
    fn the_tubes_face_out() {
        let sphere = crate::demo::uv_sphere(1.0, 16, 8);
        let fur = grow(&sphere, 1.0, &request(0.05, 5.0, 0.2)).expect("fibres");
        let mut agree = 0;
        let mut total = 0;
        for t in fur.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| fur.positions[i as usize]);
            let face = (b - a).cross(c - a);
            if face.length() < 1e-12 {
                continue;
            }
            let n = fur.normals[t[0] as usize] + fur.normals[t[1] as usize] + fur.normals[t[2] as usize];
            total += 1;
            if face.dot(n) > 0.0 {
                agree += 1;
            }
        }
        assert!(agree as f32 > 0.95 * total as f32, "{agree} of {total} face out");
    }
}
