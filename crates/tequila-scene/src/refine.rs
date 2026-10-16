//! Making a coarse model finer: the Improve model window.
//!
//! A STEP file is divided at a chord tolerance relative to the part's size,
//! so on a big part a small fillet is a few flat strips; an STL or an OBJ is
//! whatever its exporter wrote. There are two ways to add resolution:
//!
//! - divide the exact CAD surfaces again, finer (Re-Tessellate, from the
//!   B-rep kept with the project): exact, and only for CAD models;
//! - [`subdivide`] here, for any mesh: every triangle becomes four, and each
//!   new point is pushed out onto the surface the vertex normals describe.
//!
//! The push is after Phong tessellation (Boubekeur and Alexa, 2008), solved
//! for every side of an edge at once: the edge's midpoint is moved toward
//! where the tangent planes of all the normals at the edge's two ends meet,
//! as little as it can be, and half way (see [`SHAPE`]). A flat face has one plane and the midpoint is
//! already in it, so flat stays flat; a faceted cylinder's planes turn, and
//! the midpoint moves out to the round. Along a sharp edge, where the mesh
//! keeps a vertex for each side, both sides' planes are solved together, so
//! the new point is one point and the edge does not crack; a circle where a
//! cap meets a wall comes out round in the cap's plane. The original
//! vertices never move, so a CAD model's dimensions stay what they were.

use crate::command::{reserve_keys, Command, ImportPayload};
use crate::geometry::{Mesh, SubMesh};
use crate::reimport::{NodeUpdate, ReimportPayload};
use crate::tree::{NodeId, NodeKind};
use crate::SceneDocument;
use tequila_core::math::{Vec2, Vec3, Vec4};
use std::collections::HashMap;

/// How far a new point goes toward where the tangent planes meet. They meet
/// beyond the curve: for a circle whose normals turn by an angle t between
/// the edge's ends, the arc's midpoint is cos(t/2) / (1 + cos(t/2)) of the
/// way, which is a half for any but the coarsest mesh (0.49 at 30 degrees, 0.48 at
/// 45). The full move bulged an eight-sided cylinder 4% past its radius.
const SHAPE: f32 = 0.5;

/// The most triangles a subdivision may make, past which a model stops
/// being something the viewport can turn.
pub const MOST_TRIANGLES: usize = 20_000_000;

/// How many triangles `levels` subdivisions make of `triangles`.
pub fn subdivided_count(triangles: usize, levels: u32) -> usize {
    triangles.saturating_mul(4usize.saturating_pow(levels))
}

/// `mesh`, every triangle split in four `levels` times, new points on the
/// surface its normals describe. A mesh with no normals is given smooth ones
/// first, with edges sharper than `crease_degrees` kept sharp.
pub fn subdivide(mesh: &Mesh, levels: u32, crease_degrees: f32) -> Mesh {
    let mut out = mesh.clone();
    if out.normals.len() != out.positions.len() {
        out.normals.clear();
        out.generate_normals(crease_degrees.to_radians());
    }
    let had_tangents = !out.tangents.is_empty();
    for _ in 0..levels {
        out = once(&out);
    }
    out.tangents.clear();
    if had_tangents {
        out.generate_tangents();
    }
    out.recompute_bounds();
    out
}

/// `mesh` with its shading worked out again: smooth across edges gentler
/// than `crease_degrees`, sharp across steeper ones. For a mesh whose file
/// had no normals, or faceted ones; a CAD part's own normals are better than
/// any this makes.
pub fn reshade(mesh: &Mesh, crease_degrees: f32) -> Mesh {
    let mut out = mesh.clone();
    // Shared corners first, so the angle test sees which faces meet: an STL
    // stores every triangle's corners apart.
    out.normals.clear();
    out.tangents.clear();
    out.join_identical_vertices();
    out.generate_normals(crease_degrees.to_radians());
    if !out.uv_sets.is_empty() {
        out.generate_tangents();
    }
    out.recompute_bounds();
    out
}

