//! Match seam sides: the copies of a vertex of the part on either side of
//! a seam lined up with each other - at one height, one place across, or
//! one place - so artwork running over the seam meets itself.
//!
//! The copies meet half way, or the other side's go to the picked side's.
//! Each island a copy moved on is carried along with it smoothly, so a seam
//! lined up as a whole moves its islands whole, and one lined up a little
//! bends them a little; pins hold. Copies put at one place stay a seam:
//! here a seam is where the layout is apart, so they are left a hair apart,
//! each toward its own side - far less than a pixel of any texture - and
//! Stitch is what joins them.

use super::layout::UvLayout;
use super::shape::{cannot, edge_ends, follow, islands_of_pick, layout, Moves, Patch, Pick, ShapeError};
use crate::geometry::Mesh;
use std::collections::{BTreeMap, HashMap};
use tequila_core::math::{vec2, Vec2};

/// What lining up means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LineUp {
    /// The same v: side by side across the layout.
    #[default]
    Height,
    /// The same u: one above the other.
    Across,
    /// The same place.
    Place,
}

/// Which copies move.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mover {
    /// Both, to the middle.
    #[default]
    Both,
    /// The other side's, to the picked side's.
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MatchReport {
    /// Vertices of the part whose copies were lined up.
    pub vertices: usize,
    pub islands: usize,
}

/// The seam vertices `pick` names - picked vertices, the ends of picked
/// edges, or every seam vertex of the picked islands (of every island when
/// nothing is picked) - lined up with their copies; `pins` held.
pub fn match_seams(mesh: &Mesh, set: usize, pick: &Pick, line_up: LineUp, mover: Mover, pins: &[u32]) -> Result<(Moves, MatchReport), ShapeError> {
    let layout = layout(mesh, set)?;
    // Every UV vertex of each welded vertex: its copies.
    let mut copies: HashMap<u32, Vec<u32>> = HashMap::new();
    for (g, v) in layout.uv_vertices.iter().enumerate() {
        copies.entry(v.weld).or_default().push(g as u32);
    }
    let picked: Vec<u32> = if !pick.vertices.is_empty() {
        layout.uv_vertices_of(&pick.vertices)
    } else if !pick.edges.is_empty() {
        let mut out = Vec::new();
        for &edge in &pick.edges {
            out.extend(edge_ends(&layout, edge)?);
        }
        out
    } else {
        let islands = islands_of_pick(&layout, pick)?;
        islands.iter().flat_map(|&i| layout.islands[i as usize].uv_vertices.iter().copied()).collect()
    };
    // Each welded vertex once, with the side picked first.
    let mut groups: BTreeMap<u32, (u32, Vec<u32>)> = BTreeMap::new();
    for &g in &picked {
        let weld = layout.uv_vertices[g as usize].weld;
        let all = &copies[&weld];
        if all.len() < 2 {
            continue;
        }
        groups.entry(weld).or_insert_with(|| (g, all.clone()));
    }
    if groups.is_empty() {
        return cannot("none of the picked vertices is on a seam");
    }
    let place = |g: u32| layout.uv_vertices[g as usize].uv;
    let hair = layout.tolerance * 8.0;
    let inward = if line_up == LineUp::Place { inwards(&layout, groups.values().flat_map(|(_, all)| all.iter().copied())) } else { HashMap::new() };
    let inward = |g: u32| inward.get(&g).copied().unwrap_or(Vec2::ZERO);
    let mut target: HashMap<u32, Vec2> = HashMap::new();
    for (first, all) in groups.values() {
        let goal = match mover {
            Mover::Both => all.iter().fold(Vec2::ZERO, |s, &g| s + place(g)) / all.len() as f32,
            Mover::Other => place(*first),
        };
        for &g in all {
            if mover == Mover::Other && g == *first {
                continue;
            }
            let p = place(g);
            let to = match line_up {
                LineUp::Height => vec2(p.x, goal.y),
                LineUp::Across => vec2(goal.x, p.y),
                LineUp::Place => goal + inward(g) * hair,
            };
            target.insert(g, to);
        }
        if mover == Mover::Other && line_up == LineUp::Place {
            // The picked side a hair toward its own side too.
            target.insert(*first, place(*first) + inward(*first) * hair);
        }
    }
    // Each island a copy moved on carried along with it.
    let held_pins = layout.uv_vertices_of(pins);
    let mut by_island: BTreeMap<u32, Vec<(u32, Vec2)>> = BTreeMap::new();
    for (&g, &to) in &target {
        by_island.entry(layout.uv_vertices[g as usize].island).or_default().push((g, to));
    }
    let mut moves = Moves::new();
    for (&island, moved) in &by_island {
        let patch = Patch::new(mesh, &layout, &layout.islands[island as usize].triangles);
        let mut held: Vec<(usize, Vec2)> = moved.iter().map(|&(g, p)| (patch.local[&g], p)).collect();
        for g in &held_pins {
            if let Some(&i) = patch.local.get(g) {
                if !target.contains_key(g) {
                    held.push((i, patch.uv[i]));
                }
            }
        }
        // The picked side, staying, holds its island still.
        if mover == Mover::Other {
            for (first, _) in groups.values() {
                if let Some(&i) = patch.local.get(first) {
                    if !target.contains_key(first) {
                        held.push((i, patch.uv[i]));
                    }
                }
            }
        }
        let Some(uv) = follow(&patch, &held, &patch.weights()) else {
            return cannot(format!("island {} would fold: line up fewer at a time", island + 1));
        };
        moves.extend(patch.moves(&uv));
    }
    Ok((moves, MatchReport { vertices: groups.len(), islands: by_island.len() }))
}

