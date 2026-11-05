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
pub(crate) fn is_diagonal(layout: &UvLayout, mesh: &Mesh, edge: u32) -> bool {
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

/// What lining up edges did: the edges it found near enough to across or
/// down, by number, each to be made exactly so; those it had to leave as
/// they were, and of those the ones that would have put two vertices on
/// one place or a face's corners on one line; and how many other vertices
/// it carried along so the faces round the lined-up ones did not turn over.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SquareReport {
    pub across: Vec<u32>,
    pub down: Vec<u32>,
    pub left: Vec<u32>,
    pub clashing: Vec<u32>,
    pub carried: usize,
}

/// The edges among `edges` (by number) within `tolerance` radians of
/// across the layout made exactly across, and those within it of down made
/// exactly down, by moving their vertices: the vertices joined by such edges
/// take their average v (or u) together; a picked path of edges that runs
/// within `tolerance` of across or down end to end goes whole. A quad's long
/// diagonal is left out, and so is an edge that would put two vertices on
/// one place, joining runs across that hold two vertices a run down holds.
pub fn square_edges(layout: &UvLayout, mesh: &Mesh, edges: &[u32], tolerance: f32) -> Places {
    square_edges_reported(layout, mesh, edges, tolerance).0
}

/// [`square_edges`], and what it did.
///
/// All the runs go at once - a turned strip needs it, each row alone would
/// cross its neighbours still turned - moving only their own vertices where
/// that turns no face over or flattens it. Where it would, on a real part's
/// layout - a strip with a ragged end, a sliver joined on - the island's
/// other vertices are carried along by the runs' moves (`shape::carried`), so
/// its lined-up edges still end exactly straight; only where even that
/// folds are the island's runs put one by one, those that would fold left
/// as they were and reported.
pub fn square_edges_reported(layout: &UvLayout, mesh: &Mesh, edges: &[u32], tolerance: f32) -> (Places, SquareReport) {
    let mut report = SquareReport::default();
    let angle_of = |d: Vec2| d.y.abs().atan2(d.x.abs());
    // The edges near enough to across or down, each with how far off it
    // is: a picked path that runs, end to end, within the tolerance of
    // across or down goes whole, every edge of it however its single edges
    // wander - it is the line that was picked, and it ends one straight
    // line, not a stair of the pieces near enough - and first.
    let mut found: Vec<(u32, [u32; 2], bool, f32)> = Vec::new();
    let mut whole: std::collections::HashSet<u32> = std::collections::HashSet::new();
    let mut by_ends: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for &e in edges {
        if let Some(edge) = layout.edges.get(e as usize) {
            let [a, b] = edge.ends;
            by_ends.entry((a.min(b), a.max(b))).or_default().push(e);
        }
    }
    for chain in edge_chains(layout, edges).into_iter().filter(|c| c.path && c.vertices.len() >= 3) {
        let (first, last) = (chain.vertices[0], chain.vertices[chain.vertices.len() - 1]);
        let d = layout.uv_vertices[last as usize].uv - layout.uv_vertices[first as usize].uv;
        if d.length() <= 0.0 {
            continue;
        }
        let angle = angle_of(d);
        let is_across = angle <= tolerance;
        if !is_across && angle < std::f32::consts::FRAC_PI_2 - tolerance {
            continue;
        }
        for pair in chain.vertices.windows(2) {
            for &e in by_ends.get(&(pair[0].min(pair[1]), pair[0].max(pair[1]))).into_iter().flatten() {
                if whole.insert(e) {
                    found.push((e, [pair[0], pair[1]], is_across, -1.0));
                }
            }
        }
    }
    for &e in edges {
        let Some(edge) = layout.edges.get(e as usize) else { continue };
        let [a, b] = edge.ends;
        let d = layout.uv_vertices[b as usize].uv - layout.uv_vertices[a as usize].uv;
        if whole.contains(&e) || d.length() <= 0.0 || is_diagonal(layout, mesh, e) {
            continue;
        }
        let angle = angle_of(d);
        if angle <= tolerance {
            found.push((e, [a, b], true, angle));
        } else if angle >= std::f32::consts::FRAC_PI_2 - tolerance {
            found.push((e, [a, b], false, std::f32::consts::FRAC_PI_2 - angle));
        }
    }
    // Taken nearest square first, each joining the vertices at its ends in
    // a run that takes one v (across) or one u (down) - unless that would
    // put two vertices on one place, both their u and their v the same (a
    // run across holding two vertices a run down holds too), or a face's
    // three corners on one line (all three in one run): a sliver of a CAD
    // tessellation whose two long sides are both near square. Such an edge
    // is left, and only it, where a whole run used to be.
    found.sort_by(|p, q| p.3.total_cmp(&q.3));
    let mut across = super::layout::UnionFind::new(layout.uv_vertices.len());
    let mut down = super::layout::UnionFind::new(layout.uv_vertices.len());
    let mut across_members: HashMap<usize, Vec<u32>> = HashMap::new();
    let mut down_members: HashMap<usize, Vec<u32>> = HashMap::new();
    let (mut flat, mut upright) = (Vec::new(), Vec::new());
    let mut faces_round: HashMap<u32, Vec<u32>> = HashMap::new();
    for (c, &v) in layout.corner_vertex.iter().enumerate() {
        faces_round.entry(v).or_default().push((c / 3) as u32);
    }
    // Whether runs `ra` and `rb` of `own` can join: none of their vertices
    // share a run of `other`, and no face would have its three corners in
    // the one run.
    fn joins(own: &mut super::layout::UnionFind, members: &mut HashMap<usize, Vec<u32>>, other: &mut super::layout::UnionFind, layout: &UvLayout, faces_round: &HashMap<u32, Vec<u32>>, a: u32, b: u32) -> bool {
        let (ra, rb) = (own.find(a as usize), own.find(b as usize));
        if ra == rb {
            return true;
        }
        let ma = members.remove(&ra).unwrap_or_else(|| vec![a]);
        let mb = members.remove(&rb).unwrap_or_else(|| vec![b]);
        let (small, large) = if ma.len() <= mb.len() { (&ma, &mb) } else { (&mb, &ma) };
        let roots: std::collections::HashSet<usize> = small.iter().map(|&v| other.find(v as usize)).collect();
        let mut clash = large.iter().any(|&v| roots.contains(&other.find(v as usize)));
        if !clash {
            'faces: for &v in small.iter() {
                for &t in faces_round.get(&v).into_iter().flatten() {
                    let corners = [0, 1, 2].map(|k| own.find(layout.corner_vertex[t as usize * 3 + k] as usize));
                    if corners.iter().all(|&r| r == ra || r == rb) {
                        clash = true;
                        break 'faces;
                    }
                }
            }
        }
        if clash {
            members.insert(ra, ma);
            members.insert(rb, mb);
            return false;
        }
        own.union(ra, rb);
        let root = own.find(ra);
        let mut all = ma;
        all.extend(mb);
        members.insert(root, all);
        true
    }
    for &(e, [a, b], is_across, _) in &found {
        if is_across {
            report.across.push(e);
            if joins(&mut across, &mut across_members, &mut down, layout, &faces_round, a, b) {
                flat.push([a, b]);
            } else {
                report.clashing.push(e);
            }
        } else {
            report.down.push(e);
            if joins(&mut down, &mut down_members, &mut across, layout, &faces_round, a, b) {
                upright.push([a, b]);
            } else {
                report.clashing.push(e);
            }
        }
    }
    // Each run of joined vertices one way.
    let runs_of = |sets: &mut super::layout::UnionFind, own: &[[u32; 2]]| -> Vec<Vec<u32>> {
        let mut runs: HashMap<usize, Vec<u32>> = HashMap::new();
        for [a, b] in own {
            for v in [*a, *b] {
                let list = runs.entry(sets.find(v as usize)).or_default();
                if !list.contains(&v) {
                    list.push(v);
                }
            }
        }
        let mut roots: Vec<usize> = runs.keys().copied().collect();
        roots.sort_unstable();
        roots.into_iter().map(|r| runs.remove(&r).expect("listed")).collect()
    };
    let flat_runs = runs_of(&mut across, &flat);
    let upright_runs = runs_of(&mut down, &upright);
    let place = |current: &HashMap<u32, Vec2>, v: u32| *current.get(&v).unwrap_or(&layout.uv_vertices[v as usize].uv);
    // Runs moved to their averages, across ones first.
    let moved = |current: &HashMap<u32, Vec2>, runs: &[&Vec<u32>], is_across: bool| -> HashMap<u32, Vec2> {
        let mut proposed = current.clone();
        for run in runs {
            let mean = run.iter().map(|&v| if is_across { place(current, v).y } else { place(current, v).x }).sum::<f32>() / run.len() as f32;
            for &v in run.iter() {
                let p = place(&proposed, v);
                proposed.insert(v, if is_across { vec2(p.x, mean) } else { vec2(mean, p.y) });
            }
        }
        proposed
    };
    // Whether a face still faces its island's way and keeps some area: one
    // that was flat or folded already is not counted against a move.
    let keeps = |current: &HashMap<u32, Vec2>, proposed: &HashMap<u32, Vec2>, t: u32| {
        let island = &layout.islands[layout.island_of[t as usize] as usize];
        let sign = if island.mirrored { -1.0 } else { 1.0 };
        let hair = island.uv_area * super::flatten::FOLD_HAIR;
        let corners = |positions: &HashMap<u32, Vec2>| [0, 1, 2].map(|k| place(positions, layout.corner_vertex[t as usize * 3 + k]));
        let [a, b, c] = corners(current);
        let [p, q, r] = corners(proposed);
        super::flatten::facing(a, b, c) * sign <= hair || super::flatten::facing(p, q, r) * sign > hair
    };
    let all_runs: Vec<(&Vec<u32>, bool)> = flat_runs.iter().map(|r| (r, true)).chain(upright_runs.iter().map(|r| (r, false))).collect();
    let none = HashMap::new();
    let across_runs: Vec<&Vec<u32>> = flat_runs.iter().collect();
    let down_runs: Vec<&Vec<u32>> = upright_runs.iter().collect();
    let proposed = moved(&moved(&none, &across_runs, true), &down_runs, false);
    // The islands where moving only the runs' vertices folds a face.
    let mut faces_of: HashMap<u32, Vec<u32>> = HashMap::new();
    for (c, &v) in layout.corner_vertex.iter().enumerate() {
        if proposed.contains_key(&v) {
            faces_of.entry(v).or_default().push((c / 3) as u32);
        }
    }
    let mut folding: Vec<u32> = proposed
        .keys()
        .flat_map(|v| faces_of.get(v).into_iter().flatten())
        .filter(|&&t| !keeps(&none, &proposed, t))
        .map(|&t| layout.island_of[t as usize])
        .collect();
    folding.sort_unstable();
    folding.dedup();
    let mut out = Places::default();
    for (&v, &p) in &proposed {
        if !folding.contains(&layout.uv_vertices[v as usize].island) {
            out.push(v, p);
        }
    }
    for island in folding {
        // The island's other vertices carried along by the runs' moves.
        let patch = super::shape::Patch::new(mesh, layout, &layout.islands[island as usize].triangles);
        let held: Vec<(usize, Vec2)> = patch.vertices.iter().enumerate().filter_map(|(i, v)| proposed.get(v).map(|p| (i, *p))).collect();
        // Judged as the runs alone are: a face that was flat or folded
        // already, as a tessellation's slivers can be, does not count
        // against it.
        let carried = super::shape::carried(&patch, &held, &patch.weights()).filter(|uv| {
            let moved: HashMap<u32, Vec2> = patch.vertices.iter().copied().zip(uv.iter().copied()).collect();
            layout.islands[island as usize].triangles.iter().all(|&t| keeps(&none, &moved, t))
        });
        if let Some(uv) = carried {
            for (&v, p) in patch.vertices.iter().zip(uv) {
                if !proposed.contains_key(&v) && (p - layout.uv_vertices[v as usize].uv).length() > 1e-9 {
                    report.carried += 1;
                }
                out.push(v, p);
            }
            continue;
        }
        // Else run by run, each way in turn, leaving those that fold.
        let mut current: HashMap<u32, Vec2> = HashMap::new();
        for is_across in [true, false] {
            for (run, _) in all_runs.iter().filter(|(r, a)| *a == is_across && layout.uv_vertices[r[0] as usize].island == island) {
                let attempt = moved(&current, &[run], is_across);
                let fine = run.iter().flat_map(|v| faces_of.get(v).into_iter().flatten()).all(|&t| keeps(&current, &attempt, t));
                if fine {
                    current = attempt;
                }
            }
        }
        for (v, p) in current {
            out.push(v, p);
        }
    }
    let out = out.moving(layout);
    // The edges found that are not exactly across or down after all.
    let after = |v: u32| out.vertices.iter().position(|&m| m == v).map(|i| vec2(out.uv[i][0], out.uv[i][1])).unwrap_or(layout.uv_vertices[v as usize].uv);
    for (&e, is_across) in report.across.iter().map(|e| (e, true)).chain(report.down.iter().map(|e| (e, false))) {
        let [a, b] = layout.edges[e as usize].ends;
        let d = after(b) - after(a);
        if (if is_across { d.y } else { d.x }).abs() > 1e-6 * d.length().max(1e-9) {
            report.left.push(e);
        }
    }
    (out, report)
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

    /// A sliver, as CAD tessellation makes them, its two long sides both
    /// near down and its short side near across: lining all three up would
    /// put its corners on one line, so the second long side is left - and
    /// only it, where the whole run down used to go - and said.
    #[test]
    fn a_sliver_keeps_its_area_and_only_one_side_is_left() {
        use tequila_core::math::vec3;
        let positions = vec![vec3(0.0, 0.0, 0.0), vec3(0.02, 0.0, 0.0), vec3(0.01, 0.4, 0.0)];
        let mesh = Mesh::new(positions, vec![0, 1, 2]).with_uvs(vec![vec2(0.50, 0.20), vec2(0.52, 0.205), vec2(0.511, 0.60)]);
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let all: Vec<u32> = (0..layout.edges.len() as u32).collect();
        let (places, report) = square_edges_reported(&layout, &mesh, &all, 15f32.to_radians());
        assert_eq!(report.across.len() + report.down.len(), 3, "all three found");
        assert_eq!(report.left.len(), 1, "{report:?}");
        assert!(report.clashing.contains(&report.left[0]));
        let out = crate::uv::set_positions_from_lists(&mesh, 0, &places.vertices, &places.uv).expect("no fold");
        assert!(UvLayout::new(&out, 0).unwrap().folded_triangles().is_empty());
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
        let out = crate::uv::set_positions_from_lists(&mesh, 0, &places.vertices, &places.uv).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        for &v in &top {
            assert!((after.uv_vertices[v as usize].uv.y - lowest).abs() < 1e-6);
        }
        let middle = line_up(&layout, &[top.clone()], LineUp { across: false, side: 0, against: Some((0.0, 1.0)) });
        assert!(middle.uv.iter().all(|p| (p[1] - 0.5).abs() < 1e-6));
        let onto = onto_line(&layout, &[top.clone()]);
        let out = crate::uv::set_positions_from_lists(&mesh, 0, &onto.vertices, &onto.uv).unwrap();
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
        let out = crate::uv::set_positions_from_lists(&mesh, 0, &even.vertices, &even.uv).unwrap();
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

    /// A strip of narrow rows turned by 8°: a row squared alone would cross
    /// the next, still turned, so all of them go at once.
    #[test]
    fn a_turned_strip_is_squared_whole() {
        let strip = sheet(4, 16, |_, _| 0.0);
        let turn = 8f32.to_radians();
        let uv = strip.positions.iter().map(|p| {
            let (x, y) = (p.x * 0.8, p.z * 0.08);
            vec2(0.1 + x * turn.cos() - y * turn.sin(), 0.4 + x * turn.sin() + y * turn.cos())
        }).collect();
        let mesh = strip.with_uvs(uv);
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let all: Vec<u32> = (0..layout.edges.len() as u32).collect();
        let places = square_edges(&layout, &mesh, &all, 15f32.to_radians());
        let out = crate::uv::set_positions_from_lists(&mesh, 0, &places.vertices, &places.uv).expect("no fold");
        let after = UvLayout::new(&out, 0).unwrap();
        for edge in &after.edges {
            let [a, b] = edge.vertices.map(|v| mesh.positions[v as usize]);
            let [p, q] = edge.ends.map(|v| after.uv_vertices[v as usize].uv);
            if (a.z - b.z).abs() < 1e-6 {
                assert!((p.y - q.y).abs() < 1e-5, "rows across: {p:?} {q:?}");
            } else if (a.x - b.x).abs() < 1e-6 {
                assert!((p.x - q.x).abs() < 1e-5, "rungs down: {p:?} {q:?}");
            }
        }
    }

    #[test]
    fn a_crooked_strip_is_squared_but_its_diagonals_are_left() {
        let mesh = crooked_strip();
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let all: Vec<u32> = (0..layout.edges.len() as u32).collect();
        let places = square_edges(&layout, &mesh, &all, 15f32.to_radians());
        let out = crate::uv::set_positions_from_lists(&mesh, 0, &places.vertices, &places.uv).expect("no fold");
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
