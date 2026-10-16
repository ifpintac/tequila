//! Image-based lighting: the equirectangular environment and its importance
//! sampler.
//!
//! `TEQUILA_SPEC.md` §1.5 calls HDRI lighting "the second feature (after
//! drag-drop materials) that defines the product" — it is why a render looks
//! photographic within seconds of loading a scene.
//!
//! `docs/01-RENDERER-CORE.md` §5.2 is emphatic that importance sampling here is
//! **mandatory, not an optimization**: an HDRI with a small bright sun is a
//! high-variance light source, and "without it every outdoor scene is
//! unusable".
//!
//! Two details from §5.2 that are easy to get wrong and expensive to
//! discover:
//!
//! - Rows must be weighted by `sin(theta)` to account for the solid-angle
//!   distortion of the equirectangular parameterization. Forgetting it is
//!   "the single most common bug in environment sampling" and over-weights the
//!   poles.
//! - The distribution must describe the function actually being integrated.
//!   Radiance is read *bilinearly*, so a CDF built from raw texel values
//!   describes a different function than the one it is sampling, and the
//!   mismatch is worst exactly where it hurts: a small bright sun. See
//!   [`Environment::new`].
//! - Sampling happens in the environment's **local** space and the result is
//!   rotated into the world. That makes rotation free — no CDF rebuild — which
//!   matters because rotation is the primary tool for placing highlights.

use tequila_core::math::{Mat4, Vec2, Vec3};
use tequila_core::sampling::{direction_to_equirect, equirect_to_direction};
use tequila_core::spectrum::Spectrum;
use tequila_texture::Image;
use std::f32::consts::PI;
use std::sync::Arc;

/// What the camera sees when a ray escapes the scene.
///
/// `TEQUILA_SPEC.md` §1.5: the lighting environment, a solid colour, or a
/// backplate image decoupled from the lighting.
#[derive(Clone, Debug)]
pub enum Background {
    /// Show the lighting environment itself (hotkey `E`).
    Environment,
    /// A solid colour (hotkey `C`).
    Color(Spectrum),
    /// A backplate, which lights nothing (hotkey `B`).
    ///
    /// Mapped to the film rather than to a direction, because that is what a
    /// backplate *is*: a photograph the product is composited into, which must
    /// not swim about when the camera turns. It is stretched to the frame, so
    /// the render's aspect ratio is expected to match the plate's — which in a
    /// backplate workflow it does, since the plate is what set the resolution.
    Backplate { image: Arc<Image> },
}

impl Background {
    /// What a camera ray that escaped the scene shows.
    ///
    /// Only ever called for a path that reached here without a non-specular
    /// bounce — directly, or through glass and mirrors. Everything else still
    /// sees the lighting environment, which is the entire point of the
    /// control: §1.5's backplate is "decoupled from the lighting", so swapping
    /// it changes the picture behind the product and not one photon landing on
    /// it.
    pub fn radiance(&self, environment: &Environment, origin: Vec3, direction: Vec3, film: Vec2) -> Spectrum {
        match self {
            Background::Environment => {
                environment.background_radiance_from(origin, direction)
            }
            Background::Color(color) => *color,
            Background::Backplate { image } => {
                Spectrum::from_rendering_rgb(image.sample_bilinear_wrap_u(
                    film.x.clamp(0.0, 1.0),
                    film.y.clamp(0.0, 1.0),
                ))
            }
        }
    }

    /// Whether this mode leaves the environment showing.
    pub fn is_environment(&self) -> bool {
        matches!(self, Background::Environment)
    }
}

/// A sampled direction on the environment.
#[derive(Clone, Copy, Debug)]
pub struct EnvironmentSample {
    /// World-space direction towards the light.
    pub direction: Vec3,
    pub radiance: Spectrum,
    /// Solid-angle PDF.
    pub pdf: f32,
    /// How far away the dome is in that direction, or `f32::INFINITY` for a
    /// dome that is infinitely distant. A shadow ray has to stop there: with a
    /// finite dome, geometry beyond it is behind the sky.
    pub distance: f32,
}

/// The surface the environment image is mapped onto (§1.5's Size, Height and
/// Flatten Ground).
///
/// An infinite dome is the usual assumption and the one that makes lighting
/// independent of where a part stands. A *finite* one is what the Size control
/// means, and it is not cosmetic: with a dome a few times the size of the
/// product, moving the product across the floor changes which part of the sky
/// it sees, which is exactly the parallax that makes a large softbox read as a
/// large softbox rather than as a flat gradient.
///
/// Every lookup — radiance, sampling and density — goes through
/// [`Dome::surface_point`], so the three cannot disagree about where the sky
/// is. That matters most for `flatten`, which changes the shape of the dome
/// rather than the image on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dome {
    /// Radius in scene units. `None` is infinitely far away.
    pub radius: Option<f32>,
    /// Height of the dome's centre above the ground plane.
    pub height: f32,
    /// Replace the below-horizon part of the dome with the ground plane, so
    /// the floor does not visibly curve away (`G`).
    pub flatten: bool,
}

impl Default for Dome {
    fn default() -> Self {
        Dome {
            radius: None,
            height: 0.0,
            flatten: false,
        }
    }
}

/// Where a ray meets the dome, and the surface normal there.
#[derive(Clone, Copy, Debug)]
pub struct DomeHit {
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

impl Dome {
    pub fn is_infinite(&self) -> bool {
        self.radius.is_none_or(|r| !(r > 0.0))
    }

    fn centre(&self) -> Vec3 {
        Vec3 {
            x: 0.0,
            y: self.height,
            z: 0.0,
        }
    }

