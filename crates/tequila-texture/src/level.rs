//! One level of a MIP pyramid, kept in as few bytes as its values allow.
//!
//! A picture is decoded to 32-bit floats, and kept so with its mipmaps it is
//! sixteen bytes a texel for a colour map with alpha, twelve without: 268 MB
//! for a 4K colour map, and the same for a 4K roughness map, which has one
//! channel's worth of information in three. A product with a dozen such maps
//! fills a 16 GB Mac on its textures. So a level keeps its values in the
//! smallest form that holds them, and decodes a texel as it is read.
//!
//! ## Nothing is lost where nothing has to be
//!
//! A picture from an 8-bit file has at most 256 values a channel, and every
//! one is decoded the same way: `srgb_eotf(b / 255)` for colour,
//! `b / 255` for numbers. Kept as those bytes, a level reads back the very
//! floats it was built from, to the last bit, at a quarter of the memory.
//! The same goes for 16-bit numbers (a TIFF height map) and for floats a half
//! float holds exactly (an EXR saved as half floats, as most are). Whether a
//! picture is one of these is found from its values, not assumed from where
//! it came from: each is tried, and taken only if every value comes back
//! unchanged.
//!
//! Only floats that are none of those are rounded, to half floats: about
//! three significant figures, a part in two thousand at worst, finer than
//! the eight bits most maps arrive in. A height keeps its floats instead
//! (see [`crate::mip::Precision`]).
//!
//! ## A grey picture is kept as one channel
//!
//! A roughness, metalness or mask map arrives as RGB with three equal
//! channels: a PNG or JPEG saved grey is expanded to colour as it is
//! decoded. Kept as one channel and read back as three equal ones, it is a
//! third of the size and samples exactly as it did. Alpha, where a picture
//! has it, stays beside it.
//!
//! The mip levels above the base are rounded once each, from the average in
//! linear light (see `crate::mip`), as they are stored.

use crate::image::{ColorSpace, Image};
use half::f16;
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::srgb_eotf;

/// How a level keeps its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// A byte a channel on the sRGB curve, alpha straight: a colour picture
    /// from an 8-bit file, as a graphics card keeps an sRGB texture.
    Srgb8,
    /// A byte a channel, straight: numbers from an 8-bit file.
    Unorm8,
    /// Sixteen bits a channel, 0 to 1, straight.
    Unorm16,
    /// Half floats.
    Half,
    /// 32-bit floats.
    Float,
}

impl Encoding {
    /// Bytes a value.
    pub fn bytes(self) -> usize {
        match self {
            Encoding::Srgb8 | Encoding::Unorm8 => 1,
            Encoding::Unorm16 | Encoding::Half => 2,
            Encoding::Float => 4,
        }
    }
}

#[derive(Clone)]
enum Texels {
    /// With the table the bytes decode through, carried here so that a
    /// lookup does not have to ask for it.
    Srgb8(Vec<u8>, &'static [f32; 256]),
    Unorm8(Vec<u8>),
    Unorm16(Vec<u16>),
    Half(Vec<f16>),
    Float(Vec<f32>),
}

/// One level of a pyramid: its size, and its texels in an [`Encoding`].
#[derive(Clone)]
pub struct Level {
    pub width: usize,
    pub height: usize,
    /// Channels as kept: 1 grey, 2 grey and alpha, 3 RGB, 4 RGBA.
    stored: usize,
    texels: Texels,
    pub color_space: ColorSpace,
}

impl std::fmt::Debug for Level {
    // The size and the form, not the tens of millions of values.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Level")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("stored_channels", &self.stored)
            .field("encoding", &self.encoding())
            .field("color_space", &self.color_space)
            .finish()
    }
}

impl Level {
    /// `values`, `stored` channels a texel, kept in `encoding` only if every
    /// one of them reads back unchanged.
    pub(crate) fn exact(width: usize, height: usize, stored: usize, values: &[f32], color_space: ColorSpace, encoding: Encoding) -> Option<Level> {
        let texels = pack(values, stored, encoding, true)?;
        Some(Level { width, height, stored, texels, color_space })
    }

    /// `values` kept in `encoding`, each rounded to the nearest value it holds.
    pub(crate) fn rounded(width: usize, height: usize, stored: usize, values: Vec<f32>, color_space: ColorSpace, encoding: Encoding) -> Level {
        let texels = match encoding {
            Encoding::Float => Texels::Float(values),
            other => pack(&values, stored, other, false).expect("rounding always packs"),
        };
        Level { width, height, stored, texels, color_space }
    }

    pub fn encoding(&self) -> Encoding {
        match self.texels {
            Texels::Srgb8(..) => Encoding::Srgb8,
            Texels::Unorm8(_) => Encoding::Unorm8,
            Texels::Unorm16(_) => Encoding::Unorm16,
            Texels::Half(_) => Encoding::Half,
            Texels::Float(_) => Encoding::Float,
        }
    }

