//! A part's symmetry, and the layout made to keep it.
//!
//! **The plane.** A part modelled symmetric - a shell's two halves, a
//! bottle's front and back - is its own mirror image across a plane. The
//! candidates are the part's own X, Y and Z planes through the middle of
//! its box; Auto takes the one that pairs the most vertices. Each welded
//! vertex is paired with the vertex at its mirror image, within a
//! thousandth of the part's size, found through a grid of cells that size
//! rather than by trying every vertex against every other; a vertex on the
//! plane is its own pair, and one with no vertex at its image has none -
//! the part's two sides differ there.
//!
//! **On the layout.** A vertex of the part on a seam is a place on each
//! side of it, so a pair of vertices is not yet a pair of places: each
//! place takes the place of its pair whose neighbours are its own
//! neighbours' pairs ([`place_pairs`]). That does not ask the two halves to
//! be split into triangles alike, which a quad split along one diagonal
//! never is in its mirror.
//!
//! **What uses it.** Symmetrize makes one side's layout the mirror image of
//! the other's across a line of the layout; Snap to mirror puts picked
//! places on their pairs' images, across one line or two; Select one side
//! and Select mirror pick by where things are on the part.

use super::edit::UvError;
use super::layout::{weld, UvLayout};
use super::shape::{cannot, edge_ends, islands_of_pick, layout, Moves, Pick, ShapeError};
use crate::geometry::Mesh;
use std::collections::HashMap;
use tequila_core::math::{vec2, Vec2, Vec3};

/// No pair: the part's two sides differ there.
pub const NONE: u32 = u32::MAX;

/// Which plane a part is mirrored across.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Plane {
    /// The one of the three that pairs the most vertices.
    #[default]
    Auto,
    X,
    Y,
    Z,
}

impl Plane {
    pub fn key(self) -> &'static str {
        match self {
            Plane::Auto => "auto",
            Plane::X => "x",
            Plane::Y => "y",
            Plane::Z => "z",
        }
    }

    pub fn from_key(key: &str) -> Option<Plane> {
        match key {
            "auto" => Some(Plane::Auto),
            "x" | "X" => Some(Plane::X),
            "y" | "Y" => Some(Plane::Y),
            "z" | "Z" => Some(Plane::Z),
            _ => None,
        }
    }

    fn axis(self) -> Option<usize> {
        match self {
            Plane::Auto => None,
            Plane::X => Some(0),
            Plane::Y => Some(1),
            Plane::Z => Some(2),
        }
    }
}

/// A part's mirror: its plane and each welded vertex's pair.
#[derive(Clone, Debug, PartialEq)]
pub struct Mirror {
    /// The plane's axis, 0 for X, and where along it the plane is.
    pub axis: usize,
    pub at: f32,
    /// How close an image has to come to a vertex to be it.
    pub tolerance: f32,
    /// Each welded vertex's pair, itself on the plane, or [`NONE`].
    pub partner: Vec<u32>,
    /// Each welded vertex's side of the plane: -1, 0 on it, or 1.
    pub side: Vec<i8>,
    /// How many welded vertices have a pair, of how many.
    pub paired: usize,
    pub welded: usize,
}

impl Mirror {
    pub fn axis_name(&self) -> &'static str {
        ["X", "Y", "Z"][self.axis]
    }

    /// The share of the part's vertices with a pair.
    pub fn share(&self) -> f32 {
        if self.welded == 0 { 0.0 } else { self.paired as f32 / self.welded as f32 }
    }
}

/// The part's mirror across `plane`, or the best of the three. None for a
/// part with no vertices.
pub fn find_mirror(mesh: &Mesh, plane: Plane) -> Option<Mirror> {
    let welded = weld(&mesh.positions);
    let count = welded.iter().map(|&w| w as usize + 1).max()?;
    let mut points = vec![Vec3::ZERO; count];
    let mut seen = vec![false; count];
    for (v, &w) in welded.iter().enumerate() {
        if !seen[w as usize] {
            seen[w as usize] = true;
            points[w as usize] = mesh.positions[v];
        }
    }
    let (low, high) = points.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(l, h), p| (l.min(*p), h.max(*p)));
    let middle = (low + high) * 0.5;
    let tolerance = ((high - low).length() * 1e-3).max(1e-9);
    let grid = Cells::new(&points, tolerance);
    let on = |axis: usize| pair_across(&points, &grid, axis, middle[axis], tolerance);
    match plane.axis() {
        Some(axis) => Some(on(axis)),
        None => (0..3).map(on).reduce(|best, next| if next.paired > best.paired { next } else { best }),
    }
}

