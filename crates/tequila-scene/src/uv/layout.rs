//! The island model: a part's UV layout as the editor draws it and every
//! operation here reads it.
//!
//! Built from the mesh and one of its UV sets in a single pass over its
//! triangles, cheap enough to make again after every edit: [`UvLayout::new`].
//!
//! Its pieces, all numbered so they can be named in a command and found
//! again after one:
//!
//! - **Welded vertices.** The mesh's vertices at one point of the part,
//!   however many the mesh splits it into for its normals or its layout,
//!   share one welded index ([`UvLayout::weld`]). Edges and seams are
//!   edges between welded vertices: edges of the surface itself.
//! - **UV vertices.** Each triangle corner is on one UV vertex: corners at
//!   one welded vertex, joined to each other across edges whose two sides
//!   agree, at one place on the layout ([`UvLayout::corner_vertex`]). A
//!   vertex of the part on a seam is two UV vertices or more, one each side.
//! - **Islands.** Triangles joined across edges whose two sides agree: the
//!   pieces the layout is in. Numbered in the order of their first
//!   triangle, which every operation that keeps the pieces keeps.
//! - **Edges.** Every edge of the layout once as it is drawn: inside an
//!   island once, on a seam once for each side (each the other's twin), on
//!   the edge of the part once.

use super::flatten::{facing, surface_area};
use crate::geometry::Mesh;
use std::collections::HashMap;
use tequila_core::math::{vec2, Vec2, Vec3};

/// Each vertex's welded index: vertices at the same point of the part, to a
/// millionth of its size, share one, numbered in the order they come.
pub fn weld(positions: &[Vec3]) -> Vec<u32> {
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for p in positions {
        low = low.min(*p);
        high = high.max(*p);
    }
    let quantum = ((high - low).length() * 1e-6).max(1e-12);
    let mut ids: HashMap<(i64, i64, i64), u32> = HashMap::with_capacity(positions.len());
    positions
        .iter()
        .map(|p| {
            let key = ((p.x / quantum).round() as i64, (p.y / quantum).round() as i64, (p.z / quantum).round() as i64);
            let next = ids.len() as u32;
            *ids.entry(key).or_insert(next)
        })
        .collect()
}

/// What an edge of the layout is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    /// Inside an island: the faces on either side agree.
    Inner,
    /// One side of a seam: the surface goes on across it, but the layout is
    /// split there, and the other side is drawn somewhere else - its twin.
    Seam,
    /// The edge of the part itself: no face beyond it.
    Boundary,
}

/// One edge of the layout, as it is drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutEdge {
    /// Its two ends, as UV vertices.
    pub ends: [u32; 2],
    /// The mesh vertices at its ends on this side: what names the edge in a
    /// command ([`super::MeshEdge`]).
    pub vertices: [u32; 2],
    /// A triangle on this side of it.
    pub triangle: u32,
    pub island: u32,
    pub kind: EdgeKind,
    /// For a seam, the other side of it.
    pub twin: Option<u32>,
}

/// A point of the layout: a place, and the corners on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvVertex {
    pub uv: Vec2,
    /// A mesh vertex on it, the first of its corners'.
    pub vertex: u32,
    /// The welded vertex of the part it is a place of.
    pub weld: u32,
    pub island: u32,
}

/// One piece of the layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Island {
    /// Its triangles, in order.
    pub triangles: Vec<u32>,
    /// Its UV vertices, in order.
    pub uv_vertices: Vec<u32>,
    /// The mesh vertices its triangles use, in order.
    pub vertices: Vec<u32>,
    /// Its bounding box on the layout.
    pub min: Vec2,
    pub max: Vec2,
    /// The middle of its box: what it turns and scales about.
    pub centre: Vec2,
    /// The middle of its area.
    pub centroid: Vec2,
    /// Its area on the layout.
    pub uv_area: f32,
    /// Its area on the part, in square metres of the part's own space.
    pub surface_area: f32,
    /// The direction its area spreads along, in radians from the u axis
    /// (-pi/2 to pi/2]: its principal axis.
    pub axis: f32,
    /// How much more it spreads along its axis than across it, 1 or more:
    /// near 1, a round or square island has no axis to speak of.
    pub elongation: f32,
    /// The direction of the long side of its tightest bounding rectangle,
    /// in radians from the u axis (-pi/2 to pi/2].
    pub box_angle: f32,
    /// Laid out mirrored: the artwork reads backwards on it.
    pub mirrored: bool,
    /// Its edge, as loops of UV vertices, the outer one first; the inner
    /// ones go round holes.
    pub boundary: Vec<Vec<u32>>,
    /// Its edges, a run of [`UvLayout::edges`].
    pub edges: std::ops::Range<usize>,
}

impl Island {
    /// Its texel density: area on the layout to area on the part.
    pub fn density(&self) -> f32 {
        if self.surface_area > 0.0 { self.uv_area / self.surface_area } else { 0.0 }
    }

    /// Metres of the part per unit of the layout, across this island.
    pub fn units_per_uv(&self) -> f32 {
        if self.uv_area > 0.0 { (self.surface_area / self.uv_area).sqrt() } else { 0.0 }
    }

    /// The direction it is straightened by: its principal axis when it has
    /// one clearly, else the side of its tightest box - a square has no
    /// principal axis, but it has sides.
    pub fn straightening_angle(&self) -> f32 {
        if self.elongation >= 1.25 { self.axis } else { self.box_angle }
    }
}

/// A triangle under a point of the layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangleHit {
    pub triangle: u32,
    pub island: u32,
    /// The point's weights on the triangle's three corners, in order.
    pub barycentric: [f32; 3],
}

/// Where a point of the layout lands on the part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceHit {
    pub triangle: u32,
    pub island: u32,
    pub barycentric: [f32; 3],
    /// On the part, in its own space.
    pub position: Vec3,
    /// The part's shading normal there, or its face's where it has none.
    pub normal: Vec3,
}

