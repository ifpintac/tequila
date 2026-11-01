//! Islands unwrapped again as they are cut, with a choice of how: which
//! way, by which method, and what each keeps of where it was.
//!
//! - **Which way.** Both coordinates solved again; or only u, across, or
//!   only v, down, the other kept exactly as it is: an island squashed one
//!   way is evened out that way without its other way moving. One way is
//!   the conformal sum with the other coordinate held - each triangle as
//!   true to its angles as the held one lets it be - or the least-stretch
//!   rounds changing only that coordinate.
//! - **By which method** ([`Method`]): conformal, as an unwrap always has
//!   been, or least stretch, which starts from the conformal layout and
//!   keeps the surface's lengths as well as it can. The stretch left is
//!   reported either way.
//! - **What it keeps.** An island laid out again comes out wherever its
//!   solve puts it. Kept, its turn is the one that lays the new layout best
//!   over the old, its size the old size and its place the middle of the old
//!   box - the best-fitting likeness of the new onto the old, each part of
//!   it kept or not on its own. A turn not kept lays the island square, as
//!   Straighten does; a size not kept is the layout's own density. A place
//!   not kept is still the old middle here: the editor packs the layout
//!   after, which is what puts each island somewhere new.
//!
//! Islands are laid out on their own UV vertices, so a cut inside one stays
//! open. One that will not lie flat without folding is refused, as
//! re-flattening refuses it: cut it first.

use super::edit::{chosen_islands, UvError};
use super::flatten::{conformal, facing, folds, project, to_true_area};
use super::layout::{tightest_box, Island};
use super::least_stretch::{least_stretch, stretch_left, Axes, Method, ROUNDS};
use super::shape::{best_fit, box_middle, conjugate_gradients, layout, turned, Moves, Patch, ShapeError, Sparse};
use super::transform::Alignment;
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2, Vec3};

/// Which of an island's coordinates are solved again.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Direction {
    #[default]
    Both,
    /// Only u: v stays as it is.
    Across,
    /// Only v: u stays as it is.
    Down,
}

impl Direction {
    pub fn key(self) -> &'static str {
        match self {
            Direction::Both => "both",
            Direction::Across => "across",
            Direction::Down => "down",
        }
    }

    pub fn from_key(key: &str) -> Option<Direction> {
        match key {
            "both" => Some(Direction::Both),
            "across" | "u" => Some(Direction::Across),
            "down" | "v" => Some(Direction::Down),
            _ => None,
        }
    }
}

/// What an island laid out again keeps of how it lay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keep {
    pub place: bool,
    pub size: bool,
    pub turn: bool,
}

impl Default for Keep {
    fn default() -> Self {
        Keep { place: true, size: true, turn: true }
    }
}

/// How islands are unwrapped again.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AgainOptions {
    pub direction: Direction,
    pub method: Method,
    pub keep: Keep,
}

/// What unwrapping again did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AgainReport {
    pub islands: usize,
    /// The stretch left over them, by area ([`super::least_stretch::stretch_left`]).
    pub stretch: f32,
    /// The most rounds an island's least-stretch solve took.
    pub rounds: usize,
    /// Islands whose least-stretch layout would have folded, laid out
    /// conformally instead.
    pub conformal: usize,
}

/// `islands` - every island when none are named - unwrapped again as
/// `options` say: each one's new places.
pub fn unwrap_again(mesh: &Mesh, set: usize, islands: &[u32], options: &AgainOptions) -> Result<(Moves, AgainReport), ShapeError> {
    let layout = layout(mesh, set)?;
    let chosen = if islands.is_empty() { (0..layout.islands.len() as u32).collect() } else { chosen_islands(&layout, islands)? };
    if chosen.is_empty() {
        return Err(UvError::NothingChosen.into());
    }
    // The layout's own density, for a size not kept.
    let (flat, surface) = layout.islands.iter().fold((0.0f64, 0.0f64), |(f, s), i| (f + i.uv_area as f64, s + i.surface_area as f64));
    let density = if surface > 0.0 { flat / surface } else { 0.0 };
    let mut moves = Moves::new();
    let mut report = AgainReport { islands: chosen.len(), ..Default::default() };
    let (mut stretch, mut weight) = (0.0f64, 0.0f64);
    for &i in &chosen {
        let island = &layout.islands[i as usize];
        let patch = Patch::new(mesh, &layout, &island.triangles);
        let (uv, rounds, fell_back) = match options.direction {
            Direction::Both => both_ways(&patch, island.mirrored, options.method).ok_or(UvError::CannotFlatten(i))?,
            Direction::Across | Direction::Down => one_way(&patch, island.mirrored, options.method, options.direction).ok_or(UvError::CannotFlatten(i))?,
        };
        report.rounds = report.rounds.max(rounds);
        report.conformal += fell_back as usize;
        let uv = if options.direction == Direction::Both { placed(&uv, &patch, island, options.keep, density) } else { uv };
        let s = patch.surface();
        stretch += stretch_left(&patch.points, &patch.faces, &uv) as f64 * s;
        weight += s;
        moves.extend(patch.moves(&uv));
    }
    report.stretch = if weight > 0.0 { (stretch / weight) as f32 } else { 0.0 };
    Ok((moves, report))
}

