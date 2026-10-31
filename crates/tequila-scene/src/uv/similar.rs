//! Islands of the same shape: a part's repeated pieces - the two shells of
//! a pair of headphones, a strap's links, a mirrored half - found, matched
//! corner for corner, and laid one over another.
//!
//! Two islands are the same piece when their triangles join up the same
//! way: a match is grown out from one triangle of each, across edge after
//! edge, each step naming the corner the next triangle adds, until every
//! triangle of the one is paired with one of the other or a pair of edges
//! disagrees (one on the island's edge, one not; a corner already paired
//! with another). The seed of the first island is one whose corners are
//! rarest in the second; every triangle of the second like it is tried,
//! at each turn that fits, and - for a mirrored piece, whose triangles run
//! the other way round - with the second's windings reversed. A match is
//! the same piece; whether it is the same shape is a matter of tolerance:
//! its area on the part, and its outline on the layout once turned, scaled
//! and moved onto the other's as well as they go.
//!
//! An [`IslandShape`] stands on its own, so a shape copied from one part
//! can be pasted on another's islands.

use super::layout::UvLayout;
use crate::geometry::Mesh;
use std::collections::HashMap;
use tequila_core::math::{Vec2, Vec3};

/// One island on its own: its triangles over vertices of its own, each
/// with its place on the layout and on the part.
#[derive(Clone, Debug, PartialEq)]
pub struct IslandShape {
    /// Corners, by the island's own vertex numbers, as the layout winds them.
    pub triangles: Vec<[u32; 3]>,
    pub uv: Vec<Vec2>,
    pub position: Vec<Vec3>,
    /// The layout's UV vertex each of its vertices is.
    pub vertices: Vec<u32>,
}

impl IslandShape {
    pub fn of(layout: &UvLayout, mesh: &Mesh, island: u32) -> IslandShape {
        let island = &layout.islands[island as usize];
        let mut local: HashMap<u32, u32> = HashMap::with_capacity(island.uv_vertices.len());
        for (i, &v) in island.uv_vertices.iter().enumerate() {
            local.insert(v, i as u32);
        }
        let triangles = island.triangles.iter().map(|&t| [0, 1, 2].map(|k| local[&layout.corner_vertex[t as usize * 3 + k]])).collect();
        let uv = island.uv_vertices.iter().map(|&v| layout.uv_vertices[v as usize].uv).collect();
        let position = island.uv_vertices.iter().map(|&v| mesh.positions[layout.uv_vertices[v as usize].vertex as usize]).collect();
        IslandShape { triangles, uv, position, vertices: island.uv_vertices.clone() }
    }

    pub fn surface_area(&self) -> f32 {
        self.triangles.iter().map(|t| super::flatten::surface_area(self.position[t[0] as usize], self.position[t[1] as usize], self.position[t[2] as usize])).sum()
    }

    pub fn uv_area(&self) -> f32 {
        self.triangles.iter().map(|t| super::flatten::facing(self.uv[t[0] as usize], self.uv[t[1] as usize], self.uv[t[2] as usize]).abs()).sum()
    }

    /// The middle of the box round it on the layout.
    pub fn centre(&self) -> Vec2 {
        super::transform::middle(self.uv.iter().copied())
    }

    /// What two of the same piece have alike, cheap to compare first: how
    /// many triangles, vertices and edges.
    pub fn counts(&self) -> (usize, usize, usize) {
        let mut edges: Vec<(u32, u32)> = self.triangles.iter().flat_map(|t| [0, 1, 2].map(|k| (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3])))).collect();
        edges.sort_unstable();
        edges.dedup();
        (self.triangles.len(), self.uv.len(), edges.len())
    }
}

/// How one island's vertices pair with another's.
#[derive(Clone, Debug, PartialEq)]
pub struct Match {
    /// For each vertex of the first, the second's vertex it pairs with.
    pub pairs: Vec<u32>,
    /// The second runs the other way round: a mirrored piece.
    pub mirrored: bool,
}

/// How much matching may cost before it gives up: steps across an edge, in
/// all. A match that is there is found in a few times the island's size;
/// this is for two large islands that are not.
const MOST_STEPS: usize = 40_000_000;

/// An island's triangles as the matching walks them.
struct Walk<'a> {
    triangles: &'a [[u32; 3]],
    /// The triangle and corner each edge, from its corner to the next,
    /// starts at.
    half: HashMap<(u32, u32), (u32, u8)>,
    /// Each corner's kind: how many vertices it joins, and whether it is on
    /// the island's edge.
    kind: Vec<u32>,
}

