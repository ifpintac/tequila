//! Laying a patch of surface out flat, and the checks every layout made here
//! passes.
//!
//! A patch is points in 3D and triangles over them. It is laid out by
//! least-squares conformal maps (Lévy et al. 2002): the layout that keeps
//! every triangle's shape as well as one can - its angles, not its area -
//! solved as a sparse least-squares problem by conjugate gradients. Two
//! vertices or more are pinned, held where they are, which gives the answer
//! its place, size and turn: pinned by hand, where the person put them;
//! pinned here, two far apart, held where a starting layout has them.
//!
//! A layout reads the right way: seen from the front of the surface, where
//! its triangles run counter-clockwise, u runs across the picture and v down
//! it, as an image's rows do and as the renderer samples it. Such a triangle
//! has a positive [`facing`]. One turned the other way among its island is a
//! fold, where the artwork would print mirrored.

use tequila_core::math::{vec2, Vec2, Vec3};

/// The area of a triangle on the layout, positive when it reads the right
/// way - u across, v down, as seen from the front of the surface - and
/// negative when it is turned over.
#[inline]
pub fn facing(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    ((b.y - a.y) * (c.x - a.x) - (b.x - a.x) * (c.y - a.y)) * 0.5
}

/// A triangle's area on the part.
#[inline]
pub fn surface_area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    (b - a).cross(c - a).length() * 0.5
}

/// The solve works in the usual mathematical orientation, v up, where a
/// conformal map keeps a counter-clockwise triangle counter-clockwise; the
/// layouts it gives and takes have v down. This is the one between them.
#[inline]
fn mirror(p: Vec2) -> Vec2 {
    vec2(p.x, -p.y)
}

/// The points seen down `direction`, from its side: u and v across it, the
/// layout reading the right way from there.
pub fn project(points: &[Vec3], direction: Vec3) -> Vec<Vec2> {
    let n = if direction.length() > 0.0 { direction.normalize() } else { Vec3::Z };
    let helper = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = helper.cross(n).normalize();
    // u x v = -n: v runs down the picture seen from the front.
    let v = u.cross(n);
    points.iter().map(|p| vec2(p.dot(u), p.dot(v))).collect()
}

/// The patch laid out conformally. `pins` are held where they say when there
/// are two or more; with fewer, the two vertices farthest apart in `start`
/// are held where `start` has them, so the layout comes out at about its
/// size and turn. `start` is also where the solve begins, so a layout near
/// the answer settles quickly. None when it does not settle.
pub fn conformal(points: &[Vec3], faces: &[[usize; 3]], start: &[Vec2], pins: &[(usize, Vec2)]) -> Option<Vec<Vec2>> {
    let n = points.len();
    if n < 3 || faces.is_empty() || start.len() != n {
        return None;
    }
    let begin: Vec<Vec2> = start.iter().map(|p| mirror(*p)).collect();
    let mut fixed: Vec<Option<Vec2>> = vec![None; n];
    let distinct = {
        let mut held: Vec<usize> = pins.iter().map(|(i, _)| *i).filter(|&i| i < n).collect();
        held.sort_unstable();
        held.dedup();
        held.len()
    };
    if distinct >= 2 {
        for &(i, at) in pins {
            if i < n {
                fixed[i] = Some(mirror(at));
            }
        }
    } else {
        // The two farthest apart, near enough: from one end, the farthest,
        // and from there the farthest again.
        let used = used_vertices(n, faces);
        let candidates: Vec<usize> = (0..n).filter(|&i| used[i]).collect();
        let first = *candidates.iter().min_by(|&&a, &&b| begin[a].x.total_cmp(&begin[b].x))?;
        let far = |from: usize| *candidates.iter().max_by(|&&a, &&b| (begin[a] - begin[from]).length().total_cmp(&(begin[b] - begin[from]).length())).unwrap_or(&from);
        let one = far(first);
        let two = far(one);
        if one == two || (begin[one] - begin[two]).length() <= 0.0 {
            return None;
        }
        fixed[one] = Some(begin[one]);
        fixed[two] = Some(begin[two]);
    }
    let out = lscm(points, faces, &begin, &fixed)?;
    Some(out.into_iter().map(mirror).collect())
}

