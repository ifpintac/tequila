//! The OpenPBR surface parameters.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.2, in full. This is a plain parameter
//! block: no behaviour, no branches, no material "type". §1 is emphatic about
//! why —
//!
//! > Tequila implements **one** surface shading model and expresses every
//! > material type as a preset over it. A new material "type" is a data file,
//! > not a new lobe.
//!
//! So there is no `kind` field here deciding which lobes to build. A Metal is
//! `base_metalness = 1`; a Glass is `transmission_weight = 1`; a Paint is a
//! plastic with a coat. The lobes follow from the numbers, and the catalog of
//! `MaterialType` exists only to give
//! the UI a set of defaults and a list of which parameters to show.
//!
//! Keeping the type out of the shading path is what makes the promise true.
//! The moment the BSDF asks "what type is this?", a new type needs new code,
//! and the catalog stops being data.

use tequila_core::math::Vec3;
use tequila_core::spectrum::Spectrum;
use crate::fresnel::{metals, ComplexIor};
use crate::lobes::OpenPbrBsdf;
use crate::material_type::MaterialType;

/// Every parameter of §2.2, grouped as the layer stack of §2.1 orders them.
///
/// Deliberately one flat struct rather than nested per-layer structs: this is
/// what a material graph writes into, what the UI edits field by field, and
/// what the file format serializes name by name. Nesting would buy tidiness in
/// the type and cost it everywhere else.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    // --- base ------------------------------------------------------------
    /// Overall base layer strength.
    pub base_weight: f32,
    /// Diffuse albedo, or conductor reflectance when metallic.
    pub base_color: Spectrum,
    /// 0 = dielectric, 1 = conductor. A **blend**, not a switch: §2.1 mixes
    /// `lerp(dielectric_base, metal, base_metalness)`, which is what makes
    /// glTF's continuous metallic import exactly rather than by threshold.
    pub base_metalness: f32,
    /// Oren-Nayar roughness; 0 is Lambertian.
    pub base_diffuse_roughness: f32,

    // --- specular --------------------------------------------------------
    pub specular_weight: f32,
    /// Tint on the specular reflection. Rarely non-white physically.
    pub specular_color: Spectrum,
    /// Perceptual: GGX α is roughness².
    pub specular_roughness: f32,
    /// 1.0–3.0; 1.5 for plastics and glass.
    pub specular_ior: f32,
    /// Anisotropy magnitude; the direction comes from `geometry_tangent`.
    pub specular_roughness_anisotropy: f32,
    /// Rotation of the anisotropy direction about the normal, 0 to 1 over a
    /// half turn (Autodesk Standard Surface's convention): an anisotropic lobe
    /// turned half way round is the same lobe, so a half turn is every
    /// direction there is. It turns the part's own tangent, from its mesh
    /// tangents or else its UVs.
    ///
    /// Not in §2.2's table, which takes the direction from
    /// `geometry_tangent`. §3.2's Anisotropic type exposes "Anisotropy,
    /// Rotation" as separate controls, and a rotation is far easier to author
    /// than a tangent field.
    pub specular_rotation: f32,
    /// Complex index of refraction for the conductor lobe.
    ///
    /// §2.3 wants both this and the artist-friendly colour + edge-tint
    /// parameterization. [`ComplexIor::from_reflectivity`] converts, so the UI
    /// can offer either and the shading path only ever sees `(n, k)`.
    pub metal_ior: ComplexIor,

    // --- transmission ----------------------------------------------------
    pub transmission_weight: f32,
    /// Transmitted tint, for the thin-walled case.
    pub transmission_color: Spectrum,
    /// Distance at which `transmission_color` is reached by Beer-Lambert
    /// absorption. 0 means thin, with no volume.
    pub transmission_depth: f32,
    /// Volumetric scattering coefficient inside the medium.
    pub transmission_scatter: Spectrum,
    /// Henyey-Greenstein `g` for the medium, -1 to 1.
    pub transmission_scatter_anisotropy: f32,
    /// How thick the medium is, as a multiple of what its colour, depth and
    /// scatter describe: absorption and scattering both scale with it. 1 is
    /// the medium as described, 0 is clear. Not in §2.2's table; §3.2's
    /// Scattering Medium names it, and it is what a Volume Map drives to make
    /// a medium heterogeneous — smoke, cloud, a swirl in a liquid.
    pub volume_density: f32,
    /// Abbe number; 0 disables dispersion. Non-zero triggers spectral mode.
    pub transmission_dispersion_abbe: f32,
    /// Which medium is present where two overlap (`docs/01` §6.1): the
    /// higher number wins, so a liquid modelled a fraction of a millimetre
    /// into its glass fills the glass rather than leaving an air gap, and
    /// the glass's surface inside the liquid is not there at all. Equal
    /// priorities go to whichever was entered last. Whole numbers, 0 to 10.
    pub medium_priority: f32,

    // --- subsurface ------------------------------------------------------
    pub subsurface_weight: f32,
    pub subsurface_color: Spectrum,
    /// Per-channel mean free path, in scene units.
    pub subsurface_radius: Spectrum,
    pub subsurface_scatter_anisotropy: f32,

    // --- coat ------------------------------------------------------------
    pub coat_weight: f32,
    pub coat_color: Spectrum,
    pub coat_roughness: f32,
    pub coat_ior: f32,
    /// Physically-correct darkening of the base under a coat, from total
    /// internal reflection. §2.2: "Keep at 1 for realism."
    pub coat_darkening: f32,

    // --- fuzz ------------------------------------------------------------
    pub fuzz_weight: f32,
    pub fuzz_color: Spectrum,
    pub fuzz_roughness: f32,

    // --- thin film -------------------------------------------------------
    pub thin_film_weight: f32,
    /// Nanometres, 0–2000.
    pub thin_film_thickness: f32,
    pub thin_film_ior: f32,

    // --- flakes ----------------------------------------------------------
    //
    // §3.1's Metallic Paint and §4.4's Flakes: metal platelets suspended in
    // the paint under its clear coat, each tilted a little differently, so a
    // few catch the light at any angle and sparkle. Not in §2.2's OpenPBR
    // block, which has no flake layer; kept beside it the same way occlusion
    // is.
    /// How much of the base is flakes, 0 to 1. Zero turns them off.
    pub flake_weight: f32,
    pub flake_color: Spectrum,
    /// Flake diameter in scene units.
    pub flake_size: f32,
    /// How far flakes tilt from the surface, 0 (all flat, a mirror sheet) to
    /// 1 (up to sixty degrees).
    pub flake_spread: f32,
    pub flake_roughness: f32,
    /// **Per shading point, not a parameter.** Whether this point lies on a
    /// flake (0 or 1, or between at a flake's edge), filled in by the
    /// renderer from the flake field — see `tequila_render_core::flakes`.
    pub flake_coverage: f32,
    /// **Per shading point, not a parameter.** The flake's normal in the
    /// shading frame (`z` is the surface normal).
    pub flake_normal: tequila_core::math::Vec3,

    // --- displacement ----------------------------------------------------
    /// §4.4's Displace: the edge length, in scene units, the surface is
    /// subdivided to before the graph's Displacement height moves it. Zero
    /// is the bump-only approximation — the height shades as a bump and the
    /// silhouette does not change.
    pub displacement_detail: f32,

    // --- emission --------------------------------------------------------
    /// Nits. Physical units, so an emissive panel and an area light of the
    /// same luminance match.
    pub emission_luminance: f32,
    pub emission_color: Spectrum,

    // --- geometry --------------------------------------------------------
    /// Cutout transparency. Not refraction: a ray either hits or passes
    /// straight through, undeviated.
    pub geometry_opacity: f32,
    /// An infinitely thin shell: no volume, no refraction offset. The
    /// difference between a window pane and a solid glass block.
    pub geometry_thin_walled: bool,
    /// How much of the light reaching this point is not blocked by the part's
    /// own shape. 1 is unoccluded.
    ///
    /// **Not an OpenPBR parameter.** §2.2 has no occlusion, and for a good
    /// reason: a path tracer computes occlusion by tracing it, and baking it
    /// into a texture as well counts the same shadow twice.
    ///
    /// It is here because the shadowing a map bakes is usually not shadowing
    /// the tracer can find. An exported asset carries its detail in a normal
    /// map — screw recesses, panel gaps, the channel a strap sits in — and
    /// none of that detail is in the mesh, so no ray can be blocked by it. The
    /// occlusion map is the only record of it that survives the export. Left
    /// unread, every crease in an imported model lights as if it were flat.
    ///
    /// So the double-counting is real and it is the lesser error, and it only
    /// happens where an artist deliberately authored a map. The default is 1,
    /// which changes nothing.
    pub geometry_occlusion: f32,
}

