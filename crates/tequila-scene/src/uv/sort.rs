//! Islands put in order and laid out by it - in a row, a column or a grid,
//! by where they are on the part or on the layout - and laid out as they
//! sit on the part, seen from its front, so the layout reads like the
//! product.
//!
//! Where an island is on the part is the middle of its area there. Which
//! way is which on the part is given as the world's axes in the part's own
//! space ([`PartAxes`]), so a part turned in the scene sorts by where it
//! stands, not by how its file was drawn.

use super::arrange::{box_of, Plan};
use super::similar::{classes, IslandShape};
use super::transform::UvAffine;
use super::UvLayout;
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2, Vec3};

/// The world's X, Y (up) and Z (toward the front) in the part's own space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PartAxes(pub [Vec3; 3]);

impl Default for PartAxes {
    fn default() -> Self {
        PartAxes([Vec3::X, Vec3::Y, Vec3::Z])
    }
}

impl PartAxes {
    fn along(&self, axis: usize, p: Vec3) -> f32 {
        p.dot(self.0[axis.min(2)])
    }
}

/// Each island's middle on the part: the middle of its area there.
pub fn centres_on_part(layout: &UvLayout, mesh: &Mesh, islands: &[u32]) -> Vec<Vec3> {
    islands
        .iter()
        .map(|&i| {
            let (mut sum, mut total) = (Vec3::ZERO, 0.0f32);
            for &t in &layout.islands[i as usize].triangles {
                let [a, b, c] = mesh.triangle(t as usize).map(|v| mesh.positions[v as usize]);
                let area = super::flatten::surface_area(a, b, c);
                sum += (a + b + c) * (area / 3.0);
                total += area;
            }
            if total > 0.0 {
                sum / total
            } else {
                let island = &layout.islands[i as usize];
                island.vertices.iter().map(|&v| mesh.positions[v as usize]).fold(Vec3::ZERO, |s, p| s + p) / island.vertices.len().max(1) as f32
            }
        })
        .collect()
}

/// What islands are put in order by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SortBy {
    /// Where they are along one of the part's axes, 0 to 2: X and Z from
    /// low to high, Y from the top down, so a column reads as the part
    /// stands.
    Axis(usize),
    /// Their angle round the part's middle about an axis, starting after
    /// the widest gap: a band's, a strap's or a ring's pieces in turn.
    Around(usize),
    /// Rows by one axis (from the top down where it is Y), each row by
    /// another: a part of tiles, as it is laid.
    Grid { rows: usize, columns: usize },
    /// Where they are on the layout now: rows by v, each by u - a tidy-up.
    Layout,
}

/// Where a row or a column breaks onto the next.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Wrap {
    #[default]
    Never,
    /// After so many islands.
    After(usize),
    /// At the layout's edge: the rows fill the square's width, or the
    /// columns its height.
    Width,
}

/// What islands are gathered in before they are put in order: each group
/// in rows of its own, a gap apart.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Grouping {
    #[default]
    None,
    /// Islands of about the same size on the layout.
    Size,
    /// Islands of the same shape ([`super::similar`]), mirrored pieces too.
    Shape,
    /// Islands near each other on the part.
    Closeness,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SortOptions {
    pub by: SortBy,
    /// A row across the layout, or a column down it.
    pub across: bool,
    /// Where islands of different sizes line up across a row: -1 its top
    /// (a column's left), 0 its middle, 1 its bottom.
    pub align: i8,
    /// The gap between islands, in the layout's units.
    pub spacing: f32,
    pub wrap: Wrap,
    pub group: Grouping,
    /// The gap between groups.
    pub group_spacing: f32,
    pub reverse: bool,
}

impl Default for SortOptions {
    fn default() -> Self {
        SortOptions { by: SortBy::Layout, across: true, align: 0, spacing: 0.01, wrap: Wrap::Never, group: Grouping::None, group_spacing: 0.04, reverse: false }
    }
}