/// The layout of one UV set of a mesh. See the module's documentation.
#[derive(Clone, Debug)]
pub struct UvLayout {
    pub set: usize,
    /// Each mesh vertex's welded index ([`weld`]).
    pub weld: Vec<u32>,
    /// Each triangle's island.
    pub island_of: Vec<u32>,
    pub islands: Vec<Island>,
    /// Each triangle corner's UV vertex, corner `k` of triangle `t` at
    /// `3 t + k`.
    pub corner_vertex: Vec<u32>,
    pub uv_vertices: Vec<UvVertex>,
    /// Every edge, island by island.
    pub edges: Vec<LayoutEdge>,
    /// Each triangle corner's edge: the one from corner `k` to the next.
    pub corner_edge: Vec<u32>,
    /// Each mesh vertex's island, where a triangle uses it.
    pub vertex_island: Vec<u32>,
    /// Metres of the part per unit of the layout where every island is as
    /// dense as the others, as an unwrap and a pack leave them; None where
    /// they differ, and a size on the layout is no size on the part.
    pub units_per_uv: Option<f32>,
    /// Two places on the layout closer than this are one.
    pub tolerance: f32,
    /// The mesh's triangles, as its index list.
    pub(crate) indices: Vec<u32>,
    pub(crate) table: EdgeTable,
    grid: Grid,
}

/// A vertex no triangle uses, in [`UvLayout::vertex_island`].
pub const NO_ISLAND: u32 = u32::MAX;

impl UvLayout {
    /// The layout of UV set `set`, or None where the mesh has no such set or
    /// one that does not cover its vertices.
    pub fn new(mesh: &Mesh, set: usize) -> Option<UvLayout> {
        let uv = mesh.uv_sets.get(set)?;
        if uv.len() != mesh.positions.len() || mesh.indices.iter().any(|&i| i as usize >= uv.len()) {
            return None;
        }
        let weld = weld(&mesh.positions);
        Some(Self::with_weld(mesh, set, weld))
    }

    /// As [`UvLayout::new`], with the welded indices already made.
    pub(crate) fn with_weld(mesh: &Mesh, set: usize, weld: Vec<u32>) -> UvLayout {
        let uv = &mesh.uv_sets[set];
        let count = mesh.triangle_count();
        let extent = uv.iter().filter(|p| p.is_finite()).fold(0.0f32, |m, p| m.max(p.x.abs()).max(p.y.abs()));
        let tolerance = extent.max(1.0) * (1.0 / (1u32 << 20) as f32);
        let table = EdgeTable::new(mesh, &weld);
        let same = |a: Vec2, b: Vec2| (a.x - b.x).abs() <= tolerance && (a.y - b.y).abs() <= tolerance;
        let corner_uv = |c: usize| uv[mesh.indices[c] as usize];

        // Faces, and corners at each end, joined across edges whose sides
        // agree: each side to the first before it that it agrees with.
        let mut faces = UnionFind::new(count);
        let mut corners = UnionFind::new(count * 3);
        for group in table.groups() {
            let sides = &table.sides[group];
            for i in 1..sides.len() {
                let s = &sides[i];
                for r in &sides[..i] {
                    if same(corner_uv(s.lo_corner()), corner_uv(r.lo_corner())) && same(corner_uv(s.hi_corner()), corner_uv(r.hi_corner())) {
                        faces.union(s.triangle as usize, r.triangle as usize);
                        corners.union(s.lo_corner(), r.lo_corner());
                        corners.union(s.hi_corner(), r.hi_corner());
                        break;
                    }
                }
            }
        }
        let mut island_id: Vec<u32> = vec![u32::MAX; count];
        let mut island_of = Vec::with_capacity(count);
        let mut islands_count = 0u32;
        for t in 0..count {
            let root = faces.find(t);
            if island_id[root] == u32::MAX {
                island_id[root] = islands_count;
                islands_count += 1;
            }
            island_of.push(island_id[root]);
        }
        let mut vertex_id: Vec<u32> = vec![u32::MAX; count * 3];
        let mut corner_vertex = Vec::with_capacity(count * 3);
        let mut uv_vertices: Vec<UvVertex> = Vec::new();
        for c in 0..count * 3 {
            let root = corners.find(c);
            if vertex_id[root] == u32::MAX {
                vertex_id[root] = uv_vertices.len() as u32;
                let vertex = mesh.indices[c];
                uv_vertices.push(UvVertex { uv: uv[vertex as usize], vertex, weld: weld[vertex as usize], island: island_of[c / 3] });
            }
            corner_vertex.push(vertex_id[root]);
        }
        let mut vertex_island = vec![NO_ISLAND; mesh.positions.len()];
        for (c, &v) in mesh.indices.iter().enumerate() {
            if vertex_island[v as usize] == NO_ISLAND {
                vertex_island[v as usize] = island_of[c / 3];
            }
        }

        let (edges, corner_edge) = layout_edges(&table, &corner_vertex, &island_of, &mesh.indices);
        let islands = islands(mesh, &island_of, islands_count as usize, &corner_vertex, &uv_vertices, &edges, &corner_edge);
        let units_per_uv = uniform_density(&islands);
        let corner_uvs: Vec<Vec2> = corner_vertex.iter().map(|&v| uv_vertices[v as usize].uv).collect();
        let grid = Grid::new(&corner_uvs);
        UvLayout { set, weld, island_of, islands, corner_vertex, uv_vertices, edges, corner_edge, vertex_island, units_per_uv, tolerance, indices: mesh.indices.clone(), table, grid }
    }

    /// A triangle's three corners on the layout.
    pub fn triangle_uv(&self, triangle: u32) -> [Vec2; 3] {
        let t = triangle as usize * 3;
        [0, 1, 2].map(|k| self.uv_vertices[self.corner_vertex[t + k] as usize].uv)
    }

