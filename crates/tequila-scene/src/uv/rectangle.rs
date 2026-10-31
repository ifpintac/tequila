//! Rectangle: an island's outer border - or the border of faces picked on
//! the part - laid out as a rectangle square to the layout, and the inside
//! laid out again within it.
//!
//! The corners are the four places the border turns most sharply, or four
//! vertices picked on it. Between them, its four sides: each as long as it
//! is on the part, at the island's density, the rectangle as wide as the
//! two sides across it are on average, or as the longer, and as high; the
//! vertices along a side spaced evenly, or as their edges are long on the
//! part. The rectangle turned the quarter that lies best over where the
//! border was, and centred there. Inside, the vertices are laid out again
//! with the border held - as true to the surface as they can be, or spread
//! evenly, which inside a rectangle cannot fold - and holes go along with
//! them.

use super::shape::{
    best_fit, border_loops, box_middle, cannot, even_fill, island_round, layout, polygon_area, reflatten_holding, regions, turned, Moves, Patch, Pick, Region, ShapeError,
};
use super::layout::UvLayout;
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

/// How vertices are spaced along a rectangle's sides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Spacing {
    /// As their edges are long on the part.
    #[default]
    Surface,
    Even,
}

/// How the inside follows the border.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Inside {
    /// Laid out again as true to the surface as it can be.
    #[default]
    Reflatten,
    /// Spread evenly, each vertex the average of its neighbours.
    Smooth,
}

/// How long each side is, from the two opposite it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sides {
    #[default]
    Average,
    Longest,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectangleOptions {
    pub spacing: Spacing,
    pub inside: Inside,
    pub sides: Sides,
}

/// What laying out as a rectangle did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RectangleReport {
    pub rectangles: usize,
    /// The mesh vertices on the rectangles' borders, for pinning after.
    pub border: Vec<u32>,
}

/// The regions `pick` names laid out as rectangles; four of its vertices,
/// when it picks four, are the corners.
pub fn rectangle(mesh: &Mesh, set: usize, pick: &Pick, options: &RectangleOptions) -> Result<(Moves, RectangleReport), ShapeError> {
    let layout = layout(mesh, set)?;
    let mut report = RectangleReport::default();
    let mut moves = Moves::new();
    if !pick.vertices.is_empty() && pick.vertices.len() != 4 {
        return cannot("pick four corners on the border, or none");
    }
    // Four corners picked: their island, whole.
    let (list, corners) = if pick.vertices.len() == 4 {
        let island = layout.island_of_vertex(pick.vertices[0]).ok_or(super::UvError::NoSuchVertex(pick.vertices[0]))?;
        let region = Region { island, triangles: layout.islands[island as usize].triangles.clone(), whole: true };
        (vec![region], Some(pick.vertices.clone()))
    } else {
        (regions(&layout, &Pick { vertices: Vec::new(), ..pick.clone() })?, None)
    };
    for region in list {
        let loops = border_loops(&layout, &region.triangles);
        let Some(outer) = loops.first() else { return cannot(format!("island {} has no border", region.island + 1)) };
        if outer.len() < 4 {
            return cannot("the border has fewer than four vertices");
        }
        let corners = match &corners {
            Some(picked) => {
                let mut at: Vec<usize> = Vec::new();
                for &v in picked {
                    let places = layout.uv_vertices_of(&[v]);
                    let Some(k) = outer.iter().position(|g| places.contains(g)) else {
                        return cannot("the four picked vertices are not all on the island's outer border");
                    };
                    at.push(k);
                }
                at.sort_unstable();
                at.dedup();
                if at.len() != 4 {
                    return cannot("pick four different vertices on the border");
                }
                [at[0], at[1], at[2], at[3]]
            }
            None => sharpest_turns(&layout, outer),
        };
        let placed = lay_out(mesh, &layout, outer, corners, &region, options)?;
        report.border.extend(outer.iter().map(|&g| layout.uv_vertices[g as usize].vertex));
        report.rectangles += 1;
        // Inside the border: laid out again, or spread evenly.
        let patch = Patch::new(mesh, &layout, &region.triangles);
        let held: Vec<(usize, Vec2)> = placed.iter().map(|&(g, p)| (patch.local[&g], p)).collect();
        let inside = match options.inside {
            // Where the conformal layout folds, the surface's own weights
            // kept above nothing, which inside a rectangle cannot.
            Inside::Reflatten => reflatten_holding(&patch, &held).or_else(|| even_fill(&patch, &held, &patch.weights())),
            Inside::Smooth => even_fill(&patch, &held, &patch.even_weights()),
        };
        let Some(inside) = inside else { return cannot("the inside would fold: try spreading it evenly, or other corners") };
        let laid = patch.moves(&inside);
        if region.whole {
            moves.extend(laid);
        } else {
            moves.extend(island_round(mesh, &layout, region.island, &laid, &[], "round the rectangle")?);
        }
    }
    report.border.sort_unstable();
    report.border.dedup();
    Ok((moves, report))
}

