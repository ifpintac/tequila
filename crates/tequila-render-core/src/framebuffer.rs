//! The accumulation buffer and its AOVs.
//!
//! `docs/01-RENDERER-CORE.md` §4.6 constrains this module more than anything
//! else does: **accumulation must be order-independent**. The sample value at
//! `(pixel, sample_index, dimension)` is a pure function of its inputs, and if
//! the buffer then sums those samples in an order that depends on thread
//! scheduling, the render stops being reproducible.
//!
//! That rules out the obvious implementation. §4.6: "Do not use atomic float
//! adds into a shared buffer for the beauty pass — atomic float addition is
//! not associative and will silently break determinism." Instead each tile
//! owns its pixels outright, and every pixel's samples are summed in sample
//! order by exactly one thread.

use tequila_core::spectrum::Spectrum;

/// The render passes, alongside the beauty buffer (§12).
///
/// A separate allocation rather than more fields on [`Pixel`], because a pixel
/// is a small struct and there are eight million of them at 4K: a dozen spectra
/// each would cost a gigabyte and a half of a render nobody asked passes for.
/// Here the cost is paid only when passes are requested.
///
/// Everything in it is accumulated the same way the beauty buffer is —
/// summed per sample, divided by the sample count on the way out — with two
/// exceptions that are *identities* rather than quantities: the object and
/// material a pixel's camera ray hit. Averaging an identifier produces a number
/// that names nothing, so those keep the first sample's answer.
#[derive(Clone, Debug)]
pub struct AovBuffers {
    pub width: usize,
    pub height: usize,
    pub passes: crate::aov::PassBuffers,
    pub position_sum: Vec<tequila_core::math::Vec3>,
    pub object_id: Vec<u32>,
    pub material_id: Vec<u32>,
    /// Direct lighting with nothing in the way, and as it actually arrived.
    /// §12's Shadow pass is the ratio.
    pub direct_unshadowed: Vec<Spectrum>,
    pub direct_visible: Vec<Spectrum>,
    pub occlusion_sum: Vec<f32>,
    /// The labels alone, premultiplied, and their coverage, summed.
    pub labels_sum: Vec<Spectrum>,
    pub labels_alpha_sum: Vec<f32>,
    pub objects: Option<crate::cryptomatte::Cryptomatte>,
    pub materials: Option<crate::cryptomatte::Cryptomatte>,
    /// The Cryptomatte identifier of each instance and material, by index.
    ///
    /// Precomputed because the standard hashes an object's *name* and the
    /// renderer has an index: hashing a string at every sample would put a
    /// string lookup in the inner loop, and hashing the index instead would
    /// produce identifiers no manifest could name — which is a Cryptomatte a
    /// compositor can display and not select from.
    object_ids: Vec<f32>,
    material_ids: Vec<f32>,
}

impl AovBuffers {
    pub fn new(width: usize, height: usize, set: &crate::aov::AovSet) -> AovBuffers {
        let count = width * height;
        AovBuffers {
            width,
            height,
            passes: crate::aov::PassBuffers::new(width, height, set.light_paths.len()),
            position_sum: vec![tequila_core::math::Vec3::ZERO; count],
            object_id: vec![u32::MAX; count],
            material_id: vec![u32::MAX; count],
            direct_unshadowed: vec![Spectrum::ZERO; count],
            direct_visible: vec![Spectrum::ZERO; count],
            occlusion_sum: vec![0.0; count],
            labels_sum: vec![Spectrum::ZERO; if set.data.labels { count } else { 0 }],
            labels_alpha_sum: vec![0.0; if set.data.labels { count } else { 0 }],
            objects: set
                .cryptomatte
                .then(|| crate::cryptomatte::Cryptomatte::new("CryptoObject", width, height)),
            materials: set
                .cryptomatte
                .then(|| crate::cryptomatte::Cryptomatte::new("CryptoMaterial", width, height)),
            object_ids: Vec::new(),
            material_ids: Vec::new(),
        }
    }

    /// Tell the Cryptomattes what things are called.
    ///
    /// Without this every identifier is still a valid float and every matte
    /// still selects, but the manifest is empty — so a compositor can only
    /// offer "click a pixel" instead of a list of part names, which is most of
    /// what a Cryptomatte is for.
    pub fn name_things(&mut self, objects: &[String], materials: &[String]) {
        self.object_ids = objects
            .iter()
            .map(|name| crate::cryptomatte::name_to_float(name))
            .collect();
        self.material_ids = materials
            .iter()
            .map(|name| crate::cryptomatte::name_to_float(name))
            .collect();
        if let Some(matte) = &mut self.objects {
            for name in objects {
                matte.register(name);
            }
        }
        if let Some(matte) = &mut self.materials {
            for name in materials {
                matte.register(name);
            }
        }
    }

    /// The identifier of an instance, or `None` where nothing was hit.
    #[inline]
    pub fn object_id_of(&self, index: u32) -> Option<f32> {
        if index == u32::MAX {
            return None;
        }
        // An unnamed instance falls back to its index, so a scene that never
        // called `name_things` still produces a working matte — just one whose
        // manifest is empty.
        Some(
            self.object_ids
                .get(index as usize)
                .copied()
                .unwrap_or_else(|| crate::cryptomatte::hash_to_float(index)),
        )
    }

