//! What the tools that reshape a layout share: what they are given to work
//! on, why they refuse, a piece of the layout as a patch of surface, the
//! sums they solve, and the ways the rest of an island follows the part of
//! it a tool placed.
//!
//! A shape tool reads the layout as it is and works out new places for
//! some of its UV vertices ([`Moves`]); [`super::set_positions`] applies
//! them, as the `uv.set_positions` command does. So a tool never changes
//! which faces are joined - only where things are - and the command says
//! exactly what it did, for the journal and a script to say again.
//!
//! Three ways a part placed by a tool takes the rest of its island along:
//!
//! - **laid out again round it** ([`reflatten_holding`]): least-squares
//!   conformal maps with the placed vertices pinned, the rest as true to
//!   the surface as it can be - what follows a path laid straight;
//! - **spread evenly inside it** ([`even_fill`]): each free vertex the
//!   average of its neighbours, which inside a convex border never folds
//!   (Tutte 1963) - smoothing that does not look at the surface;
//! - **carried along** ([`follow`]): the placed vertices' moves spread
//!   smoothly over the rest, each free vertex moved as its neighbours are
//!   (a harmonic displacement), so a vertex nobody moved stays where it was
//!   and the shape the layout had is kept where it can be.

use super::edit::{layout_of, UvError};
use super::flatten::{conformal, facing, folds, surface_area};
use super::layout::UvLayout;
use super::MeshEdge;
use crate::geometry::Mesh;
use std::collections::HashMap;
use tequila_core::math::{vec2, Vec2, Vec3};

/// Why a shape tool was refused. Nothing is changed when one is.
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeError {
    /// The layout refused: no such set, island or vertex; a fold.
    Uv(UvError),
    /// What was picked is not what the tool works on, in a few words: "the
    /// picked edges are not one path".
    Cannot(String),
}

impl std::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShapeError::Uv(error) => write!(f, "{error}"),
            ShapeError::Cannot(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for ShapeError {}

impl From<UvError> for ShapeError {
    fn from(error: UvError) -> Self {
        ShapeError::Uv(error)
    }
}

pub(crate) fn cannot<T>(why: impl Into<String>) -> Result<T, ShapeError> {
    Err(ShapeError::Cannot(why.into()))
}

/// UV vertices, by the layout's numbers, each with its new place: what a
/// tool worked out, for [`super::set_positions`].
pub type Moves = Vec<(u32, Vec2)>;

/// What a tool works on, as the editor picks it. Each tool reads what it
/// takes and says so when it is not there.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pick {
    /// Islands, by number.
    pub islands: Vec<u32>,
    /// Mesh vertices: points of the layout, with every corner on the same
    /// place.
    pub vertices: Vec<u32>,
    /// Edges, each by the mesh vertices at its ends on the side picked.
    pub edges: Vec<MeshEdge>,
    /// Triangles of the part: faces picked on it in the viewport.
    pub faces: Vec<u32>,
}

impl Pick {
    pub fn islands(islands: Vec<u32>) -> Pick {
        Pick { islands, ..Default::default() }
    }

    pub fn vertices(vertices: Vec<u32>) -> Pick {
        Pick { vertices, ..Default::default() }
    }

    pub fn edges(edges: Vec<MeshEdge>) -> Pick {
        Pick { edges, ..Default::default() }
    }

    pub fn faces(faces: Vec<u32>) -> Pick {
        Pick { faces, ..Default::default() }
    }

    pub fn is_empty(&self) -> bool {
        self.islands.is_empty() && self.vertices.is_empty() && self.edges.is_empty() && self.faces.is_empty()
    }
}

