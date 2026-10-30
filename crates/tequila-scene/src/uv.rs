//! UV unwrapping: a part's surface laid out flat, so artwork can be placed
//! on it anywhere - a label on an angled face, a silkscreen round a
//! fillet - where a planar projection can only look down an axis.
//!
//! In three steps, each the usual one:
//!
//! 1. **Charts.** Triangles are grown into charts while they face within a
//!    cone of the chart's own direction and do not cross a sharp crease, so
//!    each chart is a patch of surface that lies out flat without folding:
//!    a CAD part's faces come apart at their edges, a smooth one into a few
//!    patches.
//! 2. **Flattening.** Each chart by least-squares conformal maps (Lévy et
//!    al. 2002): the layout that keeps every triangle's shape, solved as a
//!    sparse least-squares problem by conjugate gradients. A chart that does
//!    not settle, or folds, is laid out by its projection instead.
//! 3. **Packing.** Each chart scaled to its true area, so the texture is as
//!    dense everywhere, turned to lie along its length, and packed in rows
//!    into the unit square with a margin between charts.
//!
//! The mesh comes back with its vertices split along the charts' seams and
//! the layout as its first UV set; the sets it had move up one, kept.

use crate::geometry::Mesh;
use std::collections::HashMap;
use tequila_core::math::{vec2, Vec2, Vec3};

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
}

