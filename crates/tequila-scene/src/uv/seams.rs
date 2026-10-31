//! Seams, which are where the layout is split: cutting along edges of the
//! part, and stitching islands back together along the edges they share.
//!
//! There is no seam attribute to set: a seam is an edge whose two sides
//! the layout puts in different places (see [`super`]). So a cut has to
//! part the layout there, and a stitch has to bring it together. A cut that
//! divides an island moves the pieces a gap apart, so they are islands of
//! their own; asked to, it also lays each piece out flat again, which is
//! also what opens a cut that does not divide its island - a slit, which
//! lets a dome or a cone lie flatter - and what keeps it open. A slit not
//! laid out again closes: its sides are still in one place.

use super::edit::{finish, layout_of, UvError};
use super::flatten::facing;
use super::layout::UnionFind;
use super::reflatten::relax;
use super::transform::{check_no_new_folds, UvAffine};
use super::MeshEdge;
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

/// How a cut is made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutOptions {
    /// Lay each piece the cut touches out flat again, in its place.
    pub reflatten: bool,
    /// How far apart the pieces of a divided island are moved, in the
    /// layout's units.
    pub gap: f32,
}

impl Default for CutOptions {
    fn default() -> Self {
        CutOptions { reflatten: true, gap: 0.005 }
    }
}

/// The layout cut along `edges`, each named by the mesh vertices at its ends
/// on either side of it. Edges that are seams already stay as they are.
pub fn cut(mesh: &Mesh, set: usize, edges: &[MeshEdge], options: &CutOptions) -> Result<Mesh, UvError> {
    let layout = layout_of(mesh, set)?;
    if edges.is_empty() {
        return Err(UvError::NothingChosen);
    }
    let table = &layout.table;
    let mut cut = vec![false; table.sides.len()];
    for &edge in edges {
        let sides = layout.edge_group(edge).ok_or(UvError::NoSuchEdge(edge))?;
        cut[sides].iter_mut().for_each(|c| *c = true);
    }
    // Everything joined before joined again, but across the cut.
    let count = layout.island_of.len();
    let cv = &layout.corner_vertex;
    let mut faces = UnionFind::new(count);
    let mut corners = UnionFind::new(count * 3);
    for group in table.groups() {
        if cut[group.start] {
            continue;
        }
        let sides = &table.sides[group];
        for i in 1..sides.len() {
            let s = &sides[i];
            for r in &sides[..i] {
                if cv[s.lo_corner()] == cv[r.lo_corner()] && cv[s.hi_corner()] == cv[r.hi_corner()] {
                    faces.union(s.triangle as usize, r.triangle as usize);
                    corners.union(s.lo_corner(), r.lo_corner());
                    corners.union(s.hi_corner(), r.hi_corner());
                    break;
                }
            }
        }
    }
    // Corners at one UV vertex not joined across an edge were joined only
    // round their fan, which the cut may have opened.
    let mut group_of_root = vec![u32::MAX; count * 3];
    let mut corner_group = vec![0u32; count * 3];
    let mut uv: Vec<Vec2> = Vec::new();
    for c in 0..count * 3 {
        let root = corners.find(c);
        if group_of_root[root] == u32::MAX {
            group_of_root[root] = uv.len() as u32;
            uv.push(layout.uv_vertices[cv[c] as usize].uv);
        }
        corner_group[c] = group_of_root[root];
    }
    let mut piece_of_root = vec![u32::MAX; count];
    let mut piece_of = vec![0u32; count];
    let mut pieces: Vec<Vec<u32>> = Vec::new();
    for t in 0..count {
        let root = faces.find(t);
        if piece_of_root[root] == u32::MAX {
            piece_of_root[root] = pieces.len() as u32;
            pieces.push(Vec::new());
        }
        piece_of[t] = piece_of_root[root];
        pieces[piece_of[t] as usize].push(t as u32);
    }
    // The islands the cut changed: divided, or opened inside.
    for island in &layout.islands {
        let mut parts: Vec<u32> = island.triangles.iter().map(|&t| piece_of[t as usize]).collect();
        parts.sort_unstable();
        parts.dedup();
        let mut places: Vec<u32> = island.triangles.iter().flat_map(|&t| (0..3).map(move |k| t as usize * 3 + k)).map(|c| corner_group[c]).collect();
        places.sort_unstable();
        places.dedup();
        if parts.len() == 1 && places.len() == island.uv_vertices.len() {
            continue;
        }
        if options.reflatten {
            for &p in &parts {
                let triangles = &pieces[p as usize];
                let mut vertices: Vec<u32> = triangles.iter().flat_map(|&t| (0..3).map(move |k| t as usize * 3 + k)).map(|c| corner_group[c]).collect();
                vertices.sort_unstable();
                vertices.dedup();
                if let Some(placed) = relax(mesh, &corner_group, triangles, &vertices, &uv, &[], island.mirrored, island.density()) {
                    for (&v, at) in vertices.iter().zip(placed) {
                        uv[v as usize] = at;
                    }
                }
            }
        }
        if parts.len() > 1 {
            // Each piece away from the island's middle, so the pieces are
            // apart and the cut is a seam.
            let from = island.centroid;
            for &p in &parts {
                let triangles = &pieces[p as usize];
                let middle = centroid(triangles, &corner_group, &uv);
                let away = middle - from;
                if away.length() <= island.uv_area.sqrt() * 1e-6 {
                    continue;
                }
                let shift = away.normalize() * (options.gap.max(0.0) * 0.5);
                let mut vertices: Vec<u32> = triangles.iter().flat_map(|&t| (0..3).map(move |k| t as usize * 3 + k)).map(|c| corner_group[c]).collect();
                vertices.sort_unstable();
                vertices.dedup();
                for v in vertices {
                    uv[v as usize] = uv[v as usize] + shift;
                }
            }
        }
    }
    Ok(finish(mesh, set, &corner_group, &uv))
}

