//! The path sampler: static dimension allocation over Owen-scrambled Sobol′.
//!
//! `docs/01-RENDERER-CORE.md` §4.2 requires dimensions to be assigned
//! **statically and consistently** — allocating them dynamically as the path
//! happens to branch destroys the low-discrepancy property that is the entire
//! reason for choosing Sobol′.
//!
//! ## Deviation from §4.2, recorded here per `docs/05` §2
//!
//! The specification's table allocates 6 dimensions per bounce starting at
//! dimension 6. This implementation allocates **8 per bounce starting at
//! dimension 8**: two of [`crate::sobol`]'s padding groups of
//! [`NUM_BASE_DIMENSIONS`](crate::sobol::NUM_BASE_DIMENSIONS)`= 4`, which are
//! jointly stratified within a group and decorrelated across. The bounce's
//! direction takes the first group's leading pair, with the lobe choice and
//! Russian roulette after it; the light sample takes the second's, with the
//! light choice and medium sampling after it. So each two-dimensional draw
//! is a group's leading pair, the one Sobol′ stratifies perfectly, and each
//! choice is stratified together with the draw it goes with.

use crate::math::{vec2, Vec2};
use crate::rng::{hash_many, Pcg32};
use crate::sobol::{self, NUM_BASE_DIMENSIONS};

/// Dimensions consumed by each bounce: two whole Sobol′ padding groups, so
/// that no draw straddles two.
pub const DIMENSIONS_PER_BOUNCE: u32 = 2 * NUM_BASE_DIMENSIONS;

/// First dimension belonging to bounce 0. Dimensions below this are the
/// camera-level samples, which every path shares.
pub const FIRST_BOUNCE_DIMENSION: u32 = 2 * NUM_BASE_DIMENSIONS;

/// Camera-level dimension indices.
pub mod dim {
    /// Pixel jitter for filter importance sampling (2D).
    pub const PIXEL: u32 = 0;
    /// Lens point for depth of field (2D).
    pub const LENS: u32 = 2;
    /// Shutter time for motion blur (1D).
    pub const TIME: u32 = 4;
    /// Hero wavelength in spectral mode (1D).
    pub const WAVELENGTH: u32 = 5;

    /// Per-bounce offsets, added to the bounce's base dimension: the
    /// bounce's own group first, then the light sample's.
    pub const BSDF_DIRECTION: u32 = 0; // 2D
    pub const BSDF_LOBE: u32 = 2;
    pub const RUSSIAN_ROULETTE: u32 = 3;
    pub const LIGHT_SAMPLE: u32 = 4; // 2D
    pub const LIGHT_SELECT: u32 = 6;
    pub const MEDIUM: u32 = 7;
}

/// Beyond this dimension the low-discrepancy benefit is negligible and the
/// sampler falls back to a hashed PCG32 stream (§4.2, last row).
const SOBOL_DIMENSION_LIMIT: u32 = 64;

/// Per-pixel, per-sample view of the sample sequence.
///
/// A `Sampler` is cheap to construct and holds no shared state, so each tile
/// worker makes its own. Every value it returns is a pure function of
/// `(pixel, sample_index, dimension, scene_seed)`, which is what makes
/// rendering order-independent and bit-reproducible (§4.6).
#[derive(Clone, Debug)]
pub struct Sampler {
    /// Owen scrambling seed: decorrelates this pixel from its neighbours.
    seed: u32,
    sample_index: u32,
    /// Base dimension of the bounce currently being sampled.
    bounce_base: u32,
    /// Fallback stream for dimensions past [`SOBOL_DIMENSION_LIMIT`].
    fallback: Pcg32,
}

impl Sampler {
    /// Create a sampler for one pixel and one sample index.
    ///
    /// `scene_seed` is fixed for a render; changing it is how a user asks for
    /// a different noise pattern without changing anything else.
    pub fn new(pixel_x: u32, pixel_y: u32, sample_index: u32, scene_seed: u32) -> Self {
        let seed = hash_many(&[scene_seed, pixel_x, pixel_y]);
        Sampler {
            seed,
            sample_index,
            bounce_base: FIRST_BOUNCE_DIMENSION,
            fallback: Pcg32::new(
                hash_many(&[seed, sample_index, 0x5f3d]) as u64,
                hash_many(&[seed, sample_index, 0xa17c]) as u64,
            ),
        }
    }

