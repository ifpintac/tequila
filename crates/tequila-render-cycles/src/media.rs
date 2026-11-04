//! Nested media for Cycles: a liquid in its glass, as Tequila's renderer
//! has it.
//!
//! A filled container is two media that overlap: the liquid is modelled a
//! fraction of a millimetre into its glass so that no air gap can open
//! between them, and in the overlap the liquid, whose priority is higher,
//! is what is there. Tequila's renderer keeps a stack of the media a path is
//! in and bends light at each surface by the indices actually meeting there
//! (`Media` in tequila-render-core's integrator). Cycles has no such stack:
//! every surface of the liquid met the air, and the glass's inner wall,
//! inside the liquid, bent light a second time as glass meeting air. Light
//! crossed an air gap that was not there, and perfume and drinks came out
//! paler than in Tequila's renderer.
//!
//! So the media are worked out here, from the geometry, before Cycles sees
//! it: for every vertex of every clear solid, which other solids it lies
//! inside. A vertex inside a medium of higher priority is on a surface that
//! is no boundary at all - the glass's inner wall under the liquid - and
//! light should pass it straight. A vertex inside one of lower priority is
//! on a surface that meets that medium rather than the air - the liquid's
//! side against the glass - and light should bend there by the ratio of
//! the two indices. Both go to Cycles as a vertex attribute
//! ([`MEDIA_ATTRIBUTE`]), which the material's shader reads: the outside
//! index in red, less one so that a mesh without it reads air, and in
//! green how far the surface is no boundary.

use std::collections::HashMap;
use tequila_core::math::Vec3;
use tequila_render_core::scene::SceneSnapshot;

/// The vertex attribute the media go to Cycles as.
pub const MEDIA_ATTRIBUTE: &str = "tq_media";

/// How long an edge of a medium's mesh may be, in metres, for what is
/// worked out at its vertices to follow the line where it meets another:
/// a bottle's walls are long strips from its base to its shoulder, whose
/// corners alone would spread "under the liquid" over the whole wall.
const FINEST_EDGE: f32 = 0.001;

/// The most triangles a medium's mesh is refined to, past which its edges
/// stay longer than [`FINEST_EDGE`] and the line it meets another along is
/// that much softer.
const MOST_TRIANGLES: usize = 1_500_000;

/// A medium's mesh as it goes to Cycles - refined where its edges were too
/// long to follow the line it meets another along - with what lies outside
/// each of its vertices: four numbers a vertex, red the outside medium's
/// index less one, green 1 where the surface is no boundary.
pub struct Outside {
    pub mesh: tequila_render_core::scene::Mesh,
    pub values: Vec<[f32; 4]>,
}