    /// Channels as kept: 1 for a grey level, 2 for grey with alpha, 3 or 4
    /// for colour without or with it.
    pub fn stored_channels(&self) -> usize {
        self.stored
    }

    /// The channels an [`Image`] of this level has: four with alpha, three
    /// without. A grey level reads as three equal ones.
    pub fn channels(&self) -> usize {
        if self.has_alpha() {
            4
        } else {
            3
        }
    }

    pub fn has_alpha(&self) -> bool {
        self.stored % 2 == 0
    }

    /// Bytes the texels take.
    pub fn bytes(&self) -> usize {
        self.width * self.height * self.stored * self.encoding().bytes()
    }

    /// Texel at `(x, y)`, clamped to the level, as linear RGB: what
    /// [`Image::texel`] gave when levels were images.
    #[inline]
    pub fn texel(&self, x: usize, y: usize) -> Vec3 {
        let at = self.index(x, y);
        if self.stored < 3 {
            let grey = self.colour(at);
            vec3(grey, grey, grey)
        } else {
            vec3(self.colour(at), self.colour(at + 1), self.colour(at + 2))
        }
    }

    /// Red, green, blue and alpha at `(x, y)`, clamped to the level; 1 for
    /// the alpha of a level without one.
    #[inline]
    pub fn rgba(&self, x: usize, y: usize) -> [f32; 4] {
        let at = self.index(x, y);
        match self.stored {
            1 => {
                let grey = self.colour(at);
                [grey, grey, grey, 1.0]
            }
            2 => {
                let grey = self.colour(at);
                [grey, grey, grey, self.alpha(at + 1)]
            }
            3 => [self.colour(at), self.colour(at + 1), self.colour(at + 2), 1.0],
            _ => [self.colour(at), self.colour(at + 1), self.colour(at + 2), self.alpha(at + 3)],
        }
    }

    /// The level as a float image of three or four channels, for a caller
    /// that hands pictures on whole (Cycles takes them so).
    pub fn to_image(&self) -> Image {
        let channels = self.channels();
        let mut data = Vec::with_capacity(self.width * self.height * channels);
        for y in 0..self.height {
            for x in 0..self.width {
                data.extend_from_slice(&self.rgba(x, y)[..channels]);
            }
        }
        Image { width: self.width, height: self.height, channels, data, color_space: self.color_space }
    }

    #[inline]
    fn index(&self, x: usize, y: usize) -> usize {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        (y * self.width + x) * self.stored
    }

    #[inline]
    fn colour(&self, at: usize) -> f32 {
        match &self.texels {
            Texels::Srgb8(v, decode) => decode[usize::from(v[at])],
            Texels::Unorm8(v) => f32::from(v[at]) / 255.0,
            Texels::Unorm16(v) => f32::from(v[at]) / 65535.0,
            Texels::Half(v) => v[at].to_f32(),
            Texels::Float(v) => v[at],
        }
    }

    /// Alpha is coverage and never had the sRGB curve.
    #[inline]
    fn alpha(&self, at: usize) -> f32 {
        match &self.texels {
            Texels::Srgb8(v, _) => f32::from(v[at]) / 255.0,
            _ => self.colour(at),
        }
    }
}

/// The largest finite half float.
pub(crate) const HALF_MAX: f32 = 65504.0;

