//! Cycles' Perlin noise and its fractals (`kernel/svm/noise.h`,
//! `fractal_noise.h`, `noisetex.h`), in one to four dimensions: the Noise
//! Texture node, value for value. The scalar (GPU) formulation is the one
//! ported; Cycles' SIMD processor kernels round differently in the last bit.

use super::hash::*;

#[inline]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + t * (b - a)
}

#[inline]
fn fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

#[inline]
fn negate_if(value: f32, condition: i32) -> f32 {
    if condition != 0 { -value } else { value }
}

/// `floorf`, its integer, and what is left.
#[inline]
pub(crate) fn floorfrac(x: f32) -> (f32, i32) {
    let f = x.floor();
    (x - f, f as i32)
}

#[inline]
fn grad1(hash: u32, x: f32) -> f32 {
    let h = (hash & 15) as i32;
    let g = 1.0 + (h & 7) as f32;
    negate_if(g, h & 8) * x
}

#[inline]
fn grad2(hash: u32, x: f32, y: f32) -> f32 {
    let h = (hash & 7) as i32;
    let u = if h < 4 { x } else { y };
    let v = 2.0 * if h < 4 { y } else { x };
    negate_if(u, h & 1) + negate_if(v, h & 2)
}

#[inline]
fn grad3(hash: u32, x: f32, y: f32, z: f32) -> f32 {
    let h = (hash & 15) as i32;
    let u = if h < 8 { x } else { y };
    let vt = if h == 12 || h == 14 { x } else { z };
    let v = if h < 4 { y } else { vt };
    negate_if(u, h & 1) + negate_if(v, h & 2)
}

#[inline]
fn grad4(hash: u32, x: f32, y: f32, z: f32, w: f32) -> f32 {
    let h = (hash & 31) as i32;
    let u = if h < 24 { x } else { y };
    let v = if h < 16 { y } else { z };
    let s = if h < 8 { z } else { w };
    negate_if(u, h & 1) + negate_if(v, h & 2) + negate_if(s, h & 4)
}

