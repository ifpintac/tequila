//! Cycles' smaller texture nodes: Wave, Magic, Brick, Checker, Gradient,
//! White Noise and Gabor (`kernel/svm/wave.h`, `magic.h`, `brick.h`,
//! `checker.h`, `gradient.h`, `white_noise.h`, `gabor.h`).

use super::hash::*;
use super::noise::fbm;
use std::f32::consts::{FRAC_PI_2, PI};

const TAU: f32 = 2.0 * PI;

// ---- Wave --------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveKind {
    #[default]
    Bands,
    Rings,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveDirection {
    #[default]
    X,
    Y,
    Z,
    /// Bands: diagonal. Rings: spherical.
    Diagonal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WaveProfile {
    #[default]
    Sine,
    Saw,
    Triangle,
}

impl WaveKind {
    pub fn cycles(self) -> &'static str {
        match self {
            WaveKind::Bands => "bands",
            WaveKind::Rings => "rings",
        }
    }
}

impl WaveDirection {
    pub fn cycles_bands(self) -> &'static str {
        match self {
            WaveDirection::X => "x",
            WaveDirection::Y => "y",
            WaveDirection::Z => "z",
            WaveDirection::Diagonal => "diagonal",
        }
    }

    pub fn cycles_rings(self) -> &'static str {
        match self {
            WaveDirection::X => "x",
            WaveDirection::Y => "y",
            WaveDirection::Z => "z",
            WaveDirection::Diagonal => "spherical",
        }
    }
}

impl WaveProfile {
    pub fn cycles(self) -> &'static str {
        match self {
            WaveProfile::Sine => "sine",
            WaveProfile::Saw => "saw",
            WaveProfile::Triangle => "tri",
        }
    }
}

/// The Wave Texture's value at `p` (already multiplied by Scale).
#[allow(clippy::too_many_arguments)]
pub fn wave(
    kind: WaveKind,
    direction: WaveDirection,
    profile: WaveProfile,
    p: [f32; 3],
    distortion: f32,
    detail: f32,
    detail_scale: f32,
    detail_roughness: f32,
    phase: f32,
) -> f32 {
    let p = p.map(|v| (v + 0.000001) * 0.999999);
    let mut n = match kind {
        WaveKind::Bands => match direction {
            WaveDirection::X => p[0] * 20.0,
            WaveDirection::Y => p[1] * 20.0,
            WaveDirection::Z => p[2] * 20.0,
            WaveDirection::Diagonal => (p[0] + p[1] + p[2]) * 10.0,
        },
        WaveKind::Rings => {
            let rp = match direction {
                WaveDirection::X => [0.0, p[1], p[2]],
                WaveDirection::Y => [p[0], 0.0, p[2]],
                WaveDirection::Z => [p[0], p[1], 0.0],
                WaveDirection::Diagonal => p,
            };
            (rp[0] * rp[0] + rp[1] * rp[1] + rp[2] * rp[2]).sqrt() * 20.0
        }
    };
    n += phase;
    if distortion != 0.0 {
        n += distortion * (fbm(p.map(|v| v * detail_scale), detail, detail_roughness, 2.0, true) * 2.0 - 1.0);
    }
    match profile {
        WaveProfile::Sine => 0.5 + 0.5 * (n - FRAC_PI_2).sin(),
        WaveProfile::Saw => {
            let n = n / TAU;
            n - n.floor()
        }
        WaveProfile::Triangle => {
            let n = n / TAU;
            (n - (n + 0.5).floor()).abs() * 2.0
        }
    }
}

// ---- Magic -------------------------------------------------------------

