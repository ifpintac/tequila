//! Measurements: dimensions annotated on the model (`TEQUILA_SPEC.md` §1.11,
//! "in-viewport dimensional annotation against the real-world-scaled
//! model").
//!
//! ## Anchored to parts
//!
//! Each end of a measurement is a point in a part's own space, not in the
//! world. A dimension is *of* something — the width of a housing, the gap
//! between two buttons — and when the housing moves the dimension moves with
//! it and keeps reading true. An end in empty space would be a number that
//! silently stops meaning anything the first time the model is touched.
//!
//! ## Snapping without BREP
//!
//! `docs/03` §6 asks for snapping to BREP edges and vertices. There is no
//! BREP here — proprietary CAD kernels are out of scope — so the edges are
//! recovered from the tessellation: an edge where the surface creases, or
//! where it ends, is where the CAD model had an edge. The interior lines of
//! a tessellated face are an artefact of the tessellator and are never
//! offered: snapping to one would measure to a point that has no meaning in
//! the part.

use crate::document::SceneDocument;
use crate::tree::{NodeId, SceneTree};
use crate::units::SceneUnits;
use tequila_core::math::Vec3;
use tequila_render_core::scene::Mesh;
use std::collections::{HashMap, HashSet};

/// One end of a measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    /// The part it is on. `None` for a point fixed in the world.
    pub node: Option<NodeId>,
    /// In the part's own space, or the world's without one.
    pub point: Vec3,
}

impl Anchor {
    /// Where it is now. `None` when its part has been deleted, and the
    /// measurement with it has nothing left to measure.
    pub fn world(&self, tree: &SceneTree) -> Option<Vec3> {
        match self.node {
            None => Some(self.point),
            Some(node) => tree.contains(node).then(|| tree.world_transform(node).point(self.point)),
        }
    }
}

/// A distance between two points.
#[derive(Clone, Debug, PartialEq)]
pub struct Measurement {
    pub a: Anchor,
    pub b: Anchor,
    pub visible: bool,
}

impl Measurement {
    pub fn new(a: Anchor, b: Anchor) -> Measurement {
        Measurement { a, b, visible: true }
    }

    /// Both ends, in the world.
    pub fn ends(&self, document: &SceneDocument) -> Option<(Vec3, Vec3)> {
        Some((self.a.world(&document.tree)?, self.b.world(&document.tree)?))
    }

    /// The distance, in metres.
    pub fn length(&self, document: &SceneDocument) -> Option<f32> {
        self.ends(document).map(|(a, b)| (b - a).length())
    }

    pub fn is_valid(&self) -> bool {
        self.a.point.is_finite() && self.b.point.is_finite()
    }
}

/// A length in metres, as a person reads it: in the scene's own unit, or
/// metres when it has none, to a sensible number of figures.
pub fn format_length(length: f32, units: &SceneUnits) -> String {
    let unit = units.shown();
    let (value, symbol) = (length as f64 / unit.to_metres(), unit.symbol());
    let magnitude = value.abs();
    let decimals = if magnitude >= 1000.0 {
        0
    } else if magnitude >= 100.0 {
        1
    } else if magnitude >= 1.0 {
        2
    } else {
        3
    };
    format!("{value:.decimals$} {symbol}")
}

/// What a snapped point is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind {
    /// A corner of the model. Preferred over everything else near it.
    Vertex,
    /// The middle of an edge.
    Midpoint,
    /// Somewhere along an edge.
    Edge,
    /// Wherever the pointer is on the surface.
    Surface,
}

impl SnapKind {
    pub fn label(self) -> &'static str {
        match self {
            SnapKind::Vertex => "Vertex",
            SnapKind::Midpoint => "Midpoint",
            SnapKind::Edge => "Edge",
            SnapKind::Surface => "Surface",
        }
    }
}

/// The edges of a mesh that are edges of the part: creases and boundaries.
#[derive(Debug, Default)]
pub struct FeatureEdges {
    /// Each vertex's position-welded index. A CAD mesh is split at every
    /// crease for its normals, so the two sides of an edge are different
    /// vertices at the same place.
    weld: Vec<u32>,
    edges: HashSet<(u32, u32)>,
}