/// A homogeneous scattering medium, as a random walk crosses it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SubsurfaceMedium {
    /// Extinction, per metre, per channel.
    pub extinction: Spectrum,
    /// The fraction of each interaction that scatters rather than absorbs.
    pub albedo: Spectrum,
    /// Henyey-Greenstein `g`: positive scatters forward.
    pub anisotropy: f32,
}

/// The thickness of clear material the quick previews assume, in metres:
/// the wall of a bottle or a moulded housing.
pub const PREVIEW_WALL: f32 = 0.004;

impl Default for Material {
    /// A mid-grey dielectric — the "unassigned" look, and a safe base for a
    /// preset to override a handful of fields on.
    fn default() -> Self {
        Material {
            base_weight: 1.0,
            base_color: Spectrum::splat(0.18),
            base_metalness: 0.0,
            base_diffuse_roughness: 0.0,

            specular_weight: 1.0,
            specular_color: Spectrum::ONE,
            specular_roughness: 0.3,
            specular_ior: 1.5,
            specular_roughness_anisotropy: 0.0,
            specular_rotation: 0.0,
            metal_ior: metals::ALUMINIUM,

            transmission_weight: 0.0,
            transmission_color: Spectrum::ONE,
            transmission_depth: 0.0,
            transmission_scatter: Spectrum::ZERO,
            transmission_scatter_anisotropy: 0.0,
            volume_density: 1.0,
            transmission_dispersion_abbe: 0.0,
            medium_priority: 0.0,

            subsurface_weight: 0.0,
            subsurface_color: Spectrum::splat(0.8),
            subsurface_radius: Spectrum::splat(1.0),
            subsurface_scatter_anisotropy: 0.0,

            coat_weight: 0.0,
            coat_color: Spectrum::ONE,
            coat_roughness: 0.02,
            coat_ior: 1.5,
            coat_darkening: 1.0,

            fuzz_weight: 0.0,
            fuzz_color: Spectrum::ONE,
            fuzz_roughness: 0.3,

            thin_film_weight: 0.0,
            thin_film_thickness: 500.0,
            thin_film_ior: 1.4,

            flake_weight: 0.0,
            flake_color: Spectrum::splat(0.9),
            flake_size: 0.00005,
            flake_spread: 0.3,
            flake_roughness: 0.3,
            flake_coverage: 0.0,
            flake_normal: tequila_core::math::Vec3::Z,

            displacement_detail: 0.0005,

            emission_luminance: 0.0,
            emission_color: Spectrum::ONE,

            geometry_opacity: 1.0,
            geometry_thin_walled: false,
            geometry_occlusion: 1.0,
        }
    }
}