impl<'a> Walk<'a> {
    fn new(triangles: &'a [[u32; 3]], vertices: usize) -> Walk<'a> {
        let mut half = HashMap::with_capacity(triangles.len() * 3);
        for (t, tri) in triangles.iter().enumerate() {
            for k in 0..3 {
                half.entry((tri[k], tri[(k + 1) % 3])).or_insert((t as u32, k as u8));
            }
        }
        let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); vertices];
        let mut edge = vec![false; vertices];
        for tri in triangles {
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                neighbours[a as usize].push(b);
                neighbours[b as usize].push(a);
                if !half.contains_key(&(b, a)) {
                    edge[a as usize] = true;
                    edge[b as usize] = true;
                }
            }
        }
        let kind = neighbours
            .iter_mut()
            .zip(&edge)
            .map(|(list, &on_edge)| {
                list.sort_unstable();
                list.dedup();
                list.len() as u32 * 2 + on_edge as u32
            })
            .collect();
        Walk { triangles, half, kind }
    }

    fn corners(&self, t: u32) -> [u32; 3] {
        self.triangles[t as usize].map(|v| self.kind[v as usize])
    }
}

/// The kinds of a triangle's corners turned so the least comes first: the
/// same for a triangle however it is turned.
fn canonical(k: [u32; 3]) -> [u32; 3] {
    [k, [k[1], k[2], k[0]], [k[2], k[0], k[1]]].into_iter().min().expect("three")
}

/// How the vertices of `a` pair with those of `b` where the two are the
/// same piece: triangle for triangle, edge for edge. With `mirrored`, a
/// piece running the other way round matches too. None where they are not
/// the same piece, or it would take too long to tell.
pub fn correspond(a: &IslandShape, b: &IslandShape, mirrored: bool) -> Option<Match> {
    if a.counts() != b.counts() || a.triangles.is_empty() {
        return None;
    }
    let mut steps = 0usize;
    if let Some(pairs) = correspond_wound(a, &b.triangles, b.uv.len(), &mut steps) {
        return Some(Match { pairs, mirrored: false });
    }
    if mirrored {
        let reversed: Vec<[u32; 3]> = b.triangles.iter().map(|t| [t[0], t[2], t[1]]).collect();
        if let Some(pairs) = correspond_wound(a, &reversed, b.uv.len(), &mut steps) {
            return Some(Match { pairs, mirrored: true });
        }
    }
    None
}

fn correspond_wound(a: &IslandShape, b_triangles: &[[u32; 3]], b_vertices: usize, steps: &mut usize) -> Option<Vec<u32>> {
    let wa = Walk::new(&a.triangles, a.uv.len());
    let wb = Walk::new(b_triangles, b_vertices);
    // The second's triangles by the kinds of their corners.
    let mut by_kind: HashMap<[u32; 3], Vec<u32>> = HashMap::new();
    for t in 0..b_triangles.len() as u32 {
        by_kind.entry(canonical(wb.corners(t))).or_default().push(t);
    }
    // The first's seed: the triangle whose like is rarest in the second.
    let mut seed: Option<(u32, usize)> = None;
    for t in 0..a.triangles.len() as u32 {
        let count = by_kind.get(&canonical(wa.corners(t))).map_or(0, |l| l.len());
        if count == 0 {
            return None;
        }
        if seed.is_none_or(|(_, best)| count < best) {
            seed = Some((t, count));
        }
    }
    let (seed, _) = seed?;
    let want = wa.corners(seed);
    let candidates = by_kind.get(&canonical(want))?;
    let mut pairs = vec![u32::MAX; a.uv.len()];
    let mut taken = vec![false; b_vertices];
    let mut paired_triangle = vec![u32::MAX; a.triangles.len()];
    let mut queue: Vec<(u32, u32)> = Vec::new();
    for &tb in candidates {
        for turn in 0..3 {
            let corners = wb.corners(tb);
            if [0, 1, 2].map(|k| corners[(k + turn) % 3]) != want {
                continue;
            }
            pairs.iter_mut().for_each(|p| *p = u32::MAX);
            taken.iter_mut().for_each(|t| *t = false);
            paired_triangle.iter_mut().for_each(|t| *t = u32::MAX);
            queue.clear();
            let ta = &a.triangles[seed as usize];
            let tbv = &b_triangles[tb as usize];
            for k in 0..3 {
                pairs[ta[k] as usize] = tbv[(k + turn) % 3];
                taken[tbv[(k + turn) % 3] as usize] = true;
            }
            paired_triangle[seed as usize] = tb;
            queue.push((seed, tb));
            if grow(&wa, &wb, &mut pairs, &mut taken, &mut paired_triangle, &mut queue, steps) {
                return Some(pairs);
            }
            if *steps > MOST_STEPS {
                return None;
            }
        }
    }
    None
}