#[inline]
fn bi_mix(v0: f32, v1: f32, v2: f32, v3: f32, x: f32, y: f32) -> f32 {
    let x1 = 1.0 - x;
    (1.0 - y) * (v0 * x1 + v1 * x) + y * (v2 * x1 + v3 * x)
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn tri_mix(v: [f32; 8], x: f32, y: f32, z: f32) -> f32 {
    let (x1, y1, z1) = (1.0 - x, 1.0 - y, 1.0 - z);
    z1 * (y1 * (v[0] * x1 + v[1] * x) + y * (v[2] * x1 + v[3] * x))
        + z * (y1 * (v[4] * x1 + v[5] * x) + y * (v[6] * x1 + v[7] * x))
}

pub fn perlin_1d(x: f32) -> f32 {
    let (fx, xi) = floorfrac(x);
    let u = fade(fx);
    mix(grad1(hash_uint(xi as u32), fx), grad1(hash_uint(xi.wrapping_add(1) as u32), fx - 1.0), u)
}

pub fn perlin_2d(x: f32, y: f32) -> f32 {
    let (fx, xi) = floorfrac(x);
    let (fy, yi) = floorfrac(y);
    let (u, v) = (fade(fx), fade(fy));
    let h = |dx: i32, dy: i32| hash_uint2(xi.wrapping_add(dx) as u32, yi.wrapping_add(dy) as u32);
    bi_mix(
        grad2(h(0, 0), fx, fy),
        grad2(h(1, 0), fx - 1.0, fy),
        grad2(h(0, 1), fx, fy - 1.0),
        grad2(h(1, 1), fx - 1.0, fy - 1.0),
        u,
        v,
    )
}

pub fn perlin_3d(x: f32, y: f32, z: f32) -> f32 {
    let (fx, xi) = floorfrac(x);
    let (fy, yi) = floorfrac(y);
    let (fz, zi) = floorfrac(z);
    let (u, v, w) = (fade(fx), fade(fy), fade(fz));
    let g = |dx: i32, dy: i32, dz: i32| {
        grad3(
            hash_uint3(xi.wrapping_add(dx) as u32, yi.wrapping_add(dy) as u32, zi.wrapping_add(dz) as u32),
            fx - dx as f32,
            fy - dy as f32,
            fz - dz as f32,
        )
    };
    tri_mix(
        [g(0, 0, 0), g(1, 0, 0), g(0, 1, 0), g(1, 1, 0), g(0, 0, 1), g(1, 0, 1), g(0, 1, 1), g(1, 1, 1)],
        u,
        v,
        w,
    )
}

pub fn perlin_4d(x: f32, y: f32, z: f32, w: f32) -> f32 {
    let (fx, xi) = floorfrac(x);
    let (fy, yi) = floorfrac(y);
    let (fz, zi) = floorfrac(z);
    let (fw, wi) = floorfrac(w);
    let (u, v, t, s) = (fade(fx), fade(fy), fade(fz), fade(fw));
    let g = |dx: i32, dy: i32, dz: i32, dw: i32| {
        grad4(
            hash_uint4(
                xi.wrapping_add(dx) as u32,
                yi.wrapping_add(dy) as u32,
                zi.wrapping_add(dz) as u32,
                wi.wrapping_add(dw) as u32,
            ),
            fx - dx as f32,
            fy - dy as f32,
            fz - dz as f32,
            fw - dw as f32,
        )
    };
    let near = tri_mix(
        [g(0, 0, 0, 0), g(1, 0, 0, 0), g(0, 1, 0, 0), g(1, 1, 0, 0), g(0, 0, 1, 0), g(1, 0, 1, 0), g(0, 1, 1, 0), g(1, 1, 1, 0)],
        u,
        v,
        t,
    );
    let far = tri_mix(
        [g(0, 0, 0, 1), g(1, 0, 0, 1), g(0, 1, 0, 1), g(1, 1, 0, 1), g(0, 0, 1, 1), g(1, 0, 1, 1), g(0, 1, 1, 1), g(1, 1, 1, 1)],
        u,
        v,
        t,
    );
    mix(near, far, s)
}

/// Repeat every 100000 on each axis, half a cell over from a million out,
/// as Cycles keeps far coordinates precise.
#[inline]
fn wrap(p: f32) -> f32 {
    let correction = if p.abs() >= 1_000_000.0 { 0.5 } else { 0.0 };
    p % 100_000.0 + correction
}

/// Signed noise in about [-1, 1], with the per-dimension scales the OSL
/// developers measured.
pub fn snoise_1d(p: f32) -> f32 {
    0.2500 * perlin_1d(wrap(p))
}

pub fn snoise_2d(p: [f32; 2]) -> f32 {
    0.6616 * perlin_2d(wrap(p[0]), wrap(p[1]))
}

pub fn snoise_3d(p: [f32; 3]) -> f32 {
    0.9820 * perlin_3d(wrap(p[0]), wrap(p[1]), wrap(p[2]))
}

pub fn snoise_4d(p: [f32; 4]) -> f32 {
    0.8344 * perlin_4d(wrap(p[0]), wrap(p[1]), wrap(p[2]), wrap(p[3]))
}

/// A point noise is read at, in one to four dimensions.
pub trait NoisePoint: Copy {
    fn snoise(self) -> f32;
    fn scale(self, s: f32) -> Self;
    fn add(self, other: Self) -> Self;
    /// Cycles' seed offset (`random_floatN_offset`): each component in
    /// [100, 200].
    fn seed_offset(seed: f32) -> Self;
    /// Each component moved by its own signed noise times `amount`, from
    /// offsets 0, 1, 2... - the Noise Texture's distortion.
    fn distort(self, amount: f32) -> Self;
    /// The two offsets the colour output's green and blue read at.
    const COLOR_SEEDS: [f32; 2];
}

#[inline]
fn offset1(seed: f32) -> f32 {
    100.0 + hash_float_to_float(seed) * 100.0
}

#[inline]
fn offset_n(seed: f32, i: f32) -> f32 {
    100.0 + hash_float2_to_float([seed, i]) * 100.0
}

impl NoisePoint for f32 {
    fn snoise(self) -> f32 {
        snoise_1d(self)
    }
    fn scale(self, s: f32) -> Self {
        s * self
    }
    fn add(self, o: Self) -> Self {
        self + o
    }
    fn seed_offset(seed: f32) -> Self {
        offset1(seed)
    }
    fn distort(self, amount: f32) -> Self {
        self + snoise_1d(self + offset1(0.0)) * amount
    }
    const COLOR_SEEDS: [f32; 2] = [1.0, 2.0];
}

impl NoisePoint for [f32; 2] {
    fn snoise(self) -> f32 {
        snoise_2d(self)
    }
    fn scale(self, s: f32) -> Self {
        [s * self[0], s * self[1]]
    }
    fn add(self, o: Self) -> Self {
        [self[0] + o[0], self[1] + o[1]]
    }
    fn seed_offset(seed: f32) -> Self {
        [offset_n(seed, 0.0), offset_n(seed, 1.0)]
    }
    fn distort(self, amount: f32) -> Self {
        let d = [
            snoise_2d(self.add(Self::seed_offset(0.0))) * amount,
            snoise_2d(self.add(Self::seed_offset(1.0))) * amount,
        ];
        self.add(d)
    }
    const COLOR_SEEDS: [f32; 2] = [2.0, 3.0];
}

impl NoisePoint for [f32; 3] {
    fn snoise(self) -> f32 {
        snoise_3d(self)
    }
    fn scale(self, s: f32) -> Self {
        [s * self[0], s * self[1], s * self[2]]
    }
    fn add(self, o: Self) -> Self {
        [self[0] + o[0], self[1] + o[1], self[2] + o[2]]
    }
    fn seed_offset(seed: f32) -> Self {
        [offset_n(seed, 0.0), offset_n(seed, 1.0), offset_n(seed, 2.0)]
    }
    fn distort(self, amount: f32) -> Self {
        let d = [
            snoise_3d(self.add(Self::seed_offset(0.0))) * amount,
            snoise_3d(self.add(Self::seed_offset(1.0))) * amount,
            snoise_3d(self.add(Self::seed_offset(2.0))) * amount,
        ];
        self.add(d)
    }
    const COLOR_SEEDS: [f32; 2] = [3.0, 4.0];
}

impl NoisePoint for [f32; 4] {
    fn snoise(self) -> f32 {
        snoise_4d(self)
    }
    fn scale(self, s: f32) -> Self {
        [s * self[0], s * self[1], s * self[2], s * self[3]]
    }
    fn add(self, o: Self) -> Self {
        [self[0] + o[0], self[1] + o[1], self[2] + o[2], self[3] + o[3]]
    }
    fn seed_offset(seed: f32) -> Self {
        [offset_n(seed, 0.0), offset_n(seed, 1.0), offset_n(seed, 2.0), offset_n(seed, 3.0)]
    }
    fn distort(self, amount: f32) -> Self {
        let d = [
            snoise_4d(self.add(Self::seed_offset(0.0))) * amount,
            snoise_4d(self.add(Self::seed_offset(1.0))) * amount,
            snoise_4d(self.add(Self::seed_offset(2.0))) * amount,
            snoise_4d(self.add(Self::seed_offset(3.0))) * amount,
        ];
        self.add(d)
    }
    const COLOR_SEEDS: [f32; 2] = [4.0, 5.0];
}

/// How the octaves of a Noise Texture combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FractalKind {
    Multifractal,
    #[default]
    Fbm,
    HybridMultifractal,
    RidgedMultifractal,
    HeteroTerrain,
}

