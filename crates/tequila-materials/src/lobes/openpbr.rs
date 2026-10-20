//! The OpenPBR layer stack.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.1: fuzz over coat over base, each layer
//! attenuating what is beneath it, with the base itself mixing metal,
//! specular, transmission, subsurface and diffuse *by weight rather than by
//! addition*.
//!
//! # The one rule that breaks everything if broken
//!
//! §2.4, verbatim: "`pdf()` must return the **weighted sum over all lobes**,
//! not just the sampled one, or MIS breaks (this is the most common
//! single-lobe-pdf bug)."
//!
//! It is worth being precise about why, because the symptom does not point at
//! the cause. Multiple importance sampling weights a BSDF sample against the
//! probability that *light sampling* would have found the same direction, and
//! vice versa. If `pdf()` reports only the lobe that happened to be chosen, the
//! weight is computed against a density the sampler does not actually have.
//! The estimator stays unbiased for a single lobe and goes wrong the moment
//! two lobes overlap — which is every glossy material over a diffuse base.
//! The image does not develop artefacts; it develops a slight, uniform,
//! completely unexplainable brightness error.
//!
//! So every path through this file that computes a PDF sums over all of them,
//! including in `sample`, where the returned PDF is *not* the density of the
//! lobe that was sampled.
//!
//! # Subsurface
//!
//! Subsurface scattering is a random walk through a medium (§01 §6.4), and
//! the walk is the integrator's. What is here is its doorway: the
//! `subsurface_weight` share of what the base does not reflect crosses the
//! surface through a white Lambertian transmission, and the path carries on
//! inside the part. The colour comes back out of the walk.

use crate::bsdf::{Bsdf, BsdfFlags, BsdfSample};
use crate::lobes::{
    Conductor, DielectricSpecular, DielectricTransmission, DiffuseTransmission, OrenNayarDiffuse,
    Sheen,
};
use crate::microfacet::Ggx;
use crate::openpbr::Material;
use crate::thin_film::Film;
use tequila_core::math::{vec3, Vec2, Vec3};
use tequila_core::spectrum::Spectrum;

/// How many sub-lobes the stack can hold at once.
const MAX_LOBES: usize = 5;

/// Cosine-weighted average of the product of the transmittances of every
/// layer standing over the diffuse substrate.
///
/// One minus this is the fraction of the light the substrate sends back up
/// that the stack above turns around, which is what the trapped-light series
/// beneath a Lambertian substrate has to be summed against.
///
/// It is measured rather than assembled from the layers' individual
/// hemispherical albedos, because those multiply only if the layers are
/// independent and they are emphatically not: every one of them transmits
/// least at the same grazing angles, so the average of the product is larger
/// than the product of the averages. Assuming independence is the last place
/// the old per-layer composition invented light — four tenths of a percent on
/// a clearcoated plastic, which is small but is on the wrong side of one.
///
/// Sixteen midpoints of `mu`, cosine-weighted by `2 mu d(mu)`, and only when
/// there is more than one layer to correlate. A single layer's average
/// transmittance is one minus its tabulated hemispherical albedo, which is
/// one table entry rather than sixteen directional lookups — and a directional
/// lookup is not cheap, so taking that shortcut is the difference between a
/// plastic costing what it always did and costing five times as much. A
/// metal, a bare diffuse and a plain glass never reach this function at all.
fn stack_transmittance(
    fuzz: Option<&Sheen>,
    coat: Option<&DielectricSpecular>,
    coat_weight: f32,
    specular: Option<&DielectricSpecular>,
    specular_scale: f32,
) -> f32 {
    let layers =
        fuzz.is_some() as u32 + coat.is_some() as u32 + specular.is_some() as u32;
    if layers <= 1 {
        let reflected = fuzz
            .map(|sheen| sheen.hemispherical_albedo())
            .or_else(|| coat.map(|lobe| lobe.hemispherical_albedo() * coat_weight))
            .or_else(|| specular.map(|lobe| lobe.hemispherical_albedo() * specular_scale))
            .unwrap_or(0.0);
        return (1.0 - reflected).clamp(0.0, 1.0);
    }

    const STEPS: usize = 16;
    let mut total = 0.0;
    for step in 0..STEPS {
        let mu = (step as f32 + 0.5) / STEPS as f32;
        let wo = vec3((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu);
        let mut transmitted = 1.0f32;
        if let Some(sheen) = fuzz {
            transmitted *= sheen.transmittance(mu);
        }
        if let Some(coat) = coat {
            transmitted *= (1.0 - coat.directional_albedo(wo) * coat_weight).clamp(0.0, 1.0);
        }
        if let Some(specular) = specular {
            transmitted *=
                (1.0 - specular.directional_albedo(wo) * specular_scale).clamp(0.0, 1.0);
        }
        total += transmitted * 2.0 * mu;
    }
    (total / STEPS as f32).clamp(0.0, 1.0)
}

/// The hemispherical response of one layer over a substrate: its own
/// reflection, plus what it passes down and gets back, plus everything
/// trapped between the two.
///
/// This is what makes the stack compose. `layer` is the layer's own
/// hemispherical albedo and `substrate` the response of everything beneath
/// it — itself the result of this function one level down. A white
/// substrate under any layer returns exactly one, whatever the layer is,
/// which is the property the furnace tests measure.
#[inline]
fn layer_response(layer: f32, substrate: f32) -> f32 {
    let layer = layer.clamp(0.0, 1.0);
    let substrate = substrate.clamp(0.0, 1.0);
    let transmitted = 1.0 - layer;
    (layer + transmitted * transmitted * substrate / (1.0 - layer * substrate).max(1e-3))
        .clamp(0.0, 1.0)
}

/// One weighted component of the stack.
#[derive(Clone, Copy, Debug)]
enum Lobe {
    Fuzz(Sheen),
    Coat(DielectricSpecular),
    Metal(Conductor),
    Specular(DielectricSpecular),
    Transmission(DielectricTransmission),
    Diffuse(OrenNayarDiffuse),
    Subsurface(DiffuseTransmission),
}

impl Lobe {
    #[inline]
    fn as_bsdf(&self) -> &dyn Bsdf {
        match self {
            Lobe::Fuzz(l) => l,
            Lobe::Coat(l) => l,
            Lobe::Metal(l) => l,
            Lobe::Specular(l) => l,
            Lobe::Transmission(l) => l,
            Lobe::Diffuse(l) => l,
            Lobe::Subsurface(l) => l,
        }
    }
}

/// Which layer of §2.1's stack a lobe belongs to. Deeper layers are
/// attenuated by everything above them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Depth {
    Fuzz,
    Coat,
    /// A metal flake under the coat, in its own tilted frame. Beneath the
    /// coat like the base, and like the base's interface it hands light back
    /// near the angle it arrived by, so the coat's trapped-light series is
    /// summed for it the same way.
    Flake,
    /// At the dielectric interface: metal, specular reflection, transmission.
    Base,
    /// Underneath that interface: the diffuse substrate.
    Substrate,
}