/// A set's layout and the islands a tool works on: those picked, those the
/// picked faces, edges or vertices are on, or with nothing picked every
/// island.
pub(crate) fn islands_of_pick(layout: &UvLayout, pick: &Pick) -> Result<Vec<u32>, UvError> {
    let mut out = super::edit::chosen_islands(layout, &pick.islands)?;
    out.extend(layout.islands_of_faces(&pick.faces));
    for &v in &pick.vertices {
        out.push(layout.island_of_vertex(v).ok_or(UvError::NoSuchVertex(v))?);
    }
    for &edge in &pick.edges {
        let group = layout.edge_group(edge).ok_or(UvError::NoSuchEdge(edge))?;
        let side = layout.table.sides[group.clone()].iter().find(|s| s.vertices == edge || s.vertices == [edge[1], edge[0]]).unwrap_or(&layout.table.sides[group.start]);
        out.push(layout.island_of[side.triangle as usize]);
    }
    out.sort_unstable();
    out.dedup();
    if out.is_empty() && pick.is_empty() {
        out = (0..layout.islands.len() as u32).collect();
    }
    Ok(out)
}

/// The layout, checked, as every tool begins.
pub(crate) fn layout(mesh: &Mesh, set: usize) -> Result<UvLayout, ShapeError> {
    Ok(layout_of(mesh, set)?)
}

/// The UV vertex a picked edge's end is on, on the side picked: the
/// layout edge the two mesh vertices name, and its ends.
pub(crate) fn edge_ends(layout: &UvLayout, edge: MeshEdge) -> Result<[u32; 2], UvError> {
    let group = layout.edge_group(edge).ok_or(UvError::NoSuchEdge(edge))?;
    let sides = &layout.table.sides[group];
    let side = sides.iter().find(|s| s.vertices == edge || s.vertices == [edge[1], edge[0]]).unwrap_or(&sides[0]);
    let (lo, hi) = (layout.corner_vertex[side.lo_corner()], layout.corner_vertex[side.hi_corner()]);
    // In the order the edge was named.
    Ok(if layout.weld[edge[0] as usize] == layout.uv_vertices[lo as usize].weld { [lo, hi] } else { [hi, lo] })
}

/// Where a UV vertex is on the part.
pub(crate) fn point(mesh: &Mesh, layout: &UvLayout, v: u32) -> Vec3 {
    mesh.positions[layout.uv_vertices[v as usize].vertex as usize]
}

/// Layout units per metre of the part across some islands: the root of
/// their area on the layout over their area on the part. None where they
/// have no area.
pub(crate) fn scale_of(layout: &UvLayout, islands: &[u32]) -> Option<f32> {
    let flat: f64 = islands.iter().map(|&i| layout.islands[i as usize].uv_area as f64).sum();
    let surface: f64 = islands.iter().map(|&i| layout.islands[i as usize].surface_area as f64).sum();
    (flat > 0.0 && surface > 0.0).then(|| (flat / surface).sqrt() as f32)
}

/// A piece of the layout - some of its triangles, on their UV vertices - as
/// a patch of surface to solve: each UV vertex a point of the part and a
/// place on the layout, numbered from 0 in the order first used.
pub(crate) struct Patch {
    /// The layout's UV vertex each of the patch's is.
    pub vertices: Vec<u32>,
    pub local: HashMap<u32, usize>,
    pub points: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    pub faces: Vec<[usize; 3]>,
}

impl Patch {
    pub fn new(mesh: &Mesh, layout: &UvLayout, triangles: &[u32]) -> Patch {
        let mut patch = Patch { vertices: Vec::new(), local: HashMap::new(), points: Vec::new(), uv: Vec::new(), faces: Vec::with_capacity(triangles.len()) };
        for &t in triangles {
            let face = [0, 1, 2].map(|k| {
                let g = layout.corner_vertex[t as usize * 3 + k];
                *patch.local.entry(g).or_insert_with(|| {
                    patch.vertices.push(g);
                    patch.points.push(mesh.positions[mesh.indices[t as usize * 3 + k] as usize]);
                    patch.uv.push(layout.uv_vertices[g as usize].uv);
                    patch.vertices.len() - 1
                })
            });
            patch.faces.push(face);
        }
        patch
    }

    pub fn len(&self) -> usize {
        self.vertices.len()
    }

    /// The patch's area on the layout, signed: negative where it reads
    /// mirrored.
    pub fn signed_area(&self, uv: &[Vec2]) -> f64 {
        self.faces.iter().map(|f| facing(uv[f[0]], uv[f[1]], uv[f[2]]) as f64).sum()
    }

