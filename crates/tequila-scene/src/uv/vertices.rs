//! Vertices of the layout lined up: along one line across or down, onto the
//! straight line between the outermost, spread evenly along the path they
//! make, and the edges of a strip that are nearly square to the layout made
//! exactly so. Each works out where the vertices go - the UV vertices, and
//! their places - for `uv.set_positions` to put them.

use super::layout::{EdgeKind, UvLayout};
use crate::geometry::Mesh;
use std::collections::HashMap;
use tequila_core::math::{vec2, Vec2};

/// Where vertices go: the layout's UV vertices, and each one's place.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Places {
    pub vertices: Vec<u32>,
    pub uv: Vec<[f32; 2]>,
}

impl Places {
    pub fn push(&mut self, vertex: u32, at: Vec2) {
        match self.vertices.iter().position(|&v| v == vertex) {
            Some(i) => self.uv[i] = [at.x, at.y],
            None => {
                self.vertices.push(vertex);
                self.uv.push([at.x, at.y]);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Without the vertices that would not move.
    pub fn moving(self, layout: &UvLayout) -> Places {
        let mut out = Places::default();
        for (v, [u, w]) in self.vertices.into_iter().zip(self.uv) {
            if (layout.uv_vertices[v as usize].uv - vec2(u, w)).length() > 1e-9 {
                out.vertices.push(v);
                out.uv.push([u, w]);
            }
        }
        out
    }
}

/// Vertices joined by edges of the layout: in order along it where they
/// make a path, which has two ends; as they come where they branch or close
/// on themselves.
#[derive(Clone, Debug, PartialEq)]
pub struct Chain {
    pub vertices: Vec<u32>,
    pub path: bool,
}

/// The chains some edges of the layout (by number) make, each joined part
/// once.
pub fn edge_chains(layout: &UvLayout, edges: &[u32]) -> Vec<Chain> {
    let mut joins: HashMap<u32, Vec<u32>> = HashMap::new();
    for &e in edges {
        let Some(edge) = layout.edges.get(e as usize) else { continue };
        let [a, b] = edge.ends;
        if a == b {
            continue;
        }
        let list = joins.entry(a).or_default();
        if !list.contains(&b) {
            list.push(b);
        }
        let list = joins.entry(b).or_default();
        if !list.contains(&a) {
            list.push(a);
        }
    }
    chains_of(joins)
}

/// The chains the layout's edges between some of its vertices make; a
/// vertex joined to none of the others is a chain of its own.
pub fn vertex_chains(layout: &UvLayout, vertices: &[u32]) -> Vec<Chain> {
    let chosen: std::collections::HashSet<u32> = vertices.iter().copied().collect();
    let mut joins: HashMap<u32, Vec<u32>> = vertices.iter().map(|&v| (v, Vec::new())).collect();
    for edge in &layout.edges {
        let [a, b] = edge.ends;
        if a != b && chosen.contains(&a) && chosen.contains(&b) {
            for (from, to) in [(a, b), (b, a)] {
                let list = joins.get_mut(&from).expect("chosen");
                if !list.contains(&to) {
                    list.push(to);
                }
            }
        }
    }
    chains_of(joins)
}

fn chains_of(joins: HashMap<u32, Vec<u32>>) -> Vec<Chain> {
    let mut keys: Vec<u32> = joins.keys().copied().collect();
    keys.sort_unstable();
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for &start in &keys {
        if seen.contains(&start) {
            continue;
        }
        // The joined part, then whether it is a path.
        let mut part = vec![start];
        seen.insert(start);
        let mut at = 0;
        while at < part.len() {
            for &next in &joins[&part[at]] {
                if seen.insert(next) {
                    part.push(next);
                }
            }
            at += 1;
        }
        let ends: Vec<u32> = part.iter().copied().filter(|v| joins[v].len() <= 1).collect();
        let simple = part.iter().all(|v| joins[v].len() <= 2);
        if simple && (ends.len() == 2 || part.len() == 1) {
            let mut path = vec![*ends.iter().min().unwrap_or(&start)];
            while path.len() < part.len() {
                let last = *path.last().expect("one");
                let before = path.len().checked_sub(2).map(|i| path[i]);
                match joins[&last].iter().copied().find(|&n| Some(n) != before && !path.contains(&n)) {
                    Some(n) => path.push(n),
                    None => break,
                }
            }
            out.push(Chain { vertices: path, path: true });
        } else {
            part.sort_unstable();
            out.push(Chain { vertices: part, path: false });
        }
    }
    out
}

/// Which way vertices are lined up, and to which side of what.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineUp {
    /// Across the layout - the same u - or down it, the same v.
    pub across: bool,
    /// -1 the low side (left, top), 0 the middle, 1 the high side.
    pub side: i8,
    /// What to line up against: a span of u (or v), low to high; each
    /// group's own where there is none.
    pub against: Option<(f32, f32)>,
}

/// Each group of vertices given the same u (`across`) or v: its own low
/// side, middle or high side, or that of the span it is lined up against.
pub fn line_up(layout: &UvLayout, groups: &[Vec<u32>], how: LineUp) -> Places {
    let mut out = Places::default();
    let along = |p: Vec2| if how.across { p.x } else { p.y };
    for group in groups.iter().filter(|g| !g.is_empty()) {
        let (low, high) = how.against.unwrap_or_else(|| {
            group.iter().map(|&v| along(layout.uv_vertices[v as usize].uv)).fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), x| (a.min(x), b.max(x)))
        });
        let to = match how.side {
            s if s < 0 => low,
            0 => (low + high) * 0.5,
            _ => high,
        };
        for &v in group {
            let p = layout.uv_vertices[v as usize].uv;
            out.push(v, if how.across { vec2(to, p.y) } else { vec2(p.x, to) });
        }
    }
    out.moving(layout)
}