/// The middle of some triangles' area on the layout.
fn centroid(triangles: &[u32], corner_group: &[u32], uv: &[Vec2]) -> Vec2 {
    let (mut sum, mut area) = (Vec2::ZERO, 0.0f32);
    for &t in triangles {
        let [a, b, c] = [0, 1, 2].map(|k| uv[corner_group[t as usize * 3 + k] as usize]);
        let w = facing(a, b, c).abs();
        sum = sum + (a + b + c) * (w / 3.0);
        area += w;
    }
    if area > 0.0 { sum / area } else { triangles.first().map(|&t| uv[corner_group[t as usize * 3] as usize]).unwrap_or(Vec2::ZERO) }
}

/// Islands joined along `edges`, seams each named by the mesh vertices at
/// its ends on either side: the smaller of two islands moved onto the
/// larger - turned, scaled and, if it is mirrored against it, mirrored - so
/// the seam's two sides meet, and then put exactly on each other; where
/// that turns the smaller's triangles over beside the seam, it is laid out
/// again with its side of the seam pinned there. A seam inside one island
/// is closed where it is. Refused where the result would still fold.
pub fn stitch(mesh: &Mesh, set: usize, edges: &[MeshEdge]) -> Result<Mesh, UvError> {
    let layout = layout_of(mesh, set)?;
    if edges.is_empty() {
        return Err(UvError::NothingChosen);
    }
    let cv = &layout.corner_vertex;
    // Each seam's two sides, as the UV vertices at their ends.
    let mut joins: Vec<((u32, u32), [(u32, u32); 2])> = Vec::new();
    for &edge in edges {
        let group = layout.edge_group(edge).ok_or(UvError::NoSuchEdge(edge))?;
        let sides = &layout.table.sides[group];
        let ends = |s: &super::layout::Side| (cv[s.lo_corner()], cv[s.hi_corner()]);
        let first = sides.iter().find(|s| s.vertices == edge || s.vertices == [edge[1], edge[0]]).unwrap_or(&sides[0]);
        let Some(other) = sides.iter().find(|s| ends(s) != ends(first)) else { continue };
        let (x, y) = (ends(first), ends(other));
        let (a, b) = (layout.island_of[first.triangle as usize], layout.island_of[other.triangle as usize]);
        joins.push(((a, b), [(x.0, y.0), (x.1, y.1)]));
    }
    if joins.is_empty() {
        return Ok(mesh.clone());
    }
    joins.sort_by_key(|((a, b), _)| (*a.min(b), *a.max(b)));
    let islands = layout.islands.len();
    let mut uv: Vec<Vec2> = layout.uv_vertices.iter().map(|v| v.uv).collect();
    let mut joined = UnionFind::new(islands);
    let mut members: Vec<Vec<u32>> = layout.islands.iter().map(|i| i.uv_vertices.clone()).collect();
    let mut triangles: Vec<Vec<u32>> = layout.islands.iter().map(|i| i.triangles.clone()).collect();
    let mut area: Vec<f64> = layout.islands.iter().map(|i| i.uv_area as f64).collect();
    let mut mirrored: Vec<bool> = layout.islands.iter().map(|i| i.mirrored).collect();
    let mut touched: Vec<u32> = Vec::new();
    let mut at = 0;
    while at < joins.len() {
        let key = joins[at].0;
        let pair = (key.0.min(key.1), key.0.max(key.1));
        let mut end = at;
        while end < joins.len() && (joins[end].0 .0.min(joins[end].0 .1), joins[end].0 .0.max(joins[end].0 .1)) == pair {
            end += 1;
        }
        let batch = &joins[at..end];
        at = end;
        touched.extend([pair.0, pair.1]);
        let (p, q) = (joined.find(pair.0 as usize), joined.find(pair.1 as usize));
        if p == q {
            // One island: its seam closed where it is, each pair of ends met
            // half way.
            for (_, ends) in batch {
                for &(x, y) in ends {
                    let middle = (uv[x as usize] + uv[y as usize]) * 0.5;
                    uv[x as usize] = middle;
                    uv[y as usize] = middle;
                }
            }
            continue;
        }
        let (stay, mover) = if area[p] >= area[q] { (p, q) } else { (q, p) };
        let in_mover = |v: u32, joined: &mut UnionFind| joined.find(layout.uv_vertices[v as usize].island as usize) == mover;
        let mut pairs: Vec<(u32, u32)> = Vec::new();
        for (_, ends) in batch {
            for &(x, y) in ends {
                let pair = if in_mover(x, &mut joined) { (x, y) } else { (y, x) };
                if !pairs.contains(&pair) {
                    pairs.push(pair);
                }
            }
        }
        if mirrored[mover] != mirrored[stay] {
            let middle = members[mover].iter().fold(Vec2::ZERO, |s, &v| s + uv[v as usize]) / members[mover].len().max(1) as f32;
            for &v in &members[mover] {
                let p = uv[v as usize];
                uv[v as usize] = vec2(2.0 * middle.x - p.x, p.y);
            }
            mirrored[mover] = !mirrored[mover];
        }
        let from: Vec<Vec2> = pairs.iter().map(|(x, _)| uv[*x as usize]).collect();
        let to: Vec<Vec2> = pairs.iter().map(|(_, y)| uv[*y as usize]).collect();
        let (map, scale) = similarity(&from, &to);
        for &v in &members[mover] {
            uv[v as usize] = map.apply(uv[v as usize]);
        }
        for &(x, y) in &pairs {
            uv[x as usize] = uv[y as usize];
        }
        // Two sides laid out apart need not have the same shape, and a seam
        // snapped shut can turn over the thin triangles beside it. Then the
        // smaller is laid out again with its side of the seam pinned where
        // it was put, so it follows the larger's - when it is one island,
        // which has no stitched seam of its own to come open.
        let one_island = triangles[mover].iter().all(|&t| layout.island_of[t as usize] == layout.island_of[triangles[mover][0] as usize]);
        if one_island && turns_over(cv, &triangles[mover], &uv, mirrored[mover]) {
            let mut held: Vec<u32> = pairs.iter().map(|(x, _)| *x).collect();
            held.sort_unstable();
            held.dedup();
            let density = layout.islands[layout.island_of[triangles[mover][0] as usize] as usize].density() * scale * scale;
            if let Some(placed) = relax(mesh, cv, &triangles[mover], &members[mover], &uv, &held, mirrored[mover], density) {
                for (&v, at) in members[mover].iter().zip(placed) {
                    uv[v as usize] = at;
                }
            }
        }
        let moved = std::mem::take(&mut members[mover]);
        let moved_triangles = std::mem::take(&mut triangles[mover]);
        let gained = area[mover] * (scale as f64) * (scale as f64);
        joined.union(stay, mover);
        let root = joined.find(stay);
        let mut all = std::mem::take(&mut members[stay]);
        all.extend(moved);
        members[root] = all;
        let mut all = std::mem::take(&mut triangles[stay]);
        all.extend(moved_triangles);
        triangles[root] = all;
        area[root] = area[stay] + gained;
        mirrored[root] = mirrored[stay];
    }
    touched.sort_unstable();
    touched.dedup();
    // Every island the stitch moved, checked in full: the movers turned
    // whole and the seams snapped shut must leave no triangle turned over.
    let mut moved: Vec<u32> = (0..islands as u32).filter(|&i| touched.contains(&i) || joined.find(i as usize) != i as usize).collect();
    moved.sort_unstable();
    check_no_new_folds(&layout, &moved, &uv)?;
    Ok(finish(mesh, set, cv, &uv))
}