impl Default for UnwrapOptions {
    fn default() -> Self {
        // Blender's Smart UV Project's angle, which splits a box into its
        // sides and a cylinder into a handful of strips.
        UnwrapOptions { cone_degrees: 66.0, crease_degrees: 60.0, margin: 0.005 }
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
    // Each chart flattened on its welded vertices, so a chart stays one
    // piece across the splits a mesh has for its normals. A conformal layout
    // keeps angles, not areas: a chart it shrinks far more in one place than
    // another - a cushion, a dome - is split between the two ways it turns
    // and each half laid out again, until the texture is near enough as
    // dense all over it.
    let edges = edge_faces(mesh, &weld);
    let mut queue = grow_charts(mesh, &weld, &normals, options);
    let mut charts = Vec::new();
    let mut layouts: Vec<Layout> = Vec::new();
    while let Some(chart) = queue.pop() {
        let layout = flatten(mesh, &weld, &normals, &chart);
        if chart.triangles.len() >= 8 && chart.depth < 10 && spread(mesh, &weld, &chart, &layout) > MOST_SPREAD {
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

    // A vertex for each chart a vertex of the mesh is in.
    let mut out = mesh.clone();
    let mut remap: HashMap<(usize, u32), u32> = HashMap::new();
    let mut positions = Vec::new();
    let mut normals_out = Vec::new();
    let mut colors = Vec::new();
    let mut sets: Vec<Vec<Vec2>> = vec![Vec::new(); mesh.uv_sets.len() + 1];
    let mut indices = vec![0u32; mesh.indices.len()];
    for (chart_index, (chart, layout)) in charts.iter().zip(&layouts).enumerate() {
        for &t in &chart.triangles {
            for corner in 0..3 {
                let vertex = mesh.indices[t * 3 + corner];
                let key = (chart_index, vertex);
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
                    sets[0].push(layout.uv[&weld[v]]);
                    for (set, uvs) in mesh.uv_sets.iter().enumerate() {
                        sets[set + 1].push(uvs.get(v).copied().unwrap_or(Vec2::ZERO));
                    }
                    next
                });
                indices[t * 3 + corner] = index;
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

/// Each vertex's position-welded index.
fn weld(positions: &[Vec3]) -> Vec<u32> {
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for p in positions {
        low = low.min(*p);
        high = high.max(*p);
    }
    let quantum = ((high - low).length() * 1e-6).max(1e-12);
    let mut ids: HashMap<(i64, i64, i64), u32> = HashMap::new();
    positions
        .iter()
        .map(|p| {
            let key = ((p.x / quantum).round() as i64, (p.y / quantum).round() as i64, (p.z / quantum).round() as i64);
            let next = ids.len() as u32;
            *ids.entry(key).or_insert(next)
        })
        .collect()
}

struct Chart {
    triangles: Vec<usize>,
    /// How many times it has been split from the chart it was grown as.
    depth: u32,
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
fn spread(mesh: &Mesh, weld: &[u32], chart: &Chart, layout: &Layout) -> f32 {
    let mut scales: Vec<(f32, f32)> = Vec::with_capacity(chart.triangles.len());
    for &t in &chart.triangles {
        let [a, b, c] = mesh.triangle(t).map(|v| v as usize);
        let surface = (mesh.positions[b] - mesh.positions[a]).cross(mesh.positions[c] - mesh.positions[a]).length() * 0.5;
        if surface <= 0.0 {
            continue;
        }
        let [ua, ub, uc] = [a, b, c].map(|v| layout.uv[&weld[v]]);
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
        pieces.push(Chart { triangles: members, depth: chart.depth + 1 });
    }
    pieces
}

/// Triangles grown into charts from seeds, largest first, each taking its
/// neighbours while they face within the cone of the chart's direction and
/// across no sharp crease.
fn grow_charts(mesh: &Mesh, weld: &[u32], normals: &[Vec3], options: &UnwrapOptions) -> Vec<Chart> {
    let triangles = mesh.triangle_count();
    let mut neighbours: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for t in 0..triangles {
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
    let mut order: Vec<usize> = (0..triangles).collect();
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
        charts.push(Chart { triangles: members, depth: 0 });
    }
    charts
}

/// A chart's layout: each welded vertex's position, in units of the part's
/// own length, so area on the layout is area on the part.
struct Layout {
    uv: HashMap<u32, Vec2>,
    /// Laid out by projection: conformal flattening did not settle or folded.
    projected: bool,
}

/// One chart laid out flat: conformally where that settles without folding,
/// else projected along its direction.
fn flatten(mesh: &Mesh, weld: &[u32], normals: &[Vec3], chart: &Chart) -> Layout {
    let mut local: HashMap<u32, usize> = HashMap::new();
    let mut points: Vec<Vec3> = Vec::new();
    let mut faces: Vec<[usize; 3]> = Vec::new();
    let mut direction = Vec3::ZERO;
    for &t in &chart.triangles {
        let corners = mesh.triangle(t).map(|v| {
            let w = weld[v as usize];
            *local.entry(w).or_insert_with(|| {
                points.push(mesh.positions[v as usize]);
                points.len() - 1
            })
        });
        direction = direction + normals[t];
        faces.push(corners);
    }
    let projected = project(&points, direction);
    let conformal = if faces.len() > 1 { lscm(&points, &faces, &projected) } else { None };
    let (uv, projected_only) = match conformal {
        Some(uv) if !folds(&uv, &faces, &projected) => (uv, false),
        _ => (projected, true),
    };
    let uv = to_true_area(&points, &faces, uv);
    let by_weld = local.iter().map(|(&w, &i)| (w, uv[i])).collect();
    Layout { uv: by_weld, projected: projected_only }
}

/// The points seen down `direction`, in the plane across it.
fn project(points: &[Vec3], direction: Vec3) -> Vec<Vec2> {
    let n = if direction.length() > 0.0 { direction.normalize() } else { Vec3::Z };
    let helper = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = helper.cross(n).normalize();
    let v = n.cross(u);
    points.iter().map(|p| vec2(p.dot(u), p.dot(v))).collect()
}

/// Least-squares conformal maps: the layout keeping each triangle's angles
/// best, two vertices far apart pinned where the projection puts them so
/// the answer is the projection's size and turn.
fn lscm(points: &[Vec3], faces: &[[usize; 3]], start: &[Vec2]) -> Option<Vec<Vec2>> {
    let n = points.len();
    // The two pins: the vertices farthest apart in the projection.
    let first = (0..n).min_by(|&a, &b| start[a].x.total_cmp(&start[b].x))?;
    let second = (0..n).max_by(|&a, &b| (start[a] - start[first]).length().total_cmp(&(start[b] - start[first]).length()))?;
    if first == second {
        return None;
    }
    // Unknowns: u and v of every vertex but the pins, u at 2i, v at 2i+1.
    let mut column = vec![usize::MAX; n];
    let mut free = 0;
    for i in 0..n {
        if i != first && i != second {
            column[i] = free;
            free += 1;
        }
    }
    // Each triangle gives two equations, the real and imaginary parts of
    // sum_j W_j (u_j + i v_j) = 0, weighted by the triangle's size.
    let mut rows: Vec<Vec<(usize, f64)>> = Vec::with_capacity(faces.len() * 2);
    let mut rhs: Vec<f64> = Vec::with_capacity(faces.len() * 2);
    for face in faces {
        let [p0, p1, p2] = face.map(|i| points[i]);
        let e1 = p1 - p0;
        let e2 = p2 - p0;
        let x_axis = if e1.length() > 0.0 { e1.normalize() } else { continue };
        let normal = e1.cross(e2);
        let area2 = normal.length() as f64;
        if area2 <= 1e-20 {
            continue;
        }
        let y_axis = normal.normalize().cross(x_axis);
        // The triangle in its own plane.
        let local = [(0.0f64, 0.0f64), (e1.length() as f64, 0.0), (e2.dot(x_axis) as f64, e2.dot(y_axis) as f64)];
        let scale = 1.0 / area2.sqrt();
        let mut re: Vec<(usize, f64)> = Vec::with_capacity(6);
        let mut im: Vec<(usize, f64)> = Vec::with_capacity(6);
        let (mut re_b, mut im_b) = (0.0f64, 0.0f64);
        for j in 0..3 {
            let (k, l) = ((j + 1) % 3, (j + 2) % 3);
            let a = (local[l].0 - local[k].0) * scale;
            let b = (local[l].1 - local[k].1) * scale;
            let vertex = face[j];
            // Re: a u - b v ; Im: b u + a v
            if column[vertex] == usize::MAX {
                let pinned = start[vertex];
                let (u, v) = (pinned.x as f64, pinned.y as f64);
                re_b -= a * u - b * v;
                im_b -= b * u + a * v;
            } else {
                let c = column[vertex];
                re.push((2 * c, a));
                re.push((2 * c + 1, -b));
                im.push((2 * c, b));
                im.push((2 * c + 1, a));
            }
        }
        rows.push(re);
        rhs.push(re_b);
        rows.push(im);
        rhs.push(im_b);
    }
    let unknowns = 2 * free;
    if unknowns == 0 {
        return Some(start.to_vec());
    }
    let mut x = vec![0.0f64; unknowns];
    for i in 0..n {
        if column[i] != usize::MAX {
            x[2 * column[i]] = start[i].x as f64;
            x[2 * column[i] + 1] = start[i].y as f64;
        }
    }
    solve_least_squares(&rows, &rhs, &mut x, unknowns)?;
    let mut out = start.to_vec();
    for i in 0..n {
        if column[i] != usize::MAX {
            out[i] = vec2(x[2 * column[i]] as f32, x[2 * column[i] + 1] as f32);
        }
    }
    out.iter().all(|p| p.x.is_finite() && p.y.is_finite()).then_some(out)
}

/// Minimize |A x - b| by conjugate gradients on the normal equations, with
/// Jacobi preconditioning, from the `x` given. None when it does not settle.
fn solve_least_squares(rows: &[Vec<(usize, f64)>], rhs: &[f64], x: &mut [f64], unknowns: usize) -> Option<()> {
    let apply = |x: &[f64], out: &mut [f64]| {
        // A^T A x, without forming A^T A.
        out.iter_mut().for_each(|v| *v = 0.0);
        for row in rows {
            let dot: f64 = row.iter().map(|&(c, w)| w * x[c]).sum();
            for &(c, w) in row {
                out[c] += w * dot;
            }
        }
    };
    let mut diagonal = vec![0.0f64; unknowns];
    let mut atb = vec![0.0f64; unknowns];
    for (row, b) in rows.iter().zip(rhs) {
        for &(c, w) in row {
            diagonal[c] += w * w;
            atb[c] += w * b;
        }
    }
    let inverse: Vec<f64> = diagonal.iter().map(|d| if *d > 0.0 { 1.0 / d } else { 1.0 }).collect();
    let mut ax = vec![0.0f64; unknowns];
    apply(x, &mut ax);
    let mut r: Vec<f64> = atb.iter().zip(&ax).map(|(b, a)| b - a).collect();
    let mut z: Vec<f64> = r.iter().zip(&inverse).map(|(r, m)| r * m).collect();
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let scale: f64 = atb.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
    let mut ap = vec![0.0f64; unknowns];
    let limit = (unknowns * 2).clamp(200, 20_000);
    for _ in 0..limit {
        let residual = r.iter().map(|v| v * v).sum::<f64>().sqrt();
        if residual <= 1e-9 * scale + 1e-14 {
            return Some(());
        }
        apply(&p, &mut ap);
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap <= 0.0 || !pap.is_finite() {
            break;
        }
        let alpha = rz / pap;
        for i in 0..unknowns {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        for i in 0..unknowns {
            z[i] = r[i] * inverse[i];
        }
        let next: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let beta = next / rz;
        rz = next;
        for i in 0..unknowns {
            p[i] = z[i] + beta * p[i];
        }
    }
    // Close enough is close enough: a layout to a part in a thousand is
    // finer than any texture shows.
    let residual = r.iter().map(|v| v * v).sum::<f64>().sqrt();
    (residual <= 1e-3 * scale).then_some(())
}

/// Whether a layout turns any triangle over against the projection's
/// winding: a fold, which would print the artwork mirrored there.
fn folds(uv: &[Vec2], faces: &[[usize; 3]], reference: &[Vec2]) -> bool {
    let signed = |p: &[Vec2], f: &[usize; 3]| {
        let (a, b, c) = (p[f[0]], p[f[1]], p[f[2]]);
        (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
    };
    // The chart's own winding: the sign most of its area has.
    let total: f32 = faces.iter().map(|f| signed(uv, f)).sum();
    let reference_total: f32 = faces.iter().map(|f| signed(reference, f)).sum();
    if total == 0.0 || total.signum() != reference_total.signum() && reference_total != 0.0 {
        return true;
    }
    // A CAD mesh's slivers can come out turned over by a hair - one in a
    // few thousand, a ten-millionth of the chart's area - where no artwork
    // shows it. A fold is a turned-over share worth seeing.
    let turned: f32 = faces.iter().map(|f| signed(uv, f) * total.signum()).filter(|a| *a < 0.0).map(|a| -a).sum();
    turned > total.abs() * 1e-4
}

/// The layout scaled so its area is the chart's area on the part.
fn to_true_area(points: &[Vec3], faces: &[[usize; 3]], uv: Vec<Vec2>) -> Vec<Vec2> {
    let mut surface = 0.0f64;
    let mut flat = 0.0f64;
    for f in faces {
        surface += ((points[f[1]] - points[f[0]]).cross(points[f[2]] - points[f[0]]).length() * 0.5) as f64;
        let (a, b, c) = (uv[f[0]], uv[f[1]], uv[f[2]]);
        flat += (((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)) * 0.5).abs() as f64;
    }
    if flat <= 0.0 || surface <= 0.0 {
        return uv;
    }
    let s = (surface / flat).sqrt() as f32;
    uv.into_iter().map(|p| p * s).collect()
}

/// Every chart turned along its length and packed in rows into the unit
/// square, `margin` apart; how much of the square they cover.
fn pack(layouts: &mut [Layout], margin: f32) -> f32 {
    // Turned to lie along their principal axis, then their boxes.
    let mut boxes: Vec<(usize, Vec2, Vec2)> = Vec::new();
    for (index, layout) in layouts.iter_mut().enumerate() {
        let points: Vec<Vec2> = layout.uv.values().copied().collect();
        if points.is_empty() {
            continue;
        }
        let angle = principal_angle(&points);
        let (c, s) = (angle.cos(), angle.sin());
        for p in layout.uv.values_mut() {
            *p = vec2(p.x * c + p.y * s, -p.x * s + p.y * c);
        }
        let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for p in layout.uv.values() {
            low = low.min(*p);
            high = high.max(*p);
        }
        // Wider than tall, so rows pack tightly.
        if high.y - low.y > high.x - low.x {
            for p in layout.uv.values_mut() {
                *p = vec2(-p.y, p.x);
            }
            (low, high) = (vec2(-high.y, low.x), vec2(-low.y, high.x));
        }
        boxes.push((index, low, high - low));
    }
    if boxes.is_empty() {
        return 0.0;
    }
    let area: f32 = boxes.iter().map(|(_, _, size)| size.x * size.y).sum();
    let widest = boxes.iter().map(|(_, _, size)| size.x).fold(0.0f32, f32::max);
    // Rows as wide as the square the charts would make, or the widest chart.
    let row_width = (area.sqrt() * 1.15).max(widest);
    boxes.sort_by(|a, b| b.2.y.total_cmp(&a.2.y));
    // The gap in the charts' own units, from the margin of a square about
    // that wide.
    let gap = margin * row_width;
    let mut placed: Vec<(usize, Vec2, Vec2)> = Vec::new();
    let (mut x, mut y, mut row_height, mut width) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for (index, low, size) in boxes {
        if x > 0.0 && x + size.x > row_width {
            x = 0.0;
            y += row_height + gap;
            row_height = 0.0;
        }
        placed.push((index, low, vec2(x, y)));
        width = width.max(x + size.x);
        x += size.x + gap;
        row_height = row_height.max(size.y);
    }
    let height = y + row_height;
    let side = width.max(height).max(1e-12) * (1.0 + 2.0 * margin);
    let inset = margin * side;
    for (index, low, at) in &placed {
        for p in layouts[*index].uv.values_mut() {
            *p = (*p - *low + *at + Vec2::splat(inset)) / side;
        }
    }
    area / (side * side)
}

/// The direction a chart is longest along: its points' principal axis.
fn principal_angle(points: &[Vec2]) -> f32 {
    let n = points.len() as f32;
    let mean = points.iter().fold(Vec2::ZERO, |a, p| a + *p) / n;
    let (mut xx, mut xy, mut yy) = (0.0f32, 0.0f32, 0.0f32);
    for p in points {
        let d = *p - mean;
        xx += d.x * d.x;
        xy += d.x * d.y;
        yy += d.y * d.y;
    }
    0.5 * (2.0 * xy).atan2(xx - yy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    fn quad_mesh(corners: [Vec3; 4]) -> Mesh {
        Mesh::new(corners.to_vec(), vec![0, 1, 2, 0, 2, 3])
    }

    /// A unit cube split at its creases, four vertices a face.
    fn cube() -> Mesh {
        let faces: [[Vec3; 4]; 6] = [
            [vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(0.0, 1.0, 1.0)],
            [vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0)],
            [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 1.0), vec3(0.0, 1.0, 0.0)],
            [vec3(1.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(1.0, 1.0, 1.0)],
            [vec3(0.0, 1.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)],
            [vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0)],
        ];
        let mut positions = Vec::new();
        let mut indices = Vec::new();
        for quad in faces {
            let base = positions.len() as u32;
            positions.extend_from_slice(&quad);
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        Mesh::new(positions, indices)
    }

    /// A grid of quads over a function of x and z: a patch of surface.
    fn sheet(rows: usize, columns: usize, height: impl Fn(f32, f32) -> f32) -> Mesh {
        let mut positions = Vec::new();
        for r in 0..=rows {
            for c in 0..=columns {
                let (x, z) = (c as f32 / columns as f32, r as f32 / rows as f32);
                positions.push(vec3(x, height(x, z), z));
            }
        }
        let mut indices = Vec::new();
        let at = |r: usize, c: usize| (r * (columns + 1) + c) as u32;
        for r in 0..rows {
            for c in 0..columns {
                indices.extend_from_slice(&[at(r, c), at(r + 1, c), at(r + 1, c + 1), at(r, c), at(r + 1, c + 1), at(r, c + 1)]);
            }
        }
        Mesh::new(positions, indices)
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

    #[test]
    fn a_box_comes_apart_into_its_six_sides_without_overlap() {
        let (out, report) = unwrap(&cube(), &UnwrapOptions::default());
        assert_eq!(report.charts, 6);
        assert!(inside_the_square(&out));
        assert!(stretch(&out) < 1.001, "every side as dense as the others");
        // Each side's box apart from every other's.
        let mut boxes = Vec::new();
        for chart in 0..6 {
            let corners: Vec<Vec2> = (0..4).map(|k| out.uv_sets[0][chart * 4 + k]).collect();
            let low = corners.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
            let high = corners.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
            boxes.push((low, high));
        }
        for i in 0..6 {
            for j in i + 1..6 {
                let (a, b) = (boxes[i], boxes[j]);
                let apart = a.1.x <= b.0.x + 1e-6 || b.1.x <= a.0.x + 1e-6 || a.1.y <= b.0.y + 1e-6 || b.1.y <= a.0.y + 1e-6;
                assert!(apart, "sides {i} and {j} overlap: {a:?} {b:?}");
            }
        }
        assert!(report.coverage > 0.3, "packed tightly enough: {}", report.coverage);
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
}