    /// The triangle under a point of the layout, the first by number where
    /// islands overlap.
    pub fn triangle_at(&self, p: Vec2) -> Option<TriangleHit> {
        let cell = self.grid.cell_of(p)?;
        let mut best: Option<TriangleHit> = None;
        for &t in self.grid.items(cell) {
            if best.is_some_and(|b| b.triangle < t) {
                continue;
            }
            let [a, b, c] = self.triangle_uv(t);
            let area = facing(a, b, c);
            if area.abs() <= f32::MIN_POSITIVE {
                continue;
            }
            let w = [facing(p, b, c) / area, facing(a, p, c) / area, facing(a, b, p) / area];
            let slack = -1e-5;
            if w.iter().all(|&x| x >= slack) {
                best = Some(TriangleHit { triangle: t, island: self.island_of[t as usize], barycentric: w });
            }
        }
        best
    }

    /// The island under a point of the layout.
    pub fn island_at(&self, p: Vec2) -> Option<u32> {
        self.triangle_at(p).map(|hit| hit.island)
    }

    /// The UV vertex nearest a point of the layout, within `within` of it,
    /// with its distance.
    pub fn nearest_vertex(&self, p: Vec2, within: f32) -> Option<(u32, f32)> {
        let mut best: Option<(u32, f32)> = None;
        self.grid.search(p, within, |t, best_distance| {
            for k in 0..3 {
                let v = self.corner_vertex[t as usize * 3 + k];
                let d = (self.uv_vertices[v as usize].uv - p).length();
                if d <= within && best.is_none_or(|(bv, bd)| d < bd || (d == bd && v < bv)) {
                    best = Some((v, d));
                    *best_distance = d;
                }
            }
        });
        best
    }

    /// The edge nearest a point of the layout, within `within` of it, with
    /// its distance.
    pub fn nearest_edge(&self, p: Vec2, within: f32) -> Option<(u32, f32)> {
        let mut best: Option<(u32, f32)> = None;
        self.grid.search(p, within, |t, best_distance| {
            for k in 0..3 {
                let e = self.corner_edge[t as usize * 3 + k];
                let [a, b] = self.edges[e as usize].ends.map(|v| self.uv_vertices[v as usize].uv);
                let d = distance_to_segment(p, a, b);
                if d <= within && best.is_none_or(|(be, bd)| d < bd || (d == bd && e < be)) {
                    best = Some((e, d));
                    *best_distance = d;
                }
            }
        });
        best
    }

    /// Where a point of the layout lands on the part: the triangle under it,
    /// and the point and normal there.
    pub fn surface_at(&self, mesh: &Mesh, p: Vec2) -> Option<SurfaceHit> {
        let hit = self.triangle_at(p)?;
        let [a, b, c] = mesh.triangle(hit.triangle as usize).map(|v| v as usize);
        let w = hit.barycentric;
        let (pa, pb, pc) = (mesh.positions[a], mesh.positions[b], mesh.positions[c]);
        let position = pa * w[0] + pb * w[1] + pc * w[2];
        let face = (pb - pa).cross(pc - pa).try_normalize().unwrap_or(Vec3::Z);
        let normal = if mesh.normals.len() == mesh.positions.len() {
            (mesh.normals[a] * w[0] + mesh.normals[b] * w[1] + mesh.normals[c] * w[2]).try_normalize().unwrap_or(face)
        } else {
            face
        };
        Some(SurfaceHit { triangle: hit.triangle, island: hit.island, barycentric: w, position, normal })
    }

    /// The islands some of the mesh's triangles are on - faces picked on
    /// the part - in order, each once.
    pub fn islands_of_faces(&self, faces: &[u32]) -> Vec<u32> {
        let mut out: Vec<u32> = faces.iter().filter_map(|&t| self.island_of.get(t as usize).copied()).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The island a mesh vertex is on, where a triangle uses it.
    pub fn island_of_vertex(&self, vertex: u32) -> Option<u32> {
        self.vertex_island.get(vertex as usize).copied().filter(|&i| i != NO_ISLAND)
    }

    /// The seams between these islands, or inside one of them, each once,
    /// named by one side's mesh vertices: what stitching them back
    /// together joins.
    pub fn shared_seams(&self, islands: &[u32]) -> Vec<[u32; 2]> {
        let chosen = |i: u32| islands.contains(&i);
        let mut out = Vec::new();
        for (index, edge) in self.edges.iter().enumerate() {
            let Some(twin) = edge.twin else { continue };
            if edge.kind == EdgeKind::Seam && (index as u32) < twin && chosen(edge.island) && chosen(self.edges[twin as usize].island) {
                out.push(edge.vertices);
            }
        }
        out
    }

    /// The edges around some of the mesh's triangles - faces picked on the
    /// part - where a picked face meets one not picked or the edge of the
    /// part: what cutting the picked faces out of the layout cuts along.
    pub fn border_of_faces(&self, faces: &[u32]) -> Vec<[u32; 2]> {
        let mut picked = vec![false; self.island_of.len()];
        for &t in faces {
            if let Some(slot) = picked.get_mut(t as usize) {
                *slot = true;
            }
        }
        let mut out = Vec::new();
        for group in self.table.groups() {
            let sides = &self.table.sides[group];
            let inside = sides.iter().filter(|s| picked[s.triangle as usize]).count();
            if inside > 0 && inside < sides.len() {
                let s = sides.iter().find(|s| picked[s.triangle as usize]).expect("a picked side");
                out.push(s.vertices);
            }
        }
        out
    }

    /// Triangles laid out turned over against the rest of their island:
    /// folds, where artwork would print mirrored. A sliver turned over by a
    /// hair, a millionth of its island, is not one ([`super::flatten::FOLD_HAIR`]).
    pub fn folded_triangles(&self) -> Vec<u32> {
        let mut out = Vec::new();
        for island in &self.islands {
            let sign = if island.mirrored { -1.0 } else { 1.0 };
            let hair = island.uv_area * super::flatten::FOLD_HAIR;
            for &t in &island.triangles {
                let [a, b, c] = self.triangle_uv(t);
                if facing(a, b, c) * sign < -hair {
                    out.push(t);
                }
            }
        }
        out
    }

    /// The UV vertices some mesh vertices are on: what a selection of
    /// vertices picked on the layout moves together, so that no corner on
    /// the same place is left behind.
    pub fn uv_vertices_of(&self, vertices: &[u32]) -> Vec<u32> {
        let mut wanted = vec![false; self.vertex_island.len()];
        for &v in vertices {
            if let Some(slot) = wanted.get_mut(v as usize) {
                *slot = true;
            }
        }
        // Every corner on a wanted mesh vertex brings its UV vertex.
        let mut out: Vec<u32> = self.indices.iter().zip(&self.corner_vertex).filter(|(v, _)| wanted[**v as usize]).map(|(_, &g)| g).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// The welded edge a pair of mesh vertices names, if the part has one
    /// between them.
    pub(crate) fn edge_group(&self, edge: [u32; 2]) -> Option<std::ops::Range<usize>> {
        let [a, b] = edge;
        let (wa, wb) = (*self.weld.get(a as usize)?, *self.weld.get(b as usize)?);
        if wa == wb {
            return None;
        }
        self.table.find(wa.min(wb), wa.max(wb))
    }
}

fn distance_to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let length = ab.length_squared();
    let t = if length > 0.0 { ((p - a).dot(ab) / length).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t - p).length()
}

/// Where every island's density agrees with the whole layout's to a part
/// in twenty, the metres of the part per unit of the layout.
fn uniform_density(islands: &[Island]) -> Option<f32> {
    let surface: f64 = islands.iter().map(|i| i.surface_area as f64).sum();
    let flat: f64 = islands.iter().map(|i| i.uv_area as f64).sum();
    if surface <= 0.0 || flat <= 0.0 {
        return None;
    }
    let overall = flat / surface;
    // Islands too small to see a difference on do not decide it.
    let uniform = islands.iter().filter(|i| i.surface_area as f64 > surface * 1e-4).all(|i| {
        let d = i.density() as f64 / overall;
        (0.95..=1.05).contains(&d)
    });
    uniform.then(|| (surface / flat).sqrt() as f32)
}

/// One side of a welded edge: the triangle it is an edge of, and which.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Side {
    pub lo: u32,
    pub hi: u32,
    pub triangle: u32,
    /// The edge from corner `k` to the next.
    pub k: u8,
    /// Whether corner `k` is at `lo`.
    pub forward: bool,
    /// The mesh vertices at `lo` and `hi`.
    pub vertices: [u32; 2],
}