/// The best mirror across a plane other than `axis`'s: a part's second
/// symmetry.
pub fn find_second_mirror(mesh: &Mesh, axis: usize) -> Option<Mirror> {
    let others: Vec<Plane> = [Plane::X, Plane::Y, Plane::Z].into_iter().filter(|p| p.axis() != Some(axis)).collect();
    others.into_iter().filter_map(|p| find_mirror(mesh, p)).reduce(|best, next| if next.paired > best.paired { next } else { best })
}

/// Welded vertices by the cells of a grid of `size` they are in.
struct Cells {
    size: f32,
    cells: HashMap<(i64, i64, i64), Vec<u32>>,
}

impl Cells {
    fn new(points: &[Vec3], size: f32) -> Cells {
        let mut cells: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::with_capacity(points.len());
        for (i, p) in points.iter().enumerate() {
            cells.entry(Self::key(*p, size)).or_default().push(i as u32);
        }
        Cells { size, cells }
    }

    fn key(p: Vec3, size: f32) -> (i64, i64, i64) {
        ((p.x / size).floor() as i64, (p.y / size).floor() as i64, (p.z / size).floor() as i64)
    }

    /// The point nearest `p` within `within`.
    fn nearest(&self, points: &[Vec3], p: Vec3, within: f32) -> Option<u32> {
        let (x, y, z) = Self::key(p, self.size);
        let mut best: Option<(u32, f32)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    for &i in self.cells.get(&(x + dx, y + dy, z + dz)).map(|l| l.as_slice()).unwrap_or(&[]) {
                        let d = (points[i as usize] - p).length();
                        if d <= within && best.is_none_or(|(_, b)| d < b) {
                            best = Some((i, d));
                        }
                    }
                }
            }
        }
        best.map(|(i, _)| i)
    }
}

fn pair_across(points: &[Vec3], grid: &Cells, axis: usize, at: f32, tolerance: f32) -> Mirror {
    let mut partner = vec![NONE; points.len()];
    let mut side = vec![0i8; points.len()];
    for (w, p) in points.iter().enumerate() {
        let d = p[axis] - at;
        if d.abs() <= tolerance {
            partner[w] = w as u32;
            continue;
        }
        side[w] = if d > 0.0 { 1 } else { -1 };
        let mut image = *p;
        image[axis] = 2.0 * at - p[axis];
        if let Some(other) = grid.nearest(points, image, tolerance) {
            partner[w] = other;
        }
    }
    let paired = partner.iter().filter(|&&p| p != NONE).count();
    Mirror { axis, at, tolerance, partner, side, paired, welded: points.len() }
}

