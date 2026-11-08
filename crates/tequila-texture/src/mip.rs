//! MIP pyramids and filtered texture lookup (`docs/02-MATERIAL-SYSTEM.md` §5.1).
//!
//! §5.1's requirement is one sentence — "never point-sample a full-resolution
//! texture for a distant surface" — and the reason is worth stating, because it
//! is the difference between a render that looks professional and one that does
//! not.
//!
//! A surface far from the camera has many texels falling inside one pixel. Point
//! sampling picks one of them and discards the rest, so the pixel's value
//! depends on exactly where the ray happened to land. Move the camera a
//! hair and a different texel wins. In a still frame that reads as sparkling
//! noise on distant detail; in an animation it crawls and shimmers, and no
//! amount of extra samples per pixel removes it, because every sample is
//! answering a slightly different question. Path tracing does average many rays
//! per pixel, which softens it — but a checkerboard at a distance converges to a
//! grey mush of the wrong brightness rather than to the correct average, because
//! the sample distribution across the footprint is not uniform.
//!
//! The fix is to pre-average. Each level of the pyramid halves the resolution
//! and stores the average of four texels; a lookup picks the level whose texels
//! are about the size of the ray's footprint on the surface and blends between
//! the two nearest levels. That footprint comes from ray differentials, which
//! the integrator already tracks — the same quantity §5.4 uses to keep bump
//! mapping from strengthening as you zoom out.
//!
//! ## What this is not
//!
//! The spec names OpenImageIO for the production pipeline, and OIIO's value is
//! twofold: this filtering, and a very wide set of file formats. This module is
//! the first half, written directly against [`Image`] so that no C++ toolchain
//! stands between the renderer and correct filtering. The second half turned
//! out to be within reach as well: the formats the spec lists, and several it
//! does not, are read in pure Rust behind [`Image::load`] (see
//! [`Image::DECODABLE`]). What OIIO would still add is tiled, on-demand
//! residency, and the [`Image`] seam is unchanged so it can be dropped in
//! behind this.
//!
//! ## What a level costs
//!
//! Levels are kept in as few bytes as their values allow ([`Level`]):
//! a picture from an 8-bit file as its bytes, exactly; other floats as half
//! floats; a grey map as one channel. The averaging that builds each level
//! is done in 32-bit floats in linear light, from the full-precision level
//! below rather than from the rounded one, and each level is rounded once,
//! as it is stored.

use crate::image::{ColorSpace, Image};
use crate::level::{Encoding, Level, HALF_MAX};
use tequila_core::math::Vec3;

/// What happens to a coordinate outside the unit square.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Wrap {
    /// Tile. The default, and what a `repeat` of more than 1 means.
    #[default]
    Repeat,
    /// Hold the edge texel. Correct for a label or decal, where tiling would
    /// smear the border colour across the surface.
    Clamp,
    /// Tile, reflecting alternate copies, so tiles meet without a seam.
    Mirror,
}

impl Wrap {
    /// Map a coordinate into `[0, 1)`, or report that it falls outside.
    ///
    /// `None` only for [`Wrap::Clamp`]... which does *not* return `None`: it
    /// clamps. The option exists for the caller that wants to know it left the
    /// texture, which a decal needs in order to stay transparent outside its
    /// own bounds rather than painting the edge colour everywhere.
    #[inline]
    pub fn apply(self, t: f32) -> f32 {
        match self {
            Wrap::Repeat => t - t.floor(),
            Wrap::Clamp => t.clamp(0.0, 1.0),
            Wrap::Mirror => {
                let two = (t * 0.5).fract() * 2.0;
                let two = if two < 0.0 { two + 2.0 } else { two };
                if two > 1.0 {
                    2.0 - two
                } else {
                    two
                }
            }
        }
    }

    /// Whether a coordinate lies inside the texture's own extent, before
    /// wrapping. A decal uses this to leave the rest of the surface alone.
    #[inline]
    pub fn is_inside(self, t: f32) -> bool {
        match self {
            Wrap::Repeat | Wrap::Mirror => true,
            Wrap::Clamp => (0.0..=1.0).contains(&t),
        }
    }
}

/// How much a pyramid may round what it keeps (see [`Level`]).
///
/// A value an 8-bit or 16-bit file held, or one a half float holds exactly,
/// is kept exactly whatever this says; this decides what happens to the
/// rest, and to the mip levels, whose averages fall between the values the
/// file had.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Precision {
    /// What the eye can tell apart: other floats as half floats, a part in
    /// two thousand; mip levels of a colour picture from an 8-bit file on
    /// the sRGB curve in eight bits, as a graphics card keeps them, and of
    /// numbers from one in sixteen, since a number may drive a bump.
    #[default]
    Display,
    /// What a bump can tell apart. A bump is the slope of a height, measured
    /// a tenth of a footprint apart, and rounding a height into steps shows
    /// as terraces in its shading: other floats stay floats, and the mip
    /// levels of a map from an 8- or 16-bit file are kept in sixteen bits.
    Height,
    /// Nothing rounded: other floats and every mip level kept as 32-bit
    /// floats. What a picture too small for rounding to save anything gets.
    Lossless,
}