    pub fn surface(&self) -> f64 {
        self.faces.iter().map(|f| surface_area(self.points[f[0]], self.points[f[1]], self.points[f[2]]) as f64).sum()
    }

    /// Each edge once, with its cotangent weight from the part: half the
    /// cotangents of the angles facing it, kept a little above nothing so
    /// a long thin triangle's obtuse angle does not pull the wrong way.
    pub fn weights(&self) -> Vec<(usize, usize, f64)> {
        let mut sums: HashMap<(usize, usize), f64> = HashMap::new();
        for f in &self.faces {
            for k in 0..3 {
                let (i, j, o) = (f[k], f[(k + 1) % 3], f[(k + 2) % 3]);
                let (a, b) = (self.points[i] - self.points[o], self.points[j] - self.points[o]);
                let cross = a.cross(b).length() as f64;
                let cot = if cross > 1e-20 { a.dot(b) as f64 / cross } else { 0.0 };
                *sums.entry((i.min(j), i.max(j))).or_insert(0.0) += 0.5 * cot;
            }
        }
        let mut out: Vec<(usize, usize, f64)> = sums.into_iter().map(|((i, j), w)| (i, j, w)).collect();
        out.sort_by_key(|&(i, j, _)| (i, j));
        let mean = out.iter().map(|e| e.2.abs()).sum::<f64>() / out.len().max(1) as f64;
        let least = (mean * 1e-3).max(1e-12);
        for e in &mut out {
            e.2 = e.2.max(least);
        }
        out
    }

    /// Each edge once, all weighing the same.
    pub fn even_weights(&self) -> Vec<(usize, usize, f64)> {
        let mut out: Vec<(usize, usize, f64)> = Vec::new();
        for f in &self.faces {
            for k in 0..3 {
                let (i, j) = (f[k], f[(k + 1) % 3]);
                out.push((i.min(j), i.max(j), 1.0));
            }
        }
        out.sort_by_key(|&(i, j, _)| (i, j));
        out.dedup_by_key(|e| (e.0, e.1));
        out
    }

    /// Whether `uv` turns any triangle over against the way `reads_right`
    /// says the patch reads, or flattens it.
    pub fn folds(&self, uv: &[Vec2], reads_right: bool) -> bool {
        folds(uv, &self.faces, Some(reads_right))
    }

    /// The new places of the patch's vertices, as moves.
    pub fn moves(&self, uv: &[Vec2]) -> Moves {
        self.vertices.iter().zip(uv).map(|(&v, &p)| (v, p)).collect()
    }
}

/// The patch laid out again by least-squares conformal maps with `held`
/// (the patch's own numbers) pinned where they say; the rest as true to the
/// surface as it can be. A patch that reads mirrored is solved mirrored
/// back and mirrored again, so it keeps reading as it did. None where the
/// answer folds or does not settle.
pub(crate) fn reflatten_holding(patch: &Patch, held: &[(usize, Vec2)]) -> Option<Vec<Vec2>> {
    let mirrored = patch.signed_area(&patch.uv) < 0.0;
    let flip = |p: Vec2| if mirrored { vec2(-p.x, p.y) } else { p };
    let start: Vec<Vec2> = patch.uv.iter().map(|p| flip(*p)).collect();
    let pins: Vec<(usize, Vec2)> = held.iter().map(|&(i, p)| (i, flip(p))).collect();
    let flat = conformal(&patch.points, &patch.faces, &start, &pins)?;
    let out: Vec<Vec2> = flat.into_iter().map(flip).collect();
    (!patch.folds(&out, !mirrored)).then_some(out)
}

