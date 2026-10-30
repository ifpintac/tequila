//! Spectral rendering (`docs/01` §7.4): paths that carry wavelengths rather
//! than three colour channels.
//!
//! A spectral path carries four wavelengths [Wilkie et al. 2014]: one drawn
//! from a distribution over the visible range, and three more spaced evenly
//! from it in that distribution's measure, so the four between them cover the
//! spectrum however the first falls. Every colour the path meets — a
//! surface's reflectance, a glass's transmittance, a light's emission — is
//! read at those four as the smooth spectrum its RGB stands for, by Jakob and
//! Hanika's sigmoid-polynomial upsampling [2019]; a medium a random walk
//! crosses is made at each wavelength from its colours' spectra. The path's
//! radiance goes to the film through the CIE functions. Only at a dispersive
//! refraction, where each wavelength would leave by a direction of its own,
//! does the path keep its first wavelength alone.
//!
//! What a surface's RGB *means* spectrally is fixed by one illuminant: the
//! rendering space's white, D65 (`Spectrum` is ACEScg's AP1 primaries with
//! D65's white). An RGB reflectance upsamples to the spectrum whose colour,
//! lit by D65, is that RGB; an RGB light to D65 tinted by the spectrum of its
//! colour, so a white light is D65 and stays white. D65 is folded into the
//! film's response here rather than multiplied into every light, which leaves
//! a white light a flat spectrum of ones and so free of the colour noise a
//! spiky illuminant would add per wavelength.
//!
//! # The coefficient table
//!
//! Upsampling looks an RGB up in a table of the three coefficients of
//! `s(λ) = sigmoid(c₀ u² + c₁ u + c₂)`, `u` the wavelength across the visible
//! range: three cubes of 64³, one for each channel being the largest, indexed
//! by that channel's value (on a scale that crowds the dark and bright ends)
//! and the other two as fractions of it. Each entry is fitted by Gauss-Newton
//! iterations on the CIELAB difference between the target and the spectrum's
//! colour under D65, warm-started from its neighbour along the brightness
//! axis, as the paper and its reference implementation (`rgb2spec`) do.
//!
//! The table is built here, once, the first time a spectral render needs it,
//! rather than shipped: it is nine megabytes of floats, which compress badly,
//! and building it takes 0.6 s over sixteen cores in a debug build, less in
//! a release one (see [`table`]). Built from the same response the
//! film uses, so the fit and the render agree on what a colour is to the last
//! digit the fit reaches.

use std::sync::OnceLock;
use tequila_core::spectrum::Spectrum;

/// The range the wavelengths are drawn from, in nanometres. The CIE functions
/// hold a thousandth of their weight outside it.
pub const SHORTEST: f32 = 380.0;
pub const LONGEST: f32 = 780.0;

/// How many wavelengths a spectral path carries (Wilkie et al.'s four).
pub const WAVELENGTHS: usize = 4;

/// The table's resolution along each of its three axes.
pub const RESOLUTION: usize = 64;

/// CIE standard illuminant D65 from 380 to 780 nm in steps of 10, relative to
/// 100 at 560 nm (CIE 15:2004). Read between the steps along straight lines.
const D65: [f64; 41] = [
    49.9755, 54.6482, 82.7549, 91.486, 93.4318, 86.6823, 104.865, 117.008, 117.812, 114.861, 115.923,
    108.811, 109.354, 107.802, 104.790, 107.689, 104.405, 104.046, 100.000, 96.3342, 95.788, 88.6856,
    90.0062, 89.5991, 87.6987, 83.2886, 83.6992, 80.0268, 80.2146, 82.2778, 78.2842, 69.7213, 71.6091,
    74.349, 61.604, 69.8856, 75.087, 63.5927, 46.4182, 66.8054, 63.3828,
];

fn d65(wavelength: f64) -> f64 {
    let at = ((wavelength - SHORTEST as f64) / 10.0).clamp(0.0, (D65.len() - 1) as f64);
    let i = (at as usize).min(D65.len() - 2);
    let t = at - i as f64;
    D65[i] * (1.0 - t) + D65[i + 1] * t
}

fn cmf(wavelength: f64) -> [f64; 3] {
    tequila_materials::dispersion::cie_xyz(wavelength as f32).map(f64::from)
}

/// CIE XYZ to the rendering space: to linear Rec.709 (whose white is D65),
/// then to AP1 by the same matrix [`Spectrum::from_linear_srgb`] uses.
fn xyz_to_rendering() -> [[f64; 3]; 3] {
    let to_srgb = [
        [3.240_454_2, -1.537_138_5, -0.498_531_4],
        [-0.969_266_0, 1.876_010_8, 0.041_556_0],
        [0.055_643_4, -0.204_025_9, 1.057_225_2],
    ];
    let to_ap1 = [
        [0.613_097_4, 0.339_523_4, 0.047_379_2],
        [0.070_193_6, 0.916_353_5, 0.013_452_9],
        [0.020_615_8, 0.109_569_6, 0.869_814_6],
    ];
    multiply(&to_ap1, &to_srgb)
}