/// One subdivision.
fn once(mesh: &Mesh) -> Mesh {
    // Where each vertex is, as a number shared by every vertex at that
    // point: the mesh keeps a vertex per side of a sharp edge, and the two
    // sides' new points have to be one point.
    let mut place_of: HashMap<[u32; 3], u32> = HashMap::with_capacity(mesh.positions.len());
    let place: Vec<u32> = mesh
        .positions
        .iter()
        .map(|p| {
            let next = place_of.len() as u32;
            *place_of.entry([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]).or_insert(next)
        })
        .collect();
    let triangles = mesh.indices.len() / 3;

    // Every geometric edge: the normals at its ends, from every side.
    let mut edges: HashMap<(u32, u32), Planes> = HashMap::with_capacity(triangles * 2);
    for t in 0..triangles {
        for (a, b) in corners(&mesh.indices, t) {
            // Filed by the edge's ends in order, each end's normal with it.
            let (a, b) = if place[a as usize] <= place[b as usize] { (a as usize, b as usize) } else { (b as usize, a as usize) };
            let planes = edges.entry((place[a], place[b])).or_default();
            planes.add(mesh.normals[a], mesh.normals[b]);
        }
    }
    // The edge's ends, by the first vertex seen at each place, and how far
    // its midpoint moves.
    let mut first_at: Vec<u32> = vec![u32::MAX; place_of.len()];
    for (v, &p) in place.iter().enumerate() {
        if first_at[p as usize] == u32::MAX {
            first_at[p as usize] = v as u32;
        }
    }
    let lift: HashMap<(u32, u32), Vec3> = edges
        .iter()
        .map(|(&(pa, pb), planes)| {
            let (a, b) = (mesh.positions[first_at[pa as usize] as usize], mesh.positions[first_at[pb as usize] as usize]);
            ((pa, pb), planes.lift(a, b) * SHAPE)
        })
        .collect();

    let mut out = Mesh::new(mesh.positions.clone(), Vec::with_capacity(mesh.indices.len() * 4));
    out.normals = mesh.normals.clone();
    out.uv_sets = mesh.uv_sets.clone();
    out.colors = mesh.colors.clone();
    out.submeshes = mesh
        .submeshes
        .iter()
        .map(|s| SubMesh { first_index: s.first_index * 4, index_count: s.index_count * 4, slot: s.slot })
        .collect();
    // One new vertex per side of each edge: the same point, that side's
    // normal and texture coordinates.
    let mut middle_of: HashMap<(u32, u32), u32> = HashMap::with_capacity(triangles * 2);
    let mut middle = |out: &mut Mesh, a: u32, b: u32| -> u32 {
        let key = ordered(a, b);
        *middle_of.entry(key).or_insert_with(|| {
            let (ia, ib) = (a as usize, b as usize);
            let at = (mesh.positions[ia] + mesh.positions[ib]) * 0.5 + lift[&ordered(place[ia], place[ib])];
            out.positions.push(at);
            let n = mesh.normals[ia] + mesh.normals[ib];
            out.normals.push(if n.length() > 1e-6 { n.normalize() } else { mesh.normals[ia] });
            for (set, uvs) in out.uv_sets.iter_mut().zip(&mesh.uv_sets) {
                set.push(mid2(uvs[ia], uvs[ib]));
            }
            if !mesh.colors.is_empty() {
                out.colors.push(mid4(mesh.colors[ia], mesh.colors[ib]));
            }
            (out.positions.len() - 1) as u32
        })
    };
    for t in 0..triangles {
        let [a, b, c] = [mesh.indices[3 * t], mesh.indices[3 * t + 1], mesh.indices[3 * t + 2]];
        let ab = middle(&mut out, a, b);
        let bc = middle(&mut out, b, c);
        let ca = middle(&mut out, c, a);
        // Wound as the original, so every new triangle faces the same way.
        out.indices.extend_from_slice(&[a, ab, ca, ab, b, bc, ca, bc, c, ab, bc, ca]);
    }

    // The model's own edges, divided where their triangles were.
    for pair in mesh.edge_lines.chunks_exact(2) {
        let (p, q) = (pair[0], pair[1]);
        let at = |v: Vec3| place_of.get(&[v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]).copied();
        let m = match (at(p), at(q)) {
            (Some(a), Some(b)) => (p + q) * 0.5 + lift.get(&ordered(a, b)).copied().unwrap_or(Vec3::ZERO),
            _ => (p + q) * 0.5,
        };
        out.edge_lines.extend_from_slice(&[p, m, m, q]);
    }
    out.recompute_bounds();
    out
}

