//! Owen-scrambled Sobol′ sequence [Burley 2020, *Practical Hash-based Owen
//! Scrambling*], the sampler specified in `docs/01-RENDERER-CORE.md` §4.1.
//!
//! # Why this sampler
//!
//! It converges faster than stratified or Halton on the dimensions a path
//! tracer actually uses, every prefix of the sequence is itself a valid
//! low-discrepancy set (so it is *progressive*, which the viewport requires),
//! and it needs no shared state, so it parallelizes without synchronization
//! and stays deterministic regardless of tile order (§4.6).
//!
//! # Dimension padding
//!
//! `NUM_BASE_DIMENSIONS` genuine Sobol′ dimensions are generated from
//! primitive-polynomial direction numbers. Higher dimensions are served by
//! *padding*: dimension `d` is drawn from base dimension `d % N` with the
//! Owen seed perturbed by the group index `d / N`. Within a group of `N`
//! dimensions the samples are jointly stratified; across groups they are
//! decorrelated. This is the strategy Burley's paper describes, and it is why
//! [`crate::sampler`] allocates exactly `NUM_BASE_DIMENSIONS` dimensions per
//! bounce — so that each bounce's samples land inside one stratified group
//! rather than straddling two.
//!
//! Owen scrambling makes every returned point uniformly distributed in
//! `[0,1)` regardless of the underlying direction numbers, so the estimator is
//! unbiased by construction; the direction numbers affect convergence *rate*
//! only. The net-property tests below guard the rate.

use crate::rng::{hash_combine, hash_u32};

/// Number of genuine Sobol′ dimensions before padding kicks in.
pub const NUM_BASE_DIMENSIONS: u32 = 8;

const MATRIX_BITS: usize = 32;

/// Primitive polynomial data for Sobol′ dimensions 1.. (dimension 0 is the
/// van der Corput sequence and needs no entry). Fields are
/// `(degree s, polynomial coefficients a, initial direction numbers m_1..m_s)`
/// from the standard Joe-Kuo tabulation.
const POLYNOMIALS: [(u32, u32, &[u32]); NUM_BASE_DIMENSIONS as usize - 1] = [
    (1, 0, &[1]),
    (2, 1, &[1, 3]),
    (3, 1, &[1, 3, 1]),
    (3, 2, &[1, 1, 1]),
    (4, 1, &[1, 1, 3, 3]),
    (4, 4, &[1, 3, 5, 13]),
    (5, 2, &[1, 1, 5, 5, 17]),
];

/// Direction numbers, `[dimension][bit]`. Generated once on first use rather
/// than embedded as a literal table so the construction is auditable.
static MATRICES: std::sync::OnceLock<Vec<[u32; MATRIX_BITS]>> = std::sync::OnceLock::new();

fn matrices() -> &'static [[u32; MATRIX_BITS]] {
    MATRICES.get_or_init(|| {
        let mut all = Vec::with_capacity(NUM_BASE_DIMENSIONS as usize);

        // Dimension 0: v_i = 2^(31-i), the van der Corput / radical-inverse
        // sequence in base 2.
        let mut v0 = [0u32; MATRIX_BITS];
        for (i, slot) in v0.iter_mut().enumerate() {
            *slot = 1u32 << (31 - i);
        }
        all.push(v0);

        for &(s, a, m) in POLYNOMIALS.iter() {
            let s = s as usize;
            debug_assert_eq!(
                m.len(),
                s,
                "initial direction number count must equal the degree"
            );
            let mut v = [0u32; MATRIX_BITS];

            // Seed the first `s` entries directly from the tabulated m_i.
            for i in 0..s {
                v[i] = m[i] << (31 - i);
            }
            // Recurrence: v_i = v_{i-s} XOR (v_{i-s} >> s) XOR (a_k-selected earlier terms).
            for i in s..MATRIX_BITS {
                v[i] = v[i - s] ^ (v[i - s] >> s);
                for k in 1..s {
                    if (a >> (s - 1 - k)) & 1 == 1 {
                        v[i] ^= v[i - k];
                    }
                }
            }
            all.push(v);
        }
        all
    })
}

/// Raw Sobol′ value for `(index, dimension)` as a 32-bit fixed-point fraction.
/// `dimension` must be below [`NUM_BASE_DIMENSIONS`]; padding is handled by
/// [`sample_u32`].
#[inline]
pub fn sobol_u32(index: u32, dimension: u32) -> u32 {
    debug_assert!(dimension < NUM_BASE_DIMENSIONS);
    let v = &matrices()[dimension as usize];
    let mut x = 0u32;
    let mut i = index;
    let mut bit = 0usize;
    while i != 0 {
        if i & 1 == 1 {
            x ^= v[bit];
        }
        i >>= 1;
        bit += 1;
    }
    x
}

