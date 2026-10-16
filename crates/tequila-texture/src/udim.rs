//! UDIM tile sets (`docs/02-MATERIAL-SYSTEM.md` §5.1).
//!
//! A single 4K image is not enough resolution for a whole product, so texture
//! artists lay a model's UVs across a *grid* of images instead: one for the
//! front panel, one for the lid, one for the base. UDIM is the naming
//! convention that says which image covers which square of UV space, and it is
//! near-universal — Mari, Substance, Maya and Blender all write it — so a
//! renderer that cannot read it cannot open most professional texture sets.
//!
//! The scheme is a four-digit number: `1001` covers `u` in 0–1 and `v` in 0–1,
//! `1002` covers `u` in 1–2, and every ten tiles wraps to the next row of `v`.
//! So the tile for a coordinate is
//!
//! ```text
//! 1001 + floor(u) + 10 * floor(v)
//! ```
//!
//! Files name it either by substituting into a `<UDIM>` placeholder
//! (`panel.<UDIM>.png`) or with an `_1001` suffix. Both spellings appear in the
//! wild and §5.1 asks for both.
//!
//! ## What happens off the grid
//!
//! UV coordinates outside any tile that exists are *normal*, not an error: a
//! model's UVs commonly cover only a handful of squares in a ten-by-ten grid,
//! and geometry the artist never textured lands nowhere. Those lookups return
//! nothing rather than wrapping into a neighbouring tile, because wrapping
//! would paint the lid's texture onto the underside of the base — which looks
//! like a texturing mistake and sends people hunting through their UVs.

use crate::mip::Wrap;
use crate::texture::Texture;
use tequila_core::math::Vec3;
use std::collections::BTreeMap;

/// The first tile: `u` and `v` both in 0–1.
pub const FIRST_TILE: u32 = 1001;

/// Tiles per row before `v` advances.
pub const TILES_PER_ROW: u32 = 10;

/// The UDIM number covering a coordinate, or `None` if it falls outside the
/// addressable grid.
///
/// Negative coordinates have no UDIM number at all — the scheme starts at the
/// origin and counts up — and neither does anything past the tenth column,
/// which would collide with the next row's numbering.
pub fn tile_of(u: f32, v: f32) -> Option<u32> {
    if !u.is_finite() || !v.is_finite() || u < 0.0 || v < 0.0 {
        return None;
    }
    let column = u.floor() as u32;
    let row = v.floor() as u32;
    if column >= TILES_PER_ROW {
        return None;
    }
    // Ten rows of ten is the conventional extent; beyond it the number would
    // run past four digits and no tool writes it.
    if row >= 100 {
        return None;
    }
    Some(FIRST_TILE + column + TILES_PER_ROW * row)
}

/// The `(column, row)` a UDIM number addresses.
pub fn origin_of(tile: u32) -> Option<(u32, u32)> {
    let offset = tile.checked_sub(FIRST_TILE)?;
    Some((offset % TILES_PER_ROW, offset / TILES_PER_ROW))
}

/// Expand a UDIM pattern into the filename for one tile.
///
/// Handles both spellings §5.1 names: a `<UDIM>` placeholder anywhere in the
/// path, or an `_1001`-style suffix on the stem.
pub fn expand_pattern(pattern: &str, tile: u32) -> String {
    if pattern.contains("<UDIM>") {
        return pattern.replace("<UDIM>", &tile.to_string());
    }
    // `panel_1001.png` -> `panel_1002.png`. Replace the last four-digit run
    // that follows an underscore, rather than the first number in the path:
    // a directory called `4k_textures` must not be mistaken for the tile.
    if let Some(found) = find_tile_suffix(pattern) {
        let mut out = String::with_capacity(pattern.len());
        out.push_str(&pattern[..found]);
        out.push_str(&tile.to_string());
        out.push_str(&pattern[found + 4..]);
        return out;
    }
    pattern.to_string()
}