/// Each group of vertices put on the straight line between the two of it
/// farthest apart, each where it is nearest.
pub fn onto_line(layout: &UvLayout, groups: &[Vec<u32>]) -> Places {
    let mut out = Places::default();
    for group in groups.iter().filter(|g| g.len() >= 3) {
        let at = |v: u32| layout.uv_vertices[v as usize].uv;
        let (mut a, mut b, mut far) = (at(group[0]), at(group[0]), -1.0f32);
        for (i, &p) in group.iter().enumerate() {
            for &q in &group[i + 1..] {
                let d = (at(p) - at(q)).length_squared();
                if d > far {
                    (a, b, far) = (at(p), at(q), d);
                }
            }
        }
        let line = b - a;
        if line.length_squared() <= 1e-24 {
            continue;
        }
        for &v in group {
            let t = (at(v) - a).dot(line) / line.length_squared();
            out.push(v, a + line * t);
        }
    }
    out.moving(layout)
}

/// A path's vertices spread along it, its ends where they are: at even
/// steps along its length on the layout, or with `by_part`, at steps in
/// proportion to its edges' lengths on the part.
pub fn spread(layout: &UvLayout, mesh: &Mesh, path: &[u32], by_part: bool) -> Places {
    let mut out = Places::default();
    let n = path.len();
    if n < 3 {
        return out;
    }
    let at = |v: u32| layout.uv_vertices[v as usize].uv;
    // The path as drawn, by length along it.
    let mut lengths = vec![0.0f32; n];
    for i in 1..n {
        lengths[i] = lengths[i - 1] + (at(path[i]) - at(path[i - 1])).length();
    }
    let total = lengths[n - 1];
    if total <= 0.0 {
        return out;
    }
    // Where each vertex is to be, as a share of the way along.
    let shares: Vec<f32> = if by_part {
        let point = |v: u32| mesh.positions[layout.uv_vertices[v as usize].vertex as usize];
        let mut along = vec![0.0f32; n];
        for i in 1..n {
            along[i] = along[i - 1] + (point(path[i]) - point(path[i - 1])).length();
        }
        if along[n - 1] <= 0.0 {
            return out;
        }
        along.iter().map(|d| d / along[n - 1]).collect()
    } else {
        (0..n).map(|i| i as f32 / (n - 1) as f32).collect()
    };
    let place = |share: f32| -> Vec2 {
        let d = share * total;
        let i = lengths.partition_point(|&l| l < d).clamp(1, n - 1);
        let span = lengths[i] - lengths[i - 1];
        let t = if span > 0.0 { (d - lengths[i - 1]) / span } else { 0.0 };
        let (from, to) = (at(path[i - 1]), at(path[i]));
        from + (to - from) * t.clamp(0.0, 1.0)
    };
    let targets: Vec<Vec2> = shares[1..n - 1].iter().map(|&s| place(s)).collect();
    for (&v, p) in path[1..n - 1].iter().zip(targets) {
        out.push(v, p);
    }
    out.moving(layout)
}

