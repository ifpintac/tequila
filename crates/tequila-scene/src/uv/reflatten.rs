//! Islands laid out flat again, their cuts kept, some vertices pinned.
//!
//! Each island is flattened by least-squares conformal maps on its own UV
//! vertices, so a cut inside it stays open. Pinned vertices stay exactly
//! where they are: two or more decide the island's place, size and turn
//! themselves. With fewer, the island keeps its place in the layout: turned
//! as near as it can be to how it lay, as large as it was - as dense, so a
//! layout that was even stays even - and centred where its box was, or on
//! its one pin.

use super::edit::{chosen_islands, finish, layout_of, UvError};
use super::flatten::{conformal, facing, folds, project};
use super::pack::{pack, PackOptions};
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2, Vec3};

/// The islands laid out flat again, `pins` - mesh vertices, and every
/// corner on the same place with them - held where they are; then, if
/// asked, the whole layout packed again.
pub fn reflatten(mesh: &Mesh, set: usize, islands: &[u32], pins: &[u32], repack: Option<&PackOptions>) -> Result<Mesh, UvError> {
    let layout = layout_of(mesh, set)?;
    let chosen = chosen_islands(&layout, islands)?;
    if chosen.is_empty() {
        return Err(UvError::NothingChosen);
    }
    if let Some(&bad) = pins.iter().find(|&&v| v as usize >= mesh.positions.len()) {
        return Err(UvError::NoSuchVertex(bad));
    }
    let pinned = layout.uv_vertices_of(pins);
    let mut uv: Vec<Vec2> = layout.uv_vertices.iter().map(|v| v.uv).collect();
    for &i in &chosen {
        let island = &layout.islands[i as usize];
        let held: Vec<u32> = island.uv_vertices.iter().copied().filter(|v| pinned.binary_search(v).is_ok()).collect();
        let placed = relax(mesh, &layout.corner_vertex, &island.triangles, &island.uv_vertices, &uv, &held, island.mirrored).ok_or(UvError::CannotFlatten(i))?;
        for (&v, p) in island.uv_vertices.iter().zip(placed) {
            uv[v as usize] = p;
        }
    }
    let flattened = finish(mesh, set, &layout.corner_vertex, &uv);
    match repack {
        Some(options) => pack(&flattened, set, None, options).map(|(mesh, _)| mesh),
        None => Ok(flattened),
    }
}

/// A piece of the layout - its triangles, on these UV vertices, now at
/// `uv` - flattened again: each UV vertex's new place, in the order given.
/// None where it will not lie flat without folding, even projected.
pub(crate) fn relax(mesh: &Mesh, corner_vertex: &[u32], triangles: &[u32], vertices: &[u32], uv: &[Vec2], held: &[u32], mirrored: bool) -> Option<Vec<Vec2>> {
    let local: std::collections::HashMap<u32, usize> = vertices.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let corners_of = |t: u32| [0, 1, 2].map(|k| corner_vertex[t as usize * 3 + k]);
    // Each UV vertex's point on the part: any corner on it says, as they
    // are all at one welded vertex.
    let mut points = vec![Vec3::ZERO; vertices.len()];
    let mut faces = Vec::with_capacity(triangles.len());
    for &t in triangles {
        let corners = corners_of(t);
        let face = corners.map(|g| local[&g]);
        for (k, &i) in face.iter().enumerate() {
            points[i] = mesh.positions[mesh.indices[t as usize * 3 + k] as usize];
        }
        faces.push(face);
    }
    let start: Vec<Vec2> = vertices.iter().map(|&v| uv[v as usize]).collect();
    let pins: Vec<(usize, Vec2)> = held.iter().map(|v| (local[v], uv[*v as usize])).collect();
    let reads_right = Some(!mirrored);
    if pins.len() >= 2 {
        let flat = conformal(&points, &faces, &start, &pins)?;
        return (!folds(&flat, &faces, reads_right)).then_some(flat);
    }
    let candidate = conformal(&points, &faces, &start, &[]).filter(|flat| !folds(flat, &faces, reads_right)).or_else(|| {
        // Seen down its own direction, the way round it lay.
        let direction = faces.iter().fold(Vec3::ZERO, |sum, f| sum + (points[f[1]] - points[f[0]]).cross(points[f[2]] - points[f[0]]));
        let mut seen = project(&points, direction);
        if mirrored {
            seen.iter_mut().for_each(|p| p.x = -p.x);
        }
        (!folds(&seen, &faces, reads_right)).then_some(seen)
    })?;
    Some(fit(&candidate, &start, &faces, pins.first().copied()))
}