/// Byte offset of the four-digit tile number in a `_1001`-style name, if there
/// is one.
fn find_tile_suffix(pattern: &str) -> Option<usize> {
    let bytes = pattern.as_bytes();
    let mut found = None;
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if bytes[i] == b'_'
            && bytes[i + 1..i + 5].iter().all(|b| b.is_ascii_digit())
            // Not part of a longer number.
            && bytes.get(i + 5).is_none_or(|b| !b.is_ascii_digit())
        {
            found = Some(i + 1);
        }
        i += 1;
    }
    found
}

/// A texture spread across a grid of UDIM tiles.
#[derive(Clone, Debug, Default)]
pub struct UdimSet {
    tiles: BTreeMap<u32, Texture>,
}

impl UdimSet {
    pub fn new() -> UdimSet {
        UdimSet {
            tiles: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, tile: u32, texture: Texture) {
        self.tiles.insert(tile, texture);
    }

    pub fn get(&self, tile: u32) -> Option<&Texture> {
        self.tiles.get(&tile)
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn tiles(&self) -> impl Iterator<Item = (u32, &Texture)> {
        self.tiles.iter().map(|(k, t)| (*k, t))
    }

    /// Look up a coordinate, choosing the tile and rebasing into it.
    ///
    /// Returns `None` where no tile covers the coordinate. A caller shading a
    /// surface treats that as "no texture here" and falls back to the
    /// material's own value.
    pub fn sample(&self, u: f32, v: f32, width: f32) -> Option<Vec3> {
        let tile = tile_of(u, v)?;
        let texture = self.tiles.get(&tile)?;

        // Within a tile the coordinate is measured from that tile's own corner.
        let local_u = u - u.floor();
        let local_v = v - v.floor();

        // Tiles are hard-edged: each covers exactly its own square and nothing
        // outside it. Repeating within a tile would tile the *tile*, which is
        // never what a UDIM set means.
        Some(
            texture
                .pyramid()
                .sample(local_u, local_v, width, Wrap::Clamp),
        )
    }

    /// Total texels across every tile and every MIP level, for a cache budget.
    pub fn texels(&self) -> usize {
        self.tiles.values().map(|t| t.pyramid().texels()).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{ColorSpace, Image};
    use crate::projection::Mapping;
    use crate::texture::MapRole;
    use tequila_core::math::vec3;

    fn flat(color: Vec3) -> Texture {
        let mut image = Image::constant(4, 4, color);
        image.color_space = ColorSpace::Raw;
        Texture::from_linear(image, MapRole::Data, Mapping::default())
    }

    /// The numbering, against the convention every tool writes.
    #[test]
    fn tile_numbers_follow_the_convention() {
        assert_eq!(tile_of(0.5, 0.5), Some(1001));
        assert_eq!(tile_of(1.5, 0.5), Some(1002));
        assert_eq!(tile_of(9.5, 0.5), Some(1010));
        // Ten tiles across, then the row advances.
        assert_eq!(tile_of(0.5, 1.5), Some(1011));
        assert_eq!(tile_of(3.5, 2.5), Some(1024));

        // ...and back again.
        for tile in [1001u32, 1002, 1010, 1011, 1024] {
            let (column, row) = origin_of(tile).expect("a valid tile");
            let recovered = tile_of(column as f32 + 0.5, row as f32 + 0.5);
            assert_eq!(recovered, Some(tile), "{tile} did not round-trip");
        }
    }

    /// Off the grid is a normal condition, not an error — and must not wrap.
    #[test]
    fn coordinates_off_the_grid_have_no_tile() {
        assert_eq!(tile_of(-0.1, 0.5), None, "negative u has no UDIM number");
        assert_eq!(tile_of(0.5, -0.1), None);
        assert_eq!(
            tile_of(10.5, 0.5),
            None,
            "an eleventh column would collide with the next row"
        );
        assert_eq!(tile_of(f32::NAN, 0.5), None);
        assert_eq!(tile_of(f32::INFINITY, 0.5), None);
    }

    /// Both spellings §5.1 names.
    #[test]
    fn both_naming_conventions_expand() {
        assert_eq!(
            expand_pattern("panel.<UDIM>.png", 1002),
            "panel.1002.png"
        );
        assert_eq!(
            expand_pattern("textures/<UDIM>/color.exr", 1011),
            "textures/1011/color.exr"
        );
        assert_eq!(expand_pattern("panel_1001.png", 1024), "panel_1024.png");

        // A number elsewhere in the path is not the tile. This is the case
        // that makes a naive "replace the first four digits" wrong.
        assert_eq!(
            expand_pattern("4k_textures/panel_1001.png", 1002),
            "4k_textures/panel_1002.png"
        );
        // Nothing to substitute leaves the name alone.
        assert_eq!(expand_pattern("panel.png", 1002), "panel.png");
    }

    /// Each tile covers its own square, and the coordinate is rebased into it.
    #[test]
    fn a_lookup_lands_in_the_right_tile() {
        let mut set = UdimSet::new();
        set.insert(1001, flat(vec3(1.0, 0.0, 0.0)));
        set.insert(1002, flat(vec3(0.0, 1.0, 0.0)));
        set.insert(1011, flat(vec3(0.0, 0.0, 1.0)));

        assert_eq!(set.sample(0.5, 0.5, 0.0).map(|c| c.x), Some(1.0));
        assert_eq!(set.sample(1.5, 0.5, 0.0).map(|c| c.y), Some(1.0));
        assert_eq!(set.sample(0.5, 1.5, 0.0).map(|c| c.z), Some(1.0));
    }

    /// A coordinate in a square with no file returns nothing, rather than
    /// wrapping into a neighbour — which would paint the lid's texture onto
    /// the base.
    #[test]
    fn a_missing_tile_returns_nothing_rather_than_a_neighbour() {
        let mut set = UdimSet::new();
        set.insert(1001, flat(vec3(1.0, 0.0, 0.0)));

        assert!(set.sample(0.5, 0.5, 0.0).is_some());
        assert_eq!(
            set.sample(1.5, 0.5, 0.0),
            None,
            "an untextured square borrowed its neighbour's image"
        );
        assert_eq!(set.sample(0.5, 3.5, 0.0), None);
        assert_eq!(set.sample(-0.5, 0.5, 0.0), None);
    }

    /// A tile does not repeat within itself. Tiling a *tile* is never what a
    /// UDIM set means, and it would show as the texture appearing twice across
    /// one square.
    #[test]
    fn tiles_do_not_repeat_within_themselves() {
        // A gradient, so a repeat would be visible as a restart.
        let size = 8;
        let mut data = vec![0.0f32; size * size * 3];
        for y in 0..size {
            for x in 0..size {
                for c in 0..3 {
                    data[(y * size + x) * 3 + c] = x as f32 / (size - 1) as f32;
                }
            }
        }
        let image = Image {
            width: size,
            height: size,
            channels: 3,
            data,
            color_space: ColorSpace::Raw,
        };
        let mut set = UdimSet::new();
        set.insert(
            1001,
            Texture::from_linear(image, MapRole::Data, Mapping::default()),
        );

        // Across the tile the gradient rises once and does not restart.
        let mut previous = f32::NEG_INFINITY;
        for i in 0..=20 {
            let u = i as f32 / 20.0 * 0.999;
            let value = set.sample(u, 0.5, 0.0).expect("inside the tile").x;
            assert!(
                value >= previous - 1e-4,
                "the gradient restarted at u = {u}: {value} after {previous}"
            );
            previous = value;
        }
    }

    #[test]
    fn the_set_reports_its_own_size() {
        let mut set = UdimSet::new();
        assert!(set.is_empty());
        set.insert(1001, flat(Vec3::ONE));
        set.insert(1002, flat(Vec3::ONE));
        assert_eq!(set.len(), 2);
        // Two 4x4 tiles, each with a pyramid: 16 + 4 + 1 texels apiece.
        assert_eq!(set.texels(), 2 * (16 + 4 + 1));
    }
}