/// The instances, by index, whose surfaces meet another medium than the
/// air somewhere, and what they meet. Instances that meet only the air are
/// left out, and go to Cycles as they always did.
pub fn attributes(scene: &SceneSnapshot) -> HashMap<usize, Outside> {
    struct Part<'a> {
        instance: usize,
        priority: f32,
        ior: f32,
        mesh: &'a tequila_render_core::scene::Mesh,
        transform: &'a tequila_core::math::Transform,
        solid: Solid,
    }
    let mut parts = Vec::new();
    for (index, instance) in scene.instances.iter().enumerate() {
        if instance.light_proxy || instance.shadow_catcher.is_some() {
            continue;
        }
        let (Some(mesh), Some(material)) = (scene.meshes.get(instance.mesh.index()), scene.materials.get(instance.material.index())) else {
            continue;
        };
        // A clear solid: something a path is inside once it is through the
        // surface. A thin wall is a pane, never a medium.
        if mesh.fibres.is_some() || material.geometry_thin_walled || material.transmission_weight < 0.5 {
            continue;
        }
        let world: Vec<Vec3> = mesh.positions.iter().map(|p| instance.transform.matrix.transform_point(*p)).collect();
        let triangles: Vec<[Vec3; 3]> = mesh
            .indices
            .chunks_exact(3)
            .filter_map(|t| Some([*world.get(t[0] as usize)?, *world.get(t[1] as usize)?, *world.get(t[2] as usize)?]))
            .collect();
        if !closed(&triangles) {
            continue;
        }
        parts.push(Part {
            instance: index,
            priority: material.medium_priority,
            ior: material.specular_ior,
            mesh,
            transform: &instance.transform,
            solid: Solid::new(triangles),
        });
    }
    let mut out = HashMap::new();
    if parts.len() < 2 {
        return out;
    }
    for a in &parts {
        let others: Vec<&Part> = parts.iter().filter(|b| b.instance != a.instance && a.solid.bounds.overlaps(&b.solid.bounds)).collect();
        if others.is_empty() {
            continue;
        }
        // Fine enough, in the part's own units, for the line it meets
        // another along: a millimetre on a bottle, a three-hundredth of it
        // on a tank, where a millimetre would be millions of triangles.
        let m = &a.transform.matrix;
        let scale = [Vec3::X, Vec3::Y, Vec3::Z].iter().map(|v| m.transform_vector(*v).length()).sum::<f32>() / 3.0;
        let size = (a.solid.bounds.hi - a.solid.bounds.lo).length();
        let finest = FINEST_EDGE.max(size / 300.0);
        let mesh = refined(a.mesh, finest / scale.max(1e-9), MOST_TRIANGLES);
        let mut values = vec![[0.0f32, 0.0, 0.0, 1.0]; mesh.positions.len()];
        // The priority of the medium each vertex is found to meet so far.
        let mut meets = vec![f32::NEG_INFINITY; mesh.positions.len()];
        let mut any = false;
        for b in others {
            for (v, p) in mesh.positions.iter().enumerate() {
                let p = m.transform_point(*p);
                if !b.solid.contains(p) {
                    continue;
                }
                any = true;
                if b.priority > a.priority {
                    // Inside a medium that wins here: no boundary.
                    values[v][1] = 1.0;
                } else if b.priority > meets[v] {
                    // Meeting `b` rather than the air: the highest-priority
                    // of the media around it that do not win over it.
                    meets[v] = b.priority;
                    values[v][0] = (b.ior - 1.0).max(0.0);
                }
            }
        }
        if any {
            out.insert(a.instance, Outside { mesh, values });
        }
    }
    out
}

