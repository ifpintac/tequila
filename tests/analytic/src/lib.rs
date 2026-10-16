//! Statistical machinery for the analytic correctness tests.
//!
//! `docs/05-ROADMAP-AND-TESTING.md` §3.2 calls the χ² test "the highest-value
//! test in the entire suite", and explains why:
//!
//! > A `pdf()` that disagrees with `sample()` produces images that are wrong
//! > in a way no visual inspection reliably catches — slightly too bright at
//! > certain roughnesses, subtly wrong under MIS — and it will otherwise be
//! > found months later.
//!
//! The method: draw many samples, bin them over the sphere, integrate the
//! claimed density over each bin to get an expected count, and apply Pearson's
//! χ² test. Sampling and evaluation are derived independently, so an error in
//! either shows up.
//!
//! # Why the bins are chosen from the data
//!
//! A fixed grid over the sphere cannot test a glossy lobe. At roughness 0.05
//! the GGX lobe is about 0.3 degrees wide, while a bin of a 16x32 grid spans
//! 11 degrees — the lobe occupies well under a thousandth of one bin. Every
//! sample lands in that bin, and no affordable quadrature resolves the spike
//! inside it: the expected count comes out ~150x too low and the test reports
//! a catastrophic failure of a perfectly correct sampler.
//!
//! So the bin boundaries are **quantiles of a pilot run**: a separate,
//! discarded set of samples is drawn, and the bands are placed so each holds
//! roughly equal pilot mass. Bins are then automatically tiny where the
//! distribution is concentrated and coarse where it is flat, and the pdf
//! varies little across any one of them.
//!
//! This is not circular. The pilot fixes only the *partition*; the counts come
//! from fresh samples and the expected counts from `pdf` alone, so the two
//! sides of the comparison remain independent. Choosing a partition badly
//! could only make the test weaker, never make a wrong sampler pass — and the
//! self-tests below include deliberately wrong samplers to keep that honest.
//!
//! # Support leakage is checked separately
//!
//! Adapting the bands to the data creates one blind spot. If a sampler
//! occasionally emits a direction where the density is zero — a reflection
//! lobe scattering below the horizon, say — a quantile band wide enough to
//! contain both that direction and the legitimate bulk will show the right
//! total, and the leak vanishes into it.
//!
//! Binning cannot fix that, so it is not asked to. Every drawn sample is
//! checked directly against `pdf`, and a single sample where the density
//! claims none can exist fails the test outright. That is a stronger check
//! than any histogram: it has no resolution limit at all.
//!
//! # Bands are angles, not cosines
//!
//! The obvious parameterization — bands as intervals of `cos(angle)`, which
//! makes every bin's solid angle a plain product with no Jacobian — cannot
//! represent a concentrated lobe. Near a cosine of 1 the `f32` grid is
//! spaced about 6e-8 apart, so for a lobe a thousandth of a radian wide the
//! cosines of most of its samples round to exactly 1.0. Quantile boundaries
//! then collide, and the partition collapses.
//!
//! Bands are therefore intervals of the **angle** from the dominant axis,
//! computed as `atan2(|w x axis|, w . axis)` rather than `acos(w . axis)`.
//! The cross-product form keeps full relative precision for small angles,
//! where `acos` loses half its digits. The price is a `sin(theta)` factor in
//! the quadrature, which is a line of code.

use tequila_core::math::{vec3, Onb, Vec3};

/// A distribution over the sphere that claims to sample and evaluate itself
/// consistently.
pub trait SphericalDistribution {
    /// Draw a direction. `None` means the sampler declined — a lobe with no
    /// energy in that configuration, or a delta lobe — and the draw is
    /// discarded rather than counted.
    fn sample(&self, u: (f32, f32), uc: f32) -> Option<Vec3>;

    /// Density at `direction`, in solid-angle measure.
    fn pdf(&self, direction: Vec3) -> f32;
}

/// Result of a χ² goodness-of-fit test.
#[derive(Clone, Debug)]
pub struct Chi2Result {
    pub statistic: f64,
    pub degrees_of_freedom: usize,
    pub p_value: f64,
    /// Bins pooled because their expected count was too small for the χ²
    /// approximation to hold.
    pub pooled_bins: usize,
    pub samples_drawn: usize,
    /// Samples the sampler produced where `pdf` reports zero density. Any at
    /// all is a hard inconsistency.
    pub zero_density_samples: usize,
    /// The partition could not resolve the distribution, so the comparison
    /// says nothing either way. See [`Chi2Result::resolution_limited`].
    pub resolution_limited: bool,
    /// The worst single-bin disagreement, for diagnosis.
    pub worst_bin: Option<BinDiscrepancy>,
}

impl Chi2Result {
    /// Whether the test actually ran.
    ///
    /// A distribution can be too concentrated to partition: bands are
    /// intervals of `cos(angle)`, and near a cosine of 1 the `f32` grid runs
    /// out of distinct values. A cone of half-angle 0.08 degrees has about
    /// sixteen representable cosines across its whole support, which is not
    /// enough to place twelve boundaries in.
    ///
    /// Callers must check this. An untestable result carries `p_value = 1.0`,
    /// and treating that as a pass would let a broken sampler through in
    /// exactly the regime where errors are hardest to see.
    pub fn is_testable(&self) -> bool {
        self.samples_drawn > 0
    }

