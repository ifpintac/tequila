//! Packing islands into a square, or any box of the layout.
//!
//! Each island is packed by its bounding box, as large as they all fit:
//! MaxRects (Jylänki 2010), each box put in the free space it fits most
//! snugly by its shorter leftover side, the free space kept as every
//! maximal rectangle still empty. The size is found by halving: the largest
//! scale at which every box goes in, to a part in a quarter of a million.
//! With turns allowed a box may go in on its side.
//!
//! The margin is a gap in the layout's own units - the unit square's side
//! is 1 - so it is the same however large the islands come out: a margin of
//! 0.005 is five pixels of a 1024 texture between islands, and two and a
//! half at the region's edges.

use super::edit::{chosen_islands, finish, layout_of, UvError};
use super::layout::tightest_box;
use super::transform::UvAffine;
use crate::geometry::Mesh;
use tequila_core::math::{vec2, Vec2};

/// A box of the layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvRect {
    pub min: Vec2,
    pub max: Vec2,
}

impl UvRect {
    pub const UNIT: UvRect = UvRect { min: Vec2::ZERO, max: Vec2::ONE };

    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }

    pub fn is_valid(&self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.max.x > self.min.x && self.max.y > self.min.y
    }
}

impl Default for UvRect {
    fn default() -> Self {
        UvRect::UNIT
    }
}

/// How islands are packed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PackOptions {
    /// The gap between islands, in the layout's units; half of it is left
    /// at the region's edges.
    pub margin: f32,
    /// Islands may go in on their side, a quarter turn from how they were.
    pub rotate: bool,
    /// Islands are first scaled to one density - the average of those
    /// packed - so the texture is as fine on each; else they keep the sizes
    /// they have to one another.
    pub normalize: bool,
    /// Each island is first turned so its tightest bounding box is square to
    /// the layout, which packs closer than a box round an island at a slant.
    pub orient: bool,
    /// Where they go.
    pub region: UvRect,
}

impl Default for PackOptions {
    fn default() -> Self {
        PackOptions { margin: 0.005, rotate: true, normalize: false, orient: false, region: UvRect::UNIT }
    }
}

/// What a pack did.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PackReport {
    pub islands: usize,
    /// How much the islands were scaled by, after any evening of density.
    pub scale: f32,
    /// How much of the region their area covers, 0 to 1.
    pub coverage: f32,
}

/// The islands - all of them, or those listed - packed into the region;
/// the others stay where they are.
pub fn pack(mesh: &Mesh, set: usize, islands: Option<&[u32]>, options: &PackOptions) -> Result<(Mesh, PackReport), UvError> {
    let layout = layout_of(mesh, set)?;
    let chosen = match islands {
        Some(list) => chosen_islands(&layout, list)?,
        None => (0..layout.islands.len() as u32).collect(),
    };
    if chosen.is_empty() {
        return Err(UvError::NothingChosen);
    }
    let shapes: Vec<Vec<Vec2>> = chosen.iter().map(|&i| layout.islands[i as usize].uv_vertices.iter().map(|&v| layout.uv_vertices[v as usize].uv).collect()).collect();
    let areas: Vec<f32> = chosen.iter().map(|&i| layout.islands[i as usize].uv_area).collect();
    let prescale: Vec<f32> = if options.normalize {
        let flat: f64 = chosen.iter().map(|&i| layout.islands[i as usize].uv_area as f64).sum();
        let surface: f64 = chosen.iter().map(|&i| layout.islands[i as usize].surface_area as f64).sum();
        let target = if surface > 0.0 { flat / surface } else { 0.0 };
        chosen
            .iter()
            .map(|&i| {
                let d = layout.islands[i as usize].density() as f64;
                if d > 0.0 && target > 0.0 { (target / d).sqrt() as f32 } else { 1.0 }
            })
            .collect()
    } else {
        vec![1.0; chosen.len()]
    };
    let (maps, report) = arrange(&shapes, &areas, &prescale, options)?;
    let mut uv: Vec<Vec2> = layout.uv_vertices.iter().map(|v| v.uv).collect();
    for (&i, map) in chosen.iter().zip(&maps) {
        for &v in &layout.islands[i as usize].uv_vertices {
            uv[v as usize] = map.apply(uv[v as usize]);
        }
    }
    Ok((finish(mesh, set, &layout.corner_vertex, &uv), report))
}