impl Material {
    /// The BSDF for a shading point.
    ///
    /// `roughness_floor` implements path regularization (`docs/01` §3.5): the
    /// integrator raises it after the first couple of bounces so that
    /// near-specular lobes deep in a path stop generating fireflies. It can
    /// only ever raise a roughness, never lower one.
    ///
    /// This is the seam between the stored parameter block and the layer
    /// stack. Everything above it deals in §2.2 parameters; everything below
    /// deals in lobes.
    #[inline]
    pub fn bsdf(&self, roughness_floor: f32) -> OpenPbrBsdf {
        OpenPbrBsdf::new(self, roughness_floor)
    }

    /// The BSDF for a shading point a path is arriving at from *inside* the
    /// material.
    ///
    /// A separate entry point rather than an argument, because only a
    /// transmissive surface has two sides worth telling apart and almost
    /// every caller is on the outside of one.
    #[inline]
    pub fn bsdf_from_inside(&self, roughness_floor: f32) -> OpenPbrBsdf {
        OpenPbrBsdf::from_inside(self, roughness_floor)
    }

    /// The catalog preset of §3.1, as a starting parameter block.
    ///
    /// Every "material type" is a set of defaults over this one block — §1's
    /// central decision, and the reason adding a type is a data change rather
    /// than a code change.
    #[inline]
    pub fn preset(material_type: MaterialType) -> Material {
        material_type.defaults()
    }

    /// §3.1 Diffuse.
    pub fn diffuse(color: Spectrum) -> Material {
        Material {
            base_color: color,
            ..MaterialType::Diffuse.defaults()
        }
    }

    /// §3.1 Metal.
    pub fn metal(metal_ior: ComplexIor, roughness: f32) -> Material {
        Material {
            metal_ior,
            specular_roughness: roughness,
            ..MaterialType::Metal.defaults()
        }
    }

    /// §3.1 Plastic: a dielectric interface over a diffuse substrate.
    pub fn plastic(color: Spectrum, roughness: f32) -> Material {
        Material {
            base_color: color,
            specular_roughness: roughness,
            ..MaterialType::Plastic.defaults()
        }
    }

    /// §3.3 Emissive, in nits.
    pub fn emissive(color: Spectrum, luminance: f32) -> Material {
        Material {
            emission_color: color,
            emission_luminance: luminance,
            ..MaterialType::Emissive.defaults()
        }
    }

    /// Emission in nits, as the integrator wants it.
    #[inline]
    pub fn emission(&self) -> Spectrum {
        self.emission_color * self.emission_luminance
    }

    #[inline]
    pub fn is_emissive(&self) -> bool {
        self.emission_luminance > 0.0 && !self.emission_color.is_black()
    }

    /// Whether the surface transmits, and so needs the refraction machinery.
    #[inline]
    pub fn is_transmissive(&self) -> bool {
        self.transmission_weight > 0.0
    }

