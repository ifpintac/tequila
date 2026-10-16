//! A hollow piece of glass â€” a bottle, a tumbler, a vase â€” passes on the
//! light that reaches it.
//!
//! The integrator's own white furnace covers a *solid* glass sphere: two
//! crossings, glass in and out. A container is four â€” into the wall, out into
//! the air inside, into the far wall, out again â€” and the medium the path is
//! in has to be right at every one of them: air, glass, air, glass, air. Lose
//! track once and the cavity is taken for glass, or the far wall for air, and
//! a clear bottle comes out grey.
//!
//! In a uniform white environment a lossless object is invisible: every pixel
//! reads the environment's radiance whatever the object does to the paths.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

/// A sphere of radius 1 with a spherical cavity of radius `inner`, as one
/// closed mesh: the cavity's surface wound inward, so its normals face the
/// hollow, as a modeller's shell or a turned bottle has them.
fn shell(inner: f32) -> Mesh {
    let outside = tequila_scene::demo::uv_sphere(1.0, 96, 48);
    let cavity = tequila_scene::demo::uv_sphere(inner, 96, 48);
    let offset = outside.positions.len() as u32;
    let mut positions = outside.positions.clone();
    positions.extend(cavity.positions.iter().copied());
    let mut normals = outside.normals.clone();
    normals.extend(cavity.normals.iter().map(|n| -*n));
    let mut indices = outside.indices.clone();
    for triangle in cavity.indices.chunks(3) {
        indices.extend_from_slice(&[triangle[0] + offset, triangle[2] + offset, triangle[1] + offset]);
    }
    Mesh::new(positions, indices).with_normals(normals)
}

fn render(mesh: Mesh, material: Material) -> Framebuffer {
    render_with(mesh, material, IntegratorConfig { max_bounces: 64, max_transmission_bounces: 64, ..IntegratorConfig::product() })
}

fn render_with(mesh: Mesh, material: Material, config: IntegratorConfig) -> Framebuffer {
    render_parts(vec![(mesh, material)], config)
}

/// Several parts, each its own instance with its own material.
fn render_parts(parts: Vec<(Mesh, Material)>, config: IntegratorConfig) -> Framebuffer {
    render_parts_in(parts, config, Environment::uniform(Spectrum::ONE))
}

fn render_parts_in(parts: Vec<(Mesh, Material)>, config: IntegratorConfig, environment: Environment) -> Framebuffer {
    let mut meshes = Vec::new();
    let mut instances = Vec::new();
    let mut materials = Vec::new();
    for (i, (mesh, material)) in parts.into_iter().enumerate() {
        meshes.push(Arc::new(mesh));
        instances.push(
            Instance::new(InstanceId(i as u32), MeshId(i as u32), MaterialId(i as u32))
                .with_transform(Transform::IDENTITY),
        );
        materials.push(material);
    }
    let scene = SceneSnapshot::new(meshes, instances, materials, Arc::new(environment));
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(48, 48);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 3.5), Vec3::ZERO, Vec3::Y),
        // Enough bounces that the depth limit is not what is measured: a
        // path in a shell can reflect inside a wall many times.
        config,
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 256);
    framebuffer
}

/// The mean over the middle of the image, all of it inside the object.
fn centre(image: &Framebuffer) -> f32 {
    centre_colour(image).luminance()
}

fn centre_colour(image: &Framebuffer) -> Spectrum {
    let mut total = Spectrum::ZERO;
    let mut n = 0.0;
    for y in 18..30 {
        for x in 18..30 {
            total = total + image.pixel(x, y).mean();
            n += 1.0;
        }
    }
    total * (1.0 / n)
}

fn deep() -> IntegratorConfig {
    IntegratorConfig { max_bounces: 64, max_transmission_bounces: 64, ..IntegratorConfig::product() }
}

fn clear_glass() -> Material {
    Material {
        transmission_color: Spectrum::ONE,
        ..MaterialType::GlassSolid.defaults()
    }
}

#[test]
fn a_hollow_clear_glass_passes_on_its_light() {
    let solid = centre(&render(tequila_scene::demo::uv_sphere(1.0, 96, 48), clear_glass()));
    let hollow = centre(&render(shell(0.9), clear_glass()));
    println!("solid {solid:.4}  hollow {hollow:.4}");
    assert!((solid - 1.0).abs() < 0.03, "the solid control is off: {solid:.4}");
    assert!(
        (hollow - 1.0).abs() < 0.03,
        "a clear hollow sphere in a white furnace reads {hollow:.4}; light is \
         lost at the cavity"
    );
}