/// The islands' rows: in order, as `by` puts them.
fn ordered(layout: &UvLayout, mesh: &Mesh, islands: &[u32], by: SortBy, axes: &PartAxes) -> Vec<Vec<u32>> {
    let centres = centres_on_part(layout, mesh, islands);
    let at = |i: u32| centres[islands.iter().position(|&x| x == i).expect("one of them")];
    let mut order: Vec<u32> = islands.to_vec();
    let key = |axis: usize, p: Vec3| if axis == 1 { -axes.along(1, p) } else { axes.along(axis, p) };
    match by {
        SortBy::Axis(axis) => order.sort_by(|&a, &b| key(axis, at(a)).total_cmp(&key(axis, at(b))).then(a.cmp(&b))),
        SortBy::Around(axis) => {
            let middle = centres.iter().fold(Vec3::ZERO, |s, p| s + *p) / centres.len().max(1) as f32;
            let (p, q) = ((axis + 1) % 3, (axis + 2) % 3);
            let angle = |i: u32| {
                let d = at(i) - middle;
                axes.along(q, d).atan2(axes.along(p, d))
            };
            order.sort_by(|&a, &b| angle(a).total_cmp(&angle(b)).then(a.cmp(&b)));
            // Begin after the widest gap, so a ring open somewhere reads
            // from one end to the other.
            if order.len() > 2 {
                let tau = std::f32::consts::TAU;
                let gaps: Vec<f32> = (0..order.len()).map(|k| (angle(order[(k + 1) % order.len()]) - angle(order[k])).rem_euclid(tau)).collect();
                let widest = (0..gaps.len()).max_by(|&a, &b| gaps[a].total_cmp(&gaps[b])).unwrap_or(0);
                let count = order.len();
                order.rotate_left((widest + 1) % count);
            }
        }
        SortBy::Grid { rows, columns } => {
            // Rows of islands whose spans along the row axis overlap.
            let span = |i: u32| {
                let island = &layout.islands[i as usize];
                let values = island.vertices.iter().map(|&v| key(rows, mesh.positions[v as usize]));
                values.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)))
            };
            order.sort_by(|&a, &b| key(rows, at(a)).total_cmp(&key(rows, at(b))).then(a.cmp(&b)));
            let mut out: Vec<(f32, Vec<u32>)> = Vec::new();
            for i in order {
                let middle = key(rows, at(i));
                match out.last_mut() {
                    Some((reach, row)) if middle <= *reach => {
                        row.push(i);
                        *reach = reach.max(span(i).1);
                    }
                    _ => out.push((span(i).1, vec![i])),
                }
            }
            return out
                .into_iter()
                .map(|(_, mut row)| {
                    row.sort_by(|&a, &b| key(columns, at(a)).total_cmp(&key(columns, at(b))).then(a.cmp(&b)));
                    row
                })
                .collect();
        }
        SortBy::Layout => {
            let centre = |i: u32| layout.islands[i as usize].centre;
            order.sort_by(|&a, &b| centre(a).y.total_cmp(&centre(b).y).then(centre(a).x.total_cmp(&centre(b).x)).then(a.cmp(&b)));
            // Rows of islands whose spans down the layout overlap.
            let mut rows: Vec<(f32, Vec<u32>)> = Vec::new();
            for i in order {
                let island = &layout.islands[i as usize];
                match rows.last_mut() {
                    Some((reach, row)) if island.centre.y <= *reach => {
                        row.push(i);
                        *reach = reach.max(island.max.y);
                    }
                    _ => rows.push((island.max.y, vec![i])),
                }
            }
            return rows
                .into_iter()
                .map(|(_, mut row)| {
                    row.sort_by(|&a, &b| centre(a).x.total_cmp(&centre(b).x).then(a.cmp(&b)));
                    row
                })
                .collect();
        }
    }
    vec![order]
}