/// The tangent planes at an edge's two ends, from every side of it.
#[derive(Default)]
struct Planes {
    at_a: Vec<Vec3>,
    at_b: Vec<Vec3>,
}

impl Planes {
    fn add(&mut self, a: Vec3, b: Vec3) {
        let distinct = |list: &Vec<Vec3>, n: Vec3| !list.iter().any(|m| m.dot(n) > 0.9999);
        if distinct(&self.at_a, a) {
            self.at_a.push(a);
        }
        if distinct(&self.at_b, b) {
            self.at_b.push(b);
        }
    }

    /// How far the midpoint of `a`-`b` moves to lie, as nearly as it can, in
    /// every plane through `a` along a normal at `a` and through `b` along
    /// one at `b`, and not along the edge: least squares over the planes,
    /// with the smallest move where they leave it free.
    fn lift(&self, a: Vec3, b: Vec3) -> Vec3 {
        let m = (a + b) * 0.5;
        let length = (b - a).length();
        if length <= 0.0 {
            return Vec3::ZERO;
        }
        // Normal equations: sum of n n^T, and n (n . (end - m)).
        let mut matrix = [[0.0f64; 3]; 3];
        let mut rhs = [0.0f64; 3];
        let mut add = |n: Vec3, through: Vec3, weight: f64| {
            let n = [n.x as f64, n.y as f64, n.z as f64];
            let d = n[0] * (through.x - m.x) as f64 + n[1] * (through.y - m.y) as f64 + n[2] * (through.z - m.z) as f64;
            for i in 0..3 {
                for j in 0..3 {
                    matrix[i][j] += weight * n[i] * n[j];
                }
                rhs[i] += weight * n[i] * d;
            }
        };
        for &n in &self.at_a {
            add(n, a, 1.0);
        }
        for &n in &self.at_b {
            add(n, b, 1.0);
        }
        // Not along the edge, and a little toward not moving at all, where
        // the planes do not decide.
        let along = (b - a) * (1.0 / length);
        add(along, m, 1.0);
        for (i, row) in matrix.iter_mut().enumerate() {
            row[i] += 1e-4;
        }
        let Some(d) = solve(matrix, rhs) else { return Vec3::ZERO };
        let lift = Vec3 { x: d[0] as f32, y: d[1] as f32, z: d[2] as f32 };
        // A seam where the normals turn right round can ask for a jump;
        // half the edge is as far as a real surface bends between two points.
        let most = 0.5 * length;
        if lift.length() > most {
            lift * (most / lift.length())
        } else {
            lift
        }
    }
}