/// The assembled stack for one shading point.
#[derive(Clone, Debug)]
pub struct OpenPbrBsdf {
    lobes: [Option<(Lobe, Depth, Spectrum, f32)>; MAX_LOBES],
    count: usize,
    /// Selection probabilities, normalized. Parallel to `lobes`.
    probabilities: [f32; MAX_LOBES],
    /// The layers above the base, kept so their attenuation can be evaluated
    /// for the actual pair of directions rather than guessed once.
    fuzz: Option<Sheen>,
    coat: Option<DielectricSpecular>,
    coat_weight: f32,
    coat_darkening: f32,
    /// Hemispherical response of the assembly the coat sits on: the base
    /// interface, whatever is under it, and the light trapped between them.
    /// The coat's trapped-light gain asks this, not the base's albedo.
    base_response: f32,
    /// The same quantity one layer up — the coat's whole answer, which is
    /// what the fuzz sits on. Equal to `base_response` when there is no coat.
    coat_response: f32,
    /// The dielectric interface over the diffuse substrate, kept for the same
    /// reason as `coat`.
    specular: Option<DielectricSpecular>,
    specular_scale: f32,
    diffuse_albedo: f32,
    /// What the whole stack above the diffuse substrate turns back, for the
    /// one trapped-light series that sits under it. See
    /// [`stack_transmittance`].
    substrate_internal: f32,
    /// The flake's frame in the shading frame, when this point is on one:
    /// the flake lobe is evaluated with directions turned into it. A
    /// rotation, so densities carry over unchanged.
    flake_frame: Option<tequila_core::math::Onb>,
}

impl OpenPbrBsdf {
    /// Build the stack from §2.2's parameters.
    ///
    /// `roughness_floor` is path regularization (`docs/01` §3.5): the
    /// integrator raises it after a couple of bounces so near-specular lobes
    /// deep in a path stop producing fireflies.
    pub fn new(material: &Material, roughness_floor: f32) -> OpenPbrBsdf {
        OpenPbrBsdf::build(material, roughness_floor, false)
    }

    /// The same stack, for a path already inside the material.
    ///
    /// Only the transmission lobe cares, and only because the shading frame
    /// has thrown the answer away: it is turned to face the incoming ray, so
    /// `wo.z` is positive whichever side the path is on. Everything else in
    /// the stack is symmetric and reads the same either way.
    pub fn from_inside(material: &Material, roughness_floor: f32) -> OpenPbrBsdf {
        OpenPbrBsdf::build(material, roughness_floor, true)
    }

