//! Moving islands and vertices on the layout: by any affine map, about a
//! pivot; turned square to the layout; scaled to one texel density.
//!
//! Angles are in radians in the layout's own coordinates, where v runs down
//! the picture: a positive turn takes u toward v, which the editor, drawing
//! v down, shows clockwise.

use super::edit::{chosen_islands, finish, layout_of, UvError};
use super::flatten::facing;
use super::layout::{fold_angle, UvLayout};
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

/// An affine map of the layout, two rows of three:
/// `u' = m[0][0] u + m[0][1] v + m[0][2]` and
/// `v' = m[1][0] u + m[1][1] v + m[1][2]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvAffine {
    pub m: [[f32; 3]; 2],
}

impl Default for UvAffine {
    fn default() -> Self {
        UvAffine::IDENTITY
    }
}

impl UvAffine {
    pub const IDENTITY: UvAffine = UvAffine { m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] };

    pub fn translation(by: Vec2) -> UvAffine {
        UvAffine { m: [[1.0, 0.0, by.x], [0.0, 1.0, by.y]] }
    }

    /// A turn about the origin, u toward v.
    pub fn rotation(radians: f32) -> UvAffine {
        let (s, c) = radians.sin_cos();
        UvAffine { m: [[c, -s, 0.0], [s, c, 0.0]] }
    }

    /// Quarter turns about the origin, u toward v, exactly: no rounding
    /// creeps into four of them.
    pub fn quarter_turns(turns: i32) -> UvAffine {
        let (c, s) = match turns.rem_euclid(4) {
            0 => (1.0, 0.0),
            1 => (0.0, 1.0),
            2 => (-1.0, 0.0),
            _ => (0.0, -1.0),
        };
        UvAffine { m: [[c, -s, 0.0], [s, c, 0.0]] }
    }

    pub fn scale(by: Vec2) -> UvAffine {
        UvAffine { m: [[by.x, 0.0, 0.0], [0.0, by.y, 0.0]] }
    }

    pub fn uniform(by: f32) -> UvAffine {
        UvAffine::scale(Vec2::splat(by))
    }

    /// Mirrored across: u turned back, left for right as the editor shows it.
    pub fn flip_u() -> UvAffine {
        UvAffine::scale(vec2(-1.0, 1.0))
    }

    /// Mirrored up for down.
    pub fn flip_v() -> UvAffine {
        UvAffine::scale(vec2(1.0, -1.0))
    }

    /// This, then `after`.
    pub fn then(&self, after: &UvAffine) -> UvAffine {
        let (a, b) = (&after.m, &self.m);
        let mut m = [[0.0f32; 3]; 2];
        for (r, row) in m.iter_mut().enumerate() {
            row[0] = a[r][0] * b[0][0] + a[r][1] * b[1][0];
            row[1] = a[r][0] * b[0][1] + a[r][1] * b[1][1];
            row[2] = a[r][0] * b[0][2] + a[r][1] * b[1][2] + a[r][2];
        }
        UvAffine { m }
    }

    /// The same map with its linear part about `pivot` rather than the
    /// origin, its translation kept: `p' = L (p - pivot) + pivot + t`.
    pub fn about(&self, pivot: Vec2) -> UvAffine {
        let m = &self.m;
        let moved = vec2(m[0][0] * pivot.x + m[0][1] * pivot.y, m[1][0] * pivot.x + m[1][1] * pivot.y);
        UvAffine { m: [[m[0][0], m[0][1], m[0][2] + pivot.x - moved.x], [m[1][0], m[1][1], m[1][2] + pivot.y - moved.y]] }
    }

    pub fn apply(&self, p: Vec2) -> Vec2 {
        let m = &self.m;
        vec2(m[0][0] * p.x + m[0][1] * p.y + m[0][2], m[1][0] * p.x + m[1][1] * p.y + m[1][2])
    }

    /// The area it scales by, negative where it mirrors.
    pub fn determinant(&self) -> f32 {
        self.m[0][0] * self.m[1][1] - self.m[0][1] * self.m[1][0]
    }

    pub fn is_finite(&self) -> bool {
        self.m.iter().flatten().all(|v| v.is_finite())
    }
}

