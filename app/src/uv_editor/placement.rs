//! A label's placement on the layout, worked out from its mapping: where its
//! corners and middle land, how large it is and how far it is turned, and
//! the same placement moved, sized or turned. Every edit the editor makes to
//! a label goes through these, so what it draws is where the label prints.

use tequila_core::math::{vec2, Vec2};
use tequila_texture::projection::Mapping;

/// How much of an island a fitted label fills, so its edge does not run
/// onto the seam.
pub const FIT: f32 = 0.9;

/// The smallest a label is made, in layout units: a label dragged down to
/// nothing could not be picked up again.
pub const SMALLEST: f32 = 1e-4;

/// The mapping's scale as repeats per unit of the layout, whichever way the
/// mapping states it.
pub fn repeats(mapping: &Mapping) -> Vec2 {
    let (u, v) = mapping.repeats();
    vec2(u, v)
}

pub fn with_repeats(mut mapping: Mapping, repeats: Vec2) -> Mapping {
    let repeats = vec2(repeats.x.max(1e-4), repeats.y.max(1e-4));
    if mapping.real_world_units {
        mapping.scale.x = 1.0 / repeats.x;
        mapping.scale.y = 1.0 / repeats.y;
    } else {
        mapping.scale.x = repeats.x;
        mapping.scale.y = repeats.y;
    }
    mapping
}

/// Where a point of the layout lands on the artwork, as every renderer
/// works it out.
pub fn artwork_at(mapping: &Mapping, uv: Vec2) -> Vec2 {
    let (u, v) = mapping.apply_to(uv.x, uv.y);
    vec2(u, v)
}

/// The point of the layout a point of the artwork lands on.
pub fn layout_at(mapping: &Mapping, art: Vec2) -> Vec2 {
    let (u, v) = mapping.unapply(art.x, art.y);
    vec2(u, v)
}

/// The artwork's corners on the layout, from its top left round.
pub fn corners(mapping: &Mapping) -> [Vec2; 4] {
    [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)].map(|c| layout_at(mapping, c))
}

/// The box square to the layout that holds the artwork, turned as it is.
pub fn bounds(mapping: &Mapping) -> (Vec2, Vec2) {
    let quad = corners(mapping);
    let low = quad.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
    let high = quad.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
    (low, high)
}

/// The point of the layout the artwork's middle is on.
pub fn centre(mapping: &Mapping) -> Vec2 {
    layout_at(mapping, vec2(0.5, 0.5))
}

/// The same placement with the artwork's middle at `at`, turning about its
/// middle: the pivot put there, as the Labels tab places a label.
pub fn centred(mut mapping: Mapping, at: Vec2) -> Mapping {
    // Placed by hand, it turns as a whole: a wide label turned stays wide.
    mapping.rigid = true;
    let s = repeats(&mapping);
    mapping.pivot = vec2(at.x * s.x, at.y * s.y);
    let middle = vec2(if mapping.flip_u { -0.5 } else { 0.5 }, if mapping.flip_v { -0.5 } else { 0.5 });
    mapping.offset = middle - mapping.pivot;
    mapping
}

/// The artwork's size on the layout.
pub fn size(mapping: &Mapping) -> Vec2 {
    let s = repeats(mapping);
    vec2(1.0 / s.x, 1.0 / s.y)
}

/// Its turn on the layout, in radians from a half turn back to a half turn
/// on, counter to the rotation field's.
pub fn turn(mapping: &Mapping) -> f32 {
    let half = std::f32::consts::PI;
    (-mapping.rotation * std::f32::consts::TAU + half).rem_euclid(2.0 * half) - half
}

pub fn turned(mut mapping: Mapping, angle: f32) -> Mapping {
    let at = centre(&mapping);
    mapping.rotation = (-angle / std::f32::consts::TAU).rem_euclid(1.0);
    centred(mapping, at)
}