    /// Where the ray `(origin, direction)` meets the dome.
    ///
    /// `None` for an infinite dome, where there is no point to speak of and
    /// the direction is the whole answer.
    pub fn surface_point(&self, origin: Vec3, direction: Vec3) -> Option<DomeHit> {
        let radius = self.radius.filter(|r| *r > 0.0)?;
        let centre = self.centre();

        // Nearest positive root of |o + t·d - c|² = r². The shading point is
        // inside the dome in every sane scene, which makes exactly one root
        // positive; the max() is what keeps a point outside it from selecting
        // the near side and mapping the sky inside out.
        let oc = origin - centre;
        let b = oc.dot(direction);
        let c = oc.dot(oc) - radius * radius;
        let discriminant = b * b - c;
        if discriminant < 0.0 {
            return None;
        }
        let root = discriminant.sqrt();
        let t_sphere = {
            let far = -b + root;
            let near = -b - root;
            if near > 1e-4 {
                near
            } else if far > 1e-4 {
                far
            } else {
                return None;
            }
        };

        if self.flatten && direction.y < -1e-6 {
            // The ground plane stands in for the lower cap. Only within the
            // dome's own footprint: past that the plane is further away than
            // the sphere and taking it would fold the horizon back on itself.
            let t_plane = -origin.y / direction.y;
            if t_plane > 1e-4 && t_plane < t_sphere {
                let point = origin + direction * t_plane;
                let offset = point - centre;
                if (offset.x * offset.x + offset.z * offset.z) <= radius * radius {
                    return Some(DomeHit {
                        point,
                        normal: -Vec3::Y,
                        distance: t_plane,
                    });
                }
            }
        }

        let point = origin + direction * t_sphere;
        Some(DomeHit {
            // Inward-facing, since the scene is inside the dome.
            normal: (centre - point) / radius,
            point,
            distance: t_sphere,
        })
    }

    /// The direction the image is read along, for a ray from `origin`.
    ///
    /// This is the whole of what Size, Height and Flatten do to a lookup:
    /// find where the ray meets the dome, then read the image in the direction
    /// of that point *from the dome's centre*. An infinite dome skips both
    /// steps, which is why it costs nothing.
    pub fn lookup(&self, origin: Vec3, direction: Vec3) -> (Vec3, f32) {
        match self.surface_point(origin, direction) {
            Some(hit) => {
                let from_centre = hit.point - self.centre();
                (
                    from_centre.try_normalize().unwrap_or(direction),
                    hit.distance,
                )
            }
            None => (direction, f32::INFINITY),
        }
    }

