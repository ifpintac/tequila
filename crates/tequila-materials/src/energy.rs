//! Energy bookkeeping for GGX microfacet lobes.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.3 makes this **mandatory**: single-scatter
//! GGX loses energy at high roughness because microfacets that shadow one
//! another never get their second bounce, and "without it, rough gold looks
//! brown". Two related things are needed:
//!
//! 1. **Directional albedo** — how much energy a lobe actually reflects. The
//!    layered materials need it to attenuate what is underneath (§2.1), and
//!    the compensation term needs it to know how much is missing.
//! 2. **A compensation lobe** [Kulla & Conty 2017] that puts the missing
//!    energy back where it belongs.
//!
//! # Why the macro-surface Fresnel will not do
//!
//! Directional albedo depends on the *microfacet-relative* Fresnel, not the
//! macro-surface one. At oblique incidence on a rough surface the two diverge
//! sharply: measured reflectance at `cos_theta = 0.3, roughness = 0.5` is
//! `0.11`, while `F(0.3)` alone predicts `0.18`. Using the macro angle there
//! over-attenuates a coated substrate by more than half. So the Fresnel term
//! has to be integrated against the distribution of visible microfacets, and
//! that integral is what gets tabulated here.
//!
//! # Two tables, for two different accuracy needs
//!
//! **Conductors** use a *split-sum* table. Under Schlick's parameterization
//! the Fresnel term is affine in `F0`, so the integral factors into two 2D
//! tables that recombine for any `F0`:
//!
//! ```text
//! E(mu, alpha, F0) = F0 * scale(mu, alpha) + bias(mu, alpha)
//! ```
//!
//! This is the decomposition real-time renderers use for image-based
//! lighting. Schlick's error is proportional to `1 - F0`, so for metals —
//! where `F0` runs 0.5 to 1.0 — it stays under a percent.
//!
//! **Dielectrics** cannot use it. Their `F0` is 0.02 to 0.17, where Schlick's
//! error is at its largest: at grazing incidence on a near-smooth interface it
//! overstates reflectance by 0.06, which is a 3% energy leak in the layered
//! furnace test and well outside the 0.005 tolerance `docs/05` §3.1 sets. So
//! dielectrics get a third table axis over IOR and are integrated against the
//! **exact** Fresnel equations. The axis is parameterized by
//! `r = (ior - 1) / (ior + 1)` rather than by IOR directly, because `r` is
//! close to linear in the resulting reflectance and so interpolates well.
//!
//! Both tables are built from the same VNDF sample stream, so the extra axis
//! costs Fresnel evaluations rather than sampling work.
//!
//! # What the compensation term does and does not restore
//!
//! For a **conductor** the shadowed energy strikes another microfacet and is
//! mostly reflected, so nearly all of it comes back. The compensation is
//! large, and without it rough metal is visibly dark.
//!
//! For a **dielectric** that energy mostly *transmits* on its second
//! encounter, because `F` is small. The compensation term is correspondingly
//! tiny, and a rough dielectric genuinely reflects less than a smooth one.
//! That is physics, not a defect — and the energy is not lost from the
//! *material*, it reaches the substrate underneath, which is why
//! [`crate::lobes::SpecularOverDiffuse`] hands it there.

use crate::fresnel;
use crate::lobes::DielectricTransmission;
use crate::microfacet::Ggx;
use tequila_core::math::{sqr, vec3, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;

const MU_STEPS: usize = 32;
const ROUGHNESS_STEPS: usize = 32;
const IOR_STEPS: usize = 16;
const SAMPLES: u32 = 2048;

/// Samples per cell for the refracting-interface rows.
///
/// A quarter of [`SAMPLES`], because these rows cost far more per sample and
/// need fewer of them.
///
/// They cost more because every IOR on the axis needs its own refraction and
/// its own masking term, where the reflection rows share one masking term
/// across the whole axis. They need fewer because the reflect-or-refract
/// split is integrated rather than sampled, which leaves the microfacet draw
/// as the only source of variance.
///
/// It was an eighth, measured against the IORs the tests then swept (1.1,
/// 1.5, 2.4), where it was as good as more. It was not as good at water's
/// 1.33 from inside, where total internal reflection puts a hard edge in
/// the integrand and a rough interface passes on under half its light, so
/// the lobe's `1 / passed` doubles whatever error the cell has: at a
/// million samples a fully rough liquid seen from inside passed on 1.0114
/// of its light at 85 degrees and 0.9837 at 89. That went unseen while the
/// Liquid preset's colour was wrongly also a surface tint, which hid the
/// excess. Measured on this sweep, extended to 1.33, with release builds:
/// worst error 0.0134 at an eighth (70 ms), 0.0054 at a quarter (100 ms),
/// 0.0022 at a half (138 ms) and 0.0012 at the full count (205 ms). A
/// quarter is the knee. `docs/01` §9.2 allows 100 ms for first pixels, and
/// [`prewarm`] keeps this off that path entirely.
///
/// Taking a *prefix* of the sample stream rather than every eighth sample is
/// sound because Sobol prefixes are stratified in their own right at powers
/// of two, which this is.
const INTERFACE_SAMPLES: u32 = SAMPLES / 4;

/// Outside and inside, the two sides of a refracting interface.
const SIDES: usize = 2;
/// Index of the inside row, named because a bare `1` reads as nothing.
const INSIDE: usize = 1;

/// The smallest `cos_theta` the table is built at. Exactly zero is a
/// degenerate viewing direction with no well-defined reflection.
const MIN_MU: f32 = 1.0e-3;

/// Largest `(ior - 1) / (ior + 1)` tabulated, corresponding to `ior = 3.0` —
/// the top of the `specular_ior` range in `docs/02` §2.2.
const MAX_IOR_PARAM: f32 = 0.5;

/// Directional-albedo tables for GGX reflection.
///
/// Grid points are **endpoint-inclusive** (`i / (N - 1)`), so roughness 0 and
/// roughness 1 are tabulated values rather than clamped extrapolations. A
/// cell-centred grid biases the extremes by ~3%, enough on its own to fail the
/// furnace test at roughness 1.
#[derive(Debug)]
pub struct GgxEnergyTable {
    /// Coefficient of `F0` in the Schlick split, indexed `[roughness][mu]`.
    scale: Vec<f32>,
    /// Constant term of the Schlick split, indexed `[roughness][mu]`.
    bias: Vec<f32>,
    /// Cosine-weighted hemispherical average of `scale`, per roughness.
    average_scale: Vec<f32>,
    /// Cosine-weighted hemispherical average of `bias`, per roughness.
    average_bias: Vec<f32>,
    /// Cosine-weighted hemispherical average of the white (`F = 1`) albedo.
    white_average: Vec<f32>,
    /// Exact-Fresnel dielectric albedo, indexed `[roughness][ior][mu]`.
    dielectric: Vec<f32>,
    /// Its cosine-weighted hemispherical average, indexed `[roughness][ior]`.
    dielectric_average: Vec<f32>,
    /// Fraction of arriving flux a *refracting* interface passes on at all,
    /// by either lobe, indexed `[roughness][side][ior][mu]`.
    ///
    /// Side 0 is a ray arriving from outside, side 1 one arriving from
    /// inside. They differ sharply: from inside, past the critical angle,
    /// every photon is turned back into the microsurface, so the shadowing
    /// that single scattering throws away is at its worst there.
    interface: Vec<f32>,
}

static TABLE: std::sync::OnceLock<GgxEnergyTable> = std::sync::OnceLock::new();

/// The process-wide table, built on first use.
pub fn table() -> &'static GgxEnergyTable {
    TABLE.get_or_init(GgxEnergyTable::build)
}

