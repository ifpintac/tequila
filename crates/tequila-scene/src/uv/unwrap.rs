//! A whole part unwrapped, in three steps, each the usual one:
//!
//! 1. **Charts.** Triangles are grown into charts while they face within a
//!    cone of the chart's own direction and do not cross a sharp crease, so
//!    each chart is a patch of surface that lies out flat without folding:
//!    a CAD part's faces come apart at their edges, a smooth one into a few
//!    patches. Or, respecting its cuts, the islands of the layout it has.
//! 2. **Flattening.** Each chart by least-squares conformal maps
//!    ([`super::flatten`]). A chart that does not settle, or folds, is laid
//!    out by its projection instead.
//! 3. **Packing.** Each chart scaled to its true area, so the texture is as
//!    dense everywhere, squared to its tightest box and packed into the unit
//!    square with a margin between charts ([`super::pack`]).
//!
//! The mesh comes back with its vertices split along the charts' seams and
//! the layout as its first UV set; the sets it had move up one, kept.

use super::flatten::{conformal, folds, project, to_true_area};
use super::layout::{weld, UvLayout};
use super::pack::{arrange, PackOptions};
use crate::geometry::Mesh;
use std::collections::HashMap;
use tequila_core::math::{Vec2, Vec3};

/// How a part is unwrapped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnwrapOptions {
    /// How far a triangle may turn from its chart's direction, in degrees:
    /// the cone a chart is grown within.
    pub cone_degrees: f32,
    /// How sharp a crease, in degrees, always separates two charts.
    pub crease_degrees: f32,
    /// The gap between charts, as a fraction of the square's side.
    pub margin: f32,
    /// Keep the cuts the part's first layout has: its islands are the
    /// charts, each laid out flat again whole, and no cut is added but where
    /// an island will not lie flat without one. A part with no layout
    /// unwraps as usual.
    pub respect_cuts: bool,
}

impl Default for UnwrapOptions {
    fn default() -> Self {
        // Blender's Smart UV Project's angle, which splits a box into its
        // sides and a cylinder into a handful of strips.
        UnwrapOptions { cone_degrees: 66.0, crease_degrees: 60.0, margin: 0.005, respect_cuts: false }
    }
}

/// What an unwrap did, for the report.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnwrapReport {
    pub charts: usize,
    /// Charts that were laid out by projection, not conformally.
    pub projected: usize,
    /// How much of the square the charts cover, 0 to 1.
    pub coverage: f32,
}