/// The patch with `held` where they say and every other vertex the
/// average of its neighbours by `weights`: even weights spread the inside
/// evenly, cotangent weights as the surface lies, which inside a fixed
/// border is the conformal layout. None where it folds.
pub(crate) fn even_fill(patch: &Patch, held: &[(usize, Vec2)], weights: &[(usize, usize, f64)]) -> Option<Vec<Vec2>> {
    let mirrored = patch.signed_area(&patch.uv) < 0.0;
    let mut fixed: Vec<Option<Vec2>> = vec![None; patch.len()];
    for &(i, p) in held {
        fixed[i] = Some(p);
    }
    let out = harmonic(&patch.uv, &fixed, weights, 0.0)?;
    let reads_right = if held.len() >= 3 { patch.signed_area(&out) >= 0.0 } else { !mirrored };
    (!patch.folds(&out, reads_right)).then_some(out)
}

/// The patch with `held` moved where they say and every other vertex
/// carried along by the moves near it: the moves spread over the patch
/// smoothly by `weights`, a vertex with no held vertex joined to it left
/// where it is. None where that folds.
pub(crate) fn follow(patch: &Patch, held: &[(usize, Vec2)], weights: &[(usize, usize, f64)]) -> Option<Vec<Vec2>> {
    let mirrored = patch.signed_area(&patch.uv) < 0.0;
    let mut fixed: Vec<Option<Vec2>> = vec![None; patch.len()];
    for &(i, p) in held {
        fixed[i] = Some(p - patch.uv[i]);
    }
    let zero = vec![Vec2::ZERO; patch.len()];
    // A hair of pull toward staying put, so a piece no move reaches stays.
    let mean = weights.iter().map(|e| e.2).sum::<f64>() / weights.len().max(1) as f64;
    let moved = harmonic(&zero, &fixed, weights, mean * 1e-9)?;
    let out: Vec<Vec2> = patch.uv.iter().zip(&moved).map(|(p, d)| *p + *d).collect();
    (!patch.folds(&out, !mirrored)).then_some(out)
}

/// The values on a graph's vertices, `fixed` where they are given and each
/// other one the weighted average of its neighbours, solved for both
/// coordinates at once: the discrete Laplace equation, by conjugate
/// gradients from `start`. `pull` holds each free vertex toward `start` by
/// that much, which keeps a piece with nothing fixed where it was. None
/// where it does not settle.
pub(crate) fn harmonic(start: &[Vec2], fixed: &[Option<Vec2>], weights: &[(usize, usize, f64)], pull: f64) -> Option<Vec<Vec2>> {
    let n = start.len();
    let mut column = vec![usize::MAX; n];
    let mut free = 0usize;
    for i in 0..n {
        if fixed[i].is_none() {
            column[i] = free;
            free += 1;
        }
    }
    let mut out: Vec<Vec2> = (0..n).map(|i| fixed[i].unwrap_or(start[i])).collect();
    if free == 0 {
        return Some(out);
    }
    let mut rows: Vec<Vec<(usize, f64)>> = vec![Vec::new(); free];
    let mut b = vec![[0.0f64; 2]; free];
    for (i, row) in rows.iter_mut().enumerate() {
        row.push((i, pull));
    }
    for i in 0..n {
        if column[i] != usize::MAX {
            b[column[i]][0] += pull * start[i].x as f64;
            b[column[i]][1] += pull * start[i].y as f64;
        }
    }
    for &(i, j, w) in weights {
        for (p, q) in [(i, j), (j, i)] {
            let cp = column[p];
            if cp == usize::MAX {
                continue;
            }
            rows[cp].push((cp, w));
            match fixed[q] {
                Some(value) => {
                    b[cp][0] += w * value.x as f64;
                    b[cp][1] += w * value.y as f64;
                }
                None => rows[cp].push((column[q], -w)),
            }
        }
    }
    let matrix = Sparse::from_rows(rows);
    for axis in 0..2 {
        let rhs: Vec<f64> = b.iter().map(|v| v[axis]).collect();
        let mut x: Vec<f64> = (0..n).filter(|&i| column[i] != usize::MAX).map(|i| if axis == 0 { start[i].x as f64 } else { start[i].y as f64 }).collect();
        if !conjugate_gradients(&matrix, &rhs, &mut x, 1e-10, (free * 4).clamp(200, 40_000)) {
            return None;
        }
        for i in 0..n {
            if column[i] != usize::MAX {
                let value = x[column[i]] as f32;
                if axis == 0 { out[i].x = value } else { out[i].y = value }
            }
        }
    }
    out.iter().all(|p| p.is_finite()).then_some(out)
}

