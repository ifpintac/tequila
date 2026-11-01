//! Islands arranged: each moved, turned or scaled whole by a map of its own
//! - spread evenly, sized alike, fitted to the square, swapped about, turned
//! upright as the part stands, set to a texel density - and all of it one
//! edit, [`place_islands`].
//!
//! What each tool works out is a [`Plan`]: the islands and the map each is
//! to be moved by, read from the layout as it is. The plan is what the
//! command records (`uv.place_islands`), so a script, the journal and a redo
//! say the same arrangement again exactly, whatever chose it.

use super::edit::{chosen_islands, finish, layout_of, UvError};
use super::transform::{check_no_new_folds, UvAffine};
use super::{UvLayout, UvRect};
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2, Vec3};

/// Islands, and the map each is moved by.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub islands: Vec<u32>,
    pub maps: Vec<UvAffine>,
}

impl Plan {
    pub fn push(&mut self, island: u32, map: UvAffine) {
        self.islands.push(island);
        self.maps.push(map);
    }

    pub fn is_empty(&self) -> bool {
        self.islands.is_empty()
    }

    /// The plan without the islands it leaves where they are.
    pub fn moving(self) -> Plan {
        let mut out = Plan::default();
        for (island, map) in self.islands.into_iter().zip(self.maps) {
            if !is_still(&map) {
                out.push(island, map);
            }
        }
        out
    }

    /// Where an island's box middle goes.
    pub fn centre_of(&self, layout: &UvLayout, island: u32) -> Option<Vec2> {
        let at = self.islands.iter().position(|&i| i == island)?;
        Some(self.maps[at].apply(layout.islands[island as usize].centre))
    }
}

/// Whether a map moves nothing worth an edit.
pub fn is_still(map: &UvAffine) -> bool {
    map.m.iter().zip(UvAffine::IDENTITY.m.iter()).all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-9))
}

/// Each island moved whole by its own map. Refused where an island is named
/// twice or not at all, the lists differ in length, or a map flattens its
/// island or is not a number; a map that mirrors an island mirrors it whole,
/// which is no fold.
pub fn place_islands(mesh: &Mesh, set: usize, islands: &[u32], maps: &[UvAffine]) -> Result<Mesh, UvError> {
    let layout = layout_of(mesh, set)?;
    if islands.is_empty() {
        return Err(UvError::NothingChosen);
    }
    if islands.len() != maps.len() {
        return Err(UvError::Degenerate);
    }
    let checked = chosen_islands(&layout, islands)?;
    if checked.len() != islands.len() {
        return Err(UvError::Degenerate);
    }
    if maps.iter().any(|m| !m.is_finite() || m.determinant().abs() <= f32::MIN_POSITIVE) {
        return Err(UvError::Degenerate);
    }
    let mut uv: Vec<Vec2> = layout.uv_vertices.iter().map(|v| v.uv).collect();
    for (&island, map) in islands.iter().zip(maps) {
        for &v in &layout.islands[island as usize].uv_vertices {
            uv[v as usize] = map.apply(uv[v as usize]);
        }
    }
    check_no_new_folds(&layout, &checked, &uv)?;
    Ok(finish(mesh, set, &layout.corner_vertex, &uv))
}

/// The box round some islands.
pub fn box_of(layout: &UvLayout, islands: &[u32]) -> Option<(Vec2, Vec2)> {
    islands.iter().map(|&i| &layout.islands[i as usize]).map(|i| (i.min, i.max)).reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
}

/// How a spread spaces the islands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Spread {
    /// The gaps between neighbouring boxes equal.
    #[default]
    Gaps,
    /// The middles of their boxes equally far apart.
    Centres,
}

