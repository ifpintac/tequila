//! Render passes, rendered (`docs/01-RENDERER-CORE.md` §12).
//!
//! The claim §12 makes is that the passes are "consistent by construction" —
//! that they come out of one mechanism rather than being computed separately —
//! and the observable form of that claim is M4's exit criterion: **Beauty is
//! the sum of its components**. That is what this file checks, on a real image
//! rather than on the expressions, because the expressions partitioning the
//! paths is necessary and not sufficient: a contribution can still be routed to
//! the wrong pass, counted twice, or dropped.
//!
//! The scene is chosen to exercise every component. A diffuse floor, a mirror,
//! a glass sphere and an emissive panel between them produce diffuse,
//! specular-reflection, transmission and direct-versus-indirect paths in one
//! image — a scene of one matte sphere would make every component but Diffuse
//! zero and the sum would come out right for no reason at all.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::aov::AovSet;
use tequila_render_core::camera::Camera;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::{AovBuffers, Framebuffer};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass_with_aovs, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: usize = 48;

fn quad(half: f32, y: f32, up: bool) -> Mesh {
    let n = if up { Vec3::Y } else { -Vec3::Y };
    let indices = if up {
        vec![0, 2, 1, 0, 3, 2]
    } else {
        vec![0, 1, 2, 0, 2, 3]
    };
    Mesh::new(
        vec![
            vec3(-half, y, -half),
            vec3(half, y, -half),
            vec3(half, y, half),
            vec3(-half, y, half),
        ],
        indices,
    )
    .with_normals(vec![n; 4])
}

/// An icosphere-ish ball, wound outwards.
fn sphere(radius: f32, rings: usize, segments: usize) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for ring in 0..=rings {
        let theta = ring as f32 / rings as f32 * std::f32::consts::PI;
        for segment in 0..=segments {
            let phi = segment as f32 / segments as f32 * std::f32::consts::TAU;
            let n = vec3(theta.sin() * phi.cos(), theta.cos(), theta.sin() * phi.sin());
            positions.push(n * radius);
            normals.push(n);
        }
    }
    let mut indices = Vec::new();
    for ring in 0..rings {
        for segment in 0..segments {
            let a = (ring * (segments + 1) + segment) as u32;
            let b = a + segments as u32 + 1;
            indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    Mesh::new(positions, indices).with_normals(normals)
}

/// Floor, mirror ball, glass ball, and a panel of light above them.
fn scene() -> SceneSnapshot {
    let matte = Material {
        base_color: Spectrum::splat(0.6),
        ..MaterialType::Diffuse.defaults()
    };
    let mirror = Material {
        specular_roughness: 0.0,
        ..MaterialType::Metal.defaults()
    };
    let glass = Material {
        specular_roughness: 0.0,
        ..MaterialType::GlassSolid.defaults()
    };
    let emitter = Material {
        emission_color: Spectrum::ONE,
        emission_luminance: 30.0,
        ..MaterialType::Emissive.defaults()
    };

    let ball = Arc::new(sphere(0.5, 24, 32));
    SceneSnapshot::new(
        vec![
            Arc::new(quad(4.0, 0.0, true)),
            ball.clone(),
            ball,
            Arc::new(quad(1.2, 3.0, false)),
        ],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1))
                .with_transform(Transform::translation(vec3(-0.8, 0.5, 0.0))),
            Instance::new(InstanceId(2), MeshId(2), MaterialId(2))
                .with_transform(Transform::translation(vec3(0.8, 0.5, 0.0))),
            Instance::new(InstanceId(3), MeshId(3), MaterialId(3)),
        ],
        vec![matte, mirror, glass, emitter],
        // Dark, so every photon in the image comes from the panel and the
        // partition has no environment term hiding in it.
        Arc::new(Environment::uniform(Spectrum::ZERO)),
    )
}

fn camera() -> Camera {
    Camera::look_at(vec3(0.0, 1.6, 3.6), vec3(0.0, 0.5, 0.0), Vec3::Y)
}