/// A patch laid out both ways by `method`, reading as it did: conformally
/// where that does not fold, else seen down its own direction; then, for
/// least stretch, kept to the surface's lengths from there unless that
/// folds. The layout, the rounds it took, and whether least stretch fell
/// back to the conformal layout.
fn both_ways(patch: &Patch, mirrored: bool, method: Method) -> Option<(Vec<Vec2>, usize, bool)> {
    let flip = |p: Vec2| if mirrored { vec2(-p.x, p.y) } else { p };
    let start: Vec<Vec2> = patch.uv.iter().map(|p| flip(*p)).collect();
    let flat = conformal(&patch.points, &patch.faces, &start, &[]).filter(|uv| !folds(uv, &patch.faces, Some(true))).or_else(|| {
        let direction = patch.faces.iter().fold(Vec3::ZERO, |sum, f| sum + (patch.points[f[1]] - patch.points[f[0]]).cross(patch.points[f[2]] - patch.points[f[0]]));
        let seen = project(&patch.points, direction);
        (!folds(&seen, &patch.faces, Some(true))).then_some(seen)
    })?;
    let flat = to_true_area(&patch.points, &patch.faces, flat);
    let (uv, rounds, fell_back) = match method {
        Method::Conformal => (flat, 0, false),
        Method::LeastStretch => match least_stretch(&patch.points, &patch.faces, &flat, false, &[], Axes::Both, ROUNDS) {
            Some((least, rounds)) if !folds(&least, &patch.faces, Some(true)) => (least, rounds, false),
            _ => (flat, 0, true),
        },
    };
    Some((uv.into_iter().map(flip).collect(), rounds, fell_back))
}

/// A patch with one coordinate solved again and the other held: by the
/// conformal sum, then for least stretch by its rounds changing only that
/// coordinate. Its translation along that coordinate kept where its box's
/// middle was. None where it folds.
fn one_way(patch: &Patch, mirrored: bool, method: Method, direction: Direction) -> Option<(Vec<Vec2>, usize, bool)> {
    let across = direction == Direction::Across;
    let solved = conformal_one_way(patch, mirrored, across)?;
    let reads_right = !mirrored;
    let conformal_ok = !folds(&solved, &patch.faces, Some(reads_right));
    let axes = if across { Axes::Across } else { Axes::Down };
    let (mut uv, rounds, fell_back) = match method {
        Method::Conformal => {
            if !conformal_ok {
                return None;
            }
            (solved, 0, false)
        }
        Method::LeastStretch => {
            let from = if conformal_ok { solved.clone() } else { patch.uv.clone() };
            match least_stretch(&patch.points, &patch.faces, &from, mirrored, &[], axes, ROUNDS) {
                Some((least, rounds)) if !folds(&least, &patch.faces, Some(reads_right)) => (least, rounds, false),
                _ if conformal_ok => (solved, 0, true),
                _ => return None,
            }
        }
    };
    // Where along that way: its box's middle where it was.
    let pick = |p: Vec2| if across { p.x } else { p.y };
    let middle = |points: &[Vec2]| {
        let (low, high) = points.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), p| (l.min(pick(*p)), h.max(pick(*p))));
        (low + high) * 0.5
    };
    let shift = middle(&patch.uv) - middle(&uv);
    for p in &mut uv {
        if across { p.x += shift } else { p.y += shift }
    }
    Some((uv, rounds, fell_back))
}