    fn build(material: &Material, roughness_floor: f32, from_inside: bool) -> OpenPbrBsdf {
        let mut lobes: [Option<(Lobe, Depth, Spectrum, f32)>; MAX_LOBES] = [None; MAX_LOBES];
        let mut count = 0;

        // Baked occlusion scales every lobe, which is the one place it can go
        // without asking what kind of light is arriving.
        //
        // The usual real-time rule — occlude the diffuse, leave the specular
        // alone — exists because those renderers have an analytic direct light
        // whose shadow is already computed, and occluding it twice is visible.
        // Tequila has no such split: every photon here arrives from the
        // environment or from emissive geometry, so the whole of `L_i` is what
        // the map is describing the visibility of. Scaling the stack is then
        // the plain reading of `∫ f · L_i · V` with `V` pulled out as its
        // hemispherical average, and it is also the only version that does
        // anything at all on a metal, where there is no diffuse lobe to dim.
        //
        // Folded into `scale` rather than into `eval`, so that `sample`,
        // `albedo` and the selection probabilities all pick it up from the one
        // place. The probabilities are normalized afterwards, so a uniform
        // factor leaves them alone — which is right: being in shadow does not
        // change which lobe is worth sampling.
        let occlusion = Spectrum::splat(material.geometry_occlusion.clamp(0.0, 1.0));
        let mut push = |lobe: Lobe, depth: Depth, scale: Spectrum, weight: f32| {
            let scale = scale * occlusion;
            if count < MAX_LOBES && weight > 1e-4 && !scale.is_black() {
                lobes[count] = Some((lobe, depth, scale, weight));
                count += 1;
            }
        };

        let rough = |r: f32| r.clamp(0.0, 1.0).max(roughness_floor);

        // §2.2's thin film, which the diagram in §2.1 places over "specular
        // below" — the metal lobe for a metallic base, the dielectric
        // specular lobe otherwise. Not over the coat: the coat is a separate
        // layer of its own with its own interface, and a film on top of that
        // is the Multi-Layer Optics material rather than this one.
        let film = (material.thin_film_weight > 1e-4).then_some(Film {
            ior: material.thin_film_ior,
            thickness: material.thin_film_thickness,
            weight: material.thin_film_weight,
        });

        // --- fuzz, the topmost layer ------------------------------------
        let fuzz = (material.fuzz_weight > 1e-4).then(|| {
            Sheen::new(
                material.fuzz_color,
                material.fuzz_roughness,
                material.fuzz_weight,
            )
        });
        if let Some(sheen) = fuzz {
            push(
                Lobe::Fuzz(sheen),
                Depth::Fuzz,
                Spectrum::ONE,
                material.fuzz_weight,
            );
        }

        // --- coat --------------------------------------------------------
        let coat = (material.coat_weight > 1e-4).then(|| {
            DielectricSpecular::new(rough(material.coat_roughness), material.coat_ior)
                .with_tint(material.coat_color)
        });
        if let Some(coat) = coat {
            push(
                Lobe::Coat(coat),
                Depth::Coat,
                Spectrum::ONE,
                material.coat_weight,
            );
        }

        // --- base --------------------------------------------------------
        //
        // Attenuation by the layers above is *not* folded in here. It depends
        // on both directions, and a coated surface seen at 78 degrees loses
        // far more to its coat than the hemispherical average suggests —
        // pre-multiplying by an average lets the stack return more light than
        // it received at grazing angles.
        // Where this point is on a flake, the flake is what the light meets
        // under the coat, in place of that share of the base.
        let flake = material.flake_coverage.clamp(0.0, 1.0) * (material.flake_weight > 1e-4) as u8 as f32;
        let base_scale = Spectrum::splat(material.base_weight * (1.0 - flake));
        let flake_frame = (flake > 1e-4).then(|| tequila_core::math::Onb::from_normal(material.flake_normal));
        if flake > 1e-4 {
            let conductor = Conductor::new(
                Ggx::from_roughness(rough(material.flake_roughness), 0.0),
                crate::fresnel::metals::ALUMINIUM,
            )
            .with_tint(material.flake_color);
            push(Lobe::Metal(conductor), Depth::Flake, Spectrum::splat(material.base_weight), flake);
        }
        let mut specular = None;
        let mut specular_scale = 0.0f32;
        let mut diffuse_albedo = 0.0f32;
        if !base_scale.is_black() {
            let metalness = material.base_metalness;

            if metalness > 1e-4 {
                // §2.2: `base_color` is "conductor reflectance when
                // metallic", and §3.1's Metal preset exposes both a measured
                // Type and a Color. The measured spectrum gives the metal its
                // character; the colour tints it.
                let conductor = Conductor::new(
                    Ggx::from_roughness(
                        rough(material.specular_roughness),
                        material.specular_roughness_anisotropy,
                    ),
                    material.metal_ior,
                )
                .with_tint(material.base_color)
                .with_film(film);
                push(Lobe::Metal(conductor), Depth::Base, base_scale, metalness);
            }

            let dielectric = 1.0 - metalness;
            if dielectric > 1e-4 {
                // Specular over the dielectric substrate. Where the base
                // transmits, the interface is already accounted for by the
                // transmission lobe's own Fresnel-weighted reflect/refract
                // split, so the reflection is not added a second time.
                let opaque = 1.0 - material.transmission_weight;
                let specular_weight = material.specular_weight * opaque;
                if specular_weight > 1e-4 {
                    let lobe = DielectricSpecular::new(
                        rough(material.specular_roughness),
                        material.specular_ior,
                    )
                    .with_tint(material.specular_color)
                    .with_film(film);
                    specular = Some(lobe);
                    specular_scale = specular_weight;
                    push(
                        Lobe::Specular(lobe),
                        Depth::Base,
                        base_scale,
                        dielectric * specular_weight,
                    );
                }

                if material.transmission_weight > 1e-4 {
                    // §2.2: with a depth the colour belongs to the volume and
                    // the integrator absorbs it along the path; only a piece
                    // with no volume wears it as a tint at the surface.
                    let has_depth =
                        !material.geometry_thin_walled && material.transmission_depth > 0.0;
                    // The square root at each crossing, so light through a
                    // whole object — in at one surface, out at another —
                    // comes out tinted by the colour once, as Cycles tints
                    // its principled glass: a pane of 0.25 passes 0.5, a
                    // closed ball 0.25. The full colour at every crossing
                    // squared it, and Poly Haven's dirty bulb, authored in
                    // Blender, came out several times too dark.
                    // The surface tint is the surface's own, depth or not.
                    let c = if has_depth {
                        material.transmission_tint
                    } else {
                        material.transmission_color * material.transmission_tint
                    };
                    let tint = Spectrum { r: c.r.max(0.0).sqrt(), g: c.g.max(0.0).sqrt(), b: c.b.max(0.0).sqrt() };
                    let transmission = DielectricTransmission::new(
                        rough(material.specular_roughness),
                        material.specular_ior,
                    )
                    .with_tint(tint)
                    .thin_walled(material.geometry_thin_walled)
                    .from_inside(from_inside);
                    push(
                        Lobe::Transmission(transmission),
                        Depth::Base,
                        base_scale,
                        dielectric * material.transmission_weight,
                    );
                }

                // Whatever the transmission did not take is diffuse. §2.1:
                // "the remainder".
                let remainder = dielectric * opaque;
                let subsurface = material.subsurface_weight.clamp(0.0, 1.0);
                // What the trapped-light series bounces against: the whole
                // substrate's return, the walk's share at the colour it is
                // tuned to give back.
                diffuse_albedo = (material.base_color * (1.0 - subsurface)
                    + material.subsurface_color * subsurface)
                    .luminance()
                    .clamp(0.0, 1.0);
                let diffuse_weight = remainder * (1.0 - subsurface);
                if diffuse_weight > 1e-4 {
                    let diffuse =
                        OrenNayarDiffuse::new(material.base_color, material.base_diffuse_roughness);
                    push(
                        Lobe::Diffuse(diffuse),
                        Depth::Substrate,
                        base_scale,
                        diffuse_weight,
                    );
                }
                // The doorway into the random walk (§01 §6.4).
                let walk_weight = remainder * subsurface;
                if walk_weight > 1e-4 {
                    push(
                        Lobe::Subsurface(DiffuseTransmission::new(Spectrum::ONE, material.subsurface_color)),
                        Depth::Substrate,
                        base_scale,
                        walk_weight,
                    );
                }
            }
        }

        // --- selection probabilities (§2.4) -----------------------------
        //
        // Proportional to each lobe's estimated contribution, so the sampler
        // spends its samples where the energy is. Any positive set of
        // probabilities is unbiased; a *good* set is what keeps the variance
        // down, which is the whole reason to weight them at all.
        let mut probabilities = [0.0f32; MAX_LOBES];
        let mut total = 0.0;
        for (index, slot) in lobes.iter().enumerate().take(count) {
            if let Some((lobe, _, scale, weight)) = slot {
                let estimate = match lobe {
                    Lobe::Fuzz(l) => l.albedo(Vec3::Z).luminance(),
                    Lobe::Coat(l) => l.albedo(Vec3::Z).luminance().max(0.05),
                    Lobe::Metal(l) => l.albedo(Vec3::Z).luminance(),
                    // A clear specular interface has a low face-on albedo but
                    // dominates at grazing angles. A floor keeps it sampled
                    // often enough that its highlight is not noise.
                    Lobe::Specular(l) => l.albedo(Vec3::Z).luminance().max(0.05),
                    Lobe::Transmission(l) => l.albedo(Vec3::Z).luminance(),
                    Lobe::Diffuse(l) => l.albedo(Vec3::Z).luminance(),
                    Lobe::Subsurface(l) => l.albedo(Vec3::Z).luminance().max(0.05),
                };
                let p = (estimate * scale.luminance() * weight).max(1e-4);
                probabilities[index] = p;
                total += p;
            }
        }
        if total > 0.0 {
            for p in probabilities.iter_mut().take(count) {
                *p /= total;
            }
        }

        // What each layer is actually sitting on, built from the bottom up.
        //
        // A layer's trapped-light gain needs the hemispherical response of
        // *everything* beneath it, which for anything but the lowest layer is
        // not an albedo that appears in `Material` — it is another layer's
        // whole answer. Composing them here, once per shading point, is what
        // stops each layer assuming the stack below returns all of the light.
        let metalness = material.base_metalness.clamp(0.0, 1.0);

        // Under the dielectric interface: the diffuse substrate, or what the
        // transmission lobe carries away where the base transmits instead.
        let transmission = material.transmission_weight.clamp(0.0, 1.0);
        let under_interface = (diffuse_albedo * (1.0 - transmission)
            + material.transmission_color.luminance() * transmission)
            .clamp(0.0, 1.0);
        let interface = specular
            .map(|lobe| lobe.hemispherical_albedo() * specular_scale)
            .unwrap_or(0.0);
        let dielectric_response = layer_response(interface, under_interface);

        // §2.2 mixes metal and dielectric by weight rather than by addition,
        // so their responses mix the same way.
        let metal_response = material.base_color.luminance().clamp(0.0, 1.0);
        let base_response = metal_response * metalness + dielectric_response * (1.0 - metalness);
        // The flakes stand in for that share of the base, so they share the
        // response too: the coat over a flake is sitting on metal.
        let base_response =
            base_response * (1.0 - flake) + material.flake_color.luminance().clamp(0.0, 1.0) * flake;

        // One series for everything standing over the substrate, rather
        // than one per layer. The light under a Lambertian substrate is
        // bouncing against the *stack*, not against each layer in turn, and
        // giving every layer its own series counts the same trapped light
        // once per layer.
        let substrate_internal = if diffuse_albedo > 0.0
            && (fuzz.is_some() || coat.is_some() || specular.is_some())
        {
            1.0 - stack_transmittance(
                fuzz.as_ref(),
                coat.as_ref(),
                material.coat_weight,
                specular.as_ref(),
                specular_scale,
            )
        } else {
            0.0
        };

        let coat_response = match &coat {
            Some(lobe) => {
                layer_response(lobe.hemispherical_albedo() * material.coat_weight, base_response)
            }
            None => base_response,
        };

        OpenPbrBsdf {
            lobes,
            count,
            probabilities,
            fuzz,
            coat,
            coat_weight: material.coat_weight,
            coat_darkening: material.coat_darkening,
            base_response,
            coat_response,
            specular,
            specular_scale,
            diffuse_albedo,
            substrate_internal,
            flake_frame,
        }
    }

