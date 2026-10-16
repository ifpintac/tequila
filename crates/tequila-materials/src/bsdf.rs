//! The BSDF interface contract from `docs/01-RENDERER-CORE.md` §3.3.
//!
//! Nothing in the renderer may know what kind of material it is dealing with.
//! Every lobe implements exactly this trait, evaluates in **shading space**
//! (the shading normal is `+Z`), and satisfies one non-negotiable invariant:
//!
//! > `eval` and `pdf` are mutually consistent — the distribution `sample`
//! > produces is exactly the one `pdf` describes.
//!
//! The χ² test in `docs/05` §3.2 checks that invariant for every lobe. A
//! `pdf` that disagrees with `sample` produces images that are wrong in a way
//! no visual inspection reliably catches: slightly too bright at certain
//! roughnesses, subtly wrong under MIS.

use tequila_core::math::{Vec2, Vec3};
use tequila_core::spectrum::Spectrum;

bitflags::bitflags! {
    /// What kinds of scattering a lobe performs. The integrator uses these to
    /// decide whether next-event estimation is worth attempting and whether a
    /// path is still a specular chain.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct BsdfFlags: u32 {
        const REFLECTION   = 1 << 0;
        const TRANSMISSION = 1 << 1;
        /// Scattering is confined to a single direction (or a discrete set).
        /// Light sampling cannot generate it, so NEE must be skipped and the
        /// MIS weight is 1.
        const DELTA        = 1 << 2;
        const GLOSSY       = 1 << 3;
        const DIFFUSE      = 1 << 4;
        /// Crosses into, or out of, a part's subsurface medium: the path is
        /// starting or ending a random walk (`docs/01` §6.4).
        const SUBSURFACE   = 1 << 5;
    }
}

impl BsdfFlags {
    /// True when the lobe scatters *only* into delta directions, so
    /// next-event estimation would contribute exactly zero.
    #[inline]
    pub fn is_pure_delta(self) -> bool {
        self.contains(BsdfFlags::DELTA) && !self.intersects(BsdfFlags::GLOSSY | BsdfFlags::DIFFUSE)
    }
}

/// One scattering event produced by [`Bsdf::sample`].
#[derive(Clone, Copy, Debug)]
pub struct BsdfSample {
    /// Sampled incident direction, in shading space.
    pub wi: Vec3,
    /// BSDF value for `(wo, wi)`. Excludes the cosine term, which the
    /// integrator applies, so that the convention matches [`Bsdf::eval`].
    pub f: Spectrum,
    /// Solid-angle PDF of having sampled `wi`. For a delta lobe this is 1 and
    /// `is_delta` is set, meaning "already divided out".
    pub pdf: f32,
    pub is_delta: bool,
    pub is_transmission: bool,
    /// Relative index of refraction across the event; 1 for reflection. Feeds
    /// the `eta_scale` term of Russian roulette (§3.1).
    pub eta: f32,
    pub flags: BsdfFlags,
}

impl BsdfSample {
    /// The throughput multiplier `f * |cos| / pdf` for this event.
    #[inline]
    pub fn throughput_weight(&self) -> Spectrum {
        if self.pdf <= 0.0 {
            return Spectrum::ZERO;
        }
        self.f * (self.wi.z.abs() / self.pdf)
    }
}

/// A bidirectional scattering distribution function in shading space.
pub trait Bsdf {
    /// `f(wo, wi)`. Excludes the cosine term. Returns zero for directions the
    /// lobe cannot scatter into, and always zero for delta lobes.
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum;

    /// PDF of sampling `wi` given `wo`, in solid-angle measure. Zero for delta
    /// lobes.
    ///
    /// For a multi-lobe BSDF this must be the **weighted sum over all lobes**,
    /// not the PDF of the lobe that happens to have been chosen. Returning
    /// only the sampled lobe's PDF is the most common single-lobe-pdf bug and
    /// it breaks MIS silently (`docs/02-MATERIAL-SYSTEM.md` §2.4).
    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32;

    /// Importance-sample an incident direction. `u` drives the direction,
    /// `uc` the discrete lobe choice.
    fn sample(&self, wo: Vec3, u: Vec2, uc: f32) -> Option<BsdfSample>;

    /// Hemispherical-directional reflectance. Used for Russian roulette, the
    /// denoiser's albedo guide, and light-selection heuristics. May be
    /// approximate, but must never exceed 1 or Russian roulette stops
    /// terminating paths.
    fn albedo(&self, wo: Vec3) -> Spectrum;

    fn flags(&self) -> BsdfFlags;

    /// `f(wo, wi)` restricted to the lobes a light is allowed to reach.
    ///
    /// §01 §5.4's visibility flags: a light can be set to appear in
    /// reflections but not in the diffuse shading, or the reverse. That is a
    /// per-lobe restriction, not a per-light one — refusing the whole light
    /// would be a cruder feature, and the studio setups the flags exist for
    /// (a big key that lights the product without showing up as a broad
    /// diffuse wash) need the split.
    ///
    /// The default is the honest conservative answer for a BSDF that cannot
    /// separate its lobes: all or nothing. Implementations that hold their
    /// lobes separately override it.
    fn eval_selective(&self, wo: Vec3, wi: Vec3, diffuse: bool, specular: bool) -> Spectrum {
        if diffuse && specular {
            self.eval(wo, wi)
        } else if !diffuse && !specular {
            Spectrum::ZERO
        } else {
            let flags = self.flags();
            let is_diffuse = flags.contains(BsdfFlags::DIFFUSE);
            if (diffuse && is_diffuse) || (specular && !is_diffuse) {
                self.eval(wo, wi)
            } else {
                Spectrum::ZERO
            }
        }
    }

    /// Convenience: does next-event estimation have anything to contribute?
    #[inline]
    fn is_pure_delta(&self) -> bool {
        self.flags().is_pure_delta()
    }
}
