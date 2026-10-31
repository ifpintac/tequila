//! Grid: a patch of four-sided faces laid out as a rectangular grid, its
//! rows and columns straight across and down the layout.
//!
//! A mesh here is triangles. Its four-sided faces are found again as pairs
//! of triangles across an edge that is the longest of both, where the two
//! make a quad that is near enough flat and turns the same way at all four
//! corners - which is how a quad is split, along a diagonal. Then the quads
//! are walked from one to the next across their shared sides, each taking
//! the next place in its row or column, and a quad that would put a vertex
//! where another already is, or two vertices in one place, means the faces
//! are not in rows and columns.
//!
//! Each column is as wide as its edges are long on the part, on average,
//! and each row as high - or all of them alike, evenly spaced - at the
//! island's density, so the grid is the patch's own size; turned square to
//! the layout the way that lies best over where the patch was, and put
//! there. A grid of part of an island has the rest laid out again round it.

use super::shape::{best_fit, box_middle, cannot, island_round, layout, regions, turned, Moves, Pick, ShapeError};
use super::flatten::facing;
use super::layout::UvLayout;
use crate::geometry::Mesh;
use std::collections::{HashMap, VecDeque};
use tequila_core::math::{vec2, Vec2};

/// How a grid is laid out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GridOptions {
    /// Every column as wide as the others, every row as high: the average.
    pub even: bool,
    /// Each grid scaled to fill the layout's square, across and down.
    pub fit_square: bool,
}

/// What laying out as a grid did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GridReport {
    pub grids: usize,
    /// The largest grid's columns and rows of faces.
    pub columns: usize,
    pub rows: usize,
}

/// The faces `pick` names - picked on the part, or whole islands - laid
/// out as grids.
pub fn grid(mesh: &Mesh, set: usize, pick: &Pick, options: &GridOptions) -> Result<(Moves, GridReport), ShapeError> {
    let layout = layout(mesh, set)?;
    let mut moves = Moves::new();
    let mut report = GridReport::default();
    for region in regions(&layout, pick)? {
        let (quads, odd) = quads(mesh, &layout, &region.triangles);
        if odd > 0 {
            return cannot(if odd == 1 { "1 face isn't four-sided".to_string() } else { format!("{odd} faces aren't four-sided") });
        }
        let density = layout.islands[region.island as usize].density();
        if density <= 0.0 {
            return cannot(format!("island {} has no area to lay out", region.island + 1));
        }
        let mut placed: Vec<(u32, Vec2)> = Vec::new();
        for piece in walk(&quads)? {
            let (columns, rows) = (piece.values().map(|c| c.0).max().unwrap_or(0) as usize, piece.values().map(|c| c.1).max().unwrap_or(0) as usize);
            if columns * rows > report.columns * report.rows {
                (report.columns, report.rows) = (columns, rows);
            }
            report.grids += 1;
            placed.extend(lay_out(mesh, &layout, &region.triangles, &piece, density.sqrt(), options));
        }
        if region.whole {
            moves.extend(placed);
        } else {
            moves.extend(island_round(mesh, &layout, region.island, &placed, &[], "round the grid")?);
        }
    }
    Ok((moves, report))
}

/// The quads among some triangles, each as its four UV vertices in the
/// order the triangles run; and how many triangles are left over.
pub(crate) fn quads(mesh: &Mesh, layout: &UvLayout, triangles: &[u32]) -> (Vec<[u32; 4]>, usize) {
    let cv = |t: u32, k: usize| layout.corner_vertex[t as usize * 3 + k];
    let corner_point = |t: u32, k: usize| mesh.positions[mesh.indices[t as usize * 3 + k] as usize];
    let longest = |t: u32| (0..3).max_by(|&a, &b| {
        let length = |k: usize| (corner_point(t, (k + 1) % 3) - corner_point(t, k)).length();
        length(a).total_cmp(&length(b))
    }).expect("three edges");
    let mut by_edge: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for &t in triangles {
        let k = longest(t);
        let (a, b) = (cv(t, k), cv(t, (k + 1) % 3));
        by_edge.entry((a.min(b), a.max(b))).or_default().push(t);
    }
    let mut paired: HashMap<u32, bool> = HashMap::new();
    let mut out = Vec::new();
    for &t in triangles {
        if paired.contains_key(&t) {
            continue;
        }
        let k = longest(t);
        let (a, b) = (cv(t, k), cv(t, (k + 1) % 3));
        let Some(other) = by_edge.get(&(a.min(b), a.max(b))).and_then(|list| list.iter().copied().find(|&n| n != t && !paired.contains_key(&n))) else { continue };
        // The other's corner off the shared edge.
        let Some(q) = (0..3).find(|&j| cv(other, j) != a && cv(other, j) != b) else { continue };
        let quad = [a, cv(other, q), b, cv(t, (k + 2) % 3)];
        let points = [corner_point(t, k), corner_point(other, q), corner_point(t, (k + 1) % 3), corner_point(t, (k + 2) % 3)];
        if !flat_and_convex(&points) {
            continue;
        }
        paired.insert(t, true);
        paired.insert(other, true);
        out.push(quad);
    }
    let odd = triangles.len() - paired.len();
    (out, odd)
}