    /// A pair of directions as a lobe at `depth` sees them: unchanged, or
    /// turned into the flake's frame. `None` when the flake faces away from
    /// either one — light cannot reach a face turned from it.
    #[inline]
    fn frame_for(&self, depth: Depth, wo: Vec3, wi: Vec3) -> Option<(Vec3, Vec3)> {
        match (depth, &self.flake_frame) {
            (Depth::Flake, Some(frame)) => {
                let (o, i) = (frame.to_local(wo), frame.to_local(wi));
                (o.z > 0.0 && i.z > 0.0).then_some((o, i))
            }
            _ => Some((wo, wi)),
        }
    }

    /// One lobe's value for a pair of directions, in whichever frame it lives.
    ///
    /// A flake's value is converted to the surface's cosine. The integrator
    /// weights light by the cosine to the *surface* normal, and the flake's
    /// lobe is normalized against its *own*; left as it was, a steep flake
    /// lit from near its own normal and near grazing to the surface would
    /// return more than it received. Scaled by `cos_flake / cos_surface`,
    /// a flake returns exactly its own albedo in its own frame, which a
    /// metal cannot take past one.
    #[inline]
    fn lobe_eval(&self, lobe: &Lobe, depth: Depth, wo: Vec3, wi: Vec3) -> Spectrum {
        match self.frame_for(depth, wo, wi) {
            Some((o, i)) if depth == Depth::Flake => {
                if wi.z <= 1e-6 {
                    return Spectrum::ZERO;
                }
                lobe.as_bsdf().eval(o, i) * (i.z / wi.z)
            }
            Some((o, i)) => lobe.as_bsdf().eval(o, i),
            None => Spectrum::ZERO,
        }
    }

    #[inline]
    fn lobe_pdf(&self, lobe: &Lobe, depth: Depth, wo: Vec3, wi: Vec3) -> f32 {
        match self.frame_for(depth, wo, wi) {
            Some((o, i)) => lobe.as_bsdf().pdf(o, i),
            None => 0.0,
        }
    }

    /// How much of the light travelling `wo` in and `wi` out survives the
    /// layers above `depth`.
    ///
    /// The transmitted fraction appears **twice** for each layer, because
    /// light must cross it inbound and outbound. That double crossing is what
    /// makes a coated surface darken correctly towards grazing angles instead
    /// of staying flat — and omitting it is what let this stack return 1.46 of
    /// the light it received at 78 degrees before the attenuation was made
    /// view-dependent.
    ///
    /// # Where the trapped light is counted
    ///
    /// Light that crosses a layer, comes back off whatever is under it, meets
    /// the underside of the layer and is turned around for another try is a
    /// geometric series, and it is worth several percent. It used to be
    /// summed once per layer, against the base's albedo, which was wrong in
    /// two directions at once: a white velvet came back three and a half
    /// percent dark, a clearcoated metal three and a half percent bright, and
    /// a clamp on the result kept the bright case from being worse at the
    /// cost of making the dark one darker. There is no clamp here, and the
    /// series is summed in two different places depending on what the light
    /// is coming back off.
    ///
    /// **Off the interface** — metal, specular, transmission — the light
    /// returns close to the angle it arrived by, so each layer above sums its
    /// own series at that angle, against the response of the assembly beneath
    /// it: the fuzz sits on the coat's whole answer, the coat on the base
    /// interface's. A hemispherical average is wrong here because a mirror
    /// hands the light back at a grazing angle the coat barely lets out,
    /// while the average says most of it escapes on the first try.
    ///
    /// **Off the substrate** the light comes back spread over the hemisphere
    /// and bounces against the whole stack rather than against each layer in
    /// turn, so there is exactly one series, summed against
    /// [`Self::substrate_internal`]. Giving each layer its own counts the
    /// same trapped light once per layer.
    fn attenuation(&self, depth: Depth, wo: Vec3, wi: Vec3) -> f32 {
        // Everything below the interface is the Lambertian substrate, and it
        // is the only thing that spreads what it returns.
        let spreads = depth > Depth::Base;
        let mut through = 1.0f32;

        if depth > Depth::Fuzz {
            if let Some(sheen) = &self.fuzz {
                let down = sheen.transmittance(wo.z);
                let up = sheen.transmittance(wi.z);
                through *= if spreads {
                    down * up
                } else {
                    Self::trapped(down, up, 1.0 - up, self.coat_response)
                };
            }
        }
        if depth > Depth::Coat {
            if let Some(coat) = &self.coat {
                let down = 1.0 - coat.directional_albedo(wo) * self.coat_weight;
                let up = 1.0 - coat.directional_albedo(wi) * self.coat_weight;
                let physical = if spreads {
                    (down * up).clamp(0.0, 1.0)
                } else {
                    Self::trapped(down, up, 1.0 - up, self.base_response)
                };
                // §2.2's `coat_darkening` scales between the physical result
                // and no darkening at all, which is the artistic cheat the
                // parameter exists to allow.
                through *= physical * self.coat_darkening + (1.0 - self.coat_darkening);
            }
        }
        if spreads {
            if let Some(specular) = &self.specular {
                let scale = self.specular_scale;
                let down = 1.0 - specular.directional_albedo(wo) * scale;
                let up = 1.0 - specular.directional_albedo(wi) * scale;
                through *= down.clamp(0.0, 1.0) * up.clamp(0.0, 1.0);
            }
            // The one series, for the whole stack at once.
            through /= (1.0 - self.diffuse_albedo * self.substrate_internal).max(1e-3);
        }
        through
    }

    /// The light that crosses a layer, comes back off the assembly beneath it
    /// and leaves, including everything trapped in between.
    ///
    /// `down` and `up` are the fractions that cross the layer travelling `wo`
    /// and `wi`, `internal` is what the underside of the layer turns back,
    /// and `substrate` is the hemispherical response of what sits under it.
    ///
    /// With `internal = 1 - up` — the angle-preserving case, which is the
    /// only one this is called for — the result is provably at most `down`:
    /// the denominator is `1 - substrate + substrate * up`, which is at least
    /// `up` whenever `substrate` is at most one. A layer can only take light
    /// away, which is why nothing downstream needs clamping.
    #[inline]
    fn trapped(down: f32, up: f32, internal: f32, substrate: f32) -> f32 {
        let down = down.clamp(0.0, 1.0);
        let up = up.clamp(0.0, 1.0);
        let internal = internal.clamp(0.0, 1.0);
        let substrate = substrate.clamp(0.0, 1.0);
        down * up / (1.0 - substrate * internal).max(1e-3)
    }

    fn active(&self) -> impl Iterator<Item = (usize, &(Lobe, Depth, Spectrum, f32))> {
        self.lobes
            .iter()
            .enumerate()
            .take(self.count)
            .filter_map(|(i, slot)| slot.as_ref().map(|l| (i, l)))
    }
}