fn multiply(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, value) in row.iter_mut().enumerate() {
            *value = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

fn apply(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// `m⁻¹`, by cofactors: the matrices here are well conditioned.
fn inverse(m: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let c = |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let det = m[0][0] * c(1, 1, 2, 2) - m[0][1] * c(1, 0, 2, 2) + m[0][2] * c(1, 0, 2, 1);
    let adjugate = [
        [c(1, 1, 2, 2), -c(0, 1, 2, 2), c(0, 1, 1, 2)],
        [-c(1, 0, 2, 2), c(0, 0, 2, 2), -c(0, 0, 1, 2)],
        [c(1, 0, 2, 1), -c(0, 0, 2, 1), c(0, 0, 1, 1)],
    ];
    adjugate.map(|row| row.map(|v| v / det))
}

/// How many nanometres apart the film's response is tabulated: the CIE's own
/// spacing for its functions, and fine enough for D65's 10 nm steps to fall
/// on nodes.
const RESPONSE_STEP: f64 = 5.0;
const RESPONSE_NODES: usize = ((LONGEST - SHORTEST) as f64 / RESPONSE_STEP) as usize + 1;

/// The film's response to one wavelength: what light of unit relative
/// radiance at that wavelength adds to the rendering space's red, green and
/// blue. The CIE functions times D65, into the rendering space, scaled per
/// channel so that over the range they integrate to exactly white — so a
/// flat spectrum of ones, a white surface under a white light, is (1, 1, 1).
///
/// Tabulated [`RESPONSE_STEP`] apart and read along straight lines, and the
/// white point is taken for those straight lines, so it is exact for the
/// function the film actually uses rather than for a finer one it
/// approximates.
struct Response {
    nodes: Vec<[f32; 3]>,
    /// Back from the rendering space to XYZ (relative to D65 integrated over
    /// the same nodes), for the table's CIELAB fit.
    to_xyz: [[f64; 3]; 3],
    /// The CIE functions times D65 at each node, times the trapezoid weight
    /// of that node: the fit's quadrature, which is exactly the integral of
    /// the straight lines between the nodes for any spectrum that is itself
    /// straight between them, and close for the smooth ones the table holds
    /// (what is left is in the round trip the tests measure, which integrates
    /// finely).
    weighted_cmf: Vec<[f64; 3]>,
    /// The normalized wavelength `u` of each node.
    u: Vec<f64>,
    /// D65's XYZ over the range: CIELAB's white.
    white: [f64; 3],
}

impl Response {
    fn build() -> Response {
        let matrix = xyz_to_rendering();
        let mut weighted_cmf = Vec::with_capacity(RESPONSE_NODES);
        let mut u = Vec::with_capacity(RESPONSE_NODES);
        let mut white = [0.0f64; 3];
        for i in 0..RESPONSE_NODES {
            let wavelength = SHORTEST as f64 + i as f64 * RESPONSE_STEP;
            let weight = if i == 0 || i == RESPONSE_NODES - 1 { 0.5 } else { 1.0 } * RESPONSE_STEP;
            let illuminated = cmf(wavelength).map(|c| c * d65(wavelength) * weight);
            for c in 0..3 {
                white[c] += illuminated[c];
            }
            weighted_cmf.push(illuminated);
            u.push((wavelength - SHORTEST as f64) / (LONGEST - SHORTEST) as f64);
        }
        // Per-channel so white lands on (1, 1, 1) exactly. The correction is
        // the difference between the Wyman fit and the tabulated functions
        // the sRGB matrix was derived for, a few parts in a thousand.
        let rendered_white = apply(&matrix, white);
        let to_rendering: [[f64; 3]; 3] = [0, 1, 2].map(|i| matrix[i].map(|v| v / rendered_white[i]));
        let nodes = (0..RESPONSE_NODES)
            .map(|i| {
                let wavelength = SHORTEST as f64 + i as f64 * RESPONSE_STEP;
                let xyz = cmf(wavelength).map(|c| c * d65(wavelength));
                apply(&to_rendering, xyz).map(|v| v as f32)
            })
            .collect();
        Response { nodes, to_xyz: inverse(&to_rendering), weighted_cmf, u, white }
    }

    /// The response at `wavelength`, between the nodes.
    #[inline]
    fn at(&self, wavelength: f32) -> [f32; 3] {
        let at = ((wavelength - SHORTEST) / RESPONSE_STEP as f32).clamp(0.0, (RESPONSE_NODES - 1) as f32);
        let i = (at as usize).min(RESPONSE_NODES - 2);
        let t = at - i as f32;
        let (a, b) = (self.nodes[i], self.nodes[i + 1]);
        [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t)
    }
}

fn response() -> &'static Response {
    static RESPONSE: OnceLock<Response> = OnceLock::new();
    RESPONSE.get_or_init(Response::build)
}

/// Where in the visible range a wavelength lies, 0 at [`SHORTEST`] and 1 at
/// [`LONGEST`]: the variable the sigmoid polynomials are written in.
#[inline]
pub fn normalized(wavelength: f32) -> f32 {
    (wavelength - SHORTEST) / (LONGEST - SHORTEST)
}

// ---------------------------------------------------------------------------
// Wavelength sampling
// ---------------------------------------------------------------------------

// The distribution of Radziszewski et al. (2009), as pbrt-v4 uses it: a
// hyperbolic secant squared around 538 nm, a close fit to how much each
// wavelength matters to the eye. Truncated to the range here and normalized
// over it.
const PEAK: f32 = 538.0;
const WIDTH: f32 = 0.0072;

/// How much of the sampling goes by the film's blue response rather than by
/// the eye. The eye's distribution starves blue: AP1's blue channel takes its
/// light from the short end, where the eye barely looks, and with it alone a
/// white path's blue strayed 47% from white (RMS over the hero's position,
/// four wavelengths), against 6% for red. With an eighth by blue it strays
/// 11%, red 13% and green 5%.
const BLUE_SHARE: f64 = 0.13;

/// One bin of the tabulated distribution, in nanometres.
const SAMPLING_BIN: f64 = 1.0;
const SAMPLING_BINS: usize = ((LONGEST - SHORTEST) as f64 / SAMPLING_BIN) as usize;

fn eye_pdf(wavelength: f64) -> f64 {
    let low = (WIDTH as f64 * (SHORTEST as f64 - PEAK as f64)).tanh();
    let high = (WIDTH as f64 * (LONGEST as f64 - PEAK as f64)).tanh();
    let c = (WIDTH as f64 * (wavelength - PEAK as f64)).cosh();
    WIDTH as f64 / (c * c * (high - low))
}

/// The wavelength distribution, tabulated: each bin's density (constant
/// across it) and the cumulative distribution at its edges.
struct Sampling {
    density: Vec<f32>,
    cdf: Vec<f64>,
}

impl Sampling {
    fn build(response: &Response) -> Sampling {
        let blue = |wavelength: f64| f64::from(response.at(wavelength as f32)[2].abs());
        // Each bin's average of the mixture, by ten points across it.
        let average = |f: &dyn Fn(f64) -> f64, bin: usize| {
            (0..10).map(|j| f(SHORTEST as f64 + (bin as f64 + (j as f64 + 0.5) / 10.0) * SAMPLING_BIN)).sum::<f64>() / 10.0
        };
        let blue_total: f64 = (0..SAMPLING_BINS).map(|b| average(&blue, b) * SAMPLING_BIN).sum();
        let mixture = |wavelength: f64| (1.0 - BLUE_SHARE) * eye_pdf(wavelength) + BLUE_SHARE * blue(wavelength) / blue_total;
        let mut weights: Vec<f64> = (0..SAMPLING_BINS).map(|b| average(&mixture, b) * SAMPLING_BIN).collect();
        let total: f64 = weights.iter().sum();
        for w in weights.iter_mut() {
            *w /= total;
        }
        let mut cdf = Vec::with_capacity(SAMPLING_BINS + 1);
        cdf.push(0.0);
        for w in &weights {
            cdf.push(cdf.last().copied().unwrap_or(0.0) + w);
        }
        let density = weights.iter().map(|w| (w / SAMPLING_BIN) as f32).collect();
        Sampling { density, cdf }
    }
}

fn sampling() -> &'static Sampling {
    static SAMPLING: OnceLock<Sampling> = OnceLock::new();
    SAMPLING.get_or_init(|| Sampling::build(response()))
}