/// Build the table now if it has not been built yet.
///
/// Call this at startup. `docs/01` §9.2 requires first pixels within 100 ms of
/// a scene change, and paying a one-off table build lazily inside the first
/// shading call would blow that budget on the very first frame — the one the
/// user is most likely to be watching.
pub fn prewarm() {
    let _ = table();
}

#[inline]
fn grid_roughness(i: usize) -> f32 {
    i as f32 / (ROUGHNESS_STEPS - 1) as f32
}

/// Grid points in `cos_theta`, spaced **quadratically**.
///
/// Reflectance varies far faster near grazing than near normal incidence: on a
/// smooth interface it is flat from `mu = 1` down to about `mu = 0.4` and then
/// climbs to 1. A uniform grid spends its resolution where nothing happens and
/// runs out where everything does, leaving a 1.4% error at `mu = 0.05`.
/// Sampling in `sqrt(mu)` puts a third of the grid below `mu = 0.1`.
#[inline]
fn grid_mu(i: usize) -> f32 {
    let t = i as f32 / (MU_STEPS - 1) as f32;
    (t * t).max(MIN_MU)
}

/// The axis coordinate in `[0, 1]` for a given `cos_theta`; inverse of
/// [`grid_mu`].
#[inline]
fn mu_param(mu: f32) -> f32 {
    mu.clamp(0.0, 1.0).sqrt()
}

/// IOR corresponding to grid index `i`.
///
/// The axis is `sqrt(r)` where `r = (ior - 1) / (ior + 1)`. Two warps stacked:
/// `r` rather than IOR because `r` is close to linear in reflectance, and the
/// square root on top of it because the grazing Fresnel curve changes shape
/// fastest as the interface gets weak. A linear-in-`r` axis leaves a 1.1%
/// error at `ior = 1.1` and grazing incidence.
#[inline]
fn grid_ior(i: usize) -> f32 {
    let t = i as f32 / (IOR_STEPS - 1) as f32;
    let r = t * t * MAX_IOR_PARAM;
    (1.0 + r) / (1.0 - r)
}

/// The axis coordinate in `[0, 1]` for a given IOR; inverse of [`grid_ior`].
#[inline]
fn ior_param(ior: f32) -> f32 {
    let ior = ior.max(1.0);
    (((ior - 1.0) / (ior + 1.0) / MAX_IOR_PARAM).clamp(0.0, 1.0)).sqrt()
}