    /// Advance to the dimension block for `bounce`. Called once per path
    /// vertex; every subsequent `bounce_*` call reads from that block.
    #[inline]
    pub fn begin_bounce(&mut self, bounce: u32) {
        self.bounce_base = FIRST_BOUNCE_DIMENSION + bounce * DIMENSIONS_PER_BOUNCE;
    }

    /// One sample from an explicit dimension.
    #[inline]
    pub fn get(&mut self, dimension: u32) -> f32 {
        if dimension < SOBOL_DIMENSION_LIMIT {
            sobol::sample(self.sample_index, dimension, self.seed)
        } else {
            self.fallback.next_f32()
        }
    }

    /// Two samples from `dimension` and `dimension + 1`.
    #[inline]
    pub fn get2(&mut self, dimension: u32) -> Vec2 {
        vec2(self.get(dimension), self.get(dimension + 1))
    }

    // --- camera-level accessors --------------------------------------------

    #[inline]
    pub fn pixel_jitter(&mut self) -> Vec2 {
        self.get2(dim::PIXEL)
    }

    #[inline]
    pub fn lens(&mut self) -> Vec2 {
        self.get2(dim::LENS)
    }

    #[inline]
    pub fn time(&mut self) -> f32 {
        self.get(dim::TIME)
    }

    #[inline]
    pub fn wavelength(&mut self) -> f32 {
        self.get(dim::WAVELENGTH)
    }

    // --- per-bounce accessors ----------------------------------------------

    #[inline]
    pub fn light_select(&mut self) -> f32 {
        self.get(self.bounce_base + dim::LIGHT_SELECT)
    }

    #[inline]
    pub fn light_sample(&mut self) -> Vec2 {
        self.get2(self.bounce_base + dim::LIGHT_SAMPLE)
    }

    #[inline]
    pub fn bsdf_lobe(&mut self) -> f32 {
        self.get(self.bounce_base + dim::BSDF_LOBE)
    }

    #[inline]
    pub fn bsdf_direction(&mut self) -> Vec2 {
        self.get2(self.bounce_base + dim::BSDF_DIRECTION)
    }

    #[inline]
    pub fn russian_roulette(&mut self) -> f32 {
        self.get(self.bounce_base + dim::RUSSIAN_ROULETTE)
    }

    #[inline]
    pub fn medium(&mut self) -> f32 {
        self.get(self.bounce_base + dim::MEDIUM)
    }

    /// An unstructured sample for uses with no fixed dimension budget —
    /// random-walk subsurface scattering, delta tracking through heterogeneous
    /// media. These are unbounded in length, so they cannot have static
    /// dimensions and must not consume them.
    #[inline]
    pub fn next_unstructured(&mut self) -> f32 {
        self.fallback.next_f32()
    }

    #[inline]
    pub fn next_unstructured2(&mut self) -> Vec2 {
        vec2(self.fallback.next_f32(), self.fallback.next_f32())
    }