/// `mesh` with every edge longer than `longest` split at its middle,
/// again and again until none is, or until it has `most` triangles. An
/// edge two triangles share is split once, for both, so nothing cracks
/// open; the normals and UVs at a new vertex are its edge's ends' between.
pub fn refined(mesh: &tequila_render_core::scene::Mesh, longest: f32, most: usize) -> tequila_render_core::scene::Mesh {
    let mut positions = mesh.positions.clone();
    let shaded = mesh.normals.len() == positions.len();
    let mapped = mesh.uvs.len() == positions.len();
    let mut normals = if shaded { mesh.normals.clone() } else { Vec::new() };
    let mut uvs = if mapped { mesh.uvs.clone() } else { Vec::new() };
    let mut triangles: Vec<[u32; 3]> = mesh
        .indices
        .chunks_exact(3)
        .filter(|t| t.iter().all(|&i| (i as usize) < positions.len()))
        .map(|t| [t[0], t[1], t[2]])
        .collect();
    let limit = longest * longest;
    for _ in 0..16 {
        if triangles.len() >= most {
            break;
        }
        let mut middles: HashMap<(u32, u32), u32> = HashMap::new();
        let mut next = Vec::with_capacity(triangles.len() * 2);
        let mut split_any = false;
        for t in &triangles {
            let mut middle = [None; 3];
            for e in 0..3 {
                let (a, b) = (t[e], t[(e + 1) % 3]);
                if (positions[a as usize] - positions[b as usize]).length_squared() <= limit {
                    continue;
                }
                let key = (a.min(b), a.max(b));
                let m = *middles.entry(key).or_insert_with(|| {
                    let (i, j) = (a as usize, b as usize);
                    positions.push((positions[i] + positions[j]) * 0.5);
                    if shaded {
                        normals.push((normals[i] + normals[j]).try_normalize().unwrap_or(normals[i]));
                    }
                    if mapped {
                        uvs.push((uvs[i] + uvs[j]) * 0.5);
                    }
                    (positions.len() - 1) as u32
                });
                middle[e] = Some(m);
                split_any = true;
            }
            match middle.iter().filter(|m| m.is_some()).count() {
                0 => next.push(*t),
                1 => {
                    // Turned so the split edge is the first.
                    let e = middle.iter().position(|m| m.is_some()).unwrap();
                    let (v0, v1, v2, m0) = (t[e], t[(e + 1) % 3], t[(e + 2) % 3], middle[e].unwrap());
                    next.push([v0, m0, v2]);
                    next.push([m0, v1, v2]);
                }
                2 => {
                    // Turned so the edge left whole is the last.
                    let whole = middle.iter().position(|m| m.is_none()).unwrap();
                    let r = (whole + 1) % 3;
                    let (v0, v1, v2) = (t[r], t[(r + 1) % 3], t[(r + 2) % 3]);
                    let (m0, m1) = (middle[r].unwrap(), middle[(r + 1) % 3].unwrap());
                    next.push([m0, v1, m1]);
                    next.push([v0, m0, m1]);
                    next.push([v0, m1, v2]);
                }
                _ => {
                    let (m0, m1, m2) = (middle[0].unwrap(), middle[1].unwrap(), middle[2].unwrap());
                    next.push([t[0], m0, m2]);
                    next.push([m0, t[1], m1]);
                    next.push([m2, m1, t[2]]);
                    next.push([m0, m1, m2]);
                }
            }
        }
        triangles = next;
        if !split_any {
            break;
        }
    }
    let mut out = tequila_render_core::scene::Mesh::new(positions, triangles.into_iter().flatten().collect());
    if shaded {
        out = out.with_normals(normals);
    }
    if mapped {
        out = out.with_uvs(uvs);
    }
    out
}

/// Whether `triangles` close a solid: every edge shared by two of them,
/// their corners welded by position, since a UV seam or a hard edge stores
/// one corner several times; a gap or two, a CAD export's sliver, allowed.
pub fn closed(triangles: &[[Vec3; 3]]) -> bool {
    let bounds = Bounds::of(triangles.iter().flatten().copied());
    let reach = (bounds.hi - bounds.lo).length();
    if triangles.is_empty() || !(reach > 0.0) {
        return false;
    }
    let step = f64::from(reach) * 1e-6;
    let mut welded: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut weld = |p: Vec3| {
        let key = ((f64::from(p.x) / step).round() as i64, (f64::from(p.y) / step).round() as i64, (f64::from(p.z) / step).round() as i64);
        let next = welded.len() as u32;
        *welded.entry(key).or_insert(next)
    };
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    for t in triangles {
        let [a, b, c] = t.map(&mut weld);
        for (x, y) in [(a, b), (b, c), (c, a)] {
            if x != y {
                *edges.entry((x.min(y), x.max(y))).or_insert(0) += 1;
            }
        }
    }
    let open = edges.values().filter(|&&uses| uses != 2).count();
    !edges.is_empty() && open as f64 <= 0.01 * edges.len() as f64
}

#[derive(Clone, Copy, Debug)]
struct Bounds {
    lo: Vec3,
    hi: Vec3,
}

impl Bounds {
    fn empty() -> Bounds {
        Bounds { lo: Vec3::splat(f32::INFINITY), hi: Vec3::splat(f32::NEG_INFINITY) }
    }

    fn of(points: impl IntoIterator<Item = Vec3>) -> Bounds {
        points.into_iter().fold(Bounds::empty(), |b, p| Bounds { lo: b.lo.min(p), hi: b.hi.max(p) })
    }