/// How far two faces must turn from each other for their shared edge to
/// count as an edge of the part. Below it a curved surface's tessellation
/// would offer every facet line of a cylinder.
pub const CREASE_DEGREES: f32 = 25.0;

impl FeatureEdges {
    pub fn of(mesh: &Mesh) -> FeatureEdges {
        let bounds = mesh.bounds();
        let size = bounds.diagonal().length().max(1e-12);
        let quantum = size * 1e-6;
        let mut ids: HashMap<(i64, i64, i64), u32> = HashMap::new();
        let weld: Vec<u32> = mesh
            .positions
            .iter()
            .map(|p| {
                let key = (
                    (p.x / quantum).round() as i64,
                    (p.y / quantum).round() as i64,
                    (p.z / quantum).round() as i64,
                );
                let next = ids.len() as u32;
                *ids.entry(key).or_insert(next)
            })
            .collect();

        let mut faces: HashMap<(u32, u32), Vec<Vec3>> = HashMap::new();
        for t in 0..mesh.triangle_count() {
            let [a, b, c] = mesh.triangle(t);
            let (pa, pb, pc) = (
                mesh.positions[a as usize],
                mesh.positions[b as usize],
                mesh.positions[c as usize],
            );
            let Some(normal) = (pb - pa).cross(pc - pa).try_normalize() else {
                continue;
            };
            for (i, j) in [(a, b), (b, c), (c, a)] {
                let (wi, wj) = (weld[i as usize], weld[j as usize]);
                if wi != wj {
                    faces.entry((wi.min(wj), wi.max(wj))).or_default().push(normal);
                }
            }
        }
        let crease = CREASE_DEGREES.to_radians().cos();
        let edges = faces
            .into_iter()
            .filter(|(_, normals)| match normals.as_slice() {
                [a, b] => a.dot(*b) < crease,
                // Open, or more than two faces meeting: an edge either way.
                _ => true,
            })
            .map(|(edge, _)| edge)
            .collect();
        FeatureEdges { weld, edges }
    }