/// The mesh unwrapped: split along its charts' seams, the layout its UV set
/// 0, its other sets after it, its tangents made again for the new layout.
pub fn unwrap(mesh: &Mesh, options: &UnwrapOptions) -> (Mesh, UnwrapReport) {
    let triangles = mesh.triangle_count();
    let weld = weld(&mesh.positions);
    let normals: Vec<Vec3> = (0..triangles)
        .map(|t| {
            let [a, b, c] = mesh.triangle(t);
            let (pa, pb, pc) = (mesh.positions[a as usize], mesh.positions[b as usize], mesh.positions[c as usize]);
            (pb - pa).cross(pc - pa)
        })
        .collect();
    // What a corner's vertex is in a chart's layout: its welded vertex, so a
    // chart stays one piece across the splits a mesh has for its normals -
    // or, keeping the cuts, its UV vertex, so a cut that does not part an
    // island stays open in it.
    let existing = if options.respect_cuts && !mesh.uv_sets.is_empty() { UvLayout::new(mesh, 0) } else { None };
    let keys: Vec<u32> = match &existing {
        Some(layout) => layout.corner_vertex.clone(),
        None => mesh.indices.iter().map(|&v| weld[v as usize]).collect(),
    };
    let edges = edge_faces(mesh, &weld);
    let mut queue: Vec<Chart> = match &existing {
        Some(layout) => layout.islands.iter().map(|i| Chart { triangles: i.triangles.iter().map(|&t| t as usize).collect(), depth: 0, kept: true }).collect(),
        None => grow_charts(mesh, &weld, &normals, options, None),
    };
    // A conformal layout keeps angles, not areas: a chart it shrinks far
    // more in one place than another - a cushion, a dome - is split between
    // the two ways it turns and each half laid out again, until the texture
    // is near enough as dense all over it. A kept island is kept whole
    // unless it folds, when it is cut by the usual rule within it.
    queue.reverse();
    let mut charts = Vec::new();
    let mut layouts: Vec<Layout> = Vec::new();
    while let Some(chart) = queue.pop() {
        let layout = flatten(mesh, &keys, &normals, &chart);
        if chart.kept && layout.projected && chart.triangles.len() > 1 {
            let pieces = grow_charts(mesh, &weld, &normals, options, Some(&chart.triangles));
            if pieces.len() > 1 {
                queue.extend(pieces);
                continue;
            }
        }
        if !chart.kept && chart.triangles.len() >= 8 && chart.depth < 10 && spread(mesh, &keys, &chart, &layout) > MOST_SPREAD {
            let halves = split(mesh, &weld, &normals, &edges, &chart);
            if halves.len() > 1 {
                queue.extend(halves);
                continue;
            }
        }
        charts.push(chart);
        layouts.push(layout);
    }
    let projected = layouts.iter().filter(|l| l.projected).count();
    let coverage = pack(&mut layouts, options.margin);

    // A vertex for each chart a vertex of the mesh is in, and each place in
    // that chart's layout.
    let mut out = mesh.clone();
    let mut remap: HashMap<(usize, u32, u32), u32> = HashMap::new();
    let mut positions = Vec::new();
    let mut normals_out = Vec::new();
    let mut colors = Vec::new();
    let mut sets: Vec<Vec<Vec2>> = vec![Vec::new(); mesh.uv_sets.len() + 1];
    let mut indices = vec![0u32; mesh.indices.len()];
    for (chart_index, (chart, layout)) in charts.iter().zip(&layouts).enumerate() {
        for &t in &chart.triangles {
            for corner in 0..3 {
                let c = t * 3 + corner;
                let vertex = mesh.indices[c];
                let key = (chart_index, vertex, keys[c]);
                let next = positions.len() as u32;
                let index = *remap.entry(key).or_insert_with(|| {
                    let v = vertex as usize;
                    positions.push(mesh.positions[v]);
                    if !mesh.normals.is_empty() {
                        normals_out.push(mesh.normals[v]);
                    }
                    if !mesh.colors.is_empty() {
                        colors.push(mesh.colors[v]);
                    }
                    sets[0].push(layout.uv[&keys[c]]);
                    for (set, uvs) in mesh.uv_sets.iter().enumerate() {
                        sets[set + 1].push(uvs.get(v).copied().unwrap_or(Vec2::ZERO));
                    }
                    next
                });
                indices[c] = index;
            }
        }
    }
    out.positions = positions;
    out.normals = normals_out;
    out.colors = colors;
    out.uv_sets = sets;
    out.indices = indices;
    out.tangents = Vec::new();
    out.recompute_bounds();
    // The tangent frame follows the layout; a mesh that had none needs one
    // now only if a normal map asks, which mikktspace makes from this.
    out.generate_tangents();
    (out, UnwrapReport { charts: charts.len(), projected, coverage })
}

struct Chart {
    triangles: Vec<usize>,
    /// How many times it has been split from the chart it was grown as.
    depth: u32,
    /// An island of the layout the part had, kept whole.
    kept: bool,
}

/// How much more one part of a chart's layout may be scaled than another,
/// in area, between its least and most scaled tenths: twice as far in
/// length, about as far as a texture can be told apart by eye.
const MOST_SPREAD: f32 = 4.0;

/// The triangles each welded edge is between.
fn edge_faces(mesh: &Mesh, weld: &[u32]) -> HashMap<(u32, u32), Vec<usize>> {
    let mut edges: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for t in 0..mesh.triangle_count() {
        let [a, b, c] = mesh.triangle(t).map(|v| weld[v as usize]);
        for (p, q) in [(a, b), (b, c), (c, a)] {
            if p != q {
                edges.entry((p.min(q), p.max(q))).or_default().push(t);
            }
        }
    }
    edges
}