/// Each UV vertex's pair on the layout, or [`NONE`].
///
/// A vertex with one place has its pair's place. A vertex of the part on a
/// seam has a place each side, and its pair has as many: each of its places
/// takes the one of its pair's whose neighbours on the layout are the pairs
/// of its own neighbours - the matching side of the seam whatever way the
/// two halves' faces happen to be split into triangles - and, failing that,
/// the one on the island most of its own island's pairs are on.
pub fn place_pairs(layout: &UvLayout, mirror: &Mirror) -> Vec<u32> {
    let count = layout.uv_vertices.len();
    let mut copies: HashMap<u32, Vec<u32>> = HashMap::new();
    for (g, v) in layout.uv_vertices.iter().enumerate() {
        copies.entry(v.weld).or_default().push(g as u32);
    }
    let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); count];
    for t in 0..layout.island_of.len() {
        for k in 0..3 {
            let (a, b) = (layout.corner_vertex[t * 3 + k], layout.corner_vertex[t * 3 + (k + 1) % 3]);
            neighbours[a as usize].push(b);
            neighbours[b as usize].push(a);
        }
    }
    for list in &mut neighbours {
        list.sort_unstable();
        list.dedup();
    }
    let candidates = |g: usize| -> &[u32] {
        let pair = mirror.partner.get(layout.uv_vertices[g].weld as usize).copied().unwrap_or(NONE);
        if pair == NONE { &[] } else { copies.get(&pair).map(|c| c.as_slice()).unwrap_or(&[]) }
    };
    let mut out = vec![NONE; count];
    for (g, slot) in out.iter_mut().enumerate() {
        if let [only] = candidates(g) {
            *slot = *only;
        }
    }
    // Twice: a place whose neighbours were themselves on seams settles once
    // they have.
    for _ in 0..2 {
        let islands = mirror_islands(layout, &out);
        for g in 0..count {
            let options = candidates(g);
            if options.len() < 2 {
                continue;
            }
            let score = |c: u32| neighbours[g].iter().filter(|&&n| out[n as usize] != NONE && neighbours[c as usize].binary_search(&out[n as usize]).is_ok()).count();
            let on_mirror = |c: u32| layout.uv_vertices[c as usize].island == islands[layout.uv_vertices[g].island as usize];
            let best = options.iter().copied().max_by_key(|&c| (score(c), on_mirror(c), std::cmp::Reverse(c)));
            if let Some(best) = best {
                out[g] = best;
            }
        }
    }
    out
}

/// Each island's mirror: the island most of its places' pairs are on, or
/// [`NONE`].
pub fn mirror_islands(layout: &UvLayout, pairs: &[u32]) -> Vec<u32> {
    let mut votes: Vec<HashMap<u32, usize>> = vec![HashMap::new(); layout.islands.len()];
    for (g, &p) in pairs.iter().enumerate() {
        if p != NONE {
            *votes[layout.uv_vertices[g].island as usize].entry(layout.uv_vertices[p as usize].island).or_default() += 1;
        }
    }
    votes.iter().map(|v| v.iter().max_by_key(|(island, n)| (**n, std::cmp::Reverse(**island))).map(|(i, _)| *i).unwrap_or(NONE)).collect()
}

/// A line of the layout things are mirrored across.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Line {
    /// Up and down the layout at this u: left for right.
    Vertical(f32),
    /// Across it at this v: top for bottom.
    Horizontal(f32),
}

impl Line {
    pub fn reflect(self, p: Vec2) -> Vec2 {
        match self {
            Line::Vertical(u) => vec2(2.0 * u - p.x, p.y),
            Line::Horizontal(v) => vec2(p.x, 2.0 * v - p.y),
        }
    }

    /// The nearest point on the line.
    pub fn onto(self, p: Vec2) -> Vec2 {
        match self {
            Line::Vertical(u) => vec2(u, p.y),
            Line::Horizontal(v) => vec2(p.x, v),
        }
    }
}

/// Which side of the plane is copied.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// The side with more of the picked vertices; the plus side when that
    /// says nothing.
    #[default]
    Auto,
    Plus,
    Minus,
}

/// What symmetrizing did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SymmetrizeReport {
    pub axis: usize,
    /// Copied from the plus side.
    pub from_plus: bool,
    pub moved: usize,
    /// Vertices of the part on the side made over with no pair: the part's
    /// two sides differ there.
    pub unpaired: usize,
}

/// The least share of a part's vertices that must pair for it to count as
/// symmetric at all.
const SYMMETRIC: f32 = 0.25;

fn checked_mirror(mesh: &Mesh, plane: Plane) -> Result<Mirror, ShapeError> {
    let Some(mirror) = find_mirror(mesh, plane) else { return cannot("the part has no vertices") };
    if mirror.share() < SYMMETRIC {
        return cannot(format!("the part isn't symmetric across its {} plane: only {:.0}% of its vertices have a mirror", mirror.axis_name(), mirror.share() * 100.0));
    }
    Ok(mirror)
}