/// The texels in a base level from which a pyramid is rounded at all. Below
/// 256 x 256 a pyramid is a megabyte at most as floats: rounding saves
/// nothing worth having, and a picture made rather than read - a
/// placeholder, a swatch, a test's fixture - reads back to the last bit.
const ROUNDED_FROM: usize = 256 * 256;

/// A texture and its MIP pyramid.
///
/// Level 0 is the original image; each subsequent level is half the size in
/// each axis, rounded up, holding the average of the texels beneath it.
#[derive(Clone, Debug)]
pub struct MipPyramid {
    levels: Vec<Level>,
}

impl MipPyramid {
    /// Build the pyramid, averaging down to a single texel, at
    /// [`Precision::Display`].
    ///
    /// The reduction is a plain box filter over the four parents. Better
    /// kernels exist and OIIO offers several; a box filter is what makes the
    /// energy claim below exactly true, which is what the furnace-style test
    /// in this module checks. A Gaussian or Lanczos reduction would be
    /// marginally sharper and would no longer preserve the mean exactly.
    ///
    /// The image may have one channel (grey) or two (grey and alpha) as
    /// well as three or four.
    pub fn build(base: Image) -> MipPyramid {
        MipPyramid::build_with(base, Precision::Display)
    }

    /// Build the pyramid, rounding no more than `precision` allows.
    pub fn build_with(base: Image, precision: Precision) -> MipPyramid {
        let Image { width, height, channels, data, color_space } = base;
        let (stored, data) = kept_channels(channels, data);
        let precision = if width * height < ROUNDED_FROM { Precision::Lossless } else { precision };

        // Numbers are tried as numbers first: a map of only black and white
        // is both, and its mip levels are better kept straight.
        let tries = if color_space == ColorSpace::Raw {
            [Encoding::Unorm8, Encoding::Srgb8, Encoding::Unorm16, Encoding::Half]
        } else {
            [Encoding::Srgb8, Encoding::Unorm8, Encoding::Unorm16, Encoding::Half]
        };
        let exact = tries.into_iter().find_map(|encoding| Level::exact(width, height, stored, &data, color_space, encoding));
        let mut next = (width > 1 || height > 1).then(|| halve(&data, width, height, stored));
        let base = exact.unwrap_or_else(|| {
            let half = precision == Precision::Display && data.iter().all(|v| v.abs() <= HALF_MAX);
            Level::rounded(width, height, stored, data, color_space, if half { Encoding::Half } else { Encoding::Float })
        });
        let mip_encoding = match (precision, base.encoding()) {
            (Precision::Lossless, _) => Encoding::Float,
            (Precision::Display, Encoding::Srgb8) => Encoding::Srgb8,
            (Precision::Display | Precision::Height, Encoding::Srgb8 | Encoding::Unorm8 | Encoding::Unorm16) => Encoding::Unorm16,
            (Precision::Height, _) => Encoding::Float,
            (Precision::Display, other) => other,
        };

        let mut levels = vec![base];
        while let Some((data, width, height)) = next {
            next = (width > 1 || height > 1).then(|| halve(&data, width, height, stored));
            levels.push(Level::rounded(width, height, stored, data, color_space, mip_encoding));
        }
        MipPyramid { levels }
    }

    /// A pyramid over a single flat colour, for a texture that has not
    /// finished loading. §5.1: "never block the render thread on I/O".
    pub fn placeholder(color: Vec3) -> MipPyramid {
        MipPyramid::build(Image::constant(1, 1, color))
    }

    pub fn levels(&self) -> usize {
        self.levels.len()
    }

    pub fn level(&self, index: usize) -> &Level {
        &self.levels[index.min(self.levels.len() - 1)]
    }

    pub fn base(&self) -> &Level {
        &self.levels[0]
    }

    pub fn color_space(&self) -> ColorSpace {
        self.levels[0].color_space
    }

    /// Total texels across every level, for a cache budget.
    ///
    /// The pyramid costs one third more than the base image, which is the sum
    /// of `1/4^n` — worth knowing before deciding a memory budget.
    pub fn texels(&self) -> usize {
        self.levels.iter().map(|l| l.width * l.height).sum()
    }