fn used_vertices(n: usize, faces: &[[usize; 3]]) -> Vec<bool> {
    let mut used = vec![false; n];
    for f in faces {
        for &i in f {
            used[i] = true;
        }
    }
    used
}

/// Least-squares conformal maps, v up, `fixed` vertices held where they say.
fn lscm(points: &[Vec3], faces: &[[usize; 3]], start: &[Vec2], fixed: &[Option<Vec2>]) -> Option<Vec<Vec2>> {
    let n = points.len();
    // Unknowns: u and v of every vertex not held, u at 2i, v at 2i+1.
    let mut column = vec![usize::MAX; n];
    let mut free = 0;
    for i in 0..n {
        if fixed[i].is_none() {
            column[i] = free;
            free += 1;
        }
    }
    // Each triangle gives two equations, the real and imaginary parts of
    // sum_j W_j (u_j + i v_j) = 0, weighted by the triangle's size.
    let mut rows: Vec<[(usize, f64); 6]> = Vec::with_capacity(faces.len() * 2);
    let mut widths: Vec<u8> = Vec::with_capacity(faces.len() * 2);
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
        let mut re = [(0usize, 0.0f64); 6];
        let mut im = [(0usize, 0.0f64); 6];
        let mut width = 0usize;
        let (mut re_b, mut im_b) = (0.0f64, 0.0f64);
        for j in 0..3 {
            let (k, l) = ((j + 1) % 3, (j + 2) % 3);
            let a = (local[l].0 - local[k].0) * scale;
            let b = (local[l].1 - local[k].1) * scale;
            let vertex = face[j];
            // Re: a u - b v ; Im: b u + a v
            match fixed[vertex] {
                Some(pinned) => {
                    let (u, v) = (pinned.x as f64, pinned.y as f64);
                    re_b -= a * u - b * v;
                    im_b -= b * u + a * v;
                }
                None => {
                    let c = column[vertex];
                    re[width] = (2 * c, a);
                    re[width + 1] = (2 * c + 1, -b);
                    im[width] = (2 * c, b);
                    im[width + 1] = (2 * c + 1, a);
                    width += 2;
                }
            }
        }
        rows.push(re);
        widths.push(width as u8);
        rhs.push(re_b);
        rows.push(im);
        widths.push(width as u8);
        rhs.push(im_b);
    }
    let unknowns = 2 * free;
    let mut out: Vec<Vec2> = (0..n).map(|i| fixed[i].unwrap_or(start[i])).collect();
    if unknowns == 0 {
        return Some(out);
    }
    let mut x = vec![0.0f64; unknowns];
    for i in 0..n {
        if column[i] != usize::MAX {
            x[2 * column[i]] = start[i].x as f64;
            x[2 * column[i] + 1] = start[i].y as f64;
        }
    }
    solve_least_squares(&rows, &widths, &rhs, &mut x, unknowns)?;
    for i in 0..n {
        if column[i] != usize::MAX {
            out[i] = vec2(x[2 * column[i]] as f32, x[2 * column[i] + 1] as f32);
        }
    }
    out.iter().all(|p| p.x.is_finite() && p.y.is_finite()).then_some(out)
}

/// The normal equations' matrix, A^T A, as compressed rows: what each
/// conjugate gradient step multiplies by, once, rather than going through A
/// and back.
struct Normal {
    starts: Vec<usize>,
    columns: Vec<u32>,
    values: Vec<f64>,
}

