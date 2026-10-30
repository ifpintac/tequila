//! The material catalog of `docs/02-MATERIAL-SYSTEM.md` §3.
//!
//! Thirty-four types, plus four light types. Each is a set of defaults over
//! the one [`Material`] parameter block, and a list of which parameters the UI
//! shows for it.
//!
//! # This is data, and it is meant to stay data
//!
//! §1: "A new material 'type' is a data file, not a new lobe." Nothing here
//! reaches the shading path — the BSDF is built from parameter values alone
//! and never asks what type produced them. A type is two things and no more:
//! a starting point, and an answer to "which controls does this material
//! deserve?".
//!
//! That second part is the one worth caring about. §3's tables give each type
//! an exposed-parameter set, and the reason is stated in `TEQUILA_SPEC.md`
//! §1.4: a Metal that offered a Transmission Depth slider would be confusing,
//! and an Advanced material that hid one would be useless. The catalog is
//! where the difference lives.

use crate::openpbr::Material;
use tequila_core::spectrum::{spectrum, Spectrum};
use crate::fresnel::{metals, ComplexIor};

/// A parameter the UI can show, and the catalog can expose or hide.
///
/// Named individually rather than by string so that a preset naming a
/// parameter that does not exist fails to compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Param {
    BaseWeight,
    BaseColor,
    BaseMetalness,
    DiffuseRoughness,
    SpecularWeight,
    SpecularColor,
    SpecularRoughness,
    SpecularIor,
    Anisotropy,
    AnisotropyRotation,
    /// OpenPBR's `geometry_tangent`: the way an anisotropic lobe is rougher,
    /// before Rotation turns it - a direction in the part's own space, as
    /// the Object texture coordinate is, fed by the material graph. Unwired
    /// (or zero where it is wired) the surface's own tangent serves: along
    /// its UVs, else the frame its normal gives. A pattern that brushes its
    /// lines one way says so here, and the highlight follows the lines on
    /// every part, with UVs or without.
    Tangent,
    MetalPreset,
    TransmissionWeight,
    TransmissionColor,
    TransmissionDepth,
    /// The colour the surface itself gives light passing through it, the
    /// square root at each crossing so that in and out again is the colour
    /// once - Cycles' principled glass's base colour. With a depth, on top
    /// of what the volume absorbs: a tinted plastic is coloured both at its
    /// skin and through its thickness, and its thin edges are not clear.
    TransmissionTint,
    TransmissionScatter,
    ScatterAnisotropy,
    VolumeDensity,
    Abbe,
    MediumPriority,
    SubsurfaceWeight,
    SubsurfaceColor,
    SubsurfaceRadius,
    SubsurfaceAnisotropy,
    CoatWeight,
    CoatColor,
    CoatRoughness,
    CoatIor,
    CoatDarkening,
    FuzzWeight,
    FuzzColor,
    FuzzRoughness,
    /// §4.4's Fuzz, geometric: fibres to the square centimetre, grown out of
    /// the surface when the scene is built; 0 grows none. The fibres wear
    /// the Sheen Color and Sheen Roughness, and the sheen stays under them.
    FuzzDensity,
    FuzzLength,
    FuzzThickness,
    FuzzLean,
    FuzzCurl,
    FuzzColorVariation,
    ThinFilmWeight,
    ThinFilmThickness,
    ThinFilmIor,
    FlakeWeight,
    FlakeColor,
    FlakeSize,
    FlakeSpread,
    FlakeRoughness,
    EmissionLuminance,
    EmissionColor,
    Opacity,
    ThinWalled,
    /// Baked ambient occlusion. Not one of §2.2's; see
    /// [`Material::geometry_occlusion`](crate::openpbr::Material::geometry_occlusion)
    /// for why it is here anyway.
    Occlusion,
    Bump,
    /// §4.4's Displace: a height in scene units, fed by the graph like Bump,
    /// that moves the geometry itself — silhouettes and shadows change, not
    /// only the shading.
    Displacement,
    DisplacementDetail,
    /// Rounded edges at render time: sharp CAD edges shaded as if filleted
    /// to this radius, in metres, without touching the geometry - V-Ray's
    /// rounded corners, Cycles' Bevel. 0 is sharp. A large part of why a
    /// product's edges and chamfers catch the light.
    EdgeRadius,
    /// How many probes the rounding takes per shading point.
    EdgeSamples,
    /// Light the medium inside gives off: a lit light guide or LED window
    /// glowing from within, bright in the middle and soft toward its edges
    /// by how far light travels through it, where surface emission is flat
    /// paint. Needs a transmission depth for the medium to exist.
    VolumeEmissionColor,
    /// Its strength, as Cycles' volume emission takes it: radiance per metre
    /// travelled.
    VolumeEmissionStrength,
}

impl Param {
    /// The label the UI shows. §3's tables name these, and the names differ
    /// from the parameter names on purpose — "Roughness" is what a user looks
    /// for, `specular_roughness` is what the file stores.
    pub fn label(self) -> &'static str {
        match self {
            Param::BaseWeight => "Base Weight",
            Param::BaseColor => "Color",
            Param::BaseMetalness => "Metalness",
            Param::DiffuseRoughness => "Diffuse Roughness",
            Param::SpecularWeight => "Specular Weight",
            Param::SpecularColor => "Specular Color",
            Param::SpecularRoughness => "Roughness",
            Param::SpecularIor => "IOR",
            Param::Anisotropy => "Anisotropy",
            Param::AnisotropyRotation => "Rotation",
            Param::Tangent => "Tangent",
            Param::MetalPreset => "Type",
            Param::TransmissionWeight => "Transmission",
            Param::TransmissionColor => "Transmission Color",
            Param::TransmissionDepth => "Transmission Depth",
            Param::TransmissionTint => "Surface Tint",
            Param::TransmissionScatter => "Scatter",
            Param::ScatterAnisotropy => "Scatter Anisotropy",
            Param::VolumeDensity => "Density",
            Param::Abbe => "Abbe",
            Param::MediumPriority => "Priority",
            Param::SubsurfaceWeight => "Translucency",
            Param::SubsurfaceColor => "Subsurface Color",
            Param::SubsurfaceRadius => "Radius",
            Param::SubsurfaceAnisotropy => "Subsurface Anisotropy",
            Param::CoatWeight => "Coat",
            Param::CoatColor => "Coat Color",
            Param::CoatRoughness => "Coat Roughness",
            Param::CoatIor => "Coat IOR",
            Param::CoatDarkening => "Coat Darkening",
            Param::FuzzWeight => "Sheen",
            Param::FuzzColor => "Sheen Color",
            Param::FuzzRoughness => "Sheen Roughness",
            Param::FuzzDensity => "Fibres",
            Param::FuzzLength => "Fibre Length",
            Param::FuzzThickness => "Fibre Thickness",
            Param::FuzzLean => "Fibre Lean",
            Param::FuzzCurl => "Fibre Curl",
            Param::FuzzColorVariation => "Fibre Color Variation",
            Param::ThinFilmWeight => "Thin Film",
            Param::ThinFilmThickness => "Thickness (nm)",
            Param::ThinFilmIor => "Film IOR",
            Param::FlakeWeight => "Flake Amount",
            Param::FlakeColor => "Flake Color",
            Param::FlakeSize => "Flake Size",
            Param::FlakeSpread => "Flake Sparkle",
            Param::FlakeRoughness => "Flake Roughness",
            Param::EmissionLuminance => "Luminance",
            Param::EmissionColor => "Emission Color",
            Param::Opacity => "Opacity",
            Param::ThinWalled => "Thin Walled",
            Param::Occlusion => "Occlusion",
            Param::Bump => "Bump",
            Param::Displacement => "Displacement",
            Param::DisplacementDetail => "Displacement Detail",
            Param::EdgeRadius => "Round Edges",
            Param::EdgeSamples => "Round Edges Samples",
            Param::VolumeEmissionColor => "Glow Color",
            Param::VolumeEmissionStrength => "Glow Inside",
        }
    }

    /// Every parameter, for the types that expose all of them.
    pub const ALL: [Param; 59] = [
        Param::BaseWeight,
        Param::BaseColor,
        Param::BaseMetalness,
        Param::DiffuseRoughness,
        Param::SpecularWeight,
        Param::SpecularColor,
        Param::SpecularRoughness,
        Param::SpecularIor,
        Param::Anisotropy,
        Param::AnisotropyRotation,
        Param::Tangent,
        Param::MetalPreset,
        Param::TransmissionWeight,
        Param::TransmissionColor,
        Param::TransmissionDepth,
        Param::TransmissionTint,
        Param::TransmissionScatter,
        Param::ScatterAnisotropy,
        Param::VolumeDensity,
        Param::Abbe,
        Param::MediumPriority,
        Param::SubsurfaceWeight,
        Param::SubsurfaceColor,
        Param::SubsurfaceRadius,
        Param::SubsurfaceAnisotropy,
        Param::CoatWeight,
        Param::CoatColor,
        Param::CoatRoughness,
        Param::CoatIor,
        Param::CoatDarkening,
        Param::FuzzWeight,
        Param::FuzzColor,
        Param::FuzzRoughness,
        Param::FuzzDensity,
        Param::FuzzLength,
        Param::FuzzThickness,
        Param::FuzzLean,
        Param::FuzzCurl,
        Param::FuzzColorVariation,
        Param::ThinFilmWeight,
        Param::ThinFilmThickness,
        Param::ThinFilmIor,
        Param::FlakeWeight,
        Param::FlakeColor,
        Param::FlakeSize,
        Param::FlakeSpread,
        Param::FlakeRoughness,
        Param::EmissionLuminance,
        Param::EmissionColor,
        Param::Opacity,
        Param::ThinWalled,
        Param::Occlusion,
        Param::Bump,
        Param::Displacement,
        Param::DisplacementDetail,
        Param::EdgeRadius,
        Param::EdgeSamples,
        Param::VolumeEmissionColor,
        Param::VolumeEmissionStrength,
    ];
}

/// §3's catalog. Thirty-four surface types and four light types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MaterialType {
    // §3.1 Basic
    Diffuse,
    Flat,
    Metal,
    Plastic,
    Paint,
    MetallicPaint,
    Glass,
    GlassSolid,
    Liquid,
    Translucent,
    ThinFilm,
    // §3.2 Advanced
    Advanced,
    Generic,
    Anisotropic,
    Dielectric,
    Gem,
    Measured,
    MultiLayerOptics,
    PlasticCloudy,
    PlasticTransparent,
    RealCloth,
    ScatteringMedium,
    TranslucentMedium,
    Velvet,
    // §3.3 Special
    Emissive,
    Toon,
    Wireframe,
    XRay,
    Cutaway,
    Ground,
    // §3.4 Lights
    AreaLight,
    PointLight,
    Spotlight,
    IesLight,
}

/// Which section of §3 a type belongs to, for the library's grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Basic,
    Advanced,
    Special,
    Light,
}

impl MaterialType {
    /// Every type in the catalog, in the order §3 lists them.
    pub const ALL: [MaterialType; 34] = [
        MaterialType::Diffuse,
        MaterialType::Flat,
        MaterialType::Metal,
        MaterialType::Plastic,
        MaterialType::Paint,
        MaterialType::MetallicPaint,
        MaterialType::Glass,
        MaterialType::GlassSolid,
        MaterialType::Liquid,
        MaterialType::Translucent,
        MaterialType::ThinFilm,
        MaterialType::Advanced,
        MaterialType::Generic,
        MaterialType::Anisotropic,
        MaterialType::Dielectric,
        MaterialType::Gem,
        MaterialType::Measured,
        MaterialType::MultiLayerOptics,
        MaterialType::PlasticCloudy,
        MaterialType::PlasticTransparent,
        MaterialType::RealCloth,
        MaterialType::ScatteringMedium,
        MaterialType::TranslucentMedium,
        MaterialType::Velvet,
        MaterialType::Emissive,
        MaterialType::Toon,
        MaterialType::Wireframe,
        MaterialType::XRay,
        MaterialType::Cutaway,
        MaterialType::Ground,
        MaterialType::AreaLight,
        MaterialType::PointLight,
        MaterialType::Spotlight,
        MaterialType::IesLight,
    ];