impl FractalKind {
    pub const ALL: [FractalKind; 5] = [
        FractalKind::Fbm,
        FractalKind::Multifractal,
        FractalKind::HybridMultifractal,
        FractalKind::RidgedMultifractal,
        FractalKind::HeteroTerrain,
    ];

    /// Cycles' name for it (the noise node's `type`).
    pub fn cycles(self) -> &'static str {
        match self {
            FractalKind::Multifractal => "multifractal",
            FractalKind::Fbm => "fBM",
            FractalKind::HybridMultifractal => "hybrid_multifractal",
            FractalKind::RidgedMultifractal => "ridged_multifractal",
            FractalKind::HeteroTerrain => "hetero_terrain",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FractalKind::Multifractal => "Multifractal",
            FractalKind::Fbm => "fBM",
            FractalKind::HybridMultifractal => "Hybrid multifractal",
            FractalKind::RidgedMultifractal => "Ridged multifractal",
            FractalKind::HeteroTerrain => "Hetero terrain",
        }
    }
}

#[inline]
fn whole(detail: f32) -> i32 {
    detail as i32
}

pub fn fbm<P: NoisePoint>(p: P, detail: f32, roughness: f32, lacunarity: f32, normalize: bool) -> f32 {
    let (mut fscale, mut amp, mut maxamp, mut sum) = (1.0f32, 1.0f32, 0.0f32, 0.0f32);
    for _ in 0..=whole(detail) {
        let t = p.scale(fscale).snoise();
        sum += t * amp;
        maxamp += amp;
        amp *= roughness;
        fscale *= lacunarity;
    }
    let rmd = detail - detail.floor();
    if rmd != 0.0 {
        let t = p.scale(fscale).snoise();
        let sum2 = sum + t * amp;
        return if normalize {
            mix(0.5 * sum / maxamp + 0.5, 0.5 * sum2 / (maxamp + amp) + 0.5, rmd)
        } else {
            mix(sum, sum2, rmd)
        };
    }
    if normalize { 0.5 * sum / maxamp + 0.5 } else { sum }
}