    pub fn is_edge(&self, a: u32, b: u32) -> bool {
        let (Some(&wa), Some(&wb)) = (self.weld.get(a as usize), self.weld.get(b as usize)) else {
            return false;
        };
        self.edges.contains(&(wa.min(wb), wa.max(wb)))
    }

    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// The edges as segment end points in pairs, for drawing: what a mesh
    /// with no B-rep edges of its own shows as its edges.
    pub fn segments(&self, mesh: &Mesh) -> Vec<Vec3> {
        let mut at: HashMap<u32, Vec3> = HashMap::with_capacity(self.weld.len());
        for (vertex, &id) in self.weld.iter().enumerate() {
            at.entry(id).or_insert(mesh.positions[vertex]);
        }
        let mut out = Vec::with_capacity(self.edges.len() * 2);
        for (a, b) in &self.edges {
            if let (Some(&pa), Some(&pb)) = (at.get(a), at.get(b)) {
                out.extend_from_slice(&[pa, pb]);
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

/// Points worth snapping to near `local`, on triangle `primitive`, in the
/// mesh's own space: the triangle's corners, and the nearest point and the
/// middle of each of its sides that is an edge of the part. The caller picks
/// among them by how close each lands to the pointer on screen, which is the
/// distance that matters to a hand.
pub fn snap_candidates(mesh: &Mesh, features: &FeatureEdges, primitive: usize, local: Vec3) -> Vec<(Vec3, SnapKind)> {
    if primitive >= mesh.triangle_count() {
        return Vec::new();
    }
    let [a, b, c] = mesh.triangle(primitive);
    let mut out: Vec<(Vec3, SnapKind)> = [a, b, c]
        .iter()
        .map(|&i| (mesh.positions[i as usize], SnapKind::Vertex))
        .collect();
    for (i, j) in [(a, b), (b, c), (c, a)] {
        if !features.is_edge(i, j) {
            continue;
        }
        let (p, q) = (mesh.positions[i as usize], mesh.positions[j as usize]);
        out.push(((p + q) * 0.5, SnapKind::Midpoint));
        let along = q - p;
        let t = ((local - p).dot(along) / along.length_squared().max(1e-30)).clamp(0.0, 1.0);
        out.push((p + along * t, SnapKind::Edge));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;
    use tequila_core::units::LengthUnit;

    /// A unit cube, split at every corner as CAD exports are: each face has
    /// its own four vertices.
    fn cube() -> Mesh {
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        let faces = [
            (vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)),
            (vec3(0.0, 0.0, -1.0), vec3(-1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0)),
            (vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, -1.0), vec3(0.0, 1.0, 0.0)),
            (vec3(-1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 0.0)),
            (vec3(0.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, -1.0)),
            (vec3(0.0, -1.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0)),
        ];
        for (n, u, v) in faces {
            let base = positions.len() as u32;
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                positions.push(n + u * su + v * sv);
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Mesh::new(positions, indices)
    }

    /// The twelve edges of a cube are edges; the diagonals splitting its
    /// faces into triangles are not.
    #[test]
    fn a_cubes_edges_are_its_edges_and_nothing_else() {
        let mesh = cube();
        let features = FeatureEdges::of(&mesh);
        assert_eq!(features.len(), 12, "found {} edges", features.len());
        // The first face's diagonal, from its corner 0 to corner 2.
        assert!(!features.is_edge(0, 2), "a face's diagonal was offered as an edge");
        assert!(features.is_edge(0, 1));
    }

    /// Snapping near a corner offers the corner; near the middle of a side,
    /// the side's middle; and never the face's diagonal.
    #[test]
    fn candidates_are_corners_and_real_edges() {
        let mesh = cube();
        let features = FeatureEdges::of(&mesh);
        // Triangle 0 is corners 0, 1, 2 of the +Z face.
        let near_side = vec3(0.1, -0.95, 1.0);
        let candidates = snap_candidates(&mesh, &features, 0, near_side);
        let vertices = candidates.iter().filter(|(_, k)| *k == SnapKind::Vertex).count();
        assert_eq!(vertices, 3);
        assert!(candidates
            .iter()
            .any(|(p, k)| *k == SnapKind::Midpoint && (*p - vec3(0.0, -1.0, 1.0)).length() < 1e-5));
        assert!(candidates
            .iter()
            .any(|(p, k)| *k == SnapKind::Edge && (*p - vec3(0.1, -1.0, 1.0)).length() < 1e-5));
        // The diagonal's middle is the face's centre, which is not offered.
        assert!(!candidates.iter().any(|(p, k)| *k == SnapKind::Midpoint && p.length() < 1.01 && p.x.abs() < 1e-5 && p.y.abs() < 1e-5));
    }

    /// Lengths are metres; the scene's unit is only how they read.
    #[test]
    fn lengths_read_in_the_scenes_unit() {
        let mm = SceneUnits::new(LengthUnit::Millimetre);
        assert_eq!(format_length(0.0123456, &mm), "12.35 mm");
        assert_eq!(format_length(1.2346, &mm), "1235 mm");
        assert_eq!(format_length(0.0508, &SceneUnits::new(LengthUnit::Inch)), "2.00 in");
        let unspecified = SceneUnits::default();
        assert_eq!(format_length(0.5, &unspecified), "0.500 m");
    }

    /// An end on a part follows the part.
    #[test]
    fn an_anchor_follows_its_part() {
        let mut document = crate::demo::demo_document(crate::demo::DemoMaterial::Plastic, 0.3);
        let node = document.tree.iter().find(|n| n.mesh().is_some()).map(|n| n.id).unwrap();
        let anchor = Anchor {
            node: Some(node),
            point: vec3(1.0, 0.0, 0.0),
        };
        let before = anchor.world(&document.tree).unwrap();
        document.tree.get_mut(node).unwrap().transform =
            tequila_core::math::Transform::translation(vec3(0.0, 2.0, 0.0)) * document.tree.get(node).unwrap().transform;
        let after = anchor.world(&document.tree).unwrap();
        assert!((after - before - vec3(0.0, 2.0, 0.0)).length() < 1e-5);
    }
}
