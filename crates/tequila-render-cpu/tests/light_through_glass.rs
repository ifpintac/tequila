//! Light through glass onto what is under it: a floor beneath a slab of
//! clear glass is lit by a lamp, a glowing panel and the sky through the
//! slab, as it would be through a window - less what the slab's two faces
//! reflect, and no more. A shadow ray stopped by the glass left the floor
//! lit by the lamp not at all and by the sky only through bounces that
//! happened to refract into it, as speckle; one let through as well as the
//! bounce, both counted in full, would light it twice.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::light::{AreaLight, Light, LightKind, LightSet};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: usize = 32;

/// How far to the side the lamp and the panel are, 2.5 up.
const LIGHT_X: f32 = 1.2;

/// A square in the plane `y = height`, facing up or down.
fn square(height: f32, half: f32, up: bool) -> Mesh {
    let corners = vec![vec3(-half, height, -half), vec3(half, height, -half), vec3(half, height, half), vec3(-half, height, half)];
    let indices = if up { vec![0, 2, 1, 0, 3, 2] } else { vec![0, 1, 2, 0, 2, 3] };
    let normal = if up { Vec3::Y } else { -Vec3::Y };
    Mesh::new(corners, indices).with_normals(vec![normal; 4])
}

/// A closed slab from `low` to `high`, `half` across each way.
fn slab(low: f32, high: f32, half: f32) -> Mesh {
    let p = |x: f32, y: f32, z: f32| vec3(x * half, y, z * half);
    let corners = vec![
        p(-1.0, low, -1.0), p(1.0, low, -1.0), p(1.0, low, 1.0), p(-1.0, low, 1.0),
        p(-1.0, high, -1.0), p(1.0, high, -1.0), p(1.0, high, 1.0), p(-1.0, high, 1.0),
    ];
    let indices = vec![
        0, 1, 2, 0, 2, 3, // bottom
        4, 6, 5, 4, 7, 6, // top
        0, 4, 5, 0, 5, 1, // back
        1, 5, 6, 1, 6, 2, // right
        2, 6, 7, 2, 7, 3, // front
        3, 7, 4, 3, 4, 0, // left
    ];
    Mesh::new(corners, indices)
}

fn clear_glass() -> Material {
    Material { transmission_color: Spectrum::ONE, transmission_depth: 0.0, ..MaterialType::GlassSolid.defaults() }
}

#[derive(Clone, Copy, Debug)]
enum Lighting {
    /// A small analytic area light high above, off to the side so that its
    /// reflection in the glass is out of the picture's middle.
    Lamp,
    /// A panel of glowing geometry where the lamp is, which a bounce can
    /// also find.
    Panel,
    /// A uniform sky.
    Sky,
}

/// The floor's luminance in the middle of the picture, and how much it
/// varies from pixel to pixel there.
fn render(lighting: Lighting, cover: Option<Material>, config: IntegratorConfig, samples: u32) -> (f32, f32) {
    // Dark, so that little of its light comes back to it off the glass.
    let floor = Material { base_color: Spectrum::splat(0.1), ..MaterialType::Diffuse.defaults() };
    // Wider than the glass, so the sky below the floor cannot light the
    // glass from underneath and be reflected down onto the floor by it.
    let mut meshes = vec![Arc::new(square(0.0, 6.0, true))];
    let mut materials = vec![floor];
    let mut instances = vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))];
    let mut add = |mesh: Mesh, material: Material, meshes: &mut Vec<Arc<Mesh>>, materials: &mut Vec<Material>| {
        let i = meshes.len() as u32;
        meshes.push(Arc::new(mesh));
        materials.push(material);
        instances.push(Instance::new(InstanceId(i), MeshId(i), MaterialId(i)));
    };
    if let Some(glass) = cover {
        // Wide enough that the sky past its edges is a percent of what
        // the floor sees.
        add(slab(0.5, 0.54, 5.0), glass, &mut meshes, &mut materials);
    }
    if let Lighting::Panel = lighting {
        let glow = Material { emission_color: Spectrum::ONE, emission_luminance: 40.0, ..MaterialType::Emissive.defaults() };
        let (x, y, half) = (LIGHT_X, 2.5, 0.3);
        let corners = vec![vec3(x - half, y, -half), vec3(x + half, y, -half), vec3(x + half, y, half), vec3(x - half, y, half)];
        let panel = Mesh::new(corners, vec![0, 1, 2, 0, 2, 3]).with_normals(vec![-Vec3::Y; 4]);
        add(panel, glow, &mut meshes, &mut materials);
    }
    let sky = if let Lighting::Sky = lighting { Spectrum::ONE } else { Spectrum::ZERO };
    let mut scene = SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(sky)));
    if let Lighting::Lamp = lighting {
        let lamp = AreaLight {
            position: vec3(LIGHT_X, 2.5, 0.0),
            half_u: vec3(0.15, 0.0, 0.0),
            half_v: vec3(0.0, 0.0, 0.15),
            radiance: Spectrum::splat(300.0),
            disc: false,
            two_sided: false,
            spread: std::f32::consts::PI,
        };
        scene = scene.with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Area(lamp))])));
    }
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        // Straight down from under the glass, so the picture is the floor
        // alone, not the sky reflected in the glass as well.
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.3, 0.01), Vec3::ZERO, Vec3::Y),
        config,
        reject_fireflies: false,
        ..RenderContext::default()
    };
    for _ in 0..samples {
        render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 1);
    }
    centre(&framebuffer)
}