    /// How much of each wavelength survives `distance` inside this material.
    ///
    /// §2.2 defines `transmission_depth` as "the distance at which
    /// `transmission_color` is reached via Beer-Lambert absorption", which
    /// makes the whole thing one expression: the transmittance over `d` is
    /// `transmission_color` raised to `d / transmission_depth`. Equivalent to
    /// `exp(-σₐ·d)` with `σₐ = -ln(colour) / depth`, and better behaved —
    /// nothing has to divide by a log or special-case a channel at zero.
    ///
    /// This is what separates a glass block from a clear one. A pane is thin
    /// and `transmission_depth = 0` means no volume at all, so the tint stays
    /// on the surface where `transmission_color` already put it; a solid block
    /// has to get darker and more saturated the further light travels through
    /// it, which is the whole reason a thick piece of green glass looks
    /// nothing like a thin one.
    pub fn transmittance_over(&self, distance: f32) -> Spectrum {
        if self.transmission_depth <= 0.0 || distance <= 0.0 {
            return Spectrum::ONE;
        }
        let exponent = distance * self.volume_density.max(0.0) / self.transmission_depth;
        // A channel at zero absorbs completely, and `powf` gives exactly that
        // for any positive exponent — no floor, which would leave a black
        // channel passing a sliver of light for ever. The guard is only
        // against a non-finite colour, which the sanitizer should already have
        // removed but which would otherwise reach the accumulation buffer.
        let survive = |c: f32| {
            if c.is_finite() {
                c.clamp(0.0, 1.0).powf(exponent)
            } else {
                0.0
            }
        };
        Spectrum {
            r: survive(self.transmission_color.r),
            g: survive(self.transmission_color.g),
            b: survive(self.transmission_color.b),
        }
    }

    /// The medium a random walk crosses under this surface (`docs/01` §6.4):
    /// extinction per metre, single-scattering albedo and phase anisotropy,
    /// or `None` when nothing is scattered under the surface.
    ///
    /// `subsurface_color` is what the walk should give back and
    /// `subsurface_radius` how far light goes inside, per channel, as
    /// Cycles' Principled BSDF reads its Subsurface Radius - so a material
    /// shades the same in both of Tequila's engines. They are turned into
    /// coefficients as Cycles does: the radius scaled by `1 / 4 pi` into a
    /// mean free path (`bssrdf_setup_radius`, which keeps the random walk
    /// looking like the diffusion profiles before it), then
    /// `subsurface_random_walk_remap` (Apache-2.0) - [Chiang, Kutz & Burley
    /// 2016]'s fit, which gives the albedo a single scattering event needs
    /// for the multiply scattered result to be the colour asked for, at the
    /// phase function's anisotropy, and an extinction of one over the mean
    /// free path, reduced for that anisotropy.
    ///
    /// It used to take the radius for a profile width instead, dividing the
    /// extinction by Christensen & Burley's `1.9 - a + 3.5 (a - 0.8)^2`: a
    /// medium twenty to fifty times thinner than Cycles', the darker the
    /// thinner. A near-black stone with a five-centimetre radius rendered
    /// brighter than a grey one, seen through.
    pub fn subsurface_medium(&self) -> Option<SubsurfaceMedium> {
        if self.subsurface_weight <= 0.0 {
            return None;
        }
        let g = self.subsurface_scatter_anisotropy.clamp(-0.95, 0.95);
        let per_channel = |colour: f32, radius: f32| walk_coefficients(colour.clamp(0.0, 1.0), radius, g);
        let (r, gg, b) = (
            per_channel(self.subsurface_color.r, self.subsurface_radius.r),
            per_channel(self.subsurface_color.g, self.subsurface_radius.g),
            per_channel(self.subsurface_color.b, self.subsurface_radius.b),
        );
        Some(SubsurfaceMedium {
            extinction: Spectrum { r: r.0, g: gg.0, b: b.0 },
            albedo: Spectrum { r: r.1, g: gg.1, b: b.1 },
            anisotropy: g,
        })
    }

    /// The scattering inside a clear part's volume — a milky liquid, a
    /// smoky glass — or `None` where the volume only absorbs.
    ///
    /// §2.2: the transmission colour, reached at the depth, sets how much the
    /// volume absorbs, and `transmission_scatter` is its scattering
    /// coefficient in inverse depths. Extinction is the two together, so a
    /// scattering volume is not also absorbed along the path by its colour:
    /// the walk does both.
    pub fn volume_scattering(&self) -> Option<SubsurfaceMedium> {
        if !self.has_volume() || self.transmission_depth <= 0.0 || self.transmission_scatter.is_black() {
            return None;
        }
        // Density thickens the medium: the same colour and scatter reached in
        // a shorter distance.
        let depth = self.transmission_depth / self.volume_density.max(1e-6);
        let per_channel = |colour: f32, scatter: f32| -> (f32, f32) {
            let absorption = -colour.clamp(1e-4, 1.0).ln() / depth;
            let scattering = scatter.max(0.0) / depth;
            let extinction = absorption + scattering;
            (extinction, if extinction > 0.0 { scattering / extinction } else { 0.0 })
        };
        let (c, s) = (self.transmission_color, self.transmission_scatter);
        let (r, g, b) = (per_channel(c.r, s.r), per_channel(c.g, s.g), per_channel(c.b, s.b));
        Some(SubsurfaceMedium {
            extinction: Spectrum { r: r.0, g: g.0, b: b.0 },
            albedo: Spectrum { r: r.1, g: g.1, b: b.1 },
            anisotropy: self.transmission_scatter_anisotropy.clamp(-0.95, 0.95),
        })
    }