/// `matrix x = rhs` for a 3 by 3, by Cramer's rule; `None` when singular.
fn solve(m: [[f64; 3]; 3], r: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: &[[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d = det(&m);
    if d.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, value) in out.iter_mut().enumerate() {
        let mut c = m;
        for i in 0..3 {
            c[i][k] = r[i];
        }
        *value = det(&c) / d;
    }
    Some(out)
}

fn corners(indices: &[u32], t: usize) -> [(u32, u32); 3] {
    let [a, b, c] = [indices[3 * t], indices[3 * t + 1], indices[3 * t + 2]];
    [(a, b), (b, c), (c, a)]
}

fn ordered(a: u32, b: u32) -> (u32, u32) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

fn mid2(a: Vec2, b: Vec2) -> Vec2 {
    Vec2 { x: (a.x + b.x) * 0.5, y: (a.y + b.y) * 0.5 }
}

fn mid4(a: Vec4, b: Vec4) -> Vec4 {
    Vec4 { x: (a.x + b.x) * 0.5, y: (a.y + b.y) * 0.5, z: (a.z + b.z) * 0.5, w: (a.w + b.w) * 0.5 }
}

/// The command that gives every part in `parts` the mesh `make` makes of its
/// own, keeping everything else about it: one update, one step to undo. A
/// mesh several parts share is remade once and stays shared. `None` when
/// none of `parts` is a part with a mesh.
pub fn replace_meshes(document: &SceneDocument, parts: &[NodeId], make: impl Fn(&Mesh) -> Mesh) -> Option<Command> {
    let mut by_mesh: Vec<(crate::MeshKey, Vec<NodeId>)> = Vec::new();
    for &id in parts {
        let Some(node) = document.tree.get(id) else { continue };
        let NodeKind::Part { mesh } = node.kind else { continue };
        match by_mesh.iter_mut().find(|(key, _)| *key == mesh) {
            Some((_, nodes)) => nodes.push(id),
            None => by_mesh.push((mesh, vec![id])),
        }
    }
    if by_mesh.is_empty() {
        return None;
    }
    let (keys, _, _) = reserve_keys(document, by_mesh.len(), 0, 0);
    let mut meshes = Vec::with_capacity(by_mesh.len());
    let mut updates = Vec::new();
    for ((old, nodes), key) in by_mesh.iter().zip(keys) {
        let geometry = document.geometry.get(*old)?;
        meshes.push((key, make(geometry)));
        for &id in nodes {
            let node = document.tree.get(id)?;
            updates.push(NodeUpdate {
                node: id,
                kind: NodeKind::Part { mesh: key },
                transform: None,
                material: node.material,
                metadata: node.metadata.clone(),
                path_key: node.path_key.clone(),
            });
        }
    }
    Some(Command::Reimport {
        payload: Box::new(ReimportPayload {
            added: ImportPayload { meshes, textures: Vec::new(), materials: Vec::new(), nodes: Vec::new(), roots: Vec::new() },
            updates,
            hidden: Vec::new(),
        }),
    })
}

/// Every part under `roots`, the roots included.
pub fn parts_under(document: &SceneDocument, roots: &[NodeId]) -> Vec<NodeId> {
    let mut out = Vec::new();
    for &root in roots {
        for id in document.tree.descendants_and_self(root) {
            if matches!(document.tree.get(id).map(|n| &n.kind), Some(NodeKind::Part { .. })) && !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// A cylinder of `segments` flat sides, radius 1, height 1, with caps:
    /// a vertex per side of each sharp edge, as a CAD export has.
    fn cylinder(segments: usize) -> Mesh {
        let (mut positions, mut normals, mut indices) = (Vec::new(), Vec::new(), Vec::new());
        let ring = |k: usize| {
            let a = k as f32 / segments as f32 * std::f32::consts::TAU;
            (a.cos(), a.sin())
        };
        // The wall: smooth normals around, sharp against the caps.
        for k in 0..segments {
            let (x, z) = ring(k);
            positions.push(vec3(x, 0.0, z));
            normals.push(vec3(x, 0.0, z));
            positions.push(vec3(x, 1.0, z));
            normals.push(vec3(x, 0.0, z));
        }
        for k in 0..segments {
            let (a, b) = (2 * k as u32, 2 * ((k + 1) % segments) as u32);
            indices.extend_from_slice(&[a, a + 1, b, b, a + 1, b + 1]);
        }
        // The caps, flat, their own vertices on the same rims.
        for (y, up) in [(0.0f32, -1.0f32), (1.0, 1.0)] {
            let centre = positions.len() as u32;
            positions.push(vec3(0.0, y, 0.0));
            normals.push(vec3(0.0, up, 0.0));
            for k in 0..segments {
                let (x, z) = ring(k);
                positions.push(vec3(x, y, z));
                normals.push(vec3(0.0, up, 0.0));
            }
            for k in 0..segments as u32 {
                let (a, b) = (centre + 1 + k, centre + 1 + (k + 1) % segments as u32);
                if up > 0.0 {
                    indices.extend_from_slice(&[centre, b, a]);
                } else {
                    indices.extend_from_slice(&[centre, a, b]);
                }
            }
        }
        Mesh::new(positions, indices).with_normals(normals)
    }

    /// How far the wall and the rims are from the true radius: the points
    /// out at the edge, not the caps' inner points between centre and rim.
    fn radial_error(mesh: &Mesh) -> f32 {
        mesh.positions
            .iter()
            .filter(|p| (p.x * p.x + p.z * p.z).sqrt() > 0.8)
            .map(|p| ((p.x * p.x + p.z * p.z).sqrt() - 1.0).abs())
            .fold(0.0, f32::max)
    }

    /// Subdividing a faceted cylinder rounds it: the new points on the wall
    /// and on the rims where the caps meet it move out toward the true
    /// radius, the caps stay flat, and nothing that was there moves.
    #[test]
    fn a_faceted_cylinder_comes_out_round() {
        let coarse = cylinder(8);
        let fine = subdivide(&coarse, 1, 30.0);
        assert_eq!(fine.triangle_count(), coarse.triangle_count() * 4);
        assert_eq!(&fine.positions[..coarse.positions.len()], &coarse.positions[..], "an original vertex moved");
        // Plain midpoints would sit at cos(pi/8) of the radius: 7.6% in.
        let flat = 1.0 - (std::f32::consts::PI / 8.0).cos();
        let error = radial_error(&fine);
        assert!(error < flat * 0.25, "the wall stayed faceted: {error} against {flat} for straight midpoints");
        for p in &fine.positions {
            assert!(p.y.abs() < 1e-5 || (p.y - 1.0).abs() < 1e-5 || (p.x * p.x + p.z * p.z).sqrt() > 0.9, "a point left the caps' planes: {p:?}");
        }
        // Twice, closer still.
        assert!(radial_error(&subdivide(&coarse, 2, 30.0)) <= error);
    }

    /// Along a sharp edge the two sides' new vertices are one point, so a
    /// closed model stays closed: every edge between two places is used by
    /// exactly two triangles.
    #[test]
    fn a_sharp_edge_does_not_crack() {
        let fine = subdivide(&cylinder(6), 2, 30.0);
        let mut place_of: HashMap<[u32; 3], u32> = HashMap::new();
        let place: Vec<u32> = fine
            .positions
            .iter()
            .map(|p| {
                let next = place_of.len() as u32;
                *place_of.entry([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]).or_insert(next)
            })
            .collect();
        let mut uses: HashMap<(u32, u32), usize> = HashMap::new();
        for t in 0..fine.triangle_count() {
            for (a, b) in corners(&fine.indices, t) {
                *uses.entry(ordered(place[a as usize], place[b as usize])).or_default() += 1;
            }
        }
        let open: Vec<_> = uses.iter().filter(|(_, &n)| n != 2).collect();
        assert!(open.is_empty(), "{} edges are not shared by two triangles: {:?}", open.len(), open.iter().take(4).collect::<Vec<_>>());
    }

    /// A flat face stays exactly flat: a square's new points stay in its
    /// plane however many times it is divided.
    #[test]
    fn flat_stays_flat() {
        let square = Mesh::new(
            vec![vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0)],
            vec![0, 2, 1, 0, 3, 2],
        )
        .with_normals(vec![Vec3::Y; 4]);
        let fine = subdivide(&square, 3, 30.0);
        assert_eq!(fine.triangle_count(), 2 * 64);
        assert!(fine.positions.iter().all(|p| p.y.abs() < 1e-6), "the square bent");
    }

    /// Material groups keep their triangles: each range is four times as
    /// long and starts four times as far in.
    #[test]
    fn material_groups_keep_their_triangles() {
        let mut mesh = cylinder(8);
        let wall = 8 * 6;
        mesh.submeshes = vec![
            SubMesh { first_index: 0, index_count: wall as u32, slot: 0 },
            SubMesh { first_index: wall as u32, index_count: (mesh.indices.len() - wall) as u32, slot: 1 },
        ];
        let fine = subdivide(&mesh, 1, 30.0);
        assert_eq!(fine.submeshes[0].index_count, 4 * wall as u32);
        assert_eq!(fine.submeshes[1].first_index, 4 * wall as u32);
        assert_eq!(fine.submeshes[1].first_index + fine.submeshes[1].index_count, fine.indices.len() as u32);
    }

    /// Improving a model is one step: every part gets the finer mesh and
    /// keeps its material, and undo gives the old mesh back.
    #[test]
    fn improving_is_one_undoable_step_that_keeps_materials() {
        let mut document = crate::demo::demo_document(crate::demo::DemoMaterial::Plastic, 0.2);
        let roots = document.tree.roots().to_vec();
        let parts = parts_under(&document, &roots);
        assert!(!parts.is_empty());
        let before: Vec<_> = parts.iter().map(|&id| document.tree.get(id).unwrap().clone()).collect();
        let triangles = |document: &SceneDocument, id: NodeId| match document.tree.get(id).unwrap().kind {
            NodeKind::Part { mesh } => document.geometry.get(mesh).unwrap().triangle_count(),
            _ => 0,
        };
        let counts: Vec<usize> = parts.iter().map(|&id| triangles(&document, id)).collect();
        let command = replace_meshes(&document, &parts, |mesh| subdivide(mesh, 1, 30.0)).expect("parts to improve");
        let mut bus = crate::CommandBus::new();
        bus.apply(&mut document, command).unwrap();
        for ((&id, old), count) in parts.iter().zip(&before).zip(&counts) {
            let node = document.tree.get(id).unwrap();
            assert_eq!(node.material, old.material, "a part lost its material");
            assert_eq!(node.name, old.name);
            assert_eq!(triangles(&document, id), count * 4);
        }
        bus.undo(&mut document).unwrap();
        for (&id, old) in parts.iter().zip(&before) {
            assert_eq!(document.tree.get(id).unwrap().kind, old.kind, "undo did not give the old mesh back");
        }
    }

    /// An STL's triangles each with corners of their own, and no normals:
    /// shading again joins them and keeps the cube's edges sharp.
    #[test]
    fn reshading_joins_corners_and_keeps_sharp_edges() {
        // A unit cube, every triangle with corners of its own.
        let corner = |i: u32| vec3((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32);
        let faces: [[u32; 4]; 6] = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let mut corners_of = Vec::new();
        for [a, b, c, d] in faces {
            corners_of.extend_from_slice(&[a, b, c, a, c, d]);
        }
        let mut loose = Mesh::new(corners_of.iter().map(|&i| corner(i)).collect(), (0..corners_of.len() as u32).collect());
        loose.normals.clear();
        let shaded = reshade(&loose, 30.0);
        assert!(shaded.positions.len() < loose.positions.len(), "the corners were not joined");
        // Every normal is one of the six faces'.
        for n in &shaded.normals {
            let axis = n.x.abs().max(n.y.abs()).max(n.z.abs());
            assert!((axis - 1.0).abs() < 1e-4, "a cube's edge was smoothed: {n:?}");
        }
    }
}