/// A wavelength from `u` in `[0, 1)`, by inverting the distribution's CDF.
pub fn sample_wavelength(u: f32) -> f32 {
    sample_with_density(u).0
}

/// The same and the density it was drawn with, from the bin it was drawn
/// from, so the two cannot disagree at a bin's edge.
fn sample_with_density(u: f32) -> (f32, f32) {
    let sampling = sampling();
    let u = f64::from(u.clamp(0.0, 1.0));
    let bin = sampling.cdf.partition_point(|&c| c <= u).saturating_sub(1).min(SAMPLING_BINS - 1);
    let width = sampling.cdf[bin + 1] - sampling.cdf[bin];
    let t = if width > 0.0 { ((u - sampling.cdf[bin]) / width).clamp(0.0, 1.0) } else { 0.5 };
    ((SHORTEST as f64 + (bin as f64 + t) * SAMPLING_BIN) as f32, sampling.density[bin])
}

/// The density [`sample_wavelength`] draws `wavelength` with, per nanometre.
pub fn wavelength_pdf(wavelength: f32) -> f32 {
    if !(SHORTEST..=LONGEST).contains(&wavelength) {
        return 0.0;
    }
    let bin = (((wavelength - SHORTEST) as f64 / SAMPLING_BIN) as usize).min(SAMPLING_BINS - 1);
    sampling().density[bin]
}

// ---------------------------------------------------------------------------
// The sigmoid polynomial and its table
// ---------------------------------------------------------------------------

/// `0.5 + x / (2 √(1 + x²))`: the sigmoid that keeps the spectra in `[0, 1]`.
#[inline]
fn sigmoid(x: f32) -> f32 {
    if x.is_infinite() {
        return if x > 0.0 { 1.0 } else { 0.0 };
    }
    0.5 + 0.5 * x / (1.0 + x * x).sqrt()
}

/// The smooth spectrum of one RGB: `sigmoid(c₀ u² + c₁ u + c₂)`, with `u`
/// the [`normalized`] wavelength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sigmoid {
    pub coefficients: [f32; 3],
}

impl Sigmoid {
    /// The flat spectrum of `value`, which a grey is exactly.
    pub fn constant(value: f32) -> Sigmoid {
        let c2 = if value <= 0.0 {
            f32::NEG_INFINITY
        } else if value >= 1.0 {
            f32::INFINITY
        } else {
            (value - 0.5) / (value * (1.0 - value)).sqrt()
        };
        Sigmoid { coefficients: [0.0, 0.0, c2] }
    }

    /// The value at a [`normalized`] wavelength `u`.
    #[inline]
    pub fn at(&self, u: f32) -> f32 {
        let [c0, c1, c2] = self.coefficients;
        if c2.is_infinite() {
            return sigmoid(c2);
        }
        sigmoid((c0 * u + c1) * u + c2)
    }
}

/// Jakob and Hanika's table: for every RGB in the unit cube of the rendering
/// space, the sigmoid polynomial whose colour under D65 is that RGB, as
/// nearly as one exists.
pub struct Table {
    /// The brightness each slice along the third axis stands for.
    scale: [f32; RESOLUTION],
    /// `3 × RESOLUTION³` coefficient triples: which channel is largest, then
    /// brightness, then the next channel and the one after it as fractions
    /// of the largest.
    coefficients: Vec<[f32; 3]>,
}

impl std::fmt::Debug for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table").field("resolution", &RESOLUTION).finish_non_exhaustive()
    }
}

/// The process-wide table, built the first time it is asked for.
///
/// Built from plain threads, not Rayon, for the reason the GGX energy table
/// is (`tequila_materials::energy`): the first call can come from inside a
/// render's worker, and a Rayon build there would wait on work only its own
/// blocked pool could do.
pub fn table() -> &'static Table {
    static TABLE: OnceLock<Table> = OnceLock::new();
    TABLE.get_or_init(Table::build)
}