pub fn resized(mapping: Mapping, to: Vec2) -> Mapping {
    let at = centre(&mapping);
    centred(with_repeats(mapping, vec2(1.0 / to.x.max(SMALLEST), 1.0 / to.y.max(SMALLEST))), at)
}

/// Moved by `delta`, as it is turned and sized.
pub fn moved(mapping: Mapping, delta: Vec2) -> Mapping {
    let at = centre(&mapping);
    centred(mapping, at + delta)
}

/// Turned by `angle` about `pivot`, as one of several turned together: its
/// middle goes round the pivot and the label turns with it.
pub fn turned_about(mapping: Mapping, pivot: Vec2, angle: f32) -> Mapping {
    let (sin, cos) = angle.sin_cos();
    let d = centre(&mapping) - pivot;
    let at = pivot + vec2(d.x * cos - d.y * sin, d.x * sin + d.y * cos);
    centred(turned(mapping, turn(&mapping) + angle), at)
}

/// Scaled by `k` across and down about `pivot`, as one of several scaled
/// together. A turned label cannot be stretched along the layout's axes and
/// stay a rectangle, so its own width and height take the share of each
/// that lies along them.
pub fn scaled_about(mapping: Mapping, pivot: Vec2, k: Vec2) -> Mapping {
    let (sin, cos) = turn(&mapping).sin_cos();
    let (c, s) = (cos.abs(), sin.abs());
    let own = vec2(c * k.x + s * k.y, s * k.x + c * k.y);
    let at = pivot + (centre(&mapping) - pivot) * k;
    centred(resized(mapping, size(&mapping) * own), at)
}

/// Mirrored where it sits: across its width (`across`) or its height.
pub fn flipped(mapping: Mapping, across: bool) -> Mapping {
    let at = centre(&mapping);
    centred(mapping.mirrored(across), at)
}

/// Mirrored about a line through `about`, as one of several flipped
/// together: its middle crosses the line and its turn is reflected.
pub fn flipped_about(mapping: Mapping, about: Vec2, across: bool) -> Mapping {
    let at = centre(&mapping);
    let to = if across { vec2(2.0 * about.x - at.x, at.y) } else { vec2(at.x, 2.0 * about.y - at.y) };
    // A reflection turns the other way: across a vertical line an angle a
    // becomes pi - a, which for a mirrored picture is -a.
    let reflected = turned(mapping, -turn(&mapping));
    centred(reflected.mirrored(across), to)
}

/// Inside the island, as large as it goes keeping the artwork's shape, in
/// the label's own turn, square on the island's middle.
pub fn fitted(mapping: Mapping, island: &[Vec2]) -> Mapping {
    let angle = turn(&mapping);
    let (sin, cos) = angle.sin_cos();
    // The island in the label's frame.
    let local: Vec<Vec2> = island.iter().map(|p| vec2(p.x * cos + p.y * sin, -p.x * sin + p.y * cos)).collect();
    let low = local.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
    let high = local.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
    let room = (high - low) * FIT;
    let shape = size(&mapping);
    let k = (room.x / shape.x.max(1e-9)).min(room.y / shape.y.max(1e-9));
    let middle = (low + high) * 0.5;
    let at = vec2(middle.x * cos - middle.y * sin, middle.x * sin + middle.y * cos);
    centred(with_repeats(mapping, vec2(1.0 / (shape.x * k).max(1e-5), 1.0 / (shape.y * k).max(1e-5))), at)
}

/// Square on the island's middle, as it is turned.
pub fn centred_on(mapping: Mapping, island: &[Vec2]) -> Mapping {
    let angle = turn(&mapping);
    let (sin, cos) = angle.sin_cos();
    let local: Vec<Vec2> = island.iter().map(|p| vec2(p.x * cos + p.y * sin, -p.x * sin + p.y * cos)).collect();
    let low = local.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
    let high = local.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
    let middle = (low + high) * 0.5;
    centred(mapping, vec2(middle.x * cos - middle.y * sin, middle.x * sin + middle.y * cos))
}