/// Each UV vertex's side of the plane: its vertex's, or for a vertex on the
/// plane that is not its own pair, its island's - where the middle of the
/// island's area is on the part.
fn sides(mesh: &Mesh, layout: &UvLayout, mirror: &Mirror, pairs: &[u32]) -> Vec<i8> {
    let island_side: Vec<i8> = layout
        .islands
        .iter()
        .map(|island| {
            let (mut sum, mut area) = (0.0f64, 0.0f64);
            for &t in &island.triangles {
                let [a, b, c] = mesh.triangle(t as usize).map(|v| mesh.positions[v as usize]);
                let w = (b - a).cross(c - a).length() as f64;
                sum += w * ((a[mirror.axis] + b[mirror.axis] + c[mirror.axis]) / 3.0 - mirror.at) as f64;
                area += w;
            }
            let d = if area > 0.0 { sum / area } else { 0.0 };
            if d.abs() <= mirror.tolerance as f64 { 0 } else if d > 0.0 { 1 } else { -1 }
        })
        .collect();
    layout
        .uv_vertices
        .iter()
        .enumerate()
        .map(|(g, v)| match mirror.side[v.weld as usize] {
            0 if pairs[g] != g as u32 => island_side[v.island as usize],
            side => side,
        })
        .collect()
}

/// The islands `pick` is on - every island when it picks nothing - made
/// symmetric: each place on the side opposite `source` put at its pair's
/// image across `line`, and a place that is its own pair onto the line.
pub fn symmetrize(mesh: &Mesh, set: usize, plane: Plane, pick: &Pick, line: Line, source: Source) -> Result<(Moves, SymmetrizeReport), ShapeError> {
    let layout = layout(mesh, set)?;
    let mirror = checked_mirror(mesh, plane)?;
    let pairs = place_pairs(&layout, &mirror);
    let side = sides(mesh, &layout, &mirror, &pairs);
    let from_plus = match source {
        Source::Plus => true,
        Source::Minus => false,
        Source::Auto => {
            let picked = layout.uv_vertices_of(&pick.vertices);
            let plus = picked.iter().filter(|&&g| side[g as usize] > 0).count();
            let minus = picked.iter().filter(|&&g| side[g as usize] < 0).count();
            plus >= minus
        }
    };
    let target: i8 = if from_plus { -1 } else { 1 };
    let islands = islands_of_pick(&layout, pick)?;
    let mut moves = Moves::new();
    let mut unpaired: Vec<u32> = Vec::new();
    for i in islands {
        for &g in &layout.islands[i as usize].uv_vertices {
            let pair = pairs[g as usize];
            let p = layout.uv_vertices[g as usize].uv;
            if pair == g {
                moves.push((g, line.onto(p)));
            } else if side[g as usize] == target {
                if pair == NONE {
                    unpaired.push(layout.uv_vertices[g as usize].weld);
                } else {
                    moves.push((g, line.reflect(layout.uv_vertices[pair as usize].uv)));
                }
            }
        }
    }
    unpaired.sort_unstable();
    unpaired.dedup();
    if moves.is_empty() {
        return cannot("nothing picked is on the side to make over, or has a mirror");
    }
    Ok((moves.clone(), SymmetrizeReport { axis: mirror.axis, from_plus, moved: moves.len(), unpaired: unpaired.len() }))
}

/// What snapping to the mirror did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SnapReport {
    pub snapped: usize,
    pub unpaired: usize,
}