/// A sparse symmetric matrix as compressed rows.
pub(crate) struct Sparse {
    starts: Vec<usize>,
    columns: Vec<u32>,
    values: Vec<f64>,
}

impl Sparse {
    /// From each row's entries, a column given more than once summed.
    pub fn from_rows(rows: Vec<Vec<(usize, f64)>>) -> Sparse {
        let mut starts = Vec::with_capacity(rows.len() + 1);
        let mut columns = Vec::new();
        let mut values = Vec::new();
        starts.push(0);
        for mut row in rows {
            row.sort_by_key(|e| e.0);
            for (c, v) in row {
                if columns.len() > *starts.last().unwrap() && *columns.last().unwrap() == c as u32 {
                    *values.last_mut().unwrap() += v;
                } else {
                    columns.push(c as u32);
                    values.push(v);
                }
            }
            starts.push(columns.len());
        }
        Sparse { starts, columns, values }
    }

    pub fn size(&self) -> usize {
        self.starts.len() - 1
    }

    pub fn apply(&self, x: &[f64], out: &mut [f64]) {
        for (i, o) in out.iter_mut().enumerate() {
            let mut sum = 0.0;
            for k in self.starts[i]..self.starts[i + 1] {
                sum += self.values[k] * x[self.columns[k] as usize];
            }
            *o = sum;
        }
    }

    fn diagonal(&self, i: usize) -> f64 {
        (self.starts[i]..self.starts[i + 1]).find(|&k| self.columns[k] as usize == i).map(|k| self.values[k]).unwrap_or(0.0)
    }
}