/// The Laine-Karras bit permutation used as the core of hash-based Owen
/// scrambling. Operates on a bit-reversed value; see
/// [`nested_uniform_scramble`].
#[inline]
fn laine_karras_permutation(mut x: u32, seed: u32) -> u32 {
    x = x.wrapping_add(seed);
    x ^= x.wrapping_mul(0x6c50_b47c);
    x ^= x.wrapping_mul(0xb82f_1e52);
    x ^= x.wrapping_mul(0xc7af_e638);
    x ^= x.wrapping_mul(0x8d22_f6e6);
    x
}

/// Hash-based nested uniform (Owen) scramble of a 32-bit fixed-point value.
///
/// Owen scrambling recursively permutes the binary digits of the value in a
/// way that preserves the sequence's stratification while making each
/// individual point uniformly distributed. The bit-reversal sandwich turns the
/// Laine-Karras permutation — which mixes low bits into high bits — into one
/// that mixes high bits into low, which is the direction Owen scrambling needs.
#[inline]
pub fn nested_uniform_scramble(x: u32, seed: u32) -> u32 {
    laine_karras_permutation(x.reverse_bits(), seed).reverse_bits()
}

/// Owen-scrambled Sobol′ sample as a 32-bit fixed-point fraction.
///
/// `seed` decorrelates independent sample sets — in practice a per-pixel hash,
/// so that neighbouring pixels do not share the same error pattern (§4.3).
#[inline]
pub fn sample_u32(index: u32, dimension: u32, seed: u32) -> u32 {
    let group = dimension / NUM_BASE_DIMENSIONS;
    let base_dim = dimension % NUM_BASE_DIMENSIONS;
    let group_seed = hash_combine(seed, group.wrapping_mul(0x51633e2d));

    // Shuffling the index randomizes *which* points of the sequence this
    // sample set draws, so different pixels do not all start from the same
    // corner of the domain. It preserves the net property, so the set stays
    // stratified.
    let shuffled = nested_uniform_scramble(index, hash_u32(group_seed));
    let x = sobol_u32(shuffled, base_dim);
    nested_uniform_scramble(x, hash_combine(group_seed, base_dim ^ 0xa511_e9b3))
}