impl Side {
    pub fn lo_corner(&self) -> usize {
        self.triangle as usize * 3 + if self.forward { self.k as usize } else { (self.k as usize + 1) % 3 }
    }

    pub fn hi_corner(&self) -> usize {
        self.triangle as usize * 3 + if self.forward { (self.k as usize + 1) % 3 } else { self.k as usize }
    }

    /// Its corners in the triangle's own order: from corner `k`, to the next.
    pub fn from_to(&self) -> (usize, usize) {
        let t = self.triangle as usize * 3;
        (t + self.k as usize, t + (self.k as usize + 1) % 3)
    }
}

/// Every welded edge of a mesh with the triangles on it, sorted so the
/// sides of one edge are together.
#[derive(Clone, Debug)]
pub(crate) struct EdgeTable {
    pub sides: Vec<Side>,
    starts: Vec<u32>,
}

impl EdgeTable {
    pub fn new(mesh: &Mesh, weld: &[u32]) -> EdgeTable {
        let count = mesh.triangle_count();
        let mut sides = Vec::with_capacity(count * 3);
        for t in 0..count {
            for k in 0..3 {
                let (a, b) = (mesh.indices[t * 3 + k], mesh.indices[t * 3 + (k + 1) % 3]);
                let (wa, wb) = (weld[a as usize], weld[b as usize]);
                if wa == wb {
                    continue;
                }
                let forward = wa < wb;
                sides.push(Side {
                    lo: wa.min(wb),
                    hi: wa.max(wb),
                    triangle: t as u32,
                    k: k as u8,
                    forward,
                    vertices: if forward { [a, b] } else { [b, a] },
                });
            }
        }
        sides.sort_unstable_by_key(|s| ((s.lo as u64) << 32 | s.hi as u64, s.triangle, s.k));
        let mut starts = Vec::new();
        for (i, s) in sides.iter().enumerate() {
            if i == 0 || (sides[i - 1].lo, sides[i - 1].hi) != (s.lo, s.hi) {
                starts.push(i as u32);
            }
        }
        starts.push(sides.len() as u32);
        EdgeTable { sides, starts }
    }

    /// Each welded edge's run of sides.
    pub fn groups(&self) -> impl Iterator<Item = std::ops::Range<usize>> + '_ {
        self.starts.windows(2).map(|w| w[0] as usize..w[1] as usize)
    }

    /// The run of sides of the welded edge between `lo` and `hi`.
    pub fn find(&self, lo: u32, hi: u32) -> Option<std::ops::Range<usize>> {
        let groups = self.starts.len() - 1;
        let (mut low, mut high) = (0usize, groups);
        while low < high {
            let mid = (low + high) / 2;
            let s = &self.sides[self.starts[mid] as usize];
            match (s.lo, s.hi).cmp(&(lo, hi)) {
                std::cmp::Ordering::Less => low = mid + 1,
                std::cmp::Ordering::Greater => high = mid,
                std::cmp::Ordering::Equal => return Some(self.starts[mid] as usize..self.starts[mid + 1] as usize),
            }
        }
        None
    }
}