/// How unevenly a layout scales its chart: the ratio of the area scale of
/// its most scaled tenth to its least, by area.
fn spread(mesh: &Mesh, keys: &[u32], chart: &Chart, layout: &Layout) -> f32 {
    let mut scales: Vec<(f32, f32)> = Vec::with_capacity(chart.triangles.len());
    for &t in &chart.triangles {
        let [a, b, c] = mesh.triangle(t).map(|v| v as usize);
        let surface = (mesh.positions[b] - mesh.positions[a]).cross(mesh.positions[c] - mesh.positions[a]).length() * 0.5;
        if surface <= 0.0 {
            continue;
        }
        let [ua, ub, uc] = [0, 1, 2].map(|k| layout.uv[&keys[t * 3 + k]]);
        let flat = ((ub.x - ua.x) * (uc.y - ua.y) - (ub.y - ua.y) * (uc.x - ua.x)).abs() * 0.5;
        scales.push((flat / surface, surface));
    }
    if scales.len() < 2 {
        return 1.0;
    }
    scales.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f32 = scales.iter().map(|(_, area)| area).sum();
    let at = |fraction: f32| {
        let mut sum = 0.0;
        for (scale, area) in &scales {
            sum += area;
            if sum >= total * fraction {
                return *scale;
            }
        }
        scales.last().map(|(scale, _)| *scale).unwrap_or(1.0)
    };
    let (low, high) = (at(0.1), at(0.9));
    if low <= 0.0 {
        f32::INFINITY
    } else {
        high / low
    }
}