    /// The stable identifier written to a file. Never the display name: a
    /// label can be reworded, and a project saved last year must still open.
    pub fn id(self) -> &'static str {
        match self {
            MaterialType::Diffuse => "diffuse",
            MaterialType::Flat => "flat",
            MaterialType::Metal => "metal",
            MaterialType::Plastic => "plastic",
            MaterialType::Paint => "paint",
            MaterialType::MetallicPaint => "metallic_paint",
            MaterialType::Glass => "glass",
            MaterialType::GlassSolid => "glass_solid",
            MaterialType::Liquid => "liquid",
            MaterialType::Translucent => "translucent",
            MaterialType::ThinFilm => "thin_film",
            MaterialType::Advanced => "advanced",
            MaterialType::Generic => "generic",
            MaterialType::Anisotropic => "anisotropic",
            MaterialType::Dielectric => "dielectric",
            MaterialType::Gem => "gem",
            MaterialType::Measured => "measured",
            MaterialType::MultiLayerOptics => "multi_layer_optics",
            MaterialType::PlasticCloudy => "plastic_cloudy",
            MaterialType::PlasticTransparent => "plastic_transparent",
            MaterialType::RealCloth => "real_cloth",
            MaterialType::ScatteringMedium => "scattering_medium",
            MaterialType::TranslucentMedium => "translucent_medium",
            MaterialType::Velvet => "velvet",
            MaterialType::Emissive => "emissive",
            MaterialType::Toon => "toon",
            MaterialType::Wireframe => "wireframe",
            MaterialType::XRay => "xray",
            MaterialType::Cutaway => "cutaway",
            MaterialType::Ground => "ground",
            MaterialType::AreaLight => "area_light",
            MaterialType::PointLight => "point_light",
            MaterialType::Spotlight => "spotlight",
            MaterialType::IesLight => "ies_light",
        }
    }

    pub fn from_id(id: &str) -> Option<MaterialType> {
        MaterialType::ALL.into_iter().find(|t| t.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            MaterialType::Diffuse => "Diffuse",
            MaterialType::Flat => "Flat",
            MaterialType::Metal => "Metal",
            MaterialType::Plastic => "Plastic",
            MaterialType::Paint => "Paint",
            MaterialType::MetallicPaint => "Metallic Paint",
            MaterialType::Glass => "Glass",
            MaterialType::GlassSolid => "Glass (Solid)",
            MaterialType::Liquid => "Liquid",
            MaterialType::Translucent => "Translucent",
            MaterialType::ThinFilm => "Thin Film",
            MaterialType::Advanced => "Advanced",
            MaterialType::Generic => "Generic",
            MaterialType::Anisotropic => "Anisotropic",
            MaterialType::Dielectric => "Dielectric",
            MaterialType::Gem => "Gem",
            MaterialType::Measured => "Measured",
            MaterialType::MultiLayerOptics => "Multi-Layer Optics",
            MaterialType::PlasticCloudy => "Plastic (Cloudy)",
            MaterialType::PlasticTransparent => "Plastic (Transparent)",
            MaterialType::RealCloth => "RealCloth",
            MaterialType::ScatteringMedium => "Scattering Medium",
            MaterialType::TranslucentMedium => "Translucent Medium",
            MaterialType::Velvet => "Velvet",
            MaterialType::Emissive => "Emissive",
            MaterialType::Toon => "Toon",
            MaterialType::Wireframe => "Wireframe",
            MaterialType::XRay => "X-Ray",
            MaterialType::Cutaway => "Cutaway",
            MaterialType::Ground => "Ground",
            MaterialType::AreaLight => "Area Light",
            MaterialType::PointLight => "Point Light",
            MaterialType::Spotlight => "Spotlight",
            MaterialType::IesLight => "IES Light",
        }
    }

    pub fn category(self) -> Category {
        match self {
            MaterialType::Diffuse
            | MaterialType::Flat
            | MaterialType::Metal
            | MaterialType::Plastic
            | MaterialType::Paint
            | MaterialType::MetallicPaint
            | MaterialType::Glass
            | MaterialType::GlassSolid
            | MaterialType::Liquid
            | MaterialType::Translucent
            | MaterialType::ThinFilm => Category::Basic,
            MaterialType::Advanced
            | MaterialType::Generic
            | MaterialType::Anisotropic
            | MaterialType::Dielectric
            | MaterialType::Gem
            | MaterialType::Measured
            | MaterialType::MultiLayerOptics
            | MaterialType::PlasticCloudy
            | MaterialType::PlasticTransparent
            | MaterialType::RealCloth
            | MaterialType::ScatteringMedium
            | MaterialType::TranslucentMedium
            | MaterialType::Velvet => Category::Advanced,
            MaterialType::Emissive
            | MaterialType::Toon
            | MaterialType::Wireframe
            | MaterialType::XRay
            | MaterialType::Cutaway
            | MaterialType::Ground => Category::Special,
            MaterialType::AreaLight
            | MaterialType::PointLight
            | MaterialType::Spotlight
            | MaterialType::IesLight => Category::Light,
        }
    }

    /// The type's defaults, as §3's "Key parameter settings" column gives them.
    pub fn defaults(self) -> Material {
        let base = Material::default();
        match self {
            MaterialType::Diffuse => Material {
                base_metalness: 0.0,
                specular_weight: 0.0,
                base_diffuse_roughness: 0.0,
                ..base
            },
            // Unlit constant emission. §3.1: "renders as its colour, still
            // writes depth/ID" — so it is emission with every other lobe off,
            // not a black surface.
            MaterialType::Flat => Material {
                base_weight: 0.0,
                specular_weight: 0.0,
                emission_color: Spectrum::splat(0.5),
                emission_luminance: 1.0,
                ..base
            },
            MaterialType::Metal => Material {
                base_metalness: 1.0,
                metal_ior: metals::ALUMINIUM,
                // White, so the preset is the bare measured metal. The
                // default 18% grey would have quietly darkened every metal to
                // a fifth of its reflectance.
                base_color: Spectrum::ONE,
                specular_roughness: 0.15,
                ..base
            },
            MaterialType::Plastic => Material {
                base_metalness: 0.0,
                specular_ior: 1.5,
                specular_weight: 1.0,
                specular_roughness: 0.1,
                base_color: spectrum(0.35, 0.36, 0.4),
                ..base
            },
            MaterialType::Paint => Material {
                coat_weight: 1.0,
                coat_roughness: 0.02,
                ..MaterialType::Plastic.defaults()
            },
            // §3.1: Paint plus a flake sub-layer — pigment under a clear coat,
            // with metal flakes tilted a little at random in between. The
            // pigment is a dielectric and the flakes are metal, rather than
            // the whole base being part-metal, which is what gives metallic
            // paint its sparkle instead of the even sheen of a brushed panel.
            MaterialType::MetallicPaint => Material {
                base_color: spectrum(0.06, 0.12, 0.35),
                specular_roughness: 0.35,
                // Fifty-micron flakes, a third of the base: below a pixel
                // each at any ordinary distance, so the paint reads as a
                // metallic sheen that glitters where a few catch the light,
                // not as salt and pepper.
                flake_weight: 0.35,
                flake_color: Spectrum::splat(0.85),
                ..MaterialType::Paint.defaults()
            },
            // Thin-walled: a window pane, not a block. No refraction offset.
            MaterialType::Glass => Material {
                transmission_weight: 1.0,
                geometry_thin_walled: true,
                transmission_depth: 0.0,
                specular_roughness: 0.0,
                specular_ior: 1.5,
                base_color: Spectrum::ONE,
                ..base
            },
            // The depth is the distance at which `transmission_color` is
            // fully reached, so it sets how thick a piece has to be before it
            // looks green — a metre, for glass. It read 0.05 until absorption
            // was implemented, which would have meant five centimetres of
            // glass swallowing fifteen percent of the red: not a window, a
            // dyed block. Nothing read the number then, so nothing had ever
            // disagreed with it; the first render through it came out black.
            MaterialType::GlassSolid => Material {
                transmission_weight: 1.0,
                geometry_thin_walled: false,
                transmission_depth: 1.0,
                transmission_color: spectrum(0.85, 0.95, 0.92),
                specular_roughness: 0.0,
                specular_ior: 1.5,
                ..base
            },
            // §3.1: liquid IOR 1.33, and a medium priority that wins over
            // container glass at coincident surfaces (§01 §6.1).
            //
            // Half the depth of glass, keeping the original preset's intent
            // that a liquid tints faster over the same distance.
            MaterialType::Liquid => Material {
                specular_ior: 1.33,
                transmission_depth: 0.5,
                medium_priority: 1.0,
                ..MaterialType::GlassSolid.defaults()
            },
            MaterialType::Translucent => Material {
                subsurface_weight: 1.0,
                subsurface_radius: Spectrum::splat(0.05),
                subsurface_color: spectrum(0.9, 0.75, 0.7),
                specular_weight: 1.0,
                specular_roughness: 0.25,
                ..base
            },
            // Anodised titanium, which is what an iridescent finish on a
            // product almost always is: titanium metal under a grown oxide
            // film, coloured by interference in the oxide rather than by any
            // pigment.
            //
            // The numbers are the real ones. Titanium dioxide has an index
            // near 2.4 across the visible, and an anodising voltage in the
            // usual range grows something like 120 nm of it.
            //
            // They also replace numbers that were never checked against a
            // render, because nothing read them: a 1.4 film over the default
            // aluminium. Aluminium is the worst base in the catalog for this
            // — its reflectance is high and almost flat across the spectrum,
            // so there is little for the film to modulate — and a 1.4 film
            // over it reflects between 0.89 and 0.94 whatever the wavelength.
            // Measured saturation was 0.004 to 0.088, which renders as grey
            // metal. Over titanium with its own oxide it is 0.33 at grazing
            // and 0.89 head-on, and it sweeps between them across a sphere,
            // which is the thing the material is named for.
            MaterialType::ThinFilm => Material {
                thin_film_weight: 1.0,
                thin_film_thickness: 120.0,
                thin_film_ior: 2.4,
                base_metalness: 1.0,
                metal_ior: metals::TITANIUM,
                // White, as Metal's is: the film's colours over the bare
                // metal, not over a fifth of it.
                base_color: Spectrum::ONE,
                specular_roughness: 0.05,
                ..base
            },
            // Nothing hidden, so nothing overridden: the plain model.
            MaterialType::Advanced | MaterialType::Generic => base,
            MaterialType::Anisotropic => Material {
                base_metalness: 1.0,
                // Brushed metal: the bare measured metal, as Metal is, not
                // one darkened to a fifth by the default grey.
                metal_ior: metals::ALUMINIUM,
                base_color: Spectrum::ONE,
                specular_roughness: 0.25,
                specular_roughness_anisotropy: 0.7,
                ..base
            },
            // §3.2: "no artistic shortcuts".
            MaterialType::Dielectric => Material {
                transmission_weight: 1.0,
                specular_ior: 1.5,
                specular_roughness: 0.0,
                transmission_depth: 0.1,
                ..base
            },
            // Diamond: IOR 2.42, Abbe 55, spectral forced on.
            MaterialType::Gem => Material {
                transmission_weight: 1.0,
                specular_ior: 2.42,
                transmission_dispersion_abbe: 55.0,
                specular_roughness: 0.0,
                transmission_color: Spectrum::ONE,
                ..base
            },
            // A tabulated BRDF, which needs the file to mean anything. Until
            // the loader exists it starts as a plain glossy dielectric rather
            // than as something that pretends to be measured.
            MaterialType::Measured => Material {
                specular_roughness: 0.2,
                ..base
            },
            MaterialType::MultiLayerOptics => Material {
                thin_film_weight: 1.0,
                thin_film_thickness: 300.0,
                thin_film_ior: 2.0,
                base_metalness: 1.0,
                // The coating over the bare metal, as Metal's is.
                metal_ior: metals::ALUMINIUM,
                base_color: Spectrum::ONE,
                specular_roughness: 0.02,
                ..base
            },
            // §3.2: light penetrates a little, giving the waxy look of ABS.
            MaterialType::PlasticCloudy => Material {
                subsurface_weight: 0.35,
                subsurface_radius: Spectrum::splat(0.004),
                subsurface_color: spectrum(0.9, 0.9, 0.88),
                ..MaterialType::Plastic.defaults()
            },
            MaterialType::PlasticTransparent => Material {
                specular_ior: 1.49,
                transmission_depth: 0.2,
                transmission_color: spectrum(0.9, 0.93, 0.9),
                ..MaterialType::GlassSolid.defaults()
            },
            // Fibre-level: sheen plus anisotropic specular along a woven
            // tangent field. The weave procedural arrives with the graph.
            MaterialType::RealCloth => Material {
                base_color: spectrum(0.25, 0.28, 0.35),
                specular_roughness: 0.45,
                specular_roughness_anisotropy: 0.6,
                fuzz_weight: 0.6,
                fuzz_roughness: 0.4,
                ..base
            },
            // A pure volume: the surface is a transparent interface.
            MaterialType::ScatteringMedium => Material {
                transmission_weight: 1.0,
                specular_roughness: 0.0,
                specular_ior: 1.0,
                transmission_scatter: Spectrum::splat(4.0),
                transmission_depth: 0.5,
                ..base
            },
            // Tuned for high-albedo forward scattering: wax, jade.
            MaterialType::TranslucentMedium => Material {
                transmission_scatter: Spectrum::splat(20.0),
                transmission_scatter_anisotropy: 0.7,
                transmission_color: spectrum(0.95, 0.9, 0.8),
                ..MaterialType::ScatteringMedium.defaults()
            },
            // §3.2: high sheen roughness over a dark base, which is what makes
            // velvet's rim read.
            MaterialType::Velvet => Material {
                base_color: spectrum(0.05, 0.02, 0.06),
                fuzz_weight: 1.0,
                fuzz_roughness: 0.9,
                fuzz_color: spectrum(0.7, 0.6, 0.75),
                specular_weight: 0.0,
                ..base
            },
            MaterialType::Emissive => Material {
                base_weight: 0.0,
                specular_weight: 0.0,
                emission_luminance: 1000.0,
                emission_color: Spectrum::ONE,
                ..base
            },
            // NPR, and its shading is a pass rather than a lobe. The defaults
            // are the base colour the bands quantize.
            MaterialType::Toon => Material {
                specular_weight: 0.0,
                base_color: spectrum(0.5, 0.6, 0.8),
                ..base
            },
            MaterialType::Wireframe => Material {
                base_weight: 0.0,
                specular_weight: 0.0,
                emission_color: Spectrum::ONE,
                emission_luminance: 1.0,
                geometry_opacity: 1.0,
                ..base
            },
            MaterialType::XRay => Material {
                base_weight: 0.0,
                specular_weight: 0.0,
                emission_color: spectrum(0.4, 0.7, 1.0),
                emission_luminance: 1.0,
                geometry_opacity: 0.2,
                ..base
            },
            // A cutting volume: it is not shaded at all, it removes.
            MaterialType::Cutaway => Material {
                base_weight: 0.0,
                specular_weight: 0.0,
                geometry_opacity: 0.0,
                ..base
            },
            MaterialType::Ground => Material {
                base_color: Spectrum::splat(0.5),
                specular_roughness: 0.35,
                ..base
            },
            MaterialType::AreaLight
            | MaterialType::PointLight
            | MaterialType::Spotlight
            | MaterialType::IesLight => Material {
                base_weight: 0.0,
                specular_weight: 0.0,
                emission_luminance: 1500.0,
                emission_color: Spectrum::ONE,
                ..base
            },
        }
    }

    /// Which parameters the UI shows, from §3's "UI exposes" column.
    ///
    /// `Bump` appears on every type that has a surface, because §3 lists it on
    /// most of them and a surface without bump control is a surface the user
    /// cannot finish.
    pub fn exposed(self) -> Vec<Param> {
        let mut params = self.exposed_before_displacement();
        // Displacement goes wherever Bump does: both are the surface's
        // relief, one faked in the shading and one built into the geometry.
        if params.contains(&Param::Bump) {
            for param in [Param::Displacement, Param::DisplacementDetail] {
                if !params.contains(&param) {
                    params.push(param);
                }
            }
        }
        // Rounded edges wherever there is a surface to shade, as Bump: every
        // CAD part has edges that catch light.
        if params.contains(&Param::Bump) {
            for param in [Param::EdgeRadius, Param::EdgeSamples] {
                if !params.contains(&param) {
                    params.push(param);
                }
            }
        }
        // A glow inside wherever there is an inside: a transmission depth.
        if params.contains(&Param::TransmissionDepth) {
            for param in [Param::VolumeEmissionColor, Param::VolumeEmissionStrength] {
                if !params.contains(&param) {
                    params.push(param);
                }
            }
        }
        // Fibres wherever there is a sheen for them to thicken: velvet and
        // cloth, a felt, a peach.
        if params.contains(&Param::FuzzWeight) {
            for param in [
                Param::FuzzDensity,
                Param::FuzzLength,
                Param::FuzzThickness,
                Param::FuzzLean,
                Param::FuzzCurl,
                Param::FuzzColorVariation,
            ] {
                if !params.contains(&param) {
                    params.push(param);
                }
            }
        }
        params
    }

    fn exposed_before_displacement(self) -> Vec<Param> {
        use Param::*;
        match self {
            MaterialType::Diffuse => vec![BaseColor, DiffuseRoughness, Bump],
            MaterialType::Flat => vec![EmissionColor],
            MaterialType::Metal => vec![
                MetalPreset,
                BaseColor,
                SpecularRoughness,
                Anisotropy,
                AnisotropyRotation,
                Tangent,
                Bump,
                // Not in §3's table. These two are the types a glTF import
                // resolves to, and an imported model is where a baked
                // occlusion map comes from; without the control the map is
                // wired into the graph with nothing in the panel admitting it
                // exists, which is this project's most familiar bug.
                Occlusion,
            ],
            MaterialType::Plastic => vec![
                BaseColor,
                SpecularColor,
                SpecularRoughness,
                SpecularIor,
                Bump,
                Opacity,
                Occlusion,
            ],
            MaterialType::Paint => vec![BaseColor, SpecularRoughness, CoatRoughness, CoatIor, Bump],
            MaterialType::MetallicPaint => vec![
                BaseColor,
                SpecularRoughness,
                FlakeColor,
                FlakeWeight,
                FlakeSize,
                FlakeSpread,
                FlakeRoughness,
                CoatRoughness,
            ],
            MaterialType::Glass => {
                vec![BaseColor, SpecularRoughness, SpecularIor, TransmissionColor]
            }
            MaterialType::GlassSolid => vec![
                TransmissionColor,
                TransmissionDepth,
                SpecularRoughness,
                SpecularIor,
                Abbe,
                MediumPriority,
            ],
            // Scatter as well as tint: milk, juice and coffee are liquids
            // too, and cloudiness is a walk inside the same refracting
            // surface that the clear ones have.
            MaterialType::Liquid => vec![
                TransmissionColor,
                TransmissionDepth,
                TransmissionScatter,
                ScatterAnisotropy,
                SpecularIor,
                SpecularRoughness,
                MediumPriority,
            ],
            MaterialType::Translucent => vec![
                BaseColor,
                SubsurfaceColor,
                SubsurfaceWeight,
                SubsurfaceRadius,
                SpecularRoughness,
            ],
            MaterialType::ThinFilm => vec![
                ThinFilmThickness,
                ThinFilmIor,
                BaseMetalness,
                SpecularRoughness,
            ],
            // §3.2: "Nothing hidden."
            MaterialType::Advanced | MaterialType::Generic => Param::ALL.to_vec(),
            MaterialType::Anisotropic => vec![
                BaseColor,
                SpecularRoughness,
                Anisotropy,
                AnisotropyRotation,
                Tangent,
                Bump,
            ],
            MaterialType::Dielectric => vec![
                SpecularIor,
                TransmissionColor,
                TransmissionDepth,
                SpecularRoughness,
                Abbe,
                MediumPriority,
            ],
            MaterialType::Gem => vec![SpecularIor, Abbe, BaseColor, TransmissionColor, MediumPriority],
            MaterialType::Measured => vec![SpecularRoughness, AnisotropyRotation],
            MaterialType::MultiLayerOptics => {
                vec![
                    ThinFilmThickness,
                    ThinFilmIor,
                    BaseMetalness,
                    SpecularRoughness,
                ]
            }
            MaterialType::PlasticCloudy => {
                vec![BaseColor, SubsurfaceRadius, SpecularRoughness, Bump]
            }
            MaterialType::PlasticTransparent => vec![
                TransmissionColor,
                TransmissionDepth,
                TransmissionTint,
                TransmissionScatter,
                ScatterAnisotropy,
                SpecularRoughness,
                SpecularIor,
                MediumPriority,
                Bump,
            ],
            MaterialType::RealCloth => vec![
                BaseColor,
                FuzzColor,
                FuzzWeight,
                FuzzRoughness,
                SpecularRoughness,
                Anisotropy,
            ],
            MaterialType::ScatteringMedium => vec![
                TransmissionScatter,
                TransmissionDepth,
                TransmissionColor,
                ScatterAnisotropy,
                VolumeDensity,
            ],
            MaterialType::TranslucentMedium => {
                vec![TransmissionColor, TransmissionDepth, ScatterAnisotropy, VolumeDensity]
            }
            MaterialType::Velvet => vec![BaseColor, FuzzColor, FuzzRoughness, FuzzWeight],
            MaterialType::Emissive => vec![EmissionColor, EmissionLuminance],
            MaterialType::Toon => vec![BaseColor],
            MaterialType::Wireframe => vec![EmissionColor, Opacity],
            MaterialType::XRay => vec![EmissionColor, Opacity],
            MaterialType::Cutaway => vec![],
            MaterialType::Ground => vec![BaseColor, SpecularRoughness],
            MaterialType::AreaLight
            | MaterialType::PointLight
            | MaterialType::Spotlight
            | MaterialType::IesLight => vec![EmissionColor, EmissionLuminance],
        }
    }

    /// Whether this type is a light rather than a surface (§3.4).
    pub fn is_light(self) -> bool {
        self.category() == Category::Light
    }
}