/// Build the table now if it has not been built: before a render's workers
/// start, so none of them stalls on it.
pub fn prewarm() {
    let _ = table();
}

/// `smoothstep(smoothstep(t))`: the brightness axis, crowded at both ends,
/// where colours change fastest with brightness.
fn brightness(t: f64) -> f64 {
    let s = |x: f64| x * x * (3.0 - 2.0 * x);
    s(s(t))
}

impl Table {
    /// Fit every entry. Independent along the two chromatic axes and warm
    /// started along brightness, so each line of 64 brightnesses is one task.
    pub fn build() -> Table {
        let response = response();
        let scale: [f32; RESOLUTION] =
            std::array::from_fn(|k| brightness(k as f64 / (RESOLUTION - 1) as f64) as f32);
        let lines = 3 * RESOLUTION * RESOLUTION;
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(lines);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let mut coefficients = vec![[0.0f32; 3]; 3 * RESOLUTION * RESOLUTION * RESOLUTION];
        let finished: Vec<Vec<(usize, [[f32; 3]; RESOLUTION])>> = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..threads)
                .map(|_| {
                    scope.spawn(|| {
                        let mut done = Vec::new();
                        loop {
                            let line = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            if line >= lines {
                                break;
                            }
                            done.push((line, fit_line(response, &scale, line)));
                        }
                        done
                    })
                })
                .collect();
            workers.into_iter().map(|w| w.join().expect("a table line fits")).collect()
        });
        for (line, fitted) in finished.into_iter().flatten() {
            let (l, j, i) = (line / (RESOLUTION * RESOLUTION), (line / RESOLUTION) % RESOLUTION, line % RESOLUTION);
            for (k, c) in fitted.iter().enumerate() {
                coefficients[((l * RESOLUTION + k) * RESOLUTION + j) * RESOLUTION + i] = *c;
            }
        }
        Table { scale, coefficients }
    }

    /// The sigmoid polynomial of `rgb`, each channel in `[0, 1]`.
    pub fn sigmoid(&self, rgb: [f32; 3]) -> Sigmoid {
        let rgb = rgb.map(|c| c.clamp(0.0, 1.0));
        if rgb[0] == rgb[1] && rgb[1] == rgb[2] {
            return Sigmoid::constant(rgb[0]);
        }
        let largest = if rgb[0] > rgb[1] {
            if rgb[0] > rgb[2] {
                0
            } else {
                2
            }
        } else if rgb[1] > rgb[2] {
            1
        } else {
            2
        };
        let z = rgb[largest];
        let steps = (RESOLUTION - 1) as f32;
        let x = rgb[(largest + 1) % 3] * steps / z;
        let y = rgb[(largest + 2) % 3] * steps / z;
        let xi = (x as usize).min(RESOLUTION - 2);
        let yi = (y as usize).min(RESOLUTION - 2);
        // The last slice whose brightness is at most `z`.
        let zi = self.scale[1..RESOLUTION - 1].partition_point(|&s| s <= z);
        let (dx, dy) = (x - xi as f32, y - yi as f32);
        let dz = ((z - self.scale[zi]) / (self.scale[zi + 1] - self.scale[zi])).clamp(0.0, 1.0);
        let at = |k: usize, j: usize, i: usize| self.coefficients[((largest * RESOLUTION + k) * RESOLUTION + j) * RESOLUTION + i];
        let mut c = [0.0f32; 3];
        for (k, wz) in [(zi, 1.0 - dz), (zi + 1, dz)] {
            for (j, wy) in [(yi, 1.0 - dy), (yi + 1, dy)] {
                for (i, wx) in [(xi, 1.0 - dx), (xi + 1, dx)] {
                    let w = wz * wy * wx;
                    let v = at(k, j, i);
                    for n in 0..3 {
                        c[n] += w * v[n];
                    }
                }
            }
        }
        Sigmoid { coefficients: c }
    }
}

/// One line of the table: the 64 brightnesses of one chromaticity, fitted
/// up from a fifth of the way along and then down from there, each from the
/// last, as `rgb2spec` does — a spectrum changes little from one brightness
/// to the next, and a fit started from a neighbour's answer converges where
/// one started from nothing may not.
fn fit_line(response: &Response, scale: &[f32; RESOLUTION], line: usize) -> [[f32; 3]; RESOLUTION] {
    let l = line / (RESOLUTION * RESOLUTION);
    let j = (line / RESOLUTION) % RESOLUTION;
    let i = line % RESOLUTION;
    let x = i as f64 / (RESOLUTION - 1) as f64;
    let y = j as f64 / (RESOLUTION - 1) as f64;
    let mut out = [[0.0f32; 3]; RESOLUTION];
    let start = RESOLUTION / 5;
    let mut fit_at = |k: usize, c: &mut [f64; 3]| {
        let b = scale[k] as f64;
        let mut rgb = [0.0f64; 3];
        rgb[l] = b;
        rgb[(l + 1) % 3] = x * b;
        rgb[(l + 2) % 3] = y * b;
        gauss_newton(response, rgb, c);
        out[k] = c.map(|v| v as f32);
    };
    let mut c = [0.0f64; 3];
    for k in start..RESOLUTION {
        fit_at(k, &mut c);
    }
    let mut c = [0.0f64; 3];
    for k in (0..=start).rev() {
        fit_at(k, &mut c);
    }
    out
}