/// Islands spread across (`across`) or down between the two outermost,
/// which stay where they are: neighbours by where each box's middle is.
pub fn distribute(layout: &UvLayout, islands: &[u32], across: bool, by: Spread) -> Plan {
    let mut plan = Plan::default();
    if islands.len() < 3 {
        return plan;
    }
    let along = |v: Vec2| if across { v.x } else { v.y };
    let mut order: Vec<u32> = islands.to_vec();
    order.sort_by(|&a, &b| along(layout.islands[a as usize].centre).total_cmp(&along(layout.islands[b as usize].centre)).then(a.cmp(&b)));
    let island = |i: u32| &layout.islands[i as usize];
    let (first, last) = (island(order[0]), island(*order.last().expect("three or more")));
    let n = order.len();
    let shifts: Vec<f32> = match by {
        Spread::Gaps => {
            let span = along(last.max) - along(first.min);
            let widths: f32 = order.iter().map(|&i| along(island(i).max) - along(island(i).min)).sum();
            let gap = (span - widths) / (n - 1) as f32;
            let mut at = along(first.min);
            order
                .iter()
                .map(|&i| {
                    let shift = at - along(island(i).min);
                    at += along(island(i).max) - along(island(i).min) + gap;
                    shift
                })
                .collect()
        }
        Spread::Centres => {
            let (a, b) = (along(first.centre), along(last.centre));
            order.iter().enumerate().map(|(k, &i)| a + (b - a) * k as f32 / (n - 1) as f32 - along(island(i).centre)).collect()
        }
    };
    for (&i, shift) in order.iter().zip(shifts) {
        plan.push(i, UvAffine::translation(if across { vec2(shift, 0.0) } else { vec2(0.0, shift) }));
    }
    plan.moving()
}

/// Which size the others are matched to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SizeTarget {
    /// One island's own.
    Island(u32),
    #[default]
    Largest,
    Average,
}

/// Islands scaled about their middles to the same width (`across`) or
/// height as `to`: both ways with `keep_proportions`, else only that way.
pub fn match_size(layout: &UvLayout, islands: &[u32], to: SizeTarget, across: bool, keep_proportions: bool) -> Result<Plan, UvError> {
    let along = |i: u32| {
        let island = &layout.islands[i as usize];
        let size = island.max - island.min;
        if across { size.x } else { size.y }
    };
    let target = match to {
        SizeTarget::Island(i) => {
            if i as usize >= layout.islands.len() {
                return Err(UvError::NoSuchIsland(i));
            }
            along(i)
        }
        SizeTarget::Largest => islands.iter().map(|&i| along(i)).fold(0.0, f32::max),
        SizeTarget::Average => islands.iter().map(|&i| along(i)).sum::<f32>() / islands.len().max(1) as f32,
    };
    if !(target > 1e-9) {
        return Err(UvError::Degenerate);
    }
    let mut plan = Plan::default();
    for &i in islands {
        let own = along(i);
        if own <= 1e-9 {
            continue;
        }
        let k = target / own;
        let scale = if keep_proportions { Vec2::splat(k) } else if across { vec2(k, 1.0) } else { vec2(1.0, k) };
        plan.push(i, UvAffine::scale(scale).about(layout.islands[i as usize].centre));
    }
    Ok(plan.moving())
}

/// Islands scaled, keeping their proportions, and moved to fill `region`
/// with `margin` inside its edges: together as one, or each on its own.
pub fn fit(layout: &UvLayout, islands: &[u32], region: UvRect, margin: f32, each: bool) -> Result<Plan, UvError> {
    let room = region.size() - Vec2::splat(margin * 2.0);
    if room.x <= 0.0 || room.y <= 0.0 {
        return Err(UvError::NoRoom);
    }
    let middle = (region.min + region.max) * 0.5;
    let fitted = |low: Vec2, high: Vec2| -> Option<UvAffine> {
        let size = high - low;
        if size.x <= 1e-9 && size.y <= 1e-9 {
            return None;
        }
        let k = (room.x / size.x.max(1e-9)).min(room.y / size.y.max(1e-9));
        let centre = (low + high) * 0.5;
        Some(UvAffine::uniform(k).about(centre).then(&UvAffine::translation(middle - centre)))
    };
    let mut plan = Plan::default();
    if each {
        for &i in islands {
            let island = &layout.islands[i as usize];
            if let Some(map) = fitted(island.min, island.max) {
                plan.push(i, map);
            }
        }
    } else {
        let (low, high) = box_of(layout, islands).ok_or(UvError::NothingChosen)?;
        let map = fitted(low, high).ok_or(UvError::Degenerate)?;
        for &i in islands {
            plan.push(i, map);
        }
    }
    Ok(plan.moving())
}

/// A small, quick, well spread random number generator: the same seed
/// gives the same numbers, so an arrangement made from one is made again.
#[derive(Debug)]
pub struct Seeded(u64);