/// The value behind a [`Param`].
///
/// Four shapes cover all 47 parameters, which is what lets one command, one
/// journal entry and one panel row serve every one of them instead of
/// thirty-seven of each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Scalar(f32),
    Color(Spectrum),
    /// A conductor's complex index of refraction: §2.2's `metal_ior`, chosen
    /// from the measured-metal table rather than typed in.
    Metal(ComplexIor),
    Flag(bool),
}

impl ParamValue {
    pub fn as_scalar(self) -> Option<f32> {
        match self {
            ParamValue::Scalar(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_color(self) -> Option<Spectrum> {
        match self {
            ParamValue::Color(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_flag(self) -> Option<bool> {
        match self {
            ParamValue::Flag(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_metal(self) -> Option<ComplexIor> {
        match self {
            ParamValue::Metal(v) => Some(v),
            _ => None,
        }
    }
}

impl Param {
    /// The parameter's stable name.
    ///
    /// Deliberately identical to the [`Material`] field it addresses, so the
    /// struct, the UI, the undo journal and the saved file all spell a
    /// parameter the same way. A rename that misses one of those is the kind
    /// of bug that only shows up when an old file is opened.
    pub fn key(self) -> &'static str {
        match self {
            Param::BaseWeight => "base_weight",
            Param::BaseColor => "base_color",
            Param::BaseMetalness => "base_metalness",
            Param::DiffuseRoughness => "base_diffuse_roughness",
            Param::SpecularWeight => "specular_weight",
            Param::SpecularColor => "specular_color",
            Param::SpecularRoughness => "specular_roughness",
            Param::SpecularIor => "specular_ior",
            Param::Anisotropy => "specular_roughness_anisotropy",
            Param::AnisotropyRotation => "specular_rotation",
            Param::Tangent => "geometry_tangent",
            Param::MetalPreset => "metal_ior",
            Param::TransmissionWeight => "transmission_weight",
            Param::TransmissionColor => "transmission_color",
            Param::TransmissionDepth => "transmission_depth",
            Param::TransmissionTint => "transmission_tint",
            Param::TransmissionScatter => "transmission_scatter",
            Param::ScatterAnisotropy => "transmission_scatter_anisotropy",
            Param::VolumeDensity => "volume_density",
            Param::Abbe => "transmission_dispersion_abbe",
            Param::MediumPriority => "medium_priority",
            Param::SubsurfaceWeight => "subsurface_weight",
            Param::SubsurfaceColor => "subsurface_color",
            Param::SubsurfaceRadius => "subsurface_radius",
            Param::SubsurfaceAnisotropy => "subsurface_scatter_anisotropy",
            Param::CoatWeight => "coat_weight",
            Param::CoatColor => "coat_color",
            Param::CoatRoughness => "coat_roughness",
            Param::CoatIor => "coat_ior",
            Param::CoatDarkening => "coat_darkening",
            Param::FuzzWeight => "fuzz_weight",
            Param::FuzzColor => "fuzz_color",
            Param::FuzzRoughness => "fuzz_roughness",
            Param::FuzzDensity => "fuzz_density",
            Param::FuzzLength => "fuzz_length",
            Param::FuzzThickness => "fuzz_thickness",
            Param::FuzzLean => "fuzz_lean",
            Param::FuzzCurl => "fuzz_curl",
            Param::FuzzColorVariation => "fuzz_color_variation",
            Param::ThinFilmWeight => "thin_film_weight",
            Param::ThinFilmThickness => "thin_film_thickness",
            Param::ThinFilmIor => "thin_film_ior",
            Param::FlakeWeight => "flake_weight",
            Param::FlakeColor => "flake_color",
            Param::FlakeSize => "flake_size",
            Param::FlakeSpread => "flake_spread",
            Param::FlakeRoughness => "flake_roughness",
            Param::EmissionLuminance => "emission_luminance",
            Param::EmissionColor => "emission_color",
            Param::Opacity => "geometry_opacity",
            Param::ThinWalled => "geometry_thin_walled",
            Param::Occlusion => "geometry_occlusion",
            Param::Bump => "bump",
            Param::Displacement => "displacement",
            Param::DisplacementDetail => "displacement_detail",
            Param::EdgeRadius => "edge_radius",
            Param::EdgeSamples => "edge_samples",
            Param::VolumeEmissionColor => "volume_emission_color",
            Param::VolumeEmissionStrength => "volume_emission_strength",
        }
    }

    pub fn from_key(key: &str) -> Option<Param> {
        Some(match key {
            "base_weight" => Param::BaseWeight,
            "base_color" => Param::BaseColor,
            "base_metalness" => Param::BaseMetalness,
            "base_diffuse_roughness" => Param::DiffuseRoughness,
            "specular_weight" => Param::SpecularWeight,
            "specular_color" => Param::SpecularColor,
            "specular_roughness" => Param::SpecularRoughness,
            "specular_ior" => Param::SpecularIor,
            "specular_roughness_anisotropy" => Param::Anisotropy,
            "specular_rotation" => Param::AnisotropyRotation,
            "geometry_tangent" => Param::Tangent,
            "metal_ior" => Param::MetalPreset,
            "transmission_weight" => Param::TransmissionWeight,
            "transmission_color" => Param::TransmissionColor,
            "transmission_depth" => Param::TransmissionDepth,
            "transmission_tint" => Param::TransmissionTint,
            "transmission_scatter" => Param::TransmissionScatter,
            "transmission_scatter_anisotropy" => Param::ScatterAnisotropy,
            "volume_density" => Param::VolumeDensity,
            "transmission_dispersion_abbe" => Param::Abbe,
            "medium_priority" => Param::MediumPriority,
            "subsurface_weight" => Param::SubsurfaceWeight,
            "subsurface_color" => Param::SubsurfaceColor,
            "subsurface_radius" => Param::SubsurfaceRadius,
            "subsurface_scatter_anisotropy" => Param::SubsurfaceAnisotropy,
            "coat_weight" => Param::CoatWeight,
            "coat_color" => Param::CoatColor,
            "coat_roughness" => Param::CoatRoughness,
            "coat_ior" => Param::CoatIor,
            "coat_darkening" => Param::CoatDarkening,
            "fuzz_weight" => Param::FuzzWeight,
            "fuzz_color" => Param::FuzzColor,
            "fuzz_roughness" => Param::FuzzRoughness,
            "fuzz_density" => Param::FuzzDensity,
            "fuzz_length" => Param::FuzzLength,
            "fuzz_thickness" => Param::FuzzThickness,
            "fuzz_lean" => Param::FuzzLean,
            "fuzz_curl" => Param::FuzzCurl,
            "fuzz_color_variation" => Param::FuzzColorVariation,
            "thin_film_weight" => Param::ThinFilmWeight,
            "thin_film_thickness" => Param::ThinFilmThickness,
            "thin_film_ior" => Param::ThinFilmIor,
            "flake_weight" => Param::FlakeWeight,
            "flake_color" => Param::FlakeColor,
            "flake_size" => Param::FlakeSize,
            "flake_spread" => Param::FlakeSpread,
            "flake_roughness" => Param::FlakeRoughness,
            "emission_luminance" => Param::EmissionLuminance,
            "emission_color" => Param::EmissionColor,
            "geometry_opacity" => Param::Opacity,
            "geometry_thin_walled" => Param::ThinWalled,
            "geometry_occlusion" => Param::Occlusion,
            "bump" => Param::Bump,
            "displacement" => Param::Displacement,
            "displacement_detail" => Param::DisplacementDetail,
            "edge_radius" => Param::EdgeRadius,
            "edge_samples" => Param::EdgeSamples,
            "volume_emission_color" => Param::VolumeEmissionColor,
            "volume_emission_strength" => Param::VolumeEmissionStrength,
            _ => return None,
        })
    }

    /// Read this parameter out of a material.
    ///
    /// `None` for a parameter that is a texture slot rather than a value —
    /// [`Param::Bump`] is one, and it is fed by the material graph.
    pub fn read(self, material: &Material) -> Option<ParamValue> {
        match self {
            Param::BaseWeight => Some(ParamValue::Scalar(material.base_weight)),
            Param::BaseColor => Some(ParamValue::Color(material.base_color)),
            Param::BaseMetalness => Some(ParamValue::Scalar(material.base_metalness)),
            Param::DiffuseRoughness => Some(ParamValue::Scalar(material.base_diffuse_roughness)),
            Param::SpecularWeight => Some(ParamValue::Scalar(material.specular_weight)),
            Param::SpecularColor => Some(ParamValue::Color(material.specular_color)),
            Param::SpecularRoughness => Some(ParamValue::Scalar(material.specular_roughness)),
            Param::SpecularIor => Some(ParamValue::Scalar(material.specular_ior)),
            Param::Anisotropy => Some(ParamValue::Scalar(material.specular_roughness_anisotropy)),
            Param::AnisotropyRotation => Some(ParamValue::Scalar(material.specular_rotation)),
            Param::MetalPreset => Some(ParamValue::Metal(material.metal_ior)),
            Param::TransmissionWeight => Some(ParamValue::Scalar(material.transmission_weight)),
            Param::TransmissionColor => Some(ParamValue::Color(material.transmission_color)),
            Param::TransmissionDepth => Some(ParamValue::Scalar(material.transmission_depth)),
            Param::TransmissionTint => Some(ParamValue::Color(material.transmission_tint)),
            Param::TransmissionScatter => Some(ParamValue::Color(material.transmission_scatter)),
            Param::ScatterAnisotropy => Some(ParamValue::Scalar(material.transmission_scatter_anisotropy)),
            Param::VolumeDensity => Some(ParamValue::Scalar(material.volume_density)),
            Param::Abbe => Some(ParamValue::Scalar(material.transmission_dispersion_abbe)),
            Param::MediumPriority => Some(ParamValue::Scalar(material.medium_priority)),
            Param::SubsurfaceWeight => Some(ParamValue::Scalar(material.subsurface_weight)),
            Param::SubsurfaceColor => Some(ParamValue::Color(material.subsurface_color)),
            Param::SubsurfaceRadius => Some(ParamValue::Color(material.subsurface_radius)),
            Param::SubsurfaceAnisotropy => Some(ParamValue::Scalar(material.subsurface_scatter_anisotropy)),
            Param::CoatWeight => Some(ParamValue::Scalar(material.coat_weight)),
            Param::CoatColor => Some(ParamValue::Color(material.coat_color)),
            Param::CoatRoughness => Some(ParamValue::Scalar(material.coat_roughness)),
            Param::CoatIor => Some(ParamValue::Scalar(material.coat_ior)),
            Param::CoatDarkening => Some(ParamValue::Scalar(material.coat_darkening)),
            Param::FuzzWeight => Some(ParamValue::Scalar(material.fuzz_weight)),
            Param::FuzzColor => Some(ParamValue::Color(material.fuzz_color)),
            Param::FuzzRoughness => Some(ParamValue::Scalar(material.fuzz_roughness)),
            Param::FuzzDensity => Some(ParamValue::Scalar(material.fuzz_density)),
            Param::FuzzLength => Some(ParamValue::Scalar(material.fuzz_length)),
            Param::FuzzThickness => Some(ParamValue::Scalar(material.fuzz_thickness)),
            Param::FuzzLean => Some(ParamValue::Scalar(material.fuzz_lean)),
            Param::FuzzCurl => Some(ParamValue::Scalar(material.fuzz_curl)),
            Param::FuzzColorVariation => Some(ParamValue::Scalar(material.fuzz_color_variation)),
            Param::ThinFilmWeight => Some(ParamValue::Scalar(material.thin_film_weight)),
            Param::ThinFilmThickness => Some(ParamValue::Scalar(material.thin_film_thickness)),
            Param::ThinFilmIor => Some(ParamValue::Scalar(material.thin_film_ior)),
            Param::FlakeWeight => Some(ParamValue::Scalar(material.flake_weight)),
            Param::FlakeColor => Some(ParamValue::Color(material.flake_color)),
            Param::FlakeSize => Some(ParamValue::Scalar(material.flake_size)),
            Param::FlakeSpread => Some(ParamValue::Scalar(material.flake_spread)),
            Param::FlakeRoughness => Some(ParamValue::Scalar(material.flake_roughness)),
            Param::EmissionLuminance => Some(ParamValue::Scalar(material.emission_luminance)),
            Param::EmissionColor => Some(ParamValue::Color(material.emission_color)),
            Param::Opacity => Some(ParamValue::Scalar(material.geometry_opacity)),
            Param::ThinWalled => Some(ParamValue::Flag(material.geometry_thin_walled)),
            Param::Occlusion => Some(ParamValue::Scalar(material.geometry_occlusion)),
            Param::Bump | Param::Displacement | Param::Tangent => None,
            Param::DisplacementDetail => Some(ParamValue::Scalar(material.displacement_detail)),
            Param::EdgeRadius => Some(ParamValue::Scalar(material.edge_radius)),
            Param::EdgeSamples => Some(ParamValue::Scalar(material.edge_samples)),
            Param::VolumeEmissionColor => Some(ParamValue::Color(material.volume_emission_color)),
            Param::VolumeEmissionStrength => Some(ParamValue::Scalar(material.volume_emission_strength)),
        }
    }

    /// Write this parameter into a material.
    ///
    /// Returns whether it applied: a value of the wrong shape for the
    /// parameter is refused rather than coerced, because a colour silently
    /// becoming a scalar is worse than an edit that visibly does nothing.
    pub fn write(self, material: &mut Material, value: ParamValue) -> bool {
        match (self, value) {
            (Param::BaseWeight, ParamValue::Scalar(value)) => material.base_weight = value,
            (Param::BaseColor, ParamValue::Color(value)) => material.base_color = value,
            (Param::BaseMetalness, ParamValue::Scalar(value)) => material.base_metalness = value,
            (Param::DiffuseRoughness, ParamValue::Scalar(value)) => material.base_diffuse_roughness = value,
            (Param::SpecularWeight, ParamValue::Scalar(value)) => material.specular_weight = value,
            (Param::SpecularColor, ParamValue::Color(value)) => material.specular_color = value,
            (Param::SpecularRoughness, ParamValue::Scalar(value)) => material.specular_roughness = value,
            (Param::SpecularIor, ParamValue::Scalar(value)) => material.specular_ior = value,
            (Param::Anisotropy, ParamValue::Scalar(value)) => material.specular_roughness_anisotropy = value,
            (Param::AnisotropyRotation, ParamValue::Scalar(value)) => material.specular_rotation = value,
            (Param::MetalPreset, ParamValue::Metal(value)) => material.metal_ior = value,
            (Param::TransmissionWeight, ParamValue::Scalar(value)) => material.transmission_weight = value,
            (Param::TransmissionColor, ParamValue::Color(value)) => material.transmission_color = value,
            (Param::TransmissionDepth, ParamValue::Scalar(value)) => material.transmission_depth = value,
            (Param::TransmissionTint, ParamValue::Color(value)) => material.transmission_tint = value,
            (Param::TransmissionScatter, ParamValue::Color(value)) => material.transmission_scatter = value,
            (Param::ScatterAnisotropy, ParamValue::Scalar(value)) => material.transmission_scatter_anisotropy = value,
            (Param::VolumeDensity, ParamValue::Scalar(value)) => material.volume_density = value,
            (Param::Abbe, ParamValue::Scalar(value)) => material.transmission_dispersion_abbe = value,
            (Param::MediumPriority, ParamValue::Scalar(value)) => material.medium_priority = value,
            (Param::SubsurfaceWeight, ParamValue::Scalar(value)) => material.subsurface_weight = value,
            (Param::SubsurfaceColor, ParamValue::Color(value)) => material.subsurface_color = value,
            (Param::SubsurfaceRadius, ParamValue::Color(value)) => material.subsurface_radius = value,
            (Param::SubsurfaceAnisotropy, ParamValue::Scalar(value)) => material.subsurface_scatter_anisotropy = value,
            (Param::CoatWeight, ParamValue::Scalar(value)) => material.coat_weight = value,
            (Param::CoatColor, ParamValue::Color(value)) => material.coat_color = value,
            (Param::CoatRoughness, ParamValue::Scalar(value)) => material.coat_roughness = value,
            (Param::CoatIor, ParamValue::Scalar(value)) => material.coat_ior = value,
            (Param::CoatDarkening, ParamValue::Scalar(value)) => material.coat_darkening = value,
            (Param::FuzzWeight, ParamValue::Scalar(value)) => material.fuzz_weight = value,
            (Param::FuzzColor, ParamValue::Color(value)) => material.fuzz_color = value,
            (Param::FuzzRoughness, ParamValue::Scalar(value)) => material.fuzz_roughness = value,
            (Param::FuzzDensity, ParamValue::Scalar(value)) => material.fuzz_density = value,
            (Param::FuzzLength, ParamValue::Scalar(value)) => material.fuzz_length = value,
            (Param::FuzzThickness, ParamValue::Scalar(value)) => material.fuzz_thickness = value,
            (Param::FuzzLean, ParamValue::Scalar(value)) => material.fuzz_lean = value,
            (Param::FuzzCurl, ParamValue::Scalar(value)) => material.fuzz_curl = value,
            (Param::FuzzColorVariation, ParamValue::Scalar(value)) => material.fuzz_color_variation = value,
            (Param::ThinFilmWeight, ParamValue::Scalar(value)) => material.thin_film_weight = value,
            (Param::ThinFilmThickness, ParamValue::Scalar(value)) => material.thin_film_thickness = value,
            (Param::ThinFilmIor, ParamValue::Scalar(value)) => material.thin_film_ior = value,
            (Param::FlakeWeight, ParamValue::Scalar(value)) => material.flake_weight = value,
            (Param::FlakeColor, ParamValue::Color(value)) => material.flake_color = value,
            (Param::FlakeSize, ParamValue::Scalar(value)) => material.flake_size = value,
            (Param::FlakeSpread, ParamValue::Scalar(value)) => material.flake_spread = value,
            (Param::FlakeRoughness, ParamValue::Scalar(value)) => material.flake_roughness = value,
            (Param::EmissionLuminance, ParamValue::Scalar(value)) => material.emission_luminance = value,
            (Param::EmissionColor, ParamValue::Color(value)) => material.emission_color = value,
            (Param::Opacity, ParamValue::Scalar(value)) => material.geometry_opacity = value,
            (Param::ThinWalled, ParamValue::Flag(value)) => material.geometry_thin_walled = value,
            (Param::Occlusion, ParamValue::Scalar(value)) => material.geometry_occlusion = value,
            (Param::Bump, _) | (Param::Displacement, _) | (Param::Tangent, _) => return false,
            (Param::DisplacementDetail, ParamValue::Scalar(value)) => material.displacement_detail = value,
            (Param::EdgeRadius, ParamValue::Scalar(value)) => material.edge_radius = value,
            (Param::EdgeSamples, ParamValue::Scalar(value)) => material.edge_samples = value,
            (Param::VolumeEmissionColor, ParamValue::Color(value)) => material.volume_emission_color = value,
            (Param::VolumeEmissionStrength, ParamValue::Scalar(value)) => material.volume_emission_strength = value,
            _ => return false,
        }
        true
    }
}

/// How a parameter should be presented, which is not the same question as how
/// it is stored.
///
/// Two parameters are stored as a [`Spectrum`] and mean entirely different
/// things by it: `base_color` is a colour and belongs behind a colour picker,
/// while `subsurface_radius` is three distances in scene units and would be
/// actively misleading behind one — it invites the user to choose "a nice red"
/// for a mean free path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamUi {
    Scalar,
    Color,
    /// Three independent numbers that happen to share a [`Spectrum`].
    Vector,
    /// Chosen from the measured-metal table, not typed.
    Metal,
    Flag,
    /// A texture slot fed by the material graph; it has no value to show.
    Slot,
}

/// The bounds a control should offer for a parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamRange {
    pub low: f32,
    pub high: f32,
    /// Whether the useful values span enough orders of magnitude that a linear
    /// control is unusable. Emission in nits runs from a dim panel to the sun.
    pub logarithmic: bool,
}

impl Param {
    /// The control this parameter wants.
    pub fn ui(self) -> ParamUi {
        match self {
            Param::BaseWeight => ParamUi::Scalar,
            Param::BaseColor => ParamUi::Color,
            Param::BaseMetalness => ParamUi::Scalar,
            Param::DiffuseRoughness => ParamUi::Scalar,
            Param::SpecularWeight => ParamUi::Scalar,
            Param::SpecularColor => ParamUi::Color,
            Param::SpecularRoughness => ParamUi::Scalar,
            Param::SpecularIor => ParamUi::Scalar,
            Param::Anisotropy => ParamUi::Scalar,
            Param::AnisotropyRotation => ParamUi::Scalar,
            Param::MetalPreset => ParamUi::Metal,
            Param::TransmissionWeight => ParamUi::Scalar,
            Param::TransmissionColor => ParamUi::Color,
            Param::TransmissionDepth => ParamUi::Scalar,
            Param::TransmissionTint => ParamUi::Color,
            // Per channel, and far past 1 for anything milky: numbers, not a
            // colour picker, which holds 0 to 1.
            Param::TransmissionScatter => ParamUi::Vector,
            Param::ScatterAnisotropy => ParamUi::Scalar,
            Param::VolumeDensity => ParamUi::Scalar,
            Param::Abbe => ParamUi::Scalar,
            Param::MediumPriority => ParamUi::Scalar,
            Param::SubsurfaceWeight => ParamUi::Scalar,
            Param::SubsurfaceColor => ParamUi::Color,
            Param::SubsurfaceRadius => ParamUi::Vector,
            Param::SubsurfaceAnisotropy => ParamUi::Scalar,
            Param::CoatWeight => ParamUi::Scalar,
            Param::CoatColor => ParamUi::Color,
            Param::CoatRoughness => ParamUi::Scalar,
            Param::CoatIor => ParamUi::Scalar,
            Param::CoatDarkening => ParamUi::Scalar,
            Param::FuzzWeight => ParamUi::Scalar,
            Param::FuzzColor => ParamUi::Color,
            Param::FuzzRoughness => ParamUi::Scalar,
            Param::FuzzDensity
            | Param::FuzzLength
            | Param::FuzzThickness
            | Param::FuzzLean
            | Param::FuzzCurl
            | Param::FuzzColorVariation => ParamUi::Scalar,
            Param::ThinFilmWeight => ParamUi::Scalar,
            Param::ThinFilmThickness => ParamUi::Scalar,
            Param::ThinFilmIor => ParamUi::Scalar,
            Param::FlakeWeight | Param::FlakeSize | Param::FlakeSpread | Param::FlakeRoughness => ParamUi::Scalar,
            Param::FlakeColor => ParamUi::Color,
            Param::EmissionLuminance => ParamUi::Scalar,
            Param::EmissionColor => ParamUi::Color,
            Param::Opacity => ParamUi::Scalar,
            Param::ThinWalled => ParamUi::Flag,
            Param::Occlusion => ParamUi::Scalar,
            Param::Bump | Param::Displacement | Param::Tangent => ParamUi::Slot,
            Param::DisplacementDetail => ParamUi::Scalar,
            Param::EdgeRadius | Param::EdgeSamples | Param::VolumeEmissionStrength => ParamUi::Scalar,
            Param::VolumeEmissionColor => ParamUi::Color,
        }
    }

    /// The bounds a control should offer, or `None` for a parameter that has
    /// none — a colour, a flag, a preset, a texture slot.
    ///
    /// These mirror the clamps in [`Material::sanitized`] exactly. Where a
    /// control offers a value the sanitizer rejects, the control springs back
    /// under the user's cursor and nothing explains why, so the two have to be
    /// derived from the same table. The test below holds them together.
    /// Why this parameter does nothing yet, if it does nothing yet.
    ///
    /// §2.2 defines the full OpenPBR parameter set and the catalog offers all
    /// of it, but the renderer does not implement all of it. A parameter in
    /// this list is stored, saved, unit-scaled and shown, and read by no code
    /// that shades — so moving its control changes no pixel.
    ///
    /// Stating it here rather than quietly leaving the control live is the
    /// whole point. This project has repeatedly found features that were
    /// complete in the engine and unreachable from the UI; this is the mirror
    /// image, and it is worse, because a control that does nothing teaches the
    /// user that the renderer is broken rather than that the feature is
    /// missing. The Material tab greys these and shows the reason.
    ///
    /// Implementing one means deleting its arm here, and
    /// `every_exposed_parameter_reaches_the_shading` will say so if the arm is
    /// deleted before the work is done — or left behind after it.
    /// Whether the value is a count rather than a measure: the control steps
    /// in ones and shows no decimals.
    pub fn is_whole_number(self) -> bool {
        matches!(self, Param::MediumPriority | Param::EdgeSamples)
    }

    /// Whether the parameter belongs to the whole piece rather than to a
    /// point on its surface, so that a texture across the surface means
    /// nothing: which medium wins where two overlap is one answer per part.
    pub fn is_per_piece(self) -> bool {
        matches!(
            self,
            Param::MediumPriority
                | Param::FuzzDensity
                | Param::FuzzLength
                | Param::FuzzThickness
                | Param::FuzzLean
                | Param::FuzzCurl
                | Param::FuzzColorVariation
        )
    }

    /// How the Material tab shows a length too fine to read in metres: a
    /// factor and its unit, as the graph editor shows a Fuzz node's.
    pub fn shown_in(self) -> Option<(f32, &'static str)> {
        match self {
            Param::FuzzLength => Some((1000.0, " mm")),
            Param::FuzzThickness => Some((1.0e6, " µm")),
            _ => None,
        }
    }

    /// What the Material tab says about a control on hover, where its name
    /// alone does not.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Param::FuzzDensity => Some(
                "Real fibres grown out of the surface, to the square centimetre, in the Sheen Color and                  Sheen Roughness: felt's halo, a tennis ball's nap, velvet's pile. 0 grows none. They render                  in both renderers and in the Rendered view, not in the Solid one; a part grows at most                  250,000, so dense fibres on a large part come out sparser.",
            ),
            Param::FuzzLength => Some("How long each fibre is, give or take a third."),
            Param::FuzzThickness => Some("How wide each fibre is at its root; it tapers to a fifth of that at its tip."),
            Param::FuzzLean => Some("How far the fibres lean over, 0 standing straight up."),
            Param::FuzzCurl => Some("How much each fibre bends along its length."),
            Param::FuzzColorVariation => Some("How much darker than the Sheen Color a fibre may be, each its own way."),
            _ => None,
        }
    }

    /// Why a parameter does nothing yet, for the Material tab to grey it out
    /// with. Every parameter is implemented today; the list is kept for the
    /// next one that is not.
    pub fn unimplemented(self) -> Option<&'static str> {
        None
    }

    pub fn range(self) -> Option<ParamRange> {
        match self {
            Param::BaseWeight => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::BaseColor => None,
            Param::BaseMetalness => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::DiffuseRoughness => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::SpecularWeight => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::SpecularColor => None,
            Param::SpecularRoughness => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::SpecularIor => Some(ParamRange { low: 1.0, high: 3.0, logarithmic: false }),
            Param::Anisotropy => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::AnisotropyRotation => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::MetalPreset => None,
            Param::TransmissionWeight => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::TransmissionColor => None,
            Param::TransmissionDepth => Some(ParamRange { low: 0.0, high: 1000.0, logarithmic: true }),
            Param::TransmissionTint => None,
            Param::TransmissionScatter => Some(ParamRange { low: 0.0, high: 1000.0, logarithmic: true }),
            Param::ScatterAnisotropy => Some(ParamRange { low: -1.0, high: 1.0, logarithmic: false }),
            Param::VolumeDensity => Some(ParamRange { low: 0.0, high: 100.0, logarithmic: true }),
            Param::Abbe => Some(ParamRange { low: 0.0, high: 100.0, logarithmic: false }),
            Param::MediumPriority => Some(ParamRange { low: 0.0, high: 10.0, logarithmic: false }),
            Param::SubsurfaceWeight => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::SubsurfaceColor => None,
            Param::SubsurfaceRadius => Some(ParamRange { low: 0.0, high: 100.0, logarithmic: true }),
            Param::SubsurfaceAnisotropy => Some(ParamRange { low: -1.0, high: 1.0, logarithmic: false }),
            Param::CoatWeight => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::CoatColor => None,
            Param::CoatRoughness => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::CoatIor => Some(ParamRange { low: 1.0, high: 2.0, logarithmic: false }),
            Param::CoatDarkening => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::FuzzWeight => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::FuzzColor => None,
            Param::FuzzRoughness => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            // A few fibres to the square centimetre is a peach's skin, a few
            // hundred a tennis ball's felt, thousands a velvet's pile.
            Param::FuzzDensity => Some(ParamRange { low: 0.0, high: 5000.0, logarithmic: true }),
            // Metres: a twentieth of a millimetre to five centimetres.
            Param::FuzzLength => Some(ParamRange { low: 0.00005, high: 0.05, logarithmic: true }),
            // Two microns, a microfibre's, to a millimetre, a bristle's.
            Param::FuzzThickness => Some(ParamRange { low: 0.000002, high: 0.001, logarithmic: true }),
            Param::FuzzLean | Param::FuzzCurl | Param::FuzzColorVariation => {
                Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false })
            }
            Param::ThinFilmWeight => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::ThinFilmThickness => Some(ParamRange { low: 0.0, high: 2000.0, logarithmic: false }),
            Param::ThinFilmIor => Some(ParamRange { low: 1.0, high: 3.0, logarithmic: false }),
            Param::FlakeWeight | Param::FlakeSpread | Param::FlakeRoughness => {
                Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false })
            }
            // Twenty microns, the real thing, to two millimetres, the glitter
            // on a guitar body; scene units, so metres in a scene in metres.
            Param::FlakeSize => Some(ParamRange { low: 0.00002, high: 0.002, logarithmic: true }),
            Param::FlakeColor => None,
            Param::EmissionLuminance => Some(ParamRange { low: 0.0, high: 100000.0, logarithmic: true }),
            Param::EmissionColor => None,
            Param::Opacity => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::ThinWalled => None,
            Param::Occlusion => Some(ParamRange { low: 0.0, high: 1.0, logarithmic: false }),
            Param::Bump | Param::Displacement | Param::Tangent => None,
            // Up to a centimetre in a scene in metres; zero is the bump-only
            // approximation.
            Param::DisplacementDetail => Some(ParamRange { low: 0.0, high: 0.01, logarithmic: false }),
            // Metres: up to 5 mm, past which a fillet belongs in the model.
            Param::EdgeRadius => Some(ParamRange { low: 0.0, high: 0.005, logarithmic: false }),
            Param::EdgeSamples => Some(ParamRange { low: 1.0, high: 32.0, logarithmic: false }),
            Param::VolumeEmissionColor => None,
            Param::VolumeEmissionStrength => Some(ParamRange { low: 0.0, high: 100000.0, logarithmic: true }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The flux a material's BSDF sends back, as a fraction of what reached
    /// it.
    ///
    /// Flux and not radiance: a refraction compresses radiance by `1/eta^2`
    /// going in and expands it by the same factor coming out, so undoing that
    /// with `sample.eta` is what turns the measurement into a quantity with
    /// one right answer on both sides of a surface.
    ///
    /// # Why the points come from Sobol and not from a hand-rolled sequence
    ///
    /// This used to draw them from the rank-1 lattice
    /// `((i + 1/2), 7919 i, 4099 i) / 8192`, which looks stratified in each
    /// coordinate separately and is not stratified at all in the pair that
    /// matters. Because `2 * 4099 = 8192 + 6`, the lobe-selection variable is
    /// an *exact function* of the first direction variable: `uc = frac(3 u.x)`
    /// on even indices and `frac(3 u.x + 1/2)` on odd ones, with no residual
    /// term over all 8192 points. `sample` picks a lobe from `uc` and a
    /// direction from `u`, so every lobe was handed a `u.x` confined to six
    /// narrow bands instead of the unit interval, and the reading that came
    /// back was a property of the multipliers as much as of the material.
    /// [`crate::lobes::openpbr`]'s `mean_throughput` documents what that cost
    /// on a case where it could be pinned down exactly.
    ///
    /// The Owen-scrambled Sobol' sequence has no such structure, and its
    /// stratification survives being split across dimensions.
    fn flux_albedo(bsdf: &impl crate::bsdf::Bsdf, wo: tequila_core::math::Vec3) -> f32 {
        use tequila_core::math::Vec2;
        use tequila_core::sobol;
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
                        sample.f.luminance() * sample.wi.z.abs() / sample.pdf;
                    total += (weight * sample.eta * sample.eta) as f64;
                }
            }
        }
        (total / SAMPLES as f64) as f32
    }

    /// No material in the catalog may send back more light than reached it.
    ///
    /// The weakest statement that still covers everything: a coloured
    /// material sits well under one and this says nothing about how far
    /// under, but a material *above* one is broken however it is coloured,
    /// and that is a defect no amount of looking at renders reliably finds. A
    /// surface ten percent too bright reads as a lighting choice.
    ///
    /// **Every roughness is swept, not just the preset's own.** That is the
    /// difference between this test and a version of it that would have been
    /// useless. The fuzz lobe spent the whole life of the catalog reflecting
    /// up to 2.45 of what fell on it, and testing the presets as shipped
    /// misses it completely: the excess only appears below a fuzz roughness
    /// of about 0.1 and Velvet ships at 0.3. Energy errors track roughness —
    /// it is what every one of these lobes is a function of — so a guard that
    /// holds roughness at whatever a preset happens to use is a guard over
    /// one point of the space it is supposed to cover.
    ///
    /// Verified against that bug: with the fuzz normalization removed, this
    /// fails on Velvet at minimum roughness, reading 1.39. Only Velvet — it
    /// is the one preset whose fuzz weight is high enough for the excess to
    /// clear the tolerance, which is worth knowing. A catalog-wide sweep
    /// catches a lobe-level defect through whichever preset leans hardest on
    /// that lobe, and through no others.
    /// A type that is all metal starts as the bare measured metal: a white
    /// tint over a real metal's optics. The default 18% grey left on one
    /// darkens it to a fifth of its reflectance - the Anisotropic type,
    /// brushed metal, rendered near black wherever a preset or a person did
    /// not happen to set a colour.
    #[test]
    fn every_metal_type_starts_as_the_bare_metal() {
        for ty in MaterialType::ALL {
            let m = ty.defaults();
            if m.base_metalness >= 1.0 {
                assert_eq!(m.base_color, Spectrum::ONE, "{} starts tinted {:?}", ty.label(), m.base_color);
                assert!(metals::ALL.iter().any(|(_, ior)| *ior == m.metal_ior), "{} starts with no measured metal", ty.label());
            }
        }
    }

    #[test]
    fn nothing_in_the_catalog_reflects_more_than_arrives() {
        use tequila_core::math::vec3;

        let mut bad = Vec::new();
        for material_type in MaterialType::ALL {
            let defaults = material_type.defaults();
            // The preset as shipped, and the same preset at each end of every
            // roughness it has. Smooth and rough are where the closed forms
            // and the compensation terms are most likely to disagree with the
            // lobes they stand for.
            let variants = [
                ("as shipped", defaults.clone()),
                ("at minimum roughness", Material {
                    base_diffuse_roughness: 0.0,
                    specular_roughness: 0.0,
                    coat_roughness: 0.0,
                    fuzz_roughness: 0.0,
                    ..defaults.clone()
                }),
                ("at maximum roughness", Material {
                    base_diffuse_roughness: 1.0,
                    specular_roughness: 1.0,
                    coat_roughness: 1.0,
                    fuzz_roughness: 1.0,
                    ..defaults.clone()
                }),
            ];

            for (variant, material) in &variants {
                for &degrees in &[5.0f32, 35.0, 65.0, 85.0, 89.0] {
                    let theta: f32 = degrees.to_radians();
                    let wo = vec3(theta.sin(), 0.0, theta.cos());
                    // Both sides, because a transmissive material has two and
                    // the inside one is where its energy is hardest to keep.
                    for inside in [false, true] {
                        let bsdf = if inside {
                            material.bsdf_from_inside(0.0)
                        } else {
                            material.bsdf(0.0)
                        };
                        let reflected = flux_albedo(&bsdf, wo);
                        // One percent, and it is worth saying what that is
                        // made of rather than leaving it as a round number.
                        //
                        // **Half a percent is real, and it is the rough
                        // dielectric compensation.** `energy_scale` divides
                        // the transmission lobe by a tabulated albedo for what
                        // the interface passes on, so wherever that table
                        // reads low the lobe is over-restored by the same
                        // fraction. It reads lowest at the far corner of its
                        // domain — full roughness, from inside, where total
                        // internal reflection turns most of the light back
                        // into the microsurface. At a quarter of a million
                        // samples the worst cell of the whole sweep is `Gem at
                        // maximum roughness from inside at 65 degrees`,
                        // reading 1.0049: the table says that interface passes
                        // on 0.4893 where it actually passes on 0.4917. The
                        // bare lobe at the same roughness, index and side
                        // reads the same 1.0049, so none of this comes from
                        // the layering on top of it.
                        //
                        // **Two tenths of a percent is what 16384 Sobol points
                        // are worth here.** Over thirty-two scrambles the
                        // worst that cell ever read was 1.0067, and no cell in
                        // the sweep strayed further than 0.0022 from its
                        // converged value. Half as many points doubles that
                        // stray to 0.0043 and would not leave room for this
                        // bound, which is what taking the sweep's 1020
                        // measurements from six seconds to eleven buys.
                        //
                        // The rest is headroom, and deliberately not much.
                        //
                        // Verified against a deliberate leak rather than
                        // assumed: scaling the interface compensation by
                        // 1.0075 fails this on Gem reading 1.0125, and 1.005
                        // still passes. Removing the fuzz normalization — the
                        // bug described above — fails on Velvet at minimum
                        // roughness reading 1.3892, and on nothing else. The
                        // bound was 1.05 while the estimator was a lattice
                        // whose reading was partly an artefact of its own
                        // multipliers, which left the 1% that is actually
                        // there indistinguishable from the way it was being
                        // measured.
                        //
                        // The NaN case is named rather than left to fall
                        // out of a negated comparison: a lobe that returns
                        // one has failed in a way worth hearing about, and
                        // `reflected > 1.01` alone would quietly pass it.
                        if reflected > 1.01 || reflected.is_nan() {
                            bad.push(format!(
                                "{material_type:?} {variant}{} at {degrees} \
                                 degrees: {reflected:.4}",
                                if inside { " from inside" } else { "" }
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            bad.is_empty(),
            "these materials send back more light than reaches them: {bad:#?}"
        );
    }

    /// Every parameter of the block is reachable through [`Param`].
    ///
    /// This is the test that stops the reflection table from quietly falling
    /// behind the struct. A parameter added to [`Material`] but not to the
    /// table would be invisible to the Material tab, absent from the undo
    /// journal and dropped on save — three bugs that look unrelated and share
    /// one cause. Copying a fully-populated material one parameter at a time
    /// and demanding the result compare equal catches all three at once.
    #[test]
    fn every_parameter_of_the_block_is_reachable_through_param() {
        // Every field set away from its default, so a field the table misses
        // fails to transfer instead of coincidentally matching.
        let source = Material {
            base_weight: 0.91,
            base_color: spectrum(0.11, 0.22, 0.33),
            base_metalness: 0.44,
            base_diffuse_roughness: 0.55,
            specular_weight: 0.66,
            specular_color: spectrum(0.12, 0.23, 0.34),
            specular_roughness: 0.77,
            specular_ior: 1.63,
            specular_roughness_anisotropy: 0.31,
            specular_rotation: 0.29,
            metal_ior: metals::GOLD,
            transmission_weight: 0.37,
            transmission_color: spectrum(0.13, 0.24, 0.35),
            transmission_depth: 4.5,
            transmission_tint: spectrum(0.61, 0.72, 0.83),
            transmission_scatter: spectrum(0.14, 0.25, 0.36),
            transmission_scatter_anisotropy: 0.21,
            volume_density: 1.9,
            transmission_dispersion_abbe: 42.0,
            medium_priority: 3.0,
            subsurface_weight: 0.19,
            subsurface_color: spectrum(0.15, 0.26, 0.37),
            subsurface_radius: spectrum(0.16, 0.27, 0.38),
            subsurface_scatter_anisotropy: 0.23,
            coat_weight: 0.61,
            coat_color: spectrum(0.17, 0.28, 0.39),
            coat_roughness: 0.13,
            coat_ior: 1.71,
            coat_darkening: 0.83,
            fuzz_weight: 0.47,
            fuzz_color: spectrum(0.18, 0.29, 0.41),
            fuzz_roughness: 0.53,
            fuzz_density: 37.0,
            fuzz_length: 0.0031,
            fuzz_thickness: 0.000017,
            fuzz_lean: 0.41,
            fuzz_curl: 0.62,
            fuzz_color_variation: 0.33,
            thin_film_weight: 0.39,
            thin_film_thickness: 517.0,
            thin_film_ior: 1.39,
            flake_weight: 0.57,
            flake_color: spectrum(0.21, 0.32, 0.43),
            flake_size: 0.00042,
            flake_spread: 0.44,
            flake_roughness: 0.27,
            // Per shading point, filled in by the renderer; not parameters,
            // so they stay at their defaults for the comparison to be fair.
            flake_coverage: 0.0,
            flake_normal: tequila_core::math::Vec3::Z,
            displacement_detail: 0.00077,
            edge_radius: 0.00025,
            edge_samples: 12.0,
            volume_emission_color: spectrum(0.97, 0.91, 0.85),
            volume_emission_strength: 190.0,
            emission_luminance: 1234.0,
            emission_color: spectrum(0.19, 0.31, 0.42),
            geometry_opacity: 0.73,
            geometry_thin_walled: true,
            geometry_occlusion: 0.67,
        };
        assert_ne!(source, Material::default(), "the fixture must differ");

        let mut copy = Material::default();
        let mut transferred = 0;
        for param in Param::ALL {
            if let Some(value) = param.read(&source) {
                assert!(
                    param.write(&mut copy, value),
                    "`{}` could be read but not written",
                    param.key()
                );
                transferred += 1;
            }
        }

        assert_eq!(
            copy,
            source,
            "{transferred} parameters transferred, but the copy still differs \
             from the original: a field of `Material` is missing from `Param`"
        );
    }

    /// Every value a control can produce must survive sanitization unchanged.
    ///
    /// This is the test that stops the panel from fighting the model. If a
    /// slider offers 0 to 3 for a parameter the sanitizer clamps to 1 to 2,
    /// the control springs back under the user's cursor and nothing on screen
    /// explains why — a bug that reads as the application being broken rather
    /// than as a mismatched bound. Both sides come from tables, and this holds
    /// the tables together.
    #[test]
    fn every_value_a_control_can_offer_survives_sanitizing() {
        for param in Param::ALL {
            let Some(range) = param.range() else {
                continue;
            };
            assert!(
                range.low < range.high,
                "`{}` has an empty range {range:?}",
                param.key()
            );
            assert!(
                !range.logarithmic || range.low >= 0.0,
                "`{}` is logarithmic but starts below zero",
                param.key()
            );

            // The ends, and a point inside. The ends are what matter: a
            // mismatch shows up there first.
            let midpoint = 0.5 * (range.low + range.high);
            for value in [range.low, midpoint, range.high] {
                let mut material = Material::default();
                let written = match param.ui() {
                    ParamUi::Vector => ParamValue::Color(Spectrum::splat(value)),
                    _ => ParamValue::Scalar(value),
                };
                assert!(
                    param.write(&mut material, written),
                    "`{}` refused {value}",
                    param.key()
                );

                let survived = material.sanitized();
                let after = param.read(&survived).expect("a ranged parameter has a value");
                let got = match after {
                    ParamValue::Scalar(v) => v,
                    ParamValue::Color(c) => c.r,
                    other => panic!("`{}` read back as {other:?}", param.key()),
                };
                assert!(
                    (got - value).abs() <= 1e-6 * value.abs().max(1.0),
                    "a control offering `{}` = {value} would have it silently \
                     changed to {got} by the sanitizer",
                    param.key()
                );
            }
        }
    }

    /// A colour picker must not be put in front of something that is not a
    /// colour. `subsurface_radius` is three distances in scene units.
    #[test]
    fn only_actual_colours_are_presented_as_colours() {
        for param in Param::ALL {
            match (param.ui(), param.read(&Material::default())) {
                // Stored as a spectrum either way; the presentation differs.
                (ParamUi::Color | ParamUi::Vector, Some(ParamValue::Color(_))) => {}
                (ParamUi::Scalar, Some(ParamValue::Scalar(_))) => {}
                (ParamUi::Metal, Some(ParamValue::Metal(_))) => {}
                (ParamUi::Flag, Some(ParamValue::Flag(_))) => {}
                (ParamUi::Slot, None) => {}
                (ui, value) => panic!(
                    "`{}` is presented as {ui:?} but stored as {value:?}",
                    param.key()
                ),
            }
        }
        assert_eq!(Param::SubsurfaceRadius.ui(), ParamUi::Vector);
        assert_eq!(Param::BaseColor.ui(), ParamUi::Color);
    }

    /// Keys round-trip, are unique, and are the field names.
    #[test]
    fn parameter_keys_are_unique_and_round_trip() {
        let mut seen = std::collections::BTreeSet::new();
        for param in Param::ALL {
            let key = param.key();
            assert!(seen.insert(key), "`{key}` is used by two parameters");
            assert_eq!(Param::from_key(key), Some(param), "`{key}` did not round-trip");
            assert!(
                key.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "`{key}` is not a stable snake_case key"
            );
        }
        assert_eq!(seen.len(), Param::ALL.len());
        assert_eq!(Param::from_key("no_such_parameter"), None);
    }

    /// A value of the wrong shape is refused, not coerced.
    #[test]
    fn a_mismatched_value_is_refused() {
        let mut material = Material::default();
        let before = material.clone();
        assert!(!Param::BaseColor.write(&mut material, ParamValue::Scalar(0.5)));
        assert!(!Param::SpecularRoughness.write(&mut material, ParamValue::Flag(true)));
        assert!(!Param::Bump.write(&mut material, ParamValue::Scalar(0.5)));
        assert_eq!(material, before, "a refused write still changed something");

        assert!(Param::SpecularRoughness.write(&mut material, ParamValue::Scalar(0.5)));
        assert_eq!(material.specular_roughness, 0.5);
    }

    /// Every exposed parameter of every preset must be one the panel can
    /// actually render — that is, one with a value behind it.
    #[test]
    fn every_exposed_parameter_has_a_value_behind_it() {
        let material = Material::default();
        for material_type in MaterialType::ALL {
            for param in material_type.exposed() {
                assert!(
                    param.read(&material).is_some() || param.ui() == ParamUi::Slot,
                    "{:?} exposes `{}`, which has no value to show",
                    material_type,
                    param.key()
                );
            }
        }
    }

    /// §3's own count, and the one the M2 exit criterion runs χ² over.
    #[test]
    fn the_catalog_is_the_size_the_specification_states() {
        assert_eq!(MaterialType::ALL.len(), 34);
        let surfaces = MaterialType::ALL.iter().filter(|t| !t.is_light()).count();
        let lights = MaterialType::ALL.iter().filter(|t| t.is_light()).count();
        assert_eq!(lights, 4, "§3.4 lists four light types");
        assert_eq!(surfaces, 30);
    }

    /// Ids go into files. Two types sharing one would make a project load as
    /// the wrong material, and a renamed one would make it fail to load at
    /// all.
    #[test]
    fn every_id_is_unique_stable_and_round_trips() {
        let mut seen = std::collections::BTreeSet::new();
        for material_type in MaterialType::ALL {
            assert!(
                seen.insert(material_type.id()),
                "`{}` is used twice",
                material_type.id()
            );
            assert_eq!(
                MaterialType::from_id(material_type.id()),
                Some(material_type)
            );
            assert!(
                material_type
                    .id()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_'),
                "`{}` is not a stable identifier",
                material_type.id()
            );
        }
        assert_eq!(MaterialType::from_id("no_such_type"), None);
    }

    #[test]
    fn every_label_is_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for material_type in MaterialType::ALL {
            assert!(
                seen.insert(material_type.label()),
                "two types are both called `{}`",
                material_type.label()
            );
        }
    }

    /// Every default must be in range, or a type chosen from the menu produces
    /// a material the renderer has to repair.
    #[test]
    fn every_preset_is_already_sanitized() {
        for material_type in MaterialType::ALL {
            let defaults = material_type.defaults();
            assert_eq!(
                defaults.clone().sanitized(),
                defaults,
                "`{}` has defaults outside their stated ranges",
                material_type.label()
            );
        }
    }

    /// §3's tables are the contract. A type whose defaults contradict its own
    /// row is a type that will not look like its name.
    #[test]
    fn the_presets_match_the_settings_the_specification_states() {
        let d = MaterialType::Diffuse.defaults();
        assert_eq!(d.base_metalness, 0.0);
        assert_eq!(d.specular_weight, 0.0);

        let metal = MaterialType::Metal.defaults();
        assert_eq!(metal.base_metalness, 1.0);

        let plastic = MaterialType::Plastic.defaults();
        assert_eq!(plastic.specular_ior, 1.5);
        assert_eq!(plastic.specular_weight, 1.0);
        assert!((plastic.specular_roughness - 0.1).abs() < 1e-6);

        let paint = MaterialType::Paint.defaults();
        assert_eq!(paint.coat_weight, 1.0);
        assert!((paint.coat_roughness - 0.02).abs() < 1e-6);
        assert_eq!(
            paint.specular_ior, plastic.specular_ior,
            "Paint is a plastic base plus a coat"
        );

        // §3.1: Glass is thin-walled with no depth; Glass (Solid) is neither.
        let glass = MaterialType::Glass.defaults();
        assert_eq!(glass.transmission_weight, 1.0);
        assert!(glass.geometry_thin_walled);
        assert_eq!(glass.transmission_depth, 0.0);
        assert!(!glass.has_volume());

        let solid = MaterialType::GlassSolid.defaults();
        assert!(!solid.geometry_thin_walled);
        assert!(solid.transmission_depth > 0.0);
        assert!(solid.has_volume());

        // §3.1: liquid IOR default 1.33.
        assert!((MaterialType::Liquid.defaults().specular_ior - 1.33).abs() < 1e-6);

        // §3.2: diamond, IOR 2.42, Abbe 55.
        let gem = MaterialType::Gem.defaults();
        assert!((gem.specular_ior - 2.42).abs() < 1e-6);
        assert!((gem.transmission_dispersion_abbe - 55.0).abs() < 1e-6);

        // §3.2: plastic IOR 1.49.
        assert!((MaterialType::PlasticTransparent.defaults().specular_ior - 1.49).abs() < 1e-6);

        // §3.2: velvet is high sheen roughness over a dark base.
        let velvet = MaterialType::Velvet.defaults();
        assert_eq!(velvet.fuzz_weight, 1.0);
        assert!(velvet.fuzz_roughness > 0.5);
        assert!(velvet.base_color.luminance() < 0.1, "the base is not dark");

        // §3.1: Flat is unlit constant emission, not a black surface.
        let flat = MaterialType::Flat.defaults();
        assert!(flat.is_emissive());
        assert_eq!(flat.base_weight, 0.0);
    }

    /// §3.2: Advanced and Generic hide nothing.
    #[test]
    fn advanced_exposes_everything_and_the_rest_expose_less() {
        assert_eq!(MaterialType::Advanced.exposed().len(), Param::ALL.len());
        assert_eq!(MaterialType::Generic.exposed().len(), Param::ALL.len());

        for material_type in MaterialType::ALL {
            if matches!(
                material_type,
                MaterialType::Advanced | MaterialType::Generic
            ) {
                continue;
            }
            assert!(
                material_type.exposed().len() < Param::ALL.len(),
                "`{}` hides nothing, which makes it Advanced under another name",
                material_type.label()
            );
        }
    }

    /// A control listed twice would render twice.
    #[test]
    fn no_type_exposes_a_parameter_twice() {
        for material_type in MaterialType::ALL {
            let exposed = material_type.exposed();
            let unique: std::collections::BTreeSet<_> = exposed.iter().collect();
            assert_eq!(
                unique.len(),
                exposed.len(),
                "`{}` lists a parameter twice",
                material_type.label()
            );
        }
    }

    /// A type that exposes nothing the user can change is a type that cannot
    /// be authored. Cutaway is the one real exception — it is a cutting
    /// volume, not a surface.
    #[test]
    fn every_type_exposes_something_except_the_one_that_shades_nothing() {
        for material_type in MaterialType::ALL {
            if material_type == MaterialType::Cutaway {
                assert!(material_type.exposed().is_empty());
                continue;
            }
            assert!(
                !material_type.exposed().is_empty(),
                "`{}` exposes no controls at all",
                material_type.label()
            );
        }
    }

    /// The parameter a type is *named for* has to be one it exposes. A Metal
    /// that hid its roughness, or a Gem that hid its Abbe number, would be
    /// unusable in exactly the way the type exists to prevent.
    #[test]
    fn each_type_exposes_the_parameter_it_is_named_for() {
        let required: [(MaterialType, Param); 10] = [
            (MaterialType::Metal, Param::MetalPreset),
            (MaterialType::Gem, Param::Abbe),
            (MaterialType::Anisotropic, Param::Anisotropy),
            (MaterialType::Velvet, Param::FuzzRoughness),
            (MaterialType::ThinFilm, Param::ThinFilmThickness),
            (MaterialType::Emissive, Param::EmissionLuminance),
            (MaterialType::GlassSolid, Param::TransmissionDepth),
            (MaterialType::Translucent, Param::SubsurfaceWeight),
            (MaterialType::ScatteringMedium, Param::TransmissionScatter),
            (MaterialType::Paint, Param::CoatRoughness),
        ];
        for (material_type, param) in required {
            assert!(
                material_type.exposed().contains(&param),
                "`{}` does not expose `{}`",
                material_type.label(),
                param.label()
            );
        }
    }

    /// A colour picker holds a colour, 0 to 1 a channel. A per-channel
    /// coefficient that runs past 1 — scatter, whose default is 4 for a
    /// Scattering Medium and 20 for a Translucent Medium, and 25 to 38 for
    /// milk — was offered through one, which showed white whatever the value
    /// and, the moment it was touched, cut the density to a picker's range.
    /// So no parameter a type shows as a colour may start above 1, and one
    /// shown as numbers must have a range to offer them in.
    #[test]
    fn nothing_beyond_one_is_offered_as_a_colour() {
        for material_type in MaterialType::ALL {
            let defaults = material_type.defaults();
            for param in material_type.exposed() {
                let Some(ParamValue::Color(value)) = param.read(&defaults) else {
                    continue;
                };
                match param.ui() {
                    ParamUi::Color => assert!(
                        value.r <= 1.0 && value.g <= 1.0 && value.b <= 1.0,
                        "{} shows {} as a colour, but it starts at {value:?}",
                        material_type.label(),
                        param.label()
                    ),
                    ParamUi::Vector => assert!(
                        param.range().is_some_and(|r| r.high >= value.r.max(value.g).max(value.b)),
                        "{} shows {} as numbers with no range that holds {value:?}",
                        material_type.label(),
                        param.label()
                    ),
                    _ => {}
                }
            }
        }
    }
}

/// Does moving a control change the picture?
///
/// This project keeps finding the same defect from the other side: four
/// features were complete in the engine, tested, ticked in the roadmap, and
/// unreachable from the UI. The mirror image is a control that is reachable and
/// does nothing, and it is worse, because the user moves it and concludes the
/// renderer is broken rather than that the control is.
///
/// A parameter that a material type *exposes* is one of the handful that type
/// puts in front of someone. Each of those has to reach the shading, and the
/// only honest way to ask is to move it and look.
#[cfg(test)]
mod exposed_parameters_do_something {
    use crate::bsdf::Bsdf;
    use crate::lobes::OpenPbrBsdf;
    use crate::material_type::{MaterialType, Param, ParamUi, ParamValue};
    use crate::openpbr::Material;
    use tequila_core::math::{vec2, vec3};
    use tequila_core::spectrum::Spectrum;

    /// Everything about how a material shades, reduced to a list of numbers.
    ///
    /// Evaluation alone is not enough: a perfectly specular lobe evaluates to
    /// zero in every direction and carries all its behaviour in `sample`, so a
    /// glass material's IOR would look inert. Emission is not in the BSDF at
    /// all. All three go in.
    fn fingerprint(material: &Material) -> Vec<f32> {
        // As the renderer's flake field averages it: a point is on a flake
        // with probability equal to the amount, and the stack is linear in
        // coverage, so coverage at the amount is the surface's mean response.
        // Coverage zero would hide every flake control from this check, and
        // one would hide the paint beneath them. The tilt is fixed, since
        // which way a flake leans belongs to the field.
        let on_a_flake = Material {
            flake_coverage: material.flake_weight,
            flake_normal: vec3(0.25, 0.1, 1.0).normalize(),
            ..material.clone()
        };
        let bsdf = OpenPbrBsdf::new(&on_a_flake, 0.0);
        let mut out = Vec::new();

        let directions = [0.2f32, 0.5, 0.85, 1.4];
        for &a in &directions {
            let wo = vec3(a.sin(), 0.0, a.cos());
            for &b in &directions {
                for side in [1.0f32, -1.0] {
                    let wi = vec3(b.sin() * 0.7, b.sin() * 0.7, b.cos() * side);
                    let value = bsdf.eval(wo, wi.normalize());
                    out.extend_from_slice(&[value.r, value.g, value.b]);
                    out.push(bsdf.pdf(wo, wi.normalize()));
                }
            }
        }

        // Sampling, with fixed numbers so two runs are comparable. This is
        // what carries a delta lobe: the direction it picks and the weight it
        // returns are the whole of its behaviour.
        let wo = vec3(0.4, 0.0, 0.9).normalize();
        for i in 0..24 {
            let u = vec2((i as f32 + 0.5) / 24.0, ((i * 7) % 24) as f32 / 24.0);
            let uc = ((i * 11) % 24) as f32 / 24.0;
            match bsdf.sample(wo, u, uc) {
                Some(sample) => out.extend_from_slice(&[
                    sample.wi.x,
                    sample.wi.y,
                    sample.wi.z,
                    sample.f.r,
                    sample.f.g,
                    sample.f.b,
                    sample.pdf,
                    if sample.is_delta { 1.0 } else { 0.0 },
                ]),
                None => out.extend_from_slice(&[0.0; 8]),
            }
        }

        let emission = material.emission();
        out.extend_from_slice(&[emission.r, emission.g, emission.b]);

        // The medium a random walk crosses under the surface (§01 §6.4): the
        // walk is the integrator's, so the BSDF alone would not show how far
        // light travels inside or which way it scatters.
        match material.subsurface_medium() {
            Some(walk) => out.extend_from_slice(&[
                walk.extinction.r, walk.extinction.g, walk.extinction.b,
                walk.albedo.r, walk.albedo.g, walk.albedo.b,
                walk.anisotropy,
            ]),
            None => out.extend_from_slice(&[0.0; 7]),
        }
        // Dispersion (§01 §7.4): the index the path refracts by at the blue
        // and the red end, which the integrator picks by wavelength and the
        // BSDF alone would not show.
        out.extend_from_slice(&[
            crate::dispersion::ior_at(material.specular_ior, material.transmission_dispersion_abbe, 450.0),
            crate::dispersion::ior_at(material.specular_ior, material.transmission_dispersion_abbe, 650.0),
        ]);
        // And the scattering in a clear volume, which is the same walk.
        match material.volume_scattering() {
            Some(walk) => out.extend_from_slice(&[
                walk.extinction.r, walk.extinction.g, walk.extinction.b,
                walk.albedo.r, walk.albedo.g, walk.albedo.b,
                walk.anisotropy,
            ]),
            None => out.extend_from_slice(&[0.0; 7]),
        }

        // What a path through the body sees between its surfaces. A volume's
        // colour is absorption along the path and not at either surface
        // (§2.2), so a fingerprint of the BSDF alone would call it inert.
        // Always three numbers, so a parameter that only switches a volume
        // on is judged by what the volume does, not by the length changing.
        let through = if material.has_volume() {
            material.transmittance_over(0.1)
        } else {
            Spectrum::ONE
        };
        out.extend_from_slice(&[through.r, through.g, through.b]);
        out
    }

    fn differs(a: &[f32], b: &[f32]) -> bool {
        a.len() != b.len()
            || a.iter().zip(b).any(|(x, y)| {
                // NaN on one side only also counts as a difference.
                if x.is_nan() != y.is_nan() {
                    return true;
                }
                !x.is_nan() && (x - y).abs() > 1e-6
            })
    }

    /// A clearly different value for this parameter, within its own range.
    fn perturb(param: Param, material: &Material) -> Option<Material> {
        let mut changed = material.clone();
        let value = match (param.ui(), param.read(material)?) {
            (ParamUi::Scalar, ParamValue::Scalar(current)) => {
                let range = param.range();
                let (low, high) = range.map_or((0.0, 1.0), |r| (r.low, r.high));
                // Toward whichever end is further away, so the move is large
                // whatever the default happened to be.
                let target = if (current - low).abs() > (high - current).abs() {
                    low + (high - low) * 0.05
                } else {
                    low + (high - low) * 0.95
                };
                ParamValue::Scalar(target)
            }
            // A Vector is three independent numbers sharing a Spectrum — a
            // radius per channel, say. It arrives as a Color and is skipped by
            // the arm below unless it is named here, which is how the very
            // parameter that prompted this audit slipped past the first run.
            (ParamUi::Vector, ParamValue::Color(current)) => ParamValue::Color(Spectrum {
                r: current.r * 7.0 + 3.0,
                g: current.g * 7.0 + 3.0,
                b: current.b * 7.0 + 3.0,
            }),
            (ParamUi::Color, ParamValue::Color(current)) => {
                // A colour nothing else in a default material is, and far from
                // the current one in every channel.
                let target = Spectrum {
                    r: if current.r > 0.5 { 0.07 } else { 0.83 },
                    g: if current.g > 0.5 { 0.11 } else { 0.71 },
                    b: if current.b > 0.5 { 0.19 } else { 0.61 },
                };
                ParamValue::Color(target)
            }
            // A texture slot has no value, a metal is picked from a table, and
            // a flag is covered by the scalar path where it has one. None of
            // the three is a number this test can move.
            _ => return None,
        };
        param.write(&mut changed, value).then_some(changed)
    }

    /// Controls that are implemented but do nothing at their own type's
    /// defaults, which is a different complaint from a missing feature.
    ///
    /// Each is a consequence of the shading model rather than a gap in it, and
    /// each is listed with the reason so that a new one cannot join the list
    /// silently.
    const MASKED_AT_DEFAULTS: &[(MaterialType, Param, &str)] = &[
        // A fully transmissive surface has no diffuse lobe for a base colour
        // to tint. Glass is coloured through transmission, not through base.
        (MaterialType::Glass, Param::BaseColor, "transmission is 1"),
        (MaterialType::Gem, Param::BaseColor, "transmission is 1"),
        // Translucent hands its whole diffuse lobe to the subsurface colour,
        // so the base colour underneath it is fully replaced.
        (
            MaterialType::Translucent,
            Param::BaseColor,
            "subsurface weight is 1, so subsurface colour replaces it",
        ),
        // A clear liquid scatters nothing, so which way it would scatter is
        // moot until Scatter is raised — as it is for milk and juice.
        (
            MaterialType::Liquid,
            Param::ScatterAnisotropy,
            "the default liquid is clear; scatter is zero",
        ),
        // So does a clear plastic, until it is hazy as translucent filament.
        (
            MaterialType::PlasticTransparent,
            Param::ScatterAnisotropy,
            "the default clear plastic scatters nothing",
        ),
        // Anisotropy shapes a specular lobe, and cloth's is dominated by fuzz.
        (
            MaterialType::RealCloth,
            Param::Anisotropy,
            "the fuzz lobe carries this material, not the specular one",
        ),
        // Depth is how far the transmission colour takes to reach, and a
        // white colour is reached at any distance.
        (
            MaterialType::Dielectric,
            Param::TransmissionDepth,
            "the transmission colour is white, so there is nothing to absorb",
        ),
    ];

    /// Parameters that are implemented somewhere the fingerprint cannot see.
    ///
    /// `fingerprint` is the BSDF — evaluation, sampling, emission. A parameter
    /// the transport reads *before* building a BSDF leaves no mark on it, and
    /// is invisible here however well it works.
    ///
    /// This list is the narrow exception, and it is narrow on purpose: an
    /// entry here is an assertion that the feature is covered somewhere else,
    /// and it names where. Without that, "implemented outside the BSDF" would
    /// be an excuse that swallows the whole check.
    const IMPLEMENTED_OUTSIDE_THE_BSDF: &[(Param, &str)] = &[
        (
            Param::FuzzDensity,
            "fibres are geometry grown out of the part when the scene is built \
             (`tequila_scene::fuzz`), not a lobe of its surface. Covered by \
             tequila-scene's `fibres_follow_the_density_and_the_part_scale` and \
             the app's `fibres_render_alike_in_both_renderers`.",
        ),
        (
            Param::FuzzLength,
            "how long the grown fibres are. Covered by tequila-scene's \
             `fibres_stand_out_of_the_surface`.",
        ),
        (
            Param::FuzzThickness,
            "how wide the grown fibres are at the root. Covered by \
             tequila-scene's `fibres_taper_from_the_thickness_asked`.",
        ),
        (
            Param::FuzzLean,
            "how far the grown fibres lean. Covered by tequila-scene's \
             `fibres_lean_and_curl_as_asked`.",
        ),
        (
            Param::FuzzCurl,
            "how much the grown fibres bend. Covered by tequila-scene's \
             `fibres_lean_and_curl_as_asked`.",
        ),
        (
            Param::FuzzColorVariation,
            "how much each fibre's colour differs from the next: read at a \
             fibre, from its own random number, which no surface has. Covered \
             by tequila-render-core's `a_fibre_is_shaded_by_its_own_colour`.",
        ),
        (
            Param::EdgeRadius,
            "rounded edges bend the shading normal before the BSDF is built, by              probing the geometry around the shading point (Cycles' Bevel, and              tequila-render-core's `bevel`). Covered by the app's              `rounded_edges_catch_light_in_both_renderers`, against a render.",
        ),
        (
            Param::VolumeEmissionColor,
            "a glow inside is the medium's, picked up along the path through              it, not the surface's: the integrator adds `Material::emitted_over`              per segment. Covered by tequila-materials' `a_medium_glows_by_its_path_through_it`              and the app's `a_glowing_medium_glows_in_both_renderers`.",
        ),
        (
            Param::VolumeEmissionStrength,
            "with Glow Color: the medium's, along the path. Covered by              `a_medium_glows_by_its_path_through_it`.",
        ),
        (
            Param::EdgeSamples,
            "the Bevel's probe count, with Edge Radius. Covered by the app's              `rounded_edges_catch_light_in_cycles`.",
        ),
        (
            Param::MediumPriority,
            "which medium a path is in where two overlap is the \
             integrator's to track, not any one surface's to say. Covered by \
             tequila-render-core's `the_higher_priority_medium_is_the_one_present` \
             and tequila-render-cpu's `a_liquid_in_a_glass_keeps_its_colour`.",
        ),
        (
            Param::Opacity,
            "a cutout is decided before the BSDF is built — the ray either \
             meets the surface or carries on through it — so it changes \
             transport rather than shading. Covered by tequila-render-cpu's \
             `cutout_opacity` tests, against a render.",
        ),
        (
            Param::DisplacementDetail,
            "how finely the geometry is subdivided before the Displacement \
             height moves it: a property of the mesh the renderer builds, not \
             of the BSDF at a point on it. Covered by tequila-scene's \
             `displacement` tests.",
        ),
        (
            Param::FlakeSize,
            "how big the flake cells are: a property of the flake field over the surface, not of the BSDF at a point on one. Covered by tequila-render-core's `flakes` tests.",
        ),
        (
            Param::FlakeSpread,
            "how far flakes may lean: the field picks each flake's tilt within it, and the BSDF takes whichever tilt it is given. Covered by tequila-render-core's `flakes_cover_their_share_and_lean_within_the_spread`.",
        ),
        (
            Param::AnisotropyRotation,
            "which way the shading frame's tangent points: the lobe is the \
             same lobe in a turned frame, so a BSDF in a fixed one cannot show \
             it. Covered by tequila-render-cpu's `anisotropy` tests, against \
             a render.",
        ),
    ];

    /// The exemption above has to keep being true in the other direction too:
    /// a parameter that starts showing up in the fingerprint is one the main
    /// check can see for itself, and leaving it listed would hide it.
    #[test]
    fn nothing_is_excused_from_the_fingerprint_that_the_fingerprint_can_see() {
        let mut visible = Vec::new();
        for (param, _) in IMPLEMENTED_OUTSIDE_THE_BSDF {
            for material_type in MaterialType::ALL {
                let defaults = material_type.defaults();
                let Some(moved) = perturb(*param, &defaults) else {
                    continue;
                };
                if differs(&fingerprint(&defaults), &fingerprint(&moved)) {
                    visible.push(format!("{} on {material_type:?}", param.label()));
                }
            }
        }
        assert!(
            visible.is_empty(),
            "these are excused from the fingerprint as unreachable by it, and \
             the fingerprint now sees them; take them off \
             IMPLEMENTED_OUTSIDE_THE_BSDF so the main check covers them:\n  {}",
            visible.join("\n  ")
        );
    }

    #[test]
    fn every_exposed_parameter_reaches_the_shading() {
        let mut inert: Vec<String> = Vec::new();

        for material_type in MaterialType::ALL {
            // Advanced and Generic expose the whole catalog by design — §3.2's
            // "Nothing hidden." They are not a claim that every parameter in
            // it is implemented, so they are not evidence either way.
            if matches!(
                material_type,
                MaterialType::Advanced | MaterialType::Generic
            ) {
                continue;
            }

            let defaults = material_type.defaults();
            let before = fingerprint(&defaults);

            for param in material_type.exposed() {
                // Already declared as not implemented, and shown that way.
                if param.unimplemented().is_some() {
                    continue;
                }
                // Implemented, but not anywhere a BSDF fingerprint could show
                // it. The entry names the test that does cover it.
                if IMPLEMENTED_OUTSIDE_THE_BSDF
                    .iter()
                    .any(|(p, _)| *p == param)
                {
                    continue;
                }
                if MASKED_AT_DEFAULTS
                    .iter()
                    .any(|(t, p, _)| *t == material_type && *p == param)
                {
                    continue;
                }
                let Some(moved) = perturb(param, &defaults) else {
                    continue;
                };
                if !differs(&before, &fingerprint(&moved)) {
                    inert.push(format!("{material_type:?} · {}", param.label()));
                }
            }
        }

        assert!(
            inert.is_empty(),
            "these controls are offered by a material type and change nothing \
             about how it shades, so moving one looks like a broken renderer. \
             Either implement them, or say so in Param::unimplemented:\n  {}",
            inert.join("\n  ")
        );
    }

    /// The other direction, and the half that keeps the first honest.
    ///
    /// A parameter marked "not implemented" that has since been implemented
    /// would go on being greyed out in the inspector, with the feature sitting
    /// right there unusable — which is the exact failure this project has hit
    /// four times from the other side. So the marking is checked against the
    /// renderer rather than trusted.
    #[test]
    fn nothing_is_marked_unimplemented_that_actually_works() {
        let mut working: Vec<String> = Vec::new();

        for param in Param::ALL {
            if param.unimplemented().is_none() {
                continue;
            }
            // Against every type that offers it, because a parameter can be
            // masked on one material and live on another.
            for material_type in MaterialType::ALL {
                let defaults = material_type.defaults();
                let Some(moved) = perturb(param, &defaults) else {
                    continue;
                };
                if differs(&fingerprint(&defaults), &fingerprint(&moved)) {
                    working.push(format!("{} on {material_type:?}", param.label()));
                }
            }
        }

        assert!(
            working.is_empty(),
            "these are marked as not implemented but do change the shading, so \
             the inspector is greying out a control that works. Remove their \
             arm from Param::unimplemented:\n  {}",
            working.join("\n  ")
        );
    }

    /// And the masked list has to stay true as well: a parameter listed there
    /// that starts mattering at its type's defaults should be taken off it.
    #[test]
    fn the_masked_list_is_still_accurate() {
        for (material_type, param, reason) in MASKED_AT_DEFAULTS {
            let defaults = material_type.defaults();
            let Some(moved) = perturb(*param, &defaults) else {
                continue;
            };
            assert!(
                !differs(&fingerprint(&defaults), &fingerprint(&moved)),
                "{material_type:?} · {} is listed as masked ({reason}) but now \
                 changes the shading; take it off the list",
                param.label()
            );
        }
    }

}
