//! A checker on the part being edited, in the viewport: how the layout
//! lays a texture on the part, seen on the part itself - squares that
//! stretch where it stretches, and with a pattern that reads one way round,
//! islands turned or mirrored on it.
//!
//! It is for looking only. It is not saved, not rendered, not in the
//! material and not in the history: the editor hands it to the viewport
//! each frame - to the graphics card's shading
//! (`crate::solid_gpu::CheckerFrame`), or, where the processor draws Solid,
//! to its Solid pass (`tequila_render_core::preview::PreviewChecker`) -
//! which draws the part with it in place of its base colour. So it shows
//! in Wireframe, Solid and Material Preview, by whichever draws them, and
//! not in Rendered, which is the path tracer's picture of the scene as it
//! will render. The editor's canvas can show it behind the layout too.
//!
//! It follows the UV set the editor shows, or one picked: the scene's
//! meshes carry only their first set, so for another the editor hands the
//! viewport that set's places beside the checker.
//!
//! The patterns are drawn here, cell by cell: plain squares; squares with
//! an F in each, which reads backwards where an island is mirrored and
//! leans where it is turned; and cells numbered by row and column, each a
//! colour of its own.

use super::UvEditor;
use crate::project::Project;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Pattern {
    Plain,
    /// Each square with an F in it, one way round.
    #[default]
    Letters,
    /// Each square numbered, a colour of its own.
    Numbers,
}

impl Pattern {
    pub fn key(self) -> &'static str {
        match self {
            Pattern::Plain => "plain",
            Pattern::Letters => "letters",
            Pattern::Numbers => "numbers",
        }
    }

    pub fn from_key(key: &str) -> Option<Pattern> {
        match key {
            "arrows" | "letters" | "f" => Some(Pattern::Letters),
            "plain" => Some(Pattern::Plain),
            "numbers" | "numbered" => Some(Pattern::Numbers),
            _ => None,
        }
    }
}

/// The checker as the person has set it: never saved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Checker {
    pub on: bool,
    /// Squares across the layout's square.
    pub squares: u32,
    pub pattern: Pattern,
    /// The UV set it is drawn through: the one the editor shows, or this
    /// one.
    pub set: Option<usize>,
}

impl Default for Checker {
    fn default() -> Self {
        Checker { on: false, squares: 8, pattern: Pattern::Letters, set: None }
    }
}