/// What a transform's turn and scale are about.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Pivot {
    /// The layout's origin: the map as it is written.
    #[default]
    Origin,
    /// A point of the layout.
    Point(Vec2),
    /// The middle of the box round everything chosen.
    Selection,
    /// The middle of each island's box, each about its own, as islands are
    /// turned one by one.
    Islands,
}

/// What a transform moves.
#[derive(Clone, Debug, PartialEq)]
pub enum UvSelection {
    /// Whole islands, by number.
    Islands(Vec<u32>),
    /// Mesh vertices, by number, and every corner on the same place of the
    /// layout with them: a vertex picked on the layout is a point of it, not
    /// one of the mesh vertices that happen to be there.
    Vertices(Vec<u32>),
}

impl UvSelection {
    pub fn is_empty(&self) -> bool {
        match self {
            UvSelection::Islands(list) | UvSelection::Vertices(list) => list.is_empty(),
        }
    }
}

/// The UV vertices a selection moves, checked.
pub(crate) fn chosen_vertices(layout: &UvLayout, selection: &UvSelection) -> Result<Vec<u32>, UvError> {
    match selection {
        UvSelection::Islands(islands) => {
            let islands = chosen_islands(layout, islands)?;
            let mut out: Vec<u32> = islands.iter().flat_map(|&i| layout.islands[i as usize].uv_vertices.iter().copied()).collect();
            out.sort_unstable();
            Ok(out)
        }
        UvSelection::Vertices(vertices) => {
            if let Some(&bad) = vertices.iter().find(|&&v| v as usize >= layout.vertex_island.len()) {
                return Err(UvError::NoSuchVertex(bad));
            }
            Ok(layout.uv_vertices_of(vertices))
        }
    }
}

/// The selection moved by `transform` about `pivot`. Refused where it would
/// fold a triangle over against its island - a whole island mirrored is
/// not folded, half of one is - or flatten it.
pub fn transform(mesh: &Mesh, set: usize, selection: &UvSelection, transform: &UvAffine, pivot: Pivot) -> Result<Mesh, UvError> {
    let layout = layout_of(mesh, set)?;
    let chosen = chosen_vertices(&layout, selection)?;
    if chosen.is_empty() {
        return Err(UvError::NothingChosen);
    }
    if !transform.is_finite() {
        return Err(UvError::Degenerate);
    }
    if transform.determinant().abs() <= f32::MIN_POSITIVE {
        return Err(UvError::Degenerate);
    }
    let mut uv: Vec<Vec2> = layout.uv_vertices.iter().map(|v| v.uv).collect();
    let map_for = |centre: Vec2| transform.about(centre);
    match pivot {
        Pivot::Origin => chosen.iter().for_each(|&v| uv[v as usize] = transform.apply(uv[v as usize])),
        Pivot::Point(at) => {
            let map = map_for(at);
            chosen.iter().for_each(|&v| uv[v as usize] = map.apply(uv[v as usize]));
        }
        Pivot::Selection => {
            let map = map_for(middle(chosen.iter().map(|&v| uv[v as usize])));
            chosen.iter().for_each(|&v| uv[v as usize] = map.apply(uv[v as usize]));
        }
        Pivot::Islands => {
            // Each island about the middle of what is chosen of it.
            let mut by_island: Vec<Vec<u32>> = vec![Vec::new(); layout.islands.len()];
            for &v in &chosen {
                by_island[layout.uv_vertices[v as usize].island as usize].push(v);
            }
            for list in by_island.iter().filter(|l| !l.is_empty()) {
                let map = map_for(middle(list.iter().map(|&v| uv[v as usize])));
                list.iter().for_each(|&v| uv[v as usize] = map.apply(uv[v as usize]));
            }
        }
    }
    let touched = islands_of(&layout, &chosen);
    check_no_new_folds(&layout, &touched, &uv)?;
    Ok(finish(mesh, set, &layout.corner_vertex, &uv))
}