/// The match grown out from the triangles in `queue`, edge by edge. True
/// when every triangle and vertex of the first is paired, and no edge
/// disagrees on the way.
fn grow(wa: &Walk, wb: &Walk, pairs: &mut [u32], taken: &mut [bool], paired_triangle: &mut [u32], queue: &mut Vec<(u32, u32)>, steps: &mut usize) -> bool {
    while let Some((ta, tb)) = queue.pop() {
        let corners = wa.triangles[ta as usize];
        for k in 0..3 {
            *steps += 1;
            let (x, y) = (corners[k], corners[(k + 1) % 3]);
            let (fx, fy) = (pairs[x as usize], pairs[y as usize]);
            let across_a = wa.half.get(&(y, x));
            let across_b = wb.half.get(&(fy, fx));
            match (across_a, across_b) {
                (None, None) => {}
                (Some(&(na, ka)), Some(&(nb, kb))) => {
                    if paired_triangle[na as usize] != u32::MAX {
                        if paired_triangle[na as usize] != nb {
                            return false;
                        }
                        continue;
                    }
                    let za = wa.triangles[na as usize][(ka as usize + 2) % 3];
                    let zb = wb.triangles[nb as usize][(kb as usize + 2) % 3];
                    match pairs[za as usize] {
                        u32::MAX => {
                            if taken[zb as usize] {
                                return false;
                            }
                            pairs[za as usize] = zb;
                            taken[zb as usize] = true;
                        }
                        already if already != zb => return false,
                        _ => {}
                    }
                    paired_triangle[na as usize] = nb;
                    queue.push((na, nb));
                }
                _ => return false,
            }
        }
        let _ = tb;
    }
    paired_triangle.iter().all(|&t| t != u32::MAX) && pairs.iter().all(|&p| p != u32::MAX)
}

/// How close two matched islands are in shape: the difference of their
/// areas on the part, as a share of the first's, and how far the second's
/// outline on the layout is from the first's once turned, scaled, moved and
/// mirrored onto it as well as it goes, as a share of the first's size.
pub fn difference(a: &IslandShape, b: &IslandShape, m: &Match) -> f32 {
    let (sa, sb) = (a.surface_area(), b.surface_area());
    let area = if sa > 0.0 { (sa - sb).abs() / sa } else { 0.0 };
    let theirs: Vec<Vec2> = m.pairs.iter().map(|&j| b.uv[j as usize]).collect();
    let size = a.uv_area().sqrt();
    let outline = match (likeness(&theirs, &a.uv), size > 0.0) {
        (Some(fit), true) => {
            let squared: f32 = theirs.iter().zip(&a.uv).map(|(p, q)| (fit.apply(*p) - *q).length_squared()).sum();
            (squared / a.uv.len() as f32).sqrt() / size
        }
        _ => 0.0,
    };
    area.max(outline)
}

/// Whether `b` is the same piece as `a` and the same shape within
/// `tolerance` (a share: 0.05 is five per cent); the match if so.
pub fn similar(a: &IslandShape, b: &IslandShape, tolerance: f32, mirrored: bool) -> Option<Match> {
    let m = correspond(a, b, mirrored)?;
    (difference(a, b, &m) <= tolerance).then_some(m)
}

/// A turn, a scale, a move and maybe a mirror taking one set of places onto
/// another, point for point, as well as they go (Umeyama's method, in two
/// dimensions): `p` goes to `to + s R (M (p - from))`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Likeness {
    pub from: Vec2,
    pub to: Vec2,
    pub cos: f32,
    pub sin: f32,
    pub mirror: bool,
}

impl Likeness {
    pub fn apply(&self, p: Vec2) -> Vec2 {
        let mut d = p - self.from;
        if self.mirror {
            d.x = -d.x;
        }
        self.to + tequila_core::math::vec2(d.x * self.cos - d.y * self.sin, d.x * self.sin + d.y * self.cos)
    }
}

