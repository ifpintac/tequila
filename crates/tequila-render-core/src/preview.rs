//! A fast, deliberately non-physical shading of the scene, for scene setup.
//!
//! `TEQUILA_SPEC.md` §1.1 is emphatic that the viewport *is* the renderer, and
//! this does not change that: the path tracer remains the default and the only
//! thing that can produce an image anyone ships. What this adds is a mode for
//! the part of the work that is not look development — placing a part, aiming
//! a camera, finding which of forty brackets is `Bracket_017` — where a
//! physically correct image is not what is wanted, an instant one is, and the
//! machine's fans should not spin up for it.
//!
//! So it is not an *approximation* of the render, and no attempt is made to
//! have it converge towards one. It is a different picture with a different
//! job: unambiguous shape, readable colour, no noise, no wait. The HUD says so
//! while it is on, because a preview a person mistakes for the render is worse
//! than no preview at all.
//!
//! # What it is modelled on
//!
//! The solid-shading viewports of the CAD and DCC tools people already use.
//! Those are all built the same way and it is worth saying why, because the
//! choices look arbitrary until you see what each one is for:
//!
//! - **A world-space hemisphere** — bright from above, dark from below. This
//!   is the cue that makes a sphere read as a sphere resting under a sky
//!   rather than as a flat disc, and it is the one that has to be in world
//!   space: it must not swing as the camera orbits, or the object looks like
//!   it is tumbling.
//! - **A key and a fill fixed to the camera.** These do the opposite job: they
//!   guarantee nothing the camera can see is ever unlit, however the scene's
//!   real lights are aimed — which matters most before anyone has aimed them.
//!   Because they follow the view they cannot produce the dark side a
//!   world-fixed key would.
//! - **A screen-space background sweep** rather than the environment. The
//!   backdrop's job here is contrast against the product, not realism, and an
//!   HDRI in a mode with no light transport is a photograph of a room the
//!   render is not in.
//! - **A Fresnel rim**, which is what separates a dark part from a dark
//!   background at the silhouette.
//!
//! The values below are display-referred, and the mode is published through a
//! plain sRGB transform with no tone curve. A filmic curve exists to make
//! physical radiance printable, and there is no physical radiance here to
//! print — running one anyway is exactly what flattens a solid-mode image into
//! grey mush.
//!
//! It costs one camera ray and no shadow rays, so it runs several times faster
//! than a path-traced pass. There is no rasterizer here and no second scene
//! representation to keep in step — it traces the same BVH through the same
//! [`Intersector`], which is what keeps the selection, the picking and the
//! preview from ever disagreeing about where anything is.

use crate::intersect::Intersector;
use crate::material::Material;
use crate::shade;
use tequila_core::math::{vec3, Ray, Vec2, Vec3};
use tequila_core::spectrum::{spectrum, Spectrum};

/// The backdrop, top to bottom of the frame.
///
/// A studio sweep: a shade darker at the top than at the bottom, which is how
/// a cyclorama actually falls off and, more to the point, puts the darkest
/// part of the background behind the brightest part of most products.
pub const BACKDROP_TOP: Spectrum = spectrum(0.29, 0.30, 0.32);
pub const BACKDROP_BOTTOM: Spectrum = spectrum(0.46, 0.465, 0.475);

/// The world-space hemisphere. Faintly cool above and warm below, by a margin
/// nobody would name but everybody sees: a perfectly neutral hemisphere reads
/// as grey plastic whatever the material underneath it is.
pub const SKY: Spectrum = spectrum(0.97, 0.98, 1.0);
pub const GROUND_BOUNCE: Spectrum = spectrum(0.30, 0.285, 0.27);

/// A three-point rig fixed to the camera.
#[derive(Clone, Copy, Debug)]
pub struct Rig {
    pub key: Vec3,
    pub fill: Vec3,
    pub rim: Vec3,
}