/// Picked places - picked vertices, the ends of picked edges, or every
/// place of the picked islands - put on their pairs' images: across a
/// vertical line at `centre.x` (or a horizontal one at `centre.y`) for the
/// part's mirror across `plane`; with `both_ways`, across both lines for
/// its mirror and its second one. Where a pair is picked too, the two meet
/// half way, each the other's image.
pub fn snap_to_mirror(mesh: &Mesh, set: usize, plane: Plane, pick: &Pick, centre: Vec2, vertical: bool, both_ways: bool) -> Result<(Moves, SnapReport), ShapeError> {
    let layout = layout(mesh, set)?;
    let first = checked_mirror(mesh, plane)?;
    let line = if vertical { Line::Vertical(centre.x) } else { Line::Horizontal(centre.y) };
    let pairs = place_pairs(&layout, &first);
    let (second, other) = if both_ways {
        let Some(second) = find_second_mirror(mesh, first.axis).filter(|m| m.share() >= SYMMETRIC) else {
            return cannot(format!("the part isn't symmetric across a second plane as well as its {} plane", first.axis_name()));
        };
        let other = if vertical { Line::Horizontal(centre.y) } else { Line::Vertical(centre.x) };
        (Some(place_pairs(&layout, &second)), Some(other))
    } else {
        (None, None)
    };
    let mut picked: Vec<u32> = if !pick.vertices.is_empty() {
        layout.uv_vertices_of(&pick.vertices)
    } else if !pick.edges.is_empty() {
        let mut out = Vec::new();
        for &edge in &pick.edges {
            out.extend(edge_ends(&layout, edge)?);
        }
        out
    } else if !pick.islands.is_empty() || !pick.faces.is_empty() {
        islands_of_pick(&layout, pick)?.iter().flat_map(|&i| layout.islands[i as usize].uv_vertices.iter().copied()).collect()
    } else {
        return cannot("pick the vertices to snap to their mirror");
    };
    picked.sort_unstable();
    picked.dedup();
    let is_picked = |g: u32| picked.binary_search(&g).is_ok();
    let place = |g: u32| layout.uv_vertices[g as usize].uv;
    let mut moves = Moves::new();
    let mut unpaired = 0;
    for &g in &picked {
        // The group: the place, its pair, its second pair, and that pair's
        // pair, each with the map that takes it onto this one.
        let mut group: Vec<(u32, Box<dyn Fn(Vec2) -> Vec2>)> = vec![(g, Box::new(|p| p))];
        let p1 = pairs[g as usize];
        if p1 != NONE {
            group.push((p1, Box::new(move |p| line.reflect(p))));
        }
        if let (Some(second), Some(other)) = (&second, other) {
            let p2 = second[g as usize];
            if p2 != NONE {
                group.push((p2, Box::new(move |p| other.reflect(p))));
            }
            if p1 != NONE && second[p1 as usize] != NONE {
                group.push((second[p1 as usize], Box::new(move |p| line.reflect(other.reflect(p)))));
            }
        }
        if group.len() == 1 {
            unpaired += 1;
            continue;
        }
        // The places that stay say where it goes; all picked, they meet.
        let fixed: Vec<&(u32, Box<dyn Fn(Vec2) -> Vec2>)> = group.iter().filter(|(m, _)| *m != g && !is_picked(*m)).collect();
        let at = if fixed.is_empty() {
            group.iter().fold(Vec2::ZERO, |s, (m, f)| s + f(place(*m))) / group.len() as f32
        } else {
            fixed.iter().fold(Vec2::ZERO, |s, (m, f)| s + f(place(*m))) / fixed.len() as f32
        };
        moves.push((g, at));
    }
    if moves.is_empty() {
        return cannot("none of the picked vertices has a mirror on the part");
    }
    Ok((moves.clone(), SnapReport { snapped: moves.len(), unpaired }))
}

/// What lies on one side of the part's middle, along an axis of its own.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Side {
    /// UV vertices whose points are on that side.
    pub places: Vec<u32>,
    /// Layout edges with both ends there.
    pub edges: Vec<u32>,
    /// Islands the middle of whose area is there.
    pub islands: Vec<u32>,
}

/// What of the layout lies on the `positive` or negative side of the
/// part's middle along `axis` (0 for X).
pub fn one_side(mesh: &Mesh, set: usize, axis: usize, positive: bool) -> Result<Side, UvError> {
    let layout = super::edit::layout_of(mesh, set)?;
    let (low, high) = mesh.positions.iter().fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(l, h), p| (l.min(*p), h.max(*p)));
    let middle = ((low + high) * 0.5)[axis.min(2)];
    let tolerance = (high - low).length() * 1e-4;
    let sign = if positive { 1.0 } else { -1.0 };
    let beyond = |d: f32| d * sign > tolerance;
    let on = |g: u32| beyond(mesh.positions[layout.uv_vertices[g as usize].vertex as usize][axis] - middle);
    let places: Vec<u32> = (0..layout.uv_vertices.len() as u32).filter(|&g| on(g)).collect();
    let edges: Vec<u32> = layout.edges.iter().enumerate().filter(|(_, e)| on(e.ends[0]) && on(e.ends[1])).map(|(i, _)| i as u32).collect();
    let islands: Vec<u32> = layout
        .islands
        .iter()
        .enumerate()
        .filter(|(_, island)| {
            let (mut sum, mut area) = (0.0f64, 0.0f64);
            for &t in &island.triangles {
                let [a, b, c] = mesh.triangle(t as usize).map(|v| mesh.positions[v as usize]);
                let w = (b - a).cross(c - a).length() as f64;
                sum += w * ((a[axis] + b[axis] + c[axis]) / 3.0 - middle) as f64;
                area += w;
            }
            area > 0.0 && beyond((sum / area) as f32)
        })
        .map(|(i, _)| i as u32)
        .collect();
    Ok(Side { places, edges, islands })
}