    #[inline]
    pub fn material_id_of(&self, index: u32) -> Option<f32> {
        if index == u32::MAX {
            return None;
        }
        Some(
            self.material_ids
                .get(index as usize)
                .copied()
                .unwrap_or_else(|| crate::cryptomatte::hash_to_float(index)),
        )
    }

    /// One sample's contribution to everything but the beauty buffer.
    pub fn accumulate(
        &mut self,
        x: usize,
        y: usize,
        result: &crate::integrator::PathResult,
        passes: &crate::aov::PathPasses,
    ) {
        if x >= self.width || y >= self.height {
            return;
        }
        let index = y * self.width + x;
        self.passes.accumulate(x, y, passes);
        self.position_sum[index] = self.position_sum[index] + result.position;
        self.direct_unshadowed[index] = self.direct_unshadowed[index] + result.direct_unshadowed;
        self.direct_visible[index] = self.direct_visible[index] + result.direct_visible;
        self.occlusion_sum[index] += result.occlusion;
        if !self.labels_sum.is_empty() {
            self.labels_sum[index] = self.labels_sum[index] + result.labels;
            self.labels_alpha_sum[index] += result.labels_alpha;
        }
        // An identity, not a quantity: the first sample that hit something
        // answers for the pixel, and averaging two object numbers would name a
        // third object that is not there.
        if self.object_id[index] == u32::MAX {
            self.object_id[index] = result.object_id;
            self.material_id[index] = result.material_id;
        }
    }

    /// §12's Shadow pass: how much of the direct light was blocked.
    ///
    /// A ratio rather than a difference, so it is a matte a compositor can
    /// multiply by rather than a radiance it would have to match exposure with.
    pub fn shadow(&self, x: usize, y: usize) -> f32 {
        let index = y.min(self.height - 1) * self.width + x.min(self.width - 1);
        let unshadowed = self.direct_unshadowed[index].luminance();
        if unshadowed <= 1e-9 {
            return 0.0;
        }
        let visible = self.direct_visible[index].luminance();
        (1.0 - visible / unshadowed).clamp(0.0, 1.0)
    }

    pub fn mean_position(&self, x: usize, y: usize, samples: u32) -> tequila_core::math::Vec3 {
        if samples == 0 {
            return tequila_core::math::Vec3::ZERO;
        }
        self.position_sum[y.min(self.height - 1) * self.width + x.min(self.width - 1)]
            / samples as f32
    }

    pub fn mean_occlusion(&self, x: usize, y: usize, samples: u32) -> f32 {
        if samples == 0 {
            return 0.0;
        }
        self.occlusion_sum[y.min(self.height - 1) * self.width + x.min(self.width - 1)]
            / samples as f32
    }

    /// The labels alone at a pixel, premultiplied, and their coverage.
    pub fn mean_labels(&self, x: usize, y: usize, samples: u32) -> (Spectrum, f32) {
        if samples == 0 || self.labels_sum.is_empty() {
            return (Spectrum::ZERO, 0.0);
        }
        let index = y.min(self.height - 1) * self.width + x.min(self.width - 1);
        let n = samples as f32;
        (self.labels_sum[index] * (1.0 / n), self.labels_alpha_sum[index] / n)
    }

    /// Rank the Cryptomattes. Called once, before writing.
    pub fn finish(&mut self, samples: u32) {
        if let Some(objects) = &mut self.objects {
            objects.finish(samples);
        }
        if let Some(materials) = &mut self.materials {
            materials.finish(samples);
        }
    }
}

/// Per-pixel accumulated state.
#[derive(Clone, Copy, Debug)]
pub struct Pixel {
    /// Running sum of radiance, in scene-linear ACEScg.
    pub sum: Spectrum,
    /// The part of `sum` that is background the camera saw uncovered
    /// ([`crate::integrator::PathResult::background`]).
    pub background_sum: Spectrum,
    /// Running sum of squared luminance, for the variance estimate that drives
    /// adaptive sampling (§4.4) and firefly rejection (§3.5).
    pub sum_squared_luminance: f64,
    pub albedo_sum: Spectrum,
    pub normal_sum: tequila_core::math::Vec3,
    /// Sum of coverage: 1 for a ray that hit geometry, 0 for one that escaped.
    /// This is the alpha channel, and it is what makes a shadow-catcher and a
    /// transparent background work.
    pub coverage_sum: f32,
    pub depth_sum: f32,
    pub samples: u32,
    /// Energy removed by firefly rejection, kept so the bias is auditable
    /// (§3.5, and the "clamped energy" AOV of §12).
    pub clamped_energy: f32,
    /// Set once the pixel's error falls below the adaptive threshold.
    pub retired: bool,
    /// The instance the *first* camera ray through this pixel struck, or
    /// `u32::MAX` for background.
    ///
    /// The first rather than a majority or the last, because this is what the
    /// selection outline is drawn from and an outline that flickered between
    /// two instances along a shared edge would be worse than none. The first
    /// sample is arbitrary but it is *stable*, which is the property that
    /// matters — and it costs one comparison and one store per sample rather
    /// than a per-pixel tally.
    pub first_instance: u32,
}

impl Default for Pixel {
    fn default() -> Self {
        Pixel {
            sum: Spectrum::ZERO,
            background_sum: Spectrum::ZERO,
            sum_squared_luminance: 0.0,
            albedo_sum: Spectrum::ZERO,
            normal_sum: tequila_core::math::Vec3::ZERO,
            coverage_sum: 0.0,
            depth_sum: 0.0,
            samples: 0,
            clamped_energy: 0.0,
            retired: false,
            first_instance: u32::MAX,
        }
    }
}

