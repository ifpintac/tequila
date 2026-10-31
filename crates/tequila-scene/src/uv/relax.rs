//! Relax: picked vertices of the layout smoothed toward less stretch, a
//! number of rounds at a time.
//!
//! Each round moves every free vertex half way to the average of its
//! neighbours, weighed by the cotangents of the part's own angles - the
//! weights under which a layout with its border held settles into the
//! conformal one, as true to the surface's angles as it can be. A vertex on
//! the island's border, picked to move, is smoothed along the border only,
//! toward the place between its two border neighbours that their lengths on
//! the part say, so the border evens out rather than shrinking in. The
//! border is held unless its vertices are picked; pins are always held. A
//! round that would turn a triangle over is not taken, and the rounds stop
//! there.

use super::shape::{edge_ends, cannot, islands_of_pick, layout, Moves, Patch, Pick, ShapeError};
use crate::geometry::Mesh;
use std::collections::HashMap;
use tequila_core::math::vec2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RelaxOptions {
    /// How many rounds of smoothing: more is smoother.
    pub rounds: u32,
}

impl Default for RelaxOptions {
    fn default() -> Self {
        RelaxOptions { rounds: 25 }
    }
}

/// What relaxing did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RelaxReport {
    pub vertices: usize,
    /// The rounds taken: fewer than asked where the next would have folded.
    pub rounds: u32,
}

/// The vertices `pick` names - picked vertices or the ends of picked edges,
/// else the inside of the picked islands, or of every island - relaxed,
/// `pins` (mesh vertices) held.
pub fn relax_layout(mesh: &Mesh, set: usize, pick: &Pick, pins: &[u32], options: &RelaxOptions) -> Result<(Moves, RelaxReport), ShapeError> {
    let layout = layout(mesh, set)?;
    let held: Vec<u32> = layout.uv_vertices_of(pins);
    let mut free: Vec<u32> = if !pick.vertices.is_empty() {
        layout.uv_vertices_of(&pick.vertices)
    } else if !pick.edges.is_empty() {
        let mut out = Vec::new();
        for &edge in &pick.edges {
            out.extend(edge_ends(&layout, edge)?);
        }
        out
    } else {
        let islands = islands_of_pick(&layout, pick)?;
        let mut out = Vec::new();
        for i in islands {
            let island = &layout.islands[i as usize];
            let border: std::collections::HashSet<u32> = island.boundary.iter().flatten().copied().collect();
            out.extend(island.uv_vertices.iter().copied().filter(|v| !border.contains(v)));
        }
        out
    };
    free.sort_unstable();
    free.dedup();
    free.retain(|v| held.binary_search(v).is_err());
    if free.is_empty() {
        return cannot("nothing to relax: pick vertices inside an island, or islands with an inside");
    }
    let mut islands: Vec<u32> = free.iter().map(|&v| layout.uv_vertices[v as usize].island).collect();
    islands.sort_unstable();
    islands.dedup();
    let mut moves = Moves::new();
    let mut taken = u32::MAX;
    for i in islands {
        let island = &layout.islands[i as usize];
        let patch = Patch::new(mesh, &layout, &island.triangles);
        let weights = patch.weights();
        let mut neighbours: Vec<Vec<(usize, f64)>> = vec![Vec::new(); patch.len()];
        for &(a, b, w) in &weights {
            neighbours[a].push((b, w));
            neighbours[b].push((a, w));
        }
        // Each border vertex's neighbours along its loop.
        let mut along: HashMap<usize, (usize, usize)> = HashMap::new();
        for ring in &island.boundary {
            let n = ring.len();
            for k in 0..n {
                let at = |j: usize| patch.local[&ring[j % n]];
                along.insert(at(k), (at(k + n - 1), at(k + 1)));
            }
        }
        let moving: Vec<usize> = free.iter().filter_map(|v| patch.local.get(v).copied()).collect();
        let reads_right = patch.signed_area(&patch.uv) >= 0.0;
        let mut uv = patch.uv.clone();
        let mut rounds = 0;
        for _ in 0..options.rounds {
            let mut next = uv.clone();
            for &v in &moving {
                let target = match along.get(&v) {
                    Some(&(before, after)) => {
                        let (lb, la) = ((patch.points[v] - patch.points[before]).length(), (patch.points[after] - patch.points[v]).length());
                        if lb + la <= 0.0 { uv[v] } else { uv[before] + (uv[after] - uv[before]) * (lb / (lb + la)) }
                    }
                    None => {
                        let (mut sum, mut total) = ((0.0f64, 0.0f64), 0.0f64);
                        for &(n, w) in &neighbours[v] {
                            sum.0 += w * uv[n].x as f64;
                            sum.1 += w * uv[n].y as f64;
                            total += w;
                        }
                        if total <= 0.0 { uv[v] } else { vec2((sum.0 / total) as f32, (sum.1 / total) as f32) }
                    }
                };
                next[v] = uv[v] + (target - uv[v]) * 0.5;
            }
            if patch.folds(&next, reads_right) {
                break;
            }
            uv = next;
            rounds += 1;
        }
        taken = taken.min(rounds);
        moves.extend(moving.iter().map(|&v| (patch.vertices[v], uv[v])));
    }
    Ok((moves.clone(), RelaxReport { vertices: moves.len(), rounds: taken }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{distortion, set_positions, UvLayout};

    fn angle_stretch(mesh: &Mesh) -> f32 {
        let all = distortion(mesh, 0).unwrap();
        all.iter().map(|d| d.angle).sum::<f32>() / all.len() as f32
    }

    /// A flat sheet with its inside jumbled: relaxed, it is near as even as
    /// it was, its border where it was.
    #[test]
    fn a_jumbled_inside_relaxes_and_the_border_stays() {
        let mesh = unwrapped(sheet(6, 6, |_, _| 0.0));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let border: std::collections::HashSet<u32> = layout.islands[0].boundary[0].iter().copied().collect();
        let mut jumbled = mesh.clone();
        for (g, v) in layout.uv_vertices.iter().enumerate() {
            if !border.contains(&(g as u32)) {
                let wobble = ((g * 7919) % 13) as f32 / 13.0 - 0.5;
                jumbled.uv_sets[0][v.vertex as usize] = v.uv + vec2(wobble, -wobble) * 0.02;
            }
        }
        let before = angle_stretch(&jumbled);
        let (moves, report) = relax_layout(&jumbled, 0, &Pick::default(), &[], &RelaxOptions { rounds: 60 }).unwrap();
        assert_eq!(report.vertices, 25, "the inside of a 7 by 7 grid");
        let out = set_positions(&jumbled, 0, &moves).unwrap();
        let after = angle_stretch(&out);
        assert!(after < before && after < 1.02, "{before} to {after}");
        for &g in &border {
            let v = layout.uv_vertices[g as usize].vertex as usize;
            assert_eq!(out.uv_sets[0][v], jumbled.uv_sets[0][v], "the border held");
        }
        assert!(UvLayout::new(&out, 0).unwrap().folded_triangles().is_empty());
        // Every inside vertex pinned: nothing to relax.
        let inside: Vec<u32> = layout.uv_vertices.iter().enumerate().filter(|(g, _)| !border.contains(&(*g as u32))).map(|(_, v)| v.vertex).collect();
        assert!(relax_layout(&jumbled, 0, &Pick::default(), &inside, &RelaxOptions::default()).is_err());
    }
}