    /// The colour a quick preview shades the surface with: the base, and
    /// where light goes under the surface, the colour the walk comes back
    /// with, by weight. The previews do not walk, and a translucent part
    /// whose look is all subsurface would otherwise show its base colour.
    pub fn apparent_color(&self) -> Spectrum {
        let s = self.subsurface_weight.clamp(0.0, 1.0);
        self.base_color * (1.0 - s) + self.subsurface_color * s
    }

    /// The colour the quick previews show a material as: its apparent
    /// colour, or for a metal its measured reflectance at normal incidence
    /// tinted by the base colour - the colour a metal is seen to be. A gold
    /// is a white base over gold's n and k, and showed white.
    pub fn solid_color(&self) -> Spectrum {
        let metal = self.metal_ior.normal_incidence_reflectance() * self.base_color;
        self.apparent_color().lerp(metal, self.base_metalness.clamp(0.0, 1.0))
    }

    /// The colour the quick previews let the backdrop through a clear part
    /// with: the surface tint of a piece with no volume, and for one with a
    /// volume, what a wall of [`PREVIEW_WALL`] neither absorbs nor scatters
    /// aside. The previews do not trace the thickness a path crosses, and a
    /// bottle's wall is what most clear parts are.
    pub fn see_through_tint(&self) -> Spectrum {
        if self.has_volume() && self.transmission_depth > 0.0 {
            let scattered =
                self.transmission_scatter * (PREVIEW_WALL * self.volume_density.max(0.0) / self.transmission_depth);
            let unscattered = Spectrum {
                r: (-scattered.r).exp(),
                g: (-scattered.g).exp(),
                b: (-scattered.b).exp(),
            };
            self.transmittance_over(PREVIEW_WALL) * unscattered
        } else {
            self.transmission_color
        }
    }

    /// What the quick previews show a scattering volume sending back, per
    /// channel, before it is lit: none for a clear one, and for milk nearly
    /// white. The share of a [`PREVIEW_WALL`] the light meets something in,
    /// times the albedo of a thick layer of the stuff — van de Hulst's
    /// similarity relation from the single-scattering albedo and the
    /// anisotropy, so that orange juice, which absorbs blue at every one of
    /// its many scattering events, comes back orange and not pale.
    pub fn preview_cloud(&self) -> Spectrum {
        if !self.has_volume() || self.transmission_depth <= 0.0 || self.transmission_scatter.is_black() {
            return Spectrum::ZERO;
        }
        let depth = self.transmission_depth / self.volume_density.max(1e-6);
        let g = self.transmission_scatter_anisotropy.clamp(-0.95, 0.95);
        let per_channel = |colour: f32, scatter: f32| {
            let sigma_s = scatter.max(0.0) / depth;
            let sigma_a = -colour.clamp(1e-6, 1.0).ln() / depth;
            let sigma_t = sigma_s + sigma_a;
            if sigma_t <= 0.0 {
                return 0.0;
            }
            let albedo = sigma_s / sigma_t;
            let s = ((1.0 - albedo) / (1.0 - g * albedo)).max(0.0).sqrt();
            let thick = (1.0 - s) * (1.0 - 0.139 * s) / (1.0 + 1.17 * s);
            thick * (1.0 - (-sigma_t * PREVIEW_WALL).exp())
        };
        let (c, s) = (self.transmission_color, self.transmission_scatter);
        Spectrum {
            r: per_channel(c.r, s.r),
            g: per_channel(c.g, s.g),
            b: per_channel(c.b, s.b),
        }
    }

    /// Whether a volume is attached: §2.2's `transmission_depth = 0` means
    /// "thin, no volume", and a thin-walled surface never has one.
    #[inline]
    pub fn has_volume(&self) -> bool {
        !self.geometry_thin_walled
            && self.transmission_weight > 0.0
            && (self.transmission_depth > 0.0 || !self.transmission_scatter.is_black())
    }