/// The four corners of a loop: of the places it turns most sharply, the
/// way it runs round - and, for a loop with no corners to speak of, places
/// evenly round it - the four that hold the most area between them. A
/// rounded corner turns over several vertices and gives several
/// candidates, of which the one furthest out wins; a thin strip's two
/// corners a vertex apart are both there. Their places in the loop, in
/// order.
fn sharpest_turns(layout: &UvLayout, outer: &[u32]) -> [usize; 4] {
    let n = outer.len();
    let place = |k: usize| layout.uv_vertices[outer[k % n] as usize].uv;
    let points: Vec<Vec2> = (0..n).map(place).collect();
    let winding = polygon_area(&points).signum() as f32;
    let turn = |k: usize| {
        let (a, b, c) = (place(k + n - 1), place(k), place(k + 1));
        let (d1, d2) = (b - a, c - b);
        let cross = d1.y * d2.x - d1.x * d2.y;
        cross.atan2(d1.dot(d2)) * winding
    };
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| turn(b).total_cmp(&turn(a)));
    let mut candidates: Vec<usize> = order.into_iter().take(24).collect();
    candidates.extend((0..16).map(|i| i * n / 16));
    candidates.sort_unstable();
    candidates.dedup();
    let m = candidates.len();
    let mut best = ([0, n / 4, n / 2, 3 * n / 4], f64::NEG_INFINITY);
    for a in 0..m {
        for b in a + 1..m {
            for c in b + 1..m {
                for d in c + 1..m {
                    let quad = [candidates[a], candidates[b], candidates[c], candidates[d]];
                    let area = polygon_area(&quad.map(|k| points[k])).abs();
                    if area > best.1 {
                        best = (quad, area);
                    }
                }
            }
        }
    }
    best.0
}