impl Normal {
    fn of(rows: &[[(usize, f64); 6]], widths: &[u8], unknowns: usize) -> Normal {
        // Its pattern is the mesh's: u and v of a vertex meet u and v of every
        // vertex it shares a triangle with, itself among them.
        let vertices = unknowns / 2;
        let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); vertices];
        for (row, &width) in rows.iter().zip(widths).step_by(2) {
            let row = &row[..width as usize];
            for &(a, _) in row.iter().step_by(2) {
                for &(b, _) in row.iter().step_by(2) {
                    neighbours[a / 2].push((b / 2) as u32);
                }
            }
        }
        let mut starts = Vec::with_capacity(unknowns + 1);
        let mut columns = Vec::new();
        starts.push(0);
        for list in &mut neighbours {
            list.sort_unstable();
            list.dedup();
            for _ in 0..2 {
                for &w in list.iter() {
                    columns.push(2 * w);
                    columns.push(2 * w + 1);
                }
                starts.push(columns.len());
            }
        }
        // Every pair of entries in a row adds to A^T A where they meet.
        let mut values = vec![0.0f64; columns.len()];
        for (row, &width) in rows.iter().zip(widths) {
            let row = &row[..width as usize];
            for &(i, wi) in row {
                let span = starts[i]..starts[i + 1];
                for &(j, wj) in row {
                    if let Ok(at) = columns[span.clone()].binary_search(&(j as u32)) {
                        values[span.start + at] += wi * wj;
                    }
                }
            }
        }
        Normal { starts, columns, values }
    }

    fn apply(&self, x: &[f64], out: &mut [f64]) {
        use rayon::prelude::*;
        let rows = |(i, out): (usize, &mut f64)| {
            let mut sum = 0.0;
            for k in self.starts[i]..self.starts[i + 1] {
                sum += self.values[k] * x[self.columns[k] as usize];
            }
            *out = sum;
        };
        // Threads only where there is work enough to share.
        if out.len() > 20_000 {
            out.par_iter_mut().enumerate().with_min_len(4096).for_each(rows);
        } else {
            out.iter_mut().enumerate().for_each(rows);
        }
    }

    fn diagonal(&self, i: usize) -> f64 {
        (self.starts[i]..self.starts[i + 1]).find(|&k| self.columns[k] as usize == i).map(|k| self.values[k]).unwrap_or(0.0)
    }
}