/// Owen-scrambled Sobol′ sample in `[0, 1)`.
#[inline]
pub fn sample(index: u32, dimension: u32, seed: u32) -> f32 {
    // 24 significant bits: the most an f32 can represent exactly, and the
    // `min` guarantees the result never rounds up to 1.0.
    const ONE_MINUS_EPS: f32 = 1.0 - f32::EPSILON / 2.0;
    ((sample_u32(index, dimension, seed) >> 8) as f32 * (1.0 / 16_777_216.0)).min(ONE_MINUS_EPS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every base dimension must be a (0,1)-sequence in base 2: for the first
    /// `2^k` points, each of the `2^k` equal subintervals holds exactly one
    /// point. This is the property that makes Sobol′ worth using, and it is a
    /// direct check on the generated direction numbers.
    #[test]
    fn each_base_dimension_is_a_net() {
        for dim in 0..NUM_BASE_DIMENSIONS {
            for k in 1..=12u32 {
                let n = 1u32 << k;
                let mut seen = vec![false; n as usize];
                for i in 0..n {
                    let stratum = (sobol_u32(i, dim) >> (32 - k)) as usize;
                    assert!(
                        !seen[stratum],
                        "dimension {dim} put two of the first {n} points in stratum {stratum}"
                    );
                    seen[stratum] = true;
                }
            }
        }
    }

    /// The leading dimension pair must be a perfect (0,2)-net: for the first
    /// `2^(2k)` points, each cell of a `2^k x 2^k` grid holds exactly one
    /// point. Dimensions 0 and 1 are the pixel-jitter pair, where this matters
    /// most, and a wrong direction number for dimension 1 breaks it outright.
    #[test]
    fn the_leading_dimension_pair_is_a_perfect_net() {
        for k in 3..=6u32 {
            let n = 1u32 << (2 * k);
            let side = 1usize << k;
            let mut seen = vec![false; side * side];
            for i in 0..n {
                let x = (sobol_u32(i, 0) >> (32 - k)) as usize;
                let y = (sobol_u32(i, 1) >> (32 - k)) as usize;
                let cell = y * side + x;
                assert!(
                    !seen[cell],
                    "dimensions 0/1 collided in cell {cell} at k={k}"
                );
                seen[cell] = true;
            }
        }
    }

    /// Every other consecutive pair must be a low-`t` net. Sobol′ projections
    /// beyond the first few have `t > 0` — cells can hold `2^t` points — so
    /// perfect one-per-cell coverage is the wrong expectation here. What *is*
    /// diagnostic is that the deviation stays tiny and structured: `t <= 1`
    /// means at most 2 points per cell and at most half the cells empty. A
    /// corrupt direction number degrades the projection towards random, which
    /// at this resolution leaves ~37% of cells empty with 5-6 points in the
    /// fullest, and would fail these bounds immediately.
    #[test]
    fn every_dimension_pair_is_a_low_t_net() {
        for dim in 0..NUM_BASE_DIMENSIONS - 1 {
            for k in 3..=5u32 {
                let n = 1u32 << (2 * k);
                let side = 1usize << k;
                let mut counts = vec![0u32; side * side];
                for i in 0..n {
                    let x = (sobol_u32(i, dim) >> (32 - k)) as usize;
                    let y = (sobol_u32(i, dim + 1) >> (32 - k)) as usize;
                    counts[y * side + x] += 1;
                }
                let max = counts.iter().copied().max().unwrap();
                let empty = counts.iter().filter(|&&c| c == 0).count();
                assert!(
                    max <= 2,
                    "dimensions {dim}/{} at k={k}: {max} points in one cell",
                    dim + 1
                );
                assert!(
                    empty * 2 <= counts.len(),
                    "dimensions {dim}/{} at k={k}: {empty} of {} cells empty",
                    dim + 1,
                    counts.len()
                );
            }
        }
    }

    /// The index shuffle must map a dyadic block onto a dyadic block
    /// bijectively. That is precisely what preserves the net property: the
    /// first `2^k` samples of a scrambled set are *some* aligned block of
    /// `2^k` consecutive Sobol′ points, which is itself a net, rather than an
    /// arbitrary scattering of points that would not be.
    ///
    /// Owen scrambling permutes each bit as a function of the *more*
    /// significant bits, so with the high bits held at zero the low `k` bits
    /// carry the permutation and the high bits come out constant.
    #[test]
    fn the_index_shuffle_maps_a_dyadic_block_onto_a_dyadic_block() {
        let seed = 0x1234_5678;
        let k = 16;
        let n = 1u32 << k;
        let mut seen = std::collections::HashSet::with_capacity(n as usize);
        let block = nested_uniform_scramble(0, seed) >> k;
        for i in 0..n {
            let s = nested_uniform_scramble(i, seed);
            assert_eq!(s >> k, block, "index {i} left the block");
            assert!(
                seen.insert(s & (n - 1)),
                "index {i} collided within the block"
            );
        }
    }

    #[test]
    fn samples_stay_in_unit_interval() {
        for dim in 0..40 {
            for i in 0..5000 {
                let s = sample(i, dim, 12345);
                assert!((0.0..1.0).contains(&s), "dim {dim} sample {i} = {s}");
            }
        }
    }

    #[test]
    fn scrambling_is_deterministic() {
        for dim in [0u32, 3, 9, 63] {
            for i in [0u32, 1, 17, 4095] {
                assert_eq!(sample(i, dim, 99), sample(i, dim, 99));
            }
        }
    }

    /// Different seeds must produce genuinely different sequences, otherwise
    /// per-pixel decorrelation (§4.3) does nothing.
    #[test]
    fn different_seeds_decorrelate() {
        let a: Vec<f32> = (0..64).map(|i| sample(i, 2, 1)).collect();
        let b: Vec<f32> = (0..64).map(|i| sample(i, 2, 2)).collect();
        let matches = a
            .iter()
            .zip(&b)
            .filter(|(x, y)| (*x - *y).abs() < 1e-6)
            .count();
        assert!(
            matches < 4,
            "{matches} of 64 samples coincided across seeds"
        );
    }

    /// The mean of a scrambled dimension must converge to 1/2 — the direct
    /// statement that the sampler is unbiased.
    #[test]
    fn scrambled_mean_converges() {
        for dim in [0u32, 5, 11, 30] {
            let n = 1 << 16;
            let sum: f64 = (0..n).map(|i| sample(i, dim, 7) as f64).sum();
            let mean = sum / n as f64;
            assert!((mean - 0.5).abs() < 0.002, "dimension {dim} mean {mean}");
        }
    }

    /// Padded dimensions must still be stratified *within* a group, because
    /// that is the whole justification for the sampler's dimension allocation.
    #[test]
    fn padding_preserves_within_group_stratification() {
        // Dimensions 8 and 9 are group 1, base dims 0 and 1.
        let k = 5u32;
        let n = 1u32 << (2 * k);
        let side = 1usize << k;
        let mut counts = vec![0u32; side * side];
        for i in 0..n {
            let x = (sample_u32(i, 8, 4242) >> (32 - k)) as usize;
            let y = (sample_u32(i, 9, 4242) >> (32 - k)) as usize;
            counts[y * side + x] += 1;
        }
        assert!(
            counts.iter().all(|&c| c == 1),
            "group-1 dimension pair lost its stratification"
        );
    }
}