    /// Bytes the pyramid's texels take, every level included.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(Level::bytes).sum()
    }

    /// Bilinear lookup within one level.
    fn bilinear(&self, level: usize, u: f32, v: f32, wrap: Wrap) -> Vec3 {
        let image = self.level(level);
        let (w, h) = (image.width as f32, image.height as f32);

        // Texel centres sit at half-integer positions, so the continuous
        // coordinate has to be shifted by half a texel before flooring.
        // Without this every lookup is off by half a texel, which is invisible
        // on a photograph and glaring on a checkerboard.
        let x = wrap.apply(u) * w - 0.5;
        let y = wrap.apply(v) * h - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);

        let fetch = |ix: i64, iy: i64| -> Vec3 {
            let (cx, cy) = match wrap {
                Wrap::Clamp => (
                    ix.clamp(0, image.width as i64 - 1),
                    iy.clamp(0, image.height as i64 - 1),
                ),
                // Mirroring is already folded into the coordinate above, so at
                // this level the neighbour lookup only has to stay in range.
                Wrap::Repeat | Wrap::Mirror => (
                    ix.rem_euclid(image.width as i64),
                    iy.rem_euclid(image.height as i64),
                ),
            };
            image.texel(cx as usize, cy as usize)
        };

        let (ix, iy) = (x0 as i64, y0 as i64);
        let c00 = fetch(ix, iy);
        let c10 = fetch(ix + 1, iy);
        let c01 = fetch(ix, iy + 1);
        let c11 = fetch(ix + 1, iy + 1);

        let top = c00 * (1.0 - fx) + c10 * fx;
        let bottom = c01 * (1.0 - fx) + c11 * fx;
        top * (1.0 - fy) + bottom * fy
    }

    /// Trilinear lookup: bilinear within two levels, blended between them.
    ///
    /// `width` is the footprint's extent in texture coordinates — the width of
    /// the region this one lookup is standing in for. A footprint spanning the
    /// whole texture selects the top of the pyramid; a footprint smaller than
    /// one texel selects level 0.
    pub fn sample(&self, u: f32, v: f32, width: f32, wrap: Wrap) -> Vec3 {
        let level = self.level_for(width);
        let lower = level.floor();
        let blend = level - lower;
        let lower = lower.max(0.0) as usize;

        if blend <= 0.0 || lower + 1 >= self.levels.len() {
            return self.bilinear(lower, u, v, wrap);
        }
        let a = self.bilinear(lower, u, v, wrap);
        let b = self.bilinear(lower + 1, u, v, wrap);
        a * (1.0 - blend) + b * blend
    }

    /// The continuous pyramid level whose texels match a footprint of `width`.
    ///
    /// Level `n` has texels `2^n / resolution` wide, so the level that matches
    /// is `log2(width * resolution)`.
    pub fn level_for(&self, width: f32) -> f32 {
        let base = self.base();
        let resolution = base.width.max(base.height) as f32;
        let texels = (width * resolution).max(1e-8);
        // Below one texel there is nothing sharper to fetch.
        let level = texels.log2().max(0.0);
        level.min((self.levels.len() - 1) as f32)
    }

    /// Premultiplied RGBA, with transparent texels beyond the image bounds.
    pub fn sample_rgba_border(&self, u: f32, v: f32, footprint: f32) -> [f32; 4] {
        if !u.is_finite()
            || !v.is_finite()
            || !(0.0..=1.0).contains(&u)
            || !(0.0..=1.0).contains(&v)
        {
            return [0.0; 4];
        }
        let level = self.level_for(footprint);
        let lower = level.floor() as usize;
        let mix = level - lower as f32;
        let sample = |index| {
            let img = self.level(index);
            let x = u * img.width as f32 - 0.5;
            let y = v * img.height as f32 - 0.5;
            let mut result = [0.0; 4];
            for dy in 0..2 {
                for dx in 0..2 {
                    let ix = x.floor() as i64 + dx;
                    let iy = y.floor() as i64 + dy;
                    if ix < 0 || iy < 0 || ix >= img.width as i64 || iy >= img.height as i64 {
                        continue;
                    }
                    let weight = (if dx == 0 {
                        1.0 - x.fract().rem_euclid(1.0)
                    } else {
                        x.fract().rem_euclid(1.0)
                    }) * (if dy == 0 {
                        1.0 - y.fract().rem_euclid(1.0)
                    } else {
                        y.fract().rem_euclid(1.0)
                    });
                    let texel = img.rgba(ix as usize, iy as usize);
                    for channel in 0..4 {
                        result[channel] += texel[channel] * weight;
                    }
                }
            }
            result
        };
        let a = sample(lower);
        let b = sample(lower + 1);
        std::array::from_fn(|i| a[i] * (1.0 - mix) + b[i] * mix)
    }
}