/// Where each shape goes: the map from where it is to its place in the
/// region. `areas` are the shapes' own, for the coverage; `prescale` each
/// shape's scale before packing.
pub(crate) fn arrange(shapes: &[Vec<Vec2>], areas: &[f32], prescale: &[f32], options: &PackOptions) -> Result<(Vec<UvAffine>, PackReport), UvError> {
    let region = if options.region.is_valid() { options.region } else { UvRect::UNIT };
    let room = region.size();
    let margin = options.margin.max(0.0);
    // Each shape in a box from the origin: scaled, turned square if asked.
    let mut local: Vec<UvAffine> = Vec::with_capacity(shapes.len());
    let mut sizes: Vec<(f32, f32)> = Vec::with_capacity(shapes.len());
    for (points, &k) in shapes.iter().zip(prescale) {
        let mut map = UvAffine::uniform(k);
        if options.orient && points.len() >= 3 {
            let (angle, _) = tightest_box(points);
            map = map.then(&UvAffine::rotation(-angle));
        }
        let (mut low, mut high) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        for p in points {
            let q = map.apply(*p);
            low = low.min(q);
            high = high.max(q);
        }
        if !(low.x <= high.x) {
            low = Vec2::ZERO;
            high = Vec2::ZERO;
        }
        local.push(map.then(&UvAffine::translation(-low)));
        let size = (high - low).max(Vec2::splat(1e-9));
        sizes.push((size.x, size.y));
    }
    let total: f64 = sizes.iter().map(|(w, h)| (*w as f64) * (*h as f64)).sum();
    // No larger than the boxes' area allows, or than the largest fits.
    let mut high = (room.x as f64 * room.y as f64 / total.max(1e-30)).sqrt() as f32;
    for &(w, h) in &sizes {
        let upright = ((room.x - margin) / w).min((room.y - margin) / h);
        let turned = ((room.x - margin) / h).min((room.y - margin) / w);
        high = high.min(if options.rotate { upright.max(turned) } else { upright });
    }
    if !(high > 0.0) {
        return Err(UvError::NoRoom);
    }
    // Two orders to try, largest side first and largest area first, where
    // there are few enough islands for it to cost nothing.
    let mut orders: Vec<Vec<usize>> = Vec::new();
    let mut by_side: Vec<usize> = (0..sizes.len()).collect();
    by_side.sort_by(|&a, &b| {
        let (sa, sb) = (sizes[a].0.max(sizes[a].1), sizes[b].0.max(sizes[b].1));
        sb.total_cmp(&sa).then((sizes[b].0 * sizes[b].1).total_cmp(&(sizes[a].0 * sizes[a].1))).then(a.cmp(&b))
    });
    orders.push(by_side);
    if sizes.len() <= 400 {
        let mut by_area: Vec<usize> = (0..sizes.len()).collect();
        by_area.sort_by(|&a, &b| (sizes[b].0 * sizes[b].1).total_cmp(&(sizes[a].0 * sizes[a].1)).then(a.cmp(&b)));
        orders.push(by_area);
    }
    let mut best: Option<(f32, Vec<(f32, f32, bool)>)> = None;
    for order in &orders {
        let fits = |s: f32| {
            let boxes: Vec<(f32, f32)> = sizes.iter().map(|(w, h)| (w * s + margin, h * s + margin)).collect();
            maxrects(&boxes, room.x, room.y, options.rotate, order)
        };
        let floor = best.as_ref().map(|(s, _)| *s).unwrap_or(0.0);
        let (mut low, mut top) = (floor, high);
        let mut placed: Option<(f32, Vec<(f32, f32, bool)>)> = None;
        if let Some(at) = fits(high) {
            placed = Some((high, at));
        } else {
            for _ in 0..18 {
                let mid = (low + top) * 0.5;
                if mid <= 0.0 {
                    break;
                }
                match fits(mid) {
                    Some(at) => {
                        low = mid;
                        placed = Some((mid, at));
                    }
                    None => top = mid,
                }
            }
            if placed.is_none() && floor == 0.0 {
                // The margins alone may not leave room: try the smallest size
                // worth a texel before giving up.
                let tiny = high * 1e-3;
                placed = fits(tiny).map(|at| (tiny, at));
            }
        }
        if let Some((s, at)) = placed {
            if best.as_ref().is_none_or(|(b, _)| s > *b) {
                best = Some((s, at));
            }
        }
    }
    let Some((scale, placement)) = best else { return Err(UvError::NoRoom) };
    let mut maps = Vec::with_capacity(shapes.len());
    let mut covered = 0.0f64;
    for (i, &(x, y, turned)) in placement.iter().enumerate() {
        let (_, h) = sizes[i];
        let mut map = local[i].then(&UvAffine::uniform(scale));
        if turned {
            // A quarter turn within its box: (x, y) to (h - y, x).
            map = map.then(&UvAffine::quarter_turns(1)).then(&UvAffine::translation(vec2(h * scale, 0.0)));
        }
        map = map.then(&UvAffine::translation(region.min + vec2(x, y) + Vec2::splat(margin * 0.5)));
        maps.push(map);
        covered += areas[i] as f64 * (prescale[i] * scale) as f64 * (prescale[i] * scale) as f64;
    }
    let coverage = (covered / (room.x as f64 * room.y as f64)) as f32;
    Ok((maps, PackReport { islands: shapes.len(), scale, coverage }))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Free {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Free {
    fn contains(&self, o: &Free, eps: f32) -> bool {
        o.x >= self.x - eps && o.y >= self.y - eps && o.x + o.w <= self.x + self.w + eps && o.y + o.h <= self.y + self.h + eps
    }

    fn overlaps(&self, o: &Free, eps: f32) -> bool {
        o.x < self.x + self.w - eps && o.x + o.w > self.x + eps && o.y < self.y + self.h - eps && o.y + o.h > self.y + eps
    }
}

/// Every box placed in a `width` by `height` area, in `order`, or None when
/// one does not go in: each box's corner, and whether it went in turned.
fn maxrects(boxes: &[(f32, f32)], width: f32, height: f32, rotate: bool, order: &[usize]) -> Option<Vec<(f32, f32, bool)>> {
    let eps = width.max(height) * 1e-7;
    let mut free = vec![Free { x: 0.0, y: 0.0, w: width, h: height }];
    let mut out = vec![(0.0f32, 0.0f32, false); boxes.len()];
    for &i in order {
        let (w, h) = boxes[i];
        // Best short side fit, then long side, then lowest and leftmost.
        let mut best: Option<(f32, f32, usize, bool)> = None;
        for (f, space) in free.iter().enumerate() {
            for turned in [false, true] {
                if turned && !rotate {
                    continue;
                }
                let (bw, bh) = if turned { (h, w) } else { (w, h) };
                if bw > space.w + eps || bh > space.h + eps {
                    continue;
                }
                let (dw, dh) = (space.w - bw, space.h - bh);
                let (short, long) = (dw.min(dh), dw.max(dh));
                let better = match best {
                    None => true,
                    Some((bs, bl, bf, _)) => {
                        short < bs || (short == bs && (long < bl || (long == bl && (space.y, space.x) < (free[bf].y, free[bf].x))))
                    }
                };
                if better {
                    best = Some((short, long, f, turned));
                }
            }
        }
        let (_, _, f, turned) = best?;
        let (bw, bh) = if turned { (h, w) } else { (w, h) };
        let placed = Free { x: free[f].x, y: free[f].y, w: bw, h: bh };
        out[i] = (placed.x, placed.y, turned);
        // Split every free rectangle the box lands in into what is left of
        // it, then drop any left inside another.
        let mut kept: Vec<Free> = Vec::with_capacity(free.len() + 4);
        let mut new: Vec<Free> = Vec::new();
        for space in &free {
            if !space.overlaps(&placed, eps) {
                kept.push(*space);
                continue;
            }
            if placed.x > space.x + eps {
                new.push(Free { x: space.x, y: space.y, w: placed.x - space.x, h: space.h });
            }
            if placed.x + placed.w < space.x + space.w - eps {
                new.push(Free { x: placed.x + placed.w, y: space.y, w: space.x + space.w - placed.x - placed.w, h: space.h });
            }
            if placed.y > space.y + eps {
                new.push(Free { x: space.x, y: space.y, w: space.w, h: placed.y - space.y });
            }
            if placed.y + placed.h < space.y + space.h - eps {
                new.push(Free { x: space.x, y: placed.y + placed.h, w: space.w, h: space.y + space.h - placed.y - placed.h });
            }
        }
        // Old rectangles were already none inside another; a new one may be
        // inside an old one or another new one, and an old one inside a new.
        let mut alive = vec![true; new.len()];
        for a in 0..new.len() {
            if kept.iter().any(|k| k.contains(&new[a], eps)) {
                alive[a] = false;
                continue;
            }
            for b in 0..new.len() {
                if a != b && alive[b] && new[b].contains(&new[a], eps) && (new[a] != new[b] || b < a) {
                    alive[a] = false;
                    break;
                }
            }
        }
        let fresh: Vec<Free> = new.into_iter().zip(alive).filter(|(_, a)| *a).map(|(n, _)| n).collect();
        kept.retain(|k| !fresh.iter().any(|n| n.contains(k, eps)));
        kept.extend(fresh);
        free = kept;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uv::tests_support::*;
    use crate::uv::UvLayout;

    /// No two islands' boxes overlap, and every island is inside the region.
    fn apart_and_inside(mesh: &Mesh, region: UvRect) {
        let layout = UvLayout::new(mesh, 0).unwrap();
        for (i, a) in layout.islands.iter().enumerate() {
            assert!(a.min.x >= region.min.x - 1e-5 && a.min.y >= region.min.y - 1e-5, "island {i} out: {:?}", a.min);
            assert!(a.max.x <= region.max.x + 1e-5 && a.max.y <= region.max.y + 1e-5, "island {i} out: {:?}", a.max);
            for b in &layout.islands[i + 1..] {
                let apart = a.max.x <= b.min.x + 1e-5 || b.max.x <= a.min.x + 1e-5 || a.max.y <= b.min.y + 1e-5 || b.max.y <= a.min.y + 1e-5;
                assert!(apart, "{:?}-{:?} and {:?}-{:?}", a.min, a.max, b.min, b.max);
            }
        }
        assert!(layout.folded_triangles().is_empty());
    }

    #[test]
    fn boxes_of_many_sizes_go_in_without_overlap() {
        let boxes: Vec<(f32, f32)> = (0..60).map(|i| (0.05 + (i % 7) as f32 * 0.013, 0.03 + (i % 5) as f32 * 0.021)).collect();
        let order: Vec<usize> = (0..boxes.len()).collect();
        let placed = maxrects(&boxes, 1.0, 1.0, true, &order).expect("they fit");
        let rects: Vec<Free> = placed.iter().zip(&boxes).map(|(&(x, y, t), &(w, h))| if t { Free { x, y, w: h, h: w } } else { Free { x, y, w, h } }).collect();
        for (i, a) in rects.iter().enumerate() {
            assert!(a.x >= 0.0 && a.y >= 0.0 && a.x + a.w <= 1.0 + 1e-6 && a.y + a.h <= 1.0 + 1e-6);
            for b in &rects[i + 1..] {
                assert!(!a.overlaps(b, 1e-6), "{a:?} {b:?}");
            }
        }
    }

    #[test]
    fn a_pack_fills_the_square_without_overlap_and_keeps_shapes() {
        let mesh = unwrapped(cube());
        let (out, report) = pack(&mesh, 0, None, &PackOptions { margin: 0.01, ..Default::default() }).unwrap();
        apart_and_inside(&out, UvRect::UNIT);
        // Six equal squares go two by three: half the square at least.
        assert!(report.coverage > 0.5, "{}", report.coverage);
        let layout = UvLayout::new(&out, 0).unwrap();
        assert!(layout.units_per_uv.is_some(), "as dense all over as before");
    }

    #[test]
    fn packing_into_a_region_keeps_inside_it_and_leaves_the_rest() {
        let mesh = unwrapped(cube());
        let before = UvLayout::new(&mesh, 0).unwrap();
        let region = UvRect { min: vec2(1.2, 0.1), max: vec2(1.8, 0.4) };
        let (out, _) = pack(&mesh, 0, Some(&[1, 3]), &PackOptions { region, ..Default::default() }).unwrap();
        let after = UvLayout::new(&out, 0).unwrap();
        for i in [1usize, 3] {
            let island = &after.islands[i];
            assert!(island.min.x >= region.min.x - 1e-5 && island.max.x <= region.max.x + 1e-5);
            assert!(island.min.y >= region.min.y - 1e-5 && island.max.y <= region.max.y + 1e-5);
        }
        for i in [0usize, 2, 4, 5] {
            assert_eq!(after.islands[i].min, before.islands[i].min, "island {i} moved");
        }
    }

    #[test]
    fn normalizing_evens_the_density_before_packing() {
        let mesh = unwrapped(cube());
        let big = super::super::transform(&mesh, 0, &super::super::UvSelection::Islands(vec![0]), &UvAffine::uniform(3.0), super::super::Pivot::Islands).unwrap();
        let (kept, _) = pack(&big, 0, None, &PackOptions::default()).unwrap();
        assert!(UvLayout::new(&kept, 0).unwrap().units_per_uv.is_none(), "kept: the big one is still bigger");
        let (even, _) = pack(&big, 0, None, &PackOptions { normalize: true, ..Default::default() }).unwrap();
        assert!(UvLayout::new(&even, 0).unwrap().units_per_uv.is_some());
        apart_and_inside(&even, UvRect::UNIT);
    }
}