/// Every edge of the layout once as it is drawn, island by island, and each
/// corner's.
fn layout_edges(table: &EdgeTable, corner_vertex: &[u32], island_of: &[u32], indices: &[u32]) -> (Vec<LayoutEdge>, Vec<u32>) {
    let mut edges: Vec<LayoutEdge> = Vec::new();
    let mut corner_edge = vec![u32::MAX; corner_vertex.len()];
    for group in table.groups() {
        let sides = &table.sides[group.clone()];
        // Sides on the same two UV vertices are one edge as drawn.
        let first = edges.len();
        let mut class_of: Vec<usize> = Vec::with_capacity(sides.len());
        let mut counts: Vec<usize> = Vec::new();
        for s in sides {
            let ends = [corner_vertex[s.lo_corner()], corner_vertex[s.hi_corner()]];
            let found = (first..edges.len()).find(|&e| edges[e].ends == ends);
            let e = match found {
                Some(e) => e,
                None => {
                    edges.push(LayoutEdge {
                        ends,
                        vertices: [indices[s.lo_corner()], indices[s.hi_corner()]],
                        triangle: s.triangle,
                        island: island_of[s.triangle as usize],
                        kind: EdgeKind::Boundary,
                        twin: None,
                    });
                    counts.push(0);
                    edges.len() - 1
                }
            };
            counts[e - first] += 1;
            class_of.push(e);
        }
        let classes = edges.len() - first;
        for e in first..edges.len() {
            edges[e].kind = if counts[e - first] >= 2 {
                EdgeKind::Inner
            } else if classes > 1 {
                EdgeKind::Seam
            } else {
                EdgeKind::Boundary
            };
        }
        // A seam's twin: the next side round, which for the usual two is the
        // other one.
        let seams: Vec<usize> = (first..edges.len()).filter(|&e| edges[e].kind == EdgeKind::Seam).collect();
        if seams.len() >= 2 {
            for (i, &e) in seams.iter().enumerate() {
                edges[e].twin = Some(seams[(i + 1) % seams.len()] as u32);
            }
        }
        for (s, e) in sides.iter().zip(class_of) {
            corner_edge[s.from_to().0] = e as u32;
        }
    }
    // Island by island, keeping their order within each.
    let mut order: Vec<u32> = (0..edges.len() as u32).collect();
    order.sort_by_key(|&e| edges[e as usize].island);
    let mut new_index = vec![0u32; edges.len()];
    for (new, &old) in order.iter().enumerate() {
        new_index[old as usize] = new as u32;
    }
    let mut sorted: Vec<LayoutEdge> = order.iter().map(|&e| edges[e as usize].clone()).collect();
    for edge in &mut sorted {
        edge.twin = edge.twin.map(|t| new_index[t as usize]);
    }
    for e in &mut corner_edge {
        if *e != u32::MAX {
            *e = new_index[*e as usize];
        }
    }
    (sorted, corner_edge)
}

/// Each island's figures.
fn islands(mesh: &Mesh, island_of: &[u32], count: usize, corner_vertex: &[u32], uv_vertices: &[UvVertex], edges: &[LayoutEdge], corner_edge: &[u32]) -> Vec<Island> {
    let mut out: Vec<Island> = (0..count)
        .map(|_| Island {
            triangles: Vec::new(),
            uv_vertices: Vec::new(),
            vertices: Vec::new(),
            min: Vec2::splat(f32::INFINITY),
            max: Vec2::splat(f32::NEG_INFINITY),
            centre: Vec2::ZERO,
            centroid: Vec2::ZERO,
            uv_area: 0.0,
            surface_area: 0.0,
            axis: 0.0,
            elongation: 1.0,
            box_angle: 0.0,
            mirrored: false,
            boundary: Vec::new(),
            edges: 0..0,
        })
        .collect();
    for (t, &i) in island_of.iter().enumerate() {
        out[i as usize].triangles.push(t as u32);
    }
    for (v, vertex) in uv_vertices.iter().enumerate() {
        out[vertex.island as usize].uv_vertices.push(v as u32);
    }
    // Edges are already island by island.
    let mut start = 0;
    for (i, island) in out.iter_mut().enumerate() {
        let mut end = start;
        while end < edges.len() && edges[end].island as usize == i {
            end += 1;
        }
        island.edges = start..end;
        start = end;
    }
    for island in &mut out {
        let mut vertices: Vec<u32> = island.triangles.iter().flat_map(|&t| mesh.triangle(t as usize)).collect();
        vertices.sort_unstable();
        vertices.dedup();
        island.vertices = vertices;
        for &v in &island.uv_vertices {
            let p = uv_vertices[v as usize].uv;
            island.min = island.min.min(p);
            island.max = island.max.max(p);
        }
        island.centre = (island.min + island.max) * 0.5;
        // Areas, the middle of the area, and its second moments about the
        // origin, in f64: islands far from the origin lose them in f32.
        let (mut flat, mut signed, mut surface) = (0.0f64, 0.0f64, 0.0f64);
        let (mut cx, mut cy) = (0.0f64, 0.0f64);
        let (mut xx, mut xy, mut yy) = (0.0f64, 0.0f64, 0.0f64);
        let origin = island.centre;
        for &t in &island.triangles {
            let base = t as usize * 3;
            let [a, b, c] = [0, 1, 2].map(|k| uv_vertices[corner_vertex[base + k] as usize].uv - origin);
            let f = facing(a, b, c) as f64;
            let area = f.abs();
            flat += area;
            signed += f;
            let [pa, pb, pc] = mesh.triangle(t as usize).map(|v| mesh.positions[v as usize]);
            surface += surface_area(pa, pb, pc) as f64;
            let (sx, sy) = ((a.x + b.x + c.x) as f64, (a.y + b.y + c.y) as f64);
            cx += area * sx / 3.0;
            cy += area * sy / 3.0;
            let squares = |p: Vec2| (p.x as f64 * p.x as f64, p.x as f64 * p.y as f64, p.y as f64 * p.y as f64);
            let (mut sxx, mut sxy, mut syy) = (sx * sx, sx * sy, sy * sy);
            for p in [a, b, c] {
                let (pxx, pxy, pyy) = squares(p);
                sxx += pxx;
                sxy += pxy;
                syy += pyy;
            }
            xx += area / 12.0 * sxx;
            xy += area / 12.0 * sxy;
            yy += area / 12.0 * syy;
        }
        island.uv_area = flat as f32;
        island.surface_area = surface as f32;
        island.mirrored = signed < 0.0;
        if flat > 0.0 {
            let (mx, my) = (cx / flat, cy / flat);
            island.centroid = origin + vec2(mx as f32, my as f32);
            let (cxx, cxy, cyy) = (xx - flat * mx * mx, xy - flat * mx * my, yy - flat * my * my);
            island.axis = (0.5 * (2.0 * cxy).atan2(cxx - cyy)) as f32;
            let mean = (cxx + cyy) * 0.5;
            let spread = (((cxx - cyy) * 0.5).powi(2) + cxy * cxy).sqrt();
            let (major, minor) = (mean + spread, (mean - spread).max(0.0));
            island.elongation = if minor > 0.0 { (major / minor).sqrt() as f32 } else { f32::INFINITY };
        } else {
            island.centroid = island.centre;
        }
        let points: Vec<Vec2> = island.uv_vertices.iter().map(|&v| uv_vertices[v as usize].uv).collect();
        island.box_angle = tightest_box(&points).0;
        island.boundary = boundary_loops(island, corner_vertex, uv_vertices, edges, corner_edge);
    }
    out
}