impl Seeded {
    pub fn new(seed: u64) -> Seeded {
        Seeded(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    pub fn next(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A whole number below `n`.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Islands moved into each other's places, by the middles of their boxes:
/// two swap; more go round in an order `seed` draws, every one of them to
/// another's place (Sattolo's shuffle).
pub fn shuffle(layout: &UvLayout, islands: &[u32], seed: u64) -> Plan {
    let mut plan = Plan::default();
    let n = islands.len();
    if n < 2 {
        return plan;
    }
    let mut to: Vec<usize> = (0..n).collect();
    let mut random = Seeded::new(seed);
    for i in (1..n).rev() {
        let j = random.below(i);
        to.swap(i, j);
    }
    for (k, &i) in islands.iter().enumerate() {
        let from = layout.islands[i as usize].centre;
        let target = layout.islands[islands[to[k]] as usize].centre;
        plan.push(i, UvAffine::translation(target - from));
    }
    plan.moving()
}

/// Each island's direction on the layout of a direction on the part, `up`
/// in the part's own space: through each triangle's map from the part to
/// the layout, weighted by the triangle's area on the part. With how
/// strongly it shows, 0 to 1: near 0 where the island faces along `up` and
/// has no up of its own to speak of.
pub fn direction_on_layout(layout: &UvLayout, mesh: &Mesh, island: u32, up: Vec3) -> (Vec2, f32) {
    let (mut sum, mut scale) = (Vec2::ZERO, 0.0f32);
    for &t in &layout.islands[island as usize].triangles {
        let [a, b, c] = mesh.triangle(t as usize).map(|v| mesh.positions[v as usize]);
        let [qa, qb, qc] = layout.triangle_uv(t);
        let (e1, e2) = (b - a, c - a);
        let area = e1.cross(e2).length() * 0.5;
        if area <= 0.0 {
            continue;
        }
        // The map M with M e1 = f1 and M e2 = f2, applied to `up` through
        // the triangle's plane: M = F (EᵀE)⁻¹ Eᵀ.
        let (f1, f2) = (qb - qa, qc - qa);
        let (g11, g12, g22) = (e1.dot(e1), e1.dot(e2), e2.dot(e2));
        let det = g11 * g22 - g12 * g12;
        if det.abs() <= f32::MIN_POSITIVE {
            continue;
        }
        let (r1, r2) = (e1.dot(up), e2.dot(up));
        let (s1, s2) = ((g22 * r1 - g12 * r2) / det, (g11 * r2 - g12 * r1) / det);
        sum += (f1 * s1 + f2 * s2) * area;
        let flat = super::flatten::facing(qa, qb, qc).abs();
        scale += (flat / area).sqrt() * area * up.length();
    }
    if scale <= 0.0 {
        return (Vec2::ZERO, 0.0);
    }
    (sum, (sum.length() / scale).min(1.0))
}

/// Islands turned about their middles so `up`, a direction on the part in
/// its own space, points up the layout as the editor shows it (toward
/// lower v). With the islands left as they were because they face along
/// `up` - a top or a bottom - and have no up to turn to.
pub fn upright(layout: &UvLayout, mesh: &Mesh, islands: &[u32], up: Vec3) -> (Plan, Vec<u32>) {
    let mut plan = Plan::default();
    let mut flat = Vec::new();
    for &i in islands {
        let (direction, strength) = direction_on_layout(layout, mesh, i, up);
        if strength < 0.2 {
            flat.push(i);
            continue;
        }
        let turn = -std::f32::consts::FRAC_PI_2 - direction.y.atan2(direction.x);
        plan.push(i, UvAffine::rotation(turn).about(layout.islands[i as usize].centre));
    }
    (plan.moving(), flat)
}

/// An island's texel density as a length: layout units per metre of the
/// part across it, the square root of its area on the layout over its area
/// on the part.
pub fn linear_density(layout: &UvLayout, islands: &[u32]) -> Option<f32> {
    let flat: f64 = islands.iter().map(|&i| layout.islands[i as usize].uv_area as f64).sum();
    let surface: f64 = islands.iter().map(|&i| layout.islands[i as usize].surface_area as f64).sum();
    (surface > 0.0 && flat > 0.0).then(|| (flat / surface).sqrt() as f32)
}

/// Islands scaled about their middles so each has `density` layout units
/// per metre of the part.
pub fn set_density(layout: &UvLayout, islands: &[u32], density: f32) -> Result<Plan, UvError> {
    if !(density.is_finite() && density > 0.0) {
        return Err(UvError::Degenerate);
    }
    let mut plan = Plan::default();
    for &i in islands {
        let Some(own) = linear_density(layout, &[i]) else { continue };
        plan.push(i, UvAffine::uniform(density / own).about(layout.islands[i as usize].centre));
    }
    if plan.is_empty() {
        return Err(UvError::Degenerate);
    }
    Ok(plan.moving())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::UvSelection;

    fn moved(mesh: &Mesh, plan: &Plan) -> UvLayout {
        UvLayout::new(&place_islands(mesh, 0, &plan.islands, &plan.maps).unwrap(), 0).unwrap()
    }

    #[test]
    fn islands_are_placed_each_by_its_own_map_in_one_edit() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let plan = Plan { islands: vec![1, 4], maps: vec![UvAffine::translation(vec2(0.1, 0.0)), UvAffine::quarter_turns(1).about(layout.islands[4].centre)] };
        let after = moved(&mesh, &plan);
        assert!((after.islands[1].centre - layout.islands[1].centre - vec2(0.1, 0.0)).length() < 1e-5);
        assert!((after.islands[4].centre - layout.islands[4].centre).length() < 1e-5, "turned about its middle");
        assert!((after.islands[0].centre - layout.islands[0].centre).length() < 1e-7, "the rest stay");
        assert_eq!(place_islands(&mesh, 0, &[1, 1], &[UvAffine::IDENTITY; 2]), Err(UvError::Degenerate), "an island named twice");
        assert_eq!(place_islands(&mesh, 0, &[1], &[]), Err(UvError::Degenerate));
        assert_eq!(place_islands(&mesh, 0, &[9], &[UvAffine::IDENTITY]), Err(UvError::NoSuchIsland(9)));
        assert_eq!(place_islands(&mesh, 0, &[2], &[UvAffine::uniform(0.0)]), Err(UvError::Degenerate));
        let mirrored = place_islands(&mesh, 0, &[2], &[UvAffine::flip_u().about(layout.islands[2].centre)]).unwrap();
        assert!(UvLayout::new(&mirrored, 0).unwrap().islands[2].mirrored, "mirrored whole, not folded");
    }

    #[test]
    fn a_spread_evens_the_gaps_or_the_middles_between_the_outermost() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let all: Vec<u32> = (0..6).collect();
        for by in [Spread::Gaps, Spread::Centres] {
            let plan = distribute(&layout, &all, true, by);
            let after = moved(&mesh, &plan);
            let mut order = all.clone();
            order.sort_by(|&a, &b| after.islands[a as usize].centre.x.total_cmp(&after.islands[b as usize].centre.x));
            let first = layout.islands.iter().map(|i| i.centre.x).fold(f32::INFINITY, f32::min);
            assert!((after.islands[order[0] as usize].centre.x - first).abs() < 1e-5, "the first stays");
            let steps: Vec<f32> = order
                .windows(2)
                .map(|w| match by {
                    Spread::Gaps => after.islands[w[1] as usize].min.x - after.islands[w[0] as usize].max.x,
                    Spread::Centres => after.islands[w[1] as usize].centre.x - after.islands[w[0] as usize].centre.x,
                })
                .collect();
            assert!(steps.iter().all(|s| (s - steps[0]).abs() < 1e-4), "{by:?}: {steps:?}");
        }
        assert!(distribute(&layout, &[0, 1], true, Spread::Gaps).is_empty(), "two have nothing between them");
    }

    #[test]
    fn sizes_match_the_largest_one_way_or_both() {
        let mesh = unwrapped(cube());
        let big = crate::uv::transform(&mesh, 0, &UvSelection::Islands(vec![3]), &UvAffine::scale(vec2(2.0, 1.5)), crate::uv::Pivot::Islands).unwrap();
        let layout = UvLayout::new(&big, 0).unwrap();
        let width = |l: &UvLayout, i: usize| l.islands[i].max.x - l.islands[i].min.x;
        let height = |l: &UvLayout, i: usize| l.islands[i].max.y - l.islands[i].min.y;
        let all: Vec<u32> = (0..6).collect();
        let plan = match_size(&layout, &all, SizeTarget::Largest, true, true).unwrap();
        let after = moved(&big, &plan);
        for i in 0..6 {
            assert!((width(&after, i) - width(&layout, 3)).abs() < 1e-4);
        }
        assert!((height(&after, 0) / width(&after, 0) - height(&layout, 0) / width(&layout, 0)).abs() < 1e-4, "proportions kept");
        let plan = match_size(&layout, &all, SizeTarget::Island(3), false, false).unwrap();
        let after = moved(&big, &plan);
        assert!((height(&after, 0) - height(&layout, 3)).abs() < 1e-4 && (width(&after, 0) - width(&layout, 0)).abs() < 1e-5, "only down");
    }

    #[test]
    fn a_fit_fills_the_square_together_or_each_alone() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let after = moved(&mesh, &fit(&layout, &[0, 1], UvRect::UNIT, 0.0, false).unwrap());
        let (low, high) = box_of(&after, &[0, 1]).unwrap();
        assert!((low.x.min(low.y)).abs() < 1e-4 || (high.x.max(high.y) - 1.0).abs() < 1e-4, "touches the square: {low:?} {high:?}");
        assert!(low.x >= -1e-4 && low.y >= -1e-4 && high.x <= 1.0 + 1e-4 && high.y <= 1.0 + 1e-4);
        assert!(((low + high) * 0.5 - Vec2::splat(0.5)).length() < 1e-4, "centred");
        let each = moved(&mesh, &fit(&layout, &[0, 1], UvRect::UNIT, 0.05, true).unwrap());
        for i in [0, 1] {
            let size = each.islands[i].max - each.islands[i].min;
            assert!((size.x.max(size.y) - 0.9).abs() < 1e-4, "{size:?}");
        }
    }

    #[test]
    fn a_shuffle_takes_every_island_to_another_s_place_the_same_way_each_time() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let all: Vec<u32> = (0..6).collect();
        let plan = shuffle(&layout, &all, 7);
        assert_eq!(plan, shuffle(&layout, &all, 7), "the same seed, the same order");
        let after = moved(&mesh, &plan);
        let before: Vec<Vec2> = layout.islands.iter().map(|i| i.centre).collect();
        for (i, island) in after.islands.iter().enumerate() {
            assert!((island.centre - before[i]).length() > 1e-4, "island {i} moved");
            assert!(before.iter().any(|c| (*c - island.centre).length() < 1e-5), "into another's place");
        }
        let two = moved(&mesh, &shuffle(&layout, &[0, 5], 1));
        assert!((two.islands[0].centre - before[5]).length() < 1e-5 && (two.islands[5].centre - before[0]).length() < 1e-5, "two swap");
    }