/// Whether four points round a quad are near enough flat - its two halves
/// facing within 30 degrees of each other - and turn the same way at every
/// corner.
fn flat_and_convex(p: &[tequila_core::math::Vec3; 4]) -> bool {
    let normal = (p[2] - p[0]).cross(p[3] - p[1]);
    if normal.length() <= 0.0 {
        return false;
    }
    let halves = [(p[1] - p[0]).cross(p[2] - p[0]), (p[2] - p[0]).cross(p[3] - p[0])];
    let (h0, h1) = (halves[0].try_normalize(), halves[1].try_normalize());
    let (Some(h0), Some(h1)) = (h0, h1) else { return false };
    if h0.dot(h1) < 30f32.to_radians().cos() {
        return false;
    }
    (0..4).all(|i| {
        let (a, b, c) = (p[i], p[(i + 1) % 4], p[(i + 2) % 4]);
        (b - a).cross(c - b).dot(normal) > 0.0
    })
}

/// Quads walked into rows and columns, a piece at a time: each UV vertex's
/// column and row, from 0. Refused where they are not in rows and columns.
fn walk(quads: &[[u32; 4]]) -> Result<Vec<HashMap<u32, (i32, i32)>>, ShapeError> {
    let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (q, quad) in quads.iter().enumerate() {
        for k in 0..4 {
            let (a, b) = (quad[k], quad[(k + 1) % 4]);
            by_edge.entry((a.min(b), a.max(b))).or_default().push(q);
        }
    }
    let not_rows = || cannot("the faces aren't in rows and columns");
    let mut seen = vec![false; quads.len()];
    let mut out = Vec::new();
    for seed in 0..quads.len() {
        if seen[seed] {
            continue;
        }
        let mut at: HashMap<u32, (i32, i32)> = HashMap::new();
        for (k, place) in [(0, 0), (1, 0), (1, 1), (0, 1)].into_iter().enumerate() {
            at.insert(quads[seed][k], place);
        }
        seen[seed] = true;
        let mut queue = VecDeque::from([seed]);
        while let Some(q) = queue.pop_front() {
            let quad = quads[q];
            for k in 0..4 {
                let (a, b) = (quad[k], quad[(k + 1) % 4]);
                let (d, c) = (quad[(k + 3) % 4], quad[(k + 2) % 4]);
                for &n in &by_edge[&(a.min(b), a.max(b))] {
                    if n == q || seen[n] {
                        continue;
                    }
                    let next = quads[n];
                    let (ia, ib) = (next.iter().position(|&v| v == a), next.iter().position(|&v| v == b));
                    let (Some(ia), Some(ib)) = (ia, ib) else { return not_rows() };
                    // Across the shared side, each of its ends one step on,
                    // the way the quad behind it came.
                    let beyond = |i: usize, other: usize| if next[(i + 1) % 4] == next[other] { next[(i + 3) % 4] } else { next[(i + 1) % 4] };
                    let (a2, b2) = (beyond(ia, ib), beyond(ib, ia));
                    let step = |from: u32, back: u32| {
                        let (f, k) = (at[&from], at[&back]);
                        (2 * f.0 - k.0, 2 * f.1 - k.1)
                    };
                    for (v, place) in [(a2, step(a, d)), (b2, step(b, c))] {
                        match at.get(&v) {
                            Some(&known) if known != place => return not_rows(),
                            _ => {
                                at.insert(v, place);
                            }
                        }
                    }
                    seen[n] = true;
                    queue.push_back(n);
                }
            }
        }
        // Two vertices in one place: the faces wrap round, or overlap.
        let mut places: Vec<(i32, i32)> = at.values().copied().collect();
        places.sort_unstable();
        if places.windows(2).any(|w| w[0] == w[1]) {
            return not_rows();
        }
        let low = (places.iter().map(|p| p.0).min().unwrap_or(0), places.iter().map(|p| p.1).min().unwrap_or(0));
        out.push(at.into_iter().map(|(v, (i, j))| (v, (i - low.0, j - low.1))).collect());
    }
    Ok(out)
}