impl Bsdf for OpenPbrBsdf {
    fn eval(&self, wo: Vec3, wi: Vec3) -> Spectrum {
        let mut total = Spectrum::ZERO;
        for (_, (lobe, depth, scale, weight)) in self.active() {
            let through = self.attenuation(*depth, wo, wi);
            total += self.lobe_eval(lobe, *depth, wo, wi) * (*scale * (*weight * through));
        }
        total
    }

    /// The layer stack can separate its lobes, so §5.4's visibility flags are
    /// honoured exactly: each lobe is included or not by its own
    /// classification, and the attenuation between layers is unchanged.
    ///
    /// The attenuation deliberately still counts the excluded layers. A coat
    /// that a light is not allowed to appear in still *covers* the base, and
    /// dropping its attenuation along with its reflection would make the base
    /// brighter than it is with the coat fully enabled — which is the sort of
    /// energy inconsistency a visibility flag has no business creating.
    fn eval_selective(&self, wo: Vec3, wi: Vec3, diffuse: bool, specular: bool) -> Spectrum {
        if diffuse && specular {
            return self.eval(wo, wi);
        }
        let mut total = Spectrum::ZERO;
        for (_, (lobe, depth, scale, weight)) in self.active() {
            let is_diffuse = lobe
                .as_bsdf()
                .flags()
                .contains(crate::BsdfFlags::DIFFUSE);
            if (is_diffuse && !diffuse) || (!is_diffuse && !specular) {
                continue;
            }
            let through = self.attenuation(*depth, wo, wi);
            total += self.lobe_eval(lobe, *depth, wo, wi) * (*scale * (*weight * through));
        }
        total
    }

    /// The weighted sum over **every** lobe. See this module's header for why
    /// returning only the sampled lobe's density is the bug that hides.
    fn pdf(&self, wo: Vec3, wi: Vec3) -> f32 {
        let mut total = 0.0;
        for (index, (lobe, depth, _, _)) in self.active() {
            let p = self.probabilities[index];
            if p > 0.0 {
                total += p * self.lobe_pdf(lobe, *depth, wo, wi);
            }
        }
        total
    }

    fn sample(&self, wo: Vec3, u: Vec2, uc: f32) -> Option<BsdfSample> {
        if self.count == 0 {
            return None;
        }

        // Choose a lobe, and rescale `uc` so the chosen lobe gets a fresh
        // uniform value rather than one correlated with the choice. Reusing
        // it directly biases nested discrete decisions — the transmission
        // lobe's own reflect-or-refract split, for one.
        let mut cumulative = 0.0;
        let mut chosen = None;
        for (index, entry) in self.active() {
            let p = self.probabilities[index];
            if uc < cumulative + p || index == self.count - 1 {
                let rescaled = if p > 0.0 {
                    ((uc - cumulative) / p).clamp(0.0, 1.0 - f32::EPSILON)
                } else {
                    0.5
                };
                chosen = Some((index, entry, rescaled));
                break;
            }
            cumulative += p;
        }
        let (index, (lobe, depth, scale, weight), lobe_uc) = chosen?;

        // A flake samples in its own frame and hands the direction back in
        // the shading frame; one that leaves below the surface is lost.
        let sample = match (depth, &self.flake_frame) {
            (Depth::Flake, Some(frame)) => {
                let local = frame.to_local(wo);
                if local.z <= 0.0 {
                    return None;
                }
                let sample = lobe.as_bsdf().sample(local, u, lobe_uc)?;
                let wi = frame.to_world(sample.wi);
                if wi.z <= 1e-6 {
                    return None;
                }
                // A delta sample's value is returned as it stands, so it
                // takes the cosine conversion here; anything else is
                // re-evaluated through `eval`, which applies it.
                let f = if sample.is_delta { sample.f * (sample.wi.z / wi.z) } else { sample.f };
                BsdfSample { wi, f, ..sample }
            }
            _ => lobe.as_bsdf().sample(wo, u, lobe_uc)?,
        };
        let probability = self.probabilities[index].max(1e-6);

        if sample.is_delta {
            // A delta lobe's value is already divided by its own PDF, and no
            // other lobe can produce this direction, so the only correction
            // is the probability of having chosen it.
            let through = self.attenuation(*depth, wo, sample.wi);
            return Some(BsdfSample {
                f: sample.f * (*scale * (*weight * through / probability)),
                pdf: 1.0,
                ..sample
            });
        }

        // For everything else, re-evaluate the *whole* stack in the sampled
        // direction. Both the value and the density must describe the mixture,
        // not the component.
        let pdf = self.pdf(wo, sample.wi);
        if pdf <= 0.0 {
            return None;
        }
        let f = self.eval(wo, sample.wi);
        if f.is_black() {
            return None;
        }
        Some(BsdfSample {
            f,
            pdf,
            ..sample
        })
    }

    fn albedo(&self, wo: Vec3) -> Spectrum {
        let mut total = Spectrum::ZERO;
        for (_, (lobe, depth, scale, weight)) in self.active() {
            let through = self.attenuation(*depth, wo, wo);
            let seen = match (depth, &self.flake_frame) {
                (Depth::Flake, Some(frame)) => {
                    let local = frame.to_local(wo);
                    if local.z > 0.0 {
                        lobe.as_bsdf().albedo(local)
                    } else {
                        Spectrum::ZERO
                    }
                }
                _ => lobe.as_bsdf().albedo(wo),
            };
            total += seen * (*scale * (*weight * through));
        }
        Spectrum {
            r: total.r.clamp(0.0, 1.0),
            g: total.g.clamp(0.0, 1.0),
            b: total.b.clamp(0.0, 1.0),
        }
    }

    fn flags(&self) -> BsdfFlags {
        let mut flags = BsdfFlags::empty();
        for (_, (lobe, _, _, _)) in self.active() {
            flags |= lobe.as_bsdf().flags();
        }
        flags
    }
}

#[cfg(test)]
mod tests {
    use crate::fresnel;


    use super::*;
    use crate::fresnel::metals as tequila_core_metals;
    use tequila_core::math::shading;
    use tequila_core::math::vec3;
    use tequila_core::sobol;

    fn direction(theta: f32) -> Vec3 {
        vec3(theta.sin(), 0.0, theta.cos())
    }