/// The middle of the box round some points.
pub(crate) fn middle(points: impl Iterator<Item = Vec2>) -> Vec2 {
    let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for p in points {
        low = low.min(p);
        high = high.max(p);
    }
    if low.x <= high.x { (low + high) * 0.5 } else { Vec2::ZERO }
}

pub(crate) fn islands_of(layout: &UvLayout, vertices: &[u32]) -> Vec<u32> {
    let mut out: Vec<u32> = vertices.iter().map(|&v| layout.uv_vertices[v as usize].island).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Refuse a new layout of these islands that turns over a triangle that
/// read with the rest of its island before, or flattens them.
///
/// A sliver - a CAD triangle with next to no area - can turn over by the
/// rounding of a move alone. Turned by less than a hair of its island
/// ([`super::flatten::FOLD_HAIR`]), before or after, it is not counted:
/// the fold rule does not count it either.
pub(crate) fn check_no_new_folds(layout: &UvLayout, islands: &[u32], uv: &[Vec2]) -> Result<(), UvError> {
    for &i in islands {
        let island = &layout.islands[i as usize];
        let corners = |t: u32, from: &dyn Fn(u32) -> Vec2| {
            let base = t as usize * 3;
            let [a, b, c] = [0, 1, 2].map(|k| from(layout.corner_vertex[base + k]));
            facing(a, b, c)
        };
        let before_uv = |v: u32| layout.uv_vertices[v as usize].uv;
        let after_uv = |v: u32| uv[v as usize];
        let before: Vec<f32> = island.triangles.iter().map(|&t| corners(t, &before_uv)).collect();
        let after: Vec<f32> = island.triangles.iter().map(|&t| corners(t, &after_uv)).collect();
        let (total_before, total_after): (f32, f32) = (before.iter().sum(), after.iter().sum());
        if !total_after.is_finite() {
            return Err(UvError::Degenerate);
        }
        // An island of slivers with no area to speak of has none to lose.
        if total_before == 0.0 {
            continue;
        }
        if total_after == 0.0 {
            return Err(UvError::Degenerate);
        }
        let (was, now) = (total_before.signum(), total_after.signum());
        let hair_before = total_before.abs() * super::flatten::FOLD_HAIR;
        let hair_after = total_after.abs() * super::flatten::FOLD_HAIR;
        for (b, a) in before.iter().zip(&after) {
            if *b * was > hair_before && *a * now < -hair_after {
                return Err(UvError::WouldFold);
            }
        }
    }
    Ok(())
}

/// What a straightening lines up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StraightenBy {
    /// Each island's own axis ([`super::Island::straightening_angle`]).
    Axis,
    /// An edge, named by the mesh vertices at its ends: its island turned
    /// so the edge lies square, and any other islands chosen by as much.
    Edge([u32; 2]),
}

/// Which way a straightened line lies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Alignment {
    /// Across the layout, along u.
    Horizontal,
    /// Down it, along v.
    Vertical,
    /// Whichever of the two is the smaller turn.
    #[default]
    Nearest,
}

impl Alignment {
    /// The turn that lays a line at `angle` this way, the smallest there is.
    pub fn turn_for(self, angle: f32) -> f32 {
        let quarter = std::f32::consts::FRAC_PI_2;
        match self {
            Alignment::Horizontal => -fold_angle(angle),
            Alignment::Vertical => -fold_angle(angle - quarter),
            Alignment::Nearest => {
                let a = fold_angle(angle);
                // Folded to a quarter turn either way of square.
                let off = if a > quarter * 0.5 { a - quarter } else if a <= -quarter * 0.5 { a + quarter } else { a };
                -off
            }
        }
    }
}

