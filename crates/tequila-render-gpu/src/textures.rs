//! Pictures on the card (`docs/07` R3 tier A, image textures): every level
//! of every pyramid a material samples, exactly as `tequila-texture` keeps
//! it (`level.rs`: sRGB and plain bytes, sixteen-bit numbers, half floats,
//! floats, one to four channels), in one buffer of words, with a small
//! table of where each pyramid and level is; `textures.wgsl` reads them and
//! filters as `mip.rs` does, line for line. No hardware sampler: its
//! filtering is not the processor's to the bit, and a card's sampler
//! arrays are awkward on Metal.
//!
//! The header (`texture_header`, binding 17):
//! - words `[0, 256)`: the sRGB curve's table, each byte's linear value as
//!   `Level` decodes it (`srgb_decode_table`);
//! - `[256, 512)`: `b / 255` for an eight-bit number;
//! - `[512, 544)`: `log2(1 + k / 16)` for `k` in `0..16`, high and low
//!   halves, and `[544, 546)` `2 / ln 2` so, for the level's logarithm;
//! - `[548, 582)`: `atan(k / 16)` for `k` in `0..=16`, `[582, 584)`
//!   pi / 2 and `[584, 586)` pi, high and low halves, for the arctangent;
//! - `[588, 622)`: `2^(j / 16)` for `j` in `-8..=8` and `[622, 624)`
//!   ln 2, high and low halves, for powf;
//! - from [`PYRAMIDS`]: per pyramid [`PYRAMID_WORDS`] words, its first
//!   level's record (a word index into the header), its level count, its
//!   resolution (`max(width, height) as f32`, bits) and nought;
//! - then per level [`LEVEL_WORDS`]: its first word in the texel buffer,
//!   width, height, and its encoding with its stored channels above bit 8.
//!
//! The texels (`texture_texels`, binding 16): each level from a word of its
//! own; bytes four to a word, sixteen-bit values two, floats one, the first
//! in the low bits.

use std::collections::HashMap;
use tequila_materials::graph::{Instruction, TextureId};
use tequila_render_core::scene::SceneSnapshot;
use tequila_texture::level::{Level, RawTexels};
use tequila_texture::mip::MipPyramid;
use tequila_texture::texture::MapRole;

/// `textures.wgsl`'s encodings, by [`Level`]'s.
pub const ENCODING_SRGB8: u32 = 0;
pub const ENCODING_UNORM8: u32 = 1;
pub const ENCODING_UNORM16: u32 = 2;
pub const ENCODING_HALF: u32 = 3;
pub const ENCODING_FLOAT: u32 = 4;

/// Where the per-pyramid records start in the header.
pub const PYRAMIDS: usize = 624;
pub const PYRAMID_WORDS: usize = 4;
pub const LEVEL_WORDS: usize = 4;
/// The most pyramids an instruction can name (`program.rs`'s settings word).
pub const MAX_PYRAMIDS: usize = 1 << 14;

/// A picture as a material's program reads it on the card: its colour
/// levels, its alpha's (where it has alpha), whether a lookup is a colour
/// to bring into the rendering space (`shade.rs`'s `colour_of`), and
/// whether it is a normal map (`BumpSource::of`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardTexture {
    pub colour: u32,
    pub alpha: Option<u32>,
    pub colour_managed: bool,
    pub normal: bool,
}

/// What the lowering of a program knows of the scene's pictures.
#[derive(Clone, Copy, Debug)]
pub enum Pictures<'a> {
    /// Nothing: an image texture is refused (a program lowered on its own).
    Unknown,
    /// The scene's pictures, uploaded.
    On(&'a CardPictures),
    /// The scene's pictures take more than the card binds: every lookup is
    /// refused, with the sizes.
    TooLarge { bytes: u64, limit: u64 },
}

/// Every picture a scene's programs sample, packed for the card.
#[derive(Clone, Debug, Default)]
pub struct CardPictures {
    pub header: Vec<u32>,
    pub texels: Vec<u32>,
    pyramids: usize,
    /// The level records, appended after the pyramids' once all are known.
    levels: Vec<[u32; LEVEL_WORDS]>,
    /// Each pyramid's (first level index into `levels`, count, resolution).
    records: Vec<[u32; 3]>,
    textures: HashMap<TextureId, CardTexture>,
}