fn centre(framebuffer: &Framebuffer) -> (f32, f32) {
    let mut values = Vec::new();
    for y in SIZE / 3..SIZE * 2 / 3 {
        for x in SIZE / 3..SIZE * 2 / 3 {
            values.push(framebuffer.pixel(x, y).mean().luminance());
        }
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / values.len() as f32;
    (mean, variance.sqrt())
}

/// What two faces of 1.5 glass let by, met at `cos` to their normal.
fn two_faces(cos: f32) -> f32 {
    let face = 1.0 - tequila_materials::fresnel::dielectric(cos, 1.5);
    face * face
}

/// The same over the sky, as a matte floor gathers it: weighted by cosine,
/// and with what the faces reflect back and forth between them, which a
/// bounce through the slab finds and which counts here, where a small lamp
/// is lit through it by the light sample.
fn two_faces_over_the_sky() -> f32 {
    let steps = 2000;
    let (mut through, mut total) = (0.0f64, 0.0f64);
    for i in 0..steps {
        let theta = (i as f64 + 0.5) / steps as f64 * std::f64::consts::FRAC_PI_2;
        let weight = theta.cos() * theta.sin();
        let face = f64::from(tequila_materials::fresnel::dielectric(theta.cos() as f32, 1.5));
        through += weight * (1.0 - face) / (1.0 + face);
        total += weight;
    }
    (through / total) as f32
}

#[test]
fn a_floor_under_clear_glass_is_lit_through_it_once() {
    let lamp = two_faces(2.5 / (2.5f32 * 2.5 + LIGHT_X * LIGHT_X).sqrt());
    for (lighting, expected) in [(Lighting::Lamp, lamp), (Lighting::Panel, lamp), (Lighting::Sky, two_faces_over_the_sky())] {
        let (bare, _) = render(lighting, None, IntegratorConfig::product(), 64);
        let (covered, _) = render(lighting, Some(clear_glass()), IntegratorConfig::product(), 64);
        let through = covered / bare;
        println!("{lighting:?}: {covered:.4} under glass against {bare:.4} bare, {through:.3} through, {expected:.3} expected");
        assert!(bare > 0.005, "{lighting:?} lit nothing: {bare}");
        assert!((through - expected).abs() < 0.04, "{lighting:?}: the floor under glass had {through:.3} of its light, not {expected:.3}");
    }
}

#[test]
fn a_lamp_through_glass_is_no_noisier_than_without_it() {
    let (bare, bare_noise) = render(Lighting::Lamp, None, IntegratorConfig::product(), 16);
    let (covered, covered_noise) = render(Lighting::Lamp, Some(clear_glass()), IntegratorConfig::product(), 16);
    let (bare_relative, covered_relative) = (bare_noise / bare, covered_noise / covered);
    println!("noise: {covered_relative:.4} under glass, {bare_relative:.4} bare");
    assert!(covered_relative < bare_relative * 1.5 + 0.01, "the lamp through glass was noisy: {covered_relative:.4} against {bare_relative:.4}");
}

#[test]
fn with_caustics_on_the_photons_carry_the_light_instead() {
    // The shadow ray stops at the glass again, and the floor is lit by the
    // photons rather than twice.
    let config = IntegratorConfig { caustics: true, ..IntegratorConfig::product() };
    let (bare, _) = render(Lighting::Lamp, None, config.clone(), 64);
    let (covered, _) = render(Lighting::Lamp, Some(clear_glass()), config, 64);
    let through = covered / bare;
    println!("caustics: {through:.3} through");
    assert!(through < 1.0, "with caustics on the floor under glass was lit twice: {through:.3}");
}