/// The picture of the pattern, its side in pixels and its pixels, sRGB,
/// four bytes each, the top row first: row 0 is v = 0, as a texture's.
pub fn picture(squares: u32, pattern: Pattern) -> (usize, Vec<u8>) {
    let squares = squares.clamp(1, 64) as usize;
    let cell = (1024 / squares).max(16);
    let side = cell * squares;
    let mut out = vec![0u8; side * side * 4];
    let set = |out: &mut Vec<u8>, x: usize, y: usize, c: [u8; 3]| {
        let i = (y * side + x) * 4;
        out[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
    };
    for row in 0..squares {
        for column in 0..squares {
            let dark = (row + column) % 2 == 1;
            let (ground, ink) = match pattern {
                Pattern::Numbers => {
                    let hue = column as f32 / squares as f32;
                    let light = 0.78 - 0.4 * row as f32 / squares.max(2) as f32;
                    let c = hsl(hue, if dark { 0.55 } else { 0.42 }, if dark { light - 0.08 } else { light });
                    (c, [24, 24, 28])
                }
                _ => (if dark { [70, 72, 78] } else { [214, 216, 220] }, if dark { [214, 216, 220] } else { [70, 72, 78] }),
            };
            for y in 0..cell {
                for x in 0..cell {
                    set(&mut out, column * cell + x, row * cell + y, ground);
                }
            }
            let (x0, y0) = (column * cell, row * cell);
            match pattern {
                Pattern::Plain => {}
                Pattern::Letters => letter_f(&mut out, side, x0, y0, cell, ink),
                Pattern::Numbers => {
                    // The row's letter and the column's number, as "B4".
                    let label = format!("{}{}", (b'A' + (row % 26) as u8) as char, column + 1);
                    text(&mut out, side, x0, y0, cell, &label, ink);
                }
            }
        }
    }
    (side, out)
}

/// An F in a cell, upright as the layout reads: its stem down the left, its
/// arms to the right at the top and the middle.
fn letter_f(out: &mut [u8], side: usize, x0: usize, y0: usize, cell: usize, ink: [u8; 3]) {
    let unit = cell as f32 / 10.0;
    let block = |out: &mut [u8], x: f32, y: f32, w: f32, h: f32| {
        let (ax, ay) = ((x0 as f32 + x * unit) as usize, (y0 as f32 + y * unit) as usize);
        let (bx, by) = ((x0 as f32 + (x + w) * unit) as usize, (y0 as f32 + (y + h) * unit) as usize);
        for py in ay..by.min(side) {
            for px in ax..bx.min(side) {
                let i = (py * side + px) * 4;
                out[i..i + 3].copy_from_slice(&ink);
            }
        }
    };
    block(out, 3.0, 2.0, 1.2, 6.0);
    block(out, 3.0, 2.0, 4.0, 1.2);
    block(out, 3.0, 4.6, 3.0, 1.1);
}

/// Letters and figures in a five-by-three dot font of our own, in a cell's
/// top left.
fn text(out: &mut [u8], side: usize, x0: usize, y0: usize, cell: usize, label: &str, ink: [u8; 3]) {
    let glyph = |c: char| -> [u8; 5] {
        match c {
            '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
            '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
            '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
            '3' => [0b111, 0b001, 0b011, 0b001, 0b111],
            '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
            '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
            '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
            '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
            '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
            '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
            'A' => [0b010, 0b101, 0b111, 0b101, 0b101],
            'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
            'C' => [0b011, 0b100, 0b100, 0b100, 0b011],
            'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
            'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
            'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
            'G' => [0b011, 0b100, 0b101, 0b101, 0b011],
            'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
            _ => [0b111, 0b111, 0b111, 0b111, 0b111],
        }
    };
    let dot = (cell / 16).max(1);
    let mut x = x0 + dot * 2;
    let y = y0 + dot * 2;
    for c in label.chars() {
        let rows = glyph(c);
        for (r, bits) in rows.iter().enumerate() {
            for k in 0..3 {
                if bits & (0b100 >> k) != 0 {
                    for py in 0..dot {
                        for px in 0..dot {
                            let (qx, qy) = (x + k * dot + px, y + r * dot + py);
                            if qx < side && qy < side {
                                let i = (qy * side + qx) * 4;
                                out[i..i + 3].copy_from_slice(&ink);
                            }
                        }
                    }
                }
            }
        }
        x += dot * 4;
    }
}

/// A colour from hue, saturation and lightness, each 0 to 1.
fn hsl(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h * 6.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h * 6.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8]
}

/// The UV set's places, beside the checker, where it is not the first:
/// what they are for, a key for them, and the places.
pub type SetPlaces = ((tequila_scene::MeshKey, usize), u64, Arc<Vec<tequila_core::math::Vec2>>);

/// A hash of something, as a key to know when it changes.
fn key_of(value: impl std::hash::Hash) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

impl UvEditor {
    /// The UV set the checker is drawn through on a part with `sets`.
    pub(super) fn checker_set(&self, sets: usize) -> usize {
        self.checker.set.unwrap_or(self.shown_set).min(sets.saturating_sub(1))
    }

    /// The checker's picture, drawn again when its squares or pattern
    /// change.
    pub(super) fn checker_image(&mut self) -> Arc<(usize, Vec<u8>)> {
        let key = (self.checker.squares, self.checker.pattern);
        match &self.checker_picture {
            Some((k, image)) if *k == key => image.clone(),
            _ => {
                let image = Arc::new(picture(self.checker.squares, self.checker.pattern));
                self.checker_picture = Some((key, image.clone()));
                image
            }
        }
    }