/// A liquid in a glass keeps its colour (`docs/01` §6.1).
///
/// Modelled the way every filled container is: the liquid's surface a
/// fraction of a millimetre into the glass wall, so no air gap can open
/// between them, and the liquid's higher medium priority deciding the
/// overlap. With one medium tracked and no priorities, the path that
/// entered the glass and then crossed the liquid's surface took that as
/// *leaving* — and travelled through the liquid as though through air, so
/// an amber drink rendered grey.
#[test]
fn a_liquid_in_a_glass_keeps_its_colour() {
    let liquid = Material {
        transmission_color: tequila_core::spectrum::spectrum(0.8, 0.4, 0.1),
        transmission_depth: 0.5,
        ..MaterialType::Liquid.defaults()
    };
    let fill = || tequila_scene::demo::uv_sphere(0.905, 96, 48);
    let alone = centre_colour(&render_parts(vec![(fill(), liquid.clone())], deep()));
    let in_glass = centre_colour(&render_parts(vec![(shell(0.9), clear_glass()), (fill(), liquid)], deep()));
    let hue = |c: Spectrum| {
        let rgb = c.to_linear_srgb();
        (rgb.y / rgb.x.max(1e-6), rgb.z / rgb.x.max(1e-6))
    };
    let (alone_g, alone_b) = hue(alone);
    let (glass_g, glass_b) = hue(in_glass);
    println!("alone {alone_g:.3} {alone_b:.3}   in glass {glass_g:.3} {glass_b:.3}");
    // The control: the liquid on its own is strongly amber.
    assert!(alone_g < 0.3, "the liquid alone is not amber: green/red {alone_g:.3}");
    assert!(
        (glass_g - alone_g).abs() < 0.08 && (glass_b - alone_b).abs() < 0.08,
        "the liquid alone is {alone_g:.3} green and {alone_b:.3} blue to its red, and in \
         the glass {glass_g:.3} and {glass_b:.3}: its colour does not survive the glass"
    );
}

/// The interface between two media is between their indices.
///
/// Fill the glass with a clear liquid of the glass's own index and the
/// boundary between them vanishes: the filled shell is, optically, one solid
/// sphere. It only is if the index there is the ratio of the two, and not
/// the liquid's own against air. Seen against the studio, whose lights and
/// walls show every change in where a path goes.
#[test]
fn an_index_matched_liquid_makes_its_glass_one_solid() {
    let studio = || {
        let studio = tequila_scene::demo::studio_environment();
        Environment::new(studio.image().clone(), 1.0)
    };
    let liquid = |ior: f32| Material {
        transmission_color: Spectrum::ONE,
        specular_ior: ior,
        ..MaterialType::Liquid.defaults()
    };
    let fill = || tequila_scene::demo::uv_sphere(0.905, 96, 48);
    let solid = render_parts_in(vec![(tequila_scene::demo::uv_sphere(1.0, 96, 48), clear_glass())], deep(), studio());
    let matched = render_parts_in(vec![(shell(0.9), clear_glass()), (fill(), liquid(1.5))], deep(), studio());
    // The control: a liquid of a clearly different index.
    let other = render_parts_in(vec![(shell(0.9), clear_glass()), (fill(), liquid(1.1))], deep(), studio());

    let difference = |a: &Framebuffer, b: &Framebuffer| {
        let mut total = 0.0;
        for y in 12..36 {
            for x in 12..36 {
                total += (a.pixel(x, y).mean().luminance() - b.pixel(x, y).mean().luminance()).abs();
            }
        }
        total / (24.0 * 24.0)
    };
    let matched_off = difference(&solid, &matched);
    let other_off = difference(&solid, &other);
    println!("index-matched differs by {matched_off:.4}, another index by {other_off:.4}");
    // The control: a liquid of another index is visibly not a solid sphere.
    assert!(other_off > 0.02, "a liquid of index 1.1 looks like solid glass ({other_off:.4}); the test can see nothing");
    assert!(
        matched_off < other_off * 0.25,
        "an index-matched liquid differs from solid glass by {matched_off:.4}, against \
         {other_off:.4} for another index: the interface between them is still bending light"
    );
}