/// One piece's grid on the layout: columns and rows as long as their edges
/// on the part, `scale` layout units to the metre, square to the layout as
/// it lies best over the old, and centred where the old was.
fn lay_out(mesh: &Mesh, layout: &UvLayout, triangles: &[u32], piece: &HashMap<u32, (i32, i32)>, scale: f32, options: &GridOptions) -> Vec<(u32, Vec2)> {
    let columns = piece.values().map(|c| c.0).max().unwrap_or(0) as usize;
    let rows = piece.values().map(|c| c.1).max().unwrap_or(0) as usize;
    let mut by_place: HashMap<(i32, i32), u32> = HashMap::new();
    for (&v, &place) in piece {
        by_place.insert(place, v);
    }
    let point = |v: u32| mesh.positions[layout.uv_vertices[v as usize].vertex as usize];
    let (mut width, mut height) = (vec![(0.0f32, 0usize); columns], vec![(0.0f32, 0usize); rows]);
    for (&(i, j), &v) in &by_place {
        if let Some(&right) = by_place.get(&(i + 1, j)) {
            let slot = &mut width[i as usize];
            slot.0 += (point(right) - point(v)).length();
            slot.1 += 1;
        }
        if let Some(&below) = by_place.get(&(i, j + 1)) {
            let slot = &mut height[j as usize];
            slot.0 += (point(below) - point(v)).length();
            slot.1 += 1;
        }
    }
    let averaged = |list: &[(f32, usize)]| -> Vec<f32> {
        let each: Vec<f32> = list.iter().map(|(sum, n)| if *n > 0 { sum / *n as f32 } else { 0.0 }).collect();
        if options.even {
            let mean = each.iter().sum::<f32>() / each.len().max(1) as f32;
            vec![mean; each.len()]
        } else {
            each
        }
    };
    let (width, height) = (averaged(&width), averaged(&height));
    let at = |list: &[f32], n: i32| list[..n as usize].iter().sum::<f32>() * scale;
    let vertices: Vec<u32> = by_place.values().copied().collect();
    let old: Vec<Vec2> = vertices.iter().map(|&v| layout.uv_vertices[v as usize].uv).collect();
    let mut new: Vec<Vec2> = vertices.iter().map(|v| {
        let (i, j) = piece[v];
        vec2(at(&width, i), at(&height, j))
    }).collect();
    // Reading as the patch did: the grid mirrored across if it would not.
    let index: HashMap<u32, usize> = vertices.iter().enumerate().map(|(k, &v)| (v, k)).collect();
    let signed = |places: &[Vec2]| -> f64 {
        triangles
            .iter()
            .filter_map(|&t| {
                let k = [0, 1, 2].map(|c| index.get(&layout.corner_vertex[t as usize * 3 + c]).copied());
                Some(facing(places[k[0]?], places[k[1]?], places[k[2]?]) as f64)
            })
            .sum()
    };
    if signed(&new) * signed(&old) < 0.0 {
        new.iter_mut().for_each(|p| p.x = -p.x);
    }
    // Turned square, the quarter that lies best over the old.
    let quarter = std::f32::consts::FRAC_PI_2;
    let angle = (best_fit(&new, &old).0 / quarter).round() * quarter;
    let mut new: Vec<Vec2> = new.iter().map(|p| turned(*p, angle)).collect();
    if options.fit_square {
        let (low, high) = new.iter().fold((Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)), |(l, h), p| (l.min(*p), h.max(*p)));
        let size = (high - low).max(Vec2::splat(1e-12));
        new.iter_mut().for_each(|p| *p = vec2((p.x - low.x) / size.x, (p.y - low.y) / size.y));
    } else {
        let shift = box_middle(old.iter().copied()) - box_middle(new.iter().copied());
        new.iter_mut().for_each(|p| *p = *p + shift);
    }
    vertices.into_iter().zip(new).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{set_positions, transform, Pivot, UvAffine, UvLayout, UvSelection};

    /// A sheet of quads, curved and laid out at a slant: as a grid, its
    /// rows run straight across and its columns straight down, each as
    /// long as the part's.
    #[test]
    fn a_curved_quad_sheet_lies_out_in_straight_rows_and_columns() {
        let mesh = unwrapped(sheet(4, 6, |x, _| (x * 3.0).sin() * 0.2));
        let slanted = transform(&mesh, 0, &UvSelection::Islands(vec![0]), &UvAffine::rotation(0.3), Pivot::Islands).unwrap();
        let (moves, report) = grid(&slanted, 0, &Pick::default(), &GridOptions::default()).unwrap();
        assert_eq!(report.grids, 1);
        assert_eq!((report.columns.max(report.rows), report.columns.min(report.rows)), (6, 4));
        let out = set_positions(&slanted, 0, &moves).unwrap();
        let layout = UvLayout::new(&out, 0).unwrap();
        assert!(layout.folded_triangles().is_empty());
        // Every vertex of a row of the part on one line across or down.
        let mut levels: Vec<(f32, f32)> = Vec::new();
        for (v, p) in out.positions.iter().enumerate() {
            levels.push((p.z, out.uv_sets[0][v].y));
        }
        let rows_are_flat = levels.iter().all(|(z, y)| levels.iter().filter(|(z2, _)| (z2 - z).abs() < 1e-5).all(|(_, y2)| (y2 - y).abs() < 1e-4));
        let columns: Vec<(f32, f32)> = out.positions.iter().zip(&out.uv_sets[0]).map(|(p, uv)| (p.z, uv.x)).collect();
        let rows_are_upright = columns.iter().all(|(z, x)| columns.iter().filter(|(z2, _)| (z2 - z).abs() < 1e-5).all(|(_, x2)| (x2 - x).abs() < 1e-4));
        assert!(rows_are_flat || rows_are_upright, "the part's rows lie square on the layout");
        // In its place, about its size.
        let before = UvLayout::new(&slanted, 0).unwrap();
        assert!((layout.islands[0].centre - before.islands[0].centre).length() < 1e-4);
        assert!((layout.islands[0].uv_area / before.islands[0].uv_area - 1.0).abs() < 0.05);
        // Fitted to the square, it fills it.
        let (moves, _) = grid(&slanted, 0, &Pick::default(), &GridOptions { even: true, fit_square: true }).unwrap();
        let out = set_positions(&slanted, 0, &moves).unwrap();
        let island = &UvLayout::new(&out, 0).unwrap().islands[0];
        assert!((island.min - Vec2::ZERO).length() < 1e-5 && (island.max - Vec2::ONE).length() < 1e-5, "{:?} {:?}", island.min, island.max);
    }

    #[test]
    fn faces_that_are_not_four_sided_are_counted_and_refused() {
        // A triangle split in three round its middle: each piece's longest
        // edge is on the rim, where nothing pairs with it.
        let positions: Vec<tequila_core::math::Vec3> = std::iter::once(tequila_core::math::vec3(0.0, 0.0, 0.0))
            .chain((0..3).map(|i| {
                let a = i as f32 / 3.0 * std::f32::consts::TAU;
                tequila_core::math::vec3(a.cos(), 0.0, a.sin())
            }))
            .collect();
        let indices: Vec<u32> = (0..3).flat_map(|i| [0, 1 + (i + 1) % 3, 1 + i]).collect();
        let mesh = unwrapped(Mesh::new(positions, indices));
        let error = grid(&mesh, 0, &Pick::default(), &GridOptions::default()).unwrap_err();
        assert!(matches!(&error, ShapeError::Cannot(why) if why == "3 faces aren't four-sided"), "{error}");
    }
}