impl Rig {
    /// Built from the camera basis, so "over the left shoulder" means what it
    /// says on screen.
    ///
    /// Built once for a frame and handed to every ray. Per-ray it would swivel
    /// very slightly across the film — imperceptible, and about a fifth of the
    /// cost of shading a pixel, which is not.
    pub fn for_view(forward: Vec3, up: Vec3) -> Rig {
        let forward = forward.try_normalize().unwrap_or(vec3(0.0, 0.0, -1.0));
        let right = forward.cross(up).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(forward).try_normalize().unwrap_or(Vec3::Y);
        Rig {
            // Over the viewer's left shoulder and high: the conventional key,
            // and the one that reads as "lit from above" rather than as a
            // torch strapped to the camera.
            key: (-forward * 0.55 + up * 0.62 - right * 0.55)
                .try_normalize()
                .unwrap_or(Vec3::Y),
            fill: (-forward * 0.55 + right * 0.75 - up * 0.25)
                .try_normalize()
                .unwrap_or(Vec3::X),
            rim: (forward * 0.7 + up * 0.55 + right * 0.3)
                .try_normalize()
                .unwrap_or(Vec3::Y),
        }
    }
}

/// What one preview ray produced. The subset of a path result this mode can
/// honestly fill in.
#[derive(Clone, Copy, Debug)]
pub struct PreviewSample {
    pub radiance: Spectrum,
    pub albedo: Spectrum,
    pub normal: Vec3,
    pub position: Vec3,
    pub depth: f32,
    pub escaped: bool,
    pub object_id: u32,
    pub material_id: u32,
}

/// The backdrop at a point on the film. `film.y` runs 0 at the top.
#[inline]
pub fn backdrop(film: Vec2) -> Spectrum {
    BACKDROP_TOP.lerp(BACKDROP_BOTTOM, film.y.clamp(0.0, 1.0))
}

/// Shade one camera ray.
///
/// `film` is the normalized film position, used for the backdrop sweep — so
/// the gradient is fixed to the frame, the way a real cyclorama is fixed to
/// the studio rather than to the camera's rotation.
/// Ignore the synthetic render floor when inspecting or selecting solids,
/// and a part's fibres (§4.4's Fuzz): what is under them is the solid, and
/// a preview of shape is no place for fur.
pub fn intersect_solid(intersector: &dyn Intersector, mut ray: Ray) -> Option<crate::intersect::Hit> {
    loop {
        let hit = intersector.intersect(&ray)?;
        let scene = intersector.scene();
        let solid = scene.instances.get(hit.instance.0 as usize).is_none_or(|i| {
            i.shadow_catcher.is_none() && scene.meshes.get(i.mesh.index()).is_none_or(|m| m.fibres.is_none())
        });
        if solid {
            return Some(hit);
        }
        let next = hit.t + 1e-5 * hit.t.abs().max(1.0);
        if next <= ray.t_min || !next.is_finite() { return None; }
        ray.t_min = next;
    }
}

/// The parameters [`shade_ray`] reads. Everything else a material computes —
/// normal maps above all — is never looked at here.
pub const SOLID_PARAMS: [tequila_materials::material_type::Param; 11] = {
    use tequila_materials::material_type::Param::*;
    [
        BaseWeight,
        BaseColor,
        BaseMetalness,
        SpecularWeight,
        SpecularRoughness,
        CoatWeight,
        CoatRoughness,
        TransmissionWeight,
        TransmissionColor,
        EmissionLuminance,
        EmissionColor,
    ]
};

/// Each material's program reduced to [`SOLID_PARAMS`], built once a frame.
///
/// A textured material's full program samples every map it has — colour,
/// roughness, metalness, normal — at every pixel. Solid shading uses the
/// colour and a few scalars, so the rest was pure cost: half of every Solid
/// frame on a textured model.
#[derive(Debug)]
pub struct SolidPrograms(Vec<Option<tequila_materials::graph::CompiledGraph>>);

impl SolidPrograms {
    pub fn for_scene(scene: &crate::scene::SceneSnapshot) -> SolidPrograms {
        SolidPrograms(
            scene
                .programs
                .iter()
                .map(|program| program.as_ref().map(|p| p.restricted_to(&SOLID_PARAMS)))
                .collect(),
        )
    }
}

pub fn shade_ray(
    intersector: &dyn Intersector,
    ray: Ray,
    film: Vec2,
    rig: &Rig,
    registers: &mut shade::Registers,
    scratch: &mut Material,
) -> PreviewSample {
    shade_ray_with(intersector, ray, film, rig, None, registers, scratch)
}