/// Turned to run along the island's length, the nearer of the two ways.
pub fn along(mapping: Mapping, axis: f32) -> Mapping {
    let now = turn(&mapping);
    let half = std::f32::consts::PI;
    let mut best = axis;
    for candidate in [axis, axis + half, axis - half] {
        let distance = |a: f32| ((a - now + half).rem_euclid(2.0 * half) - half).abs();
        if distance(candidate) < distance(best) {
            best = candidate;
        }
    }
    turned(mapping, best)
}

// ---- a texture's placement ----------------------------------------------------

/// The same texture mapping moved so the point `art` of the texture (0 to 1
/// across one tile) lands on `at` of the layout, nothing else changed: its
/// turn, scale, pivot and whether it is rigid are the texture's own, and a
/// texture laid on the part by a material is left meaning what it meant.
///
/// The mapping is a flip of a linear map plus its offset, so the offset that
/// puts one point in place is found from where that point lands now.
pub fn placed(mut mapping: Mapping, art: Vec2, at: Vec2) -> Mapping {
    let now = artwork_at(&mapping, at);
    let flip = |d: Vec2| vec2(if mapping.flip_u { -d.x } else { d.x }, if mapping.flip_v { -d.y } else { d.y });
    mapping.offset += flip(art - now);
    mapping
}

/// One tile of a texture, sized to `to` on the layout about its middle.
pub fn tile_resized(mapping: Mapping, to: Vec2) -> Mapping {
    let at = centre(&mapping);
    placed(with_repeats(mapping, vec2(1.0 / to.x.max(SMALLEST), 1.0 / to.y.max(SMALLEST))), vec2(0.5, 0.5), at)
}