    /// The ratio estimator: the mean of `f * cos / pdf` is the hemispherical
    /// reflectance, which for a passive surface cannot exceed one.
    ///
    /// # Why the points come from Sobol and not from a hand-rolled sequence
    ///
    /// This used to draw them from the rank-1 lattice
    /// `((i + 1/2), 7919 i, 4099 i) / 8192`, which looks stratified in each
    /// coordinate separately and is not stratified at all in the pair that
    /// matters. Because `2 * 4099 = 8192 + 6`, the lobe-selection variable is
    /// an *exact function* of the first direction variable: `uc = frac(3 u.x)`
    /// on even indices and `frac(3 u.x + 1/2)` on odd ones, with no residual
    /// term — checked over all 8192 points, not argued.
    ///
    /// The stack picks a lobe from `uc` and a direction from `u`, so that
    /// pairing hands each lobe a `u.x` confined to six narrow bands instead
    /// of the unit interval. It does the most damage exactly where the stack
    /// is hardest to get right: on `white plastic` at 78 degrees the specular
    /// lobe is chosen 4.8% of the time and carries 24% of the reflected
    /// energy, so a quarter of the answer came from 390 points drawn from six
    /// narrow bands of `u.x`. That read **1.0150** against a converged
    /// 1.0002 — and changing only the multiplier `4099`, nothing else, moved
    /// the same measurement anywhere between 0.9725 and 1.0454. The reading
    /// was a property of the multipliers, not of the material.
    ///
    /// The Owen-scrambled Sobol' sequence has no such structure, and its
    /// stratification survives being split across dimensions. Over thirty-two
    /// scrambles, every configuration below stays within 0.0011 of its
    /// converged value at this sample count.
    fn mean_throughput(bsdf: &OpenPbrBsdf, wo: Vec3) -> f32 {
        const SAMPLES: u32 = 16_384;
        const SEED: u32 = 0x5eed_1e55;
        // Accumulated in f64. Sixteen thousand additions into an f32 whose
        // running total is heading for one lose the low bits of every late
        // term, which is worth more than the effect being measured.
        let mut total = 0.0f64;
        for i in 0..SAMPLES {
            let u = Vec2 {
                x: sobol::sample(i, 0, SEED),
                y: sobol::sample(i, 1, SEED),
            };
            let uc = sobol::sample(i, 2, SEED);
            if let Some(sample) = bsdf.sample(wo, u, uc) {
                if sample.pdf > 0.0 {
                    let weight =
                        sample.f.luminance() * shading::abs_cos_theta(sample.wi) / sample.pdf;
                    assert!(weight.is_finite() && weight >= 0.0, "weight {weight}");
                    total += weight as f64;
                }
            }
        }
        (total / SAMPLES as f64) as f32
    }

    /// §2.4's rule, tested directly: the density reported by `sample` must be
    /// the density of the *mixture*. If it were the chosen lobe's own, it
    /// would disagree with `pdf()` evaluated at the same direction — and every
    /// MIS weight in the renderer would be computed against the wrong number.
    #[test]
    fn the_reported_pdf_is_the_mixture_density_not_the_sampled_lobes() {
        let material = Material {
            base_color: Spectrum::splat(0.6),
            specular_roughness: 0.3,
            coat_weight: 1.0,
            fuzz_weight: 0.4,
            ..Default::default()
        };
        let bsdf = OpenPbrBsdf::new(&material, 0.0);
        assert!(bsdf.count >= 3, "this material should build several lobes");

        let wo = direction(0.6);
        let mut checked = 0;
        for i in 0..256 {
            let u = Vec2 {
                x: (i as f32 + 0.5) / 256.0,
                y: ((i * 37) % 256) as f32 / 256.0,
            };
            let uc = ((i * 13) % 256) as f32 / 256.0;
            if let Some(sample) = bsdf.sample(wo, u, uc) {
                if sample.is_delta {
                    continue;
                }
                let independent = bsdf.pdf(wo, sample.wi);
                assert!(
                    (sample.pdf - independent).abs() <= 1e-4 * independent.max(1.0),
                    "sample reported {} but pdf() says {independent}",
                    sample.pdf
                );
                checked += 1;
            }
        }
        assert!(checked > 100, "only {checked} samples were checked");
    }

    /// A passive surface cannot return more light than it receives, whatever
    /// the layers are set to. This is the white-furnace property, applied to
    /// the stack rather than to a single lobe.
    #[test]
    fn no_configuration_of_layers_creates_energy() {
        let flakes = |coverage: f32, degrees: f32| Material {
            base_color: Spectrum::ONE,
            coat_weight: 1.0,
            flake_weight: 1.0,
            flake_color: Spectrum::ONE,
            flake_roughness: 0.1,
            flake_coverage: coverage,
            flake_normal: vec3(degrees.to_radians().sin(), 0.0, degrees.to_radians().cos()),
            ..Default::default()
        };
        let configurations: [(&str, Material); 11] = [
            // Metallic paint on a flake, tilted towards and away from the
            // viewer, and half on one: the flake's own frame must not let it
            // return more than it received.
            ("a flake, tilted 30 degrees", flakes(1.0, 30.0)),
            ("a flake, tilted -55 degrees", flakes(1.0, -55.0)),
            ("half a flake, tilted 40 degrees", flakes(0.5, 40.0)),
            ("plain diffuse", Material {
                specular_weight: 0.0,
                base_color: Spectrum::ONE,
                ..Default::default()
            }),
            ("white metal", Material {
                base_metalness: 1.0,
                specular_roughness: 0.4,
                ..Default::default()
            }),
            // Uncoated, and still a two-layer stack: a dielectric
            // interface over a white diffuse substrate. Adding those two
            // together instead of layering them returned 1.26 at 78 degrees.
            ("white plastic", Material {
                base_color: Spectrum::ONE,
                ..Default::default()
            }),
            ("smooth white plastic", Material {
                base_color: Spectrum::ONE,
                specular_roughness: 0.05,
                ..Default::default()
            }),
            ("coated white plastic", Material {
                base_color: Spectrum::ONE,
                coat_weight: 1.0,
                ..Default::default()
            }),
            ("velvet", Material {
                fuzz_weight: 1.0,
                fuzz_color: Spectrum::ONE,
                fuzz_roughness: 0.8,
                base_color: Spectrum::ONE,
                ..Default::default()
            }),
            ("everything at once", Material {
                base_color: Spectrum::ONE,
                base_metalness: 0.5,
                coat_weight: 1.0,
                coat_color: Spectrum::ONE,
                fuzz_weight: 1.0,
                fuzz_color: Spectrum::ONE,
                specular_roughness: 0.35,
                ..Default::default()
            }),
            ("rough glass", Material {
                transmission_weight: 1.0,
                specular_roughness: 0.25,
                transmission_color: Spectrum::ONE,
                ..Default::default()
            }),
        ];

        for (name, material) in configurations {
            let bsdf = OpenPbrBsdf::new(&material, 0.0);
            for degrees in [10.0f32, 45.0, 78.0] {
                let throughput = mean_throughput(&bsdf, direction(degrees.to_radians()));
                // Half a percent, and it is worth saying what that is made of
                // rather than leaving it as a round number.
                //
                // **Two tenths of a percent is real, and it is the energy
                // table.** `attenuation` hands the substrate `1 - E(wo)` read
                // from the tabulated directional albedo, while the specular
                // lobe reflects whatever it actually reflects. The table is
                // integrated at 2048 quasi-random samples per cell and stored
                // as f32, and near grazing on a roughness-0.3 dielectric it
                // reads about 0.0003 low. Both uses of it then err the same
                // way — the interface keeps a little more than the table
                // admits, and the substrate under it is attenuated by a
                // little less than it should be — so the two add rather than
                // cancel. Measured at a million samples, where the estimator
                // contributes nothing: 1.0002 for `white plastic` at 78
                // degrees, and 1.0008 for `coated white plastic`, which is
                // the worst of the eight.
                //
                // **A tenth of a percent is what 16384 Sobol points are worth
                // here.** Over thirty-two different scrambles, the worst
                // reading of any configuration at any of these angles was
                // 1.0019.
                //
                // The rest is headroom, and deliberately not much.
                //
                // Verified against a deliberate leak rather than assumed:
                // scaling every layer's attenuation by 1.005 fails this on
                // `white plastic` at 78 degrees reading 1.0057, and 1.003
                // still passes. Dropping the outbound crossing from the
                // substrate's attenuation — the bug `attenuation` is
                // written to avoid — fails at 1.0829. The bound was 1.02
                // while the estimator was a lattice that read 1.5% high on a
                // stack that conserves energy to within 0.02%, which left a
                // half-percent leak indistinguishable from the way the
                // measurement was being taken.
                assert!(
                    throughput <= 1.005,
                    "`{name}` at {degrees} degrees returns {throughput:.4} of the light \
                     it receives"
                );
            }
        }
    }