    /// The point on the dome that is seen from its centre along `local`, and
    /// the density conversion from that direction to solid angle at `origin`.
    ///
    /// The inverse of [`Self::lookup`], which is what sampling needs: the CDF
    /// draws a direction as seen from the centre, and the estimator needs the
    /// direction and density as seen from the shading point.
    fn from_centre(&self, origin: Vec3, local: Vec3) -> Option<(Vec3, f32, f32)> {
        let radius = self.radius.filter(|r| *r > 0.0)?;
        let centre = self.centre();

        // The same two surfaces as `surface_point`, inverted.
        let (point, normal) = if self.flatten && local.y < -1e-6 {
            let t = -centre.y / local.y;
            let candidate = centre + local * t;
            if t > 0.0 && (candidate.x * candidate.x + candidate.z * candidate.z) <= radius * radius
            {
                (candidate, -Vec3::Y)
            } else {
                (centre + local * radius, -local)
            }
        } else {
            (centre + local * radius, -local)
        };

        let offset = point - origin;
        let distance = offset.length();
        if distance < 1e-4 {
            return None;
        }
        let direction = offset / distance;

        // Solid angle at the centre -> area on the dome -> solid angle at the
        // shading point. The first factor is `d_c² / |local·n|`, the second
        // `|direction·n| / d²`; for a sphere `|local·n|` is 1 and the pair
        // collapses to the familiar `r²`.
        let from_centre_distance = (point - centre).length().max(1e-6);
        let cos_centre = local.dot(normal).abs().max(1e-6);
        let cos_surface = direction.dot(normal).abs().max(1e-6);
        let jacobian = (cos_centre / (from_centre_distance * from_centre_distance))
            * (distance * distance / cos_surface);
        Some((direction, jacobian, distance))
    }
}

/// An equirectangular environment light with a 2D CDF for importance sampling.
#[derive(Clone, Debug)]
pub struct Environment {
    /// In the rendering space (ACEScg), as everything that paints it
    /// writes: a loaded HDRI is converted as it loads.
    image: Arc<Image>,
    /// Local-to-world rotation. Rotation is applied to directions, never to
    /// the image, so the CDF survives it untouched.
    rotation: Mat4,
    inverse_rotation: Mat4,
    /// What the camera sees, when that differs from what is sampled. `None`
    /// means the two are the same, which is the usual case.
    camera_image: Option<Arc<Image>>,
    intensity: f32,
    /// Marginal CDF over rows, `height + 1` entries.
    marginal_cdf: Vec<f32>,
    /// Conditional CDF per row, `height * (width + 1)` entries.
    conditional_cdf: Vec<f32>,
    /// Per-texel luminance of the *reconstructed* image (see
    /// [`Environment::new`]), which is what both the CDF and [`Self::pdf`]
    /// are built from.
    luminance: Vec<f32>,
    /// `sum of luminance * sin(theta)` over every texel: the CDF's
    /// normalization constant.
    weighted_sum: f32,
    width: usize,
    height: usize,
    /// The surface the image is mapped onto. Not part of the CDF, so changing
    /// it is free.
    dome: Dome,
}

impl Environment {
    /// Build an environment from an equirectangular image.
    ///
    /// Cost is `O(width * height)`; §9.4 requires this to happen off the
    /// render thread, and §5.2 budgets ~10 ms for a 4K map.
    pub fn new(image: Arc<Image>, intensity: f32) -> Self {
        let (width, height) = (image.width.max(1), image.height.max(1));

        // Build the distribution from the *reconstruction filter's* view of
        // the image, not from the raw texels.
        //
        // `radiance` reads bilinearly, so the function being integrated is the
        // bilinear reconstruction, and the sampler's density has to describe
        // that same function or the estimator's variance explodes. Integrating
        // a separable tent over one texel's footprint gives the 1D weights
        // `(1/8, 3/4, 1/8)`; their outer product is the 3x3 kernel below. The
        // result is that the probability assigned to a texel equals the
        // integral of the radiance actually returned across it.
        //
        // Concretely, for a one-texel sun this changes the worst-case
        // radiance-to-density ratio from unbounded — the sampler can land a
        // hair inside a neighbouring texel whose density is a thousand times
        // too small — to under 2.
        // Per texel once, then filtered: nine luminance conversions per texel
        // was most of the cost of a rebuild.
        use rayon::prelude::*;
        let mut texel_luminance = vec![0.0f32; width * height];
        texel_luminance.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                *out = Spectrum::from_rendering_rgb(image.texel(x, y)).luminance().max(0.0);
            }
        });
        let mut luminance = vec![0.0f32; width * height];
        luminance.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
            for (x, out) in row.iter_mut().enumerate() {
                let mut acc = 0.0f32;
                for (dy, wy) in [(-1i64, 0.125f32), (0, 0.75), (1, 0.125)] {
                    for (dx, wx) in [(-1i64, 0.125f32), (0, 0.75), (1, 0.125)] {
                        // Wrap horizontally, clamp vertically: the same
                        // addressing the bilinear lookup uses.
                        let sx = (x as i64 + dx).rem_euclid(width as i64) as usize;
                        let sy = (y as i64 + dy).clamp(0, height as i64 - 1) as usize;
                        acc += wx * wy * texel_luminance[sy * width + sx];
                    }
                }
                *out = acc;
            }
        });

        let mut conditional_cdf = vec![0.0f32; height * (width + 1)];
        let mut marginal_cdf = vec![0.0f32; height + 1];

        let row_integrals: Vec<f32> = conditional_cdf
            .par_chunks_mut(width + 1)
            .enumerate()
            .map(|(y, row)| {
                // Solid angle per texel shrinks towards the poles as
                // sin(theta). Omitting this weight is the classic
                // environment-sampling bug.
                let theta = (y as f32 + 0.5) / height as f32 * PI;
                let sin_theta = theta.sin();
                let mut running = 0.0f32;
                for x in 0..width {
                    running += luminance[y * width + x] * sin_theta;
                    row[x + 1] = running;
                }
                // Normalize the row. A fully black row leaves its CDF at zero
                // and is simply never selected, which the marginal CDF
                // handles.
                if running > 0.0 {
                    let inv = 1.0 / running;
                    for entry in row.iter_mut().skip(1) {
                        *entry *= inv;
                    }
                }
                running
            })
            .collect();

        let mut running = 0.0f32;
        for (y, integral) in row_integrals.iter().enumerate() {
            running += integral;
            marginal_cdf[y + 1] = running;
        }
        let total = running;
        if total > 0.0 {
            let inv = 1.0 / total;
            for entry in marginal_cdf.iter_mut().skip(1) {
                *entry *= inv;
            }
        }

        Environment {
            image,
            rotation: Mat4::IDENTITY,
            inverse_rotation: Mat4::IDENTITY,
            camera_image: None,
            intensity,
            marginal_cdf,
            conditional_cdf,
            luminance,
            weighted_sum: total,
            width,
            height,
            dome: Dome::default(),
        }
    }

    /// A uniform environment of the given radiance. The white-furnace
    /// reference case, and a sensible default for a scene with no HDRI.
    pub fn uniform(radiance: Spectrum) -> Self {
        Environment::new(
            Arc::new(Image::constant(16, 8, radiance.to_rendering_rgb())),
            1.0,
        )
    }

    /// Spin the environment about the vertical axis.
    ///
    /// Free: it rotates directions, not pixels, so no CDF rebuild is needed.
    /// That is what makes dragging the rotation slider interactive.
    pub fn set_rotation_y(&mut self, radians: f32) {
        self.rotation = Mat4::rotation(Vec3::Y, radians);
        self.inverse_rotation = Mat4::rotation(Vec3::Y, -radians);
    }

    pub fn set_intensity(&mut self, intensity: f32) {
        self.intensity = intensity.max(0.0);
    }

    pub fn set_dome(&mut self, dome: Dome) {
        self.dome = dome;
    }

    pub fn dome(&self) -> Dome {
        self.dome
    }

    pub fn intensity(&self) -> f32 {
        self.intensity
    }

    pub fn image(&self) -> &Arc<Image> {
        &self.image
    }

    /// The image's local-to-world rotation, for a renderer that maps the
    /// image itself (Cycles) and so must turn it the same way.
    pub fn rotation(&self) -> Mat4 {
        self.rotation
    }

    /// The image the camera sees, when it differs from the one that lights.
    pub fn camera_image(&self) -> Option<&Arc<Image>> {
        self.camera_image.as_ref()
    }

    /// Radiance arriving at `origin` from a world-space direction.
    ///
    /// The same as [`Self::radiance`] for an infinite dome, and the version
    /// every caller that has a position should use: with a finite dome the two
    /// differ, and that difference *is* the Size control.
    /// What the *camera* sees in this direction, which is not always what a
    /// bounce sees.
    ///
    /// A pin marked not to show in the background is painted into the image
    /// the renderer samples and left out of this one. Keeping the difference
    /// here rather than in `Background` is what lets a chrome surface go on
    /// reflecting a softbox while the backdrop behind it stays clean --
    /// switching the background to a flat colour cannot do that, because a
    /// path that has only ever bounced off mirrors is one the camera sees
    /// along and takes the background too.
    pub fn background_radiance_from(&self, origin: Vec3, world_direction: Vec3) -> Spectrum {
        let Some(image) = &self.camera_image else {
            return self.radiance_from(origin, world_direction);
        };
        let direction = if self.dome.is_infinite() {
            world_direction
        } else {
            self.dome.lookup(origin, world_direction).0
        };
        let local = self.inverse_rotation.transform_vector(direction);
        let uv = tequila_core::sampling::direction_to_equirect(local);
        let rgb = image.sample_bilinear_wrap_u(uv.x, uv.y);
        Spectrum::from_rendering_rgb(rgb) * self.intensity
    }

    /// Give the camera a different image from the one that is sampled.
    pub fn set_camera_image(&mut self, image: Option<Arc<Image>>) {
        self.camera_image = image;
    }

    pub fn radiance_from(&self, origin: Vec3, world_direction: Vec3) -> Spectrum {
        if self.dome.is_infinite() {
            return self.radiance(world_direction);
        }
        let (direction, _) = self.dome.lookup(origin, world_direction);
        self.radiance(direction)
    }

    /// Density of [`Self::sample_from`] having produced `world_direction` at
    /// `origin`.
    pub fn pdf_from(&self, origin: Vec3, world_direction: Vec3) -> f32 {
        if self.dome.is_infinite() {
            return self.pdf(world_direction);
        }
        let Some(hit) = self.dome.surface_point(origin, world_direction) else {
            return 0.0;
        };
        let Some(local) = (hit.point - self.dome.centre()).try_normalize() else {
            return 0.0;
        };
        let Some((_, jacobian, _)) = self.dome.from_centre(origin, local) else {
            return 0.0;
        };
        self.pdf(local) / jacobian.max(1e-12)
    }

    /// Importance-sample a direction as seen from `origin`.
    pub fn sample_from(&self, origin: Vec3, u: Vec2) -> Option<EnvironmentSample> {
        if self.dome.is_infinite() {
            return self.sample(u);
        }
        let centre_sample = self.sample(u)?;
        let (direction, jacobian, distance) =
            self.dome.from_centre(origin, centre_sample.direction)?;
        let pdf = centre_sample.pdf / jacobian.max(1e-12);
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(pdf > 0.0) {
            return None;
        }
        Some(EnvironmentSample {
            direction,
            radiance: centre_sample.radiance,
            pdf,
            distance,
        })
    }

    /// Radiance arriving from a world-space direction, with the dome
    /// infinitely far away.
    pub fn radiance(&self, world_direction: Vec3) -> Spectrum {
        let local = self.inverse_rotation.transform_vector(world_direction);
        let uv = direction_to_equirect(local);
        let rgb = self.image.sample_bilinear_wrap_u(uv.x, uv.y);
        Spectrum::from_rendering_rgb(rgb) * self.intensity
    }

    /// Solid-angle PDF of [`Self::sample`] having produced `world_direction`.
    ///
    /// Required by MIS: when a BSDF sample happens to escape into the
    /// environment, the integrator needs the probability the light-sampling
    /// strategy would have assigned it (§3.1).
    pub fn pdf(&self, world_direction: Vec3) -> f32 {
        if self.weighted_sum <= 0.0 {
            return 0.0;
        }
        let local = self.inverse_rotation.transform_vector(world_direction);
        let uv = direction_to_equirect(local);
        let x = ((uv.x * self.width as f32) as usize).min(self.width - 1);
        let y = ((uv.y * self.height as f32) as usize).min(self.height - 1);
        // The filtered luminance, matching what the CDF was built from. Using
        // the raw texel here instead would make `pdf` describe a different
        // distribution than `sample` draws from.
        let luminance = self.luminance[y * self.width + x];

        // Derivation. The discrete probability of selecting texel (x, y) is
        // `L * sin(theta) / weighted_sum`. Each texel covers `1 / (W * H)` of
        // the unit square, so the image-space density is that times `W * H`.
        // Converting to solid angle divides by the Jacobian of the
        // equirectangular map, `dw / (du dv) = 2 * pi^2 * sin(theta)`.
        //
        // The two `sin(theta)` factors cancel exactly — the row weighting
        // appears in the CDF construction and nowhere else. Writing the
        // cancellation out is worth it: carrying a spurious `sin(theta)` here
        // is the same class of bug as omitting it there, and it is invisible
        // except as a slow brightness drift towards the poles.
        luminance * (self.width * self.height) as f32 / (self.weighted_sum * 2.0 * PI * PI)
    }

    /// Importance-sample a direction.
    pub fn sample(&self, u: Vec2) -> Option<EnvironmentSample> {
        if self.weighted_sum <= 0.0 {
            return None;
        }

        // Marginal: pick a row.
        let y = find_interval(&self.marginal_cdf, u.y);
        let dy = segment_offset(&self.marginal_cdf, y, u.y);

        // Conditional: pick a column within it.
        let base = y * (self.width + 1);
        let row = &self.conditional_cdf[base..base + self.width + 1];
        let x = find_interval(row, u.x);
        let dx = segment_offset(row, x, u.x);

        let uv = Vec2 {
            x: (x as f32 + dx) / self.width as f32,
            y: (y as f32 + dy) / self.height as f32,
        };
        let local = equirect_to_direction(uv);
        let direction = self.rotation.transform_vector(local).normalize();

        let pdf = self.pdf(direction);
        // Negated deliberately, so that a NaN density is rejected too.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(pdf > 0.0) {
            return None;
        }

        Some(EnvironmentSample {
            direction,
            radiance: self.radiance(direction),
            pdf,
            distance: f32::INFINITY,
        })
    }
}