/// Average each 2x2 block of texels into one: `source` is `width` by
/// `height` texels of `channels` floats, and so is what comes back, with its
/// own size.
///
/// Odd dimensions are handled by clamping the second sample to the edge rather
/// than by dropping the last row: dropping it shifts the image by half a texel
/// per level, which accumulates into a visible drift up the pyramid.
fn halve(source: &[f32], source_width: usize, source_height: usize, channels: usize) -> (Vec<f32>, usize, usize) {
    let width = (source_width / 2).max(1);
    let height = (source_height / 2).max(1);
    let mut data = vec![0.0f32; width * height * channels];

    for y in 0..height {
        for x in 0..width {
            let (x0, y0) = (2 * x, 2 * y);
            let x1 = (x0 + 1).min(source_width - 1);
            let y1 = (y0 + 1).min(source_height - 1);
            for c in 0..channels {
                let at = |px: usize, py: usize| source[(py * source_width + px) * channels + c];
                let sum = at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1);
                data[(y * width + x) * channels + c] = sum * 0.25;
            }
        }
    }

    (data, width, height)
}

/// The channels worth keeping of `data`, `channels` floats a texel, and how
/// many there are: one where red, green and blue are the same to the bit
/// throughout, so that reading it back as three changes nothing, and alpha
/// beside it where there is one.
fn kept_channels(channels: usize, data: Vec<f32>) -> (usize, Vec<f32>) {
    let channels = channels.max(1);
    let alpha = channels == 2 || channels >= 4;
    let grey = channels < 3
        || data.chunks_exact(channels).all(|p| p[0].to_bits() == p[1].to_bits() && p[1].to_bits() == p[2].to_bits());
    match (grey, alpha) {
        (true, false) if channels == 1 => (1, data),
        (true, true) if channels == 2 => (2, data),
        (false, false) if channels == 3 => (3, data),
        (false, true) if channels == 4 => (4, data),
        (true, false) => (1, data.chunks_exact(channels).map(|p| p[0]).collect()),
        (true, true) => (2, data.chunks_exact(channels).flat_map(|p| [p[0], p[3]]).collect()),
        // More than four channels: the first four, as `Image::texel` read them.
        (false, alpha) => {
            let keep = if alpha { 4 } else { 3 };
            (keep, data.chunks_exact(channels).flat_map(|p| p[..keep].to_vec()).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec3;
    use tequila_core::spectrum::srgb_inverse_eotf;

    /// A black-and-white checkerboard, the classic aliasing case.
    fn checkerboard(size: usize, square: usize) -> Image {
        let mut data = vec![0.0f32; size * size * 3];
        for y in 0..size {
            for x in 0..size {
                let on = ((x / square) + (y / square)) % 2 == 0;
                let value = if on { 1.0 } else { 0.0 };
                for c in 0..3 {
                    data[(y * size + x) * 3 + c] = value;
                }
            }
        }
        Image {
            width: size,
            height: size,
            channels: 3,
            data,
            color_space: ColorSpace::Raw,
        }
    }

    fn gradient(size: usize) -> Image {
        let mut data = vec![0.0f32; size * size * 3];
        for y in 0..size {
            for x in 0..size {
                for c in 0..3 {
                    data[(y * size + x) * 3 + c] = x as f32 / (size - 1) as f32;
                }
            }
        }
        Image {
            width: size,
            height: size,
            channels: 3,
            data,
            color_space: ColorSpace::Raw,
        }
    }

    #[test]
    fn the_pyramid_halves_down_to_one_texel() {
        let pyramid = MipPyramid::build(checkerboard(64, 8));
        assert_eq!(pyramid.levels(), 7, "64 -> 32 -> 16 -> 8 -> 4 -> 2 -> 1");
        for level in 0..pyramid.levels() {
            let image = pyramid.level(level);
            let expected = (64usize >> level).max(1);
            assert_eq!((image.width, image.height), (expected, expected));
        }
    }

    /// A box reduction preserves the mean exactly, at every level. This is the
    /// energy statement for a texture: averaging down must not brighten or
    /// darken it, or a distant surface picks up a tint the near one does not
    /// have.
    #[test]
    fn averaging_down_preserves_the_mean() {
        for source in [checkerboard(64, 8), gradient(64), checkerboard(32, 1)] {
            let base_mean: f64 =
                source.data.iter().map(|v| *v as f64).sum::<f64>() / source.data.len() as f64;
            let pyramid = MipPyramid::build(source);

            for level in 0..pyramid.levels() {
                let image = pyramid.level(level);
                let mut sum = 0.0f64;
                for y in 0..image.height {
                    for x in 0..image.width {
                        let t = image.texel(x, y);
                        sum += t.x as f64 + t.y as f64 + t.z as f64;
                    }
                }
                let mean = sum / (3 * image.width * image.height) as f64;
                assert!(
                    (mean - base_mean).abs() < 1e-5,
                    "level {level} has mean {mean:.6}, base has {base_mean:.6}"
                );
            }
        }
    }

    /// The point of the whole module: a footprint covering the whole texture
    /// returns its average, not whichever texel happened to be under the ray.
    ///
    /// A 1-pixel checkerboard is the worst case — point sampling it at a
    /// distance gives 0 or 1 depending on sub-texel position, and that
    /// difference is what crawls and shimmers in an animation.
    #[test]
    fn a_wide_footprint_returns_the_average_rather_than_a_texel() {
        let pyramid = MipPyramid::build(checkerboard(64, 1));

        for (u, v) in [(0.5, 0.5), (0.123, 0.789), (0.0, 0.0), (0.999, 0.001)] {
            let wide = pyramid.sample(u, v, 1.0, Wrap::Repeat);
            assert!(
                (wide.x - 0.5).abs() < 1e-3,
                "a full-texture footprint at ({u}, {v}) returned {} rather than the \
                 0.5 average",
                wide.x
            );
        }

        // ...and a footprint smaller than a texel still resolves the detail.
        let near = pyramid.sample(0.5 / 64.0, 0.5 / 64.0, 1.0 / 64.0, Wrap::Repeat);
        assert!(
            near.x > 0.9 || near.x < 0.1,
            "a one-texel footprint blurred the checkerboard to {}",
            near.x
        );
    }

    /// Moving the lookup across the surface with a wide footprint must not make
    /// the value jump around. This is the shimmering, measured.
    #[test]
    fn a_wide_footprint_is_stable_as_the_lookup_moves() {
        let pyramid = MipPyramid::build(checkerboard(64, 1));
        let mut lowest = f32::INFINITY;
        let mut highest = f32::NEG_INFINITY;
        for i in 0..200 {
            let t = i as f32 / 200.0;
            let value = pyramid.sample(t, t * 0.37, 0.5, Wrap::Repeat).x;
            lowest = lowest.min(value);
            highest = highest.max(value);
        }
        assert!(
            highest - lowest < 0.02,
            "a wide footprint swung between {lowest:.3} and {highest:.3} as it \
             moved; that swing is what shimmers in an animation"
        );
    }

    /// Level selection has a closed form, so it can be checked against one
    /// rather than eyeballed.
    #[test]
    fn the_level_matches_the_footprint() {
        let pyramid = MipPyramid::build(checkerboard(256, 4));
        // A footprint of one texel is level 0; each doubling is one level up.
        for (footprint, expected) in [
            (1.0 / 256.0, 0.0),
            (2.0 / 256.0, 1.0),
            (8.0 / 256.0, 3.0),
            (1.0, 8.0),
        ] {
            let level = pyramid.level_for(footprint);
            assert!(
                (level - expected).abs() < 1e-4,
                "a footprint of {footprint} chose level {level}, expected {expected}"
            );
        }
        // Sub-texel footprints cannot go below the sharpest level, and huge
        // ones cannot go past the top.
        assert_eq!(pyramid.level_for(1e-9), 0.0);
        assert_eq!(pyramid.level_for(1e9), (pyramid.levels() - 1) as f32);
    }

    #[test]
    fn wrapping_modes_behave() {
        assert!((Wrap::Repeat.apply(1.25) - 0.25).abs() < 1e-6);
        assert!((Wrap::Repeat.apply(-0.25) - 0.75).abs() < 1e-6);
        assert!((Wrap::Clamp.apply(1.25) - 1.0).abs() < 1e-6);
        assert!((Wrap::Clamp.apply(-0.25) - 0.0).abs() < 1e-6);
        // Mirror folds back: 1.25 is 0.75 of the way into a reflected tile.
        assert!((Wrap::Mirror.apply(1.25) - 0.75).abs() < 1e-6);
        assert!((Wrap::Mirror.apply(-0.25) - 0.25).abs() < 1e-6);
        for t in [0.0f32, 0.3, 0.999] {
            assert!(
                (Wrap::Mirror.apply(t) - t).abs() < 1e-6,
                "{t} inside a tile"
            );
        }

        assert!(!Wrap::Clamp.is_inside(1.5));
        assert!(Wrap::Clamp.is_inside(0.5));
        assert!(Wrap::Repeat.is_inside(1.5));
    }

    /// Mirroring exists so that tiles meet without a seam; that is testable —
    /// but not by sampling either side of the boundary itself.
    ///
    /// Bilinear filtering blends the last texel into the first under
    /// [`Wrap::Repeat`], so *at* the boundary both sides read the same blended
    /// value and nothing appears to jump. The discontinuity is smeared across
    /// one texel rather than removed. What actually distinguishes the two modes
    /// is reflection symmetry a little way in: mirroring makes the texture fold
    /// about the boundary, so equal distances either side match.
    #[test]
    fn mirrored_tiles_meet_without_a_seam() {
        let pyramid = MipPyramid::build(gradient(64));
        let step = 0.1;

        let before = pyramid.sample(1.0 - step, 0.5, 0.0, Wrap::Mirror).x;
        let after = pyramid.sample(1.0 + step, 0.5, 0.0, Wrap::Mirror).x;
        assert!(
            (before - after).abs() < 0.01,
            "mirroring should fold about the boundary, but {step} before it \
             reads {before:.3} and {step} after reads {after:.3}"
        );

        // Plain repeat restarts the gradient instead, which is the seam
        // mirroring exists to avoid.
        let a = pyramid.sample(1.0 - step, 0.5, 0.0, Wrap::Repeat).x;
        let b = pyramid.sample(1.0 + step, 0.5, 0.0, Wrap::Repeat).x;
        assert!(
            (a - b).abs() > 0.5,
            "the fixture needs a gradient that visibly restarts: {a:.3} against {b:.3}"
        );
    }

    /// A half-texel offset in the lookup is invisible on a photograph and
    /// glaring on a checkerboard, so it is worth pinning: the centre of texel
    /// (0, 0) must return exactly that texel.
    #[test]
    fn texel_centres_land_on_their_own_texel() {
        let pyramid = MipPyramid::build(checkerboard(8, 1));
        for (x, y) in [(0usize, 0usize), (1, 0), (0, 1), (7, 7), (3, 4)] {
            let u = (x as f32 + 0.5) / 8.0;
            let v = (y as f32 + 0.5) / 8.0;
            let sampled = pyramid.sample(u, v, 0.0, Wrap::Repeat);
            let expected = pyramid.base().texel(x, y);
            assert!(
                (sampled.x - expected.x).abs() < 1e-5,
                "texel ({x}, {y}) sampled as {} but holds {}",
                sampled.x,
                expected.x
            );
        }
    }

    #[test]
    fn a_placeholder_is_a_valid_pyramid() {
        let pyramid = MipPyramid::placeholder(vec3(0.5, 0.5, 0.5));
        assert_eq!(pyramid.levels(), 1);
        let sampled = pyramid.sample(0.37, 0.91, 0.25, Wrap::Repeat);
        assert!((sampled.x - 0.5).abs() < 1e-6);
    }

    /// A pyramid costs a third more than the image it is built from.
    #[test]
    fn the_pyramid_costs_a_third_more_than_the_base() {
        let pyramid = MipPyramid::build(checkerboard(256, 4));
        let base = 256 * 256;
        let total = pyramid.texels();
        let overhead = (total - base) as f64 / base as f64;
        assert!(
            (overhead - 1.0 / 3.0).abs() < 0.01,
            "the pyramid added {:.1}% rather than the expected 33%",
            overhead * 100.0
        );
    }

    /// Non-square and odd-sized images must still reduce sensibly rather than
    /// panicking or drifting.
    #[test]
    fn odd_and_non_square_images_reduce_without_drifting() {
        for (w, h) in [(17usize, 5usize), (1, 64), (64, 1), (3, 3), (1, 1)] {
            let image = Image::constant(w, h, vec3(0.25, 0.5, 0.75));
            let pyramid = MipPyramid::build(image);
            let top = pyramid.level(pyramid.levels() - 1);
            assert_eq!(
                (top.width, top.height),
                (1, 1),
                "{w}x{h} did not reduce to 1x1"
            );
            // A constant image stays constant all the way up: any drift in the
            // reduction would show as a changed value.
            let value = top.texel(0, 0);
            assert!(
                (value.x - 0.25).abs() < 1e-5 && (value.z - 0.75).abs() < 1e-5,
                "{w}x{h} reduced a constant image to {value:?}"
            );
        }
    }

    /// Bytes from a fixed sequence, so a picture has detail at every scale.
    fn bytes(count: usize, seed: u32) -> Vec<u8> {
        let mut state = seed | 1;
        (0..count)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 24) as u8
            })
            .collect()
    }

    fn sample_points() -> Vec<(f32, f32, f32)> {
        let noise = bytes(3 * 4000, 5);
        noise
            .chunks_exact(3)
            .map(|p| (p[0] as f32 / 255.0, p[1] as f32 / 255.0, (p[2] as f32 / 255.0).powi(3) * 0.2))
            .collect()
    }

    /// A picture from an 8-bit file is kept as its bytes and reads back the
    /// very floats it was decoded to: colour on the sRGB curve with its
    /// alpha straight, numbers straight.
    #[test]
    fn an_8bit_picture_is_kept_as_its_bytes_and_reads_back_to_the_bit() {
        let (w, h) = (300, 260);
        for (image, encoding) in [
            (Image::from_srgb8(w, h, 4, &bytes(w * h * 4, 1)), Encoding::Srgb8),
            (Image::from_raw8(w, h, 3, &bytes(w * h * 3, 2)), Encoding::Unorm8),
        ] {
            let pyramid = MipPyramid::build(image.clone());
            let base = pyramid.base();
            assert_eq!((base.encoding(), base.stored_channels(), base.channels()), (encoding, image.channels, image.channels));
            assert_eq!(base.bytes(), w * h * image.channels, "a byte a channel");
            for y in 0..h {
                for x in 0..w {
                    let at = (y * w + x) * image.channels;
                    let read = base.rgba(x, y);
                    for c in 0..image.channels {
                        assert_eq!(read[c].to_bits(), image.data[at + c].to_bits(), "{encoding:?} ({x}, {y}) channel {c}");
                    }
                }
            }
        }
    }

    /// A grey map - a roughness, a mask - arrives as three equal channels,
    /// is kept as one, and reads as three: the base level to the bit, and
    /// any lookup within the rounding of the levels above it.
    #[test]
    fn a_grey_map_is_kept_as_one_channel_and_samples_as_before() {
        let (w, h) = (512, 384);
        let grey = bytes(w * h, 3);
        let rgb: Vec<u8> = grey.iter().flat_map(|&v| [v, v, v]).collect();
        let image = Image::from_raw8(w, h, 3, &rgb);
        let pyramid = MipPyramid::build(image.clone());
        let base = pyramid.base();
        assert_eq!((base.stored_channels(), base.channels(), base.encoding()), (1, 3, Encoding::Unorm8));
        for (x, y) in [(0, 0), (17, 300), (511, 383), (256, 1)] {
            let expected = image.data[(y * w + x) * 3];
            assert_eq!(base.texel(x, y), Vec3::splat(expected), "({x}, {y})");
        }
        // What every lookup returned when the pyramid was floats.
        let floats = MipPyramid::build_with(image, Precision::Lossless);
        assert_eq!(floats.base().stored_channels(), 1, "the floats' pyramid is grey too");
        for (u, v, width) in sample_points() {
            let (now, before) = (pyramid.sample(u, v, width, Wrap::Repeat), floats.sample(u, v, width, Wrap::Repeat));
            assert!((now - before).abs().max_component() <= 0.5 / 65535.0 + 1e-6, "({u}, {v}, {width}): {now:?} against {before:?}");
            assert!(now.x == now.y && now.y == now.z);
        }
        // Grey with alpha keeps both.
        let rgba: Vec<u8> = grey.iter().flat_map(|&v| [v, v, v, 255 - v]).collect();
        let level = MipPyramid::build(Image::from_raw8(w, h, 4, &rgba));
        assert_eq!((level.base().stored_channels(), level.base().channels()), (2, 4));
        assert_eq!(level.base().rgba(5, 7), Image::from_raw8(w, h, 4, &rgba).data[(7 * w + 5) * 4..][..4]);
    }

    /// Each mip level is the average of the full-precision level below,
    /// taken in linear light, rounded once as it is kept: a colour picture's
    /// to the nearest byte along the sRGB curve, numbers' to sixteen bits.
    #[test]
    fn mip_levels_are_rounded_once_from_the_linear_average() {
        let (w, h) = (512, 512);
        let colour = Image::from_srgb8(w, h, 3, &bytes(w * h * 3, 4));
        let data = Image::from_raw8(w, h, 3, &bytes(w * h * 3, 6));
        for (image, rounding) in [(colour, Encoding::Srgb8), (data, Encoding::Unorm16)] {
            let kept = MipPyramid::build(image.clone());
            let floats = MipPyramid::build_with(image, Precision::Lossless);
            assert_eq!(kept.levels(), floats.levels());
            for index in 1..kept.levels() {
                let (level, exact) = (kept.level(index), floats.level(index));
                assert_eq!((level.encoding(), exact.encoding()), (rounding, Encoding::Float));
                for y in 0..level.height {
                    for x in 0..level.width {
                        let (a, b) = (level.texel(x, y), exact.texel(x, y));
                        for (a, b) in [(a.x, b.x), (a.y, b.y), (a.z, b.z)] {
                            let off = match rounding {
                                // Half a byte along the curve.
                                Encoding::Srgb8 => (srgb_inverse_eotf(a) - srgb_inverse_eotf(b)).abs() * 255.0,
                                _ => (a - b).abs() * 65535.0,
                            };
                            // Give or take the float arithmetic of the
                            // rounding itself, a hundredth of a step.
                            assert!(off <= 0.51, "level {index} ({x}, {y}): {a} kept for {b}");
                        }
                    }
                }
            }
        }
        // A black-and-white checkerboard still averages to 0.5 in linear
        // light, not to the 0.21 averaging the encoded bytes would give.
        let checker: Vec<u8> = (0..w * h).flat_map(|i| [if (i % w + i / w) % 2 == 0 { 255 } else { 0 }; 3]).collect();
        let pyramid = MipPyramid::build(Image::from_srgb8(w, h, 3, &checker));
        let top = pyramid.level(pyramid.levels() - 1).texel(0, 0).x;
        assert!((srgb_inverse_eotf(top) - srgb_inverse_eotf(0.5)).abs() * 255.0 <= 0.5, "averaged to {top}");
    }

    /// Floats that no compact form holds exactly are rounded to half floats,
    /// within a part in two thousand; floats a half float does hold are
    /// kept so exactly; and what a half float cannot reach, a height, or a
    /// picture too small to be worth it keeps its floats.
    #[test]
    fn floats_are_rounded_to_half_only_where_they_must_be() {
        let (w, h) = (300, 300);
        let image = |data: Vec<f32>, w: usize, h: usize| Image { width: w, height: h, channels: 3, data, color_space: ColorSpace::LinearSrgb };
        let noise: Vec<f32> = bytes(w * h * 3, 7).iter().enumerate().map(|(i, &b)| b as f32 * 0.0137 + (i % 7) as f32 * 0.1).collect();

        let rounded = MipPyramid::build(image(noise.clone(), w, h));
        assert_eq!(rounded.base().encoding(), Encoding::Half);
        assert_eq!(rounded.bytes(), rounded.texels() * 3 * 2, "two bytes a value, every level");
        for (i, texel) in noise.chunks_exact(3).enumerate().step_by(97) {
            let read = rounded.base().texel(i % w, i / w);
            for (r, t) in [(read.x, texel[0]), (read.y, texel[1]), (read.z, texel[2])] {
                assert!((r - t).abs() <= t.abs() / 2048.0, "{t} read as {r}");
            }
        }

        let halves: Vec<f32> = noise.iter().map(|&v| half::f16::from_f32(v).to_f32()).collect();
        let exact = MipPyramid::build(image(halves.clone(), w, h));
        assert_eq!(exact.base().encoding(), Encoding::Half);
        assert_eq!(exact.base().texel(4, 2), vec3(halves[(2 * w + 4) * 3], halves[(2 * w + 4) * 3 + 1], halves[(2 * w + 4) * 3 + 2]));

        let mut bright = noise.clone();
        bright[10] = 1.0e5;
        assert_eq!(MipPyramid::build(image(bright, w, h)).base().encoding(), Encoding::Float, "past a half float's range");
        let height = MipPyramid::build_with(image(noise.clone(), w, h), Precision::Height);
        assert_eq!((height.base().encoding(), height.level(1).encoding()), (Encoding::Float, Encoding::Float));
        let small = MipPyramid::build(image(noise[..64 * 64 * 3].to_vec(), 64, 64));
        assert_eq!((small.base().encoding(), small.level(1).encoding()), (Encoding::Float, Encoding::Float));

        // An 8-bit height keeps its bytes, and its mip levels sixteen bits
        // whatever it is tagged.
        let eight = Image::from_srgb8(w, h, 3, &bytes(w * h * 3, 8));
        let height = MipPyramid::build_with(eight, Precision::Height);
        assert_eq!((height.base().encoding(), height.level(1).encoding()), (Encoding::Srgb8, Encoding::Unorm16));
    }

    /// The memory the change is for. A 4K colour map from an 8-bit file
    /// took 268 MB as floats with its pyramid, and a 4K roughness map the
    /// same in three equal channels; kept as their bytes, the colour map
    /// takes a quarter of that and the roughness map about a tenth.
    #[test]
    fn a_4k_colour_map_and_a_4k_roughness_map_take_a_quarter_and_a_tenth() {
        let side = 4096;
        let texels: usize = (0..=12).map(|n| (side >> n) * (side >> n)).sum();
        let floats = texels * 3 * 4;
        assert_eq!(floats, 268_435_452, "what a 4K map took");

        // Decoded as `Image::from_srgb8` and `Image::from_raw8` decode, by
        // table, to keep the test quick.
        let srgb: [f32; 256] = std::array::from_fn(|b| tequila_core::spectrum::srgb_eotf(b as f32 / 255.0));
        let noise = bytes(side * side * 3, 9);
        let colour = Image {
            width: side,
            height: side,
            channels: 3,
            data: noise.iter().map(|&b| srgb[b as usize]).collect(),
            color_space: ColorSpace::LinearSrgb,
        };
        let colour = MipPyramid::build(colour);
        assert_eq!(colour.base().encoding(), Encoding::Srgb8);
        assert_eq!(colour.bytes(), texels * 3, "a byte a channel, every level");
        assert_eq!(colour.bytes(), 67_108_863);

        let roughness = Image {
            width: side,
            height: side,
            channels: 3,
            data: noise[..side * side].iter().flat_map(|&b| [b as f32 / 255.0; 3]).collect(),
            color_space: ColorSpace::Raw,
        };
        let roughness = MipPyramid::build(roughness);
        assert_eq!((roughness.base().encoding(), roughness.base().stored_channels()), (Encoding::Unorm8, 1));
        // A byte a texel at the base; sixteen bits a texel above it.
        assert_eq!(roughness.bytes(), side * side + (texels - side * side) * 2);
        assert_eq!(roughness.bytes(), 27_962_026);
    }
}