/// CIELAB of an XYZ against `white`, and its derivative by X, Y and Z.
fn lab(xyz: [f64; 3], white: [f64; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    const DELTA: f64 = 6.0 / 29.0;
    let f = |t: f64| {
        if t > DELTA * DELTA * DELTA {
            (t.cbrt(), 1.0 / (3.0 * t.cbrt() * t.cbrt()))
        } else {
            (t / (3.0 * DELTA * DELTA) + 4.0 / 29.0, 1.0 / (3.0 * DELTA * DELTA))
        }
    };
    let (fx, dx) = f(xyz[0] / white[0]);
    let (fy, dy) = f(xyz[1] / white[1]);
    let (fz, dz) = f(xyz[2] / white[2]);
    let (dx, dy, dz) = (dx / white[0], dy / white[1], dz / white[2]);
    (
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)],
        [[0.0, 116.0 * dy, 0.0], [500.0 * dx, -500.0 * dy, 0.0], [0.0, 200.0 * dy, -200.0 * dz]],
    )
}

/// The XYZ under D65 of the sigmoid polynomial `c`, and its derivative by
/// each coefficient.
fn spectrum_xyz(response: &Response, c: &[f64; 3]) -> ([f64; 3], [[f64; 3]; 3]) {
    let mut xyz = [0.0f64; 3];
    let mut jacobian = [[0.0f64; 3]; 3];
    for (q, &u) in response.weighted_cmf.iter().zip(&response.u) {
        let x = (c[0] * u + c[1]) * u + c[2];
        let root = (1.0 + x * x).sqrt();
        let s = 0.5 + 0.5 * x / root;
        let ds = 0.5 / (root * root * root);
        let by = [ds * u * u, ds * u, ds];
        for a in 0..3 {
            xyz[a] += q[a] * s;
            for n in 0..3 {
                jacobian[a][n] += q[a] * by[n];
            }
        }
    }
    (xyz, jacobian)
}