/// The header's fixed tables: the sRGB curve, eight-bit numbers, and the
/// level logarithm's constants.
pub fn header_tables() -> Vec<u32> {
    let mut words: Vec<u32> = tequila_texture::level::srgb_decode_table().iter().map(|v| v.to_bits()).collect();
    words.extend((0..256u32).map(|b| (b as f32 / 255.0).to_bits()));
    for k in 0..16 {
        let exact = (1.0 + k as f64 / 16.0).log2();
        let high = exact as f32;
        let low = (exact - high as f64) as f32;
        words.extend_from_slice(&[high.to_bits(), low.to_bits()]);
    }
    let split = |v: f64| {
        let high = v as f32;
        [high.to_bits(), ((v - high as f64) as f32).to_bits()]
    };
    words.extend_from_slice(&split(2.0 / std::f64::consts::LN_2));
    words.resize(548, 0);
    for k in 0..=16 {
        words.extend_from_slice(&split((k as f64 / 16.0).atan()));
    }
    words.extend_from_slice(&split(std::f64::consts::FRAC_PI_2));
    words.extend_from_slice(&split(std::f64::consts::PI));
    words.resize(588, 0);
    for j in -8..=8 {
        words.extend_from_slice(&split((j as f64 / 16.0).exp2()));
    }
    words.extend_from_slice(&split(std::f64::consts::LN_2));
    words.resize(PYRAMIDS, 0);
    words
}

impl CardPictures {
    /// The pictures `scene`'s programs sample: each picture's colour levels
    /// where an image texture reads it, its alpha's where an image's alpha
    /// does. A picture not loaded has nothing to upload (the program takes
    /// its fallback).
    pub fn of(scene: &SceneSnapshot) -> CardPictures {
        let mut pictures = CardPictures::default();
        let mut alpha_wanted: Vec<TextureId> = Vec::new();
        for program in scene.programs.iter().flatten() {
            for instruction in &program.instructions {
                let (id, alpha) = match instruction {
                    Instruction::Sample { texture, .. } => (*texture, false),
                    Instruction::SampleAlpha { texture, .. } => (*texture, true),
                    _ => continue,
                };
                let Some(loaded) = scene.textures.get(id) else { continue };
                if !pictures.textures.contains_key(&id) {
                    let colour = pictures.add(loaded.pyramid());
                    let colour_managed = loaded.role == MapRole::Color && loaded.source_color_space != tequila_texture::image::ColorSpace::Raw;
                    pictures.textures.insert(id, CardTexture { colour, alpha: None, colour_managed, normal: loaded.role == MapRole::Normal });
                }
                if alpha && !alpha_wanted.contains(&id) {
                    alpha_wanted.push(id);
                    if let Some(levels) = loaded.alpha_pyramid() {
                        let index = pictures.add(levels);
                        pictures.textures.get_mut(&id).expect("added above").alpha = Some(index);
                    }
                }
            }
        }
        pictures.finish();
        pictures
    }

    /// One pyramid's levels into the texel words, and its index.
    pub fn add(&mut self, pyramid: &MipPyramid) -> u32 {
        let first = self.levels.len() as u32;
        for i in 0..pyramid.levels() {
            let level = pyramid.level(i);
            let offset = self.texels.len() as u32;
            let encoding = pack(level, &mut self.texels);
            self.levels.push([offset, level.width as u32, level.height as u32, encoding | (level.stored_channels() as u32) << 8]);
        }
        let base = pyramid.base();
        self.records.push([first, pyramid.levels() as u32, (base.width.max(base.height) as f32).to_bits()]);
        self.pyramids += 1;
        self.pyramids as u32 - 1
    }

    /// The header, once every pyramid is in.
    pub fn finish(&mut self) {
        let mut header = header_tables();
        let levels_at = PYRAMIDS + PYRAMID_WORDS * self.records.len();
        for [first, count, resolution] in &self.records {
            header.extend_from_slice(&[(levels_at + LEVEL_WORDS * *first as usize) as u32, *count, *resolution, 0]);
        }
        for level in &self.levels {
            header.extend_from_slice(level);
        }
        self.header = header;
        // A buffer may not be empty.
        if self.texels.is_empty() {
            self.texels.push(0);
        }
    }

    /// How the card reads picture `id`, or `None` where it is not loaded.
    pub fn texture(&self, id: TextureId) -> Option<CardTexture> {
        self.textures.get(&id).copied()
    }

    /// How many pyramids there are.
    pub fn pyramids(&self) -> usize {
        self.pyramids
    }

    /// Bytes the texels take on the card.
    pub fn texel_bytes(&self) -> u64 {
        self.texels.len() as u64 * 4
    }

    /// Bytes the header takes on the card.
    pub fn header_bytes(&self) -> u64 {
        self.header.len() as u64 * 4
    }
}

