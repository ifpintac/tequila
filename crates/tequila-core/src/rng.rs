//! Hashing and the PCG32 fallback generator.
//!
//! `docs/01-RENDERER-CORE.md` §4.2 uses Sobol′ for the first ~64 path
//! dimensions and falls back to a hashed uniform generator beyond that, where
//! low-discrepancy structure no longer pays for itself. PCG32 is that
//! fallback: statistically excellent, trivially seedable, and stateless enough
//! to reproduce exactly (§4.6 determinism).

/// A 32-bit integer hash (Wang / Jenkins lineage) used to derive decorrelated
/// seeds. Fast, avalanches well, and — critically for determinism — has no
/// platform-dependent behaviour.
#[inline]
pub fn hash_u32(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// Combine a hash with another value. Used to build per-pixel, per-dimension,
/// and per-bounce seeds from the single scene seed.
#[inline]
pub fn hash_combine(seed: u32, v: u32) -> u32 {
    hash_u32(seed ^ v.wrapping_mul(0x9e37_79b9))
}

/// Hash of an arbitrary list of u32s.
#[inline]
pub fn hash_many(values: &[u32]) -> u32 {
    let mut h = 0x811c_9dc5;
    for &v in values {
        h = hash_combine(h, v);
    }
    h
}

/// PCG32 (`pcg_setseq_64_xsh_rr_32`) [O'Neill 2014].
#[derive(Clone, Copy, Debug)]
pub struct Pcg32 {
    state: u64,
    inc: u64,
}

const PCG32_MULT: u64 = 0x5851_f42d_4c95_7f2d;

impl Pcg32 {
    /// Seed from a sequence value and an initial state. Both are hashed so
    /// that adjacent seeds (adjacent pixels, adjacent samples) produce
    /// uncorrelated streams.
    pub fn new(seq: u64, seed: u64) -> Self {
        let mut r = Pcg32 {
            state: 0,
            inc: (seq << 1) | 1,
        };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }

    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(PCG32_MULT).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        let rot = (old >> 59) as u32;
        xorshifted.rotate_right(rot)
    }

    /// Uniform float in `[0, 1)`. Uses 24 bits so the result is exactly
    /// representable and never rounds up to 1.0 — a value of exactly 1.0
    /// escapes range checks in the sampling warps and produces NaNs.
    #[inline]
    pub fn next_f32(&mut self) -> f32 {
        const ONE_MINUS_EPS: f32 = 1.0 - f32::EPSILON / 2.0;
        ((self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)).min(ONE_MINUS_EPS)
    }

    #[inline]
    pub fn next_f32_pair(&mut self) -> (f32, f32) {
        (self.next_f32(), self.next_f32())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcg_is_reproducible() {
        let a: Vec<u32> = {
            let mut r = Pcg32::new(7, 42);
            (0..8).map(|_| r.next_u32()).collect()
        };
        let b: Vec<u32> = {
            let mut r = Pcg32::new(7, 42);
            (0..8).map(|_| r.next_u32()).collect()
        };
        assert_eq!(a, b);
    }

    #[test]
    fn pcg_floats_stay_in_range() {
        let mut r = Pcg32::new(1, 2);
        for _ in 0..100_000 {
            let f = r.next_f32();
            assert!((0.0..1.0).contains(&f), "out of range: {f}");
        }
    }

    #[test]
    fn pcg_mean_is_near_one_half() {
        let mut r = Pcg32::new(3, 9);
        let n = 200_000;
        let sum: f64 = (0..n).map(|_| r.next_f32() as f64).sum();
        assert!((sum / n as f64 - 0.5).abs() < 0.005);
    }

    #[test]
    fn hash_decorrelates_adjacent_seeds() {
        // Adjacent pixel indices must not produce adjacent streams.
        let a = hash_combine(0, 1000);
        let b = hash_combine(0, 1001);
        assert!(a.wrapping_sub(b) > 1000, "hash barely moved: {a} vs {b}");
    }
}