/// Separate dark from bright, about the image's own mean luminance (§1.5).
///
/// Baked into the image rather than applied at lookup time, because contrast
/// changes the *function* being importance-sampled: leaving the CDF describing
/// the original would mean sampling one distribution and evaluating another,
/// which is the high-variance failure mode §5.2 calls out. So this runs before
/// [`Environment::new`] and the pyramid is built from the result.
///
/// The pivot is the mean of the log luminance — the photographic mid-tone —
/// rather than the arithmetic mean, which in an HDRI is dominated by whatever
/// small bright region is the sun and would make contrast behave like a
/// brightness control.
pub fn apply_contrast(image: &Image, contrast: f32) -> Image {
    if (contrast - 1.0).abs() < 1e-4 {
        return image.clone();
    }
    let contrast = contrast.max(0.01);

    let mut log_sum = 0.0f64;
    let mut counted = 0u64;
    for y in 0..image.height {
        for x in 0..image.width {
            let luminance = image.texel_spectrum(x, y).luminance();
            if luminance > 1e-6 {
                log_sum += (luminance as f64).ln();
                counted += 1;
            }
        }
    }
    if counted == 0 {
        return image.clone();
    }
    let pivot = (log_sum / counted as f64).exp() as f32;

    let mut out = image.clone();
    let channels = out.channels;
    for y in 0..out.height {
        for x in 0..out.width {
            let rgb = out.texel(x, y);
            let luminance = Spectrum::from_rendering_rgb(rgb).luminance();
            if luminance <= 1e-6 {
                continue;
            }
            // Scaled by the luminance ratio rather than raised per channel, so
            // increasing contrast does not also saturate every colour in the
            // sky.
            let scaled = pivot * (luminance / pivot).powf(contrast);
            let adjusted = rgb * (scaled / luminance);
            let base = (y * out.width + x) * channels;
            for (offset, value) in [adjusted.x, adjusted.y, adjusted.z]
                .into_iter()
                .enumerate()
                .take(channels.min(3))
            {
                out.data[base + offset] = value;
            }
        }
    }
    out
}

