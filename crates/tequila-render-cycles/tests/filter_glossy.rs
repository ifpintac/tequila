//! What Cycles is sent unless asked otherwise does not blur light away.
//!
//! Filter Glossy roughens what a path meets after a diffuse or rough bounce,
//! to cut the noise of the caustics it finds there. Through clear glass that
//! is light lost: a part inside a clear case is lit by bounces that leave it
//! through the glass, and made rough, the glass sent them back in and round
//! until the bounce limits ended them. On the palma P8 it took 44% of the
//! light reaching the circuit board through the shell, which Tequila's
//! renderer and Cycles both find with it off. Tequila sent Cycles Filter
//! Glossy 1 until 2026-11-19; now none unless the project sets it.
//!
//! Skipped where Cycles is not installed.

use std::sync::Arc;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::camera::Camera;
use tequila_render_core::environment::Environment;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cycles::{translate, Session};

const SIZE: usize = 48;

/// A sphere of radius 1 with a spherical cavity of radius `inner`, one
/// closed mesh, the cavity's faces wound towards the hollow.
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

/// A white matte ball inside a hollow sphere of clear glass, in a white
/// world: nothing in it absorbs, so all of it reads the world's 1 but for
/// what a limit or a blur takes. The glass a little rough, as the palma's
/// shell is: mirror-smooth glass is passed straight on by the bounces
/// weighed against the shadow rays (`Builder::shadows_through`), and Filter
/// Glossy never meets it.
fn scene() -> SceneSnapshot {
    let glass = Material { transmission_color: Spectrum::ONE, specular_roughness: 0.1, ..MaterialType::GlassSolid.defaults() };
    let white = Material { base_color: Spectrum::ONE, specular_weight: 0.0, ..MaterialType::Plastic.defaults() };
    let parts = [(shell(0.9), glass), (tequila_scene::demo::uv_sphere(0.6, 64, 32), white)];
    let (mut meshes, mut instances, mut materials) = (Vec::new(), Vec::new(), Vec::new());
    for (i, (mesh, material)) in parts.into_iter().enumerate() {
        meshes.push(Arc::new(mesh));
        instances.push(Instance::new(InstanceId(i as u32), MeshId(i as u32), MaterialId(i as u32)).with_transform(Transform::IDENTITY));
        materials.push(material);
    }
    SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(Spectrum::ONE)))
}

/// The mean over the middle of a Cycles picture of the ball, from 3.5 in
/// front, with `lighting`.
fn render(lighting: &IntegratorConfig) -> f64 {
    let view = translate::View {
        camera: Camera::look_at(vec3(0.0, 0.0, 3.5), Vec3::ZERO, Vec3::Y),
        width: SIZE,
        height: SIZE,
        transparent: false,
        ground_illumination: false,
        light_materials: true,
        light_groups: false,
        shadow_glass: lighting.shadow_glass(),
        manifold_caustics: false,
    };
    let mut session = Session::new("CPU", 0, None).expect("a Cycles session");
    let translation = translate::build(&mut session, &scene(), &view).expect("the scene translates");
    for warning in &translation.warnings {
        eprintln!("translation: {warning}");
    }
    translate::configure(&mut session, &translate::Quality::with_lighting(false, false, None, lighting)).expect("the settings take");
    let rgba = session.render(SIZE, SIZE, 256, |_, _| {}).expect("Cycles renders");
    let mut sum = 0.0f64;
    let mut count = 0.0;
    for y in SIZE * 3 / 8..SIZE * 5 / 8 {
        for x in SIZE * 3 / 8..SIZE * 5 / 8 {
            let px = &rgba[(y * SIZE + x) * 4..];
            sum += (px[0] as f64 + px[1] as f64 + px[2] as f64) / 3.0;
            count += 1.0;
        }
    }
    sum / count
}

/// At the Product preset's limits with nothing set, the ball inside the
/// glass reads what it reads with every limit lifted; with Filter Glossy 1,
/// what Cycles was sent before, it reads darker.
#[test]
fn nothing_set_blurs_no_light_away_behind_glass() {
    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    let product = IntegratorConfig::product();
    assert_eq!(product.cycles.filter_glossy, None);
    let mut open = IntegratorConfig { max_bounces: 64, max_transmission_bounces: 64, ..product.clone() };
    open.cycles.filter_glossy = Some(0.0);
    let mut blurred = product.clone();
    blurred.cycles.filter_glossy = Some(1.0);
    let (open, unset, blurred) = (render(&open), render(&product), render(&blurred));
    println!("every limit lifted {open:.4}  nothing set {unset:.4}  Filter Glossy 1 {blurred:.4}");
    assert!((open - 1.0).abs() < 0.05, "the white furnace is off with every limit lifted: {open:.4}");
    assert!(
        (unset - open).abs() < 0.03,
        "with nothing set the ball in the glass reads {unset:.4}, with every limit lifted {open:.4}"
    );
    assert!(blurred < unset - 0.03, "Filter Glossy 1 takes no light from the ball in the glass ({blurred:.4} against {unset:.4}): the test sees nothing");
}