pub fn multi_fractal<P: NoisePoint>(mut p: P, detail: f32, roughness: f32, lacunarity: f32) -> f32 {
    let (mut value, mut pwr) = (1.0f32, 1.0f32);
    for _ in 0..=whole(detail) {
        value *= pwr * p.snoise() + 1.0;
        pwr *= roughness;
        p = p.scale(lacunarity);
    }
    let rmd = detail - detail.floor();
    if rmd != 0.0 {
        value *= rmd * pwr * p.snoise() + 1.0;
    }
    value
}

pub fn hetero_terrain<P: NoisePoint>(mut p: P, detail: f32, roughness: f32, lacunarity: f32, offset: f32) -> f32 {
    let mut pwr = roughness;
    let mut value = offset + p.snoise();
    p = p.scale(lacunarity);
    for _ in 1..=whole(detail) {
        let increment = (p.snoise() + offset) * pwr * value;
        value += increment;
        pwr *= roughness;
        p = p.scale(lacunarity);
    }
    let rmd = detail - detail.floor();
    if rmd != 0.0 {
        let increment = (p.snoise() + offset) * pwr * value;
        value += rmd * increment;
    }
    value
}

pub fn hybrid_multi_fractal<P: NoisePoint>(mut p: P, detail: f32, roughness: f32, lacunarity: f32, offset: f32, gain: f32) -> f32 {
    let (mut pwr, mut value, mut weight) = (1.0f32, 0.0f32, 1.0f32);
    let mut i = 0;
    while weight > 0.001 && i <= whole(detail) {
        weight = weight.min(1.0);
        let signal = (p.snoise() + offset) * pwr;
        pwr *= roughness;
        value += weight * signal;
        weight *= gain * signal;
        p = p.scale(lacunarity);
        i += 1;
    }
    let rmd = detail - detail.floor();
    if rmd != 0.0 && weight > 0.001 {
        weight = weight.min(1.0);
        let signal = (p.snoise() + offset) * pwr;
        value += rmd * weight * signal;
    }
    value
}