impl Pixel {
    #[inline]
    pub fn mean(&self) -> Spectrum {
        if self.samples == 0 {
            Spectrum::ZERO
        } else {
            self.sum / self.samples as f32
        }
    }

    /// The mean without the background: the colour premultiplied by the
    /// coverage, which is what an image with alpha carries.
    ///
    /// Taking [`Pixel::mean`] there counted the background into every pixel
    /// that was partly covered. At an edge that is a faint halo of the
    /// backdrop; on glass whose alpha lets most rays through (a dirty bulb,
    /// grime as coverage) most of the colour was the white sky, and the
    /// glass came out white.
    #[inline]
    pub fn mean_foreground(&self) -> Spectrum {
        if self.samples == 0 {
            Spectrum::ZERO
        } else {
            let f = (self.sum - self.background_sum) / self.samples as f32;
            Spectrum { r: f.r.max(0.0), g: f.g.max(0.0), b: f.b.max(0.0) }
        }
    }

    #[inline]
    pub fn mean_albedo(&self) -> Spectrum {
        if self.samples == 0 {
            Spectrum::ZERO
        } else {
            self.albedo_sum / self.samples as f32
        }
    }

    #[inline]
    pub fn mean_normal(&self) -> tequila_core::math::Vec3 {
        self.normal_sum
            .try_normalize()
            .unwrap_or(tequila_core::math::Vec3::ZERO)
    }

    #[inline]
    pub fn alpha(&self) -> f32 {
        if self.samples == 0 {
            0.0
        } else {
            self.coverage_sum / self.samples as f32
        }
    }

    /// Mean distance to the first hit. Zero where every ray escaped.
    #[inline]
    pub fn mean_depth(&self) -> f32 {
        if self.samples == 0 {
            0.0
        } else {
            self.depth_sum / self.samples as f32
        }
    }

    /// Mean energy removed by firefly rejection (§3.5, §12).
    #[inline]
    pub fn mean_clamped_energy(&self) -> f32 {
        if self.samples == 0 {
            0.0
        } else {
            self.clamped_energy / self.samples as f32
        }
    }

    /// Standard deviation of the pixel's luminance across its samples.
    pub fn luminance_std_dev(&self) -> f32 {
        if self.samples < 2 {
            return f32::INFINITY;
        }
        let n = self.samples as f64;
        let mean = self.mean().luminance() as f64;
        let variance = (self.sum_squared_luminance / n - mean * mean).max(0.0);
        variance.sqrt() as f32
    }

    /// Relative error, the halting criterion of §4.4:
    /// `sigma / (sqrt(n) * max(mean_luminance, eps))`.
    pub fn relative_error(&self) -> f32 {
        if self.samples < 2 {
            return f32::INFINITY;
        }
        let mean = self.mean().luminance().max(1e-4);
        self.luminance_std_dev() / ((self.samples as f32).sqrt() * mean)
    }
}

/// Which image a viewer wants out of the buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aov {
    Beauty,
    Albedo,
    Normal,
    Depth,
    Alpha,
    /// Per-pixel sample count, as a diagnostic for adaptive sampling.
    SampleCount,
    /// Energy removed by firefly rejection (§12).
    ClampedEnergy,
}

/// The accumulation buffer.
#[derive(Clone, Debug)]
pub struct Framebuffer {
    pub width: usize,
    pub height: usize,
    pixels: Vec<Pixel>,
}

impl Framebuffer {
    pub fn new(width: usize, height: usize) -> Self {
        Framebuffer {
            width,
            height,
            pixels: vec![Pixel::default(); width * height],
        }
    }

    #[inline]
    pub fn pixel(&self, x: usize, y: usize) -> &Pixel {
        &self.pixels[y * self.width + x]
    }

    #[inline]
    pub fn pixel_mut(&mut self, x: usize, y: usize) -> &mut Pixel {
        &mut self.pixels[y * self.width + x]
    }

    pub fn pixels(&self) -> &[Pixel] {
        &self.pixels
    }

    /// Exclusive access to the raw pixel storage.
    ///
    /// The tile renderer partitions this slice across threads. It is exposed
    /// rather than hidden behind a per-pixel accessor because the disjointness
    /// argument lives at the tile level, not the pixel level.
    pub fn pixels_mut(&mut self) -> &mut [Pixel] {
        &mut self.pixels
    }

    /// Discard everything and start over. The response to a change the dirty
    /// -state taxonomy classifies as a full restart (§9.3).
    pub fn clear(&mut self) {
        for p in &mut self.pixels {
            *p = Pixel::default();
        }
    }