pub fn likeness(before: &[Vec2], after: &[Vec2]) -> Option<Likeness> {
    let n = before.len().min(after.len());
    if n < 2 {
        return None;
    }
    let mean = |p: &[Vec2]| p[..n].iter().fold(Vec2::ZERO, |s, q| s + *q) / n as f32;
    let (from, to) = (mean(before), mean(after));
    let (mut spread, mut dot, mut cross, mut dot_m, mut cross_m) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for (p, q) in before[..n].iter().zip(&after[..n]) {
        let (p, q) = (*p - from, *q - to);
        spread += (p.x * p.x + p.y * p.y) as f64;
        dot += (p.x * q.x + p.y * q.y) as f64;
        cross += (p.x * q.y - p.y * q.x) as f64;
        dot_m += (-p.x * q.x + p.y * q.y) as f64;
        cross_m += (-p.x * q.y - p.y * q.x) as f64;
    }
    if spread <= 1e-30 {
        return None;
    }
    let (plain, mirrored) = ((dot * dot + cross * cross).sqrt(), (dot_m * dot_m + cross_m * cross_m).sqrt());
    let mirror = mirrored > plain * (1.0 + 1e-6);
    let (d, c, strength) = if mirror { (dot_m, cross_m, mirrored) } else { (dot, cross, plain) };
    let scale = strength / spread;
    let angle = c.atan2(d);
    Some(Likeness { from, to, cos: (angle.cos() * scale) as f32, sin: (angle.sin() * scale) as f32, mirror })
}

/// Islands sorted into pieces of the same shape: each list of islands the
/// same piece and shape as its first, within `tolerance`. Islands are
/// compared with each class's first, so a piece is never matched twice.
pub fn classes(shapes: &[(u32, IslandShape)], tolerance: f32, mirrored: bool) -> Vec<Vec<u32>> {
    let mut out: Vec<(usize, Vec<u32>)> = Vec::new();
    for (k, (island, shape)) in shapes.iter().enumerate() {
        let found = out.iter_mut().find(|(first, _)| {
            let first = &shapes[*first].1;
            first.counts() == shape.counts() && similar(first, shape, tolerance, mirrored).is_some()
        });
        match found {
            Some((_, list)) => list.push(*island),
            None => out.push((k, vec![*island])),
        }
    }
    out.into_iter().map(|(_, list)| list).collect()
}