/// A new layout of a piece put where the old one was: turned to match it
/// best, as large in area, centred on the old box, or with its one pin
/// exactly on the pin's place.
pub(crate) fn fit(new: &[Vec2], old: &[Vec2], faces: &[[usize; 3]], pin: Option<(usize, Vec2)>) -> Vec<Vec2> {
    let area = |p: &[Vec2]| faces.iter().map(|f| facing(p[f[0]], p[f[1]], p[f[2]]).abs() as f64).sum::<f64>();
    let (new_area, old_area) = (area(new), area(old));
    let scale = if new_area > 0.0 && old_area > 0.0 { (old_area / new_area).sqrt() as f32 } else { 1.0 };
    // The turn that matches them best, about their middles (Procrustes).
    let mean = |p: &[Vec2]| p.iter().fold(Vec2::ZERO, |a, q| a + *q) / p.len().max(1) as f32;
    let (mn, mo) = (mean(new), mean(old));
    let (mut dot, mut cross) = (0.0f64, 0.0f64);
    for (a, b) in new.iter().zip(old) {
        let (a, b) = (*a - mn, *b - mo);
        dot += (a.x * b.x + a.y * b.y) as f64;
        cross += (a.x * b.y - a.y * b.x) as f64;
    }
    let (s, c) = (cross.atan2(dot) as f32).sin_cos();
    let turned: Vec<Vec2> = new.iter().map(|p| {
        let q = (*p - mn) * scale;
        vec2(q.x * c - q.y * s, q.x * s + q.y * c)
    }).collect();
    let shift = match pin {
        Some((i, at)) => at - turned[i],
        None => {
            let middle = |p: &[Vec2]| {
                let (low, high) = p.iter().fold((Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY)), |(l, h), q| (l.min(*q), h.max(*q)));
                (low + high) * 0.5
            };
            middle(old) - middle(&turned)
        }
    };
    turned.into_iter().map(|p| p + shift).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::{transform, Pivot, UvAffine, UvLayout, UvSelection};

    #[test]
    fn a_distorted_island_relaxes_back_in_place_at_its_size() {
        // A curved sheet unwrapped, then squashed: re-flattened, it comes
        // back to its conformal shape, centred where it was, as large.
        let mesh = unwrapped(sheet(8, 8, |x, z| (x * 2.5).sin() * 0.3 + z * 0.2));
        let squash = UvAffine::scale(vec2(1.0, 0.5));
        let squashed = transform(&mesh, 0, &UvSelection::Islands(vec![0]), &squash, Pivot::Islands).unwrap();
        let before = UvLayout::new(&squashed, 0).unwrap();
        let out = reflatten(&squashed, 0, &[0], &[], None).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        let (a, b) = (&before.islands[0], &after.islands[0]);
        assert!((a.centre - b.centre).length() < 1e-5, "in place");
        assert!((a.uv_area - b.uv_area).abs() / a.uv_area < 1e-4, "as large");
        assert!(after.folded_triangles().is_empty());
        // Shaped as the unwrap had it: its box's proportions back.
        let original = &UvLayout::new(&mesh, 0).unwrap().islands[0];
        let ratio = |i: &crate::uv::Island| (i.max.x - i.min.x) / (i.max.y - i.min.y);
        let back = ratio(original).max(1.0 / ratio(original));
        let now = ratio(b).max(1.0 / ratio(b));
        assert!((back - now).abs() < 0.05, "{back} {now}");
        // Its vertices are not split: the cuts are the ones it had.
        assert_eq!(out.positions.len(), squashed.positions.len());
    }

    #[test]
    fn pinned_vertices_do_not_move() {
        let mesh = unwrapped(sheet(6, 6, |x, z| x * z * 0.4));
        // The layout's leftmost and rightmost vertices, the second pulled
        // further out, then both pinned.
        let layout = UvLayout::new(&mesh, 0).unwrap();
        let by_u = |a: &&crate::uv::UvVertex, b: &&crate::uv::UvVertex| a.uv.x.total_cmp(&b.uv.x);
        let left = layout.uv_vertices.iter().min_by(by_u).unwrap().vertex;
        let right = layout.uv_vertices.iter().max_by(by_u).unwrap().vertex;
        let corners = [left, right];
        let pulled = transform(&mesh, 0, &UvSelection::Vertices(vec![right]), &UvAffine::translation(vec2(0.05, 0.0)), Pivot::Origin).unwrap();
        let held: Vec<Vec2> = corners.iter().map(|&v| pulled.uv_sets[0][v as usize]).collect();
        let out = reflatten(&pulled, 0, &[0], &corners, None).unwrap();
        for (&v, at) in corners.iter().zip(held) {
            assert_eq!(out.uv_sets[0][v as usize], at, "pin {v} moved");
        }
        assert!(UvLayout::new(&out, 0).unwrap().folded_triangles().is_empty());
        // One pin: the island keeps its size and that pin its place.
        let out = reflatten(&pulled, 0, &[0], &[right], None).unwrap();
        assert_eq!(out.uv_sets[0][right as usize], pulled.uv_sets[0][right as usize]);
    }

    #[test]
    fn repacking_after_puts_every_island_in_the_square() {
        let mesh = unwrapped(cube());
        let moved = transform(&mesh, 0, &UvSelection::Islands(vec![0]), &UvAffine::translation(vec2(3.0, 0.0)), Pivot::Origin).unwrap();
        let out = reflatten(&moved, 0, &[0], &[], Some(&PackOptions::default())).unwrap();
        assert!(out.uv_sets[0].iter().all(|p| (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y)));
    }
}