    #[inline]
    pub fn sample_index(&self) -> u32 {
        self.sample_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_give_the_same_samples() {
        let mut a = Sampler::new(37, 91, 12, 0xdead);
        let mut b = Sampler::new(37, 91, 12, 0xdead);
        a.begin_bounce(3);
        b.begin_bounce(3);
        assert_eq!(a.pixel_jitter(), b.pixel_jitter());
        assert_eq!(a.bsdf_direction(), b.bsdf_direction());
        assert_eq!(a.next_unstructured(), b.next_unstructured());
    }

    #[test]
    fn neighbouring_pixels_are_decorrelated() {
        let mut a = Sampler::new(10, 10, 0, 1);
        let mut b = Sampler::new(11, 10, 0, 1);
        let ja = a.pixel_jitter();
        let jb = b.pixel_jitter();
        assert!((ja.x - jb.x).abs() > 1e-3 || (ja.y - jb.y).abs() > 1e-3);
    }

    /// Reading the same dimension twice must give the same value: dimensions
    /// are positions in a sequence, not a stream of consumed randomness.
    #[test]
    fn dimensions_are_positional_not_consumed() {
        let mut s = Sampler::new(1, 2, 3, 4);
        assert_eq!(s.get(dim::PIXEL), s.get(dim::PIXEL));
    }

    /// Each bounce must start on a Sobol′ padding group, and each of its
    /// two-dimensional draws must be a group's leading pair, which is the
    /// reason this module deviates from the spec's 6-wide allocation.
    #[test]
    fn bounce_blocks_align_to_padding_groups() {
        for bounce in 0..8u32 {
            let base = FIRST_BOUNCE_DIMENSION + bounce * DIMENSIONS_PER_BOUNCE;
            assert_eq!(base % NUM_BASE_DIMENSIONS, 0);
            for pair in [dim::BSDF_DIRECTION, dim::LIGHT_SAMPLE] {
                assert_eq!((base + pair) % NUM_BASE_DIMENSIONS, 0, "bounce {bounce}: a pair off a group's start");
            }
        }
        assert_eq!(dim::PIXEL % NUM_BASE_DIMENSIONS, 0);
    }

    /// A bounce's direction and its light sample are each a perfect
    /// (0,2)-net, in every pixel and at every bounce: for `4^k` samples,
    /// one in each cell of a `2^k` by `2^k` grid. The pair a bounce's
    /// direction was once drawn from (Sobol′'s dimensions 4 and 5) is not,
    /// and made a glossy reflection of a softbox several times noisier than
    /// it needs to be (`docs/07` R1).
    #[test]
    fn every_two_dimensional_draw_is_a_perfect_net() {
        let draws: [(&str, fn(&mut Sampler) -> Vec2); 3] =
            [("direction", Sampler::bsdf_direction), ("light", Sampler::light_sample), ("pixel", Sampler::pixel_jitter)];
        for (pixel, bounce) in [((0u32, 0u32), 0u32), ((37, 91), 1), ((640, 2), 4)] {
            for (name, draw) in draws {
                for k in 2..=4u32 {
                    let side = 1usize << k;
                    let mut seen = vec![false; side * side];
                    for i in 0..(side * side) as u32 {
                        let mut s = Sampler::new(pixel.0, pixel.1, i, 9);
                        s.begin_bounce(bounce);
                        let v = draw(&mut s);
                        let cell = (v.y * side as f32) as usize * side + (v.x * side as f32) as usize;
                        assert!(!seen[cell], "{name} at bounce {bounce}, pixel {pixel:?}: two of {} samples in one cell", side * side);
                        seen[cell] = true;
                    }
                }
            }
        }
    }

    /// The pixel-jitter dimension must be uniform over the pixel footprint,
    /// averaged across a tile of pixels — a bias here shifts the whole image.
    #[test]
    fn pixel_jitter_is_unbiased_across_a_tile() {
        let mut sum = (0.0f64, 0.0f64);
        let mut n = 0u32;
        for y in 0..16 {
            for x in 0..16 {
                for s in 0..64 {
                    let j = Sampler::new(x, y, s, 5).pixel_jitter();
                    sum.0 += j.x as f64;
                    sum.1 += j.y as f64;
                    n += 1;
                }
            }
        }
        let mean = (sum.0 / n as f64, sum.1 / n as f64);
        assert!((mean.0 - 0.5).abs() < 0.005, "x mean {}", mean.0);
        assert!((mean.1 - 0.5).abs() < 0.005, "y mean {}", mean.1);
    }

    #[test]
    fn dimensions_past_the_sobol_limit_still_produce_valid_samples() {
        let mut s = Sampler::new(0, 0, 0, 1);
        for d in SOBOL_DIMENSION_LIMIT..SOBOL_DIMENSION_LIMIT + 32 {
            let v = s.get(d);
            assert!((0.0..1.0).contains(&v));
        }
    }
}