/// Solve `a x = b` for a symmetric positive (semi-)definite `a` by
/// conjugate gradients with Jacobi preconditioning, from the `x` given,
/// to a residual of `tolerance` of `b`'s size. Whether it settled - or came
/// near enough that no layout would show the difference.
pub(crate) fn conjugate_gradients(a: &Sparse, b: &[f64], x: &mut [f64], tolerance: f64, most: usize) -> bool {
    let n = a.size();
    let inverse: Vec<f64> = (0..n).map(|i| a.diagonal(i)).map(|d| if d > 0.0 { 1.0 / d } else { 1.0 }).collect();
    let mut ax = vec![0.0; n];
    a.apply(x, &mut ax);
    let mut r: Vec<f64> = b.iter().zip(&ax).map(|(b, a)| b - a).collect();
    let mut z: Vec<f64> = r.iter().zip(&inverse).map(|(r, m)| r * m).collect();
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let scale = b.iter().map(|v| v * v).sum::<f64>().sqrt().max(x.iter().map(|v| v * v).sum::<f64>().sqrt() * 1e-6).max(1e-30);
    let mut ap = vec![0.0; n];
    for _ in 0..most {
        let residual = r.iter().map(|v| v * v).sum::<f64>().sqrt();
        if residual <= tolerance * scale {
            return true;
        }
        a.apply(&p, &mut ap);
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap <= 0.0 || !pap.is_finite() {
            break;
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
            z[i] = r[i] * inverse[i];
        }
        let next: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let beta = next / rz.max(1e-300);
        rz = next;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    let residual = r.iter().map(|v| v * v).sum::<f64>().sqrt();
    residual <= 1e-4 * scale && x.iter().all(|v| v.is_finite())
}

/// The border of some of the layout's triangles as loops of UV vertices,
/// the largest by area first, each running as its triangles do: where a
/// triangle of them meets one that is not, a seam or the edge of the part.
pub(crate) fn border_loops(layout: &UvLayout, triangles: &[u32]) -> Vec<Vec<u32>> {
    let cv = &layout.corner_vertex;
    let mut directed: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    for &t in triangles {
        for k in 0..3 {
            directed.insert((cv[t as usize * 3 + k], cv[t as usize * 3 + (k + 1) % 3]));
        }
    }
    let mut outgoing: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut ends: Vec<(u32, u32)> = Vec::new();
    for &t in triangles {
        for k in 0..3 {
            let (a, b) = (cv[t as usize * 3 + k], cv[t as usize * 3 + (k + 1) % 3]);
            if a != b && !directed.contains(&(b, a)) {
                outgoing.entry(a).or_default().push(ends.len() as u32);
                ends.push((a, b));
            }
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
            match outgoing.get(&to).and_then(|list| list.iter().copied().find(|&h| !used[h as usize])) {
                Some(h) => at = h as usize,
                None => break,
            }
        }
        let area = polygon_area(&path.iter().map(|&v| layout.uv_vertices[v as usize].uv).collect::<Vec<_>>());
        loops.push((area.abs(), path));
    }
    loops.sort_by(|a, b| b.0.total_cmp(&a.0));
    loops.into_iter().map(|(_, path)| path).collect()
}

/// A polygon's area, signed as [`facing`] signs a triangle's.
pub(crate) fn polygon_area(points: &[Vec2]) -> f64 {
    let mut area = 0.0f64;
    for i in 0..points.len() {
        let (p, q) = (points[i], points[(i + 1) % points.len()]);
        area += (q.x as f64) * (p.y as f64) - (p.x as f64) * (q.y as f64);
    }
    area * 0.5
}

/// The turn and scale that best put `from` on `to`, about their middles
/// (Umeyama 1991): the angle, u toward v, and the scale.
pub(crate) fn best_fit(from: &[Vec2], to: &[Vec2]) -> (f32, f32, Vec2, Vec2) {
    let n = from.len().max(1) as f64;
    let mean = |p: &[Vec2]| {
        let (x, y) = p.iter().fold((0.0f64, 0.0f64), |(x, y), q| (x + q.x as f64, y + q.y as f64));
        vec2((x / n) as f32, (y / n) as f32)
    };
    let (mf, mt) = (mean(from), mean(to));
    let (mut dot, mut cross, mut spread) = (0.0f64, 0.0f64, 0.0f64);
    for (a, b) in from.iter().zip(to) {
        let (a, b) = (*a - mf, *b - mt);
        dot += (a.x * b.x + a.y * b.y) as f64;
        cross += (a.x * b.y - a.y * b.x) as f64;
        spread += a.length_squared() as f64;
    }
    if spread <= 0.0 {
        return (0.0, 1.0, mf, mt);
    }
    let angle = cross.atan2(dot) as f32;
    let scale = ((dot * dot + cross * cross).sqrt() / spread) as f32;
    (angle, scale, mf, mt)
}

/// A point turned by `angle`, u toward v, about the origin.
pub(crate) fn turned(p: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos();
    vec2(p.x * c - p.y * s, p.x * s + p.y * c)
}

/// The middle of the box round some points.
pub(crate) fn box_middle(points: impl Iterator<Item = Vec2>) -> Vec2 {
    super::transform::middle(points)
}

/// Picked edges or vertices as UV vertices on one island, and that
/// island's edges between them, each once as a pair of UV vertices.
pub(crate) fn picked_chain(layout: &UvLayout, pick: &Pick) -> Result<(u32, Vec<(u32, u32)>), ShapeError> {
    if !pick.edges.is_empty() {
        let mut pairs = Vec::new();
        let mut island = None;
        for &edge in &pick.edges {
            let [a, b] = edge_ends(layout, edge)?;
            let on = layout.uv_vertices[a as usize].island;
            if island.is_some_and(|i| i != on) {
                return cannot("the picked edges are on more than one island");
            }
            island = Some(on);
            if a != b && !pairs.contains(&(a.min(b), a.max(b))) {
                pairs.push((a.min(b), a.max(b)));
            }
        }
        return Ok((island.expect("an edge"), pairs));
    }
    if pick.vertices.is_empty() {
        return cannot("pick edges (3) or vertices (4) first");
    }
    // The island every picked point has a place on.
    let places: Vec<Vec<u32>> = pick.vertices.iter().map(|&v| layout.uv_vertices_of(&[v])).collect();
    if let Some(i) = places.iter().position(|p| p.is_empty()) {
        return Err(UvError::NoSuchVertex(pick.vertices[i]).into());
    }
    let island_of = |v: u32| layout.uv_vertices[v as usize].island;
    let Some(island) = places[0].iter().map(|&v| island_of(v)).find(|&i| places.iter().all(|p| p.iter().any(|&v| island_of(v) == i))) else {
        return cannot("the picked vertices are on more than one island");
    };
    let mut chosen: Vec<u32> = places.iter().flat_map(|p| p.iter().copied().filter(|&v| island_of(v) == island)).collect();
    chosen.sort_unstable();
    chosen.dedup();
    let mut pairs = Vec::new();
    for edge in &layout.edges[layout.islands[island as usize].edges.clone()] {
        let [a, b] = edge.ends;
        if a != b && chosen.binary_search(&a).is_ok() && chosen.binary_search(&b).is_ok() && !pairs.contains(&(a.min(b), a.max(b))) {
            pairs.push((a.min(b), a.max(b)));
        }
    }
    if pairs.is_empty() {
        return cannot("no two of the picked vertices share an edge");
    }
    Ok((island, pairs))
}

/// Edges in a row: an open path from one end to the other, or a loop.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Chain {
    Path(Vec<u32>),
    Loop(Vec<u32>),
}