/// An island's edge as loops of UV vertices, the largest first.
fn boundary_loops(island: &Island, corner_vertex: &[u32], uv_vertices: &[UvVertex], edges: &[LayoutEdge], corner_edge: &[u32]) -> Vec<Vec<u32>> {
    // Each edge on the island's edge, the way its triangle runs round.
    let mut outgoing: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut ends: Vec<(u32, u32)> = Vec::new();
    for &t in &island.triangles {
        for k in 0..3 {
            let c = t as usize * 3 + k;
            let e = corner_edge[c];
            if e == u32::MAX || edges[e as usize].kind == EdgeKind::Inner {
                continue;
            }
            let from = corner_vertex[c];
            let to = corner_vertex[t as usize * 3 + (k + 1) % 3];
            outgoing.entry(from).or_default().push(ends.len() as u32);
            ends.push((from, to));
        }
    }
    let mut used = vec![false; ends.len()];
    let mut loops: Vec<(f64, Vec<u32>)> = Vec::new();
    for first in 0..ends.len() {
        if used[first] {
            continue;
        }
        let mut path = Vec::new();
        let mut at = first;
        loop {
            used[at] = true;
            let (from, to) = ends[at];
            path.push(from);
            let next = outgoing.get(&to).and_then(|list| list.iter().copied().find(|&h| !used[h as usize]));
            match next {
                Some(h) => at = h as usize,
                None => break,
            }
            if ends[at].0 == ends[first].0 && used[at] {
                break;
            }
        }
        let mut area = 0.0f64;
        for i in 0..path.len() {
            let (p, q) = (uv_vertices[path[i] as usize].uv, uv_vertices[path[(i + 1) % path.len()] as usize].uv);
            area += (p.x as f64) * (q.y as f64) - (q.x as f64) * (p.y as f64);
        }
        loops.push((area.abs(), path));
    }
    loops.sort_by(|a, b| b.0.total_cmp(&a.0));
    loops.into_iter().map(|(_, path)| path).collect()
}

/// The tightest bounding rectangle of some points: the direction of its
/// long side, in radians (-pi/2 to pi/2], and its width and height along
/// and across that direction.
pub fn tightest_box(points: &[Vec2]) -> (f32, Vec2) {
    let hull = convex_hull(points);
    if hull.len() < 3 {
        let low = points.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
        let high = points.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
        if hull.len() == 2 {
            let d = hull[1] - hull[0];
            return (fold_angle(d.y.atan2(d.x)), vec2(d.length(), 0.0));
        }
        return (0.0, (high - low).max(Vec2::ZERO));
    }
    let mut best = (f32::INFINITY, 0.0f32, Vec2::ZERO);
    for i in 0..hull.len() {
        let d = hull[(i + 1) % hull.len()] - hull[i];
        if d.length() <= 0.0 {
            continue;
        }
        let angle = d.y.atan2(d.x);
        let (s, c) = angle.sin_cos();
        let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for p in &hull {
            let q = vec2(p.x * c + p.y * s, -p.x * s + p.y * c);
            low = low.min(q);
            high = high.max(q);
        }
        let size = high - low;
        let area = size.x * size.y;
        if area < best.0 * (1.0 - 1e-6) {
            best = (area, angle, size);
        }
    }
    let (_, mut angle, mut size) = best;
    if size.y > size.x {
        angle += std::f32::consts::FRAC_PI_2;
        size = vec2(size.y, size.x);
    }
    (fold_angle(angle), size)
}

/// An angle of a line, which has no direction, in (-pi/2, pi/2].
pub(crate) fn fold_angle(angle: f32) -> f32 {
    let pi = std::f32::consts::PI;
    let mut a = angle.rem_euclid(pi);
    if a > pi * 0.5 {
        a -= pi;
    }
    a
}