/// Islands turned about their middles so a line in each lies square to the
/// layout: by their own axes, or all by as much as an edge needs.
pub fn straighten(mesh: &Mesh, set: usize, islands: &[u32], by: StraightenBy, to: Alignment) -> Result<Mesh, UvError> {
    let layout = layout_of(mesh, set)?;
    let mut chosen = chosen_islands(&layout, islands)?;
    let mut uv: Vec<Vec2> = layout.uv_vertices.iter().map(|v| v.uv).collect();
    let edge_turn = match by {
        StraightenBy::Axis => None,
        StraightenBy::Edge(edge) => {
            let (island, direction) = edge_on_layout(&layout, edge)?;
            if !chosen.contains(&island) {
                chosen.push(island);
            }
            Some(to.turn_for(direction.y.atan2(direction.x)))
        }
    };
    if chosen.is_empty() {
        return Err(UvError::NothingChosen);
    }
    for &i in &chosen {
        let island = &layout.islands[i as usize];
        let turn = edge_turn.unwrap_or_else(|| to.turn_for(island.straightening_angle()));
        let map = UvAffine::rotation(turn).about(island.centre);
        for &v in &island.uv_vertices {
            uv[v as usize] = map.apply(uv[v as usize]);
        }
    }
    Ok(finish(mesh, set, &layout.corner_vertex, &uv))
}

/// The island an edge named by its mesh vertices is drawn on, and its
/// direction there: the side those vertices are on, or its first.
pub(crate) fn edge_on_layout(layout: &UvLayout, edge: [u32; 2]) -> Result<(u32, Vec2), UvError> {
    let sides = layout.edge_group(edge).ok_or(UvError::NoSuchEdge(edge))?;
    let sides = &layout.table.sides[sides];
    let side = sides.iter().find(|s| s.vertices == edge || s.vertices == [edge[1], edge[0]]).unwrap_or(&sides[0]);
    let (lo, hi) = (layout.corner_vertex[side.lo_corner()], layout.corner_vertex[side.hi_corner()]);
    let direction = layout.uv_vertices[hi as usize].uv - layout.uv_vertices[lo as usize].uv;
    if direction.length() <= 0.0 {
        return Err(UvError::Degenerate);
    }
    Ok((layout.island_of[side.triangle as usize], direction))
}

/// The density islands are evened to.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DensityTarget {
    /// The average of those chosen, by area: together they keep the room
    /// they take.
    #[default]
    Average,
    /// One island's own.
    Island(u32),
}