/// A level's values onto `words`, from a word of its own; its encoding.
fn pack(level: &Level, words: &mut Vec<u32>) -> u32 {
    let bytes = |values: &[u8], words: &mut Vec<u32>| {
        for chunk in values.chunks(4) {
            words.push(chunk.iter().enumerate().fold(0u32, |w, (i, b)| w | (*b as u32) << (8 * i)));
        }
    };
    let halves = |values: &mut dyn Iterator<Item = u16>, words: &mut Vec<u32>| {
        let all: Vec<u16> = values.collect();
        for chunk in all.chunks(2) {
            words.push(chunk.iter().enumerate().fold(0u32, |w, (i, h)| w | (*h as u32) << (16 * i)));
        }
    };
    match level.raw() {
        RawTexels::Srgb8(v) => {
            bytes(v, words);
            ENCODING_SRGB8
        }
        RawTexels::Unorm8(v) => {
            bytes(v, words);
            ENCODING_UNORM8
        }
        RawTexels::Unorm16(v) => {
            halves(&mut v.iter().copied(), words);
            ENCODING_UNORM16
        }
        RawTexels::Half(v) => {
            halves(&mut v.iter().map(|h| h.to_bits()), words);
            ENCODING_HALF
        }
        RawTexels::Float(v) => {
            words.extend(v.iter().map(|f| f.to_bits()));
            ENCODING_FLOAT
        }
    }
}

/// What of `scene`'s pictures the card renders otherwise than the processor
/// does under `config`, one sentence each. The pictures themselves are the
/// processor's; what is not on the card yet each material says, as a
/// program refusal (labels, normal maps). `config` is for symmetry with the
/// other `*_limits`: nothing here depends on it.
pub fn texture_limits(scene: &SceneSnapshot, _config: &tequila_render_core::settings::IntegratorConfig) -> Vec<&'static str> {
    let mut limits = Vec::new();
    let mut wanted: Vec<(TextureId, bool)> = Vec::new();
    for program in scene.programs.iter().flatten() {
        for instruction in &program.instructions {
            let want = match instruction {
                Instruction::Sample { texture, .. } => (*texture, false),
                Instruction::SampleAlpha { texture, .. } => (*texture, true),
                _ => continue,
            };
            if scene.textures.get(want.0).is_some() && !wanted.contains(&want) {
                wanted.push(want);
            }
        }
    }
    if wanted.len() > MAX_PYRAMIDS {
        limits.push("past the card's 16,384 pictures, a material renders as its constants");
    }
    limits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::*;

    /// `textures.wgsl` reads the header and the settings word by the same
    /// numbers this file and `program.rs` write them by.
    #[test]
    fn the_layout_and_the_flags_are_textures_wgsls() {
        let wgsl = crate::shader::module("textures").expect("textures.wgsl");
        for (name, value) in [
            ("TEX_TABLE_UNORM8", "256u"),
            ("TEX_TABLE_LOG2", "512u"),
            ("TEX_LOG2_K", "544u"),
            ("TEX_TABLE_ATAN", "548u"),
            ("TEX_HALF_PI", "582u"),
            ("TEX_PI", "584u"),
            ("TEX_TABLE_EXP2", "588u"),
            ("TEX_LN2", "622u"),
            ("TEX_PYRAMIDS", &format!("{PYRAMIDS}u")),
            ("TEX_MODE_COLOUR", &format!("{TEXTURE_COLOUR}u")),
            ("TEX_MODE_ALPHA", &format!("{TEXTURE_ALPHA}u")),
            ("TEX_MODE_PROJECT", &format!("{TEXTURE_PROJECT}u")),
            ("TEX_MODE_NORMAL", &format!("{TEXTURE_NORMAL}u")),
            ("TEX_FLIP_U", &format!("0x{TEXTURE_FLIP_U:X}u")),
            ("TEX_FLIP_V", &format!("0x{TEXTURE_FLIP_V:X}u")),
            ("TEX_REAL_WORLD", &format!("0x{TEXTURE_REAL_WORLD:X}u")),
            ("TEX_RIGID", &format!("0x{TEXTURE_RIGID:X}u")),
            ("TEX_ROTATES", &format!("0x{TEXTURE_ROTATES:X}u")),
            ("TEX_AT", &format!("0x{TEXTURE_AT:X}u")),
            ("TEX_MANAGED", &format!("0x{TEXTURE_MANAGED:X}u")),
        ] {
            let line = format!("const {name}: u32 = {value};");
            assert!(wgsl.contains(&line), "textures.wgsl has no `{line}`");
        }
        assert_eq!(header_tables().len(), PYRAMIDS);
        assert_eq!(TEXTURE_PYRAMID_SHIFT as usize + 14, 32, "the pyramid takes the settings word's last fourteen bits");
        assert_eq!(1usize << 14, MAX_PYRAMIDS);
    }
}