    /// The part the checker is on this frame, its instance in the scene,
    /// and the places of the set it is drawn through where that is not the
    /// first: none while it is off or the editor shows no part.
    fn checked(&mut self, project: &Project) -> Option<(u32, Option<(u64, Arc<Vec<tequila_core::math::Vec2>>)>)> {
        if !self.open || !self.checker.on {
            return None;
        }
        let node = self.shown_part()?;
        let instance = project.mapping().instance_of(node)?;
        let document = project.document();
        let mesh_key = document.tree.get(node)?.mesh()?;
        let mesh = document.geometry.get(mesh_key)?;
        let set = self.checker_set(mesh.uv_sets.len());
        let places = match mesh.uv_sets.get(set) {
            Some(uvs) if set > 0 && uvs.len() == mesh.positions.len() => match &self.checker_places {
                Some((k, key, places)) if *k == (mesh_key, set) => Some((*key, places.clone())),
                _ => {
                    let key = key_of((mesh_key, set));
                    let places = Arc::new(uvs.clone());
                    self.checker_places = Some(((mesh_key, set), key, places.clone()));
                    Some((key, places))
                }
            },
            _ => None,
        };
        Some((instance.0, places))
    }

    /// What the graphics card draws the checker on this frame, while it is
    /// on and the editor shows a part: that part's instance in the scene,
    /// through the set chosen.
    pub fn checker_frame(&mut self, project: &Project) -> Option<crate::solid_gpu::CheckerFrame> {
        let (instance, uvs) = self.checked(project)?;
        let image = self.checker_image();
        let key = self.checker.squares as u64 * 4 + self.checker.pattern as u64;
        Some(crate::solid_gpu::CheckerFrame { instances: vec![instance], image, key, uvs })
    }

    /// The same for the processor's Solid, and what it is: made again only
    /// when the picture, the part or the set changes.
    pub fn cpu_checker(&mut self, project: &Project) -> Option<(u64, Arc<tequila_render_core::preview::PreviewChecker>)> {
        let (instance, uvs) = self.checked(project)?;
        let key = key_of((instance, self.checker.squares, self.checker.pattern as u8, uvs.as_ref().map(|u| u.0)));
        if let Some((k, checker)) = &self.cpu_checker {
            if *k == key {
                return Some((key, checker.clone()));
            }
        }
        let image = self.checker_image();
        let checker = Arc::new(tequila_render_core::preview::PreviewChecker::from_srgb(vec![instance], image.0, &image.1, uvs.map(|u| u.1)));
        self.cpu_checker = Some((key, checker.clone()));
        Some((key, checker))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_patterns_have_their_squares_and_read_one_way_round() {
        let (side, pixels) = picture(8, Pattern::Plain);
        assert_eq!(side % 8, 0);
        assert_eq!(pixels.len(), side * side * 4);
        let at = |pixels: &[u8], x: usize, y: usize| pixels[(y * side + x) * 4];
        let cell = side / 8;
        assert_ne!(at(&pixels, cell / 2, cell / 2), at(&pixels, cell + cell / 2, cell / 2), "neighbouring squares differ");
        assert_eq!(at(&pixels, cell / 2, cell / 2), at(&pixels, cell + cell / 2, cell + cell / 2), "and diagonal ones agree");
        // The F's arm runs right from its stem, not left: mirrored, it
        // would not.
        let (_, letters) = picture(8, Pattern::Letters);
        let ground = at(&letters, 1, 1);
        let arm_right = at(&letters, cell * 6 / 10, cell * 25 / 100);
        let left_of_stem = at(&letters, cell * 15 / 100, cell * 25 / 100);
        assert_ne!(arm_right, ground, "the top arm, right of the stem");
        assert_eq!(left_of_stem, ground, "nothing left of the stem");
        let (_, numbers) = picture(4, Pattern::Numbers);
        let c = side / 4;
        let colour = |x: usize, y: usize| &numbers[(y * side + x) * 4..(y * side + x) * 4 + 3];
        assert_ne!(colour(c / 2 + c * 3 / 10, c * 9 / 10), colour(c + c / 2 + c * 3 / 10, c * 9 / 10), "each column a colour of its own");
        assert_eq!(Pattern::from_key("letters"), Some(Pattern::Letters));
    }
}