/// The way into each of some UV vertices' own side of the layout: toward
/// the middle of the triangles round it.
fn inwards(layout: &UvLayout, vertices: impl Iterator<Item = u32>) -> HashMap<u32, Vec2> {
    let mut sums: HashMap<u32, (Vec2, usize)> = vertices.map(|g| (g, (Vec2::ZERO, 0))).collect();
    for (c, &v) in layout.corner_vertex.iter().enumerate() {
        if let Some(slot) = sums.get_mut(&v) {
            let [a, b, d] = layout.triangle_uv((c / 3) as u32);
            slot.0 = slot.0 + (a + b + d) / 3.0;
            slot.1 += 1;
        }
    }
    sums.into_iter()
        .map(|(g, (sum, count))| {
            let d = if count > 0 { sum / count as f32 - layout.uv_vertices[g as usize].uv } else { Vec2::ZERO };
            (g, if d.length() > 0.0 { d.normalize() } else { Vec2::ZERO })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{cut, set_positions, transform, CutOptions, Pivot, UvAffine, UvLayout, UvSelection};

    /// A sheet laid out as it lies, cut down its middle into halves a gap
    /// apart across the layout, the second dropped a little.
    fn halves() -> Mesh {
        let flat = sheet(2, 4, |_, _| 0.0);
        let uv = flat.positions.iter().map(|p| vec2(p.x, p.z)).collect();
        let mesh = flat.with_uvs(uv);
        let edges: Vec<[u32; 2]> = (0..mesh.triangle_count())
            .flat_map(|t| {
                let tri = mesh.triangle(t);
                (0..3).map(move |k| [tri[k], tri[(k + 1) % 3]])
            })
            .filter(|[a, b]| (mesh.positions[*a as usize].x - 0.5).abs() < 1e-5 && (mesh.positions[*b as usize].x - 0.5).abs() < 1e-5 && a < b)
            .collect();
        let parted = cut(&mesh, 0, &edges, &CutOptions { reflatten: false, gap: 0.1 }).unwrap();
        transform(&parted, 0, &UvSelection::Islands(vec![1]), &UvAffine::translation(vec2(0.0, 0.07)), Pivot::Origin).unwrap()
    }

    #[test]
    fn seam_copies_line_up_at_one_height_and_the_islands_move_whole() {
        let mesh = halves();
        let before = UvLayout::new(&mesh, 0).unwrap();
        let (moves, report) = match_seams(&mesh, 0, &Pick::islands(vec![0]), LineUp::Height, Mover::Other, &[]).unwrap();
        assert_eq!((report.vertices, report.islands), (3, 1));
        let out = set_positions(&mesh, 0, &moves).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        assert_eq!(after.islands.len(), 2, "still apart");
        let shift = after.islands[1].centre - before.islands[1].centre;
        assert!((shift - vec2(0.0, -0.07)).length() < 1e-4, "moved whole, back up: {shift:?}");
        assert_eq!(after.islands[0].centre, before.islands[0].centre, "the picked side stayed");
        // At one place: they meet, but a hair apart, so the seam stays.
        let (moves, _) = match_seams(&mesh, 0, &Pick::islands(vec![0]), LineUp::Place, Mover::Both, &[]).unwrap();
        let out = set_positions(&mesh, 0, &moves).unwrap();
        let met = UvLayout::new(&out, 0).unwrap();
        assert_eq!(met.islands.len(), 2, "a seam still");
        assert!(met.folded_triangles().is_empty());
        let gap = (met.islands[0].max.x.min(met.islands[1].max.x) - met.islands[0].min.x.max(met.islands[1].min.x)).abs();
        assert!(gap < 1e-4, "side by side: {gap}");
    }
}