/// The places `b`'s vertices take to lie exactly over `a`'s, matched by
/// `m`, moved by `offset`: the layout's UV vertices of `b`, and where each
/// goes.
pub fn laid_over(a: &IslandShape, b: &IslandShape, m: &Match, offset: Vec2) -> (Vec<u32>, Vec<[f32; 2]>) {
    let mut vertices = Vec::with_capacity(m.pairs.len());
    let mut places = Vec::with_capacity(m.pairs.len());
    for (i, &j) in m.pairs.iter().enumerate() {
        let p = a.uv[i] + offset;
        vertices.push(b.vertices[j as usize]);
        places.push([p.x, p.y]);
    }
    (vertices, places)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{transform, Pivot, UvAffine, UvSelection};
    use tequila_core::math::{vec2, vec3};

    /// Two copies of a sheet side by side, the second mirrored on the part
    /// (its x turned back) when asked, each its own island.
    fn pair(mirror_second: bool) -> Mesh {
        let one = sheet(3, 4, |x, z| (x * 3.0).sin() * 0.1 + z * 0.05);
        let mut positions = one.positions.clone();
        let mut indices = one.indices.clone();
        // Laid out as seen from above, the second beside the first: across
        // the other way where it is mirrored, so it reads the right way.
        let mut uv: Vec<Vec2> = one.positions.iter().map(|p| vec2(0.05 + p.x * 0.4, 0.1 + p.z * 0.3)).collect();
        let base = positions.len() as u32;
        for p in &one.positions {
            positions.push(if mirror_second { vec3(3.0 - p.x, p.y, p.z) } else { vec3(p.x + 2.0, p.y, p.z) });
            uv.push(vec2(0.55 + (if mirror_second { 1.0 - p.x } else { p.x }) * 0.4, 0.1 + p.z * 0.3));
        }
        for t in one.indices.chunks(3) {
            if mirror_second {
                indices.extend_from_slice(&[t[0] + base, t[2] + base, t[1] + base]);
            } else {
                indices.extend_from_slice(&[t[0] + base, t[1] + base, t[2] + base]);
            }
        }
        Mesh::new(positions, indices).with_uvs(uv)
    }

    fn shapes(mesh: &Mesh) -> (UvLayout, Vec<IslandShape>) {
        let layout = UvLayout::new(mesh, 0).unwrap();
        let shapes = (0..layout.islands.len() as u32).map(|i| IslandShape::of(&layout, mesh, i)).collect();
        (layout, shapes)
    }

    #[test]
    fn two_copies_of_a_piece_match_vertex_for_vertex() {
        let mesh = pair(false);
        let (layout, s) = shapes(&mesh);
        assert_eq!(layout.islands.len(), 2);
        let m = correspond(&s[0], &s[1], false).expect("the same piece");
        assert!(!m.mirrored);
        // Paired vertices are the same point of the piece: two apart in x.
        for (i, &j) in m.pairs.iter().enumerate() {
            let (p, q) = (s[0].position[i], s[1].position[j as usize]);
            assert!((q - p - vec3(2.0, 0.0, 0.0)).length() < 1e-4, "{p:?} {q:?}");
        }
        assert!(difference(&s[0], &s[1], &m) < 1e-3);
    }

    #[test]
    fn a_mirrored_piece_matches_only_when_mirrors_are_allowed() {
        let mesh = pair(true);
        let (_, s) = shapes(&mesh);
        assert!(correspond(&s[0], &s[1], false).is_none(), "it runs the other way round");
        let m = correspond(&s[0], &s[1], true).expect("mirrored, the same piece");
        assert!(m.mirrored);
        for (i, &j) in m.pairs.iter().enumerate() {
            let (p, q) = (s[0].position[i], s[1].position[j as usize]);
            assert!((q.x - (3.0 - p.x)).abs() < 1e-4 && (q.y - p.y).abs() < 1e-4 && (q.z - p.z).abs() < 1e-4);
        }
        assert!(similar(&s[0], &s[1], 0.02, true).is_some());
    }

    #[test]
    fn a_different_piece_or_shape_does_not_match() {
        let mesh = pair(false);
        let (_, s) = shapes(&mesh);
        // The same sheet with one corner's diagonal the other way: as many
        // triangles, vertices and edges, joined otherwise.
        let mut flipped = sheet(3, 4, |_, _| 0.0);
        let at = |r: u32, c: u32| r * 5 + c;
        flipped.indices[..6].copy_from_slice(&[at(0, 0), at(1, 0), at(0, 1), at(1, 0), at(1, 1), at(0, 1)]);
        let flat = flipped.positions.iter().map(|p| vec2(p.x * 0.4, p.z * 0.3)).collect();
        let other = shapes(&flipped.with_uvs(flat)).1.remove(0);
        assert_eq!(other.counts(), s[0].counts());
        assert!(correspond(&s[0], &other, true).is_none(), "the same counts, joined otherwise");
        // The same piece stretched one way on the layout: not the same shape.
        let stretched = transform(&mesh, 0, &UvSelection::Islands(vec![1]), &UvAffine::scale(vec2(1.6, 1.0)), Pivot::Islands).unwrap();
        let (_, t) = shapes(&stretched);
        assert!(correspond(&t[0], &t[1], false).is_some());
        assert!(similar(&t[0], &t[1], 0.05, false).is_none());
        // Turned and scaled as a whole, it is.
        let turned = transform(&mesh, 0, &UvSelection::Islands(vec![1]), &UvAffine::rotation(0.8).then(&UvAffine::uniform(0.5)), Pivot::Islands).unwrap();
        let (_, t) = shapes(&turned);
        assert!(similar(&t[0], &t[1], 0.05, false).is_some());
    }

    #[test]
    fn a_piece_laid_over_another_takes_its_shape_exactly() {
        let mesh = pair(false);
        let turned = transform(&mesh, 0, &UvSelection::Islands(vec![1]), &UvAffine::rotation(0.5), Pivot::Islands).unwrap();
        let (_, s) = shapes(&turned);
        let m = correspond(&s[0], &s[1], false).unwrap();
        let (vertices, places) = laid_over(&s[0], &s[1], &m, vec2(0.0, 0.0));
        let out = crate::uv::set_positions(&turned, 0, &vertices, &places).unwrap();
        let (_, after) = shapes(&out);
        for (i, &j) in m.pairs.iter().enumerate() {
            assert!((after[1].uv[j as usize] - after[0].uv[i]).length() < 1e-6);
        }
    }

    #[test]
    fn pieces_sort_into_classes() {
        let mesh = pair(true);
        let (_, s) = shapes(&mesh);
        let all: Vec<(u32, IslandShape)> = s.into_iter().enumerate().map(|(i, s)| (i as u32, s)).collect();
        assert_eq!(classes(&all, 0.05, true), vec![vec![0, 1]]);
        assert_eq!(classes(&all, 0.05, false), vec![vec![0], vec![1]]);
    }
}