/// Render, returning the beauty buffer and the passes beside it.
fn render(set: &AovSet, samples: u32) -> (Framebuffer, AovBuffers) {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene()).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let mut aovs = AovBuffers::new(SIZE, SIZE, set);
    let context = RenderContext {
        camera: camera(),
        // Firefly rejection removes energy on purpose, and the exit criterion
        // is an energy identity. Auditing that removal is the Clamped Energy
        // pass's job; here it would just be a term the sum is missing.
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass_with_aovs(
        &intersector,
        &mut framebuffer,
        Some((&mut aovs, set)),
        &context,
        &ProgressiveSchedule::default(),
        samples,
    );
    (framebuffer, aovs)
}

fn pass_index(set: &AovSet, name: &str) -> usize {
    set.light_paths
        .iter()
        .position(|pass| pass.name == name)
        .unwrap_or_else(|| panic!("no pass called {name}"))
}

/// M4's exit criterion, at the level a renderer can assert it.
///
/// Every path the beauty pass counts is counted by exactly one of the
/// first-bounce components, so the two sums must agree — not approximately, but
/// to floating-point tolerance, because they are literally the same
/// contributions added in a different order.
#[test]
fn beauty_is_the_sum_of_its_first_bounce_components() {
    let mut set = AovSet::standard();
    // The first-bounce partition: diffuse, specular reflection, glossy
    // reflection, transmission. Glossy is not in §12's table — the table names
    // the passes a compositor asks for, not a partition — so it is added here,
    // and saying so is better than a test that quietly leaves a quarter of the
    // paths out.
    set.light_paths
        .push(tequila_render_core::aov::LightPathPass::new("Glossy", "C<RG>.*L").unwrap());
    set.cryptomatte = false;

    let (framebuffer, aovs) = render(&set, 128);
    let samples = framebuffer.min_samples();
    assert!(samples > 0);

    let beauty = pass_index(&set, "Beauty");
    let components: Vec<usize> = ["Diffuse", "Reflection", "Refraction", "Glossy"]
        .iter()
        .map(|name| pass_index(&set, name))
        .collect();

    let mut worst = 0.0f32;
    let mut total_beauty = 0.0f64;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let whole = aovs.passes.mean(beauty, x, y, samples);
            let sum = components
                .iter()
                .fold(Spectrum::ZERO, |acc, i| acc + aovs.passes.mean(*i, x, y, samples));
            total_beauty += whole.luminance() as f64;
            let difference = (whole.luminance() - sum.luminance()).abs();
            let scale = whole.luminance().abs().max(1e-4);
            worst = worst.max(difference / scale);
        }
    }
    assert!(total_beauty > 1.0, "the render is empty");
    assert!(
        worst < 1e-4,
        "the components differ from the beauty pass by {worst} of it at worst"
    );
}

/// And the beauty pass is the image. A set of passes that agreed with each
/// other but not with the render would be self-consistently wrong.
#[test]
fn the_beauty_pass_is_the_rendered_image() {
    let mut set = AovSet::standard();
    set.cryptomatte = false;
    let (framebuffer, aovs) = render(&set, 96);
    let samples = framebuffer.min_samples();
    let beauty = pass_index(&set, "Beauty");

    let mut worst = 0.0f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let image = framebuffer.pixel(x, y).mean().luminance();
            let pass = aovs.passes.mean(beauty, x, y, samples).luminance();
            worst = worst.max((image - pass).abs() / image.abs().max(1e-4));
        }
    }
    assert!(
        worst < 1e-4,
        "the beauty pass differs from the image by {worst} of it at worst"
    );
}

/// Direct and indirect have to add up too, and this is the pair most likely to
/// drift: they are separated by a repetition count in the expression rather
/// than by the kind of the first bounce.
#[test]
fn direct_and_indirect_sum_to_the_reflected_whole() {
    let mut set = AovSet::standard();
    set.light_paths
        .push(tequila_render_core::aov::LightPathPass::new("AllReflected", "C<R.>.*L").unwrap());
    set.cryptomatte = false;

    let (framebuffer, aovs) = render(&set, 128);
    let samples = framebuffer.min_samples();
    let whole = pass_index(&set, "AllReflected");
    let direct = pass_index(&set, "Lighting");
    let indirect = pass_index(&set, "GlobalIllumination");

    let mut worst = 0.0f32;
    let mut any = false;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let all = aovs.passes.mean(whole, x, y, samples).luminance();
            let sum = aovs.passes.mean(direct, x, y, samples).luminance()
                + aovs.passes.mean(indirect, x, y, samples).luminance();
            if all > 1e-3 {
                any = true;
            }
            worst = worst.max((all - sum).abs() / all.abs().max(1e-4));
        }
    }
    assert!(any, "nothing was lit by reflection at all");
    assert!(
        worst < 1e-4,
        "direct plus indirect differs from the whole by {worst} at worst"
    );
}