impl GgxEnergyTable {
    /// Build by quasi-Monte-Carlo integration with a fixed seed, so the tables
    /// are bit-identical on every machine and every run. Determinism
    /// (`docs/01` §4.6) applies to precomputation too: a table that varied
    /// between runs would make renders vary between runs.
    pub fn build() -> Self {
        let iors: Vec<f32> = (0..IOR_STEPS).map(grid_ior).collect();

        // One roughness row per task. Rows are independent and each writes
        // only its own buffer, so the result does not depend on scheduling —
        // determinism (`docs/01` §4.6) survives the parallelism.
        struct Row {
            scale: Vec<f32>,
            bias: Vec<f32>,
            dielectric: Vec<f32>,
            interface: Vec<f32>,
        }

        let compute_row = |ri: usize| -> Row {
            {
                use tequila_core::sobol;

                let ggx = Ggx::isotropic(sqr(grid_roughness(ri)));
                let mut row = Row {
                    scale: vec![0.0; MU_STEPS],
                    bias: vec![0.0; MU_STEPS],
                    dielectric: vec![0.0; IOR_STEPS * MU_STEPS],
                    interface: vec![0.0; SIDES * IOR_STEPS * MU_STEPS],
                };

                for mi in 0..MU_STEPS {
                    let mu = grid_mu(mi);
                    let wo = vec3((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu);
                    let g1 = ggx.g1(wo);
                    if g1 <= 0.0 {
                        continue;
                    }

                    // With VNDF sampling the estimator for directional albedo
                    // collapses to a weight of `G2 / G1`: the NDF, the
                    // reflection Jacobian and both cosines all cancel against
                    // the PDF. That exactness is what makes this cheap enough
                    // to build at startup rather than ship as a binary blob.
                    let mut sum_scale = 0.0f64;
                    let mut sum_bias = 0.0f64;
                    let mut sum_dielectric = [0.0f64; IOR_STEPS];
                    let mut sum_interface = [[0.0f64; IOR_STEPS]; SIDES];
                    let seed = ((ri as u32) << 16) | mi as u32;

                    for s in 0..SAMPLES {
                        let u = Vec2 {
                            x: sobol::sample(s, 0, seed),
                            y: sobol::sample(s, 1, seed),
                        };
                        let wm = ggx.sample_visible_normal(wo, u);
                        let wi = -wo + wm * (2.0 * wo.dot(wm));
                        let c = wo.dot(wm).clamp(0.0, 1.0);
                        // A reflection that came out underneath the surface is
                        // shadowed and carries nothing. It is given a zero
                        // weight rather than skipped, because light refracting
                        // through that same microfacet may still get away, and
                        // the interface rows below need the sample.
                        let weight = if wi.z > 0.0 {
                            (ggx.g2(wo, wi) / g1) as f64
                        } else {
                            0.0
                        };

                        // The same microfacet, asked what a refracting
                        // interface does with it: part of the light reflects
                        // and part goes through, and the interface keeps
                        // whichever parts reach the far side of the
                        // macrosurface. The Fresnel split is taken in
                        // expectation rather than sampled, which costs one
                        // more `g2` per sample and removes its variance
                        // entirely.
                        //
                        // Above the skip below, because a reflection that was
                        // shadowed away still had a refraction through the
                        // same facet, and that refraction may well have got
                        // out.
                        if s < INTERFACE_SAMPLES {
                            for (j, &ior) in iors.iter().enumerate() {
                                for (side, sums) in sum_interface.iter_mut().enumerate() {
                                    let eta = if side == INSIDE { 1.0 / ior } else { ior };
                                    let f = fresnel::dielectric(c, eta) as f64;
                                    let mut passed = f * weight;
                                    if let Some(wt) = refraction(wo, wm, eta) {
                                        if wt.z < 0.0 {
                                            passed += (1.0 - f) * (ggx.g2(wo, wt) / g1) as f64;
                                        }
                                    }
                                    sums[j] += passed;
                                }
                            }
                        }

                        // Nothing further to add for a shadowed reflection,
                        // and the IOR loop below is expensive enough that
                        // running it for a zero weight costs 80 ms of startup.
                        if weight == 0.0 {
                            continue;
                        }

                        // Schlick: F = F0 * (1 - (1 - c)^5) + (1 - c)^5.
                        let fc = (1.0 - c).powi(5) as f64;
                        sum_scale += weight * (1.0 - fc);
                        sum_bias += weight * fc;

                        // Exact Fresnel, reusing the same microfacet sample
                        // for every IOR on the axis.
                        for (j, &ior) in iors.iter().enumerate() {
                            sum_dielectric[j] += weight * fresnel::dielectric(c, ior) as f64;
                        }

                    }

                    let n = SAMPLES as f64;
                    row.scale[mi] = (sum_scale / n).clamp(0.0, 1.0) as f32;
                    row.bias[mi] = (sum_bias / n).clamp(0.0, 1.0) as f32;
                    for (j, sum) in sum_dielectric.iter().enumerate() {
                        row.dielectric[j * MU_STEPS + mi] = (sum / n).clamp(0.0, 1.0) as f32;
                    }
                    let interface_n = INTERFACE_SAMPLES as f64;
                    for (side, sums) in sum_interface.iter().enumerate() {
                        for (j, sum) in sums.iter().enumerate() {
                            row.interface[(side * IOR_STEPS + j) * MU_STEPS + mi] =
                                (sum / interface_n).clamp(0.0, 1.0) as f32;
                        }
                    }
                }
                row
            }
        };

        // Plain threads rather than Rayon.
        //
        // This runs inside a `OnceLock` initializer, and the first call can
        // easily come from *inside* a Rayon worker — a shading call on a
        // render thread. Using Rayon here would then block that worker on
        // nested work that only the same starved pool could steal, while every
        // other worker blocks on the `OnceLock`. The result is a deadlock that
        // depends on which test or frame happens to touch the table first, so
        // it appears and disappears between runs.
        //
        // `std::thread` has no such coupling: these threads are outside any
        // pool, so nothing they do can starve the caller's.
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(ROUGHNESS_STEPS);
        let next = std::sync::atomic::AtomicUsize::new(0);
        let results: std::sync::Mutex<Vec<Option<Row>>> =
            std::sync::Mutex::new((0..ROUGHNESS_STEPS).map(|_| None).collect());

        std::thread::scope(|scope| {
            for _ in 0..threads {
                scope.spawn(|| loop {
                    let ri = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if ri >= ROUGHNESS_STEPS {
                        break;
                    }
                    let row = compute_row(ri);
                    results.lock().unwrap()[ri] = Some(row);
                });
            }
        });

        let rows: Vec<Row> = results
            .into_inner()
            .unwrap()
            .into_iter()
            .map(|row| row.expect("every roughness row is computed exactly once"))
            .collect();

        let mut scale = Vec::with_capacity(ROUGHNESS_STEPS * MU_STEPS);
        let mut bias = Vec::with_capacity(ROUGHNESS_STEPS * MU_STEPS);
        let mut dielectric = Vec::with_capacity(ROUGHNESS_STEPS * IOR_STEPS * MU_STEPS);
        let mut interface = Vec::with_capacity(ROUGHNESS_STEPS * SIDES * IOR_STEPS * MU_STEPS);
        for row in &rows {
            scale.extend_from_slice(&row.scale);
            bias.extend_from_slice(&row.bias);
            dielectric.extend_from_slice(&row.dielectric);
            interface.extend_from_slice(&row.interface);
        }

        // Cosine-weighted hemispherical averages: 2 * integral over [0,1] of
        // E(mu) * mu dmu, by the trapezoid rule over the endpoint-inclusive
        // grid the rows were sampled on.
        let cosine_average = |row: &dyn Fn(usize) -> f64| -> f32 {
            let mut acc = 0.0f64;
            for mi in 0..MU_STEPS - 1 {
                let (m0, m1) = (grid_mu(mi) as f64, grid_mu(mi + 1) as f64);
                acc += 0.5 * (row(mi) * m0 + row(mi + 1) * m1) * (m1 - m0);
            }
            (2.0 * acc).clamp(0.0, 1.0) as f32
        };

        let mut average_scale = vec![0.0f32; ROUGHNESS_STEPS];
        let mut average_bias = vec![0.0f32; ROUGHNESS_STEPS];
        let mut white_average = vec![0.0f32; ROUGHNESS_STEPS];
        let mut dielectric_average = vec![0.0f32; ROUGHNESS_STEPS * IOR_STEPS];

        for ri in 0..ROUGHNESS_STEPS {
            average_scale[ri] = cosine_average(&|mi| scale[ri * MU_STEPS + mi] as f64);
            average_bias[ri] = cosine_average(&|mi| bias[ri * MU_STEPS + mi] as f64);
            white_average[ri] = (average_scale[ri] + average_bias[ri]).clamp(0.0, 1.0);
            for j in 0..IOR_STEPS {
                dielectric_average[ri * IOR_STEPS + j] =
                    cosine_average(&|mi| dielectric[(ri * IOR_STEPS + j) * MU_STEPS + mi] as f64);
            }
        }

        GgxEnergyTable {
            scale,
            bias,
            average_scale,
            average_bias,
            white_average,
            dielectric,
            dielectric_average,
            interface,
        }
    }

    #[inline]
    fn lookup2d(values: &[f32], mu: f32, alpha: f32) -> f32 {
        let (ri, rf) = index(alpha.max(0.0).sqrt(), ROUGHNESS_STEPS);
        let (mi, mf) = index(mu_param(mu), MU_STEPS);
        let ri1 = (ri + 1).min(ROUGHNESS_STEPS - 1);
        let mi1 = (mi + 1).min(MU_STEPS - 1);
        let v00 = values[ri * MU_STEPS + mi];
        let v01 = values[ri * MU_STEPS + mi1];
        let v10 = values[ri1 * MU_STEPS + mi];
        let v11 = values[ri1 * MU_STEPS + mi1];
        let v0 = v00 + (v01 - v00) * mf;
        let v1 = v10 + (v11 - v10) * mf;
        v0 + (v1 - v0) * rf
    }

    #[inline]
    fn lookup1d(values: &[f32], alpha: f32) -> f32 {
        let (ri, rf) = index(alpha.max(0.0).sqrt(), ROUGHNESS_STEPS);
        let ri1 = (ri + 1).min(ROUGHNESS_STEPS - 1);
        values[ri] + (values[ri1] - values[ri]) * rf
    }

    /// Directional albedo of a **white** (`F = 1`) single-scattering GGX lobe.
    /// The quantity the Kulla-Conty compensation is defined against.
    #[inline]
    pub fn white_directional(&self, cos_theta: f32, alpha: f32) -> f32 {
        (Self::lookup2d(&self.scale, cos_theta, alpha)
            + Self::lookup2d(&self.bias, cos_theta, alpha))
        .clamp(0.0, 1.0)
    }

    /// Cosine-weighted hemispherical average of [`Self::white_directional`].
    #[inline]
    pub fn white_average(&self, alpha: f32) -> f32 {
        Self::lookup1d(&self.white_average, alpha).clamp(0.0, 1.0)
    }

    /// Single-scattering directional albedo for a spectral `F0`, via the
    /// Schlick split. Intended for **conductors**, where `F0` is high enough
    /// that Schlick's error is negligible; dielectrics should use
    /// [`Self::dielectric_directional`].
    #[inline]
    pub fn conductor_directional(&self, cos_theta: f32, alpha: f32, f0: Spectrum) -> Spectrum {
        let s = Self::lookup2d(&self.scale, cos_theta, alpha);
        let b = Self::lookup2d(&self.bias, cos_theta, alpha);
        (f0 * s + Spectrum::splat(b)).clamp(0.0, 1.0)
    }

    /// Cosine-weighted hemispherical average of
    /// [`Self::conductor_directional`].
    #[inline]
    pub fn conductor_average(&self, alpha: f32, f0: Spectrum) -> Spectrum {
        let s = Self::lookup1d(&self.average_scale, alpha);
        let b = Self::lookup1d(&self.average_bias, alpha);
        (f0 * s + Spectrum::splat(b)).clamp(0.0, 1.0)
    }

    /// Single-scattering directional albedo of a dielectric interface,
    /// integrated against the exact Fresnel equations.
    pub fn dielectric_directional(&self, cos_theta: f32, alpha: f32, ior: f32) -> f32 {
        let (ri, rf) = index(alpha.max(0.0).sqrt(), ROUGHNESS_STEPS);
        let (ii, if_) = index(ior_param(ior), IOR_STEPS);
        let (mi, mf) = index(mu_param(cos_theta), MU_STEPS);
        let ri1 = (ri + 1).min(ROUGHNESS_STEPS - 1);
        let ii1 = (ii + 1).min(IOR_STEPS - 1);
        let mi1 = (mi + 1).min(MU_STEPS - 1);

        let at = |r: usize, i: usize, m: usize| self.dielectric[(r * IOR_STEPS + i) * MU_STEPS + m];
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;

        let m00 = lerp(at(ri, ii, mi), at(ri, ii, mi1), mf);
        let m01 = lerp(at(ri, ii1, mi), at(ri, ii1, mi1), mf);
        let m10 = lerp(at(ri1, ii, mi), at(ri1, ii, mi1), mf);
        let m11 = lerp(at(ri1, ii1, mi), at(ri1, ii1, mi1), mf);
        lerp(lerp(m00, m01, if_), lerp(m10, m11, if_), rf).clamp(0.0, 1.0)
    }

    /// Cosine-weighted hemispherical average of
    /// [`Self::dielectric_directional`].
    ///
    /// This is what a layered material needs for the light trapped between its
    /// substrate and the underside of the interface.
    pub fn dielectric_hemispherical(&self, alpha: f32, ior: f32) -> f32 {
        let (ri, rf) = index(alpha.max(0.0).sqrt(), ROUGHNESS_STEPS);
        let (ii, if_) = index(ior_param(ior), IOR_STEPS);
        let ri1 = (ri + 1).min(ROUGHNESS_STEPS - 1);
        let ii1 = (ii + 1).min(IOR_STEPS - 1);
        let at = |r: usize, i: usize| self.dielectric_average[r * IOR_STEPS + i];
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
        lerp(
            lerp(at(ri, ii), at(ri, ii1), if_),
            lerp(at(ri1, ii), at(ri1, ii1), if_),
            rf,
        )
        .clamp(0.0, 1.0)
    }

    /// Fraction of arriving flux that a rough refracting interface passes on,
    /// counting both the light it reflects and the light it transmits.
    ///
    /// One for a smooth interface, and less than one for a rough one, by the
    /// energy single-scattering GGX drops. Its reciprocal is the compensation
    /// [`crate::lobes::DielectricTransmission`] applies.
    ///
    /// Note this is *flux*, not radiance: refraction compresses radiance by
    /// `1/eta^2` on the way in and expands it by the same factor on the way
    /// out, so a table that counted radiance would read 0.44 for a perfect
    /// interface and be useless as a measure of what had been lost.
    pub fn interface_directional(
        &self,
        cos_theta: f32,
        alpha: f32,
        ior: f32,
        from_inside: bool,
    ) -> f32 {
        let (ri, rf) = index(alpha.max(0.0).sqrt(), ROUGHNESS_STEPS);
        let (ii, if_) = index(ior_param(ior), IOR_STEPS);
        let (mi, mf) = index(mu_param(cos_theta), MU_STEPS);
        let ri1 = (ri + 1).min(ROUGHNESS_STEPS - 1);
        let ii1 = (ii + 1).min(IOR_STEPS - 1);
        let mi1 = (mi + 1).min(MU_STEPS - 1);
        let side = if from_inside { INSIDE } else { 0 };

        let at = |r: usize, i: usize, m: usize| {
            self.interface[((r * SIDES + side) * IOR_STEPS + i) * MU_STEPS + m]
        };
        let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;

        let m00 = lerp(at(ri, ii, mi), at(ri, ii, mi1), mf);
        let m01 = lerp(at(ri, ii1, mi), at(ri, ii1, mi1), mf);
        let m10 = lerp(at(ri1, ii, mi), at(ri1, ii, mi1), mf);
        let m11 = lerp(at(ri1, ii1, mi), at(ri1, ii1, mi1), mf);
        lerp(lerp(m00, m01, if_), lerp(m10, m11, if_), rf).clamp(0.0, 1.0)
    }
}

/// Refraction through a microfacet, borrowed from the lobe this table
/// compensates.
///
/// Shared rather than reimplemented on purpose. A table built against a
/// slightly different refraction than the lobe performs would compensate for
/// a loss that is not the one happening, and nothing about the result would
/// look wrong — still smooth, still plausible, just not conserving.
#[inline]
fn refraction(wo: Vec3, wm: Vec3, eta: f32) -> Option<Vec3> {
    DielectricTransmission::refract(wo, wm, eta)
}

#[inline]
fn index(x: f32, steps: usize) -> (usize, f32) {
    let scaled = x.clamp(0.0, 1.0) * (steps - 1) as f32;
    let i = (scaled as usize).min(steps - 2);
    (i, (scaled - i as f32).clamp(0.0, 1.0))
}

/// Value of the multiple-scattering compensation lobe [Kulla & Conty 2017],
/// excluding the cosine term and the spectral scale.
///
/// The energy the single-scattering lobe fails to reflect is redistributed as
/// a smooth lobe whose hemispherical integral exactly closes the gap.
#[inline]
pub fn multiple_scattering_value(e_o: f32, e_i: f32, e_avg: f32) -> f32 {
    let missing = 1.0 - e_avg;
    if missing <= 1e-6 {
        return 0.0; // nothing to compensate for
    }
    ((1.0 - e_o) * (1.0 - e_i)) / (std::f32::consts::PI * missing)
}

/// Spectral scale on the compensation lobe.
///
/// Chosen so that a perfectly reflective surface (`F_avg = 1`) conserves
/// energy exactly, and so that a coloured one saturates with each additional
/// inter-reflection the way a real rough metal does — successive bounces
/// multiply the tint, which is why rough copper reads as *more* saturated
/// than polished copper rather than less.
#[inline]
pub fn multiple_scattering_scale(f_avg: Spectrum, e_avg: f32) -> Spectrum {
    let denom = Spectrum::ONE - f_avg * (1.0 - e_avg);
    if denom.min_component() <= 1e-6 {
        return Spectrum::ZERO;
    }
    (f_avg * f_avg * e_avg) / denom
}

/// PDF of the cosine-weighted lobe used to sample the compensation term.
#[inline]
pub fn multiple_scattering_pdf(wi: Vec3) -> f32 {
    tequila_core::sampling::cosine_hemisphere_pdf(wi.z.abs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec2;
    use tequila_core::rng::Pcg32;

    fn wo_at(mu: f32) -> Vec3 {
        vec3((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu)
    }

    /// Brute-force directional albedo of a single-scattering GGX lobe by VNDF
    /// sampling, using whatever Fresnel `f` supplies.
    fn measure(roughness: f32, mu: f32, f: impl Fn(f32) -> f32, rng: &mut Pcg32) -> f32 {
        let ggx = Ggx::isotropic(sqr(roughness));
        let wo = wo_at(mu);
        let g1 = ggx.g1(wo);
        let n = 400_000;
        let mut sum = 0.0f64;
        for _ in 0..n {
            let wm = ggx.sample_visible_normal(wo, vec2(rng.next_f32(), rng.next_f32()));
            let wi = -wo + wm * (2.0 * wo.dot(wm));
            if wi.z <= 0.0 || g1 <= 0.0 {
                continue;
            }
            sum += (f(wo.dot(wm).clamp(0.0, 1.0)) * ggx.g2(wo, wi) / g1) as f64;
        }
        (sum / n as f64) as f32
    }

    /// Brute-force flux albedo of a refracting interface.
    ///
    /// Deliberately a different estimator from the one the table is built
    /// with: this one *samples* the reflect-or-refract branch, where the
    /// build integrates it. A shared mistake in the Fresnel term or in the
    /// masking would survive a test that reused the build's own arithmetic,
    /// and would not survive this.
    fn measure_interface(
        roughness: f32,
        mu: f32,
        ior: f32,
        from_inside: bool,
        rng: &mut Pcg32,
    ) -> f32 {
        let ggx = Ggx::isotropic(sqr(roughness));
        let wo = wo_at(mu);
        let g1 = ggx.g1(wo);
        if g1 <= 0.0 {
            return 0.0;
        }
        let eta = if from_inside { 1.0 / ior } else { ior };
        let n = 200_000;
        let mut sum = 0.0f64;
        for _ in 0..n {
            let wm = ggx.sample_visible_normal(wo, vec2(rng.next_f32(), rng.next_f32()));
            let f = fresnel::dielectric(wo.dot(wm).clamp(0.0, 1.0), eta);
            let reflected = -wo + wm * (2.0 * wo.dot(wm));
            // Past the critical angle `refract` has no answer and `f` is
            // exactly 1, so the branch below is never asked for one.
            let (wi, transmitted) = if rng.next_f32() < f {
                (reflected, false)
            } else {
                match refraction(wo, wm, eta) {
                    Some(wi) => (wi, true),
                    None => (reflected, false),
                }
            };
            // A direction that came out on the wrong side of the
            // macrosurface was shadowed, and its light is what is missing.
            if (wi.z < 0.0) != transmitted {
                continue;
            }
            sum += (ggx.g2(wo, wi) / g1) as f64;
        }
        (sum / n as f64) as f32
    }

    /// The interface rows against that measurement, on both sides.
    ///
    /// These rows are what every rough glass surface divides by, so an error
    /// here is an error in the brightness of the glass rather than in some
    /// second-order correction — and it would show up as glass that conserves
    /// energy at one roughness and not at another, which is far harder to
    /// recognise than glass that is simply too dark.
    #[test]
    fn interface_table_matches_a_measurement() {
        let t = table();
        let mut rng = Pcg32::new(77, 77);
        let mut worst = (0.0f32, String::new());
        for ior in [1.1f32, 1.5, 2.42] {
            for roughness in [0.1f32, 0.3, 0.6, 1.0] {
                for from_inside in [false, true] {
                    for mu in [0.1f32, 0.35, 0.7, 1.0] {
                        let measured = measure_interface(roughness, mu, ior, from_inside, &mut rng);
                        let predicted =
                            t.interface_directional(mu, sqr(roughness), ior, from_inside);
                        let error = (measured - predicted).abs();
                        if error > worst.0 {
                            worst = (
                                error,
                                format!(
                                    "ior {ior}, roughness {roughness}, {} at mu {mu}: \
                                     measured {measured:.4}, table {predicted:.4}",
                                    if from_inside { "inside" } else { "outside" }
                                ),
                            );
                        }
                    }
                }
            }
        }
        assert!(worst.0 < 0.01, "interface table is off: {}", worst.1);
    }

    /// A flat interface loses nothing, so the compensation it asks for is
    /// none. Worth pinning separately because it is the case every smooth
    /// glass object in every scene takes, and a table that drifted at
    /// roughness 0 would tint all of them.
    #[test]
    fn a_smooth_interface_passes_everything() {
        let t = table();
        for ior in [1.1f32, 1.5, 2.42, 3.0] {
            for from_inside in [false, true] {
                for mu in [0.05f32, 0.3, 0.8, 1.0] {
                    let passed = t.interface_directional(mu, 0.0, ior, from_inside);
                    assert!(
                        (passed - 1.0).abs() < 0.005,
                        "ior {ior}, {} at mu {mu}: a flat interface passed {passed:.4}",
                        if from_inside { "inside" } else { "outside" }
                    );
                }
            }
        }
    }

    /// The white row must match a direct measurement: it is what the
    /// compensation term is defined against and what every layered material
    /// scales by.
    #[test]
    fn white_albedo_matches_a_measurement() {
        let t = table();
        let mut rng = Pcg32::new(52, 52);
        for roughness in [0.0f32, 0.1, 0.4, 0.7, 1.0] {
            for mu in [0.05f32, 0.2, 0.5, 0.9, 1.0] {
                let measured = measure(roughness, mu, |_| 1.0, &mut rng);
                let predicted = t.white_directional(mu, sqr(roughness));
                assert!(
                    (measured - predicted).abs() < 0.01,
                    "roughness {roughness} mu {mu}: measured {measured}, table {predicted}"
                );
            }
        }
    }

    /// The load-bearing claim for dielectrics: the IOR-axis table reproduces
    /// the exact Fresnel equations everywhere, *including* the near-smooth
    /// grazing corner where the Schlick split is worst. This is what pins the
    /// layered furnace test.
    #[test]
    fn dielectric_table_matches_exact_fresnel() {
        let t = table();
        let mut rng = Pcg32::new(51, 51);
        let mut worst = 0.0f32;
        for ior in [1.1f32, 1.33, 1.5, 1.8, 2.42, 3.0] {
            for roughness in [0.0f32, 0.05, 0.1, 0.3, 0.6, 1.0] {
                for mu in [0.05f32, 0.1, 0.3, 0.6, 0.9, 1.0] {
                    let exact = measure(roughness, mu, |c| fresnel::dielectric(c, ior), &mut rng);
                    let predicted = t.dielectric_directional(mu, sqr(roughness), ior);
                    let error = (exact - predicted).abs();
                    worst = worst.max(error);
                    assert!(
                        error < 0.01,
                        "ior {ior} roughness {roughness} mu {mu}: \
                         exact {exact}, table {predicted}"
                    );
                }
            }
        }
        // Recorded rather than merely asserted, so a regression in accuracy
        // shows up in the failure message rather than silently eating margin.
        assert!(worst < 0.01, "worst dielectric table error {worst}");
    }

    /// The Schlick split is used only for conductors, where `F0` is high. Hold
    /// it to account across the metal range so that the choice stays
    /// justified rather than merely assumed.
    #[test]
    fn conductor_split_sum_is_accurate_where_it_is_used() {
        let t = table();
        let mut rng = Pcg32::new(53, 53);
        let mut worst = 0.0f32;
        for f0 in [0.5f32, 0.7, 0.85, 0.95, 1.0] {
            for roughness in [0.05f32, 0.3, 0.6, 1.0] {
                for mu in [0.1f32, 0.3, 0.6, 1.0] {
                    let schlick = |c: f32| f0 + (1.0 - f0) * (1.0 - c).powi(5);
                    let measured = measure(roughness, mu, schlick, &mut rng);
                    let predicted = t
                        .conductor_directional(mu, sqr(roughness), Spectrum::splat(f0))
                        .g;
                    let error = (measured - predicted).abs();
                    worst = worst.max(error);
                    assert!(
                        error < 0.01,
                        "f0 {f0} roughness {roughness} mu {mu}: \
                         measured {measured}, table {predicted}"
                    );
                }
            }
        }
        assert!(worst < 0.01, "worst conductor split-sum error {worst}");
    }

    /// The hemispherical averages must match integrating the directional
    /// values, or a layered material's inter-reflection factor is wrong.
    #[test]
    fn hemispherical_averages_match_integration() {
        let t = table();
        const N: usize = 512;
        let integrate = |f: &dyn Fn(f32) -> f32| -> f64 {
            let mut acc = 0.0f64;
            for i in 0..N {
                let mu = (i as f32 + 0.5) / N as f32;
                acc += f(mu) as f64 * mu as f64;
            }
            2.0 * acc / N as f64
        };

        for roughness in [0.0f32, 0.3, 0.7, 1.0] {
            let alpha = sqr(roughness);
            for ior in [1.33f32, 1.5, 2.0] {
                let integrated = integrate(&|mu| t.dielectric_directional(mu, alpha, ior));
                let tabulated = t.dielectric_hemispherical(alpha, ior) as f64;
                assert!(
                    (integrated - tabulated).abs() < 0.005,
                    "ior {ior} roughness {roughness}: \
                     integrated {integrated}, tabulated {tabulated}"
                );
            }
            let integrated = integrate(&|mu| t.white_directional(mu, alpha));
            let tabulated = t.white_average(alpha) as f64;
            assert!(
                (integrated - tabulated).abs() < 0.005,
                "white roughness {roughness}: \
                 integrated {integrated}, tabulated {tabulated}"
            );
        }
    }

    /// A smooth dielectric's tabulated albedo must equal the closed-form
    /// Fresnel exactly — that limit is where every other approximation is
    /// anchored.
    #[test]
    fn smooth_dielectric_row_matches_closed_form_fresnel() {
        let t = table();
        for ior in [1.33f32, 1.5, 2.0, 2.42] {
            for mu in [0.05f32, 0.2, 0.5, 0.8, 1.0] {
                let table_value = t.dielectric_directional(mu, 0.0, ior);
                let exact = fresnel::dielectric(mu, ior);
                assert!(
                    (table_value - exact).abs() < 0.01,
                    "ior {ior} mu {mu}: table {table_value}, exact {exact}"
                );
            }
        }
    }

    /// Rougher surfaces lose more energy to inter-reflection, never less.
    #[test]
    fn white_albedo_falls_monotonically_with_roughness() {
        let t = table();
        for mu in [0.3f32, 0.7, 1.0] {
            let mut previous = 1.01;
            for roughness in [0.0f32, 0.2, 0.4, 0.6, 0.8, 1.0] {
                let e = t.white_directional(mu, sqr(roughness));
                assert!((0.0..=1.0).contains(&e), "roughness {roughness}: {e}");
                assert!(
                    e < previous + 1e-3,
                    "E rose at roughness {roughness}, mu {mu}"
                );
                previous = e;
            }
        }
        // A mirror loses nothing; a fully rough surface loses a substantial
        // fraction — that is the energy the compensation lobe restores.
        assert!(t.white_directional(0.7, 0.0) > 0.999);
        assert!(t.white_directional(0.7, 1.0) < 0.85);
    }

    /// A perfect reflector must have its full energy restored: the
    /// compensation term exists precisely so this holds.
    #[test]
    fn compensation_closes_the_budget_for_a_perfect_reflector() {
        let t = table();
        for roughness in [0.2f32, 0.5, 0.8, 1.0] {
            let e_avg = t.white_average(sqr(roughness));
            let scale = multiple_scattering_scale(Spectrum::ONE, e_avg);
            assert!(
                (scale.g - 1.0).abs() < 1e-4,
                "roughness {roughness}: scale {scale:?}"
            );
        }
    }

    #[test]
    fn coloured_compensation_saturates_rather_than_brightening() {
        let e_avg = 0.8;
        let f_avg = Spectrum {
            r: 0.95,
            g: 0.6,
            b: 0.3,
        };
        let scale = multiple_scattering_scale(f_avg, e_avg);
        // Every channel is attenuated relative to its own Fresnel...
        assert!(scale.r < f_avg.r && scale.g < f_avg.g && scale.b < f_avg.b);
        // ...and the weak channel hardest, so the tint deepens with roughness.
        assert!(scale.r / f_avg.r > scale.b / f_avg.b);
    }

    #[test]
    fn table_is_deterministic() {
        let a = GgxEnergyTable::build();
        let b = GgxEnergyTable::build();
        assert_eq!(a.scale, b.scale);
        assert_eq!(a.bias, b.bias);
        assert_eq!(a.white_average, b.white_average);
        assert_eq!(a.dielectric, b.dielectric);
        assert_eq!(a.dielectric_average, b.dielectric_average);
    }

    #[test]
    fn grids_cover_both_endpoints() {
        assert_eq!(grid_roughness(0), 0.0);
        assert_eq!(grid_roughness(ROUGHNESS_STEPS - 1), 1.0);
        assert_eq!(grid_mu(MU_STEPS - 1), 1.0);
        assert_eq!(grid_mu(0), MIN_MU);
        // The quadratic warp must round-trip, or lookups read the wrong cell.
        for i in 1..MU_STEPS {
            let expected = i as f32 / (MU_STEPS - 1) as f32;
            assert!(
                (mu_param(grid_mu(i)) - expected).abs() < 1e-5,
                "mu index {i}"
            );
        }
        // A third of the grid must sit below mu = 0.1, where it is needed.
        let low = (0..MU_STEPS).filter(|&i| grid_mu(i) < 0.1).count();
        assert!(
            low >= MU_STEPS / 4,
            "only {low} of {MU_STEPS} grid points near grazing"
        );
        assert_eq!(grid_ior(0), 1.0);
        assert!((grid_ior(IOR_STEPS - 1) - 3.0).abs() < 1e-5);
        // Round-tripping an IOR through the axis parameterization must land on
        // the grid point it came from.
        for i in 0..IOR_STEPS {
            let p = ior_param(grid_ior(i));
            let expected = i as f32 / (IOR_STEPS - 1) as f32;
            assert!(
                (p - expected).abs() < 1e-5,
                "ior index {i}: {p} vs {expected}"
            );
        }
        // Lookups at the endpoints hit tabulated values, not extrapolations.
        assert_eq!(index(1.0, ROUGHNESS_STEPS), (ROUGHNESS_STEPS - 2, 1.0));
        assert_eq!(index(0.0, ROUGHNESS_STEPS), (0, 0.0));
    }
}