/// Whether any of these triangles is turned over, by more than a hair,
/// against the way their island reads.
fn turns_over(corner_vertex: &[u32], triangles: &[u32], uv: &[Vec2], mirrored: bool) -> bool {
    let sign = if mirrored { -1.0 } else { 1.0 };
    let areas: Vec<f32> = triangles
        .iter()
        .map(|&t| {
            let [a, b, c] = [0, 1, 2].map(|k| uv[corner_vertex[t as usize * 3 + k] as usize]);
            facing(a, b, c) * sign
        })
        .collect();
    let hair = areas.iter().map(|a| a.abs()).sum::<f32>() * super::flatten::FOLD_HAIR;
    areas.iter().any(|&a| a < -hair)
}

/// The turn, scale and move that best put `from` on `to` (Umeyama): the map,
/// and its scale.
fn similarity(from: &[Vec2], to: &[Vec2]) -> (UvAffine, f32) {
    let n = from.len().max(1) as f32;
    let mf = from.iter().fold(Vec2::ZERO, |a, p| a + *p) / n;
    let mt = to.iter().fold(Vec2::ZERO, |a, p| a + *p) / n;
    let (mut dot, mut cross, mut spread) = (0.0f64, 0.0f64, 0.0f64);
    for (a, b) in from.iter().zip(to) {
        let (a, b) = (*a - mf, *b - mt);
        dot += (a.x * b.x + a.y * b.y) as f64;
        cross += (a.x * b.y - a.y * b.x) as f64;
        spread += a.length_squared() as f64;
    }
    if spread <= 0.0 {
        return (UvAffine::translation(mt - mf), 1.0);
    }
    let angle = cross.atan2(dot) as f32;
    let scale = ((dot * dot + cross * cross).sqrt() / spread) as f32;
    let map = UvAffine::translation(-mf).then(&UvAffine::rotation(angle)).then(&UvAffine::uniform(scale)).then(&UvAffine::translation(mt));
    (map, scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{EdgeKind, UvLayout};

    /// The edges of a sheet along the line across it at `x`, by mesh vertex.
    fn line_across(mesh: &Mesh, x: f32) -> Vec<MeshEdge> {
        let mut out = Vec::new();
        for t in 0..mesh.triangle_count() {
            let tri = mesh.triangle(t);
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                let (pa, pb) = (mesh.positions[a as usize], mesh.positions[b as usize]);
                if (pa.x - x).abs() < 1e-5 && (pb.x - x).abs() < 1e-5 && a < b {
                    out.push([a, b]);
                }
            }
        }
        out
    }

    #[test]
    fn cutting_a_sheet_across_makes_two_islands_with_a_seam_between() {
        let mesh = unwrapped(sheet(4, 8, |x, _| x * 0.3));
        let edges = line_across(&mesh, 0.5);
        assert_eq!(edges.len(), 4);
        for reflatten in [false, true] {
            let out = cut(&mesh, 0, &edges, &CutOptions { reflatten, ..Default::default() }).unwrap();
            let layout = UvLayout::new(&out, 0).unwrap();
            assert_eq!(layout.islands.len(), 2, "reflatten {reflatten}");
            assert_eq!(layout.edges.iter().filter(|e| e.kind == EdgeKind::Seam).count(), 8, "four edges, two sides each");
            assert!(layout.folded_triangles().is_empty());
            // The part is the part: the same triangles at the same places.
            for t in 0..out.triangle_count() {
                assert_eq!(out.triangle(t).map(|v| out.positions[v as usize]), mesh.triangle(t).map(|v| mesh.positions[v as usize]));
            }
            // The area on the layout is all still there.
            let before: f32 = UvLayout::new(&mesh, 0).unwrap().islands.iter().map(|i| i.uv_area).sum();
            let after: f32 = layout.islands.iter().map(|i| i.uv_area).sum();
            assert!((before - after).abs() / before < 1e-3, "{before} {after}");
            // And the islands are apart.
            let (a, b) = (&layout.islands[0], &layout.islands[1]);
            assert!(a.max.x < b.min.x || b.max.x < a.min.x || a.max.y < b.min.y || b.max.y < a.min.y);
        }
    }

    #[test]
    fn stitching_the_cut_back_makes_one_island_again() {
        let mesh = unwrapped(sheet(4, 8, |x, _| x * 0.3));
        let edges = line_across(&mesh, 0.5);
        let parted = cut(&mesh, 0, &edges, &CutOptions::default()).unwrap();
        let layout = UvLayout::new(&parted, 0).unwrap();
        let seams = layout.shared_seams(&[0, 1]);
        assert_eq!(seams.len(), 4);
        let whole = stitch(&parted, 0, &seams).unwrap();
        let layout = UvLayout::new(&whole, 0).unwrap();
        assert_eq!(layout.islands.len(), 1);
        assert_eq!(layout.edges.iter().filter(|e| e.kind == EdgeKind::Seam).count(), 0);
        assert!(layout.folded_triangles().is_empty());
    }

    #[test]
    fn stitching_two_sides_of_a_box_moves_the_smaller_onto_the_larger() {
        let mesh = unwrapped(cube());
        // Make side 0 the larger, then stitch it to a neighbour.
        let mesh = crate::uv::transform(&mesh, 0, &crate::uv::UvSelection::Islands(vec![0]), &UvAffine::uniform(1.5), crate::uv::Pivot::Islands).unwrap();
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let first = &layout.edges[layout.islands[0].edges.start];
        let neighbour = layout.edges[first.twin.unwrap() as usize].island;
        let seams = layout.shared_seams(&[0, neighbour]);
        let big_before = layout.islands[0].clone();
        let out = stitch(&mesh, 0, &seams).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        assert_eq!(after.islands.len(), 5);
        assert!(after.folded_triangles().is_empty());
        // The larger did not move: the island holding its triangles still
        // has its corners where they were.
        let holder = &after.islands[after.island_of[big_before.triangles[0] as usize] as usize];
        assert!(holder.min.x <= big_before.min.x + 1e-6 && holder.min.y <= big_before.min.y + 1e-6);
        for &t in &big_before.triangles {
            assert_eq!(after.triangle_uv(t), layout.triangle_uv(t));
        }
        // And the smaller came at the larger's density, joining it evenly.
        let d = holder.density();
        assert!((d - big_before.density()).abs() / d < 1e-3, "{d} {}", big_before.density());
    }

    #[test]
    fn a_mirrored_island_is_mirrored_back_to_stitch() {
        let mesh = unwrapped(sheet(4, 8, |x, _| x * 0.3));
        let edges = line_across(&mesh, 0.5);
        let parted = cut(&mesh, 0, &edges, &CutOptions::default()).unwrap();
        let flipped = crate::uv::transform(&parted, 0, &crate::uv::UvSelection::Islands(vec![1]), &UvAffine::flip_u(), crate::uv::Pivot::Islands).unwrap();
        let layout = UvLayout::new(&flipped, 0).unwrap();
        assert!(layout.islands[0].mirrored != layout.islands[1].mirrored);
        let whole = stitch(&flipped, 0, &layout.shared_seams(&[0, 1])).unwrap();
        let layout = UvLayout::new(&whole, 0).unwrap();
        assert_eq!(layout.islands.len(), 1);
        assert!(layout.folded_triangles().is_empty());
    }

    #[test]
    fn a_slit_opens_when_its_island_is_laid_out_again() {
        // A dome: a cut from its rim to its top lets it lie flatter.
        let dome = sheet(10, 10, |x, z| 0.3 - ((x - 0.5).powi(2) + (z - 0.5).powi(2)) * 0.8);
        let mesh = unwrapped(dome);
        let before = UvLayout::new(&mesh, 0).unwrap();
        assert_eq!(before.islands.len(), 1, "the dome is one island");
        // The edges down the middle from the rim at z = 0 to the top.
        let mut edges = Vec::new();
        for t in 0..mesh.triangle_count() {
            let tri = mesh.triangle(t);
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                let (pa, pb) = (mesh.positions[a as usize], mesh.positions[b as usize]);
                if (pa.x - 0.5).abs() < 1e-5 && (pb.x - 0.5).abs() < 1e-5 && pa.z.max(pb.z) <= 0.5 + 1e-5 && a < b {
                    edges.push([a, b]);
                }
            }
        }
        assert_eq!(edges.len(), 5);
        let out = cut(&mesh, 0, &edges, &CutOptions::default()).unwrap();
        let layout = UvLayout::new(&out, 0).unwrap();
        assert_eq!(layout.islands.len(), 1, "a slit does not divide it");
        assert!(layout.edges.iter().filter(|e| e.kind == EdgeKind::Seam).count() >= 8, "but it is open");
        assert!(layout.folded_triangles().is_empty());
        // Not laid out again, a slit has nothing to keep it open.
        let closed = cut(&mesh, 0, &edges, &CutOptions { reflatten: false, ..Default::default() }).unwrap();
        assert_eq!(UvLayout::new(&closed, 0).unwrap().edges.iter().filter(|e| e.kind == EdgeKind::Seam).count(), 0);
    }

    #[test]
    fn an_edge_that_is_not_one_is_refused() {
        let mesh = unwrapped(cube());
        assert_eq!(cut(&mesh, 0, &[[0, 0]], &CutOptions::default()), Err(UvError::NoSuchEdge([0, 0])));
        assert_eq!(stitch(&mesh, 0, &[]), Err(UvError::NothingChosen));
    }
}