/// One tile of a texture turned by `angle` on the layout about its middle.
pub fn tile_turned(mut mapping: Mapping, angle: f32) -> Mapping {
    let at = centre(&mapping);
    mapping.rotation = (mapping.rotation - angle / std::f32::consts::TAU).rem_euclid(1.0);
    placed(mapping, vec2(0.5, 0.5), at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_texture::projection::Projection;

    fn label() -> Mapping {
        let mut mapping = Mapping::default();
        mapping.projection = Projection::Uv;
        with_repeats(mapping, vec2(4.0, 8.0))
    }

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn the_artwork_lands_where_its_corners_say() {
        let mapping = turned(centred(label(), vec2(0.3, 0.6)), 0.7);
        for corner in [vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0), vec2(0.5, 0.5)] {
            assert!(near(artwork_at(&mapping, layout_at(&mapping, corner)), corner), "{corner:?}");
        }
        assert!(near(centre(&mapping), vec2(0.3, 0.6)), "turning keeps its middle where it was");
        let flipped = Mapping { flip_u: true, ..mapping };
        let flipped = centred(flipped, vec2(0.25, 0.25));
        assert!(near(artwork_at(&flipped, vec2(0.25, 0.25)), vec2(0.5, 0.5)), "a flipped label's middle too");
    }

    #[test]
    fn a_label_moves_scales_and_turns_about_its_middle() {
        let mapping = centred(label(), vec2(0.5, 0.5));
        assert!(near(size(&mapping), vec2(0.25, 0.125)));
        let moved = centred(mapping, vec2(0.6, 0.4));
        assert!(near(centre(&moved), vec2(0.6, 0.4)));
        let bigger = resized(moved, vec2(0.5, 0.25));
        assert!(near(centre(&bigger), vec2(0.6, 0.4)) && near(size(&bigger), vec2(0.5, 0.25)));
        let quarter = turned(bigger, std::f32::consts::FRAC_PI_2);
        let [a, b, ..] = corners(&quarter);
        // The top edge, a half wide, now runs down the layout.
        assert!(near(b - a, vec2(0.0, 0.5)) || near(b - a, vec2(0.0, -0.5)), "{:?}", b - a);
    }

    #[test]
    fn a_label_fits_an_island_keeping_its_shape() {
        let island = [vec2(0.1, 0.1), vec2(0.5, 0.1), vec2(0.5, 0.3), vec2(0.1, 0.3)];
        let fitted = fitted(label(), &island);
        let s = size(&fitted);
        assert!((s.x / s.y - 2.0).abs() < 1e-3, "two wide to one high, as the artwork is");
        assert!((s.x - 0.4 * FIT).abs() < 1e-4 || (s.y - 0.2 * FIT).abs() < 1e-4, "{s:?}");
        assert!(near(centre(&fitted), vec2(0.3, 0.2)));
        // Along a tilted island, the nearer way round.
        let along = along(fitted, 0.1);
        assert!((turn(&along) - 0.1).abs() < 1e-4);
    }

    /// Several labels turned and scaled together keep their places relative
    /// to each other: their middles go round, or out from, the shared pivot.
    #[test]
    fn labels_turn_and_scale_together_about_a_pivot() {
        let a = centred(label(), vec2(0.2, 0.5));
        let pivot = vec2(0.5, 0.5);
        let quarter = turned_about(a, pivot, std::f32::consts::FRAC_PI_2);
        assert!(near(centre(&quarter), vec2(0.5, 0.2)), "{:?}", centre(&quarter));
        assert!((turn(&quarter) - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        let double = scaled_about(a, pivot, vec2(2.0, 2.0));
        assert!(near(centre(&double), vec2(-0.1, 0.5)) && near(size(&double), size(&a) * 2.0));
        // A label turned a quarter takes the layout's stretch across as its
        // own stretch down.
        let wide = scaled_about(quarter, pivot, vec2(2.0, 1.0));
        assert!(near(size(&wide), vec2(size(&a).x, size(&a).y * 2.0)), "{:?}", size(&wide));
    }

    /// Flipped in place, a label keeps its square; flipped about a line, it
    /// crosses it.
    #[test]
    fn a_label_flips_in_place_and_about_a_line() {
        let a = turned(centred(label(), vec2(0.2, 0.3)), 0.4);
        let once = flipped(a, true);
        assert!(near(centre(&once), centre(&a)) && near(size(&once), size(&a)));
        assert!(once.flip_u != a.flip_u);
        let mirrored = flipped_about(a, vec2(0.5, 0.0), true);
        assert!(near(centre(&mirrored), vec2(0.8, 0.3)), "{:?}", centre(&mirrored));
        assert!((turn(&mirrored) + 0.4).abs() < 1e-4, "the turn is reflected: {}", turn(&mirrored));
        // Each corner of the mirror image is a corner of the original,
        // reflected.
        let reflect = |p: Vec2| vec2(1.0 - p.x, p.y);
        for corner in corners(&mirrored) {
            assert!(corners(&a).iter().any(|c| near(reflect(*c), corner)), "{corner:?}");
        }
    }

    /// A texture's tile moved, sized and turned by hand keeps everything else
    /// about its mapping: a texture that is not rigid stays so.
    #[test]
    fn a_texture_tile_moves_without_becoming_a_label() {
        let mut mapping = Mapping::default();
        mapping.scale.x = 3.0;
        mapping.scale.y = 5.0;
        mapping.rotation = 0.1;
        mapping.flip_v = true;
        let moved = placed(mapping, vec2(0.5, 0.5), vec2(0.4, 0.45));
        assert!(near(centre(&moved), vec2(0.4, 0.45)), "{:?}", centre(&moved));
        assert_eq!((moved.rigid, moved.pivot, moved.rotation, moved.scale), (mapping.rigid, mapping.pivot, mapping.rotation, mapping.scale));
        let bigger = tile_resized(moved, vec2(0.5, 0.25));
        assert!(near(centre(&bigger), vec2(0.4, 0.45)) && near(size(&bigger), vec2(0.5, 0.25)));
        let turned = tile_turned(bigger, 0.3);
        assert!(near(centre(&turned), vec2(0.4, 0.45)));
        assert!(!turned.rigid);
    }
}