/// Edges, as pairs of vertices, put in a row: None where they branch or
/// are in more than one piece.
pub(crate) fn chain(edges: &[(u32, u32)]) -> Option<Chain> {
    let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(a, b) in edges {
        next.entry(a).or_default().push(b);
        next.entry(b).or_default().push(a);
    }
    if next.values().any(|n| n.len() > 2) || next.is_empty() {
        return None;
    }
    let ends: Vec<u32> = {
        let mut ends: Vec<u32> = next.iter().filter(|(_, n)| n.len() == 1).map(|(v, _)| *v).collect();
        ends.sort_unstable();
        ends
    };
    let closed = ends.is_empty();
    if !closed && ends.len() != 2 {
        return None;
    }
    let first = if closed { *next.keys().min().expect("a vertex") } else { ends[0] };
    let mut order = vec![first];
    let mut previous = u32::MAX;
    let mut at = first;
    loop {
        let Some(&to) = next[&at].iter().find(|&&n| n != previous && (order.len() < 2 || n != order[order.len() - 2])) else { break };
        if to == first {
            break;
        }
        if order.contains(&to) {
            return None;
        }
        order.push(to);
        previous = at;
        at = to;
    }
    if order.len() != next.len() {
        return None;
    }
    Some(if closed { Chain::Loop(order) } else { Chain::Path(order) })
}

/// A piece of the layout a tool shapes whole: an island's triangles, or
/// those of its faces picked on the part.
pub(crate) struct Region {
    pub island: u32,
    pub triangles: Vec<u32>,
    /// The whole island, not a part of it the rest must follow.
    pub whole: bool,
}

/// The regions a pick names: its faces picked on the part, island by
/// island; else the islands picked, or touched by what is picked; else
/// every island.
pub(crate) fn regions(layout: &UvLayout, pick: &Pick) -> Result<Vec<Region>, UvError> {
    if !pick.faces.is_empty() {
        let mut by_island: std::collections::BTreeMap<u32, Vec<u32>> = std::collections::BTreeMap::new();
        for &t in &pick.faces {
            let island = *layout.island_of.get(t as usize).ok_or(UvError::NothingChosen)?;
            by_island.entry(island).or_default().push(t);
        }
        return Ok(by_island
            .into_iter()
            .map(|(island, mut triangles)| {
                triangles.sort_unstable();
                triangles.dedup();
                let whole = triangles.len() == layout.islands[island as usize].triangles.len();
                Region { island, triangles, whole }
            })
            .collect());
    }
    let islands = islands_of_pick(layout, &Pick { faces: Vec::new(), ..pick.clone() })?;
    Ok(islands.into_iter().map(|island| Region { island, triangles: layout.islands[island as usize].triangles.clone(), whole: true }).collect())
}