/// An edge of the layout that is the long diagonal of a quad split in two:
/// the longest side, on the part, of the triangles on both its sides. A
/// strip's diagonals run nearly along it and are no edge to square.
fn is_diagonal(layout: &UvLayout, mesh: &Mesh, edge: u32) -> bool {
    let e = &layout.edges[edge as usize];
    if e.kind != EdgeKind::Inner {
        return false;
    }
    let Some(sides) = layout.edge_group(e.vertices) else { return false };
    let length = |a: u32, b: u32| (mesh.positions[a as usize] - mesh.positions[b as usize]).length();
    let mine = length(e.vertices[0], e.vertices[1]);
    let longest_of = |t: u32| {
        let [a, b, c] = mesh.triangle(t as usize);
        length(a, b).max(length(b, c)).max(length(c, a))
    };
    let sides = &layout.table.sides[sides];
    sides.len() == 2 && sides.iter().all(|s| mine >= longest_of(s.triangle) * 0.9999)
}

/// The edges among `edges` (by number) within `tolerance` radians of
/// across the layout made exactly across, and those within it of down made
/// exactly down, by moving their vertices: the vertices joined by such edges
/// take their average v (or u) together. A quad's long diagonal is left out,
/// and so is a run of edges that would close up an edge the other way, or
/// turn over or flatten a face round it: on a real part's layout - a strip
/// with a ragged end, a sliver joined on - the rest is squared and those
/// runs are left as they were.
pub fn square_edges(layout: &UvLayout, mesh: &Mesh, edges: &[u32], tolerance: f32) -> Places {
    let mut across = super::layout::UnionFind::new(layout.uv_vertices.len());
    let mut down = super::layout::UnionFind::new(layout.uv_vertices.len());
    let (mut flat, mut upright) = (Vec::new(), Vec::new());
    for &e in edges {
        let Some(edge) = layout.edges.get(e as usize) else { continue };
        let [a, b] = edge.ends;
        let d = layout.uv_vertices[b as usize].uv - layout.uv_vertices[a as usize].uv;
        if d.length() <= 0.0 || is_diagonal(layout, mesh, e) {
            continue;
        }
        let angle = d.y.abs().atan2(d.x.abs());
        if angle <= tolerance {
            across.union(a as usize, b as usize);
            flat.push([a, b]);
        } else if angle >= std::f32::consts::FRAC_PI_2 - tolerance {
            down.union(a as usize, b as usize);
            upright.push([a, b]);
        }
    }
    let mut out = Places::default();
    let mut current: HashMap<u32, Vec2> = HashMap::new();
    // The faces round each vertex, to see that a run turns none over.
    let mut faces_of: HashMap<u32, Vec<u32>> = HashMap::new();
    for [a, b] in flat.iter().chain(&upright) {
        for v in [*a, *b] {
            faces_of.entry(v).or_default();
        }
    }
    for (c, &v) in layout.corner_vertex.iter().enumerate() {
        if let Some(list) = faces_of.get_mut(&v) {
            list.push((c / 3) as u32);
        }
    }
    for (sets, own, other, is_across) in [(&mut across, &flat, &upright, true), (&mut down, &upright, &flat, false)] {
        // Each run of joined vertices, and the average it takes.
        let mut runs: HashMap<usize, Vec<u32>> = HashMap::new();
        for [a, b] in own {
            for v in [*a, *b] {
                let root = sets.find(v as usize);
                let list = runs.entry(root).or_default();
                if !list.contains(&v) {
                    list.push(v);
                }
            }
        }
        // A run holding both ends of an edge the other way would close it.
        let mut closing: std::collections::HashSet<usize> = std::collections::HashSet::new();
        for [a, b] in other {
            let (ra, rb) = (sets.find(*a as usize), sets.find(*b as usize));
            if ra == rb && runs.contains_key(&ra) {
                closing.insert(ra);
            }
        }
        let mut roots: Vec<usize> = runs.keys().copied().collect();
        roots.sort_unstable();
        for root in roots {
            if closing.contains(&root) {
                continue;
            }
            let run = &runs[&root];
            let place = |current: &HashMap<u32, Vec2>, v: u32| *current.get(&v).unwrap_or(&layout.uv_vertices[v as usize].uv);
            let mean = run.iter().map(|&v| if is_across { place(&current, v).y } else { place(&current, v).x }).sum::<f32>() / run.len() as f32;
            let mut proposed = current.clone();
            for &v in run {
                let p = place(&current, v);
                proposed.insert(v, if is_across { vec2(p.x, mean) } else { vec2(mean, p.y) });
            }
            // Every face round the run still facing its island's way, and
            // with area left.
            let keeps = run.iter().flat_map(|v| faces_of.get(v).into_iter().flatten()).all(|&t| {
                let island = &layout.islands[layout.island_of[t as usize] as usize];
                let sign = if island.mirrored { -1.0 } else { 1.0 };
                let hair = island.uv_area * super::flatten::FOLD_HAIR;
                let corners = |positions: &HashMap<u32, Vec2>| [0, 1, 2].map(|k| place(positions, layout.corner_vertex[t as usize * 3 + k]));
                let [a, b, c] = corners(&current);
                let [p, q, r] = corners(&proposed);
                let before = super::flatten::facing(a, b, c) * sign;
                let after = super::flatten::facing(p, q, r) * sign;
                before <= hair || after > hair
            });
            if !keeps {
                continue;
            }
            for &v in run {
                out.push(v, proposed[&v]);
            }
            current = proposed;
        }
    }
    out.moving(layout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;

    /// A strip of eight quads laid out a little crooked and wavy.
    fn crooked_strip() -> Mesh {
        let strip = sheet(1, 8, |_, _| 0.0);
        let uv = strip.positions.iter().map(|p| vec2(0.1 + p.x * 0.8 + p.z * 0.01, 0.4 + p.z * 0.1 + (p.x * 20.0).sin() * 0.004 + p.x * 0.02)).collect();
        strip.with_uvs(uv)
    }

    #[test]
    fn edges_and_vertices_make_chains_in_order() {
        let mesh = crooked_strip();
        let layout = UvLayout::new(&mesh, 0).unwrap();
        // The strip's top: the edge of the part along z = 0.
        let top: Vec<u32> = (0..layout.edges.len() as u32)
            .filter(|&e| layout.edges[e as usize].kind == EdgeKind::Boundary && layout.edges[e as usize].vertices.iter().all(|&v| mesh.positions[v as usize].z == 0.0))
            .collect();
        assert_eq!(top.len(), 8);
        let chains = edge_chains(&layout, &top);
        assert_eq!(chains.len(), 1);
        assert!(chains[0].path && chains[0].vertices.len() == 9);
        let xs: Vec<f32> = chains[0].vertices.iter().map(|&v| layout.uv_vertices[v as usize].uv.x).collect();
        assert!(xs.windows(2).all(|w| w[0] < w[1]) || xs.windows(2).all(|w| w[0] > w[1]), "in order: {xs:?}");
        let again = vertex_chains(&layout, &chains[0].vertices);
        assert_eq!(again.len(), 1);
        assert!(again[0].path);
    }

    #[test]
    fn vertices_line_up_by_a_side_or_onto_a_line() {
        let mesh = crooked_strip();
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let top: Vec<u32> = (0..layout.uv_vertices.len() as u32).filter(|&v| mesh.positions[layout.uv_vertices[v as usize].vertex as usize].z == 0.0).collect();
        let lowest = top.iter().map(|&v| layout.uv_vertices[v as usize].uv.y).fold(f32::INFINITY, f32::min);
        let places = line_up(&layout, &[top.clone()], LineUp { across: false, side: -1, against: None });
        let out = crate::uv::set_positions(&mesh, 0, &places.vertices, &places.uv).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        for &v in &top {
            assert!((after.uv_vertices[v as usize].uv.y - lowest).abs() < 1e-6);
        }
        let middle = line_up(&layout, &[top.clone()], LineUp { across: false, side: 0, against: Some((0.0, 1.0)) });
        assert!(middle.uv.iter().all(|p| (p[1] - 0.5).abs() < 1e-6));
        let onto = onto_line(&layout, &[top.clone()]);
        let out = crate::uv::set_positions(&mesh, 0, &onto.vertices, &onto.uv).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        let points: Vec<Vec2> = top.iter().map(|&v| after.uv_vertices[v as usize].uv).collect();
        let (a, b) = (points[0], points[points.len() - 1]);
        for p in &points {
            let cross = (b - a).x * (*p - a).y - (b - a).y * (*p - a).x;
            assert!(cross.abs() < 1e-6, "on the line");
        }
    }

    #[test]
    fn a_path_spreads_evenly_or_as_the_part_has_it() {
        let strip = sheet(1, 4, |_, _| 0.0);
        // Laid out bunched up toward one end.
        let uv = strip.positions.iter().map(|p| vec2(0.1 + p.x * p.x * 0.8, 0.3 + p.z * 0.2)).collect();
        let mesh = strip.with_uvs(uv);
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let top: Vec<u32> = (0..layout.edges.len() as u32)
            .filter(|&e| layout.edges[e as usize].kind == EdgeKind::Boundary && layout.edges[e as usize].vertices.iter().all(|&v| mesh.positions[v as usize].z == 0.0))
            .collect();
        let path = edge_chains(&layout, &top).remove(0).vertices;
        let even = spread(&layout, &mesh, &path, false);
        let out = crate::uv::set_positions(&mesh, 0, &even.vertices, &even.uv).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        let xs: Vec<f32> = path.iter().map(|&v| after.uv_vertices[v as usize].uv.x).collect();
        let steps: Vec<f32> = xs.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
        assert!(steps.iter().all(|s| (s - 0.2).abs() < 1e-5), "{steps:?}");
        // On the part the quads are all as long, so by the part is even too.
        let by_part = spread(&layout, &mesh, &path, true);
        for (a, b) in even.uv.iter().zip(&by_part.uv) {
            assert!((a[0] - b[0]).abs() < 1e-5);
        }
    }

    #[test]
    fn a_crooked_strip_is_squared_but_its_diagonals_are_left() {
        let mesh = crooked_strip();
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let all: Vec<u32> = (0..layout.edges.len() as u32).collect();
        let places = square_edges(&layout, &mesh, &all, 15f32.to_radians());
        let out = crate::uv::set_positions(&mesh, 0, &places.vertices, &places.uv).expect("no fold");
        let after = UvLayout::new(&out, 0).unwrap();
        for edge in &after.edges {
            let [a, b] = edge.vertices.map(|v| mesh.positions[v as usize]);
            let [p, q] = edge.ends.map(|v| after.uv_vertices[v as usize].uv);
            if (a.z - b.z).abs() < 1e-6 {
                assert!((p.y - q.y).abs() < 1e-6, "along the strip, across exactly");
            } else if (a.x - b.x).abs() < 1e-6 {
                assert!((p.x - q.x).abs() < 1e-6, "its rungs, down exactly");
            }
        }
        assert!(after.islands[0].uv_area > layout.islands[0].uv_area * 0.8, "not flattened");
    }
}