/// The convex hull, counter-clockwise in the usual sense: Andrew's
/// monotone chain.
pub(crate) fn convex_hull(points: &[Vec2]) -> Vec<Vec2> {
    let mut sorted: Vec<Vec2> = points.iter().copied().filter(|p| p.is_finite()).collect();
    sorted.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    sorted.dedup();
    if sorted.len() < 3 {
        return sorted;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut hull: Vec<Vec2> = Vec::with_capacity(sorted.len() * 2);
    for &p in &sorted {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len() + 1;
    for &p in sorted.iter().rev().skip(1) {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0.0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    hull
}

/// Disjoint sets, by path halving and union by size.
pub(crate) struct UnionFind {
    parent: Vec<u32>,
    size: Vec<u32>,
}

impl UnionFind {
    pub fn new(n: usize) -> UnionFind {
        UnionFind { parent: (0..n as u32).collect(), size: vec![1; n] }
    }

    pub fn find(&mut self, mut x: usize) -> usize {
        while self.parent[x] as usize != x {
            let up = self.parent[self.parent[x] as usize];
            self.parent[x] = up;
            x = up as usize;
        }
        x
    }

    pub fn union(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.find(a), self.find(b));
        if a == b {
            return;
        }
        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a as u32;
        self.size[a] += self.size[b];
    }
}

/// Triangles by the cells of a grid over the layout they reach into: what
/// finds the triangle under a point, and the vertex and edge nearest it,
/// without going through them all.
#[derive(Clone, Debug)]
struct Grid {
    min: Vec2,
    cell: Vec2,
    columns: usize,
    rows: usize,
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl Grid {
    fn new(corners: &[Vec2]) -> Grid {
        let triangles = corners.len() / 3;
        let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for p in corners.iter().filter(|p| p.is_finite()) {
            low = low.min(*p);
            high = high.max(*p);
        }
        if !(low.x <= high.x && low.y <= high.y) {
            return Grid { min: Vec2::ZERO, cell: Vec2::ONE, columns: 1, rows: 1, starts: vec![0, 0], items: Vec::new() };
        }
        let size = (high - low).max(Vec2::splat(1e-6));
        // About two triangles to a cell, cells about square.
        let cells = (triangles as f32 / 2.0).clamp(1.0, 1_000_000.0);
        let aspect = size.x / size.y;
        let columns = (cells * aspect).sqrt().ceil().clamp(1.0, 2048.0) as usize;
        let rows = (cells / aspect).sqrt().ceil().clamp(1.0, 2048.0) as usize;
        let cell = vec2(size.x / columns as f32, size.y / rows as f32);
        let mut grid = Grid { min: low, cell, columns, rows, starts: vec![0; columns * rows + 1], items: Vec::new() };
        let spans: Vec<Option<(usize, usize, usize, usize)>> = (0..triangles)
            .map(|t| {
                let [a, b, c] = [corners[t * 3], corners[t * 3 + 1], corners[t * 3 + 2]];
                if !(a.is_finite() && b.is_finite() && c.is_finite()) {
                    return None;
                }
                let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
                let (x0, y0) = grid.clamped(lo);
                let (x1, y1) = grid.clamped(hi);
                Some((x0, y0, x1, y1))
            })
            .collect();
        let span = |t: usize| spans[t];
        for t in 0..triangles {
            if let Some((x0, y0, x1, y1)) = span(t) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        grid.starts[y * columns + x + 1] += 1;
                    }
                }
            }
        }
        for i in 1..grid.starts.len() {
            grid.starts[i] += grid.starts[i - 1];
        }
        let mut fill = grid.starts.clone();
        grid.items = vec![0; *grid.starts.last().expect("a total") as usize];
        for t in 0..triangles {
            if let Some((x0, y0, x1, y1)) = span(t) {
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let cell = y * columns + x;
                        grid.items[fill[cell] as usize] = t as u32;
                        fill[cell] += 1;
                    }
                }
            }
        }
        grid
    }

    fn clamped(&self, p: Vec2) -> (usize, usize) {
        let x = ((p.x - self.min.x) / self.cell.x).floor();
        let y = ((p.y - self.min.y) / self.cell.y).floor();
        (x.clamp(0.0, (self.columns - 1) as f32) as usize, y.clamp(0.0, (self.rows - 1) as f32) as usize)
    }

    /// The cell a point is in, if it is on the grid.
    fn cell_of(&self, p: Vec2) -> Option<usize> {
        if !p.is_finite() {
            return None;
        }
        let x = (p.x - self.min.x) / self.cell.x;
        let y = (p.y - self.min.y) / self.cell.y;
        let eps = 1e-4;
        if x < -eps || y < -eps || x > self.columns as f32 + eps || y > self.rows as f32 + eps {
            return None;
        }
        let (x, y) = self.clamped(p);
        Some(y * self.columns + x)
    }

    fn items(&self, cell: usize) -> &[u32] {
        &self.items[self.starts[cell] as usize..self.starts[cell + 1] as usize]
    }

    /// Visit the triangles in rings of cells round a point, nearest first,
    /// until the next ring is farther than `within` or than the best found
    /// so far, which `visit` lowers.
    fn search(&self, p: Vec2, within: f32, mut visit: impl FnMut(u32, &mut f32)) {
        if !p.is_finite() {
            return;
        }
        let (cx, cy) = self.clamped(p);
        let step = self.cell.x.min(self.cell.y);
        let mut best = f32::INFINITY;
        let most = self.columns.max(self.rows);
        for ring in 0..=most {
            // Every cell of this ring is at least this far from the point,
            // on the grid or off it: the rings inside are in the way.
            let near = (ring as f32 - 1.0).max(0.0) * step;
            if near > within || near > best {
                break;
            }
            let (x0, y0) = (cx as i64 - ring as i64, cy as i64 - ring as i64);
            let (x1, y1) = (cx as i64 + ring as i64, cy as i64 + ring as i64);
            for y in y0..=y1 {
                if y < 0 || y >= self.rows as i64 {
                    continue;
                }
                for x in x0..=x1 {
                    if x < 0 || x >= self.columns as i64 {
                        continue;
                    }
                    let on_ring = y == y0 || y == y1 || x == x0 || x == x1;
                    if !on_ring {
                        continue;
                    }
                    for &t in self.items(y as usize * self.columns + x as usize) {
                        visit(t, &mut best);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use tequila_core::math::vec3;

    #[test]
    fn a_box_unwrapped_is_six_islands_with_seams_between_them() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).expect("a layout");
        assert_eq!(layout.islands.len(), 6);
        // Twelve edges of the box, each a seam with a side on two islands;
        // each face's diagonal inside it.
        let seams = layout.edges.iter().filter(|e| e.kind == EdgeKind::Seam).count();
        let inner = layout.edges.iter().filter(|e| e.kind == EdgeKind::Inner).count();
        assert_eq!(seams, 24);
        assert_eq!(inner, 6);
        assert!(layout.edges.iter().all(|e| e.kind != EdgeKind::Boundary), "a closed box has no edge");
        for edge in layout.edges.iter().filter(|e| e.kind == EdgeKind::Seam) {
            let twin = &layout.edges[edge.twin.expect("a twin") as usize];
            assert_eq!(twin.twin.map(|t| &layout.edges[t as usize]), Some(edge), "twins point at each other");
            assert_ne!(twin.island, edge.island);
        }
        for island in &layout.islands {
            assert_eq!(island.triangles.len(), 2);
            assert_eq!(island.boundary.len(), 1, "one loop round each side");
            assert_eq!(island.boundary[0].len(), 4);
            assert!(!island.mirrored, "an unwrap reads the right way");
            assert!((island.surface_area - 1.0).abs() < 1e-5);
        }
        assert!(layout.units_per_uv.is_some(), "an unwrap is as dense all over");
        assert!(layout.folded_triangles().is_empty());
    }

    #[test]
    fn faces_split_for_their_normals_but_laid_out_as_one_are_one_island() {
        // A flat strip of two quads whose middle vertices are doubled, as a
        // mesh with a hard edge has them, with the same UVs on both.
        let positions = vec![
            vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0),
            vec3(1.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0), vec3(2.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0),
        ];
        let uv = vec![vec2(0.0, 1.0), vec2(0.5, 1.0), vec2(0.5, 0.5), vec2(0.0, 0.5), vec2(0.5, 1.0), vec2(1.0, 1.0), vec2(1.0, 0.5), vec2(0.5, 0.5)];
        let mesh = Mesh::new(positions, vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]).with_uvs(uv);
        let layout = UvLayout::new(&mesh, 0).unwrap();
        assert_eq!(layout.islands.len(), 1);
        assert_eq!(layout.edges.iter().filter(|e| e.kind == EdgeKind::Seam).count(), 0);
        assert_eq!(layout.uv_vertices.len(), 6, "the doubled vertices are one place each");
        assert_eq!(layout.islands[0].boundary.len(), 1);
        assert_eq!(layout.islands[0].boundary[0].len(), 6);
    }

    #[test]
    fn hit_tests_find_the_triangle_the_vertex_and_the_edge() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        for (i, island) in layout.islands.iter().enumerate() {
            let hit = layout.triangle_at(island.centroid).expect("a triangle under the middle");
            assert_eq!(hit.island, i as u32);
            let sum: f32 = hit.barycentric.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4);
            // The point on the part is on the box's surface, on that side.
            let on = layout.surface_at(&mesh, island.centroid).unwrap();
            let p = on.position;
            let on_a_face = [p.x, p.y, p.z].iter().any(|c| c.abs() < 1e-4 || (c - 1.0).abs() < 1e-4);
            assert!(on_a_face, "{p:?}");
            assert!((on.normal.length() - 1.0).abs() < 1e-4);
        }
        assert!(layout.triangle_at(vec2(5.0, 5.0)).is_none());
        // The nearest vertex to a corner is that corner, and the nearest
        // edge to a point just off a boundary is on it.
        let corner = layout.uv_vertices[layout.islands[2].boundary[0][0] as usize].uv;
        let (v, d) = layout.nearest_vertex(corner + vec2(1e-4, 1e-4), 0.1).expect("a vertex near");
        assert!(d < 2e-4);
        assert_eq!(layout.uv_vertices[v as usize].uv, corner);
        let ends = layout.islands[2].boundary[0][..2].iter().map(|&v| layout.uv_vertices[v as usize].uv).collect::<Vec<_>>();
        let middle = (ends[0] + ends[1]) * 0.5;
        let (e, d) = layout.nearest_edge(middle, 0.1).expect("an edge near");
        assert!(d < 1e-5);
        assert_ne!(layout.edges[e as usize].kind, EdgeKind::Inner);
        assert!(layout.nearest_vertex(vec2(40.0, 40.0), 0.5).is_none());
    }

    #[test]
    fn faces_find_their_islands_and_a_pick_its_border() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        // The two triangles of the first side are one island.
        assert_eq!(layout.islands_of_faces(&[0, 1]), vec![layout.island_of[0]]);
        assert_eq!(layout.islands_of_faces(&[0, 2, 4]).len(), 3);
        // A side's border is its four edges.
        assert_eq!(layout.border_of_faces(&[0, 1]).len(), 4);
        // Two sides that meet share one seam.
        let a = layout.island_of[0];
        let neighbour = layout.edges[layout.islands[a as usize].edges.start].twin.map(|t| layout.edges[t as usize].island).unwrap();
        assert_eq!(layout.shared_seams(&[a, neighbour]).len(), 1);
    }

    #[test]
    fn a_long_island_has_its_axis_along_it_and_a_square_its_sides() {
        let strip = sheet(2, 20, |_, _| 0.0);
        let mesh = strip.clone().with_uvs(strip.positions.iter().map(|p| vec2(p.x * 2.0, p.z * 0.2)).collect());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let island = &layout.islands[0];
        assert!(island.axis.abs() < 1e-3, "{}", island.axis);
        assert!(island.elongation > 5.0);
        // A square turned 30 degrees has no principal axis, but its sides.
        let turn = 30f32.to_radians();
        let square = sheet(4, 4, |_, _| 0.0);
        let uv = square.positions.iter().map(|p| vec2(p.x * turn.cos() - p.z * turn.sin(), p.x * turn.sin() + p.z * turn.cos())).collect();
        let layout = UvLayout::new(&square.clone().with_uvs(uv), 0).unwrap();
        let island = &layout.islands[0];
        let off = |a: f32| fold_angle(a - turn).abs().min(fold_angle(a - turn - std::f32::consts::FRAC_PI_2).abs());
        assert!(off(island.straightening_angle()) < 1e-3, "{}", island.straightening_angle());
    }
}