    /// Whether the result is inconclusive because the partition was too coarse
    /// for the distribution, rather than because the sampler is wrong.
    ///
    /// This happens with a lobe far narrower than a bin *alongside* a broad
    /// one: the pilot samples that place the band boundaries are dominated by
    /// the broad lobe, so the partition is laid out for it, and the narrow lobe
    /// lands inside a single cell as an effective delta. Quadrature over that
    /// cell steps straight over the spike and reports an expected count of
    /// almost nothing against thousands of observations.
    ///
    /// It is distinguishable from a real fault, and the thing that
    /// distinguishes it is [`Chi2Result::zero_density_samples`]. If the
    /// sampler were producing directions the density excludes, that check
    /// would have caught it — it needs no resolution at all, only a pointwise
    /// evaluation. When every sample sits where the density is positive and
    /// the *integral* over their cell is nevertheless zero, the integral is
    /// what is wrong.
    ///
    /// Callers should treat this as "no answer", not as a pass.
    pub fn resolution_limited(&self) -> bool {
        self.resolution_limited
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BinDiscrepancy {
    pub band: usize,
    pub sector: usize,
    pub observed: f64,
    pub expected: f64,
}

/// Configuration for [`chi2_test`].
#[derive(Clone, Copy, Debug)]
pub struct Chi2Config {
    /// Bands along the distribution's dominant axis.
    pub bands: usize,
    /// Sectors around it.
    pub sectors: usize,
    pub samples: usize,
    /// Pilot samples used to place the band boundaries. Discarded afterwards.
    pub pilot_samples: usize,
    /// Sub-samples per axis when integrating the density over a bin.
    pub quadrature_resolution: usize,
    /// Bins with fewer expected counts than this are pooled; the χ²
    /// approximation is poor for sparse cells.
    pub min_expected_count: f64,
}

impl Default for Chi2Config {
    fn default() -> Self {
        Chi2Config {
            bands: 12,
            sectors: 24,
            samples: 400_000,
            pilot_samples: 40_000,
            quadrature_resolution: 24,
            min_expected_count: 5.0,
        }
    }
}

/// The partition of the sphere the test bins into.
///
/// Bands are intervals of the polar angle from the dominant axis; sectors are
/// intervals of the azimuth around it.
#[derive(Clone, Debug)]
struct Partition {
    frame: Onb,
    /// `bands + 1` polar-angle boundaries in radians, ascending within
    /// `[0, pi]`.
    boundaries: Vec<f32>,
    sectors: usize,
}

/// Polar angle of `local` from `+Z`, accurate for small angles.
///
/// `acos(local.z)` would lose about half its significant digits here: near
/// `z = 1` the derivative of `acos` is unbounded, so a rounding error in `z`
/// of one ulp becomes an error in the angle of `sqrt(ulp)`. The `atan2` form
/// keeps full relative precision, which is what makes a near-specular lobe
/// partitionable at all.
#[inline]
fn polar_angle(local: Vec3) -> f32 {
    let horizontal = (local.x * local.x + local.y * local.y).sqrt();
    horizontal.atan2(local.z)
}

impl Partition {
    /// Locate a direction. Returns `(band, sector)`.
    fn locate(&self, direction: Vec3) -> (usize, usize) {
        let local = self.frame.to_local(direction);
        // Clamped into the partition. A sample landing outside the bands is a
        // rare tail beyond the pilot's extremes; putting it in the nearest
        // band costs a negligible amount of accuracy, and genuine
        // out-of-support samples are caught by the density check instead.
        let theta = polar_angle(local).clamp(
            *self.boundaries.first().unwrap(),
            *self.boundaries.last().unwrap(),
        );
        let band = match self
            .boundaries
            .binary_search_by(|b| b.partial_cmp(&theta).unwrap_or(std::cmp::Ordering::Less))
        {
            Ok(i) => i.min(self.boundaries.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.boundaries.len() - 2),
        };
        let phi = local.y.atan2(local.x).rem_euclid(std::f32::consts::TAU);
        let sector = ((phi / std::f32::consts::TAU) * self.sectors as f32) as usize;
        (band, sector.min(self.sectors - 1))
    }

    fn direction(&self, theta: f32, phi: f32) -> Vec3 {
        let (sin, cos) = theta.sin_cos();
        self.frame
            .to_world(vec3(sin * phi.cos(), sin * phi.sin(), cos))
    }
}

/// The direction the distribution is densest around.
///
/// The *mean* direction is the obvious choice and the wrong one. A layered
/// material is bimodal — a narrow specular spike sitting on a broad diffuse
/// lobe — and the mean lands between the two, so the spike ends up off-axis.
/// Bands are annuli around the axis, so an off-axis spike occupies a thin arc
/// of one annulus: most sectors of that band are empty, and the few holding
/// the spike are under-resolved azimuthally by any affordable quadrature.
///
/// Centring on the mode puts the spike at the pole instead, where a band is a
/// small disc spanning the full azimuth and every sector carries mass.
///
/// Found by histogramming the pilot over a coarse equal-area grid and taking
/// the centroid of the densest cell — linear in the sample count, and precise
/// enough given the bands are placed by quantile afterwards.
fn dominant_axis(directions: &[Vec3]) -> Vec3 {
    const BANDS: usize = 32;
    const SECTORS: usize = 64;

    let mut counts = vec![0u32; BANDS * SECTORS];
    let cell_of = |w: Vec3| -> usize {
        let band = (((w.z + 1.0) * 0.5) * BANDS as f32) as usize;
        let phi = w.y.atan2(w.x).rem_euclid(std::f32::consts::TAU);
        let sector = ((phi / std::f32::consts::TAU) * SECTORS as f32) as usize;
        band.min(BANDS - 1) * SECTORS + sector.min(SECTORS - 1)
    };
    for w in directions {
        counts[cell_of(*w)] += 1;
    }

    // The centroid of the whole *high-density region*, not of the single
    // densest cell.
    //
    // Taking one cell works for a peaked distribution and fails badly for a
    // flat one: inside a uniform cone every cell holds the same expected
    // count, so the densest is decided by Poisson noise and the axis lands at
    // a random point inside the cone rather than at its centre. The bands are
    // then annuli about an off-centre axis, the cone's rim cuts across several
    // of them, and quadrature at those discontinuities fails a correct
    // sampler.
    //
    // Averaging over every cell within half the peak density gives the right
    // answer in both regimes: for a flat distribution nearly all cells qualify
    // and the result is the ordinary mean, while for a peaked one only the
    // peak qualifies.
    let peak = counts.iter().copied().max().unwrap_or(0);
    if peak == 0 {
        return Vec3::Z;
    }
    let threshold = peak / 2;

    let mut centroid = Vec3::ZERO;
    for w in directions {
        if counts[cell_of(*w)] >= threshold {
            centroid += *w;
        }
    }
    // A distribution with no preferred direction at all — a uniform sphere —
    // has a centroid near zero, and any axis serves equally well.
    centroid.try_normalize().unwrap_or(Vec3::Z)
}

/// Draw a pilot set and derive a partition adapted to it.
fn build_partition(
    distribution: &dyn SphericalDistribution,
    config: Chi2Config,
    seed: u64,
) -> Option<Partition> {
    use tequila_core::rng::Pcg32;

    let mut rng = Pcg32::new(seed ^ 0xa5a5_a5a5, seed.wrapping_add(0x1234_5678));
    let mut directions = Vec::with_capacity(config.pilot_samples);
    for _ in 0..config.pilot_samples {
        let u = (rng.next_f32(), rng.next_f32());
        if let Some(w) = distribution.sample(u, rng.next_f32()) {
            directions.push(w);
        }
    }
    if directions.len() < config.bands * 8 {
        return None;
    }

    let axis = dominant_axis(&directions);
    let frame = Onb::from_normal(axis);

    let mut projections: Vec<f32> = directions
        .iter()
        .map(|w| polar_angle(frame.to_local(*w)))
        .collect();
    projections.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // Equal-mass bands, by empirical quantile.
    //
    // The outer boundaries hug the observed support rather than reaching to
    // the poles. A band stretching from the pole to the first quantile would
    // be enormous while holding a sliver of support at its very edge, and no
    // affordable quadrature would find that sliver — the expected count would
    // come out at zero and a correct sampler would be reported as broken.
    // Leakage outside the support is caught by the direct density check
    // instead, which needs no resolution at all.
    let lowest = projections[0];
    let highest = projections[projections.len() - 1];
    // A deliberately tiny margin. It exists only so that main-run samples
    // fractionally beyond the pilot's extremes still land inside the
    // partition. Any larger and the outer bands straddle the edge of the
    // support, where the density steps discontinuously to zero — and midpoint
    // quadrature smears that step across a whole sub-cell, which for a sharply
    // bounded distribution is a systematic error large enough to fail a
    // correct sampler.
    let margin = ((highest - lowest) * 1e-3).max(f32::MIN_POSITIVE);

    let mut boundaries = Vec::with_capacity(config.bands + 1);
    boundaries.push((lowest - margin).max(0.0));
    for i in 1..config.bands {
        let q = i as f32 / config.bands as f32;
        let index = ((q * projections.len() as f32) as usize).min(projections.len() - 1);
        let previous = *boundaries.last().unwrap();
        // Keep the boundaries strictly increasing. Adjacent quantiles can
        // still collide if a distribution is degenerate enough, and a band of
        // zero width would divide by zero in the quadrature.
        let step = (previous.abs() * f32::EPSILON * 8.0).max(f32::MIN_POSITIVE);
        boundaries.push(projections[index].max(previous + step));
    }
    let previous = *boundaries.last().unwrap();
    let step = (previous.abs() * f32::EPSILON * 8.0).max(f32::MIN_POSITIVE);
    boundaries.push(
        (highest + margin)
            .max(previous + step)
            .min(std::f32::consts::PI),
    );
    // Degenerate partitions (every boundary collapsed to 1.0) cannot be tested.
    if boundaries.windows(2).any(|w| w[1] <= w[0]) {
        return None;
    }

    Some(Partition {
        frame,
        boundaries,
        sectors: config.sectors,
    })
}

/// Run Pearson's χ² goodness-of-fit test between a sampler and its density.
pub fn chi2_test(
    distribution: &dyn SphericalDistribution,
    config: Chi2Config,
    seed: u64,
) -> Chi2Result {
    use tequila_core::rng::Pcg32;

    let Some(partition) = build_partition(distribution, config, seed) else {
        // Nothing to test: the sampler produced too few usable samples.
        return Chi2Result {
            statistic: 0.0,
            degrees_of_freedom: 1,
            p_value: 1.0,
            pooled_bins: 0,
            samples_drawn: 0,
            zero_density_samples: 0,
            resolution_limited: false,
            worst_bin: None,
        };
    };

    let bins = config.bands * config.sectors;
    let mut observed = vec![0.0f64; bins];

    // Fresh samples: the pilot set fixed the partition and is not reused.
    let mut rng = Pcg32::new(seed, seed.wrapping_mul(0x9e37_79b9));
    let mut drawn = 0usize;
    let mut zero_density = 0usize;
    let attempts = config.samples;
    for _ in 0..config.samples {
        let u = (rng.next_f32(), rng.next_f32());
        let Some(w) = distribution.sample(u, rng.next_f32()) else {
            continue;
        };
        // The support check. A sample where the density is zero cannot be
        // reconciled with that density by any amount of binning.
        // `is_sign_positive` alone would accept zero, and `<= 0.0` would
        // accept NaN. Both are leaks, so both are rejected.
        let density = distribution.pdf(w);
        if density.is_nan() || density <= 0.0 {
            zero_density += 1;
        }
        let (band, sector) = partition.locate(w);
        observed[band * config.sectors + sector] += 1.0;
        drawn += 1;
    }

    // Expected counts, by quadrature of the density over each bin. Bands are
    // angles, so the solid-angle element carries a `sin(theta)` factor.
    //
    // The polar resolution is refined until the estimate stops moving, rather
    // than fixed. Equal-mass bands hold equal *probability*, not equal
    // *range*: a lobe with a heavy tail — and GGX's falls off as roughly the
    // inverse fourth power of the angle — puts its outermost band across a
    // huge angular span with almost all the mass bunched at the inner edge.
    // A fixed midpoint rule misses that bunching and reports an expected count
    // several times too low, which reads as a catastrophic sampling failure of
    // a perfectly correct lobe. Refinement is cheap because it only happens in
    // the few bands that need it.
    let dphi = std::f32::consts::TAU / config.sectors as f32;
    let mut expected = vec![0.0f64; bins];

    for band in 0..config.bands {
        let theta0 = partition.boundaries[band];
        let theta1 = partition.boundaries[band + 1];
        for sector in 0..config.sectors {
            let phi0 = sector as f32 * dphi;
            let integral = integrate_bin(
                distribution,
                &partition,
                (theta0, theta1),
                (phi0, phi0 + dphi),
                config,
            );
            // Scaled by *attempts*, not acceptances.
            //
            // A BSDF's `sample` may legitimately decline: a visible-normal
            // draw can reflect below the horizon, and the lobe reports no
            // scattering rather than inventing a direction. Its `pdf` is
            // therefore a sub-probability density, integrating to the
            // acceptance rate rather than to 1 — which is exactly what MIS
            // needs, since the quantity that matters there is the probability
            // of actually generating a given direction.
            //
            // So the comparison has to be against every draw made, including
            // the declined ones. Scaling by acceptances instead silently
            // renormalizes the empirical distribution and leaves a systematic
            // discrepancy the size of the rejection rate — a few percent at
            // grazing incidence, which is enough to fail a correct lobe.
            expected[band * config.sectors + sector] = integral * attempts as f64;
        }
    }

    // Pool sparse bins: Pearson's statistic assumes each cell's count is
    // roughly normal, which fails below about five expected counts, and
    // unpooled sparse cells produce spurious failures.
    let mut statistic = 0.0f64;
    let mut cells = 0usize;
    let mut pooled_observed = 0.0f64;
    let mut pooled_expected = 0.0f64;
    let mut pooled_bins = 0usize;
    let mut worst: Option<BinDiscrepancy> = None;
    let mut worst_contribution = 0.0f64;

    for band in 0..config.bands {
        for sector in 0..config.sectors {
            let i = band * config.sectors + sector;
            let (o, e) = (observed[i], expected[i]);
            if e < config.min_expected_count {
                pooled_observed += o;
                pooled_expected += e;
                pooled_bins += 1;
                continue;
            }
            let contribution = (o - e) * (o - e) / e;
            statistic += contribution;
            cells += 1;
            if contribution > worst_contribution {
                worst_contribution = contribution;
                worst = Some(BinDiscrepancy {
                    band,
                    sector,
                    observed: o,
                    expected: e,
                });
            }
        }
    }

    let mut resolution_limited = false;
    if pooled_expected >= config.min_expected_count {
        statistic += (pooled_observed - pooled_expected).powi(2) / pooled_expected;
        cells += 1;
    } else if pooled_observed > config.min_expected_count {
        // Samples landed in bins whose integrated density is essentially zero.
        // There are two ways to get here and they need different answers.
        //
        // If any sample sat where `pdf` itself reports zero, the sampler is
        // producing directions the density excludes — a hard inconsistency,
        // and one that needs no resolution to detect because it is a pointwise
        // check. `zero_density` below turns that into an infinite statistic.
        //
        // Otherwise the density is positive at every one of those samples and
        // it is the *integral* over their cells that came out at zero, which
        // means the quadrature stepped over the distribution rather than the
        // sampler misplacing it. Reporting infinity there fails a correct
        // sampler; reporting a pass hides a real one. The honest answer is
        // neither, so the result says it could not tell.
        if zero_density > 0 {
            statistic = f64::INFINITY;
            cells = cells.max(1);
        } else {
            resolution_limited = true;
        }
        if worst.is_none() {
            worst = Some(BinDiscrepancy {
                band: usize::MAX,
                sector: usize::MAX,
                observed: pooled_observed,
                expected: pooled_expected,
            });
        }
    }

    // Leakage outside the support is not a matter of degree.
    if zero_density > 0 {
        statistic = f64::INFINITY;
    }

    let dof = cells.saturating_sub(1).max(1);
    Chi2Result {
        p_value: chi2_p_value(statistic, dof),
        statistic,
        degrees_of_freedom: dof,
        pooled_bins,
        resolution_limited,
        samples_drawn: drawn,
        zero_density_samples: zero_density,
        worst_bin: worst,
    }
}

/// Integrate the density over one bin, refining the polar resolution until the
/// estimate converges.
fn integrate_bin(
    distribution: &dyn SphericalDistribution,
    partition: &Partition,
    theta: (f32, f32),
    phi: (f32, f32),
    config: Chi2Config,
) -> f64 {
    /// Beyond this the cost stops being worth it; a bin that has not settled
    /// by here contributes a slightly noisy expected count, not a wrong one.
    const MAX_STEPS: usize = 512;
    const TOLERANCE: f64 = 1e-3;

    let midpoint = |polar_steps: usize, azimuth_steps: usize| -> f64 {
        let dtheta = (theta.1 - theta.0) / polar_steps as f32;
        let dphi = (phi.1 - phi.0) / azimuth_steps as f32;
        let mut sum = 0.0f64;
        for a in 0..polar_steps {
            let t = theta.0 + (a as f32 + 0.5) * dtheta;
            let sin_theta = t.sin() as f64;
            if sin_theta <= 0.0 {
                continue;
            }
            for b in 0..azimuth_steps {
                let p = phi.0 + (b as f32 + 0.5) * dphi;
                sum += distribution.pdf(partition.direction(t, p)) as f64 * sin_theta;
            }
        }
        sum * (dtheta * dphi) as f64
    };

    // Both axes refine, not just the polar one.
    //
    // The azimuth used to be pinned at `quadrature_resolution`. That is fine
    // for a lobe that is broad around its axis, and badly wrong for one that
    // is narrow in *both* angles — a near-specular lobe seen at grazing
    // incidence, where the reflection is a thin arc rather than a disc. The
    // fixed sub-sampling stepped over most of it and reported an expected
    // count a factor of twenty low, failing a correct sampler with a χ²
    // statistic in the millions.
    //
    // Refining both and watching the estimate settle takes that away: on the
    // case that exposed it, rough dielectric transmission at roughness 0.05
    // and cos 0.15, the statistic falls from 2.2 million to 25 as the azimuth
    // refines, and it is 25 that is the truth about the sampler. Doubling both
    // axes quadruples the work per step, but only the few bins that have not
    // settled ever take another one.
    let mut polar = config.quadrature_resolution;
    let mut azimuth = config.quadrature_resolution;
    let mut previous = midpoint(polar, azimuth);
    while polar < MAX_STEPS && azimuth < MAX_STEPS {
        polar *= 2;
        azimuth *= 2;
        let current = midpoint(polar, azimuth);
        // Converged when the refinement stops changing the answer. An
        // absolute floor keeps a bin of essentially zero density from
        // refining forever chasing relative precision it cannot have.
        if (current - previous).abs() <= TOLERANCE * current.max(1e-12) {
            return current;
        }
        previous = current;
    }
    previous
}

/// Upper tail probability of the χ² distribution: the probability of a
/// statistic at least this extreme when sampler and density do agree.
pub fn chi2_p_value(statistic: f64, degrees_of_freedom: usize) -> f64 {
    if !statistic.is_finite() {
        return 0.0;
    }
    if statistic <= 0.0 {
        return 1.0;
    }
    regularized_gamma_q(degrees_of_freedom as f64 * 0.5, statistic * 0.5)
}

/// Regularized upper incomplete gamma function `Q(a, x)`.
///
/// Series expansion below the crossover, continued fraction above; the
/// standard pairing, because each converges quickly exactly where the other
/// does not.
fn regularized_gamma_q(a: f64, x: f64) -> f64 {
    if x < a + 1.0 {
        1.0 - gamma_series(a, x)
    } else {
        gamma_continued_fraction(a, x)
    }
}

fn gamma_series(a: f64, x: f64) -> f64 {
    const MAX_ITERATIONS: usize = 1000;
    const EPSILON: f64 = 1e-14;
    if x <= 0.0 {
        return 0.0;
    }
    let mut ap = a;
    let mut sum = 1.0 / a;
    let mut delta = sum;
    for _ in 0..MAX_ITERATIONS {
        ap += 1.0;
        delta *= x / ap;
        sum += delta;
        if delta.abs() < sum.abs() * EPSILON {
            break;
        }
    }
    sum * (-x + a * x.ln() - ln_gamma(a)).exp()
}

fn gamma_continued_fraction(a: f64, x: f64) -> f64 {
    const MAX_ITERATIONS: usize = 1000;
    const EPSILON: f64 = 1e-14;
    const TINY: f64 = 1e-300;

    let mut b = x + 1.0 - a;
    let mut c = 1.0 / TINY;
    let mut d = 1.0 / b;
    let mut h = d;
    for i in 1..=MAX_ITERATIONS {
        let an = -(i as f64) * (i as f64 - a);
        b += 2.0;
        d = an * d + b;
        if d.abs() < TINY {
            d = TINY;
        }
        c = b + an / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;
        if (delta - 1.0).abs() < EPSILON {
            break;
        }
    }
    (-x + a * x.ln() - ln_gamma(a)).exp() * h
}

/// Natural log of the gamma function, by the Lanczos approximation.
fn ln_gamma(x: f64) -> f64 {
    const COEFFICIENTS: [f64; 6] = [
        76.180_091_729_471_46,
        -86.505_320_329_416_77,
        24.014_098_240_830_91,
        -1.231_739_572_450_155,
        0.120_865_097_386_617_9e-2,
        -0.539_523_938_495_3e-5,
    ];
    let mut y = x;
    let tmp = x + 5.5 - (x + 0.5) * (x + 5.5).ln();
    let mut series = 1.000_000_000_190_015;
    for c in COEFFICIENTS {
        y += 1.0;
        series += c / y;
    }
    -tmp + (2.506_628_274_631_000_5 * series / x).ln()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::vec2;
    use tequila_core::sampling::{
        cosine_hemisphere_pdf, sample_cosine_hemisphere, sample_uniform_sphere, UNIFORM_SPHERE_PDF,
    };

    struct UniformSphere;
    impl SphericalDistribution for UniformSphere {
        fn sample(&self, u: (f32, f32), _uc: f32) -> Option<Vec3> {
            Some(sample_uniform_sphere(vec2(u.0, u.1)))
        }
        fn pdf(&self, _direction: Vec3) -> f32 {
            UNIFORM_SPHERE_PDF
        }
    }

    struct CosineHemisphere;
    impl SphericalDistribution for CosineHemisphere {
        fn sample(&self, u: (f32, f32), _uc: f32) -> Option<Vec3> {
            Some(sample_cosine_hemisphere(vec2(u.0, u.1)))
        }
        fn pdf(&self, direction: Vec3) -> f32 {
            if direction.z <= 0.0 {
                0.0
            } else {
                cosine_hemisphere_pdf(direction.z)
            }
        }
    }

    /// A uniform cone, specified by its **half-angle in radians** rather than
    /// by the cosine of that angle. The analogue of a near-specular lobe, and
    /// the case that defeats a fixed grid.
    ///
    /// The parameterization matters at these widths. Given a cosine, the
    /// support test `z >= cos_max` and the sampler disagree by an ulp or two,
    /// and the solid angle `2*pi*(1 - cos_max)` loses most of its significant
    /// digits to cancellation — a 10% error in the density for a cone this
    /// narrow. Working in the angle avoids both: the support test uses the
    /// same stable `atan2` form the partition does, and the solid angle uses
    /// the half-angle identity `1 - cos(a) = 2 sin^2(a/2)`.
    struct NarrowCone {
        half_angle: f32,
    }

    impl NarrowCone {
        fn from_half_angle(half_angle: f32) -> Self {
            NarrowCone { half_angle }
        }

        fn solid_angle(&self) -> f32 {
            // 2 * pi * (1 - cos a), evaluated without cancellation.
            let s = (self.half_angle * 0.5).sin();
            2.0 * std::f32::consts::TAU * s * s
        }
    }

    impl SphericalDistribution for NarrowCone {
        fn sample(&self, u: (f32, f32), _uc: f32) -> Option<Vec3> {
            // Sampled through the angle, not through its cosine.
            //
            // The usual formulation draws `cos(theta)` uniformly over
            // `[cos a, 1]` and recovers `sin(theta)` as `sqrt(1 - cos^2)`.
            // That subtraction cancels catastrophically for a narrow cone: at
            // a half-angle of a milliradian the recovered angle carries only
            // one or two correct digits, and the fixture then disagrees with
            // its own support test.
            //
            // Uniform solid angle means `1 - cos(theta)` is uniform, and
            // `1 - cos(theta) = 2 sin^2(theta/2)`, so `sin(theta/2)` is
            // distributed as `sqrt(u) * sin(a/2)`. That form has no
            // cancellation anywhere.
            let half = (u.0.max(0.0).sqrt() * (self.half_angle * 0.5).sin())
                .clamp(-1.0, 1.0)
                .asin();
            let theta = 2.0 * half;
            let phi = std::f32::consts::TAU * u.1;
            let (sin_theta, cos_theta) = theta.sin_cos();
            Some(vec3(
                sin_theta * phi.cos(),
                sin_theta * phi.sin(),
                cos_theta,
            ))
        }

        fn pdf(&self, direction: Vec3) -> f32 {
            // A tolerance of a few ulps only. The sampler above is exact to
            // within rounding, so anything further outside is a real leak.
            if polar_angle(direction) <= self.half_angle * (1.0 + 1e-5) {
                1.0 / self.solid_angle()
            } else {
                0.0
            }
        }
    }

    /// Independent seeds, and how many of them must clear the significance
    /// level for a distribution to count as passing.
    ///
    /// Not all of them, deliberately. A *correct* sampler tested at p = 0.01
    /// fails once in a hundred runs by construction — that is what a
    /// significance level means. Asserting that a single run passes would make
    /// this suite flaky at exactly the rate it is designed to tolerate, and a
    /// flaky correctness suite gets muted, which is worse than not having one.
    /// Requiring a majority across independent seeds keeps the power to catch
    /// systematic error while dropping the false-alarm rate to negligible.
    const SEEDS: [u64; 5] = [12345, 777, 31337, 90210, 4242];
    const MUST_PASS: usize = 4;

    fn passes(d: &dyn SphericalDistribution) -> (usize, Vec<f64>) {
        let p_values: Vec<f64> = SEEDS
            .iter()
            .map(|&seed| chi2_test(d, Chi2Config::default(), seed).p_value)
            .collect();
        (p_values.iter().filter(|&&p| p > 0.01).count(), p_values)
    }

    #[test]
    fn correct_distributions_pass() {
        for (name, d) in [
            (
                "uniform sphere",
                &UniformSphere as &dyn SphericalDistribution,
            ),
            ("cosine hemisphere", &CosineHemisphere),
        ] {
            let (passed, p_values) = passes(d);
            assert!(
                passed >= MUST_PASS,
                "{name}: only {passed} of {} seeds passed; p-values {p_values:?}",
                SEEDS.len()
            );
        }
    }

    /// The property the quantile binning exists for: a distribution
    /// concentrated into a tiny solid angle must still be testable. A cone of
    /// half-angle 0.15 degrees is narrower than a roughness-0.05 GGX lobe.
    #[test]
    fn extremely_concentrated_distributions_are_testable() {
        for half_angle in [0.1415f32, 0.01415] {
            let d = NarrowCone::from_half_angle(half_angle);
            let r = chi2_test(&d, Chi2Config::default(), 4242);
            assert!(
                r.is_testable(),
                "cone {half_angle} rad: the harness declined it"
            );
            // The test must not have degenerated into one giant bin: that
            // would "pass" by having no resolution at all.
            assert!(
                r.degrees_of_freedom > 10,
                "cone {half_angle} rad collapsed to {} degrees of freedom",
                r.degrees_of_freedom
            );
            let (passed, p_values) = passes(&d);
            assert!(
                passed >= MUST_PASS,
                "cone {half_angle} rad: only {passed} of {} seeds passed; \
                 p-values {p_values:?}",
                SEEDS.len()
            );
        }
    }

    /// A cone of half-angle 0.08 degrees. Binning in `cos(theta)` cannot
    /// represent this at all — every sample's cosine rounds to 1.0 — while
    /// binning in the angle handles it with room to spare. Kept as a
    /// regression test against anyone reintroducing the cosine
    /// parameterization for the Jacobian it would save.
    #[test]
    fn a_lobe_far_narrower_than_any_material_is_still_testable() {
        let d = NarrowCone::from_half_angle(0.001_415);
        let r = chi2_test(&d, Chi2Config::default(), 4242);
        assert!(r.is_testable(), "the harness declined a 0.08-degree cone");
        assert!(r.degrees_of_freedom > 10);
        let (passed, p_values) = passes(&d);
        assert!(
            passed >= MUST_PASS,
            "0.08-degree cone: only {passed} of {} seeds passed; p-values {p_values:?}",
            SEEDS.len()
        );
    }

    /// A sampler that never produces anything must be reported as untestable
    /// rather than silently "passing" — that is what `is_testable` is for, and
    /// the integration tests assert on it.
    #[test]
    fn a_sampler_that_declines_everything_is_reported_not_passed() {
        struct Silent;
        impl SphericalDistribution for Silent {
            fn sample(&self, _u: (f32, f32), _uc: f32) -> Option<Vec3> {
                None
            }
            fn pdf(&self, _direction: Vec3) -> f32 {
                0.0
            }
        }
        let r = chi2_test(&Silent, Chi2Config::default(), 1);
        assert!(!r.is_testable());
        // And it must not look like a pass to a careless caller reading only
        // the p-value... which is exactly why callers must check.
        assert_eq!(r.samples_drawn, 0);
    }

    /// The harness must have teeth. If it passed everything it would be worse
    /// than useless — it would be a false assurance.
    #[test]
    fn a_mismatched_sampler_is_caught() {
        struct Mismatched;
        impl SphericalDistribution for Mismatched {
            fn sample(&self, u: (f32, f32), _uc: f32) -> Option<Vec3> {
                Some(sample_cosine_hemisphere(vec2(u.0, u.1)))
            }
            fn pdf(&self, direction: Vec3) -> f32 {
                // Uniform over the hemisphere: a valid density, the wrong one.
                if direction.z <= 0.0 {
                    0.0
                } else {
                    std::f32::consts::FRAC_1_PI * 0.5
                }
            }
        }
        let r = chi2_test(&Mismatched, Chi2Config::default(), 999);
        assert!(
            r.p_value < 1e-6,
            "a cosine sampler claiming a uniform density passed: p = {}",
            r.p_value
        );
    }

    /// A subtly wrong density: 3% too high everywhere. This is what a missing
    /// normalization constant looks like, and exactly the class of bug visual
    /// inspection never catches.
    #[test]
    fn a_three_percent_error_is_caught() {
        struct SlightlyWrong;
        impl SphericalDistribution for SlightlyWrong {
            fn sample(&self, u: (f32, f32), _uc: f32) -> Option<Vec3> {
                Some(sample_cosine_hemisphere(vec2(u.0, u.1)))
            }
            fn pdf(&self, direction: Vec3) -> f32 {
                if direction.z <= 0.0 {
                    0.0
                } else {
                    cosine_hemisphere_pdf(direction.z) * 1.03
                }
            }
        }
        let r = chi2_test(&SlightlyWrong, Chi2Config::default(), 4242);
        assert!(
            r.p_value < 0.01,
            "a 3% density error passed: chi2 = {:.1}, p = {:.5}",
            r.statistic,
            r.p_value
        );
    }

    /// A density that is right on average but wrong in shape — too much
    /// weight near the pole, too little at the edge. Total energy is
    /// preserved, so an "integrates to one" check would pass it.
    #[test]
    fn a_misshapen_density_is_caught() {
        struct Misshapen;
        impl SphericalDistribution for Misshapen {
            fn sample(&self, u: (f32, f32), _uc: f32) -> Option<Vec3> {
                Some(sample_cosine_hemisphere(vec2(u.0, u.1)))
            }
            fn pdf(&self, direction: Vec3) -> f32 {
                if direction.z <= 0.0 {
                    return 0.0;
                }
                // cos^1.1 renormalized: same total, different shape.
                let n = 1.1f32;
                direction.z.powf(n) * (n + 1.0) / std::f32::consts::TAU
            }
        }
        let r = chi2_test(&Misshapen, Chi2Config::default(), 31337);
        assert!(
            r.p_value < 0.01,
            "a misshapen density passed: chi2 = {:.1}, p = {:.5}",
            r.statistic,
            r.p_value
        );
    }

    /// Samples landing where the density says nothing can are a hard
    /// inconsistency, however few of them there are.
    #[test]
    fn samples_outside_the_support_fail_outright() {
        struct LeaksBelowHorizon;
        impl SphericalDistribution for LeaksBelowHorizon {
            fn sample(&self, u: (f32, f32), _uc: f32) -> Option<Vec3> {
                let mut w = sample_cosine_hemisphere(vec2(u.0, u.1));
                // One sample in two hundred escapes below the surface.
                if u.0 < 0.005 {
                    w.z = -w.z;
                }
                Some(w)
            }
            fn pdf(&self, direction: Vec3) -> f32 {
                if direction.z <= 0.0 {
                    0.0
                } else {
                    cosine_hemisphere_pdf(direction.z)
                }
            }
        }
        let r = chi2_test(&LeaksBelowHorizon, Chi2Config::default(), 7);
        assert!(
            r.zero_density_samples > 0,
            "the leak was not detected at all"
        );
        assert_eq!(
            r.p_value, 0.0,
            "a leaking sampler passed: p = {}",
            r.p_value
        );
    }

    /// The p-value machinery, against known values of the χ² distribution.
    #[test]
    fn chi_squared_tail_probabilities_are_right() {
        // Medians: chi2 with 1 dof is ~0.4549; with 2 dof it is 2 ln 2.
        assert!((chi2_p_value(0.454_936, 1) - 0.5).abs() < 1e-3);
        assert!((chi2_p_value(1.386_294, 2) - 0.5).abs() < 1e-3);
        // 95th percentile with 1 dof is 3.841.
        assert!((chi2_p_value(3.841_459, 1) - 0.05).abs() < 1e-3);
        // 99th percentile with 10 dof is 23.209.
        assert!((chi2_p_value(23.209_25, 10) - 0.01).abs() < 1e-3);
        assert_eq!(chi2_p_value(0.0, 5), 1.0);
        assert!(chi2_p_value(1e9, 5) < 1e-12);
        assert_eq!(chi2_p_value(f64::INFINITY, 5), 0.0);
    }

    #[test]
    fn log_gamma_matches_known_values() {
        // ln(1!) = 0, ln(4!) = ln 24, ln(gamma(0.5)) = ln(sqrt(pi)).
        assert!(ln_gamma(2.0).abs() < 1e-10);
        assert!((ln_gamma(5.0) - 24.0f64.ln()).abs() < 1e-10);
        assert!((ln_gamma(0.5) - std::f64::consts::PI.sqrt().ln()).abs() < 1e-10);
    }

    /// Band boundaries must be strictly increasing and span the sphere, or
    /// `locate` puts samples in the wrong bin.
    #[test]
    fn the_partition_is_well_formed_and_covers_the_sphere() {
        for d in [
            &UniformSphere as &dyn SphericalDistribution,
            &CosineHemisphere,
            &NarrowCone::from_half_angle(0.01415),
        ] {
            let p = build_partition(d, Chi2Config::default(), 5).expect("partition");
            assert!(p.boundaries.windows(2).all(|w| w[1] > w[0]));
            assert!(*p.boundaries.first().unwrap() >= 0.0);
            assert!(*p.boundaries.last().unwrap() <= std::f32::consts::PI);

            // Every direction on the sphere must land in a valid bin.
            let mut rng = tequila_core::rng::Pcg32::new(1, 1);
            for _ in 0..5_000 {
                let w = sample_uniform_sphere(vec2(rng.next_f32(), rng.next_f32()));
                let (band, sector) = p.locate(w);
                assert!(band < p.boundaries.len() - 1);
                assert!(sector < p.sectors);
            }
        }
    }
}