/// A chart in two or more: its triangles divided between the two ways it
/// faces furthest apart, each side's connected pieces a chart of its own.
fn split(mesh: &Mesh, weld: &[u32], normals: &[Vec3], edges: &HashMap<(u32, u32), Vec<usize>>, chart: &Chart) -> Vec<Chart> {
    let unit = |t: usize| if normals[t].length() > 0.0 { normals[t].normalize() } else { Vec3::ZERO };
    let mean = chart.triangles.iter().fold(Vec3::ZERO, |sum, &t| sum + normals[t]);
    let mean = if mean.length() > 0.0 { mean.normalize() } else { Vec3::Y };
    let Some(&first) = chart.triangles.iter().min_by(|&&a, &&b| unit(a).dot(mean).total_cmp(&unit(b).dot(mean))) else {
        return Vec::new();
    };
    let Some(&second) = chart.triangles.iter().min_by(|&&a, &&b| unit(a).dot(unit(first)).total_cmp(&unit(b).dot(unit(first)))) else {
        return Vec::new();
    };
    let (toward, away) = (unit(first), unit(second));
    let side: HashMap<usize, bool> = if toward.dot(away) < 0.999 {
        chart.triangles.iter().map(|&t| (t, unit(t).dot(toward) >= unit(t).dot(away))).collect()
    } else {
        // Facing all one way: halved along its length instead.
        let centre = |t: usize| {
            let [a, b, c] = mesh.triangle(t).map(|v| mesh.positions[v as usize]);
            (a + b + c) / 3.0
        };
        let centres: Vec<f32> = {
            let points: Vec<Vec3> = chart.triangles.iter().map(|&t| centre(t)).collect();
            let low = points.iter().fold(Vec3::splat(f32::INFINITY), |a, p| a.min(*p));
            let high = points.iter().fold(Vec3::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
            let span = high - low;
            let axis = if span.x >= span.y && span.x >= span.z { Vec3::X } else if span.y >= span.z { Vec3::Y } else { Vec3::Z };
            points.iter().map(|p| p.dot(axis)).collect()
        };
        let mut sorted = centres.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let middle = sorted[sorted.len() / 2];
        chart.triangles.iter().zip(&centres).map(|(&t, c)| (t, *c < middle)).collect()
    };
    // Each side's connected pieces.
    let mut seen: HashMap<usize, bool> = HashMap::new();
    let mut pieces = Vec::new();
    for &seed in &chart.triangles {
        if seen.contains_key(&seed) {
            continue;
        }
        let group = side[&seed];
        seen.insert(seed, true);
        let mut members = vec![seed];
        let mut front = vec![seed];
        while let Some(t) = front.pop() {
            let [a, b, c] = mesh.triangle(t).map(|v| weld[v as usize]);
            for (p, q) in [(a, b), (b, c), (c, a)] {
                let Some(across) = edges.get(&(p.min(q), p.max(q))) else { continue };
                for &n in across {
                    if side.get(&n) == Some(&group) && !seen.contains_key(&n) {
                        seen.insert(n, true);
                        members.push(n);
                        front.push(n);
                    }
                }
            }
        }
        pieces.push(Chart { triangles: members, depth: chart.depth + 1, kept: false });
    }
    pieces
}

/// Triangles grown into charts from seeds, largest first, each taking its
/// neighbours while they face within the cone of the chart's direction and
/// across no sharp crease; all of them, or those of `within`.
fn grow_charts(mesh: &Mesh, weld: &[u32], normals: &[Vec3], options: &UnwrapOptions, within: Option<&[usize]>) -> Vec<Chart> {
    let triangles = mesh.triangle_count();
    let allowed: Vec<bool> = match within {
        Some(list) => {
            let mut allowed = vec![false; triangles];
            list.iter().for_each(|&t| allowed[t] = true);
            allowed
        }
        None => vec![true; triangles],
    };
    let mut neighbours: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for t in (0..triangles).filter(|&t| allowed[t]) {
        let [a, b, c] = mesh.triangle(t).map(|v| weld[v as usize]);
        for (p, q) in [(a, b), (b, c), (c, a)] {
            if p != q {
                neighbours.entry((p.min(q), p.max(q))).or_default().push(t);
            }
        }
    }
    let cone = options.cone_degrees.to_radians().cos();
    let crease = options.crease_degrees.to_radians().cos();
    let unit: Vec<Vec3> = normals.iter().map(|n| if n.length() > 0.0 { n.normalize() } else { Vec3::ZERO }).collect();
    let mut order: Vec<usize> = (0..triangles).filter(|&t| allowed[t]).collect();
    order.sort_by(|&a, &b| normals[b].length().total_cmp(&normals[a].length()));
    let mut owner = vec![usize::MAX; triangles];
    let mut charts = Vec::new();
    for seed in order {
        if owner[seed] != usize::MAX {
            continue;
        }
        let id = charts.len();
        owner[seed] = id;
        let mut direction = normals[seed];
        let mut members = vec![seed];
        let mut front = vec![seed];
        while let Some(t) = front.pop() {
            let [a, b, c] = mesh.triangle(t).map(|v| weld[v as usize]);
            for (p, q) in [(a, b), (b, c), (c, a)] {
                let Some(across) = neighbours.get(&(p.min(q), p.max(q))) else { continue };
                // An edge of more than two faces is not a surface to cross.
                if across.len() != 2 {
                    continue;
                }
                for &n in across {
                    if owner[n] != usize::MAX {
                        continue;
                    }
                    let heading = if direction.length() > 0.0 { direction.normalize() } else { unit[n] };
                    let degenerate = unit[n] == Vec3::ZERO;
                    if degenerate || (unit[n].dot(heading) >= cone && unit[n].dot(unit[t]) >= crease) {
                        owner[n] = id;
                        direction = direction + normals[n];
                        members.push(n);
                        front.push(n);
                    }
                }
            }
        }
        charts.push(Chart { triangles: members, depth: 0, kept: false });
    }
    charts
}

/// A chart's layout: each of its vertices' place, by key, in units of the
/// part's own length, so area on the layout is area on the part.
struct Layout {
    uv: HashMap<u32, Vec2>,
    /// Its area on the layout, before packing: its area on the part.
    area: f32,
    /// Laid out by projection: conformal flattening did not settle or folded.
    projected: bool,
}

/// One chart laid out flat: conformally where that settles without folding,
/// else projected along its direction.
fn flatten(mesh: &Mesh, keys: &[u32], normals: &[Vec3], chart: &Chart) -> Layout {
    let mut local: HashMap<u32, usize> = HashMap::new();
    let mut points: Vec<Vec3> = Vec::new();
    let mut faces: Vec<[usize; 3]> = Vec::new();
    let mut direction = Vec3::ZERO;
    for &t in &chart.triangles {
        let corners = [0, 1, 2].map(|k| {
            let c = t * 3 + k;
            *local.entry(keys[c]).or_insert_with(|| {
                points.push(mesh.positions[mesh.indices[c] as usize]);
                points.len() - 1
            })
        });
        direction = direction + normals[t];
        faces.push(corners);
    }
    let projected = project(&points, direction);
    let flat = if faces.len() > 1 { conformal(&points, &faces, &projected, &[]) } else { None };
    let (uv, projected_only) = match flat {
        Some(uv) if !folds(&uv, &faces, Some(true)) => (uv, false),
        _ => (projected, true),
    };
    let uv = to_true_area(&points, &faces, uv);
    let area = super::flatten::areas(&points, &faces, &uv).1 as f32;
    let by_key = local.iter().map(|(&key, &i)| (key, uv[i])).collect();
    Layout { uv: by_key, area, projected: projected_only }
}

/// Every chart squared to its tightest box and packed into the unit square,
/// `margin` apart; how much of the square they cover.
fn pack(layouts: &mut [Layout], margin: f32) -> f32 {
    if layouts.is_empty() {
        return 0.0;
    }
    // In a fixed order, so the same part always packs the same way.
    let keys: Vec<Vec<u32>> = layouts.iter().map(|l| {
        let mut keys: Vec<u32> = l.uv.keys().copied().collect();
        keys.sort_unstable();
        keys
    }).collect();
    let shapes: Vec<Vec<Vec2>> = layouts.iter().zip(&keys).map(|(l, k)| k.iter().map(|key| l.uv[key]).collect()).collect();
    let areas: Vec<f32> = layouts.iter().map(|l| l.area).collect();
    let options = PackOptions { margin, rotate: true, normalize: false, orient: true, ..Default::default() };
    let Ok((maps, report)) = arrange(&shapes, &areas, &vec![1.0; shapes.len()], &options) else {
        return 0.0;
    };
    for (layout, map) in layouts.iter_mut().zip(&maps) {
        for p in layout.uv.values_mut() {
            *p = map.apply(*p);
        }
    }
    report.coverage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::flatten::facing;
    use crate::uv::tests_support::*;
    use tequila_core::math::{vec2, vec3};

    fn quad_mesh(corners: [Vec3; 4]) -> Mesh {
        Mesh::new(corners.to_vec(), vec![0, 1, 2, 0, 2, 3])
    }

    /// Every triangle's area on the layout against on the part, as the
    /// largest ratio of one to the other's mean: 1 is a perfect layout.
    fn stretch(mesh: &Mesh) -> f32 {
        let uv = &mesh.uv_sets[0];
        let mut ratios = Vec::new();
        for t in 0..mesh.triangle_count() {
            let [a, b, c] = mesh.triangle(t).map(|v| v as usize);
            let surface = (mesh.positions[b] - mesh.positions[a]).cross(mesh.positions[c] - mesh.positions[a]).length();
            let flat = ((uv[b].x - uv[a].x) * (uv[c].y - uv[a].y) - (uv[b].y - uv[a].y) * (uv[c].x - uv[a].x)).abs();
            if surface > 1e-9 {
                ratios.push(flat / surface);
            }
        }
        let mean = ratios.iter().sum::<f32>() / ratios.len() as f32;
        ratios.iter().map(|r| (r / mean).max(mean / r)).fold(1.0, f32::max)
    }

    fn inside_the_square(mesh: &Mesh) -> bool {
        mesh.uv_sets[0].iter().all(|p| (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y))
    }

    #[test]
    fn a_flat_square_lies_out_as_itself() {
        let mesh = quad_mesh([vec3(0.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0), vec3(2.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)]);
        let (out, report) = unwrap(&mesh, &UnwrapOptions::default());
        assert_eq!(report.charts, 1);
        assert_eq!(report.projected, 0);
        assert!(inside_the_square(&out));
        assert!(stretch(&out) < 1.001);
        // Twice as long as it is high, on the layout as on the part.
        let uv = &out.uv_sets[0];
        let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for p in uv {
            low = low.min(*p);
            high = high.max(*p);
        }
        let size = high - low;
        assert!((size.x / size.y - 2.0).abs() < 1e-3, "{size:?}");
    }

    /// Seen from the front of a face, the layout's u runs across and its v
    /// down, as the picture's do: artwork placed square on the layout
    /// prints the right way round, not mirrored.
    #[test]
    fn artwork_on_an_unwrapped_face_reads_the_right_way_round() {
        let mesh = quad_mesh([vec3(0.0, 0.0, 0.0), vec3(2.0, 0.0, 0.0), vec3(2.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)]);
        let (out, _) = unwrap(&mesh, &UnwrapOptions::default());
        for t in 0..out.triangle_count() {
            let [a, b, c] = out.triangle(t).map(|v| out.uv_sets[0][v as usize]);
            assert!(facing(a, b, c) > 0.0, "triangle {t} is mirrored");
        }
        for mesh in [cube(), sheet(6, 9, |x, z| (x * 3.0).sin() * 0.2 + z * 0.1)] {
            let (out, _) = unwrap(&mesh, &UnwrapOptions::default());
            let layout = UvLayout::new(&out, 0).unwrap();
            assert!(layout.islands.iter().all(|i| !i.mirrored));
            assert!(layout.folded_triangles().is_empty());
        }
    }

    #[test]
    fn a_box_comes_apart_into_its_six_sides_without_overlap() {
        let (out, report) = unwrap(&cube(), &UnwrapOptions::default());
        assert_eq!(report.charts, 6);
        assert!(inside_the_square(&out));
        assert!(stretch(&out) < 1.001, "every side as dense as the others");
        // Each side's box apart from every other's.
        let layout = UvLayout::new(&out, 0).unwrap();
        assert_eq!(layout.islands.len(), 6);
        for (i, a) in layout.islands.iter().enumerate() {
            for (j, b) in layout.islands.iter().enumerate().skip(i + 1) {
                let apart = a.max.x <= b.min.x + 1e-6 || b.max.x <= a.min.x + 1e-6 || a.max.y <= b.min.y + 1e-6 || b.max.y <= a.min.y + 1e-6;
                assert!(apart, "sides {i} and {j} overlap: {a:?} {b:?}");
            }
        }
        assert!(report.coverage > 0.5, "packed tightly enough: {}", report.coverage);
        // The shape is the part's: every corner where it was.
        for t in 0..out.triangle_count() {
            let corners = out.triangle(t).map(|v| out.positions[v as usize]);
            let original = cube().triangle(t).map(|v| cube().positions[v as usize]);
            assert_eq!(corners, original);
        }
    }

    #[test]
    fn a_curved_sheet_lies_flat_with_little_stretch() {
        // A quarter of a cylinder's side: curved one way, so a projection
        // squashes its far edges and the conformal layout does not.
        let mesh = sheet(8, 16, |x, _| (1.0 - (x - 0.5) * (x - 0.5) * 2.0).sqrt() * 0.5);
        let (out, report) = unwrap(&mesh, &UnwrapOptions::default());
        assert_eq!(report.projected, 0, "laid out conformally");
        assert!(inside_the_square(&out));
        assert!(stretch(&out) < 1.05, "stretch {}", stretch(&out));
    }

    #[test]
    fn the_sets_a_part_had_are_kept_after_the_new_one() {
        let mut mesh = cube();
        mesh.uv_sets = vec![(0..24).map(|i| vec2(i as f32, 0.5)).collect()];
        let (out, _) = unwrap(&mesh, &UnwrapOptions::default());
        assert_eq!(out.uv_sets.len(), 2);
        // The old set, carried through the split vertices unchanged.
        for t in 0..out.triangle_count() {
            let before = mesh.triangle(t).map(|v| mesh.uv_sets[0][v as usize]);
            let after = out.triangle(t).map(|v| out.uv_sets[1][v as usize]);
            assert_eq!(before, after);
        }
    }

    /// A tube cut once along its length by hand: unwrapped keeping its
    /// cuts, it lies out as one strip, where the usual unwrap makes several.
    #[test]
    fn respecting_cuts_keeps_the_islands_the_part_has() {
        let (segments, rings) = (24, 6);
        let tube = tube(segments, rings);
        // One island: the tube opened at its doubled column, laid out as a
        // strip - squashed, so there is something to lay out again.
        let strip: Vec<Vec2> = (0..tube.positions.len()).map(|i| {
            let (s, r) = (i % (segments + 1), i / (segments + 1));
            vec2(s as f32 / segments as f32, r as f32 / rings as f32 * 0.5)
        }).collect();
        let mut cut = tube.clone();
        cut.uv_sets = vec![strip];
        assert_eq!(UvLayout::new(&cut, 0).unwrap().islands.len(), 1);
        let (_, usual) = unwrap(&cut, &UnwrapOptions::default());
        assert!(usual.charts > 1, "the usual unwrap cuts a tube into strips");
        let (kept, report) = unwrap(&cut, &UnwrapOptions { respect_cuts: true, ..Default::default() });
        assert_eq!(report.charts, 1, "one strip, as it was cut");
        assert_eq!(report.projected, 0);
        let layout = UvLayout::new(&kept, 0).unwrap();
        assert_eq!(layout.islands.len(), 1);
        assert!(layout.folded_triangles().is_empty());
        assert!(stretch(&kept) < 1.05, "a cylinder opened is flat: {}", stretch(&kept));
        // A tube with no cut at all, its layout one point: it lies flat as
        // a ring, or is cut where it will not, but it never folds.
        let mut closed = tube.clone();
        closed.uv_sets = vec![vec![vec2(0.5, 0.5); closed.positions.len()]];
        let (out, report) = unwrap(&closed, &UnwrapOptions { respect_cuts: true, ..Default::default() });
        let folded = UvLayout::new(&out, 0).unwrap().folded_triangles();
        assert!(folded.is_empty(), "{report:?}: {} folded of {}", folded.len(), out.triangle_count());
    }
}