/// Each component has to be non-empty in a scene built to produce it, or the
/// sums above would balance with three of four terms at zero.
#[test]
fn every_component_pass_has_something_in_it() {
    let mut set = AovSet::standard();
    set.cryptomatte = false;
    let (framebuffer, aovs) = render(&set, 128);
    let samples = framebuffer.min_samples();

    for name in ["Diffuse", "Reflection", "Refraction", "Lighting"] {
        let index = pass_index(&set, name);
        let total: f64 = (0..SIZE)
            .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
            .map(|(x, y)| aovs.passes.mean(index, x, y, samples).luminance() as f64)
            .sum();
        assert!(
            total > 1e-3,
            "the {name} pass is empty, so the scene does not exercise it"
        );
    }
}

/// The Cryptomatte has to select the things that are there, with partial
/// coverage on the edges. This is the other half of M4's exit criterion.
#[test]
fn the_cryptomatte_selects_objects_and_finds_edges() {
    let set = AovSet::standard();
    let (framebuffer, mut aovs) = render(&set, 64);
    aovs.finish(framebuffer.min_samples());

    let objects = aovs.objects.as_ref().expect("an object Cryptomatte");
    let mut edge_pixels = 0usize;
    let mut solid_pixels = 0usize;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let ranks = objects.ranks_at(x, y);
            let covered: f32 = ranks.iter().map(|(_, c)| *c).sum();
            if covered < 1e-4 {
                continue;
            }
            // A pixel's coverage sums to the fraction of it that is *geometry*.
            // Not to one: a Cryptomatte mattes objects, and the background is
            // not an object, so a pixel half on a silhouette sums to a half.
            // The framebuffer's alpha is the same fraction, arrived at
            // independently, so the two agreeing is a real check rather than a
            // restatement.
            let alpha = framebuffer.pixel(x, y).alpha();
            assert!(
                (covered - alpha).abs() < 1e-3,
                "coverage at ({x}, {y}) sums to {covered} where alpha is {alpha}"
            );
            let live = ranks.iter().filter(|(_, c)| *c > 1e-4).count();
            if live > 1 {
                edge_pixels += 1;
            } else {
                solid_pixels += 1;
            }
        }
    }
    assert!(solid_pixels > 100, "hardly anything is covered at all");
    assert!(
        edge_pixels > 5,
        "no pixel has partial coverage, so the matte is an ID pass with extra \
         steps"
    );
}