    fn overlaps(&self, other: &Bounds) -> bool {
        self.lo.x <= other.hi.x && other.lo.x <= self.hi.x && self.lo.y <= other.hi.y && other.lo.y <= self.hi.y && self.lo.z <= other.hi.z && other.lo.z <= self.hi.z
    }

    fn contains(&self, p: Vec3) -> bool {
        p.x >= self.lo.x && p.x <= self.hi.x && p.y >= self.lo.y && p.y <= self.hi.y && p.z >= self.lo.z && p.z <= self.hi.z
    }

    /// Where along `origin + t direction` the ray enters and leaves, if it
    /// meets the box at all ahead of the origin.
    fn hit(&self, origin: Vec3, inverse: Vec3) -> bool {
        let (mut near, mut far) = (0.0f32, f32::INFINITY);
        for (o, inv, lo, hi) in [(origin.x, inverse.x, self.lo.x, self.hi.x), (origin.y, inverse.y, self.lo.y, self.hi.y), (origin.z, inverse.z, self.lo.z, self.hi.z)] {
            let (a, b) = ((lo - o) * inv, (hi - o) * inv);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        near <= far
    }
}

/// A closed mesh in the world, to ask whether a point is inside it.
struct Solid {
    triangles: Vec<[Vec3; 3]>,
    bounds: Bounds,
    nodes: Vec<Branch>,
}

/// A node of the bounding-volume tree: a box, and either two children (the
/// first at `first`, the second after it) or a run of triangles.
struct Branch {
    bounds: Bounds,
    first: u32,
    count: u32,
}

impl Solid {
    fn new(mut triangles: Vec<[Vec3; 3]>) -> Solid {
        let bounds = Bounds::of(triangles.iter().flatten().copied());
        let mut nodes = vec![Branch { bounds, first: 0, count: triangles.len() as u32 }];
        let mut stack = vec![0usize];
        while let Some(at) = stack.pop() {
            let (first, count) = (nodes[at].first as usize, nodes[at].count as usize);
            if count <= 8 {
                continue;
            }
            let span = &mut triangles[first..first + count];
            let centres = Bounds::of(span.iter().map(|t| (t[0] + t[1] + t[2]) / 3.0));
            let size = centres.hi - centres.lo;
            let axis = if size.x >= size.y && size.x >= size.z { 0 } else if size.y >= size.z { 1 } else { 2 };
            let key = |t: &[Vec3; 3]| {
                let c = t[0] + t[1] + t[2];
                [c.x, c.y, c.z][axis]
            };
            span.sort_unstable_by(|a, b| key(a).total_cmp(&key(b)));
            let half = count / 2;
            let left = Branch { bounds: Bounds::of(span[..half].iter().flatten().copied()), first: first as u32, count: half as u32 };
            let right = Branch { bounds: Bounds::of(span[half..].iter().flatten().copied()), first: (first + half) as u32, count: (count - half) as u32 };
            let child = nodes.len();
            nodes.push(left);
            nodes.push(right);
            // A branch: its children's index, no triangles of its own.
            nodes[at] = Branch { bounds: nodes[at].bounds, first: child as u32, count: 0 };
            stack.push(child);
            stack.push(child + 1);
        }
        Solid { triangles, bounds, nodes }
    }

    /// Whether `p` is inside: the surface crossed an odd number of times on
    /// the way out, in each of three directions chosen to miss the grid an
    /// exporter's vertices sit on, by majority, so that a ray grazing an
    /// edge does not decide it.
    fn contains(&self, p: Vec3) -> bool {
        if !self.bounds.contains(p) {
            return false;
        }
        const WAYS: [[f32; 3]; 3] = [[0.5773, 0.5779, 0.5767], [-0.6015, 0.3112, 0.7356], [0.2441, -0.8513, 0.4645]];
        let odd = WAYS.iter().filter(|w| self.crossings(p, tequila_core::math::vec3(w[0], w[1], w[2]).normalize()) % 2 == 1).count();
        odd >= 2
    }