/// A border loop laid on a rectangle with its corners at `corners`.
fn lay_out(mesh: &Mesh, layout: &UvLayout, outer: &[u32], corners: [usize; 4], region: &Region, options: &RectangleOptions) -> Result<Vec<(u32, Vec2)>, ShapeError> {
    let n = outer.len();
    let point = |k: usize| mesh.positions[layout.uv_vertices[outer[k % n] as usize].vertex as usize];
    // The four sides, each as its vertices from one corner to the next.
    let sides: Vec<Vec<usize>> = (0..4)
        .map(|s| {
            let (from, to) = (corners[s], corners[(s + 1) % 4]);
            let to = if to <= from { to + n } else { to };
            (from..=to).map(|k| k % n).collect()
        })
        .collect();
    let lengths: Vec<Vec<f32>> = sides.iter().map(|side| side.windows(2).map(|w| (point(w[1]) - point(w[0])).length()).collect()).collect();
    let totals: Vec<f32> = lengths.iter().map(|l| l.iter().sum()).collect();
    let combine = |a: f32, b: f32| match options.sides {
        Sides::Average => (a + b) * 0.5,
        Sides::Longest => a.max(b),
    };
    let density = layout.islands[region.island as usize].density();
    if density <= 0.0 || totals.iter().any(|&t| t <= 0.0) {
        return cannot("a side of the border has no length on the part");
    }
    let scale = density.sqrt();
    let (width, height) = (combine(totals[0], totals[2]) * scale, combine(totals[1], totals[3]) * scale);
    let rectangle = [vec2(0.0, 0.0), vec2(width, 0.0), vec2(width, height), vec2(0.0, height)];
    let mut new = vec![Vec2::ZERO; n];
    for (s, side) in sides.iter().enumerate() {
        let (from, to) = (rectangle[s], rectangle[(s + 1) % 4]);
        let steps = side.len() - 1;
        let mut done = 0.0f32;
        for (k, &at) in side.iter().enumerate() {
            let t = match options.spacing {
                Spacing::Even => k as f32 / steps as f32,
                Spacing::Surface => {
                    if k > 0 {
                        done += lengths[s][k - 1];
                    }
                    done / totals[s]
                }
            };
            new[at] = from + (to - from) * t;
        }
    }
    let old: Vec<Vec2> = (0..n).map(|k| layout.uv_vertices[outer[k] as usize].uv).collect();
    // Running round the way the border ran: mirrored across where not.
    if polygon_area(&new) * polygon_area(&old) < 0.0 {
        new.iter_mut().for_each(|p| p.x = -p.x);
    }
    let quarter = std::f32::consts::FRAC_PI_2;
    let angle = (best_fit(&new, &old).0 / quarter).round() * quarter;
    let mut new: Vec<Vec2> = new.iter().map(|p| turned(*p, angle)).collect();
    let shift = box_middle(old.iter().copied()) - box_middle(new.iter().copied());
    new.iter_mut().for_each(|p| *p = *p + shift);
    Ok(outer.iter().copied().zip(new).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{set_positions, UvLayout};

    /// A disc of quads laid out round: as a rectangle, its four sides lie
    /// straight across and down the layout.
    #[test]
    fn a_bulging_quad_patch_becomes_a_rectangle_square_to_the_layout() {
        // A sheet whose layout bulges: each point pushed out from the
        // middle by how far it is from the edge.
        let mesh = unwrapped(sheet(6, 8, |x, z| (x * 2.0).sin() * 0.1 + z * 0.05));
        let layout0 = UvLayout::new(&mesh, 0).unwrap();
        let centre = layout0.islands[0].centre;
        let bulged: Vec<Vec2> = mesh.uv_sets[0].iter().map(|p| {
            let d = *p - centre;
            centre + d * (1.0 + 0.3 * (1.0 - d.length() / 0.8).max(0.0))
        }).collect();
        let mut bulging = mesh.clone();
        bulging.uv_sets[0] = bulged;
        for inside in [Inside::Reflatten, Inside::Smooth] {
            let (moves, report) = rectangle(&bulging, 0, &Pick::default(), &RectangleOptions { inside, ..Default::default() }).unwrap();
            assert_eq!(report.rectangles, 1);
            assert_eq!(report.border.len(), 28, "the sheet's border");
            let out = set_positions(&bulging, 0, &moves).unwrap();
            let layout = UvLayout::new(&out, 0).unwrap();
            assert!(layout.folded_triangles().is_empty(), "{inside:?}");
            // Every border vertex on the box's edge.
            let island = &layout.islands[0];
            for &g in &island.boundary[0] {
                let p = layout.uv_vertices[g as usize].uv;
                let on = (p.x - island.min.x).abs() < 1e-4 || (p.x - island.max.x).abs() < 1e-4 || (p.y - island.min.y).abs() < 1e-4 || (p.y - island.max.y).abs() < 1e-4;
                assert!(on, "{p:?} is off the rectangle");
            }
            // As large as the part at the island's density, near enough: the
            // sheet bends one way only, so its sides are its size.
            let size = island.max - island.min;
            let area = size.x * size.y;
            let laid = UvLayout::new(&bulging, 0).unwrap().islands[0].uv_area;
            assert!((area / laid - 1.0).abs() < 0.02, "{area} {laid}");
        }
    }

    /// A long thin strip's corners are its four, though two of them are
    /// only a few vertices apart: as a rectangle it is as long and as thin.
    #[test]
    fn a_long_strip_keeps_its_proportions() {
        let strip = sheet(2, 40, |x, _| (x * 4.0).sin() * 0.05);
        let positions: Vec<tequila_core::math::Vec3> = strip.positions.iter().map(|p| tequila_core::math::vec3(p.x, p.y, p.z * 0.06)).collect();
        let mesh = unwrapped(Mesh::new(positions, strip.indices.clone()));
        let (moves, _) = rectangle(&mesh, 0, &Pick::default(), &RectangleOptions::default()).unwrap();
        let out = set_positions(&mesh, 0, &moves).unwrap();
        let island = &UvLayout::new(&out, 0).unwrap().islands[0];
        let size = island.max - island.min;
        let ratio = size.x.max(size.y) / size.x.min(size.y);
        assert!((ratio - 1.0 / 0.06).abs() < 1.0, "as long and thin as the part: {size:?}");
    }

    #[test]
    fn four_picked_corners_are_the_corners() {
        let mesh = unwrapped(sheet(3, 3, |_, _| 0.0));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        // Four border vertices, not the sheet's own corners: the middles of
        // its sides.
        let border = &layout.islands[0].boundary[0];
        let n = border.len();
        let picked: Vec<u32> = [1usize, 1 + n / 4, 1 + n / 2, 1 + 3 * n / 4].iter().map(|&k| layout.uv_vertices[border[k] as usize].vertex).collect();
        let (moves, _) = rectangle(&mesh, 0, &Pick::vertices(picked.clone()), &RectangleOptions { spacing: Spacing::Even, ..Default::default() }).unwrap();
        let out = set_positions(&mesh, 0, &moves).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        assert!(after.folded_triangles().is_empty());
        let island = &after.islands[0];
        for v in picked {
            let p = out.uv_sets[0][v as usize];
            let corner = [island.min, island.max, vec2(island.min.x, island.max.y), vec2(island.max.x, island.min.y)].iter().any(|c| (*c - p).length() < 1e-4);
            assert!(corner, "{p:?} is a corner of {:?} {:?}", island.min, island.max);
        }
    }
}