    /// Reset only the pixels inside a rectangle.
    ///
    /// This is the mechanism behind "region restart" in §9.3 — the row that
    /// lets a material edit keep every accumulated sample outside the affected
    /// object's screen bounds. §9.3 calls it worth more perceived performance
    /// than a 2x raw speedup.
    pub fn clear_region(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        let x1 = x1.min(self.width);
        let y1 = y1.min(self.height);
        for y in y0..y1 {
            for x in x0..x1 {
                self.pixels[y * self.width + x] = Pixel::default();
            }
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        if width != self.width || height != self.height {
            *self = Framebuffer::new(width, height);
        }
    }

    /// The smallest sample count any live pixel has.
    ///
    /// This, not the maximum, is what the HUD should report as "samples": with
    /// adaptive sampling on, the maximum overstates how converged the image
    /// is.
    pub fn min_samples(&self) -> u32 {
        self.pixels
            .iter()
            .filter(|p| !p.retired)
            .map(|p| p.samples)
            .min()
            .unwrap_or_else(|| self.pixels.iter().map(|p| p.samples).max().unwrap_or(0))
    }

    pub fn max_samples(&self) -> u32 {
        self.pixels.iter().map(|p| p.samples).max().unwrap_or(0)
    }

    /// Fraction of pixels that have met the convergence threshold.
    ///
    /// §4.4 asks for this in the HUD: "a far more meaningful progress signal
    /// than sample count, and something KeyShot does not offer".
    pub fn converged_fraction(&self) -> f32 {
        if self.pixels.is_empty() {
            return 1.0;
        }
        let retired = self.pixels.iter().filter(|p| p.retired).count();
        retired as f32 / self.pixels.len() as f32
    }

    /// Mark pixels whose relative error has fallen below `threshold`.
    ///
    /// A retired pixel is re-activated if a neighbour's error rises sharply,
    /// which §4.4 requires to prevent premature halting at edges — an edge
    /// pixel can look converged for a while simply because it has not yet
    /// sampled the high-contrast side.
    pub fn update_adaptive_retirement(&mut self, threshold: f32, warmup_samples: u32) {
        let mut retire = vec![false; self.pixels.len()];
        for (i, p) in self.pixels.iter().enumerate() {
            retire[i] = p.samples >= warmup_samples && p.relative_error() < threshold;
        }
        // Un-retire anything bordering a pixel that is still noisy.
        for y in 0..self.height {
            for x in 0..self.width {
                let i = y * self.width + x;
                if !retire[i] {
                    continue;
                }
                'neighbours: for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let nx = x as i64 + dx;
                        let ny = y as i64 + dy;
                        if nx < 0 || ny < 0 || nx >= self.width as i64 || ny >= self.height as i64 {
                            continue;
                        }
                        let n = &self.pixels[ny as usize * self.width + nx as usize];
                        if n.samples >= warmup_samples && n.relative_error() > threshold * 4.0 {
                            retire[i] = false;
                            break 'neighbours;
                        }
                    }
                }
            }
        }
        for (p, r) in self.pixels.iter_mut().zip(retire) {
            p.retired = r;
        }
    }

    /// Read an AOV as RGB.
    pub fn read(&self, aov: Aov, x: usize, y: usize) -> Spectrum {
        let p = self.pixel(x, y);
        match aov {
            Aov::Beauty => p.mean(),
            Aov::Albedo => p.mean_albedo(),
            Aov::Normal => {
                // Remapped from [-1,1] to [0,1] for display; the EXR writer
                // stores the raw signed values.
                let n = p.mean_normal() * 0.5 + tequila_core::math::Vec3::splat(0.5);
                Spectrum::from_rendering_rgb(n)
            }
            Aov::Depth => {
                let d = if p.samples == 0 {
                    0.0
                } else {
                    p.depth_sum / p.samples as f32
                };
                Spectrum::splat(d)
            }
            Aov::Alpha => Spectrum::splat(p.alpha()),
            Aov::SampleCount => Spectrum::splat(p.samples as f32),
            Aov::ClampedEnergy => Spectrum::splat(p.clamped_energy),
        }
    }

    /// Copy the beauty pass and its guide AOVs into flat `f32` buffers, the
    /// layout a denoiser wants.
    pub fn to_denoiser_buffers(&self) -> DenoiserBuffers {
        self.to_denoiser_buffers_with(false)
    }

    /// The same, with the colour as an image with alpha carries it — the
    /// foreground alone — when `foreground` is set.
    pub fn to_denoiser_buffers_with(&self, foreground: bool) -> DenoiserBuffers {
        let n = self.pixels.len();
        let mut color = Vec::with_capacity(n * 3);
        let mut albedo = Vec::with_capacity(n * 3);
        let mut normal = Vec::with_capacity(n * 3);
        for p in &self.pixels {
            let c = if foreground { p.mean_foreground() } else { p.mean() };
            color.extend_from_slice(&c.to_array());
            albedo.extend_from_slice(&p.mean_albedo().clamp(0.0, 1.0).to_array());
            let nrm = p.mean_normal();
            normal.extend_from_slice(&[nrm.x, nrm.y, nrm.z]);
        }
        DenoiserBuffers {
            width: self.width,
            height: self.height,
            color,
            albedo,
            normal,
        }
    }
}

/// Flat buffers for the denoiser.
#[derive(Clone, Debug)]
pub struct DenoiserBuffers {
    pub width: usize,
    pub height: usize,
    /// `width * height * 3`, scene-linear.
    pub color: Vec<f32>,
    pub albedo: Vec<f32>,
    pub normal: Vec<f32>,
}

/// Accumulate one sample into a pixel, applying firefly rejection.
///
/// §3.5's three-layer defence, layer 2: reject a sample whose luminance
/// exceeds `mean + k*sigma` with `k = 8`, **only** once the pixel already has
/// at least 16 samples, and record the discarded energy so the bias can be
/// audited. Rejecting earlier would clip legitimate highlights, which is
/// exactly the look product rendering needs to keep.
pub const FIREFLY_SIGMA_THRESHOLD: f32 = 8.0;
pub const FIREFLY_MIN_SAMPLES: u32 = 16;