    fn crossings(&self, origin: Vec3, direction: Vec3) -> usize {
        let inverse = tequila_core::math::vec3(1.0 / direction.x, 1.0 / direction.y, 1.0 / direction.z);
        let mut count = 0;
        let mut stack = vec![0usize];
        while let Some(at) = stack.pop() {
            let node = &self.nodes[at];
            if !node.bounds.hit(origin, inverse) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.first as usize);
                stack.push(node.first as usize + 1);
                continue;
            }
            for t in &self.triangles[node.first as usize..(node.first + node.count) as usize] {
                if crosses(origin, direction, t) {
                    count += 1;
                }
            }
        }
        count
    }
}

/// Whether the ray from `origin` along `direction` crosses triangle `t`
/// ahead of the origin (Möller and Trumbore).
fn crosses(origin: Vec3, direction: Vec3, t: &[Vec3; 3]) -> bool {
    let (e1, e2) = (t[1] - t[0], t[2] - t[0]);
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return false;
    }
    let inv = 1.0 / det;
    let s = origin - t[0];
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return false;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return false;
    }
    e2.dot(q) * inv > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// A box from `lo` to `hi`, its faces turned out (or in, for a cavity).
    pub(crate) fn boxed(lo: Vec3, hi: Vec3, inward: bool) -> (Vec<Vec3>, Vec<u32>) {
        let p = |x: f32, y: f32, z: f32| vec3(x, y, z);
        let positions = vec![
            p(lo.x, lo.y, lo.z), p(hi.x, lo.y, lo.z), p(hi.x, hi.y, lo.z), p(lo.x, hi.y, lo.z),
            p(lo.x, lo.y, hi.z), p(hi.x, lo.y, hi.z), p(hi.x, hi.y, hi.z), p(lo.x, hi.y, hi.z),
        ];
        let faces: [[u32; 4]; 6] = [[0, 3, 2, 1], [4, 5, 6, 7], [0, 1, 5, 4], [2, 3, 7, 6], [1, 2, 6, 5], [0, 4, 7, 3]];
        let mut indices = Vec::new();
        for [a, b, c, d] in faces {
            if inward {
                indices.extend_from_slice(&[a, c, b, a, d, c]);
            } else {
                indices.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }
        (positions, indices)
    }

    fn triangles(positions: &[Vec3], indices: &[u32]) -> Vec<[Vec3; 3]> {
        indices.chunks_exact(3).map(|t| [positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]]).collect()
    }

    /// Refined, no edge is longer than asked, the surface does not crack
    /// open, and the solid is the same solid.
    #[test]
    fn refining_splits_long_edges_and_keeps_the_solid_closed() {
        let (positions, indices) = boxed(vec3(0.0, 0.0, 0.0), vec3(0.1, 0.01, 0.002), false);
        let mesh = tequila_render_core::scene::Mesh::new(positions, indices);
        let fine = refined(&mesh, 0.001, 1_000_000);
        let longest = fine
            .indices
            .chunks_exact(3)
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .map(|(a, b)| (fine.positions[a as usize] - fine.positions[b as usize]).length())
            .fold(0.0f32, f32::max);
        assert!(longest <= 0.001 + 1e-6, "an edge {longest} long is left");
        assert!(closed(&triangles(&fine.positions, &fine.indices)), "refining cracked the box open");
        let volume = |m: &tequila_render_core::scene::Mesh| {
            m.indices
                .chunks_exact(3)
                .map(|t| {
                    let [a, b, c] = [t[0], t[1], t[2]].map(|i| m.positions[i as usize]);
                    f64::from(a.dot(b.cross(c))) / 6.0
                })
                .sum::<f64>()
        };
        assert!((volume(&fine) / volume(&mesh) - 1.0).abs() < 1e-4, "the solid changed");
        assert!(refined(&mesh, 0.0001, 5_000).indices.len() / 3 < 20_000, "the budget was not kept");
    }

    /// A box of glass, hollow, and a liquid in it modelled a millimetre into
    /// its walls and floor and filled two thirds up: the liquid's side
    /// against the wall meets the glass, its surface the air; the glass's
    /// wall under the liquid is no boundary, above it glass meeting air.
    #[test]
    fn a_liquid_meets_its_glass_and_the_glass_wall_under_it_is_no_boundary() {
        use std::sync::Arc;
        use tequila_core::ids::{InstanceId, MaterialId, MeshId};
        use tequila_materials::material_type::MaterialType;
        use tequila_render_core::environment::Environment;
        use tequila_render_core::scene::{Instance, Mesh};
        let (mut glass_positions, mut glass_indices) = boxed(vec3(-0.03, 0.0, -0.03), vec3(0.03, 0.08, 0.03), false);
        let (cavity, cavity_indices) = boxed(vec3(-0.027, 0.003, -0.027), vec3(0.027, 0.077, 0.027), true);
        glass_positions.extend(cavity);
        glass_indices.extend(cavity_indices.iter().map(|i| i + 8));
        let (liquid_positions, liquid_indices) = boxed(vec3(-0.028, 0.002, -0.028), vec3(0.028, 0.05, 0.028), false);
        let scene = SceneSnapshot::new(
            vec![Arc::new(Mesh::new(glass_positions, glass_indices)), Arc::new(Mesh::new(liquid_positions, liquid_indices))],
            vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)), Instance::new(InstanceId(1), MeshId(1), MaterialId(1))],
            vec![MaterialType::GlassSolid.defaults(), MaterialType::Liquid.defaults()],
            Arc::new(Environment::uniform(tequila_core::spectrum::Spectrum::splat(0.5))),
        );
        let found = attributes(&scene);
        let at = |outside: &Outside, p: Vec3| {
            let nearest = outside.mesh.positions.iter().enumerate().min_by(|a, b| (*a.1 - p).length().total_cmp(&(*b.1 - p).length())).unwrap();
            outside.values[nearest.0]
        };
        let liquid = found.get(&1).expect("the liquid meets its glass");
        let glass_ior = MaterialType::GlassSolid.defaults().specular_ior;
        let side = at(liquid, vec3(0.028, 0.025, 0.0));
        assert!((side[0] - (glass_ior - 1.0)).abs() < 1e-4 && side[1] == 0.0, "the liquid's side reads {side:?}");
        let top = at(liquid, vec3(0.0, 0.05, 0.0));
        assert_eq!((top[0], top[1]), (0.0, 0.0), "the liquid's surface meets the air");
        let glass = found.get(&0).expect("the glass meets its liquid");
        let under = at(glass, vec3(0.027, 0.025, 0.0));
        assert_eq!(under[1], 1.0, "the wall under the liquid reads {under:?}");
        let above = at(glass, vec3(0.027, 0.065, 0.0));
        assert_eq!((above[0], above[1]), (0.0, 0.0), "the wall above the liquid reads {above:?}");
        let outer = at(glass, vec3(0.03, 0.025, 0.0));
        assert_eq!((outer[0], outer[1]), (0.0, 0.0), "the glass's outside reads {outer:?}");
    }

    /// A box knows what is inside it, edges and corners included; an open
    /// box is not closed, a whole one is.
    #[test]
    fn a_solid_knows_its_inside() {
        let (positions, indices) = boxed(vec3(-1.0, -1.0, -1.0), vec3(1.0, 1.0, 1.0), false);
        let solid = Solid::new(triangles(&positions, &indices));
        assert!(solid.contains(Vec3::ZERO));
        assert!(solid.contains(vec3(0.99, 0.99, -0.99)), "near a corner");
        assert!(solid.contains(vec3(0.0, 0.0, 0.999)), "near a face");
        assert!(!solid.contains(vec3(1.01, 0.0, 0.0)));
        assert!(!solid.contains(vec3(5.0, 5.0, 5.0)));
        assert!(closed(&triangles(&positions, &indices)));
        assert!(!closed(&triangles(&positions, &indices[6..])), "a box with its lid off");
    }
}