/// The islands gathered as `grouping` says, the groups in the order of
/// their first island in `order`.
fn grouped(layout: &UvLayout, mesh: &Mesh, islands: &[u32], grouping: Grouping) -> Vec<Vec<u32>> {
    match grouping {
        Grouping::None => vec![islands.to_vec()],
        Grouping::Size => {
            let mut by_size: Vec<u32> = islands.to_vec();
            by_size.sort_by(|&a, &b| layout.islands[b as usize].uv_area.total_cmp(&layout.islands[a as usize].uv_area).then(a.cmp(&b)));
            let mut out: Vec<Vec<u32>> = Vec::new();
            for i in by_size {
                let area = layout.islands[i as usize].uv_area;
                match out.last_mut() {
                    Some(group) if area >= layout.islands[group[0] as usize].uv_area * 0.6 => group.push(i),
                    _ => out.push(vec![i]),
                }
            }
            out
        }
        Grouping::Shape => {
            let shapes: Vec<(u32, IslandShape)> = islands.iter().map(|&i| (i, IslandShape::of(layout, mesh, i))).collect();
            classes(&shapes, 0.05, true)
        }
        Grouping::Closeness => {
            // Islands closer than half again the usual distance to their
            // nearest neighbour, one group, and so on through them.
            let centres = centres_on_part(layout, mesh, islands);
            let n = islands.len();
            let nearest: Vec<f32> = (0..n).map(|a| (0..n).filter(|&b| b != a).map(|b| (centres[a] - centres[b]).length()).fold(f32::INFINITY, f32::min)).collect();
            let mut sorted: Vec<f32> = nearest.iter().copied().filter(|d| d.is_finite()).collect();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let reach = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0) * 1.5;
            let mut sets = super::layout::UnionFind::new(n);
            for a in 0..n {
                for b in a + 1..n {
                    if (centres[a] - centres[b]).length() <= reach {
                        sets.union(a, b);
                    }
                }
            }
            let mut out: Vec<(usize, Vec<u32>)> = Vec::new();
            for (k, &i) in islands.iter().enumerate() {
                let root = sets.find(k);
                match out.iter_mut().find(|(r, _)| *r == root) {
                    Some((_, list)) => list.push(i),
                    None => out.push((root, vec![i])),
                }
            }
            out.into_iter().map(|(_, list)| list).collect()
        }
    }
}

/// Islands laid out in order: in rows across the layout (or columns down
/// it), from the top left of the box round them - from the square's left
/// where they wrap at its width - each a gap from the last.
pub fn sort(layout: &UvLayout, mesh: &Mesh, islands: &[u32], options: &SortOptions, axes: &PartAxes) -> Plan {
    let mut plan = Plan::default();
    let Some((low, _)) = box_of(layout, islands) else { return plan };
    let groups = grouped(layout, mesh, islands, options.group);
    let mut lines: Vec<(Vec<u32>, bool)> = Vec::new();
    for group in groups {
        let mut rows = ordered(layout, mesh, &group, options.by, axes);
        if options.reverse {
            rows.reverse();
            rows.iter_mut().for_each(|r| r.reverse());
        }
        let mut first = true;
        for row in rows {
            lines.push((row, first));
            first = false;
        }
    }
    let size = |i: u32| {
        let island = &layout.islands[i as usize];
        let s = island.max - island.min;
        if options.across { s } else { vec2(s.y, s.x) }
    };
    let gap = options.spacing.max(0.0);
    // Along a line is x, across the lines y, a column's turned so.
    let low = if options.across { low } else { vec2(low.y, low.x) };
    let start = vec2(if options.wrap == Wrap::Width { gap * 0.5 } else { low.x }, low.y);
    let limit = 1.0 - gap * 0.5;
    let mut places: Vec<(u32, Vec2)> = Vec::new();
    let mut y = start.y;
    let mut first_line = true;
    for (line, starts_group) in lines {
        // The line broken where it wraps.
        let mut pieces: Vec<Vec<u32>> = Vec::new();
        let mut piece: Vec<u32> = Vec::new();
        let mut x = start.x;
        for i in line {
            let w = size(i).x;
            let full = match options.wrap {
                Wrap::Never => false,
                Wrap::After(n) => piece.len() >= n.max(1),
                Wrap::Width => !piece.is_empty() && x + w > limit,
            };
            if full {
                pieces.push(std::mem::take(&mut piece));
                x = start.x;
            }
            piece.push(i);
            x += w + gap;
        }
        if !piece.is_empty() {
            pieces.push(piece);
        }
        for (k, piece) in pieces.into_iter().enumerate() {
            if !first_line {
                y += if starts_group && k == 0 && options.group != Grouping::None { options.group_spacing.max(gap) } else { gap };
            }
            first_line = false;
            let height = piece.iter().map(|&i| size(i).y).fold(0.0, f32::max);
            let mut x = start.x;
            for i in piece {
                let s = size(i);
                let top = match options.align {
                    a if a < 0 => y,
                    0 => y + (height - s.y) * 0.5,
                    _ => y + height - s.y,
                };
                places.push((i, vec2(x, top) + s * 0.5));
                x += s.x + gap;
            }
            y += height;
        }
    }
    for (i, middle) in places {
        let middle = if options.across { middle } else { vec2(middle.y, middle.x) };
        plan.push(i, UvAffine::translation(middle - layout.islands[i as usize].centre));
    }
    plan.moving()
}