/// The Magic Texture's colour at `p` (not multiplied by Scale: the node
/// passes it in). Its Fac is the colour's average.
pub fn magic(p: [f32; 3], scale: f32, depth: i32, mut distortion: f32) -> [f32; 3] {
    let px = p[0] % TAU;
    let py = p[1] % TAU;
    let pz = p[2] % TAU;
    let mut x = ((px + py + pz) * 5.0 * scale).sin();
    let mut y = ((-px + py - pz) * 5.0 * scale).cos();
    let mut z = -((-px - py + pz) * 5.0 * scale).cos();
    let n = depth;
    if n > 0 {
        x *= distortion;
        y *= distortion;
        z *= distortion;
        y = -(x - y + z).cos();
        y *= distortion;
        if n > 1 {
            x = (x - y - z).cos();
            x *= distortion;
            if n > 2 {
                z = (-x - y - z).sin();
                z *= distortion;
                if n > 3 {
                    x = -(-x + y - z).cos();
                    x *= distortion;
                    if n > 4 {
                        y = -(-x + y + z).sin();
                        y *= distortion;
                        if n > 5 {
                            y = -(-x + y + z).cos();
                            y *= distortion;
                            if n > 6 {
                                x = (x + y + z).cos();
                                x *= distortion;
                                if n > 7 {
                                    z = (x + y - z).sin();
                                    z *= distortion;
                                    if n > 8 {
                                        x = -(-x - y + z).cos();
                                        x *= distortion;
                                        if n > 9 {
                                            y = -(x - y + z).sin();
                                            y *= distortion;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if distortion != 0.0 {
        distortion *= 2.0;
        x /= distortion;
        y /= distortion;
        z /= distortion;
    }
    [0.5 - x, 0.5 - y, 0.5 - z]
}

// ---- Brick -------------------------------------------------------------

fn brick_noise(n: u32) -> f32 {
    let mut n = n.wrapping_add(1013) & 0x7fff_ffff;
    n ^= n >> 13;
    let nn = n
        .wrapping_mul(n.wrapping_mul(n).wrapping_mul(60493).wrapping_add(19990303))
        .wrapping_add(1376312589)
        & 0x7fff_ffff;
    0.5 * (nn as f32 / 1073741824.0)
}

fn smoothstepf(f: f32) -> f32 {
    if f <= 0.0 {
        0.0
    } else if f >= 1.0 {
        1.0
    } else {
        let ff = f * f;
        3.0 * ff - 2.0 * ff * f
    }
}

/// The Brick Texture's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrickSettings {
    pub mortar_size: f32,
    pub mortar_smooth: f32,
    pub bias: f32,
    pub brick_width: f32,
    pub row_height: f32,
    pub offset_amount: f32,
    pub offset_frequency: i32,
    pub squash_amount: f32,
    pub squash_frequency: i32,
}

impl Default for BrickSettings {
    /// Blender's defaults for a new Brick Texture.
    fn default() -> Self {
        BrickSettings {
            mortar_size: 0.02,
            mortar_smooth: 0.1,
            bias: 0.0,
            brick_width: 0.5,
            row_height: 0.25,
            offset_amount: 0.5,
            offset_frequency: 2,
            squash_amount: 1.0,
            squash_frequency: 2,
        }
    }
}

impl BrickSettings {
    /// The brick's tint (0 to 1, for mixing its two colours) and how much
    /// of `p` (already multiplied by Scale) is mortar.
    pub fn brick(&self, p: [f32; 3]) -> (f32, f32) {
        let mut brick_width = self.brick_width;
        let row_height = self.row_height;
        let rownum = (p[1] / row_height).floor() as i32;
        let mut offset = 0.0;
        if self.offset_frequency != 0 && self.squash_frequency != 0 {
            brick_width *= if rownum % self.squash_frequency != 0 { 1.0 } else { self.squash_amount };
            offset = if rownum % self.offset_frequency != 0 { 0.0 } else { brick_width * self.offset_amount };
        }
        let bricknum = ((p[0] + offset) / brick_width).floor() as i32;
        let x = (p[0] + offset) - brick_width * bricknum as f32;
        let y = p[1] - row_height * rownum as f32;
        let key = (rownum.wrapping_shl(16)).wrapping_add(bricknum & 0xFFFF) as u32;
        let tint = (brick_noise(key) + self.bias).clamp(0.0, 1.0);
        let mut min_dist = x.min(y).min((brick_width - x).min(row_height - y));
        let mortar = if min_dist >= self.mortar_size {
            0.0
        } else if self.mortar_smooth == 0.0 {
            1.0
        } else {
            min_dist = 1.0 - min_dist / self.mortar_size;
            smoothstepf(min_dist / self.mortar_smooth)
        };
        (tint, mortar)
    }

    /// The node's Color and Fac from its three colours.
    pub fn texture(&self, p: [f32; 3], color1: [f32; 3], color2: [f32; 3], mortar: [f32; 3]) -> ([f32; 3], f32) {
        let (tint, f) = self.brick(p);
        let mut c1 = color1;
        if f != 1.0 {
            let facm = 1.0 - tint;
            c1 = std::array::from_fn(|i| facm * color1[i] + tint * color2[i]);
        }
        (std::array::from_fn(|i| c1[i] * (1.0 - f) + mortar[i] * f), f)
    }
}

// ---- Checker -----------------------------------------------------------

/// 1 on one colour's squares, 0 on the other's, at `p` (already multiplied
/// by Scale).
pub fn checker(p: [f32; 3]) -> f32 {
    let p = p.map(|v| (v + 0.000001) * 0.999999);
    let xi = (p[0].floor() as i32).abs();
    let yi = (p[1].floor() as i32).abs();
    let zi = (p[2].floor() as i32).abs();
    if (xi % 2 == yi % 2) == (zi % 2 != 0) { 1.0 } else { 0.0 }
}

// ---- Gradient ----------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GradientKind {
    #[default]
    Linear,
    Quadratic,
    Easing,
    Diagonal,
    Spherical,
    QuadraticSphere,
    Radial,
}

impl GradientKind {
    pub const ALL: [GradientKind; 7] = [
        GradientKind::Linear,
        GradientKind::Quadratic,
        GradientKind::Easing,
        GradientKind::Diagonal,
        GradientKind::Spherical,
        GradientKind::QuadraticSphere,
        GradientKind::Radial,
    ];

    pub fn cycles(self) -> &'static str {
        match self {
            GradientKind::Linear => "linear",
            GradientKind::Quadratic => "quadratic",
            GradientKind::Easing => "easing",
            GradientKind::Diagonal => "diagonal",
            GradientKind::Spherical => "spherical",
            GradientKind::QuadraticSphere => "quadratic_sphere",
            GradientKind::Radial => "radial",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            GradientKind::Linear => "Linear",
            GradientKind::Quadratic => "Quadratic",
            GradientKind::Easing => "Easing",
            GradientKind::Diagonal => "Diagonal",
            GradientKind::Spherical => "Spherical",
            GradientKind::QuadraticSphere => "Quadratic sphere",
            GradientKind::Radial => "Radial",
        }
    }
}

/// The Gradient Texture's Fac at `p`, clamped to [0, 1].
pub fn gradient(kind: GradientKind, p: [f32; 3]) -> f32 {
    let [x, y, z] = p;
    let f = match kind {
        GradientKind::Linear => x,
        GradientKind::Quadratic => {
            let r = x.max(0.0);
            r * r
        }
        GradientKind::Easing => {
            let r = x.clamp(0.0, 1.0);
            let t = r * r;
            3.0 * t - 2.0 * t * r
        }
        GradientKind::Diagonal => (x + y) * 0.5,
        GradientKind::Radial => y.atan2(x) / TAU + 0.5,
        GradientKind::Spherical | GradientKind::QuadraticSphere => {
            let r = (0.999999 - (x * x + y * y + z * z).sqrt()).max(0.0);
            if kind == GradientKind::QuadraticSphere { r * r } else { r }
        }
    };
    f.clamp(0.0, 1.0)
}

// ---- White Noise -------------------------------------------------------

/// The White Noise Texture's Value and Color, in `dims` dimensions (1
/// reads `w`, 4 reads `p` and `w`).
pub fn white_noise(dims: u8, p: [f32; 3], w: f32) -> (f32, [f32; 3]) {
    match dims {
        1 => (hash_float_to_float(w), hash_float_to_float3(w)),
        2 => (hash_float2_to_float([p[0], p[1]]), hash_float2_to_float3([p[0], p[1]])),
        3 => (hash_float3_to_float(p), hash_float3_to_float3(p)),
        _ => {
            let k = [p[0], p[1], p[2], w];
            (hash_float4_to_float(k), hash_float4_to_float3(k))
        }
    }
}

/// The Gabor texture's two kinds: a pattern laid in a plane, or through
/// space.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GaborKind {
    #[default]
    Plane,
    Space,
}

impl GaborKind {
    pub fn cycles(self) -> &'static str {
        match self {
            GaborKind::Plane => "2D",
            GaborKind::Space => "3D",
        }
    }
}

/// Impulses per cell: Cycles' stratified count (Tavernier et al. 2019).
const GABOR_IMPULSES: i32 = 8;

fn polar(r: f32, angle: f32) -> [f32; 2] {
    [r * angle.cos(), r * angle.sin()]
}

/// One windowed Gabor kernel as a phasor: its real part the classic
/// cosine Gabor, its imaginary part the sine one the noise is read from.
fn gabor_envelope(distance_squared: f32) -> f32 {
    let hann = 0.5 + 0.5 * (PI * distance_squared).cos();
    (-PI * distance_squared).exp() * hann
}

fn gabor_cell_2d(cell: [f32; 2], position: [f32; 2], frequency: f32, isotropy: f32, base: f32) -> [f32; 2] {
    let mut noise = [0.0f32; 2];
    for i in 0..GABOR_IMPULSES {
        let seed = |k: i32| [cell[0], cell[1], (i * 3 + k) as f32];
        let random_orientation = (hash_float3_to_float(seed(0)) - 0.5) * PI;
        let orientation = base + random_orientation * isotropy;
        let centre = hash_float3_to_float2(seed(1));
        let p = [position[0] - centre[0], position[1] - centre[1]];
        let d2 = p[0] * p[0] + p[1] * p[1];
        // Windowed to nothing beyond a unit radius.
        if d2 >= 1.0 {
            continue;
        }
        let weight = if hash_float3_to_float(seed(2)) < 0.5 { -1.0 } else { 1.0 };
        let f = polar(frequency, orientation);
        let k = polar(gabor_envelope(d2), 2.0 * PI * (p[0] * f[0] + p[1] * f[1]));
        noise[0] += weight * k[0];
        noise[1] += weight * k[1];
    }
    noise
}

fn gabor_2d(coordinates: [f32; 2], frequency: f32, isotropy: f32, orientation: f32) -> [f32; 2] {
    let cell = [coordinates[0].floor(), coordinates[1].floor()];
    let local = [coordinates[0] - cell[0], coordinates[1] - cell[1]];
    let mut sum = [0.0f32; 2];
    for j in -1..=1 {
        for i in -1..=1 {
            let offset = [i as f32, j as f32];
            let n = gabor_cell_2d(
                [cell[0] + offset[0], cell[1] + offset[1]],
                [local[0] - offset[0], local[1] - offset[1]],
                frequency,
                isotropy,
                orientation,
            );
            sum[0] += n[0];
            sum[1] += n[1];
        }
    }
    sum
}

/// A kernel's direction in space: the base one, turned at random by as
/// much as the noise is isotropic, in spherical coordinates.
fn gabor_orientation_3d(orientation: [f32; 3], isotropy: f32, seed: [f32; 4]) -> [f32; 3] {
    if isotropy == 0.0 {
        return orientation;
    }
    let mut inclination = orientation[2].acos();
    let mut azimuth = if orientation[1] < 0.0 { -1.0 } else { 1.0 }
        * (orientation[0] / (orientation[0] * orientation[0] + orientation[1] * orientation[1]).sqrt()).acos();
    let random = hash_float4_to_float2(seed);
    inclination += random[0] * PI * isotropy;
    azimuth += random[1] * PI * isotropy;
    let s = inclination.sin();
    [s * azimuth.cos(), s * azimuth.sin(), inclination.cos()]
}

fn gabor_cell_3d(cell: [f32; 3], position: [f32; 3], frequency: f32, isotropy: f32, base: [f32; 3]) -> [f32; 2] {
    let mut noise = [0.0f32; 2];
    for i in 0..GABOR_IMPULSES {
        let seed = |k: i32| [cell[0], cell[1], cell[2], (i * 3 + k) as f32];
        let orientation = gabor_orientation_3d(base, isotropy, seed(0));
        let centre = hash_float4_to_float3(seed(1));
        let p = [position[0] - centre[0], position[1] - centre[1], position[2] - centre[2]];
        let d2 = p[0] * p[0] + p[1] * p[1] + p[2] * p[2];
        if d2 >= 1.0 {
            continue;
        }
        let weight = if hash_float4_to_float(seed(2)) < 0.5 { -1.0 } else { 1.0 };
        let along = p[0] * orientation[0] + p[1] * orientation[1] + p[2] * orientation[2];
        let k = polar(gabor_envelope(d2), 2.0 * PI * frequency * along);
        noise[0] += weight * k[0];
        noise[1] += weight * k[1];
    }
    noise
}

fn gabor_3d(coordinates: [f32; 3], frequency: f32, isotropy: f32, orientation: [f32; 3]) -> [f32; 2] {
    let cell = coordinates.map(f32::floor);
    let local = [coordinates[0] - cell[0], coordinates[1] - cell[1], coordinates[2] - cell[2]];
    let mut sum = [0.0f32; 2];
    for k in -1..=1 {
        for j in -1..=1 {
            for i in -1..=1 {
                let offset = [i as f32, j as f32, k as f32];
                let n = gabor_cell_3d(
                    [cell[0] + offset[0], cell[1] + offset[1], cell[2] + offset[2]],
                    [local[0] - offset[0], local[1] - offset[1], local[2] - offset[2]],
                    frequency,
                    isotropy,
                    orientation,
                );
                sum[0] += n[0];
                sum[1] += n[1];
            }
        }
    }
    sum
}

/// Cycles' Gabor texture (`svm_node_tex_gabor`, Blender 4.3): sparse Gabor
/// convolution noise (Lagae et al. 2009) made fast and normalised
/// (Tavernier et al. 2019), with phasor phase and intensity (Tricard et al.
/// 2019). Streaks of a set frequency and direction — brushed metal, wood
/// figure, fibres — that Noise cannot make.
///
/// Returns (value, phase, intensity), each in 0 to 1. `orientation_2d` is
/// an angle in radians for a plane; `orientation_3d` a direction for space.
#[allow(clippy::too_many_arguments)]
pub fn gabor(
    kind: GaborKind,
    coordinates: [f32; 3],
    scale: f32,
    frequency: f32,
    anisotropy: f32,
    orientation_2d: f32,
    orientation_3d: [f32; 3],
) -> (f32, f32, f32) {
    let p = coordinates.map(|x| x * scale);
    let isotropy = 1.0 - anisotropy.clamp(0.0, 1.0);
    let frequency = frequency.max(0.001);
    let (phasor, deviation) = match kind {
        GaborKind::Plane => (
            gabor_2d([p[0], p[1]], frequency, isotropy, orientation_2d),
            (GABOR_IMPULSES as f32 * 0.5 * 0.25).sqrt(),
        ),
        GaborKind::Space => {
            let length = (orientation_3d[0] * orientation_3d[0] + orientation_3d[1] * orientation_3d[1] + orientation_3d[2] * orientation_3d[2]).sqrt();
            let orientation = orientation_3d.map(|x| x / length);
            (
                gabor_3d(p, frequency, isotropy, orientation),
                (GABOR_IMPULSES as f32 * 0.5 * (1.0 / (4.0 * std::f32::consts::SQRT_2))).sqrt(),
            )
        }
    };
    // Six standard deviations, found by Cycles' authors to span it.
    let normalization = 6.0 * deviation;
    let value = (phasor[1] / normalization) * 0.5 + 0.5;
    let phase = (phasor[1].atan2(phasor[0]) + PI) / (2.0 * PI);
    let intensity = (phasor[0] * phasor[0] + phasor[1] * phasor[1]).sqrt() / normalization;
    (value, phase, intensity)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gabor noise sits around the middle, reaches out from it, and has
    /// its streaks along the orientation: fully anisotropic in a plane, it
    /// changes fast across the orientation and slowly along it.
    #[test]
    fn gabor_streaks_run_along_its_orientation() {
        let sample = |x: f32, y: f32| gabor(GaborKind::Plane, [x, y, 0.0], 1.0, 4.0, 1.0, 0.0, [1.0, 0.0, 0.0]).0;
        let (mut sum, mut lo, mut hi) = (0.0, 1.0f32, 0.0f32);
        let (mut across, mut along) = (0.0, 0.0);
        let n = 400;
        for i in 0..n {
            let (x, y) = (i as f32 * 0.137, (i * 7 % 23) as f32 * 0.31);
            let v = sample(x, y);
            assert!(v.is_finite());
            sum += v;
            lo = lo.min(v);
            hi = hi.max(v);
            // Orientation 0 is the x axis: the waves vary along x.
            across += (sample(x + 0.02, y) - v).abs();
            along += (sample(x, y + 0.02) - v).abs();
        }
        let mean = sum / n as f32;
        assert!((mean - 0.5).abs() < 0.05, "centred: {mean}");
        assert!(lo < 0.35 && hi > 0.65, "it varies: {lo}..{hi}");
        assert!(across > along * 2.0, "streaks along the orientation: {across} across, {along} along");
        let (_, phase, intensity) = gabor(GaborKind::Space, [0.3, 1.7, 2.2], 2.0, 2.0, 0.0, 0.0, [1.0, 1.0, 0.0]);
        assert!((0.0..=1.0).contains(&phase) && intensity >= 0.0);
    }

    #[test]
    fn checker_alternates() {
        assert_ne!(checker([0.5, 0.5, 0.5]), checker([1.5, 0.5, 0.5]));
        assert_eq!(checker([0.5, 0.5, 0.5]), checker([1.5, 1.5, 0.5]));
    }

    #[test]
    fn brick_has_mortar_at_its_edges() {
        let b = BrickSettings::default();
        // The first row is offset by half a brick (0.25).
        assert_eq!(b.brick([0.0, 0.125, 0.0]).1, 0.0, "the middle of a brick is brick");
        assert!(b.brick([0.249, 0.125, 0.0]).1 > 0.5, "its edge is mortar");
    }

    #[test]
    fn gradients_stay_in_range() {
        for kind in GradientKind::ALL {
            for i in -20..20 {
                let v = gradient(kind, [i as f32 * 0.1, 0.3, -0.2]);
                assert!((0.0..=1.0).contains(&v));
            }
        }
    }
}