    /// Clamp every parameter into the range §2.2 states.
    ///
    /// Applied where values enter from outside — a file, an importer, a
    /// script — rather than trusted. A negative roughness or an IOR below 1
    /// does not produce a strange-looking material; it produces NaNs that
    /// spread through the framebuffer and are then very hard to trace back.
    pub fn sanitized(mut self) -> Material {
        let unit = |v: f32| {
            if v.is_finite() {
                v.clamp(0.0, 1.0)
            } else {
                0.0
            }
        };
        let positive = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
        let signed = |v: f32| {
            if v.is_finite() {
                v.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        };
        let colour = |c: Spectrum| {
            if c.r.is_finite() && c.g.is_finite() && c.b.is_finite() {
                Spectrum {
                    r: c.r.max(0.0),
                    g: c.g.max(0.0),
                    b: c.b.max(0.0),
                }
            } else {
                Spectrum::ZERO
            }
        };

        self.base_weight = unit(self.base_weight);
        self.base_color = colour(self.base_color);
        self.base_metalness = unit(self.base_metalness);
        self.base_diffuse_roughness = unit(self.base_diffuse_roughness);

        self.specular_weight = unit(self.specular_weight);
        self.specular_color = colour(self.specular_color);
        self.specular_roughness = unit(self.specular_roughness);
        self.specular_ior = if self.specular_ior.is_finite() {
            self.specular_ior.clamp(1.0, 3.0)
        } else {
            1.5
        };
        self.specular_roughness_anisotropy = unit(self.specular_roughness_anisotropy);
        self.specular_rotation = if self.specular_rotation.is_finite() {
            self.specular_rotation
        } else {
            0.0
        };

        self.transmission_weight = unit(self.transmission_weight);
        self.transmission_color = colour(self.transmission_color);
        self.transmission_depth = positive(self.transmission_depth);
        self.transmission_scatter = colour(self.transmission_scatter);
        self.transmission_scatter_anisotropy = signed(self.transmission_scatter_anisotropy);
        self.volume_density = if self.volume_density.is_finite() { self.volume_density.max(0.0) } else { 1.0 };
        self.transmission_dispersion_abbe = if self.transmission_dispersion_abbe.is_finite() {
            self.transmission_dispersion_abbe.clamp(0.0, 100.0)
        } else {
            0.0
        };
        self.medium_priority = if self.medium_priority.is_finite() {
            self.medium_priority.round().clamp(0.0, 10.0)
        } else {
            0.0
        };

        self.subsurface_weight = unit(self.subsurface_weight);
        self.subsurface_color = colour(self.subsurface_color);
        self.subsurface_radius = colour(self.subsurface_radius);
        self.subsurface_scatter_anisotropy = signed(self.subsurface_scatter_anisotropy);

        self.coat_weight = unit(self.coat_weight);
        self.coat_color = colour(self.coat_color);
        self.coat_roughness = unit(self.coat_roughness);
        self.coat_ior = if self.coat_ior.is_finite() {
            self.coat_ior.clamp(1.0, 2.0)
        } else {
            1.5
        };
        self.coat_darkening = unit(self.coat_darkening);

        self.fuzz_weight = unit(self.fuzz_weight);
        self.fuzz_color = colour(self.fuzz_color);
        self.fuzz_roughness = unit(self.fuzz_roughness);

        self.thin_film_weight = unit(self.thin_film_weight);
        self.thin_film_thickness = if self.thin_film_thickness.is_finite() {
            self.thin_film_thickness.clamp(0.0, 2000.0)
        } else {
            0.0
        };
        self.thin_film_ior = if self.thin_film_ior.is_finite() {
            self.thin_film_ior.clamp(1.0, 3.0)
        } else {
            1.4
        };

        self.flake_weight = unit(self.flake_weight);
        self.flake_color = colour(self.flake_color);
        self.flake_size = if self.flake_size.is_finite() && self.flake_size > 0.0 {
            self.flake_size
        } else {
            0.00005
        };
        self.flake_spread = unit(self.flake_spread);
        self.flake_roughness = unit(self.flake_roughness);
        self.flake_coverage = unit(self.flake_coverage);
        self.displacement_detail = if self.displacement_detail.is_finite() {
            self.displacement_detail.max(0.0)
        } else {
            0.0005
        };
        self.flake_normal = self
            .flake_normal
            .try_normalize()
            .filter(|n| n.z > 0.0 && n.x.is_finite())
            .unwrap_or(tequila_core::math::Vec3::Z);

        self.emission_luminance = positive(self.emission_luminance);
        self.emission_color = colour(self.emission_color);

        self.geometry_opacity = unit(self.geometry_opacity);
        self.geometry_occlusion = unit(self.geometry_occlusion);
        self
    }

    /// A rough hemispherical reflectance, for the denoiser's albedo guide and
    /// for light-selection heuristics.
    ///
    /// Approximate on purpose: it is a guide, not a rendering. It must never
    /// exceed 1, or Russian roulette stops terminating paths.
    pub fn approximate_albedo(&self) -> Spectrum {
        let metal = self.base_color;
        let dielectric = self.base_color * (1.0 - self.transmission_weight)
            + self.transmission_color * self.transmission_weight;
        let base = dielectric * (1.0 - self.base_metalness) + metal * self.base_metalness;
        let with_fuzz =
            base * (1.0 - self.fuzz_weight * 0.5) + self.fuzz_color * (self.fuzz_weight * 0.5);
        let albedo = with_fuzz * self.base_weight * (1.0 - self.coat_weight * 0.1)
            + self.coat_color * (self.coat_weight * 0.1);
        Spectrum {
            r: albedo.r.clamp(0.0, 1.0),
            g: albedo.g.clamp(0.0, 1.0),
            b: albedo.b.clamp(0.0, 1.0),
        }
    }
}

/// Per-shading-point inputs that come from the geometry rather than the
/// material: §2.2's `geometry_normal`, `geometry_coat_normal` and
/// `geometry_tangent`.
///
/// Separate from [`Material`] because they vary per hit while the material
/// does not — and because a material shared by four thousand parts must not
/// carry any one part's normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceFrame {
    /// Shading normal after bump or normal mapping.
    pub normal: Vec3,
    /// An independent normal for the coat, so orange peel can sit over a
    /// smooth base.
    pub coat_normal: Vec3,
    /// Anisotropy direction.
    pub tangent: Vec3,
}