pub fn ridged_multi_fractal<P: NoisePoint>(mut p: P, detail: f32, roughness: f32, lacunarity: f32, offset: f32, gain: f32) -> f32 {
    let mut pwr = roughness;
    let mut signal = offset - p.snoise().abs();
    signal *= signal;
    let mut value = signal;
    for _ in 1..=whole(detail) {
        p = p.scale(lacunarity);
        let weight = (signal * gain).clamp(0.0, 1.0);
        signal = offset - p.snoise().abs();
        signal *= signal;
        signal *= weight;
        value += signal * pwr;
        pwr *= roughness;
    }
    value
}

/// The Noise Texture node's settings, as Cycles clamps them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoiseSettings {
    pub kind: FractalKind,
    pub detail: f32,
    pub roughness: f32,
    pub lacunarity: f32,
    pub offset: f32,
    pub gain: f32,
    pub distortion: f32,
    pub normalize: bool,
}

impl Default for NoiseSettings {
    /// Blender's defaults for a new Noise Texture.
    fn default() -> Self {
        NoiseSettings { kind: FractalKind::Fbm, detail: 2.0, roughness: 0.5, lacunarity: 2.0, offset: 0.0, gain: 1.0, distortion: 0.0, normalize: true }
    }
}

impl NoiseSettings {
    fn select<P: NoisePoint>(&self, p: P) -> f32 {
        let (d, r, l) = (self.detail.clamp(0.0, 15.0), self.roughness.max(0.0), self.lacunarity);
        match self.kind {
            FractalKind::Multifractal => multi_fractal(p, d, r, l),
            FractalKind::Fbm => fbm(p, d, r, l, self.normalize),
            FractalKind::HybridMultifractal => hybrid_multi_fractal(p, d, r, l, self.offset, self.gain),
            FractalKind::RidgedMultifractal => ridged_multi_fractal(p, d, r, l, self.offset, self.gain),
            FractalKind::HeteroTerrain => hetero_terrain(p, d, r, l, self.offset),
        }
    }

    /// The node's Fac and Color at `p`, already multiplied by its Scale.
    pub fn texture<P: NoisePoint>(&self, p: P) -> (f32, [f32; 3]) {
        let p = if self.distortion != 0.0 { p.distort(self.distortion) } else { p };
        let value = self.select(p);
        let [g, b] = P::COLOR_SEEDS;
        (value, [value, self.select(p.add(P::seed_offset(g))), self.select(p.add(P::seed_offset(b)))])
    }

    /// Just the Fac, which is all most uses read.
    pub fn value<P: NoisePoint>(&self, p: P) -> f32 {
        let p = if self.distortion != 0.0 { p.distort(self.distortion) } else { p };
        self.select(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Perlin noise is zero at every lattice point, in every dimension.
    #[test]
    fn noise_vanishes_on_the_lattice() {
        for i in -3..3 {
            let x = i as f32;
            assert!(perlin_1d(x).abs() < 1e-6);
            assert!(perlin_2d(x, 2.0).abs() < 1e-6);
            assert!(perlin_3d(x, 2.0, -1.0).abs() < 1e-6);
            assert!(perlin_4d(x, 2.0, -1.0, 5.0).abs() < 1e-6);
        }
    }

    /// Normalized fBM sits around a half, within [0, 1].
    #[test]
    fn normalized_fbm_is_a_unit_value() {
        let s = NoiseSettings::default();
        let mut sum = 0.0;
        for i in 0..2000 {
            let p = [i as f32 * 0.137, i as f32 * 0.071, i as f32 * 0.029];
            let v = s.value(p);
            assert!((-0.01..=1.01).contains(&v), "{v}");
            sum += v;
        }
        assert!((sum / 2000.0 - 0.5).abs() < 0.05);
    }
}