    /// §2.2: `base_color` is "conductor reflectance when metallic", and §3.1
    /// gives the Metal preset a Color control alongside its measured Type.
    ///
    /// The control has to do something. It was exposed by the preset table
    /// while the layer stack ignored it for metals, so the Material tab would
    /// have shown a colour swatch that changed nothing — the kind of bug that
    /// gets reported as "the app is broken" rather than as a shading fault.
    #[test]
    fn a_metals_colour_tints_it_without_erasing_the_measured_metal() {
        let build = |color: Spectrum| Material {
            base_metalness: 1.0,
            metal_ior: tequila_core_metals::GOLD,
            base_color: color,
            specular_roughness: 0.25,
            ..Default::default()
        };
        let wo = direction(35.0f32.to_radians());
        let wi = direction(35.0f32.to_radians()) * -1.0 + Vec3::Z * 2.0 * 0.819;
        let wi = wi.normalize();

        let bare = OpenPbrBsdf::new(&build(Spectrum::ONE), 0.0).eval(wo, wi);
        let tinted = OpenPbrBsdf::new(&build(Spectrum::splat(0.5)), 0.0).eval(wo, wi);
        assert!(!bare.is_black(), "the metal reflected nothing to compare");
        assert!(
            (tinted.g - bare.g * 0.5).abs() < 1e-4 * bare.g.max(1.0),
            "a 50% colour gave {} against a bare {}",
            tinted.g,
            bare.g
        );

        // ...and gold is still gold underneath: a white tint leaves the
        // measured spectrum's warmth intact.
        assert!(
            bare.r > bare.b * 1.5,
            "gold at a white tint came back neutral: {bare:?}"
        );
    }

    /// §2.1 again, one layer lower: the specular interface of a plastic
    /// attenuates the diffuse beneath it rather than being added on top.
    ///
    /// The difference only becomes obvious at grazing angles, where the
    /// interface reflects most of what arrives. If the two lobes were summed,
    /// the diffuse would keep its full brightness there while the reflection
    /// climbed towards 1, and the surface would return more light than it
    /// received — which is exactly what it did.
    /// §2.2: with a depth, `transmission_color` is the colour the *volume*
    /// reaches at that depth — absorption along the path, applied by the
    /// integrator — and not also a tint at every surface crossed. Applied at
    /// both, a clear bottle with three-millimetre walls crossed four times
    /// took the colour to the fourth power and came out solid teal, and an
    /// amber liquid came out nearly black.
    #[test]
    fn a_volumes_colour_is_its_absorption_not_a_tint_at_every_surface() {
        let glass = |colour: Spectrum, depth: f32| Material {
            transmission_weight: 1.0,
            transmission_color: colour,
            transmission_depth: depth,
            ..Default::default()
        };
        let red_absorbing = Spectrum { r: 0.2, g: 0.9, b: 0.9 };
        for degrees in [0.0f32, 45.0] {
            let wo = direction(degrees.to_radians());
            let clear = mean_throughput(&OpenPbrBsdf::new(&glass(Spectrum::ONE, 0.5), 0.0), wo);
            let coloured = mean_throughput(&OpenPbrBsdf::new(&glass(red_absorbing, 0.5), 0.0), wo);
            assert!(
                (clear - coloured).abs() < 1e-3,
                "at {degrees} degrees the surface of a coloured volume passes {coloured:.4} \
                 against a clear one's {clear:.4}; the colour is being applied at the surface"
            );
            // The control: with no depth there is no volume, and the colour
            // is the surface's tint, as it is for thin glass.
            let tinted = mean_throughput(&OpenPbrBsdf::new(&glass(red_absorbing, 0.0), 0.0), wo);
            assert!(tinted < clear - 0.05, "a thin tint no longer tints: {tinted:.4} against {clear:.4}");
        }
    }

    /// Glass with no depth tints light by its colour once per trip through
    /// it, as Cycles does: each of the two crossings by the square root.
    /// Measured in Blender 5.0: a pane of base colour 0.25 passes 0.4993 and
    /// a closed ball 0.2496.
    #[test]
    fn a_surface_tint_is_the_colour_once_per_trip_through() {
        let pane = Material {
            transmission_weight: 1.0,
            transmission_color: Spectrum::splat(0.25),
            specular_ior: 1.0,
            geometry_thin_walled: true,
            ..Default::default()
        };
        let clear = Material { transmission_color: Spectrum::ONE, ..pane.clone() };
        let wo = direction(0.0);
        let ratio = mean_throughput(&OpenPbrBsdf::new(&pane, 0.0), wo) / mean_throughput(&OpenPbrBsdf::new(&clear, 0.0), wo);
        assert!((ratio - 0.5).abs() < 0.02, "one crossing passes {ratio:.4} of a 0.25 tint, not its square root");
    }

    #[test]
    fn the_specular_interface_attenuates_the_diffuse_beneath_it() {
        let material = Material {
            base_color: Spectrum::ONE,
            ..Default::default()
        };
        let bsdf = OpenPbrBsdf::new(&material, 0.0);

        let mut previous = f32::INFINITY;
        for degrees in [5.0f32, 40.0, 65.0, 80.0] {
            let wo = direction(degrees.to_radians());
            let reaching = bsdf.attenuation(Depth::Substrate, wo, wo);
            assert!(
                reaching < previous,
                "at {degrees} degrees {reaching:.4} of the light reaches the \
                 substrate, no less than the {previous:.4} at a steeper angle"
            );
            previous = reaching;
        }
        // A rough interface spreads its Fresnel over the microfacet
        // distribution, so it never approaches a mirror's grazing
        // reflectance. A smooth one does, and the two must not agree.
        let smooth = OpenPbrBsdf::new(
            &Material {
                base_color: Spectrum::ONE,
                specular_roughness: 0.02,
                ..Default::default()
            },
            0.0,
        );
        let grazing = direction(80.0f32.to_radians());
        let through_smooth = smooth.attenuation(Depth::Substrate, grazing, grazing);
        // For a smooth interface the answer is closed-form: the light
        // crosses twice, so `(1 - F)^2`, plus whatever the trapped-light
        // series adds back. Anything near `1 - F` means an outbound crossing
        // was dropped.
        let f = fresnel::dielectric(grazing.z, 1.5);
        let two_crossings = (1.0 - f) * (1.0 - f);
        assert!(
            through_smooth >= two_crossings - 1e-3 && through_smooth <= two_crossings * 1.2,
            "at 80 degrees {through_smooth:.4} reaches the substrate; two \
             crossings of a Fresnel {f:.3} interface predict {two_crossings:.4}"
        );
        assert!(
            through_smooth < previous,
            "roughening the interface from 0.02 to 0.3 did not let more light \
             through at grazing: {previous:.4} against {through_smooth:.4}"
        );
    }