impl SurfaceFrame {
    pub fn new(normal: Vec3, tangent: Vec3) -> SurfaceFrame {
        SurfaceFrame {
            normal,
            coat_normal: normal,
            tangent,
        }
    }
}

/// A random walk's extinction and single-scattering albedo for a surface
/// albedo `albedo`, a mean free path `radius` and an anisotropy `g`.
///
/// Ported from Cycles' `subsurface_random_walk_remap`
/// (`src/kernel/integrator/subsurface_random_walk.h`, SPDX-FileCopyrightText:
/// 2011-2022 Blender Foundation, Apache-2.0), which is the fit of [Chiang,
/// Kutz & Burley 2016, "Practical and Controllable Subsurface Scattering for
/// Production Path Tracing"] extended to anisotropy by Wrenninge, Villemin &
/// Hery. See THIRD_PARTY_LICENSES.md.
fn walk_coefficients(albedo: f32, radius: f32, g: f32) -> (f32, f32) {
    let (g2, g3, g4, g5, g6, g7) = (g * g, g * g * g, g.powi(4), g.powi(5), g.powi(6), g.powi(7));
    let a = 1.8260523782 - 1.28451056436 * g - 1.79904629312 * g2 + 9.19393289202 * g3 - 22.8215585862 * g4
        + 32.0234874259 * g5 - 23.6264803333 * g6 + 7.21067002658 * g7;
    let b = 4.98511194385
        + 0.127355959438
            * (31.1491581433 * g - 201.847017512 * g2 + 841.576016723 * g3 - 2018.09288505 * g4 + 2731.71560286 * g5
                - 1935.41424244 * g6
                + 559.009054474 * g7)
                .exp();
    let c = 1.09686102424 - 0.394704063468 * g + 1.05258115941 * g2 - 8.83963712726 * g3 + 28.8643230661 * g4
        - 46.8802913581 * g5 + 38.5402837518 * g6 - 12.7181042538 * g7;
    let d = 0.496310210422 + 0.360146581622 * g - 2.15139309747 * g2 + 17.8896899217 * g3 - 55.2984010333 * g4
        + 82.065982243 * g5 - 58.5106008578 * g6 + 15.8478295021 * g7;
    let e = 4.23190299701
        + 0.00310603949088
            * (76.7316253952 * g - 594.356773233 * g2 + 2448.8834203 * g3 - 5576.68528998 * g4 + 7116.60171912 * g5
                - 4763.54467887 * g6
                + 1303.5318055 * g7)
                .exp();
    let f = 2.40602999408 - 2.51814844609 * g + 9.18494908356 * g2 - 79.2191708682 * g3 + 259.082868209 * g4
        - 403.613804597 * g5 + 302.85712436 * g6 - 87.4370473567 * g7;
    let blend = albedo.powf(0.25);
    let alpha = (1.0 - blend) * a * (b * albedo).atan().powf(c) + blend * d * (e * albedo).atan().powf(f);
    let mean_free_path = radius * 0.25 * std::f32::consts::FRAC_1_PI;
    let extinction = 1.0 / mean_free_path.max(1e-16) / (1.0 - g);
    (extinction, alpha.clamp(0.0, 0.999_999))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Density thickens a medium and does nothing else: twice the density
    /// over a distance is the medium as described over twice the distance,
    /// for what it absorbs along a path and for what the walk scatters, and
    /// zero is clear.
    #[test]
    fn density_is_thickness() {
        let medium = crate::material_type::MaterialType::ScatteringMedium.defaults();
        let thick = Material { volume_density: 2.0, ..medium.clone() };
        let absorbing = Material { transmission_scatter: Spectrum::ZERO, transmission_color: Spectrum { r: 0.5, g: 0.6, b: 0.7 }, ..medium.clone() };
        let thick_absorbing = Material { volume_density: 2.0, ..absorbing.clone() };
        let (a, b) = (thick_absorbing.transmittance_over(0.1), absorbing.transmittance_over(0.2));
        assert!((a.r - b.r).abs() < 1e-6 && (a.b - b.b).abs() < 1e-6, "{a:?} against {b:?}");

        let (one, two) = (medium.volume_scattering().unwrap(), thick.volume_scattering().unwrap());
        assert!((two.extinction.g - 2.0 * one.extinction.g).abs() < 1e-3 * one.extinction.g);
        assert!((two.albedo.g - one.albedo.g).abs() < 1e-6, "density changed the albedo");

        let clear = Material { volume_density: 0.0, ..absorbing };
        assert_eq!(clear.transmittance_over(5.0), Spectrum::ONE);
    }

    #[test]
    fn the_default_is_a_plain_dielectric() {
        let material = Material::default();
        assert_eq!(material.base_metalness, 0.0);
        assert_eq!(material.transmission_weight, 0.0);
        assert_eq!(material.coat_weight, 0.0);
        assert_eq!(material.fuzz_weight, 0.0);
        assert!(!material.is_emissive());
        assert!(!material.has_volume());
    }

    /// A NaN roughness does not make a strange-looking surface — it makes NaN
    /// pixels that spread through the framebuffer and are hard to trace back
    /// to their source.
    #[test]
    fn sanitizing_replaces_every_non_finite_value() {
        let broken = Material {
            base_weight: f32::NAN,
            specular_roughness: f32::INFINITY,
            specular_ior: -1.0,
            coat_ior: f32::NAN,
            transmission_depth: f32::NEG_INFINITY,
            emission_luminance: f32::NAN,
            base_color: Spectrum {
                r: f32::NAN,
                g: 0.5,
                b: 0.5,
            },
            subsurface_scatter_anisotropy: 5.0,
            thin_film_thickness: 1.0e9,
            ..Default::default()
        }
        .sanitized();

        assert!(broken.base_weight.is_finite());
        assert!((0.0..=1.0).contains(&broken.specular_roughness));
        assert!((1.0..=3.0).contains(&broken.specular_ior));
        assert!((1.0..=2.0).contains(&broken.coat_ior));
        assert_eq!(broken.transmission_depth, 0.0);
        assert_eq!(broken.emission_luminance, 0.0);
        assert!(broken.base_color.r.is_finite());
        assert_eq!(broken.subsurface_scatter_anisotropy, 1.0);
        assert_eq!(broken.thin_film_thickness, 2000.0);
    }

    /// A translucent part previews in the colour its walk returns, not its
    /// base colour, which a fully subsurface material barely shows.
    #[test]
    fn a_preview_shows_a_translucent_part_in_its_subsurface_colour() {
        let wax = Material {
            base_color: Spectrum::splat(0.5),
            subsurface_weight: 1.0,
            subsurface_color: Spectrum { r: 0.9, g: 0.7, b: 0.4 },
            ..Default::default()
        };
        assert_eq!(wax.apparent_color(), wax.subsurface_color);
        let plain = Material { subsurface_weight: 0.0, ..wax.clone() };
        assert_eq!(plain.apparent_color(), plain.base_color);
    }

    /// What the quick previews tint the view through a clear part by. With
    /// a depth, the transmission colour is what the *volume* reaches at that
    /// depth, and painting it on as a surface tint showed amber bottle glass
    /// — whose colour is reached over two centimetres — as solid orange. The
    /// previews cannot trace the thickness, so they assume a bottle's wall.
    #[test]
    fn a_preview_sees_through_a_wall_of_the_glass_not_its_depth() {
        let amber = Material {
            transmission_weight: 1.0,
            transmission_color: Spectrum { r: 0.55, g: 0.2, b: 0.023 },
            transmission_depth: 0.02,
            ..Default::default()
        };
        let tint = amber.see_through_tint();
        assert!(tint.b > 0.3 && tint.r > tint.g && tint.g > tint.b, "a wall of amber glass tints {tint:?}");
        // A thin pane has no volume: its colour is its tint, as before.
        let pane = Material { transmission_depth: 0.0, ..amber.clone() };
        assert_eq!(pane.see_through_tint(), pane.transmission_color);
    }

    /// Sanitizing a sensible material must not change it, or every load and
    /// import would quietly drift the numbers.
    #[test]
    fn sanitizing_leaves_a_valid_material_alone() {
        let material = Material {
            base_color: Spectrum::splat(0.5),
            specular_roughness: 0.25,
            coat_weight: 1.0,
            transmission_dispersion_abbe: 55.0,
            ..Default::default()
        };
        assert_eq!(material.clone().sanitized(), material);
    }

    /// Russian roulette divides by this. Above 1 and paths stop terminating.
    #[test]
    fn the_albedo_guide_never_exceeds_one() {
        for material in [
            Material {
                base_color: Spectrum::splat(4.0),
                ..Default::default()
            },
            Material {
                base_color: Spectrum::ONE,
                coat_weight: 1.0,
                fuzz_weight: 1.0,
                coat_color: Spectrum::splat(3.0),
                ..Default::default()
            },
        ] {
            let albedo = material.approximate_albedo();
            for channel in [albedo.r, albedo.g, albedo.b] {
                assert!((0.0..=1.0).contains(&channel), "albedo {albedo:?}");
            }
        }
    }

    /// §2.2: `transmission_depth = 0` means thin, with no volume — and a
    /// thin-walled surface never has one however the other numbers read.
    #[test]
    fn a_volume_needs_depth_and_a_solid_surface() {
        let solid_glass = Material {
            transmission_weight: 1.0,
            transmission_depth: 0.05,
            ..Default::default()
        };
        assert!(solid_glass.has_volume());

        let window = Material {
            geometry_thin_walled: true,
            ..solid_glass.clone()
        };
        assert!(!window.has_volume(), "a thin shell has no interior");

        let no_depth = Material {
            transmission_depth: 0.0,
            ..solid_glass
        };
        assert!(!no_depth.has_volume());
    }
}