/// Islands scaled about their middles so each is as dense - texture to
/// surface - as the others, or as one island.
pub fn even_density(mesh: &Mesh, set: usize, islands: &[u32], target: DensityTarget) -> Result<Mesh, UvError> {
    let layout = layout_of(mesh, set)?;
    let chosen = chosen_islands(&layout, islands)?;
    if chosen.is_empty() {
        return Err(UvError::NothingChosen);
    }
    let density = match target {
        DensityTarget::Average => {
            let flat: f64 = chosen.iter().map(|&i| layout.islands[i as usize].uv_area as f64).sum();
            let surface: f64 = chosen.iter().map(|&i| layout.islands[i as usize].surface_area as f64).sum();
            if surface <= 0.0 {
                return Err(UvError::Degenerate);
            }
            (flat / surface) as f32
        }
        DensityTarget::Island(i) => layout.islands.get(i as usize).ok_or(UvError::NoSuchIsland(i))?.density(),
    };
    if density <= 0.0 || !density.is_finite() {
        return Err(UvError::Degenerate);
    }
    let mut uv: Vec<Vec2> = layout.uv_vertices.iter().map(|v| v.uv).collect();
    for &i in &chosen {
        let island = &layout.islands[i as usize];
        let own = island.density();
        if own <= 0.0 {
            continue;
        }
        let map = UvAffine::uniform((density / own).sqrt()).about(island.centre);
        for &v in &island.uv_vertices {
            uv[v as usize] = map.apply(uv[v as usize]);
        }
    }
    Ok(finish(mesh, set, &layout.corner_vertex, &uv))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;

    #[test]
    fn maps_compose_and_turn_about_their_pivot() {
        let turn = UvAffine::rotation(0.7).about(vec2(0.3, 0.4));
        assert!((turn.apply(vec2(0.3, 0.4)) - vec2(0.3, 0.4)).length() < 1e-6, "the pivot stays");
        let four = (0..4).fold(UvAffine::IDENTITY, |m, _| m.then(&UvAffine::quarter_turns(1)));
        assert_eq!(four, UvAffine::IDENTITY, "four quarter turns are exactly none");
        let moved = UvAffine::translation(vec2(1.0, 0.0)).then(&UvAffine::uniform(2.0));
        assert_eq!(moved.apply(vec2(0.0, 0.0)), vec2(2.0, 0.0), "moved, then scaled");
        assert!(UvAffine::flip_u().determinant() < 0.0);
    }

    #[test]
    fn moving_an_island_moves_it_whole_and_nothing_else() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let by = vec2(0.25, -0.125);
        let out = transform(&mesh, 0, &UvSelection::Islands(vec![2]), &UvAffine::translation(by), Pivot::Origin).unwrap();
        assert_eq!(out.positions.len(), mesh.positions.len(), "no vertex split");
        let after = UvLayout::new(&out, 0).unwrap();
        assert_eq!(after.islands.len(), 6);
        for (i, (a, b)) in layout.islands.iter().zip(&after.islands).enumerate() {
            let shift = if i == 2 { by } else { Vec2::ZERO };
            assert!((b.centre - a.centre - shift).length() < 1e-6, "island {i}");
            assert!((b.uv_area - a.uv_area).abs() < 1e-6);
        }
        assert!(after.folded_triangles().is_empty());
    }

    #[test]
    fn a_flip_mirrors_whole_islands_without_folding_them_and_twice_puts_them_back() {
        let mesh = unwrapped(cube());
        let all = UvSelection::Islands((0..6).collect());
        let once = transform(&mesh, 0, &all, &UvAffine::flip_u(), Pivot::Islands).unwrap();
        let layout = UvLayout::new(&once, 0).unwrap();
        assert!(layout.islands.iter().all(|i| i.mirrored));
        assert!(layout.folded_triangles().is_empty(), "mirrored whole is not folded");
        let twice = transform(&once, 0, &all, &UvAffine::flip_u(), Pivot::Islands).unwrap();
        for (a, b) in mesh.uv_sets[0].iter().zip(&twice.uv_sets[0]) {
            assert!((*a - *b).length() < 1e-6);
        }
    }

    #[test]
    fn dragging_one_vertex_across_its_triangle_is_refused_as_a_fold() {
        let mesh = unwrapped(sheet(4, 4, |_, _| 0.0));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        // An inner vertex, moved far past its neighbours.
        let inner = layout.uv_vertices.iter().position(|v| {
            let p = v.uv;
            (p - layout.islands[0].centre).length() < 1e-3
        });
        let vertex = layout.uv_vertices[inner.expect("a middle vertex")].vertex;
        let far = UvAffine::translation(vec2(0.6, 0.6));
        assert_eq!(transform(&mesh, 0, &UvSelection::Vertices(vec![vertex]), &far, Pivot::Origin), Err(UvError::WouldFold));
        // A small move is fine, and moves every corner on that point.
        let near = UvAffine::translation(vec2(0.01, 0.0));
        let out = transform(&mesh, 0, &UvSelection::Vertices(vec![vertex]), &near, Pivot::Origin).unwrap();
        let moved = out.uv_sets[0][vertex as usize];
        assert!((moved - layout.uv_vertices[inner.unwrap()].uv - vec2(0.01, 0.0)).length() < 1e-6);
        assert_eq!(transform(&mesh, 0, &UvSelection::Islands(vec![0]), &UvAffine::uniform(0.0), Pivot::Islands), Err(UvError::Degenerate));
    }

    /// An island of slivers laid out as a line has no area to lose, and is
    /// moved like any other.
    #[test]
    fn an_island_with_no_area_still_moves() {
        let mut mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        for &v in &layout.islands[0].vertices {
            let p = mesh.uv_sets[0][v as usize];
            mesh.uv_sets[0][v as usize] = vec2(p.x, 0.5);
        }
        let moved = transform(&mesh, 0, &UvSelection::Islands(vec![0]), &UvAffine::translation(vec2(0.1, 0.1)), Pivot::Origin).unwrap();
        let v = layout.islands[0].vertices[0] as usize;
        assert!((moved.uv_sets[0][v] - mesh.uv_sets[0][v] - vec2(0.1, 0.1)).length() < 1e-6);
    }

    #[test]
    fn straightening_lays_an_island_s_axis_or_an_edge_square() {
        let strip = sheet(2, 12, |_, _| 0.0);
        let turn = 0.4f32;
        let uv = strip.positions.iter().map(|p| UvAffine::rotation(turn).apply(vec2(p.x, p.z * 0.25))).collect();
        let mesh = strip.clone().with_uvs(uv);
        let out = straighten(&mesh, 0, &[0], StraightenBy::Axis, Alignment::Horizontal).unwrap();
        let island = &UvLayout::new(&out, 0).unwrap().islands[0];
        assert!(island.axis.abs() < 1e-3, "{}", island.axis);
        let out = straighten(&mesh, 0, &[0], StraightenBy::Axis, Alignment::Vertical).unwrap();
        let island = &UvLayout::new(&out, 0).unwrap().islands[0];
        assert!((island.axis.abs() - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "{}", island.axis);
        // By an edge: the first edge along the strip's length.
        let out = straighten(&mesh, 0, &[], StraightenBy::Edge([0, 1]), Alignment::Nearest).unwrap();
        let (a, b) = (out.uv_sets[0][0], out.uv_sets[0][1]);
        assert!((a.y - b.y).abs() < 1e-5, "{a:?} {b:?}");
        // Turned, not stretched: the area is what it was.
        let before = UvLayout::new(&mesh, 0).unwrap().islands[0].uv_area;
        assert!((island.uv_area - before).abs() < 1e-5);
    }

    #[test]
    fn evening_density_gives_islands_one_density() {
        let mesh = unwrapped(cube());
        // Make island 1 twice as large across.
        let big = transform(&mesh, 0, &UvSelection::Islands(vec![1]), &UvAffine::uniform(2.0), Pivot::Islands).unwrap();
        let layout = UvLayout::new(&big, 0).unwrap();
        assert!(layout.units_per_uv.is_none(), "not uniform any more");
        let even = even_density(&big, 0, &(0..6).collect::<Vec<_>>(), DensityTarget::Island(0)).unwrap();
        let layout = UvLayout::new(&even, 0).unwrap();
        let first = layout.islands[0].density();
        for island in &layout.islands {
            assert!((island.density() / first - 1.0).abs() < 1e-4);
        }
        assert!(layout.units_per_uv.is_some());
        // By the average, the total area is kept.
        let total: f32 = UvLayout::new(&big, 0).unwrap().islands.iter().map(|i| i.uv_area).sum();
        let averaged = even_density(&big, 0, &(0..6).collect::<Vec<_>>(), DensityTarget::Average).unwrap();
        let after: f32 = UvLayout::new(&averaged, 0).unwrap().islands.iter().map(|i| i.uv_area).sum();
        assert!((after - total).abs() < 1e-4);
    }
}