/// The conformal layout of a patch with one coordinate held as it is: the
/// least-squares conformal sum (Lévy et al. 2002) with only u, or only v,
/// unknown. In the solve's orientation, v up and a mirrored patch mirrored
/// back, as [`super::flatten`] solves.
fn conformal_one_way(patch: &Patch, mirrored: bool, across: bool) -> Option<Vec<Vec2>> {
    let n = patch.len();
    let into = |p: Vec2| vec2(if mirrored { -p.x } else { p.x }, -p.y);
    let solve: Vec<Vec2> = patch.uv.iter().map(|p| into(*p)).collect();
    // Each triangle's two rows, as the conformal sum writes them:
    // Re = sum(a u - b v), Im = sum(b u + a v), both nothing.
    let mut normal: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
    let mut rhs = vec![0.0f64; n];
    let mut diagonal = 0.0f64;
    for face in &patch.faces {
        let [p0, p1, p2] = face.map(|i| patch.points[i]);
        let (e1, e2) = (p1 - p0, p2 - p0);
        let normal_3d = e1.cross(e2);
        let area2 = normal_3d.length() as f64;
        if e1.length() <= 0.0 || area2 <= 1e-20 {
            continue;
        }
        let ax = e1.normalize();
        let ay = normal_3d.normalize().cross(ax);
        let local = [(0.0f64, 0.0f64), (e1.length() as f64, 0.0), (e2.dot(ax) as f64, e2.dot(ay) as f64)];
        let scale = 1.0 / area2.sqrt();
        let mut ab = [(0.0f64, 0.0f64); 3];
        for j in 0..3 {
            let (k, l) = ((j + 1) % 3, (j + 2) % 3);
            ab[j] = ((local[l].0 - local[k].0) * scale, (local[l].1 - local[k].1) * scale);
        }
        // With v known, the rows in u: sum(a u) = sum(b v), sum(b u) =
        // -sum(a v); with u known, the rows in v: sum(-b v) = -sum(a u),
        // sum(a v) = -sum(b u).
        let known = |j: usize| if across { solve[face[j]].y as f64 } else { solve[face[j]].x as f64 };
        let rows: [([f64; 3], f64); 2] = if across {
            let c0: f64 = (0..3).map(|j| ab[j].1 * known(j)).sum();
            let c1: f64 = (0..3).map(|j| -ab[j].0 * known(j)).sum();
            [([ab[0].0, ab[1].0, ab[2].0], c0), ([ab[0].1, ab[1].1, ab[2].1], c1)]
        } else {
            let c0: f64 = (0..3).map(|j| -ab[j].0 * known(j)).sum();
            let c1: f64 = (0..3).map(|j| -ab[j].1 * known(j)).sum();
            [([-ab[0].1, -ab[1].1, -ab[2].1], c0), ([ab[0].0, ab[1].0, ab[2].0], c1)]
        };
        for (coefficients, c) in rows {
            for p in 0..3 {
                for q in 0..3 {
                    normal[face[p]].push((face[q], coefficients[p] * coefficients[q]));
                }
                rhs[face[p]] += coefficients[p] * c;
                diagonal += coefficients[p] * coefficients[p];
            }
        }
    }
    // A hair of pull toward where each vertex is: the sum says nothing of
    // where along that way the patch lies.
    let pull = (diagonal / n.max(1) as f64) * 1e-9;
    let mut x: Vec<f64> = solve.iter().map(|p| if across { p.x as f64 } else { p.y as f64 }).collect();
    for (i, row) in normal.iter_mut().enumerate() {
        row.push((i, pull));
        rhs[i] += pull * x[i];
    }
    let matrix = Sparse::from_rows(normal);
    if !conjugate_gradients(&matrix, &rhs, &mut x, 1e-10, (n * 4).clamp(200, 40_000)) {
        return None;
    }
    let out: Vec<Vec2> = solve
        .iter()
        .zip(&x)
        .map(|(p, &value)| {
            let q = if across { vec2(value as f32, p.y) } else { vec2(p.x, value as f32) };
            into(q)
        })
        .collect();
    out.iter().all(|p| p.is_finite()).then_some(out)
}