/// `values` in `encoding`, rounded to the nearest value it holds; or, when
/// `exact`, `None` unless every one of them reads back to the same bits.
fn pack(values: &[f32], stored: usize, encoding: Encoding, exact: bool) -> Option<Texels> {
    let keeps = |v: f32, back: f32| !exact || v.to_bits() == back.to_bits();
    Some(match encoding {
        Encoding::Srgb8 => {
            let srgb = srgb();
            // Colour channels are on the curve; the alpha, last of an even
            // number, is not.
            let colours = if stored % 2 == 0 { stored - 1 } else { stored };
            let mut out = Vec::with_capacity(values.len());
            for texel in values.chunks_exact(stored) {
                for (c, &v) in texel.iter().enumerate() {
                    let (b, back) = if c < colours {
                        let b = srgb.encode(v);
                        (b, srgb.decode[usize::from(b)])
                    } else {
                        let b = unorm8(v);
                        (b, f32::from(b) / 255.0)
                    };
                    if !keeps(v, back) {
                        return None;
                    }
                    out.push(b);
                }
            }
            Texels::Srgb8(out, &srgb.decode)
        }
        Encoding::Unorm8 => {
            let mut out = Vec::with_capacity(values.len());
            for &v in values {
                let b = unorm8(v);
                if !keeps(v, f32::from(b) / 255.0) {
                    return None;
                }
                out.push(b);
            }
            Texels::Unorm8(out)
        }
        Encoding::Unorm16 => {
            let mut out = Vec::with_capacity(values.len());
            for &v in values {
                let s = (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
                if !keeps(v, f32::from(s) / 65535.0) {
                    return None;
                }
                out.push(s);
            }
            Texels::Unorm16(out)
        }
        Encoding::Half => {
            let mut out = Vec::with_capacity(values.len());
            for &v in values {
                let h = f16::from_f32(v);
                if !keeps(v, h.to_f32()) {
                    return None;
                }
                out.push(h);
            }
            Texels::Half(out)
        }
        Encoding::Float => Texels::Float(values.to_vec()),
    })
}

#[inline]
fn unorm8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The sRGB curve, byte by byte, both ways.
struct Srgb {
    /// Each byte's linear value, exactly as [`Image::from_srgb8`] decodes it.
    decode: [f32; 256],
    /// The linear value halfway along the curve from each byte to the next:
    /// a value at or past `halfway[b]` is nearer `b + 1`.
    halfway: [f32; 255],
    /// For each run of floats sharing their top seventeen bits (sign,
    /// exponent and eight bits of mantissa), how many of `halfway` lie
    /// below the first of them, up to 1.0. A run is narrower than the gap
    /// between two bytes anywhere on the curve, so the search from there
    /// takes a step at most.
    below: Vec<u8>,
}

impl Srgb {
    /// The byte nearest `v` along the curve: the one a graphics card stores
    /// for it in an sRGB texture.
    #[inline]
    fn encode(&self, v: f32) -> u8 {
        // Not `v <= 0.0`: a NaN goes to black too.
        if !(v > 0.0) {
            return 0;
        }
        if v >= 1.0 {
            return 255;
        }
        let mut b = usize::from(self.below[(v.to_bits() >> 15) as usize]);
        while b < 255 && self.halfway[b] <= v {
            b += 1;
        }
        b as u8
    }
}

fn srgb() -> &'static Srgb {
    static TABLES: std::sync::OnceLock<Srgb> = std::sync::OnceLock::new();
    TABLES.get_or_init(|| {
        let decode = std::array::from_fn(|b| srgb_eotf(b as f32 / 255.0));
        let halfway: [f32; 255] = std::array::from_fn(|b| srgb_eotf((b as f32 + 0.5) / 255.0));
        let runs = (1.0f32.to_bits() >> 15) as usize;
        let below = (0..=runs)
            .map(|run| {
                let first = f32::from_bits((run as u32) << 15);
                halfway.partition_point(|&h| h < first) as u8
            })
            .collect();
        Srgb { decode, halfway, below }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every byte goes to its own decoded value and back; and a value
    /// between two bytes goes to the nearer along the curve, as rounding
    /// its encoded value would.
    #[test]
    fn the_srgb_table_rounds_to_the_nearest_byte_along_the_curve() {
        let srgb = srgb();
        for b in 0..=255u8 {
            assert_eq!(srgb.encode(srgb.decode[usize::from(b)]), b, "byte {b}");
        }
        let inverse = |v: f32| (tequila_core::spectrum::srgb_inverse_eotf(v) * 255.0 + 0.5) as u8;
        let mut v = 0.0f32;
        while v < 1.0 {
            let (ours, theirs) = (srgb.encode(v), inverse(v));
            // Within a hair of a halfway point the float curve and its
            // inverse may each round their own way.
            assert!(ours.abs_diff(theirs) <= 1, "{v}: {ours} against {theirs}");
            if ours != theirs {
                let edge = srgb.halfway[usize::from(ours.min(theirs))];
                assert!((v - edge).abs() < 1e-6, "{v} is not near a halfway point ({edge})");
            }
            v += 0.000_37;
        }
        assert_eq!((srgb.encode(-0.5), srgb.encode(f32::NAN), srgb.encode(7.0)), (0, 0, 255));
    }

    /// Floats an encoding cannot hold exactly are refused when exactness is
    /// asked for, and rounded otherwise.
    #[test]
    fn exactness_is_checked_value_by_value() {
        // 0.3 is 76.5 / 255.
        assert!(pack(&[0.0, 0.3, 1.0, 51.0 / 255.0], 1, Encoding::Unorm8, true).is_none(), "0.3 is no byte");
        assert!(pack(&[0.0, 1.0, 51.0 / 255.0], 1, Encoding::Unorm8, true).is_some());
        assert!(pack(&[0.5, 0.25, 1024.0], 1, Encoding::Half, true).is_some());
        assert!(pack(&[0.1], 1, Encoding::Half, true).is_none(), "0.1 is no half float");
        let Some(Texels::Half(rounded)) = pack(&[0.1], 1, Encoding::Half, false) else { panic!() };
        assert!((rounded[0].to_f32() - 0.1).abs() < 0.1 / 2048.0);
        // An sRGB level keeps its alpha straight.
        let half = 128.0 / 255.0;
        let Some(Texels::Srgb8(rgba, _)) = pack(&[srgb_eotf(half), 0.0, 1.0, half], 4, Encoding::Srgb8, true) else { panic!() };
        assert_eq!(rgba, vec![128, 0, 255, 128]);
        assert!(pack(&[half, 0.0, 1.0, half], 4, Encoding::Srgb8, true).is_none(), "a straight colour is not on the curve");
    }
}