impl Pixel {
    pub fn accumulate(&mut self, result: &crate::integrator::PathResult, reject_fireflies: bool) {
        self.accumulate_with(result, reject_fireflies, None)
    }

    /// The same, with §3.5's optional hard clamp on a single sample.
    ///
    /// A different instrument from firefly rejection, and they compose. The
    /// rejection is adaptive — it compares against what this pixel has already
    /// seen, so a genuinely bright pixel keeps its brightness — while a clamp
    /// is a fixed ceiling that removes energy whether or not there was anything
    /// wrong with the sample. §3.5 allows it only as an explicit choice, which
    /// is why it arrives as an `Option` rather than a number with a default.
    ///
    /// The clamp is applied *first*. Clamping afterwards would let a sample
    /// that rejection had already pulled down be pulled down again and
    /// double-counted in the clamped-energy total, which is the one number that
    /// makes the bias auditable.
    pub fn accumulate_with(
        &mut self,
        result: &crate::integrator::PathResult,
        reject_fireflies: bool,
        clamp: Option<f32>,
    ) {
        if self.samples == 0 {
            self.first_instance = result.object_id;
        }
        let mut radiance = result.radiance;
        let original = radiance.luminance();
        // How bright the sample is, for the clamp and the rejection to judge.
        // Its luminance, except that a path carrying one wavelength is
        // coloured by it, and a green wavelength is three times as luminous
        // as a red one carrying the same light: judged by luminance, the
        // green paths through a diamond were clipped hardest and the stone
        // turned lavender. Dividing out the colour's own luminance judges
        // every wavelength by the light it carries, so whether a path is
        // clipped, and by how much, no longer depends on its colour. A
        // spectral path says how luminous its own wavelengths are.
        let hue = match (result.hue, result.wavelength) {
            (Some(hue), _) => hue.max(1e-3),
            (None, Some(lambda)) => tequila_materials::dispersion::weight(lambda).luminance().max(1e-3),
            (None, None) => 1.0,
        };
        let brightness = |radiance: Spectrum| radiance.luminance() / hue;

        if let Some(limit) = clamp {
            let luminance = brightness(radiance);
            if luminance > limit && limit > 0.0 {
                self.clamped_energy += luminance - limit;
                radiance *= limit / luminance.max(1e-9);
            }
        }

        // Scaled as the clamp scaled the sample, so the background never
        // outweighs it.
        let kept = if original > 1e-12 { (radiance.luminance() / original).clamp(0.0, 1.0) } else { 1.0 };
        let background = result.background * kept;

        // A sample whose camera ray met a fibre is not judged at all. A
        // fibre is finer than a pixel, so the few samples of it that catch a
        // glint are the fibre's share of the pixel, not outliers: judged by
        // the pixel's spread, which the surface between the fibres keeps
        // narrow, they were clipped, and fur came out 15% darker than in
        // Cycles.
        if reject_fireflies && !result.fine && self.samples >= FIREFLY_MIN_SAMPLES {
            // Only what the path found by sampling is judged. The background
            // is what the camera sees, the same however often it looks, and a
            // shadow catcher's floor is that background darkened, never
            // brighter than it. Judged with it, a pixel deep in a shadow whose
            // first samples were all blocked had no spread at all, and every
            // lit sample after them was clipped to the dark mean: one pixel in
            // sixteen of a shadow came out black.
            //
            // A spectral path's is kept signed: its wavelengths can register
            // a little below zero in a channel (`spectral::Wavelengths`), and
            // clipped there a blue hero, whose green is negative, was judged
            // brighter than it was and scaled down out of proportion, and a
            // gem in a white world went yellow.
            let sampled = if result.hue.is_some() {
                radiance - background
            } else {
                Spectrum {
                    r: (radiance.r - background.r).max(0.0),
                    g: (radiance.g - background.g).max(0.0),
                    b: (radiance.b - background.b).max(0.0),
                }
            };
            let luminance = brightness(sampled);
            let mean = self.mean().luminance();
            let limit = mean + FIREFLY_SIGMA_THRESHOLD * self.luminance_std_dev();
            if luminance > limit && limit.is_finite() {
                self.clamped_energy += luminance - limit;
                // Scale the sample down rather than dropping it: dropping it
                // would leave the sample count and the sum inconsistent, and
                // scaling preserves the sample's hue.
                radiance = radiance - sampled * (1.0 - limit / luminance.max(1e-9));
            }
        }

        self.sum += radiance;
        self.background_sum += background;
        self.sum_squared_luminance += (radiance.luminance() as f64).powi(2);
        self.albedo_sum += result.albedo;
        self.normal_sum += result.normal;
        // The path's own coverage, not a re-derivation from `escaped`. They
        // agree for ordinary geometry and deliberately do not for a shadow
        // catcher, where a shadow on an invisible floor is partial alpha.
        self.coverage_sum += result.coverage.clamp(0.0, 1.0);
        self.depth_sum += if result.depth.is_finite() {
            result.depth
        } else {
            0.0
        };
        self.samples += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrator::PathResult;
    use tequila_core::math::vec3;

    fn sample(luminance: f32) -> PathResult {
        PathResult {
            radiance: Spectrum::splat(luminance),
            albedo: Spectrum::splat(0.5),
            normal: vec3(0.0, 1.0, 0.0),
            depth: 2.0,
            escaped: false,
            coverage: 1.0,
            clamped_energy: 0.0,
            ..Default::default()
        }
    }

    /// §3.5's optional clamp removes energy on purpose, and the Clamped Energy
    /// pass is what makes that removal auditable. A clamp that did not record
    /// what it took would be bias nobody could measure.
    /// A pixel half covered and half open to a bright sky: with alpha, its
    /// colour is the surface's alone, premultiplied by the half it covers —
    /// not the surface and the sky blended. Blended, glass whose alpha lets
    /// most rays through rendered as the white sky behind it.
    #[test]
    fn with_alpha_a_pixel_carries_its_foreground_alone() {
        let mut pixel = Pixel::default();
        let sky = crate::integrator::PathResult {
            radiance: Spectrum::splat(1.0),
            background: Spectrum::splat(1.0),
            escaped: true,
            coverage: 0.0,
            ..Default::default()
        };
        let surface = crate::integrator::PathResult {
            radiance: Spectrum::splat(0.3),
            coverage: 1.0,
            ..Default::default()
        };
        pixel.accumulate_with(&sky, false, None);
        pixel.accumulate_with(&surface, false, None);
        assert!((pixel.mean().r - 0.65).abs() < 1e-6, "the picture as it looks keeps the sky");
        assert!((pixel.mean_foreground().r - 0.15).abs() < 1e-6, "{:?}", pixel.mean_foreground());
        assert!((pixel.alpha() - 0.5).abs() < 1e-6);
        // Straight, the surface's own colour.
        assert!((pixel.mean_foreground().r / pixel.alpha() - 0.3).abs() < 1e-6);
    }

    #[test]
    fn a_clamp_removes_energy_and_says_how_much() {
        let mut pixel = Pixel::default();
        let bright = sample(10.0);
        pixel.accumulate_with(&bright, false, Some(2.0));

        let kept = pixel.mean().luminance();
        assert!(
            (kept - 2.0).abs() < 1e-3,
            "the clamp let {kept} through against a ceiling of 2"
        );
        let removed = bright.radiance.luminance() - 2.0;
        assert!(
            (pixel.clamped_energy - removed).abs() < 1e-3,
            "it removed {removed} and recorded {}",
            pixel.clamped_energy
        );
    }

    /// A sample under the ceiling is untouched, and nothing is recorded. A
    /// clamp that nudged every sample would be a brightness control.
    #[test]
    fn a_clamp_leaves_ordinary_samples_alone() {
        let mut pixel = Pixel::default();
        pixel.accumulate_with(&sample(0.5), false, Some(2.0));
        assert!((pixel.mean().luminance() - 0.5).abs() < 1e-4);
        assert_eq!(pixel.clamped_energy, 0.0);
    }

    /// The clamp and the rejection are different instruments and they compose.
    /// The clamp runs first, so a sample the rejection would also have pulled
    /// down is not counted twice in the energy total — which is the one number
    /// that makes the bias measurable.
    #[test]
    fn a_clamp_and_firefly_rejection_do_not_double_count() {
        let mut clamped = Pixel::default();
        let mut both = Pixel::default();
        // Enough ordinary samples for the rejection to have a mean to judge
        // against, then one very bright one.
        for _ in 0..FIREFLY_MIN_SAMPLES {
            clamped.accumulate_with(&sample(1.0), false, Some(4.0));
            both.accumulate_with(&sample(1.0), true, Some(4.0));
        }
        clamped.accumulate_with(&sample(100.0), false, Some(4.0));
        both.accumulate_with(&sample(100.0), true, Some(4.0));

        // Both clamped the same amount; only the second also rejected.
        let clamp_took = 100.0 - 4.0;
        assert!((clamped.clamped_energy - clamp_took).abs() < 0.1);
        assert!(
            both.clamped_energy >= clamped.clamped_energy,
            "rejection after the clamp recorded less than the clamp alone"
        );
        // And what rejection took on top is bounded by what was left after the
        // clamp, not by the original sample: counting the original twice would
        // report far more energy removed than existed.
        let extra = both.clamped_energy - clamped.clamped_energy;
        assert!(
            extra <= 4.0 + 1e-3,
            "rejection claims to have removed {extra} from a sample the clamp \
             had already cut to 4"
        );
    }

    #[test]
    fn mean_is_the_running_average() {
        let mut p = Pixel::default();
        for v in [1.0f32, 2.0, 3.0, 4.0] {
            p.accumulate(&sample(v), false);
        }
        assert_eq!(p.samples, 4);
        assert!((p.mean().g - 2.5).abs() < 1e-5);
        assert!((p.alpha() - 1.0).abs() < 1e-6);
        assert!((p.depth_sum / p.samples as f32 - 2.0).abs() < 1e-5);
    }

    /// Accumulation must not depend on the order samples arrive in. This is
    /// the property §4.6 hangs determinism on.
    #[test]
    fn accumulation_is_order_independent_for_the_mean() {
        let values = [0.3f32, 1.7, 0.9, 4.2, 0.1];
        let mut forward = Pixel::default();
        for v in values {
            forward.accumulate(&sample(v), false);
        }
        let mut backward = Pixel::default();
        for v in values.iter().rev() {
            backward.accumulate(&sample(*v), false);
        }
        assert!((forward.mean().g - backward.mean().g).abs() < 1e-5);
    }

    #[test]
    fn escaped_rays_leave_alpha_at_zero() {
        let mut p = Pixel::default();
        let mut background = sample(1.0);
        background.escaped = true;
        background.coverage = 0.0;
        background.depth = f32::INFINITY;
        for _ in 0..4 {
            p.accumulate(&background, false);
        }
        assert_eq!(p.alpha(), 0.0);
        // An infinite depth must not poison the depth AOV.
        assert!(p.depth_sum.is_finite());
    }

    #[test]
    fn half_covered_pixels_get_partial_alpha() {
        let mut p = Pixel::default();
        let mut background = sample(1.0);
        background.escaped = true;
        background.coverage = 0.0;
        p.accumulate(&sample(1.0), false);
        p.accumulate(&background, false);
        assert!((p.alpha() - 0.5).abs() < 1e-6);
    }

    /// Fireflies must be suppressed — but only after enough samples exist to
    /// tell a firefly from a legitimate highlight.
    #[test]
    fn fireflies_are_rejected_only_after_the_warmup() {
        let mut early = Pixel::default();
        early.accumulate(&sample(10_000.0), true);
        assert!(
            early.mean().g > 9_000.0,
            "an early bright sample must not be clipped: it may be a highlight"
        );
        assert_eq!(early.clamped_energy, 0.0);

        let mut warm = Pixel::default();
        for _ in 0..FIREFLY_MIN_SAMPLES {
            warm.accumulate(&sample(1.0), true);
        }
        let before = warm.mean().g;
        warm.accumulate(&sample(10_000.0), true);
        assert!(
            warm.mean().g < before * 2.0,
            "firefly was not suppressed: mean went {before} -> {}",
            warm.mean().g
        );
        assert!(
            warm.clamped_energy > 0.0,
            "rejected energy must be recorded so the bias is auditable"
        );
    }

    /// What the camera sees of the background is not sampled, and is never a
    /// firefly. A shadow catcher's floor is that background darkened by
    /// coverage, a sample at a time: deep in a shadow the first sixteen
    /// samples may all be blocked, leaving no spread at all, and every lit
    /// sample after them was clipped to the dark mean. One pixel in sixteen
    /// of a shadow came out black.
    #[test]
    fn the_background_is_never_rejected_as_a_firefly() {
        let behind = Spectrum::splat(0.3);
        let blocked = PathResult { coverage: 1.0, ..Default::default() };
        let lit = PathResult { radiance: behind, background: behind, ..Default::default() };
        let mut pixel = Pixel::default();
        for _ in 0..FIREFLY_MIN_SAMPLES {
            pixel.accumulate(&blocked, true);
        }
        for i in 0..64 {
            pixel.accumulate(if i % 4 == 0 { &lit } else { &blocked }, true);
        }
        let expected = 0.3 * 16.0 / 80.0;
        assert!(
            (pixel.mean().g - expected).abs() < 1e-5,
            "the floor came out {} where its samples average {expected}",
            pixel.mean().g
        );
        assert_eq!(pixel.clamped_energy, 0.0);
        assert!((pixel.mean_foreground().g).abs() < 1e-6, "{:?}", pixel.mean_foreground());

        // The same light found by sampling, on a surface rather than seen
        // past it, is still judged: rejection is not switched off.
        let mut surface = Pixel::default();
        for _ in 0..FIREFLY_MIN_SAMPLES {
            surface.accumulate(&blocked, true);
        }
        surface.accumulate(&PathResult { radiance: behind, ..Default::default() }, true);
        assert!(surface.clamped_energy > 0.0);
    }

    /// A pixel with genuine variation must not have that variation clipped
    /// away — the rejector keys on 8 sigma precisely so it does not flatten
    /// legitimate contrast.
    #[test]
    fn moderate_variation_survives_firefly_rejection() {
        let mut p = Pixel::default();
        let values: Vec<f32> = (0..64).map(|i| 1.0 + (i % 8) as f32 * 0.5).collect();
        for v in &values {
            p.accumulate(&sample(*v), true);
        }
        let expected: f32 = values.iter().sum::<f32>() / values.len() as f32;
        assert!(
            (p.mean().g - expected).abs() < 1e-3,
            "clipped legitimate variation: {} vs {expected}",
            p.mean().g
        );
        assert_eq!(p.clamped_energy, 0.0);
    }

    #[test]
    fn relative_error_falls_as_samples_accumulate() {
        let mut p = Pixel::default();
        let mut rng = tequila_core::rng::Pcg32::new(1, 1);
        let mut previous = f32::INFINITY;
        for i in 1..=4096u32 {
            p.accumulate(&sample(1.0 + rng.next_f32()), false);
            if i.is_power_of_two() && i >= 16 {
                let e = p.relative_error();
                assert!(e <= previous, "error rose at {i} samples");
                previous = e;
            }
        }
        assert!(previous < 0.01, "error did not converge: {previous}");
    }

    #[test]
    fn a_constant_pixel_has_zero_error() {
        let mut p = Pixel::default();
        for _ in 0..32 {
            p.accumulate(&sample(2.0), false);
        }
        assert!(p.relative_error() < 1e-3, "{}", p.relative_error());
    }

    #[test]
    fn region_clear_leaves_the_rest_untouched() {
        let mut fb = Framebuffer::new(8, 8);
        for y in 0..8 {
            for x in 0..8 {
                fb.pixel_mut(x, y).accumulate(&sample(1.0), false);
            }
        }
        fb.clear_region(2, 2, 5, 5);
        for y in 0..8 {
            for x in 0..8 {
                let inside = (2..5).contains(&x) && (2..5).contains(&y);
                let samples = fb.pixel(x, y).samples;
                if inside {
                    assert_eq!(samples, 0, "({x},{y}) should have been cleared");
                } else {
                    assert_eq!(samples, 1, "({x},{y}) should have been kept");
                }
            }
        }
    }

    /// The HUD reports the *minimum* live sample count, not the maximum: with
    /// adaptive sampling on, the maximum overstates convergence.
    #[test]
    fn sample_counts_report_the_least_converged_pixel() {
        let mut fb = Framebuffer::new(4, 1);
        for _ in 0..10 {
            fb.pixel_mut(0, 0).accumulate(&sample(1.0), false);
        }
        fb.pixel_mut(1, 0).accumulate(&sample(1.0), false);
        assert_eq!(fb.min_samples(), 0);
        assert_eq!(fb.max_samples(), 10);
    }

    /// Adaptive sampling must retire converged pixels but keep sampling next
    /// to noisy ones, or edges halt prematurely (§4.4).
    #[test]
    fn adaptive_retirement_keeps_working_next_to_noisy_pixels() {
        let mut fb = Framebuffer::new(3, 1);
        // Two quiet pixels and one very noisy one, all past the warmup.
        let mut rng = tequila_core::rng::Pcg32::new(2, 2);
        for _ in 0..64 {
            fb.pixel_mut(0, 0).accumulate(&sample(1.0), false);
            fb.pixel_mut(1, 0).accumulate(&sample(1.0), false);
            fb.pixel_mut(2, 0)
                .accumulate(&sample(rng.next_f32() * 100.0), false);
        }
        fb.update_adaptive_retirement(0.01, 16);
        assert!(
            fb.pixel(0, 0).retired,
            "a quiet, isolated pixel must retire"
        );
        assert!(
            !fb.pixel(1, 0).retired,
            "a pixel bordering a noisy one must keep sampling"
        );
        assert!(
            !fb.pixel(2, 0).retired,
            "the noisy pixel must keep sampling"
        );
        assert!((fb.converged_fraction() - 1.0 / 3.0).abs() < 1e-6);
    }

    /// A path that took a wavelength is coloured by it, and a green one is
    /// three times as luminous as a red one carrying the same light. Judged
    /// by luminance, rejection clipped the green paths through a diamond far
    /// harder than the red and blue, and the stone came out lavender. Here a
    /// neutral scene seen through dispersion, bright paths and all, must
    /// average to neutral with rejection on.
    #[test]
    fn rejection_does_not_tint_a_dispersive_pixel() {
        let mut pixel = Pixel::default();
        // The bright paths take their wavelengths from a sequence of their
        // own, so they are spread across the spectrum however many have
        // arrived: the rejection limit settles as they do, and a render draws
        // each wavelength independently of when its sample lands.
        for i in 0..20_000u32 {
            let (u, brightness) = if i % 97 == 50 {
                ((i / 97) as f32 * 0.618_034, 30.0)
            } else {
                (i as f32 * 0.754_878, 1.0)
            };
            let lambda = tequila_materials::dispersion::sample(u.fract());
            let result = PathResult {
                radiance: tequila_materials::dispersion::weight(lambda) * brightness,
                wavelength: Some(lambda),
                ..sample(0.0)
            };
            pixel.accumulate(&result, true);
        }
        assert!(pixel.clamped_energy > 0.0, "the bright paths should have been rejected");
        let c = pixel.mean().to_linear_srgb();
        let (high, low) = (c.x.max(c.y).max(c.z), c.x.min(c.y).min(c.z));
        assert!(
            (high - low) / high < 0.03,
            "a neutral scene came out ({:.3}, {:.3}, {:.3}) through dispersion",
            c.x,
            c.y,
            c.z
        );
    }

    #[test]
    fn resize_discards_stale_accumulation() {
        let mut fb = Framebuffer::new(4, 4);
        fb.pixel_mut(0, 0).accumulate(&sample(1.0), false);
        fb.resize(8, 8);
        assert_eq!((fb.width, fb.height), (8, 8));
        assert_eq!(fb.pixel(0, 0).samples, 0);
        // Resizing to the same size must not throw work away.
        fb.pixel_mut(0, 0).accumulate(&sample(1.0), false);
        fb.resize(8, 8);
        assert_eq!(fb.pixel(0, 0).samples, 1);
    }

    #[test]
    fn denoiser_buffers_have_the_expected_layout() {
        let mut fb = Framebuffer::new(2, 3);
        for y in 0..3 {
            for x in 0..2 {
                fb.pixel_mut(x, y).accumulate(&sample(1.0), false);
            }
        }
        let b = fb.to_denoiser_buffers();
        assert_eq!(b.color.len(), 2 * 3 * 3);
        assert_eq!(b.albedo.len(), b.color.len());
        assert_eq!(b.normal.len(), b.color.len());
        // Normals must be unit length, or the denoiser's guide is meaningless.
        for n in b.normal.chunks(3) {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-4, "normal guide not normalized");
        }
        // Albedo must be bounded: the denoiser expects [0,1].
        assert!(b.albedo.iter().all(|&v| (0.0..=1.0).contains(&v)));
    }
}