/// A new layout of an island put where the old one was, as `keep` says:
/// turned to lie best over the old or laid square, the old size or the
/// layout's density, centred where the old box was.
fn placed(new: &[Vec2], patch: &Patch, isl: &Island, keep: Keep, density: f64) -> Vec<Vec2> {
    let angle = if keep.turn { best_fit(new, &patch.uv).0 } else { Alignment::Nearest.turn_for(tightest_box(new).0) };
    let now: f64 = patch.faces.iter().map(|f| facing(new[f[0]], new[f[1]], new[f[2]]).abs() as f64).sum();
    let target = if keep.size { isl.uv_area as f64 } else { isl.surface_area as f64 * density };
    let scale = if now > 0.0 && target > 0.0 { (target / now).sqrt() as f32 } else { 1.0 };
    let middle = box_middle(new.iter().copied());
    let moved: Vec<Vec2> = new.iter().map(|p| turned((*p - middle) * scale, angle)).collect();
    let shift = isl.centre - box_middle(moved.iter().copied());
    moved.into_iter().map(|p| p + shift).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{set_positions, transform, Pivot, UvAffine, UvLayout, UvSelection};

    fn dome() -> Mesh {
        unwrapped(sheet(10, 10, |x, z| 0.35 - ((x - 0.5).powi(2) + (z - 0.5).powi(2)) * 1.2))
    }

    #[test]
    fn again_in_place_keeps_an_island_where_it_was_and_least_stretch_stretches_less() {
        let mesh = dome();
        let before = UvLayout::new(&mesh, 0).unwrap();
        let (moves, conformal) = unwrap_again(&mesh, 0, &[0], &AgainOptions::default()).unwrap();
        let out = set_positions(&mesh, 0, &moves).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        let (a, b) = (&before.islands[0], &after.islands[0]);
        assert!((a.centre - b.centre).length() < 1e-5, "in place");
        assert!((a.uv_area - b.uv_area).abs() / a.uv_area < 1e-3, "as large");
        assert!(after.folded_triangles().is_empty());
        let least = AgainOptions { method: Method::LeastStretch, ..Default::default() };
        let (moves, report) = unwrap_again(&mesh, 0, &[0], &least).unwrap();
        assert!(report.stretch < conformal.stretch * 0.8, "least stretch {} against conformal {}", report.stretch, conformal.stretch);
        assert!(report.rounds > 0 && report.conformal == 0);
        let out = set_positions(&mesh, 0, &moves).unwrap();
        assert!(UvLayout::new(&out, 0).unwrap().folded_triangles().is_empty());
    }

    #[test]
    fn one_way_evens_out_a_squash_and_leaves_the_other_way_alone() {
        // A flat sheet squashed across to half: solved again across, it is
        // square again, and every v is what it was.
        let mesh = unwrapped(sheet(6, 6, |_, _| 0.0));
        let squashed = transform(&mesh, 0, &UvSelection::Islands(vec![0]), &UvAffine::scale(vec2(0.5, 1.0)), Pivot::Islands).unwrap();
        let options = AgainOptions { direction: Direction::Across, ..Default::default() };
        let (moves, report) = unwrap_again(&squashed, 0, &[], &options).unwrap();
        assert!(report.stretch < 1e-3, "{}", report.stretch);
        let out = set_positions(&squashed, 0, &moves).unwrap();
        let island = &UvLayout::new(&out, 0).unwrap().islands[0];
        let size = island.max - island.min;
        assert!((size.x / size.y - 1.0).abs() < 1e-3, "square again: {size:?}");
        for (a, b) in squashed.uv_sets[0].iter().zip(&out.uv_sets[0]) {
            assert!((a.y - b.y).abs() < 1e-6, "v kept");
        }
        // Down only on a mirrored island: it stays mirrored.
        let mirrored = transform(&squashed, 0, &UvSelection::Islands(vec![0]), &UvAffine::flip_u(), Pivot::Islands).unwrap();
        let mirrored = transform(&mirrored, 0, &UvSelection::Islands(vec![0]), &UvAffine::scale(vec2(1.0, 0.25)), Pivot::Islands).unwrap();
        let down = AgainOptions { direction: Direction::Down, method: Method::LeastStretch, ..Default::default() };
        let (moves, _) = unwrap_again(&mirrored, 0, &[0], &down).unwrap();
        let out = set_positions(&mirrored, 0, &moves).unwrap();
        let layout = UvLayout::new(&out, 0).unwrap();
        assert!(layout.islands[0].mirrored && layout.folded_triangles().is_empty());
        for (a, b) in mirrored.uv_sets[0].iter().zip(&out.uv_sets[0]) {
            assert!((a.x - b.x).abs() < 1e-6, "u kept");
        }
    }

    #[test]
    fn a_turn_and_size_not_kept_lay_it_square_at_the_layout_s_density() {
        let mesh = unwrapped(cube());
        // Side 0 turned and made large.
        let moved = transform(&mesh, 0, &UvSelection::Islands(vec![0]), &UvAffine::rotation(0.4).then(&UvAffine::uniform(2.0)), Pivot::Islands).unwrap();
        let keep = Keep { place: true, size: false, turn: false };
        let (moves, _) = unwrap_again(&moved, 0, &[0], &AgainOptions { keep, ..Default::default() }).unwrap();
        let out = set_positions(&moved, 0, &moves).unwrap();
        let layout = UvLayout::new(&out, 0).unwrap();
        let island = &layout.islands[0];
        let off = crate::uv::layout::fold_angle(island.box_angle).abs();
        assert!(off < 1e-3 || (off - std::f32::consts::FRAC_PI_2).abs() < 1e-3, "square: {}", island.box_angle);
        let before = UvLayout::new(&moved, 0).unwrap();
        let density: f32 = before.islands.iter().map(|i| i.uv_area).sum::<f32>() / before.islands.iter().map(|i| i.surface_area).sum::<f32>();
        assert!((island.density() / density - 1.0).abs() < 1e-3, "{} {}", island.density(), density);
        assert!((island.centre - before.islands[0].centre).length() < 1e-5);
    }
}