/// [`shade_ray`], running `programs` in place of the scene's full ones.
pub fn shade_ray_with(
    intersector: &dyn Intersector,
    ray: Ray,
    film: Vec2,
    rig: &Rig,
    programs: Option<&SolidPrograms>,
    registers: &mut shade::Registers,
    scratch: &mut Material,
) -> PreviewSample {
    let scene = intersector.scene();

    let Some(hit) = intersect_solid(intersector, ray) else {
        let radiance = backdrop(film);
        return PreviewSample {
            radiance,
            albedo: radiance,
            normal: Vec3::ZERO,
            position: Vec3::ZERO,
            depth: f32::INFINITY,
            escaped: true,
            object_id: u32::MAX,
            material_id: u32::MAX,
        };
    };

    let (_, normal) = hit.facing_forward(ray.direction);
    let program = match programs {
        Some(solid) => solid.0.get(hit.material.index()).and_then(|p| p.as_ref()),
        None => scene.program(hit.material).map(|p| p.as_ref()),
    };
    let material = match program {
        Some(program) => {
            let point = scene.shading_point_along(&hit, normal, crate::settings::DEFAULT_PIXEL_SPREAD, ray.direction);
            shade::shade(program, &scene.textures, &point, registers, scratch)
        }
        None => scene.material(hit.material),
    };

    let view = -ray.direction;
    let base = material.solid_color() * material.base_weight.clamp(0.0, 1.0);

    // --- form -------------------------------------------------------------
    //
    // The hemisphere carries the shape and the camera rig carries the
    // crispness. Weighted so the hemisphere dominates: a rig-dominated image
    // is the flat, headlamp-lit look that makes a badly built solid mode
    // recognisable from across a room.
    let hemisphere = GROUND_BOUNCE.lerp(SKY, 0.5 + 0.5 * normal.y);
    let key = normal.dot(rig.key).max(0.0);
    let fill = normal.dot(rig.fill).max(0.0);
    let lighting = hemisphere * 0.62 + Spectrum::splat(key * 0.42 + fill * 0.14);
    let mut colour = base * lighting;

    // --- highlight ---------------------------------------------------------
    let roughness = material
        .specular_roughness
        .clamp(0.02, 1.0)
        .max(material.coat_roughness.clamp(0.0, 1.0) * material.coat_weight);
    let half = (rig.key + view).try_normalize().unwrap_or(normal);
    let cosine = normal.dot(half);
    let weight = material.specular_weight.clamp(0.0, 1.0) * (1.0 - roughness * 0.8);
    // `powf` is the most expensive operation here by some way, and most pixels
    // are nowhere near the highlight — off-axis, or on a material with no
    // specular lobe. Both are answered before paying for it.
    if cosine > 0.0 && weight > 0.0 {
        let exponent = (2.0 / (roughness * roughness).max(1e-4)).min(2048.0);
        // A metal's highlight takes the metal's colour and a dielectric's is
        // white. That one difference is most of what separates chrome from
        // white plastic at a glance.
        let metalness = material.base_metalness.clamp(0.0, 1.0);
        let tint = Spectrum::splat(1.0).lerp(base, metalness);
        colour += tint * (cosine.powf(exponent) * weight * 0.9);
    }

    // --- silhouette --------------------------------------------------------
    //
    // Without this a dark part against a dark backdrop has no edge at all, and
    // seeing where things are is the whole point of the mode.
    let facing = normal.dot(view).abs().clamp(0.0, 1.0);
    let fresnel = (1.0 - facing) * (1.0 - facing);
    let rim = normal.dot(rig.rim).max(0.0);
    colour += Spectrum::splat(fresnel * fresnel * (0.10 + 0.14 * rim));

    // --- transmission ------------------------------------------------------
    //
    // Shown by letting the backdrop through rather than by refracting it. A
    // preview that traced glass properly would be a path tracer with extra
    // steps; what is needed here is to see which part is the glass one and
    // what is behind it.
    // A cloudy volume sends light back as well, lit as the surface is.
    let transmission = material.transmission_weight.clamp(0.0, 1.0);
    if transmission > 0.0 {
        let behind = backdrop(film) * material.see_through_tint();
        let cloud = material.preview_cloud() * lighting;
        colour = colour.lerp(behind + cloud, transmission * 0.8);
    }

    colour += material.emission();

    PreviewSample {
        radiance: colour,
        albedo: base,
        normal,
        position: hit.position,
        depth: hit.t,
        escaped: false,
        object_id: hit.instance.0,
        material_id: hit.material.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::Environment;
    use crate::scene::SceneSnapshot;
    use crate::test_support::{AnalyticIntersector, Sphere};
    use std::sync::Arc;

    fn one_sphere(material: Material, environment: Spectrum) -> AnalyticIntersector {
        let scene = SceneSnapshot::new(
            Vec::new(),
            Vec::new(),
            vec![material],
            Arc::new(Environment::uniform(environment)),
        );
        AnalyticIntersector::new(scene).with_sphere(Sphere::new(
            Vec3::ZERO,
            1.0,
            tequila_core::ids::MaterialId(0),
        ))
    }

    fn rig() -> Rig {
        Rig::for_view(vec3(0.0, 0.0, -1.0), Vec3::Y)
    }

    /// Shade a straight-on grid over the sphere, returning the film coordinate
    /// and luminance at every pixel that hit it.
    fn sweep(material: Material, resolution: usize) -> Vec<(Vec2, f32)> {
        let intersector = one_sphere(material, Spectrum::ZERO);
        let mut registers = shade::Registers::new();
        let mut scratch = Material::default();
        let rig = rig();
        let mut out = Vec::new();
        for j in 0..resolution {
            for i in 0..resolution {
                let u = i as f32 / (resolution - 1) as f32;
                let v = j as f32 / (resolution - 1) as f32;
                let ray = Ray::new(
                    vec3((u - 0.5) * 1.9, (0.5 - v) * 1.9, 4.0),
                    vec3(0.0, 0.0, -1.0),
                );
                let sample = shade_ray(
                    &intersector,
                    ray,
                    Vec2 { x: u, y: v },
                    &rig,
                    &mut registers,
                    &mut scratch,
                );
                if !sample.escaped {
                    out.push((Vec2 { x: u, y: v }, sample.radiance.luminance()));
                }
            }
        }
        out
    }

    /// The property that makes the mode worth having: nothing the camera can
    /// see is ever unlit, whatever the scene's lights are doing — because they
    /// are not consulted. The scene here has no lights and a black
    /// environment, which is to say the path tracer renders it correctly and
    /// entirely black.
    #[test]
    fn nothing_facing_the_camera_is_unlit() {
        let lit = sweep(
            Material {
                base_color: Spectrum::splat(0.5),
                ..Material::default()
            },
            32,
        );
        assert!(!lit.is_empty(), "the sweep missed the sphere");
        let darkest = lit.iter().map(|(_, l)| *l).fold(f32::MAX, f32::min);
        assert!(
            darkest > 0.02,
            "the darkest pixel on the product came out at {darkest}"
        );
    }

    /// And it has *form*: the top of a sphere is markedly brighter than the
    /// bottom, because the hemisphere is world-space. Without that a solid
    /// preview is a flat disc — which is the specific way this mode fails, and
    /// it fails while still passing "nothing is black".
    #[test]
    fn a_sphere_reads_as_a_sphere() {
        let lit = sweep(
            Material {
                base_color: Spectrum::splat(0.6),
                // No highlight, so this measures the diffuse form alone and
                // cannot be satisfied by a lucky specular hit.
                specular_weight: 0.0,
                ..Material::default()
            },
            48,
        );
        let mean_over = |predicate: fn(f32) -> bool| {
            let picked: Vec<f32> = lit
                .iter()
                .filter(|(uv, _)| predicate(uv.y))
                .map(|(_, l)| *l)
                .collect();
            picked.iter().sum::<f32>() / picked.len().max(1) as f32
        };
        let top = mean_over(|y| y < 0.25);
        let bottom = mean_over(|y| y > 0.75);
        assert!(
            top > bottom * 1.8,
            "the top of the sphere reads {top} and the bottom {bottom}; that is \
             a flat disc, not a sphere"
        );
    }

    /// The backdrop is a screen-space sweep, not the environment: fixed to the
    /// frame so it does not swing as the camera orbits, and independent of the
    /// HDRI because a photograph of a room is not a useful backdrop for a mode
    /// with no light transport in it.
    #[test]
    fn the_backdrop_is_a_frame_fixed_sweep() {
        let intersector = one_sphere(Material::default(), spectrum(1.0, 0.0, 0.0));
        let mut registers = shade::Registers::new();
        let mut scratch = Material::default();
        let rig = rig();
        let mut miss = |film: Vec2| {
            let sample = shade_ray(
                &intersector,
                Ray::new(vec3(0.0, 4.0, 4.0), vec3(0.0, 1.0, 0.0)),
                film,
                &rig,
                &mut registers,
                &mut scratch,
            );
            assert!(sample.escaped);
            sample.radiance
        };
        let top = miss(Vec2 { x: 0.5, y: 0.0 });
        let bottom = miss(Vec2 { x: 0.5, y: 1.0 });
        assert!(
            bottom.luminance() > top.luminance() * 1.3,
            "the sweep runs {} at the top and {} at the bottom",
            top.luminance(),
            bottom.luminance()
        );
        assert!(
            top.r < 0.4 && bottom.r < 0.6,
            "a red environment leaked into the backdrop: {top:?}"
        );
    }

    /// A rough surface and a polished one have to look different, or the mode
    /// cannot do the job people will use it for: picking a material off the
    /// shelf and seeing at a glance which one they picked.
    ///
    /// The measurement is the highlight's *spread*, isolated by differencing
    /// against the same material with the specular lobe switched off.
    /// Isolating it is the point: dimming a highlight as roughness rises is
    /// easy, looks plausible in a still, and is not what reads as polish — a
    /// small hot spot against a large soft one is. Measured on the shaded
    /// image as a whole this would pass on an implementation that ignored
    /// roughness entirely. It does not, measured here.
    /// A cloudy liquid previews as what it is. Milk scatters light back long
    /// before it lets the backdrop through, and the previews counted only
    /// what a volume absorbs, so a glass of milk showed as a glass of water.
    #[test]
    fn milk_previews_as_milk_not_water() {
        let centre = |material: Material| {
            let intersector = one_sphere(material, Spectrum::ZERO);
            let sample = shade_ray(
                &intersector,
                Ray::new(vec3(0.0, 0.3, 4.0), vec3(0.0, 0.0, -1.0)),
                Vec2 { x: 0.5, y: 0.42 },
                &rig(),
                &mut shade::Registers::new(),
                &mut Material::default(),
            );
            sample.radiance.to_linear_srgb()
        };
        let water = Material {
            transmission_depth: 0.01,
            transmission_color: spectrum(0.99, 0.98, 0.87),
            ..tequila_materials::material_type::MaterialType::Liquid.defaults()
        };
        let milk = Material {
            transmission_scatter: spectrum(25.0, 32.0, 38.0),
            ..water.clone()
        };
        let (water, milk) = (centre(water), centre(milk));
        let luminance = |c: Vec3| 0.2126 * c.x + 0.7152 * c.y + 0.0722 * c.z;
        assert!(
            luminance(milk) > 1.5 * luminance(water),
            "milk previews at {milk:?}, no brighter than water at {water:?}"
        );
        assert!(milk.x > 0.5 && milk.z > 0.35, "milk should preview near white; it is {milk:?}");
    }

    #[test]
    fn polish_shows_as_a_tighter_highlight() {
        let spread = |roughness: f32| {
            let base = Material {
                base_color: Spectrum::splat(0.4),
                specular_roughness: roughness,
                ..Material::default()
            };
            let with = sweep(base.clone(), 96);
            let without = sweep(
                Material {
                    specular_weight: 0.0,
                    ..base
                },
                96,
            );
            let lobe: Vec<f32> = with
                .iter()
                .zip(&without)
                .map(|((_, a), (_, b))| (a - b).max(0.0))
                .collect();
            let peak = lobe.iter().copied().fold(0.0f32, f32::max);
            assert!(peak > 1e-4, "no highlight at all at roughness {roughness}");
            lobe.iter().filter(|v| **v > peak * 0.5).count() as f32 / lobe.len() as f32
        };

        let polished = spread(0.05);
        let matte = spread(0.9);
        assert!(
            polished < matte * 0.5,
            "the polished highlight covers {:.2}% of the sphere and the matte \
             one {:.2}%; the two are not telling a person anything",
            polished * 100.0,
            matte * 100.0
        );
    }
}