/// Solve `m x = b` by Gaussian elimination with partial pivoting; `None` if
/// `m` is singular.
fn solve(mut m: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let pivot = (col..3).max_by(|&p, &q| m[p][col].abs().total_cmp(&m[q][col].abs()))?;
        if m[pivot][col].abs() < 1e-15 {
            return None;
        }
        m.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..3 {
            let f = m[row][col] / m[col][col];
            for k in col..3 {
                m[row][k] -= f * m[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0f64; 3];
    for row in (0..3).rev() {
        let sum: f64 = (row + 1..3).map(|k| m[row][k] * x[k]).sum();
        x[row] = (b[row] - sum) / m[row][row];
    }
    Some(x)
}

/// Gauss-Newton on the CIELAB difference between the spectrum `c` stands for
/// and `rgb`, from `c` as it is. Up to fifteen steps, as `rgb2spec` takes, and
/// the coefficients held to a largest of 200, as it holds them: past that the
/// sigmoid is a step function and the fit is chasing a colour no smooth
/// spectrum has.
///
/// One change from `rgb2spec`: a step that makes the difference worse is
/// halved until it does not. Much of AP1 lies outside the spectral locus — its
/// green primary is not a colour, and its red-green edge runs just beyond the
/// yellows — and for a colour there the full step overshoots, the next
/// overshoots further, and the fit ends tens of thousands of times further
/// from it than where it started: a line of saturated oranges fitted as
/// blues. Halved, it settles on the nearest spectrum it can reach.
fn gauss_newton(response: &Response, rgb: [f64; 3], c: &mut [f64; 3]) {
    let (target, _) = lab(apply(&response.to_xyz, rgb), response.white);
    let evaluate = |c: &[f64; 3]| {
        let (xyz, by_coefficient) = spectrum_xyz(response, c);
        let (here, by_xyz) = lab(xyz, response.white);
        let residual = [0, 1, 2].map(|a| here[a] - target[a]);
        let cost: f64 = residual.iter().map(|r| r * r).sum();
        (residual, multiply(&by_xyz, &by_coefficient), cost)
    };
    let (mut residual, mut jacobian, mut cost) = evaluate(c);
    for _ in 0..15 {
        if cost < 1e-6 {
            break;
        }
        let Some(step) = solve(jacobian, residual) else {
            break;
        };
        let mut length = 1.0f64;
        let mut taken = None;
        for _ in 0..12 {
            let mut next = [0, 1, 2].map(|n| c[n] - length * step[n]);
            let largest = next[0].max(next[1]).max(next[2]);
            if largest > 200.0 {
                for v in next.iter_mut() {
                    *v *= 200.0 / largest;
                }
            }
            let evaluated = evaluate(&next);
            if evaluated.2 < cost {
                taken = Some((next, evaluated));
                break;
            }
            length *= 0.5;
        }
        let Some((next, (r, j, k))) = taken else {
            break;
        };
        *c = next;
        (residual, jacobian, cost) = (r, j, k);
    }
}

// ---------------------------------------------------------------------------
// Upsampling
// ---------------------------------------------------------------------------

/// How bright an unbounded colour is made before it is looked up: a light's
/// RGB, or a weight above one. Half, as pbrt-v4's unbounded spectra are: low
/// enough that a saturated colour has a smooth spectrum below the sigmoid's
/// ceiling, high enough to stay off the dark end, where the table's
/// coefficients swing widest.
const UNBOUNDED_BRIGHTNESS: f32 = 0.5;

/// The spectrum of a reflectance or transmittance: the table's at that RGB,
/// which keeps a dark colour's spectrum smooth and a bright one's within what
/// a surface can reflect. Above one (a weight, not a reflectance) as an
/// unbounded colour.
pub fn reflectance(rgb: Spectrum) -> (Sigmoid, f32) {
    if rgb.max_component() > 1.0 {
        return unbounded(rgb);
    }
    (table().sigmoid(rgb.to_array()), 1.0)
}

/// The spectrum of an unbounded colour, as a sigmoid polynomial and the
/// factor it is scaled by: the colour brought to [`UNBOUNDED_BRIGHTNESS`] at
/// its largest channel and looked up there.
pub fn unbounded(rgb: Spectrum) -> (Sigmoid, f32) {
    let largest = rgb.max_component();
    if largest <= 0.0 {
        return (Sigmoid::constant(0.0), 0.0);
    }
    let scale = largest / UNBOUNDED_BRIGHTNESS;
    let rgb = rgb.to_array().map(|c| c / scale);
    (table().sigmoid(rgb), scale)
}

/// The colour of a spectrum under D65, in the rendering space: what the film
/// records of it, on average over the wavelengths a render draws. Integrated
/// finely, a tenth of a nanometre at a time, so it measures the render rather
/// than the table's own quadrature.
pub fn colour_of(spectrum: impl Fn(f32) -> f32) -> Spectrum {
    let response = response();
    let step = 0.1f64;
    let n = ((LONGEST - SHORTEST) as f64 / step) as usize;
    let mut sum = [0.0f64; 3];
    for i in 0..n {
        let wavelength = SHORTEST + ((i as f64 + 0.5) * step) as f32;
        let r = response.at(wavelength);
        let s = f64::from(spectrum(wavelength)) * step;
        for c in 0..3 {
            sum[c] += f64::from(r[c]) * s;
        }
    }
    Spectrum { r: sum[0] as f32, g: sum[1] as f32, b: sum[2] as f32 }
}

/// CIELAB of a colour in the rendering space, against D65: how the table
/// measures its own error, for its tests.
pub fn lab_of(rgb: Spectrum) -> [f32; 3] {
    let response = response();
    let xyz = apply(&response.to_xyz, rgb.to_array().map(f64::from));
    lab(xyz, response.white).0.map(|v| v as f32)
}

// ---------------------------------------------------------------------------
// What a spectral path carries
// ---------------------------------------------------------------------------

/// One value per wavelength a path carries.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sampled(pub [f32; WAVELENGTHS]);

impl Sampled {
    pub const ZERO: Sampled = Sampled([0.0; WAVELENGTHS]);
    pub const ONE: Sampled = Sampled([1.0; WAVELENGTHS]);

    #[inline]
    pub fn splat(v: f32) -> Sampled {
        Sampled([v; WAVELENGTHS])
    }
}

impl std::ops::Add for Sampled {
    type Output = Sampled;
    #[inline]
    fn add(self, o: Sampled) -> Sampled {
        Sampled(std::array::from_fn(|i| self.0[i] + o.0[i]))
    }
}

impl std::ops::AddAssign for Sampled {
    #[inline]
    fn add_assign(&mut self, o: Sampled) {
        *self = *self + o;
    }
}

impl std::ops::Mul for Sampled {
    type Output = Sampled;
    #[inline]
    fn mul(self, o: Sampled) -> Sampled {
        Sampled(std::array::from_fn(|i| self.0[i] * o.0[i]))
    }
}

impl std::ops::MulAssign for Sampled {
    #[inline]
    fn mul_assign(&mut self, o: Sampled) {
        *self = *self * o;
    }
}

impl std::ops::Mul<f32> for Sampled {
    type Output = Sampled;
    #[inline]
    fn mul(self, s: f32) -> Sampled {
        Sampled(self.0.map(|v| v * s))
    }
}

impl std::ops::MulAssign<f32> for Sampled {
    #[inline]
    fn mul_assign(&mut self, s: f32) {
        *self = *self * s;
    }
}

impl std::ops::DivAssign<f32> for Sampled {
    #[inline]
    fn div_assign(&mut self, s: f32) {
        let inv = 1.0 / s;
        *self = *self * inv;
    }
}

/// The wavelengths of one path and how the film weighs each.
#[derive(Clone, Debug)]
pub struct Wavelengths {
    /// In nanometres; the first is the hero, which a dispersive refraction
    /// keeps.
    pub lambda: [f32; WAVELENGTHS],
    /// The same, [`normalized`], for the sigmoid polynomials.
    u: [f32; WAVELENGTHS],
    /// What one unit of radiance at each wavelength adds to the film: its
    /// response over the density it was drawn with, shared among the four.
    film: [Spectrum; WAVELENGTHS],
    table: &'static Table,
}

impl Wavelengths {
    /// Four wavelengths from `u`: the hero at `u` in the sampling
    /// distribution's measure, the others a quarter, a half and three
    /// quarters further round, so the four are stratified however `u` falls.
    pub fn new(u: f32) -> Wavelengths {
        let response = response();
        let mut lambda = [0.0f32; WAVELENGTHS];
        let mut film = [Spectrum::ZERO; WAVELENGTHS];
        for i in 0..WAVELENGTHS {
            let ui = (u + i as f32 / WAVELENGTHS as f32).fract();
            let (wavelength, density) = sample_with_density(ui);
            lambda[i] = wavelength;
            let [r, g, b] = response.at(wavelength);
            let weight = 1.0 / (WAVELENGTHS as f32 * density.max(1e-12));
            film[i] = Spectrum { r: r * weight, g: g * weight, b: b * weight };
        }
        Wavelengths { lambda, u: lambda.map(normalized), film, table: table() }
    }

    /// A spectrum at the four wavelengths.
    #[inline]
    pub fn at(&self, sigmoid: &Sigmoid, scale: f32) -> Sampled {
        Sampled(std::array::from_fn(|i| sigmoid.at(self.u[i]) * scale))
    }

    /// A reflectance or transmittance's spectrum, as [`reflectance`] makes it.
    #[inline]
    pub fn reflectance(&self, rgb: Spectrum) -> Sampled {
        if rgb.r == rgb.g && rgb.g == rgb.b {
            return Sampled::splat(rgb.r);
        }
        if rgb.max_component() > 1.0 {
            return self.unbounded(rgb);
        }
        self.at(&self.table.sigmoid(rgb.to_array()), 1.0)
    }

    /// An unbounded colour's spectrum, as [`unbounded`] makes it.
    #[inline]
    pub fn unbounded(&self, rgb: Spectrum) -> Sampled {
        if rgb.r == rgb.g && rgb.g == rgb.b {
            return Sampled::splat(rgb.r);
        }
        let largest = rgb.max_component();
        if largest <= 0.0 {
            return Sampled::ZERO;
        }
        let scale = largest / UNBOUNDED_BRIGHTNESS;
        let inverse = 1.0 / scale;
        self.at(&self.table.sigmoid(rgb.to_array().map(|c| c * inverse)), scale)
    }

    /// The colour the four values add to the film.
    #[inline]
    pub fn to_rgb(&self, v: Sampled) -> Spectrum {
        let mut out = Spectrum::ZERO;
        for i in 0..WAVELENGTHS {
            out += self.film[i] * v.0[i];
        }
        out
    }

    /// How luminous the film finds what this path carries, per unit of
    /// light: the luminance a flat spectrum of ones would give it through
    /// these wavelengths — near 1 for all four, and the hero's own once it
    /// carries on alone, when a green hero counts three times a red one.
    pub fn luminance(&self, alone: bool) -> f32 {
        if alone {
            (self.film[0] * WAVELENGTHS as f32).luminance()
        } else {
            self.to_rgb(Sampled::ONE).luminance()
        }
    }
}

// ---------------------------------------------------------------------------
// One integrator for both
// ---------------------------------------------------------------------------

/// What a path's throughput and radiance are made of: three colour channels
/// or four wavelengths. The integrator is written once against this, and the
/// RGB version of it does exactly the arithmetic it did before there was a
/// spectral one, operation for operation, so an RGB render is the same to
/// the bit.
pub(crate) trait Carried:
    Copy
    + std::ops::Add<Output = Self>
    + std::ops::AddAssign
    + std::ops::Mul<Output = Self>
    + std::ops::MulAssign
    + std::ops::Mul<f32, Output = Self>
    + std::ops::MulAssign<f32>
    + std::ops::DivAssign<f32>
{
    const ZERO: Self;
    const ONE: Self;
    /// How many values: the random walk samples its distances in one,
    /// chosen by throughput, and weighs them over all.
    const CHANNELS: usize;
    fn channel(&self, i: usize) -> f32;
    fn from_channels(f: impl FnMut(usize) -> f32) -> Self;
    fn max_component(&self) -> f32;
    fn is_black(&self) -> bool;
    fn is_finite(&self) -> bool;
}

impl Carried for Spectrum {
    const ZERO: Self = Spectrum::ZERO;
    const ONE: Self = Spectrum::ONE;
    const CHANNELS: usize = 3;
    #[inline(always)]
    fn channel(&self, i: usize) -> f32 {
        match i {
            0 => self.r,
            1 => self.g,
            _ => self.b,
        }
    }
    #[inline(always)]
    fn from_channels(mut f: impl FnMut(usize) -> f32) -> Self {
        let r = f(0);
        let g = f(1);
        let b = f(2);
        Spectrum { r, g, b }
    }
    #[inline(always)]
    fn max_component(&self) -> f32 {
        Spectrum::max_component(*self)
    }
    #[inline(always)]
    fn is_black(&self) -> bool {
        Spectrum::is_black(*self)
    }
    #[inline(always)]
    fn is_finite(&self) -> bool {
        Spectrum::is_finite(*self)
    }
}

impl Carried for Sampled {
    const ZERO: Self = Sampled::ZERO;
    const ONE: Self = Sampled::ONE;
    const CHANNELS: usize = WAVELENGTHS;
    #[inline]
    fn channel(&self, i: usize) -> f32 {
        self.0[i]
    }
    #[inline]
    fn from_channels(f: impl FnMut(usize) -> f32) -> Self {
        Sampled(std::array::from_fn(f))
    }
    #[inline]
    fn max_component(&self) -> f32 {
        self.0.iter().fold(self.0[0], |a, &b| a.max(b))
    }
    #[inline]
    fn is_black(&self) -> bool {
        self.0.iter().all(|&v| v == 0.0)
    }
    #[inline]
    fn is_finite(&self) -> bool {
        self.0.iter().all(|v| v.is_finite())
    }
}

/// How a path reads the RGB the scene is described in.
pub(crate) trait PathColour {
    type Value: Carried;
    /// Whether the path carries wavelengths, so what is made per channel
    /// from a colour has to be made per wavelength from its spectrum.
    const SPECTRAL: bool;
    /// A reflectance, transmittance or weight that filters the path.
    fn reflectance(&self, rgb: Spectrum) -> Self::Value;
    /// Light, or anything else with no ceiling: a radiance, an emission, an
    /// extinction coefficient.
    fn unbounded(&self, rgb: Spectrum) -> Self::Value;
    /// What the film records of a value.
    fn rgb(&self, value: Self::Value) -> Spectrum;
    /// The wavelength a dispersive surface refracts this path by, where the
    /// path has its wavelengths from the start.
    fn hero(&self) -> Option<f32>;
    /// The hero wavelength alone, weighted for the three it leaves behind.
    fn hero_alone(&self, value: Self::Value) -> Self::Value;
    /// How luminous the film finds this path's light, for firefly rejection.
    fn hue(&self, alone: bool) -> Option<f32>;
    /// The path's radiance as it goes to the film, made safe to add up.
    fn finish(&self, rgb: Spectrum) -> Spectrum;
}

/// An RGB path: every reading is the RGB itself.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rgb;

impl PathColour for Rgb {
    type Value = Spectrum;
    const SPECTRAL: bool = false;
    #[inline(always)]
    fn reflectance(&self, rgb: Spectrum) -> Spectrum {
        rgb
    }
    #[inline(always)]
    fn unbounded(&self, rgb: Spectrum) -> Spectrum {
        rgb
    }
    #[inline(always)]
    fn rgb(&self, value: Spectrum) -> Spectrum {
        value
    }
    #[inline(always)]
    fn hero(&self) -> Option<f32> {
        None
    }
    #[inline(always)]
    fn hero_alone(&self, value: Spectrum) -> Spectrum {
        value
    }
    #[inline(always)]
    fn hue(&self, _alone: bool) -> Option<f32> {
        None
    }
    #[inline(always)]
    fn finish(&self, rgb: Spectrum) -> Spectrum {
        rgb.sanitized()
    }
}

impl PathColour for Wavelengths {
    type Value = Sampled;
    const SPECTRAL: bool = true;
    #[inline]
    fn reflectance(&self, rgb: Spectrum) -> Sampled {
        Wavelengths::reflectance(self, rgb)
    }
    #[inline]
    fn unbounded(&self, rgb: Spectrum) -> Sampled {
        Wavelengths::unbounded(self, rgb)
    }
    #[inline]
    fn rgb(&self, value: Sampled) -> Spectrum {
        self.to_rgb(value)
    }
    #[inline]
    fn hero(&self) -> Option<f32> {
        Some(self.lambda[0])
    }
    #[inline]
    fn hero_alone(&self, value: Sampled) -> Sampled {
        let mut alone = Sampled::ZERO;
        alone.0[0] = value.0[0] * WAVELENGTHS as f32;
        alone
    }
    #[inline]
    fn hue(&self, alone: bool) -> Option<f32> {
        Some(self.luminance(alone))
    }
    /// Non-finite values are dropped, as an RGB path's are, but a negative
    /// channel is kept. AP1's responses dip below zero over parts of the
    /// spectrum (its green is negative in the deep blue), so one path's
    /// estimate of a saturated colour can be too, and only the average over
    /// paths is the colour: clipped one path at a time, a blue light came
    /// out with three times the green it has and 30% too bright.
    #[inline]
    fn finish(&self, rgb: Spectrum) -> Spectrum {
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        Spectrum { r: finite(rgb.r), g: finite(rgb.g), b: finite(rgb.b) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sampling density integrates to one over the range and is what
    /// the sampler actually draws with.
    #[test]
    fn the_wavelength_density_matches_its_sampler() {
        let n = 4000;
        let mut integral = 0.0f64;
        for i in 0..n {
            let wavelength = SHORTEST + (LONGEST - SHORTEST) * (i as f32 + 0.5) / n as f32;
            integral += f64::from(wavelength_pdf(wavelength)) * f64::from(LONGEST - SHORTEST) / n as f64;
        }
        assert!((integral - 1.0).abs() < 1e-3, "the density integrates to {integral}");
        // Equal steps in `u` are equal steps in probability: the CDF between
        // two samples is their difference in `u`.
        let (a, b) = (sample_wavelength(0.3), sample_wavelength(0.6));
        let m = 2000;
        let mut between = 0.0f64;
        for i in 0..m {
            let wavelength = a + (b - a) * (i as f32 + 0.5) / m as f32;
            between += f64::from(wavelength_pdf(wavelength)) * f64::from(b - a) / m as f64;
        }
        assert!((between - 0.3).abs() < 1e-3, "between u = 0.3 and 0.6 lies {between}");
    }

    /// A flat spectrum of ones is white, exactly: the film's white point.
    #[test]
    fn a_flat_spectrum_is_white() {
        let white = colour_of(|_| 1.0);
        for c in white.to_array() {
            assert!((c - 1.0).abs() < 1e-5, "a flat spectrum reads {white:?}");
        }
    }

    /// A white path is white on average over where its hero falls, and
    /// never far from it on any one path: the sampling's share for blue is
    /// what keeps blue from straying furthest.
    #[test]
    fn a_white_path_is_nearly_white_wherever_its_hero_falls() {
        let n = 4096;
        let (mut mean, mut squares) = ([0.0f64; 3], [0.0f64; 3]);
        for k in 0..n {
            let white = Wavelengths::new((k as f32 + 0.5) / n as f32).to_rgb(Sampled::ONE).to_array();
            for c in 0..3 {
                mean[c] += f64::from(white[c]) / n as f64;
                squares[c] += f64::from(white[c] - 1.0).powi(2) / n as f64;
            }
        }
        let spread = squares.map(f64::sqrt);
        for c in 0..3 {
            assert!((mean[c] - 1.0).abs() < 1e-3, "a white path averages {mean:?}");
            assert!(spread[c] < 0.15, "a white path strays {spread:?} (RMS) from white");
        }
    }

    /// Greys are flat spectra, and flat spectra are greys.
    #[test]
    fn a_grey_is_flat() {
        for v in [0.0f32, 0.18, 0.5, 0.9, 1.0] {
            let s = Sigmoid::constant(v);
            for u in [0.0f32, 0.3, 0.7, 1.0] {
                assert!((s.at(u) - v).abs() < 1e-6, "grey {v} is {} at {u}", s.at(u));
            }
        }
    }

    /// The hero alone carries the light of all four.
    #[test]
    fn the_hero_alone_stands_for_four() {
        let wavelengths = Wavelengths::new(0.37);
        let alone = PathColour::hero_alone(&wavelengths, Sampled::splat(0.5));
        assert_eq!(alone.0, [2.0, 0.0, 0.0, 0.0]);
    }
}