/// Index of the CDF interval containing `u`, by binary search.
fn find_interval(cdf: &[f32], u: f32) -> usize {
    let n = cdf.len() - 1;
    let mut lo = 0usize;
    let mut hi = n;
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if cdf[mid] <= u {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo.min(n - 1)
}

/// Position of `u` within CDF interval `i`, in `[0, 1)`.
///
/// Reusing the sample this way rather than drawing a fresh one keeps the
/// stratification of the input: a low-discrepancy sample stays low-discrepancy
/// through the warp.
fn segment_offset(cdf: &[f32], i: usize, u: f32) -> f32 {
    let width = cdf[i + 1] - cdf[i];
    if width > 0.0 {
        ((u - cdf[i]) / width).clamp(0.0, 1.0 - f32::EPSILON)
    } else {
        0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_core::math::{vec2, vec3};
    use tequila_core::rng::Pcg32;
    use tequila_core::sampling::sample_uniform_sphere;
    use tequila_texture::image::ColorSpace;

    fn constant_env(radiance: f32) -> Environment {
        Environment::new(
            Arc::new(Image::constant(64, 32, Vec3::splat(radiance))),
            1.0,
        )
    }

    /// An environment's pixels are in the rendering space already: a loaded
    /// HDRI is converted as it loads (`tequila_scene::environment`), and the
    /// sky and the pins paint in it. Looking them up must not convert them a
    /// second time. It did, and every HDRI came out a primaries conversion
    /// short of its own colour — a lawn greyer and a sky paler than the file,
    /// next to the same file rendered in Blender.
    #[test]
    fn an_environment_shows_its_pixels_as_they_are() {
        let colour = vec3(0.2, 0.5, 0.1);
        let env = Environment::new(Arc::new(Image::constant(16, 8, colour)), 1.0);
        for direction in [vec3(0.0, 0.0, -1.0), vec3(1.0, 0.0, 0.0), vec3(0.3, 0.8, 0.2).normalize()] {
            let lit = env.radiance(direction).to_rendering_rgb();
            let seen = env.background_radiance_from(Vec3::ZERO, direction).to_rendering_rgb();
            assert!((lit - colour).length() < 1e-4, "lit {lit:?}, pixels {colour:?}");
            assert!((seen - colour).length() < 1e-4, "seen {seen:?}, pixels {colour:?}");
        }
        let radiance = Spectrum { r: 0.3, g: 0.6, b: 0.9 };
        let uniform = Environment::uniform(radiance).radiance(vec3(0.0, 1.0, 0.0));
        assert!((uniform.to_rendering_rgb() - radiance.to_rendering_rgb()).length() < 1e-4, "{uniform:?}");
    }

    fn finite_dome(radius: f32) -> Dome {
        Dome {
            radius: Some(radius),
            height: 0.0,
            flatten: false,
        }
    }

    /// The consistency that makes MIS correct: the density `sample_from`
    /// reports must be the one `pdf_from` would assign to the direction it
    /// produced. If they disagree the weights are computed against a
    /// distribution nothing drew from, and the error shows up as an energy
    /// shift no amount of sampling removes.
    #[test]
    fn a_finite_dome_agrees_with_its_own_density() {
        let mut env = sun_env(20, 8, 500.0);
        env.set_dome(finite_dome(50.0));
        let mut rng = Pcg32::new(7, 11);
        let origins = [
            Vec3::ZERO,
            vec3(12.0, 3.0, -7.0),
            vec3(-30.0, 1.0, 20.0),
        ];
        for origin in origins {
            for _ in 0..2000 {
                let u = vec2(rng.next_f32(), rng.next_f32());
                let Some(sample) = env.sample_from(origin, u) else {
                    continue;
                };
                let recovered = env.pdf_from(origin, sample.direction);
                let ratio = recovered / sample.pdf;
                assert!(
                    (ratio - 1.0).abs() < 2e-2,
                    "at {origin:?}: sample said {} and pdf says {recovered}",
                    sample.pdf
                );
            }
        }
    }

    /// A dome far larger than the scene has to behave like an infinite one.
    /// This is the limit the Size control slides towards, and a mapping that
    /// did not converge to it would make the default setting a special case.
    #[test]
    fn a_very_large_dome_matches_an_infinite_one() {
        let plain = sun_env(20, 8, 500.0);
        let mut large = sun_env(20, 8, 500.0);
        large.set_dome(finite_dome(1.0e6));

        let mut rng = Pcg32::new(3, 5);
        for _ in 0..500 {
            let direction = sample_uniform_sphere(vec2(rng.next_f32(), rng.next_f32()));
            let a = plain.radiance(direction);
            let b = large.radiance_from(vec3(1.0, 2.0, 3.0), direction);
            assert!(
                (a.luminance() - b.luminance()).abs() <= 1e-3 * a.luminance().max(1.0),
                "{a:?} vs {b:?}"
            );
            let pa = plain.pdf(direction);
            let pb = large.pdf_from(vec3(1.0, 2.0, 3.0), direction);
            if pa > 1e-4 {
                assert!((pb / pa - 1.0).abs() < 1e-2, "{pa} vs {pb}");
            }
        }
    }

    /// Size is not cosmetic: with a dome close to the scene, two points see
    /// different parts of the sky. That parallax is the whole reason the
    /// control exists, so a mapping that produced the same lookup everywhere
    /// would be a no-op wearing a slider.
    #[test]
    fn a_close_dome_shows_parallax() {
        // A horizontal ramp, so *every* azimuth reads a different value and
        // the test cannot pass or fail on whether it happened to hit a sun.
        let (w, h) = (64usize, 32usize);
        let mut data = vec![0.0f32; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let value = 0.1 + x as f32 / w as f32;
                for c in 0..3 {
                    data[(y * w + x) * 3 + c] = value;
                }
            }
        }
        let mut env = Environment::new(
            Arc::new(Image {
                width: w,
                height: h,
                channels: 3,
                data,
                color_space: ColorSpace::Raw,
            }),
            1.0,
        );
        env.set_dome(finite_dome(10.0));
        let direction = vec3(0.0, 0.3, -1.0).normalize();
        let near = env.radiance_from(vec3(-6.0, 0.0, 0.0), direction);
        let far = env.radiance_from(vec3(6.0, 0.0, 0.0), direction);
        assert!(
            (near.luminance() - far.luminance()).abs() > 1e-4,
            "the dome looks identical from both sides: {near:?} vs {far:?}"
        );
    }

    /// Flatten replaces the bottom of the dome with the ground plane. A ray
    /// aimed steeply down must therefore read the image from where it meets
    /// the floor, not from the bottom pole — which is what stops the floor
    /// from visibly curving.
    #[test]
    fn flattening_puts_the_lower_dome_on_the_ground() {
        let dome = Dome {
            radius: Some(20.0),
            height: 5.0,
            flatten: true,
        };
        let origin = vec3(0.0, 2.0, 0.0);
        let down = vec3(0.3, -1.0, 0.0).normalize();
        let hit = dome.surface_point(origin, down).expect("a dome hit");
        assert!(
            hit.point.y.abs() < 1e-4,
            "a downward ray should land on the ground, not at {:?}",
            hit.point
        );

        // And the round trip holds: the direction that lookup reports must be
        // the one from_centre maps back to this same point.
        let (local, _) = dome.lookup(origin, down);
        let (direction, _, _) = dome.from_centre(origin, local).expect("an inverse");
        assert!(
            (direction - down).length() < 1e-3,
            "{direction:?} is not {down:?}"
        );
    }

    /// Straight up is unaffected by flattening — only the lower half of the
    /// dome is replaced, and a version that moved the sky would put the sun
    /// somewhere new every time the ground was switched on.
    #[test]
    fn flattening_leaves_the_upper_dome_alone() {
        let plain = Dome {
            radius: Some(20.0),
            height: 0.0,
            flatten: false,
        };
        let flat = Dome {
            flatten: true,
            ..plain
        };
        for direction in [Vec3::Y, vec3(0.5, 0.8, -0.2).normalize()] {
            let (a, _) = plain.lookup(Vec3::ZERO, direction);
            let (b, _) = flat.lookup(Vec3::ZERO, direction);
            assert!((a - b).length() < 1e-6, "{a:?} vs {b:?}");
        }
    }

    /// Contrast separates dark from bright about the image's own mid-tone, so
    /// the mid-tone itself must not move. Otherwise the control is a disguised
    /// brightness slider and the two can never be adjusted independently.
    #[test]
    fn contrast_pivots_on_the_image_mid_tone() {
        let (w, h) = (32usize, 16usize);
        let mut data = vec![0.0f32; w * h * 3];
        // A log-uniform spread, so the geometric mean is a real mid-tone
        // rather than an artefact of two values.
        for i in 0..w * h {
            let value = f32::exp2(-4.0 + 8.0 * (i as f32 / (w * h - 1) as f32));
            for c in 0..3 {
                data[i * 3 + c] = value;
            }
        }
        let image = Image {
            width: w,
            height: h,
            channels: 3,
            data,
            color_space: ColorSpace::Raw,
        };

        let boosted = apply_contrast(&image, 1.8);
        let reduced = apply_contrast(&image, 0.5);

        // The geometric mean is the pivot, and it is where the middle sample
        // sits in a log-uniform ramp.
        let middle = (w * h) / 2;
        let (x, y) = (middle % w, middle / w);
        let original = image.texel(x, y).x;
        for (name, adjusted) in [("boosted", &boosted), ("reduced", &reduced)] {
            let value = adjusted.texel(x, y).x;
            assert!(
                (value / original - 1.0).abs() < 0.05,
                "{name} moved the mid-tone: {original} -> {value}"
            );
        }

        // And it has to actually separate: the brightest texel moves away from
        // the pivot when contrast rises and towards it when it falls.
        let brightest = image.texel(w - 1, h - 1).x;
        assert!(boosted.texel(w - 1, h - 1).x > brightest * 1.5);
        assert!(reduced.texel(w - 1, h - 1).x < brightest * 0.9);
    }

    /// Contrast of exactly 1 must not touch a single texel. A control that
    /// altered the image at its neutral value could not be switched off.
    #[test]
    fn neutral_contrast_is_the_identity() {
        let image = Image::constant(8, 4, Vec3::splat(0.3));
        let out = apply_contrast(&image, 1.0);
        assert_eq!(out.data, image.data);
    }

    /// An image with a single bright texel — a stand-in for the sun, and the
    /// case that makes importance sampling mandatory.
    fn sun_env(sun_x: usize, sun_y: usize, brightness: f32) -> Environment {
        let (w, h) = (64usize, 32usize);
        let mut data = vec![0.01f32; w * h * 3];
        let i = (sun_y * w + sun_x) * 3;
        data[i] = brightness;
        data[i + 1] = brightness;
        data[i + 2] = brightness;
        Environment::new(
            Arc::new(Image {
                width: w,
                height: h,
                channels: 3,
                data,
                color_space: ColorSpace::LinearSrgb,
            }),
            1.0,
        )
    }

    /// Total radiance arriving from the whole sphere, by deterministic
    /// quadrature.
    ///
    /// Uniform Monte Carlo is useless as a reference here: it hits a
    /// one-texel sun roughly once in two thousand samples, so its own error
    /// bars are wider than the discrepancies these tests are looking for.
    /// Quadrature over the sphere has no such problem.
    fn integrate_radiance(env: &Environment) -> f64 {
        const THETA_STEPS: usize = 2048;
        const PHI_STEPS: usize = 4096;
        let d_theta = PI / THETA_STEPS as f32;
        let d_phi = 2.0 * PI / PHI_STEPS as f32;
        let mut sum = 0.0f64;
        for ti in 0..THETA_STEPS {
            let theta = (ti as f32 + 0.5) * d_theta;
            let (sin_theta, cos_theta) = theta.sin_cos();
            for pi in 0..PHI_STEPS {
                let phi = (pi as f32 + 0.5) * d_phi - PI;
                let d = vec3(sin_theta * phi.sin(), cos_theta, -sin_theta * phi.cos());
                sum += (env.radiance(d).luminance() * sin_theta) as f64;
            }
        }
        sum * (d_theta * d_phi) as f64
    }

    /// The PDF must integrate to 1 over the sphere. Everything else in the
    /// sampler is downstream of this.
    #[test]
    fn pdf_integrates_to_one_over_the_sphere() {
        for env in [constant_env(1.0), sun_env(10, 8, 500.0)] {
            let mut rng = Pcg32::new(1, 1);
            let n = 400_000;
            let mut sum = 0.0f64;
            for _ in 0..n {
                let d = sample_uniform_sphere(vec2(rng.next_f32(), rng.next_f32()));
                // Uniform sphere pdf is 1 / 4pi.
                sum += (env.pdf(d) * 4.0 * PI) as f64;
            }
            let integral = sum / n as f64;
            assert!(
                (integral - 1.0).abs() < 0.02,
                "pdf integrates to {integral}"
            );
        }
    }

    /// `sample` must draw from exactly the distribution `pdf` describes.
    ///
    /// If they disagree, `radiance / pdf` is a biased estimator of the
    /// environment's total radiance — and it is biased in a way that looks
    /// like "the HDRI is a bit too bright", which no visual inspection
    /// catches.
    #[test]
    fn sample_and_pdf_agree() {
        for (name, env) in [
            ("uniform", constant_env(2.0)),
            ("one-texel sun", sun_env(40, 20, 800.0)),
        ] {
            let reference = integrate_radiance(&env);
            let mut rng = Pcg32::new(2, 2);
            let n = 400_000;
            let mut estimate = 0.0f64;
            for _ in 0..n {
                if let Some(s) = env.sample(vec2(rng.next_f32(), rng.next_f32())) {
                    estimate += (s.radiance.luminance() / s.pdf) as f64;
                }
            }
            estimate /= n as f64;

            let relative = (estimate - reference).abs() / reference.max(1e-9);
            assert!(
                relative < 0.02,
                "{name}: importance sampling gives {estimate}, quadrature says {reference}"
            );
        }
    }

    /// The point of the exercise: importance sampling must find a small bright
    /// sun with far lower variance than uniform sampling. Without this an
    /// outdoor scene is unusable, per §5.2.
    #[test]
    fn importance_sampling_beats_uniform_on_a_small_sun() {
        let env = sun_env(20, 12, 5000.0);
        let mut rng = Pcg32::new(3, 3);
        let n = 20_000;

        let variance_of = |mut f: Box<dyn FnMut() -> f64>| {
            let mut mean = 0.0f64;
            let mut m2 = 0.0f64;
            for i in 1..=n {
                let x = f();
                let delta = x - mean;
                mean += delta / i as f64;
                m2 += delta * (x - mean);
            }
            (mean, m2 / (n - 1) as f64)
        };

        let env_a = env.clone();
        let mut rng_a = rng;
        let (mean_is, var_is) = variance_of(Box::new(move || {
            match env_a.sample(vec2(rng_a.next_f32(), rng_a.next_f32())) {
                Some(s) => (s.radiance.luminance() / s.pdf) as f64,
                None => 0.0,
            }
        }));

        let env_b = env.clone();
        let (mean_uniform, var_uniform) = variance_of(Box::new(move || {
            let d = sample_uniform_sphere(vec2(rng.next_f32(), rng.next_f32()));
            (env_b.radiance(d).luminance() * 4.0 * PI) as f64
        }));

        // Both estimate the same quantity, and importance sampling gets
        // there accurately with the sample budget given...
        let reference = integrate_radiance(&env);
        assert!(
            (mean_is - reference).abs() / reference < 0.05,
            "importance sampling gives {mean_is}, quadrature says {reference}"
        );
        // ...while uniform sampling, with the same budget, is nowhere near.
        // (Its mean is not asserted: that is exactly the point.)
        assert!(
            var_is * 50.0 < var_uniform,
            "importance sampling only reduced variance from {var_uniform} to {var_is}"
        );
        let _ = mean_uniform;
    }

    /// Row weighting by `sin(theta)`: without it the poles are over-sampled.
    /// A uniform environment must produce directions distributed uniformly
    /// over the *sphere*, not over the image.
    #[test]
    fn sampling_a_uniform_environment_is_uniform_on_the_sphere() {
        let env = constant_env(1.0);
        let mut rng = Pcg32::new(4, 4);
        // Bin by cos(theta): equal-area bands on the sphere.
        const BINS: usize = 10;
        let mut counts = [0u32; BINS];
        let n = 200_000;
        for _ in 0..n {
            if let Some(s) = env.sample(vec2(rng.next_f32(), rng.next_f32())) {
                let bin = (((s.direction.y + 1.0) * 0.5 * BINS as f32) as usize).min(BINS - 1);
                counts[bin] += 1;
            }
        }
        let expected = n as f32 / BINS as f32;
        for (i, &c) in counts.iter().enumerate() {
            let ratio = c as f32 / expected;
            assert!(
                (ratio - 1.0).abs() < 0.06,
                "band {i} has {ratio:.3}x the expected count (poles over-weighted?)"
            );
        }
    }

    /// Rotation must move the lighting without touching the CDF, since that is
    /// what makes the rotation slider interactive.
    #[test]
    fn rotation_moves_the_light_without_rebuilding() {
        let mut env = sun_env(0, 16, 1000.0);
        let before = env.sample(vec2(0.5, 0.5)).unwrap().direction;
        let cdf_before = env.conditional_cdf.clone();

        env.set_rotation_y(std::f32::consts::FRAC_PI_2);
        let after = env.sample(vec2(0.5, 0.5)).unwrap().direction;

        assert_eq!(cdf_before, env.conditional_cdf, "CDF was rebuilt");
        assert!(
            (before - after).length() > 0.5,
            "rotation did not move the light: {before:?} -> {after:?}"
        );
        // The vertical component is unchanged by a rotation about Y.
        assert!((before.y - after.y).abs() < 1e-4);
    }

    #[test]
    fn radiance_scales_with_intensity() {
        let mut env = constant_env(1.0);
        let d = vec3(0.3, 0.5, -0.8).normalize();
        let base = env.radiance(d).luminance();
        env.set_intensity(3.0);
        assert!((env.radiance(d).luminance() / base - 3.0).abs() < 1e-4);
        // Intensity must not disturb the sampling distribution.
        assert!(env.pdf(d) > 0.0);
    }

    #[test]
    fn a_black_environment_samples_nothing_rather_than_dividing_by_zero() {
        let env = Environment::new(Arc::new(Image::constant(8, 4, Vec3::ZERO)), 1.0);
        assert!(env.sample(vec2(0.5, 0.5)).is_none());
        assert_eq!(env.pdf(vec3(0.0, 1.0, 0.0)), 0.0);
        assert!(env.radiance(vec3(0.0, 1.0, 0.0)).is_black());
    }

    #[test]
    fn sampled_directions_are_unit_length_and_finite() {
        let env = sun_env(31, 15, 200.0);
        let mut rng = Pcg32::new(5, 5);
        for _ in 0..20_000 {
            if let Some(s) = env.sample(vec2(rng.next_f32(), rng.next_f32())) {
                assert!((s.direction.length() - 1.0).abs() < 1e-4);
                assert!(s.pdf.is_finite() && s.pdf > 0.0);
                assert!(s.radiance.is_finite());
            }
        }
    }
}