/// Minimize |A x - b| by conjugate gradients on the normal equations, with
/// Jacobi preconditioning, from the `x` given. None when it does not settle.
fn solve_least_squares(rows: &[[(usize, f64); 6]], widths: &[u8], rhs: &[f64], x: &mut [f64], unknowns: usize) -> Option<()> {
    let normal = Normal::of(rows, widths, unknowns);
    let mut atb = vec![0.0f64; unknowns];
    for ((row, &width), b) in rows.iter().zip(widths).zip(rhs) {
        for &(c, w) in &row[..width as usize] {
            atb[c] += w * b;
        }
    }
    let inverse: Vec<f64> = (0..unknowns).map(|i| normal.diagonal(i)).map(|d| if d > 0.0 { 1.0 / d } else { 1.0 }).collect();
    let mut ax = vec![0.0f64; unknowns];
    normal.apply(x, &mut ax);
    let mut r: Vec<f64> = atb.iter().zip(&ax).map(|(b, a)| b - a).collect();
    let mut z: Vec<f64> = r.iter().zip(&inverse).map(|(r, m)| r * m).collect();
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let scale: f64 = atb.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
    let mut ap = vec![0.0f64; unknowns];
    let limit = (unknowns * 2).clamp(200, 20_000);
    // Settled to a part in a billion where that costs little. A large patch
    // gets to a part in a hundred thousand in a few hundred steps and then
    // creeps, taking thousands more to even out a gradient of density of a
    // few percent across it: on 50,000 triangles, a fifteenth of the time
    // for angles kept to half a percent and density within eight.
    let settled = if unknowns > 20_000 { 1e-5 } else { 1e-9 };
    for _ in 0..limit {
        let residual = r.iter().map(|v| v * v).sum::<f64>().sqrt();
        if residual <= settled * scale + 1e-14 {
            return Some(());
        }
        normal.apply(&p, &mut ap);
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

/// Whether a layout turns any of its triangles over against the rest: a
/// fold, which would print the artwork mirrored there. With `reads_right`,
/// also whether the layout as a whole is turned the other way from it.
pub fn folds(uv: &[Vec2], faces: &[[usize; 3]], reads_right: Option<bool>) -> bool {
    let area = |f: &[usize; 3]| facing(uv[f[0]], uv[f[1]], uv[f[2]]);
    // The layout's own winding: the sign most of its area has.
    let total: f32 = faces.iter().map(area).sum();
    if total == 0.0 || !total.is_finite() {
        return true;
    }
    if reads_right.is_some_and(|right| (total > 0.0) != right) {
        return true;
    }
    // A CAD mesh's slivers can come out turned over by a hair - one in a
    // few thousand, a ten-millionth of the chart's area - where no artwork
    // shows it. A fold is a triangle turned over by more than that, or a
    // turned-over share worth seeing.
    let hair = total.abs() * FOLD_HAIR;
    let turned: Vec<f32> = faces.iter().map(|f| area(f) * total.signum()).filter(|a| *a < 0.0).map(|a| -a).collect();
    turned.iter().any(|&a| a > hair) || turned.iter().sum::<f32>() > total.abs() * 1e-4
}

/// How much of its layout's area a triangle may be turned over by and not
/// count as a fold: a sliver's hair.
pub const FOLD_HAIR: f32 = 1e-6;

/// The layout scaled so its area is the patch's area on the part.
pub fn to_true_area(points: &[Vec3], faces: &[[usize; 3]], uv: Vec<Vec2>) -> Vec<Vec2> {
    let (surface, flat) = areas(points, faces, &uv);
    if flat <= 0.0 || surface <= 0.0 {
        return uv;
    }
    let s = (surface / flat).sqrt() as f32;
    uv.into_iter().map(|p| p * s).collect()
}

/// The patch's area on the part, and on the layout.
pub fn areas(points: &[Vec3], faces: &[[usize; 3]], uv: &[Vec2]) -> (f64, f64) {
    let mut surface = 0.0f64;
    let mut flat = 0.0f64;
    for f in faces {
        surface += surface_area(points[f[0]], points[f[1]], points[f[2]]) as f64;
        flat += facing(uv[f[0]], uv[f[1]], uv[f[2]]).abs() as f64;
    }
    (surface, flat)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;

    /// A grid of quads over a function of x and z, as points and faces.
    fn patch(rows: usize, columns: usize, height: impl Fn(f32, f32) -> f32) -> (Vec<Vec3>, Vec<[usize; 3]>) {
        let mut points = Vec::new();
        for r in 0..=rows {
            for c in 0..=columns {
                let (x, z) = (c as f32 / columns as f32, r as f32 / rows as f32);
                points.push(vec3(x, height(x, z), z));
            }
        }
        let at = |r: usize, c: usize| r * (columns + 1) + c;
        let mut faces = Vec::new();
        for r in 0..rows {
            for c in 0..columns {
                faces.push([at(r, c), at(r + 1, c), at(r + 1, c + 1)]);
                faces.push([at(r, c), at(r + 1, c + 1), at(r, c + 1)]);
            }
        }
        (points, faces)
    }

    #[test]
    fn a_projection_reads_the_right_way_from_the_front() {
        // A square facing +Z, counter-clockwise from there.
        let points = [vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0)];
        let uv = project(&points, Vec3::Z);
        assert!(facing(uv[0], uv[1], uv[2]) > 0.0);
        // And from behind it reads mirrored, as it should.
        let uv = project(&points, -Vec3::Z);
        assert!(facing(uv[0], uv[1], uv[2]) < 0.0);
    }

    #[test]
    fn pinned_vertices_stay_exactly_where_they_are_put() {
        let (points, faces) = patch(6, 6, |x, z| (x * 2.0).sin() * 0.3 + z * z * 0.2);
        let start = project(&points, Vec3::Y);
        let pins = [(0, vec2(0.0, 0.0)), (6, vec2(1.5, 0.0)), (48, vec2(1.0, 1.2))];
        let uv = conformal(&points, &faces, &start, &pins).expect("it settles");
        for (i, at) in pins {
            assert_eq!(uv[i], at, "pin {i} moved");
        }
        assert!(!folds(&uv, &faces, None));
    }

    #[test]
    fn without_pins_the_layout_keeps_the_start_s_turn_and_reads_right() {
        let (points, faces) = patch(8, 12, |x, _| (1.0 - (x - 0.5) * (x - 0.5) * 2.0).sqrt() * 0.5);
        let start = project(&points, Vec3::Y);
        let uv = conformal(&points, &faces, &start, &[]).expect("it settles");
        assert!(!folds(&uv, &faces, Some(true)), "reads the right way, no folds");
    }
}
