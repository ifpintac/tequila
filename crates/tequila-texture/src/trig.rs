//! Sine, cosine and tangent as the graphics card works them out
//! (`tequila-render-gpu`'s `nodes.wgsl`, `cy_trig`), for the nodes and
//! patterns the card runs: one implementation on both, so a pattern's
//! heights are the same to the bit on either (`docs/07` R3). The platform's
//! `sinf` cannot be matched on the card - it rounds correctly all but about
//! once in four hundred times, and how it rounds the rest is its own.
//!
//! Cephes' `sinf` and `cosf`: the argument brought to within a quarter turn
//! by multiples of pi/4 taken off in three parts, each step fused, then a
//! polynomial, its steps fused as written. Within a unit in the last place
//! of the true value for arguments up to some thousands; further out the
//! reduction's error grows with the argument (about 2^-48 of pi/4 for each
//! multiple of it taken off).

/// `x as i32`'s float, rounded as Rust rounds it: `j` is never negative.
#[inline]
fn whole(j: i32) -> f32 {
    j as f32
}

/// (sin x, cos x). The constants are Cephes', written as the card's are.
#[inline]
#[allow(clippy::excessive_precision)]
pub fn sin_cos(xx: f32) -> (f32, f32) {
    let x0 = xx.abs();
    if x0.is_nan() || x0 > 3.4e38 {
        return (f32::NAN, f32::NAN);
    }
    let mut j = (x0 * 1.27323954473516) as i32;
    let mut y = whole(j);
    if j & 1 != 0 {
        j = j.wrapping_add(1);
        y += 1.0;
    }
    let k = (j >> 1) & 3;
    let mut x = (-y).mul_add(0.78515625, x0);
    x = (-y).mul_add(2.4187564849853515625e-4, x);
    x = (-y).mul_add(3.77489497744594108e-8, x);
    let z = x * x;
    let p = (-1.9515295891e-4f32).mul_add(z, 8.3321608736e-3).mul_add(z, -1.6666654611e-1);
    let s = (p * z).mul_add(x, x);
    let q = (2.443315711809948e-5f32).mul_add(z, -1.388731625493765e-3).mul_add(z, 4.166664568298827e-2);
    let c = (q * z).mul_add(z, (-0.5f32).mul_add(z, 1.0));
    let (sine, cosine) = match k {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    };
    (if xx < 0.0 { -sine } else { sine }, cosine)
}

#[inline]
pub fn sin(x: f32) -> f32 {
    sin_cos(x).0
}

#[inline]
pub fn cos(x: f32) -> f32 {
    sin_cos(x).1
}

/// The sine over the cosine, one division.
#[inline]
pub fn tan(x: f32) -> f32 {
    let (s, c) = sin_cos(x);
    s / c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Within a couple of units in the last place of the platform's, a
    /// quarter turn either side of any argument up to a thousand, and the
    /// two agree on every quarter turn's sign.
    #[test]
    fn the_cards_sine_is_the_sine() {
        let mut worst = 0.0f32;
        for i in -200_000..200_000 {
            let x = i as f32 * 0.005;
            let (s, c) = sin_cos(x);
            worst = worst.max((s - x.sin()).abs()).max((c - x.cos()).abs());
        }
        assert!(worst < 3e-7, "{worst}");
        assert_eq!(sin_cos(0.0), (0.0, 1.0));
        assert!(sin(f32::NAN).is_nan() && cos(f32::INFINITY).is_nan());
    }
}