    /// A tube stood up along Y, laid out on its side: upright turns it so
    /// the part's up runs up the layout.
    #[test]
    fn upright_turns_the_part_s_up_to_the_layout_s() {
        let mesh = unwrapped(tube(16, 4));
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let all: Vec<u32> = (0..layout.islands.len() as u32).collect();
        let turned = crate::uv::transform(&mesh, 0, &UvSelection::Islands(all.clone()), &UvAffine::rotation(1.1), crate::uv::Pivot::Islands).unwrap();
        let layout = UvLayout::new(&turned, 0).unwrap();
        let (plan, flat) = upright(&layout, &turned, &all, Vec3::Y);
        assert!(flat.is_empty());
        let out = place_islands(&turned, 0, &plan.islands, &plan.maps).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        for i in 0..after.islands.len() as u32 {
            let (d, strength) = direction_on_layout(&after, &out, i, Vec3::Y);
            assert!(strength > 0.5, "{strength}");
            let d = d.normalize();
            assert!((d - vec2(0.0, -1.0)).length() < 1e-3, "island {i}: up is {d:?}");
        }
        // A box's top faces up: it has no up of its own, and stays.
        let cube = unwrapped(cube());
        let layout = UvLayout::new(&cube, 0).unwrap();
        let (_, flat) = upright(&layout, &cube, &(0..6).collect::<Vec<_>>(), Vec3::Y);
        assert_eq!(flat.len(), 2, "the top and the bottom");
    }

    #[test]
    fn a_density_is_read_and_set() {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let own = linear_density(&layout, &[0]).unwrap();
        let plan = set_density(&layout, &[0, 1, 2], own * 2.0).unwrap();
        let after = moved(&mesh, &plan);
        for i in 0..3 {
            assert!((linear_density(&after, &[i]).unwrap() / own - 2.0).abs() < 1e-3);
            assert!((after.islands[i as usize].centre - layout.islands[i as usize].centre).length() < 1e-5, "about its middle");
        }
        assert!(set_density(&layout, &[0], 0.0).is_err());
    }
}