    /// §2.1: each layer attenuates what is beneath it. Adding a coat to a
    /// bright diffuse base must not make the surface brighter overall — that
    /// is the "why is my coated material too dark, or too bright" failure the
    /// spec's opening section warns about, in its other direction.
    #[test]
    fn a_coat_attenuates_the_base_rather_than_adding_to_it() {
        let base = Material {
            base_color: Spectrum::ONE,
            specular_weight: 0.0,
            ..Default::default()
        };
        let coated = Material {
            coat_weight: 1.0,
            ..base.clone()
        };

        let wo = direction(0.4);
        let plain = mean_throughput(&OpenPbrBsdf::new(&base, 0.0), wo);
        let with_coat = mean_throughput(&OpenPbrBsdf::new(&coated, 0.0), wo);

        assert!(
            with_coat <= plain + 0.02,
            "coating a white surface raised its reflectance from {plain:.4} to {with_coat:.4}"
        );
        assert!(
            with_coat > plain * 0.7,
            "the coat swallowed the base: {plain:.4} to {with_coat:.4}"
        );
    }

    /// §2.1 mixes metal and dielectric by weight rather than adding lobes, so
    /// a half-metal must sit between the two ends and not above either.
    #[test]
    fn metalness_interpolates_rather_than_accumulating() {
        let at = |metalness: f32| {
            let material = Material {
                base_color: Spectrum::splat(0.7),
                base_metalness: metalness,
                specular_roughness: 0.3,
                ..Default::default()
            };
            mean_throughput(&OpenPbrBsdf::new(&material, 0.0), direction(0.5))
        };
        let (dielectric, half, metal) = (at(0.0), at(0.5), at(1.0));
        let span = dielectric.max(metal);
        assert!(
            half <= span + 0.02,
            "a half-metal ({half:.4}) reflects more than either end ({dielectric:.4}, {metal:.4})"
        );
    }

    /// A material with nothing switched on has nothing to sample, and must say
    /// so rather than returning a zero-weight sample the integrator would
    /// divide by.
    #[test]
    fn an_empty_material_samples_nothing() {
        let material = Material {
            base_weight: 0.0,
            specular_weight: 0.0,
            ..Default::default()
        };
        let bsdf = OpenPbrBsdf::new(&material, 0.0);
        assert_eq!(bsdf.count, 0);
        assert!(bsdf.sample(direction(0.5), Vec2::splat(0.5), 0.5).is_none());
        assert_eq!(bsdf.pdf(direction(0.5), direction(0.9)), 0.0);
    }

    /// Every lobe must get chosen sometimes, or its contribution is noise that
    /// never converges.
    #[test]
    fn every_lobe_is_reachable_by_the_selector() {
        let material = Material {
            base_color: Spectrum::splat(0.5),
            base_metalness: 0.5,
            coat_weight: 1.0,
            fuzz_weight: 0.5,
            specular_roughness: 0.3,
            ..Default::default()
        };
        let bsdf = OpenPbrBsdf::new(&material, 0.0);
        assert!(bsdf.count >= 4);
        for index in 0..bsdf.count {
            assert!(
                bsdf.probabilities[index] > 0.0,
                "lobe {index} can never be chosen"
            );
        }
        let total: f32 = bsdf.probabilities.iter().take(bsdf.count).sum();
        assert!((total - 1.0).abs() < 1e-5, "probabilities sum to {total}");
    }

    /// A flake is a mirror in its own frame: its highlight is where the
    /// light mirrors about the *flake's* normal, not the surface's. That
    /// shift is the sparkle — each flake lights up at its own angle.
    #[test]
    fn a_tilted_flake_moves_the_highlight() {
        let tilt = 20f32.to_radians();
        let material = Material {
            base_color: Spectrum::splat(0.05),
            flake_weight: 1.0,
            flake_coverage: 1.0,
            flake_roughness: 0.05,
            flake_color: Spectrum::ONE,
            flake_normal: vec3(tilt.sin(), 0.0, tilt.cos()),
            ..Default::default()
        };
        let bsdf = OpenPbrBsdf::new(&material, 0.0);
        let wo = direction(0.0);
        let mirrored_by_flake = vec3((2.0 * tilt).sin(), 0.0, (2.0 * tilt).cos());
        let mirrored_by_surface = vec3(-(2.0 * tilt).sin(), 0.0, (2.0 * tilt).cos());
        let flake = bsdf.eval(wo, mirrored_by_flake).luminance();
        let surface = bsdf.eval(wo, mirrored_by_surface).luminance();
        assert!(flake > surface * 20.0, "the flake's own mirror direction reads {flake}, the surface's {surface}");

        // And the densities still agree: the sampler hands back what pdf()
        // says, through the rotation.
        let mut checked = 0;
        for i in 0..256 {
            let u = Vec2 { x: (i as f32 + 0.5) / 256.0, y: ((i * 37) % 256) as f32 / 256.0 };
            let uc = ((i * 13) % 256) as f32 / 256.0;
            if let Some(sample) = bsdf.sample(direction(0.4), u, uc) {
                if sample.is_delta {
                    continue;
                }
                let independent = bsdf.pdf(direction(0.4), sample.wi);
                assert!((sample.pdf - independent).abs() <= 1e-4 * independent.max(1.0));
                checked += 1;
            }
        }
        assert!(checked > 100, "only {checked} samples were checked");
    }

    #[test]
    fn nothing_in_the_stack_produces_a_non_finite_value() {
        let awkward = [
            // A mirror flake leaning steeply: its reflections run close to
            // the surface, where the cosine conversion divides.
            Material {
                flake_weight: 1.0,
                flake_coverage: 1.0,
                flake_roughness: 0.0,
                flake_normal: vec3(0.8, 0.0, 0.6),
                ..Default::default()
            },
            Material {
                specular_roughness: 0.0,
                transmission_weight: 1.0,
                ..Default::default()
            },
            Material {
                base_metalness: 1.0,
                specular_roughness: 0.0,
                ..Default::default()
            },
            Material {
                fuzz_weight: 1.0,
                fuzz_roughness: 0.0,
                coat_weight: 1.0,
                coat_roughness: 0.0,
                ..Default::default()
            },
        ];
        for material in awkward {
            let bsdf = OpenPbrBsdf::new(&material, 0.0);
            for degrees in [0.5f32, 45.0, 89.5] {
                let wo = direction(degrees.to_radians());
                for i in 0..128 {
                    let u = Vec2 {
                        x: (i as f32 + 0.5) / 128.0,
                        y: ((i * 31) % 128) as f32 / 128.0,
                    };
                    if let Some(sample) = bsdf.sample(wo, u, (i % 7) as f32 / 7.0) {
                        assert!(sample.f.is_finite(), "{sample:?}");
                        assert!(sample.pdf.is_finite() && sample.pdf >= 0.0);
                        assert!(sample.wi.is_finite());
                    }
                }
            }
        }
    }
}