/// The UV vertices of a pick's mesh vertices on one island.
pub(crate) fn on_island(layout: &UvLayout, vertices: &[u32], island: u32) -> Vec<u32> {
    layout.uv_vertices_of(vertices).into_iter().filter(|&v| layout.uv_vertices[v as usize].island == island).collect()
}

/// An island laid out again with some of its UV vertices placed: the rest
/// of it as true to the surface as it can be round them, and the pins held
/// too where a tool keeps them. Refused, saying what for, where it would
/// fold.
pub(crate) fn island_round(mesh: &Mesh, layout: &UvLayout, island: u32, placed: &[(u32, Vec2)], pins: &[u32], what: &str) -> Result<Moves, ShapeError> {
    let isl = &layout.islands[island as usize];
    let patch = Patch::new(mesh, layout, &isl.triangles);
    let mut held: Vec<(usize, Vec2)> = Vec::new();
    let mut taken = vec![false; patch.len()];
    for &(v, p) in placed {
        if let Some(&i) = patch.local.get(&v) {
            held.push((i, p));
            taken[i] = true;
        }
    }
    for &v in pins {
        if let Some(&i) = patch.local.get(&v) {
            if !taken[i] {
                held.push((i, patch.uv[i]));
                taken[i] = true;
            }
        }
    }
    if held.len() == patch.len() {
        let mut uv = patch.uv.clone();
        held.iter().for_each(|&(i, p)| uv[i] = p);
        return Ok(patch.moves(&uv));
    }
    match reflatten_holding(&patch, &held) {
        Some(uv) => Ok(patch.moves(&uv)),
        None => cannot(format!("island {} would fold {what}: cut it, or pick less", island + 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;

    #[test]
    fn a_fixed_border_fills_evenly_and_a_move_carries_the_rest_along() {
        let mesh = unwrapped(sheet(4, 4, |_, _| 0.0));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let island = &layout.islands[0];
        let patch = Patch::new(&mesh, &layout, &island.triangles);
        assert_eq!(patch.len(), 25);
        // The border where it is: the inside comes back where it was, the
        // sheet being flat and even.
        let border: Vec<(usize, Vec2)> = island.boundary[0].iter().map(|v| (patch.local[v], layout.uv_vertices[*v as usize].uv)).collect();
        let filled = even_fill(&patch, &border, &patch.weights()).unwrap();
        for (a, b) in filled.iter().zip(&patch.uv) {
            assert!((*a - *b).length() < 1e-4, "{a:?} {b:?}");
        }
        // One corner moved: the far corner stays nearly put, the near ones
        // go most of the way with it.
        let corner = border[0];
        let moved = follow(&patch, &[(corner.0, corner.1 + vec2(0.05, 0.0))], &patch.weights()).unwrap();
        let shift: Vec<f32> = moved.iter().zip(&patch.uv).map(|(a, b)| (*a - *b).length()).collect();
        assert!((shift[corner.0] - 0.05).abs() < 1e-5);
        assert!(shift.iter().all(|s| *s <= 0.05 + 1e-5));
    }

    #[test]
    fn a_patch_s_border_runs_round_it_and_a_fit_finds_the_turn() {
        let mesh = unwrapped(sheet(3, 3, |_, _| 0.0));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        // The first row's six triangles: a strip with one loop round it.
        let loops = border_loops(&layout, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 8);
        let whole = border_loops(&layout, &layout.islands[0].triangles);
        assert_eq!(whole[0].len(), layout.islands[0].boundary[0].len());
        let from = [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(0.0, 1.0)];
        let to: Vec<Vec2> = from.iter().map(|p| turned(*p * 2.0, 0.5) + vec2(3.0, 1.0)).collect();
        let (angle, scale, _, _) = best_fit(&from, &to);
        assert!((angle - 0.5).abs() < 1e-5 && (scale - 2.0).abs() < 1e-5);
    }
}