/// Which way the part is seen from to lay its islands out as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Front,
    /// From its right.
    Side,
    /// From above.
    Top,
    /// From whichever of the three spreads the islands most.
    Auto,
}

/// Where a point of the part is in a view: across and down it.
fn seen(view: View, axes: &PartAxes, p: Vec3) -> Vec2 {
    let (x, y, z) = (axes.along(0, p), axes.along(1, p), axes.along(2, p));
    match view {
        View::Front | View::Auto => vec2(x, -y),
        View::Side => vec2(-z, -y),
        View::Top => vec2(x, z),
    }
}

/// The view whose depth the islands spread along least: what sees them
/// most spread out.
pub fn widest_view(centres: &[Vec3], axes: &PartAxes) -> View {
    let n = centres.len().max(1) as f32;
    let variance = |axis: usize| {
        let mean = centres.iter().map(|p| axes.along(axis, *p)).sum::<f32>() / n;
        centres.iter().map(|p| (axes.along(axis, *p) - mean).powi(2)).sum::<f32>() / n
    };
    let [x, y, z] = [variance(0), variance(1), variance(2)];
    if z <= x && z <= y {
        View::Front
    } else if x <= y {
        View::Side
    } else {
        View::Top
    }
}

/// Islands laid out as they sit on the part, seen from `view`: each moved,
/// not turned or scaled, so its middle is where its middle on the part is
/// seen, at the layout's own scale (`density`, layout units to a metre of
/// the part); then pushed apart, as little as they must, until no two
/// boxes overlap or come closer than `gap`; the whole kept where the box
/// round them was.
pub fn as_on_part(layout: &UvLayout, mesh: &Mesh, islands: &[u32], view: View, axes: &PartAxes, density: f32, gap: f32) -> Plan {
    let mut plan = Plan::default();
    let Some((low, high)) = box_of(layout, islands) else { return plan };
    let centres = centres_on_part(layout, mesh, islands);
    let view = if view == View::Auto { widest_view(&centres, axes) } else { view };
    let half: Vec<Vec2> = islands.iter().map(|&i| (layout.islands[i as usize].max - layout.islands[i as usize].min) * 0.5 + Vec2::splat(gap * 0.5)).collect();
    let mut at: Vec<Vec2> = centres.iter().map(|p| seen(view, axes, *p) * density).collect();
    let n = islands.len();
    // Apart, each overlapping pair by half the overlap each, the shorter
    // way, until none overlap.
    for _ in 0..500 {
        let mut moved = false;
        for a in 0..n {
            for b in a + 1..n {
                let d = at[b] - at[a];
                let overlap = half[a] + half[b] - vec2(d.x.abs(), d.y.abs());
                if overlap.x <= 1e-7 || overlap.y <= 1e-7 {
                    continue;
                }
                moved = true;
                let push = if overlap.x < overlap.y {
                    vec2(if d.x > 0.0 || (d.x == 0.0 && a < b) { overlap.x } else { -overlap.x }, 0.0)
                } else {
                    vec2(0.0, if d.y > 0.0 || (d.y == 0.0 && a < b) { overlap.y } else { -overlap.y })
                } * 0.5;
                at[a] -= push * 1.0001;
                at[b] += push * 1.0001;
            }
        }
        if !moved {
            break;
        }
    }
    // Kept where the box round them was.
    let (mut lo, mut hi) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
    for (p, h) in at.iter().zip(&half) {
        lo = lo.min(*p - *h);
        hi = hi.max(*p + *h);
    }
    let shift = (low + high) * 0.5 - (lo + hi) * 0.5;
    for (k, &i) in islands.iter().enumerate() {
        plan.push(i, UvAffine::translation(at[k] + shift - layout.islands[i as usize].centre));
    }
    plan.moving()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::arrange::place_islands;
    use crate::uv::tests_support::*;

    fn moved(mesh: &Mesh, plan: &Plan) -> UvLayout {
        UvLayout::new(&place_islands(mesh, 0, &plan.islands, &plan.maps).unwrap(), 0).unwrap()
    }

    /// A box's six sides, unwrapped and packed as the unwrap leaves them.
    fn boxed() -> (Mesh, UvLayout) {
        let mesh = unwrapped(cube());
        let layout = UvLayout::new(&mesh, 0).unwrap();
        (mesh, layout)
    }

    #[test]
    fn islands_sort_into_a_row_by_where_they_are_on_the_part() {
        let (mesh, layout) = boxed();
        let all: Vec<u32> = (0..6).collect();
        let options = SortOptions { by: SortBy::Axis(0), spacing: 0.02, ..Default::default() };
        let plan = sort(&layout, &mesh, &all, &options, &PartAxes::default());
        let after = moved(&mesh, &plan);
        let centres = centres_on_part(&after, &mesh, &all);
        let mut order = all.clone();
        order.sort_by(|&a, &b| after.islands[a as usize].centre.x.total_cmp(&after.islands[b as usize].centre.x));
        let xs: Vec<f32> = order.iter().map(|&i| centres[i as usize].x).collect();
        assert!(xs.windows(2).all(|w| w[0] <= w[1] + 1e-5), "left to right as on the part: {xs:?}");
        // One row, a gap apart, middles level.
        for w in order.windows(2) {
            let gap = after.islands[w[1] as usize].min.x - after.islands[w[0] as usize].max.x;
            assert!((gap - 0.02).abs() < 1e-4, "{gap}");
            assert!((after.islands[w[1] as usize].centre.y - after.islands[w[0] as usize].centre.y).abs() < 1e-4);
        }
        // Wrapped after two: three rows.
        let wrapped = sort(&layout, &mesh, &all, &SortOptions { wrap: Wrap::After(2), ..options }, &PartAxes::default());
        let after = moved(&mesh, &wrapped);
        let mut rows: Vec<f32> = after.islands.iter().map(|i| (i.centre.y * 1000.0).round()).collect();
        rows.sort_by(|a, b| a.total_cmp(b));
        rows.dedup();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn a_column_reads_from_the_top_of_the_part_down_and_reverses() {
        let (mesh, layout) = boxed();
        let all: Vec<u32> = (0..6).collect();
        let options = SortOptions { by: SortBy::Axis(1), across: false, ..Default::default() };
        let after = moved(&mesh, &sort(&layout, &mesh, &all, &options, &PartAxes::default()));
        let centres = centres_on_part(&after, &mesh, &all);
        let mut order = all.clone();
        order.sort_by(|&a, &b| after.islands[a as usize].centre.y.total_cmp(&after.islands[b as usize].centre.y));
        assert!((centres[order[0] as usize].y - 1.0).abs() < 1e-4, "the top first");
        assert!(centres[order[5] as usize].y.abs() < 1e-4, "the bottom last");
        let reversed = moved(&mesh, &sort(&layout, &mesh, &all, &SortOptions { reverse: true, ..options }, &PartAxes::default()));
        let top = (0..6).min_by(|&a, &b| reversed.islands[a].centre.y.total_cmp(&reversed.islands[b].centre.y)).unwrap();
        assert!(centres_on_part(&reversed, &mesh, &[top as u32])[0].y.abs() < 1e-4, "reversed, the bottom first");
    }

    /// Eight tiles of a band round Y: sorted round, they come in turn.
    #[test]
    fn a_band_sorts_round_its_axis() {
        let mesh = tube(8, 1);
        // Each of the eight faces its own island, laid out in a jumble.
        let mut positions = Vec::new();
        let mut uv = Vec::new();
        let mut indices = Vec::new();
        let order = [5, 2, 7, 0, 3, 6, 1, 4];
        for (k, &s) in order.iter().enumerate() {
            let at = |r: usize, s: usize| mesh.positions[r * 9 + s];
            let base = positions.len() as u32;
            positions.extend_from_slice(&[at(0, s), at(1, s), at(1, s + 1), at(0, s + 1)]);
            let x = 0.05 + k as f32 * 0.11;
            uv.extend_from_slice(&[vec2(x, 0.5), vec2(x, 0.4), vec2(x + 0.1, 0.4), vec2(x + 0.1, 0.5)]);
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        let mesh = Mesh::new(positions, indices).with_uvs(uv);
        let layout = UvLayout::new(&mesh, 0).unwrap();
        assert_eq!(layout.islands.len(), 8);
        let all: Vec<u32> = (0..8).collect();
        let plan = sort(&layout, &mesh, &all, &SortOptions { by: SortBy::Around(1), ..Default::default() }, &PartAxes::default());
        let after = moved(&mesh, &plan);
        let mut by_u = all.clone();
        by_u.sort_by(|&a, &b| after.islands[a as usize].centre.x.total_cmp(&after.islands[b as usize].centre.x));
        let centres = centres_on_part(&after, &mesh, &by_u);
        let angles: Vec<f32> = centres.iter().map(|p| p.z.atan2(p.x)).collect();
        let steps: Vec<f32> = angles.windows(2).map(|w| (w[1] - w[0]).rem_euclid(std::f32::consts::TAU)).collect();
        let eighth = std::f32::consts::TAU / 8.0;
        let step = if (steps[0] - eighth).abs() < 1e-3 { eighth } else { std::f32::consts::TAU - eighth };
        assert!(steps.iter().all(|s| (s - step).abs() < 1e-3), "in turn round the band, one way: {steps:?}");
    }

    #[test]
    fn a_tidy_up_keeps_rows_and_groups_go_apart() {
        let (mesh, layout) = boxed();
        let all: Vec<u32> = (0..6).collect();
        let tidy = moved(&mesh, &sort(&layout, &mesh, &all, &SortOptions { by: SortBy::Layout, wrap: Wrap::Width, ..Default::default() }, &PartAxes::default()));
        for island in &tidy.islands {
            assert!(island.min.x >= 0.0 && island.max.x <= 1.0, "within the square's width");
        }
        // Grouped by shape, the six sides of a box are one piece.
        let grouped = grouped(&layout, &mesh, &all, Grouping::Shape);
        assert_eq!(grouped.len(), 1);
        let sizes = grouped_sizes(&layout, &mesh);
        assert_eq!(sizes, 1, "six sides alike in size");
    }

    fn grouped_sizes(layout: &UvLayout, mesh: &Mesh) -> usize {
        grouped(layout, mesh, &(0..6).collect::<Vec<_>>(), Grouping::Size).len()
    }

    #[test]
    fn islands_lay_out_as_on_the_part_and_do_not_overlap() {
        let (mesh, layout) = boxed();
        let all: Vec<u32> = (0..6).collect();
        let density = crate::uv::arrange::linear_density(&layout, &all).unwrap();
        let plan = as_on_part(&layout, &mesh, &all, View::Front, &PartAxes::default(), density, 0.01);
        let after = moved(&mesh, &plan);
        for a in 0..6 {
            for b in a + 1..6 {
                let (p, q) = (&after.islands[a], &after.islands[b]);
                let apart = p.max.x <= q.min.x + 1e-4 || q.max.x <= p.min.x + 1e-4 || p.max.y <= q.min.y + 1e-4 || q.max.y <= p.min.y + 1e-4;
                assert!(apart, "islands {a} and {b} overlap");
            }
        }
        // The top of the box is above its bottom on the layout, as seen.
        let centres = centres_on_part(&after, &mesh, &all);
        let top = (0..6).find(|&i| (centres[i].y - 1.0).abs() < 1e-4).unwrap();
        let bottom = (0..6).find(|&i| centres[i].y.abs() < 1e-4).unwrap();
        assert!(after.islands[top].centre.y < after.islands[bottom].centre.y, "the top above");
        assert_eq!(widest_view(&[vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.1), vec3(0.0, 1.0, 0.0)], &PartAxes::default()), View::Front);
        assert_eq!(widest_view(&[vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 1.0), vec3(0.1, 0.0, 1.0)], &PartAxes::default()), View::Side);
    }

    use tequila_core::math::vec3;
}