/// The mirror images of some of the layout: places, layout edges and
/// islands, each that has one, each once.
pub fn mirrored(mesh: &Mesh, set: usize, plane: Plane, places: &[u32], edges: &[u32], islands: &[u32]) -> Result<Side, ShapeError> {
    let layout = layout(mesh, set)?;
    let mirror = checked_mirror(mesh, plane)?;
    let pairs = place_pairs(&layout, &mirror);
    let island_pairs = mirror_islands(&layout, &pairs);
    let mut out = Side::default();
    for &g in places {
        if let Some(&p) = pairs.get(g as usize).filter(|&&p| p != NONE) {
            out.places.push(p);
        }
    }
    let by_ends: HashMap<(u32, u32), u32> = layout.edges.iter().enumerate().map(|(i, e)| ((e.ends[0].min(e.ends[1]), e.ends[0].max(e.ends[1])), i as u32)).collect();
    for &e in edges {
        let Some(edge) = layout.edges.get(e as usize) else { continue };
        let [a, b] = edge.ends.map(|g| pairs[g as usize]);
        if a != NONE && b != NONE {
            if let Some(&found) = by_ends.get(&(a.min(b), a.max(b))) {
                out.edges.push(found);
            }
        }
    }
    for &i in islands {
        if let Some(&other) = island_pairs.get(i as usize).filter(|&&o| o != NONE) {
            out.islands.push(other);
        }
    }
    for list in [&mut out.places, &mut out.edges, &mut out.islands] {
        list.sort_unstable();
        list.dedup();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::set_positions;
    use crate::uv::UvLayout;
    use tequila_core::math::vec3;

    /// A part symmetric across its X plane: a wavy sheet from x = -1 to 1,
    /// as high at -x as at x, laid out by where it is seen from above but
    /// for a dent in the minus half.
    fn wavy() -> Mesh {
        let mesh = sheet(6, 8, |x, z| ((x * 2.0 - 1.0).abs() * 2.5).sin() * 0.2 + z * 0.1);
        let positions: Vec<Vec3> = mesh.positions.iter().map(|p| vec3(p.x * 2.0 - 1.0, p.y, p.z)).collect();
        let uv: Vec<Vec2> = positions.iter().map(|p| {
            let dent = if p.x < -0.2 { 0.04 * (p.z * 9.0).sin() } else { 0.0 };
            vec2(p.x * 0.4 + 0.5, p.z * 0.4 + 0.3 + dent)
        }).collect();
        Mesh::new(positions, mesh.indices.clone()).with_uvs(uv)
    }

    #[test]
    fn the_plane_is_found_and_every_vertex_paired_through_the_grid() {
        let mesh = wavy();
        let mirror = find_mirror(&mesh, Plane::Auto).unwrap();
        assert_eq!(mirror.axis, 0, "across X");
        assert_eq!(mirror.paired, mirror.welded, "every vertex has its pair");
        // A pair's pair is the vertex itself.
        for (w, &p) in mirror.partner.iter().enumerate() {
            assert_eq!(mirror.partner[p as usize], w as u32);
        }
        assert!(find_mirror(&mesh, Plane::Y).unwrap().paired < mirror.paired / 2);
    }

    #[test]
    fn symmetrize_mirrors_the_plus_side_over_the_minus_side() {
        let mesh = wavy();
        let (moves, report) = symmetrize(&mesh, 0, Plane::Auto, &Pick::default(), Line::Vertical(0.5), Source::Auto).unwrap();
        assert!(report.from_plus && report.unpaired == 0);
        let out = set_positions(&mesh, 0, &moves).unwrap();
        // Every vertex's place is its pair's, mirrored across u = 0.5.
        for (v, p) in out.positions.iter().enumerate() {
            let pair = out.positions.iter().position(|q| (*q - vec3(-p.x, p.y, p.z)).length() < 1e-5).unwrap();
            let (a, b) = (out.uv_sets[0][v], out.uv_sets[0][pair]);
            assert!((a.x - (1.0 - b.x)).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5, "{a:?} {b:?}");
        }
        assert!(crate::uv::UvLayout::new(&out, 0).unwrap().folded_triangles().is_empty());
    }

    /// The part cut down its plane into two islands a gap apart, the minus
    /// one dented: made over from the plus one, it is its mirror image,
    /// the copies of the vertices on the plane paired across the seam.
    #[test]
    fn two_halves_on_islands_of_their_own_are_made_mirror_images() {
        let mesh = wavy();
        let middle: Vec<[u32; 2]> = (0..mesh.triangle_count())
            .flat_map(|t| {
                let tri = mesh.triangle(t);
                (0..3).map(move |k| [tri[k], tri[(k + 1) % 3]])
            })
            .filter(|[a, b]| mesh.positions[*a as usize].x.abs() < 1e-5 && mesh.positions[*b as usize].x.abs() < 1e-5 && a < b)
            .collect();
        let halves = crate::uv::cut(&mesh, 0, &middle, &crate::uv::CutOptions { reflatten: false, gap: 0.1 }).unwrap();
        let layout = UvLayout::new(&halves, 0).unwrap();
        assert_eq!(layout.islands.len(), 2);
        let mirror = find_mirror(&halves, Plane::X).unwrap();
        let pairs = place_pairs(&layout, &mirror);
        assert_eq!(mirror_islands(&layout, &pairs), vec![1, 0], "each half the other's mirror");
        assert!(pairs.iter().enumerate().all(|(g, &p)| p != NONE && layout.uv_vertices[g].island != layout.uv_vertices[p as usize].island), "every place paired across");
        let (moves, report) = symmetrize(&halves, 0, Plane::X, &Pick::default(), Line::Vertical(0.5), Source::Plus).unwrap();
        assert_eq!(report.unpaired, 0);
        let out = set_positions(&halves, 0, &moves).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        assert_eq!(after.islands.len(), 2);
        assert!(after.folded_triangles().is_empty());
        let pairs = place_pairs(&after, &find_mirror(&out, Plane::X).unwrap());
        for (g, &p) in pairs.iter().enumerate() {
            let (a, b) = (after.uv_vertices[g].uv, after.uv_vertices[p as usize].uv);
            assert!((a.x - (1.0 - b.x)).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5, "{a:?} {b:?}");
        }
    }

    #[test]
    fn snapping_puts_picked_places_on_their_pair_s_image() {
        let mesh = wavy();
        let dented: Vec<u32> = (0..mesh.positions.len() as u32).filter(|&v| mesh.positions[v as usize].x < -0.2).collect();
        let (moves, report) = snap_to_mirror(&mesh, 0, Plane::X, &Pick::vertices(dented.clone()), vec2(0.5, 0.5), true, false).unwrap();
        assert_eq!(report.snapped, dented.len());
        let out = set_positions(&mesh, 0, &moves).unwrap();
        for &v in &dented {
            let p = out.positions[v as usize];
            assert!((out.uv_sets[0][v as usize].y - (p.z * 0.4 + 0.3)).abs() < 1e-5, "the dent is gone");
        }
    }

    #[test]
    fn one_side_and_the_mirror_of_a_selection() {
        let mesh = wavy();
        let plus = one_side(&mesh, 0, 0, true).unwrap();
        let minus = one_side(&mesh, 0, 0, false).unwrap();
        assert_eq!(plus.places.len(), minus.places.len());
        assert!(!plus.places.is_empty() && plus.places.iter().all(|g| !minus.places.contains(g)));
        let images = mirrored(&mesh, 0, Plane::X, &plus.places, &plus.edges, &[]).unwrap();
        assert_eq!(images.places, minus.places, "the mirror of one side is the other");
        // Each quad's diagonal mirrors to the other diagonal, which a sheet
        // split all one way has not: the rows and columns, 21 and 24.
        assert_eq!((plus.edges.len(), images.edges.len()), (63, 45));
    }
}