/// §5.5's light layers: every contribution ends at exactly one light, so a
/// pass per light group — each layer, the environment, glowing materials —
/// adds up to Beauty, and a light's pass holds that light and nothing else.
/// That identity is what post-render relighting rescales.
#[test]
fn the_light_groups_add_up_to_beauty() {
    use tequila_render_core::light::{Light, LightKind, LightSet, PointLight};
    let lights = vec![
        Light::new(LightKind::Point(PointLight {
            position: vec3(-2.0, 2.5, 2.0),
            intensity: Spectrum::splat(8.0),
            radius: 0.2,
        }))
        .with_layer(0),
        Light::new(LightKind::Point(PointLight {
            position: vec3(2.0, 1.5, 2.5),
            intensity: Spectrum { r: 6.0, g: 2.0, b: 1.0 },
            radius: 0.2,
        }))
        .with_layer(3),
    ];
    let mut snapshot = scene().with_lights(Arc::new(LightSet::new(lights)));
    // Some sky as well, so the environment's group has something in it.
    snapshot.environment = Arc::new(Environment::uniform(Spectrum::splat(0.3)));
    let mut set = AovSet::standard_for(&snapshot);
    set.cryptomatte = false;
    let names: Vec<&str> = set.light_paths.iter().map(|p| p.name.as_str()).collect();
    for wanted in ["LightLayer0", "LightLayer3", "Environment", "Emissive"] {
        assert!(names.contains(&wanted), "no {wanted} pass among {names:?}");
    }
    assert!(!names.contains(&"LightLayer1"), "a pass for a layer no light is on");

    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, snapshot).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let mut aovs = AovBuffers::new(SIZE, SIZE, &set);
    let context = RenderContext {
        camera: camera(),
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass_with_aovs(&intersector, &mut framebuffer, Some((&mut aovs, &set)), &context, &ProgressiveSchedule::default(), 64);
    let samples = framebuffer.min_samples();

    let beauty = pass_index(&set, "Beauty");
    let groups: Vec<usize> = ["LightLayer0", "LightLayer3", "Environment", "Emissive"]
        .iter()
        .map(|name| pass_index(&set, name))
        .collect();
    let mut worst = 0.0f32;
    let mut totals = vec![0.0f64; groups.len()];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let whole = aovs.passes.mean(beauty, x, y, samples);
            let mut sum = Spectrum::ZERO;
            for (k, &g) in groups.iter().enumerate() {
                let part = aovs.passes.mean(g, x, y, samples);
                totals[k] += part.luminance() as f64;
                sum = sum + part;
            }
            let difference = (whole.luminance() - sum.luminance()).abs();
            worst = worst.max(difference / whole.luminance().abs().max(1e-4));
        }
    }
    assert!(worst < 1e-4, "the light groups differ from Beauty by {worst} of it at worst");
    for (name, total) in ["LightLayer0", "LightLayer3", "Environment", "Emissive"].iter().zip(&totals) {
        assert!(*total > 0.1, "the {name} pass is empty: {total}");
    }
    // The warm light is on layer 3 alone: its pass is redder than layer 0's.
    let colour = |pass: usize| {
        let mut sum = Spectrum::ZERO;
        for y in 0..SIZE {
            for x in 0..SIZE {
                sum = sum + aovs.passes.mean(pass, x, y, samples);
            }
        }
        sum
    };
    let (warm, white) = (colour(groups[1]), colour(groups[0]));
    assert!(warm.r / warm.b > 2.0 * white.r / white.b, "layer 3 did not get the warm light: {warm:?} against {white:?}");
}

/// With the ground on, the Beauty pass is still the image: the floor the
/// camera sees, with its shadows, is in it. It was not — the shadow catcher
/// added to the image and to no pass — so a multi-layer EXR's Beauty had a
/// hole where the floor was, and so would a relit render.
#[test]
fn the_beauty_pass_includes_the_ground() {
    use tequila_render_core::scene::ShadowCatcher;
    let ball = Arc::new(sphere(0.5, 24, 32));
    let snapshot = SceneSnapshot::new(
        vec![Arc::new(quad(4.0, 0.0, true)), ball],
        vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_shadow_catcher(ShadowCatcher::default()),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(0))
                .with_transform(Transform::translation(vec3(0.0, 0.5, 0.0))),
        ],
        vec![Material {
            base_color: Spectrum::splat(0.6),
            ..MaterialType::Diffuse.defaults()
        }],
        Arc::new(Environment::uniform(Spectrum::splat(0.8))),
    );
    let mut set = AovSet::standard_for(&snapshot);
    set.cryptomatte = false;
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, snapshot).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let mut aovs = AovBuffers::new(SIZE, SIZE, &set);
    let context = RenderContext {
        camera: camera(),
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass_with_aovs(&intersector, &mut framebuffer, Some((&mut aovs, &set)), &context, &ProgressiveSchedule::default(), 32);
    let samples = framebuffer.min_samples();
    let beauty = pass_index(&set, "Beauty");
    let mut worst = 0.0f32;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let image = framebuffer.pixel(x, y).mean().luminance();
            let pass = aovs.passes.mean(beauty, x, y, samples).luminance();
            worst = worst.max((image - pass).abs() / image.abs().max(1e-3));
        }
    }
    assert!(worst < 1e-3, "the Beauty pass differs from the image by {worst} of it at worst");
}

