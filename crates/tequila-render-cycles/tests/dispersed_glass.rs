//! Glass that disperses passes in Cycles what the same glass passes
//! without dispersion: in a white world, the middle of a smooth tinted ball
//! seen head on reads what its front face reflects plus the tint on all the
//! rest, F0 + c (1 - F0) - light in through the front and out at the back
//! is tinted once, and every reflection inside comes out the same way. The
//! dispersed glass is three glasses, one a colour channel; built of
//! Cycles' Glass BSDF, which tints its reflections too, a garnet's face
//! reflected red and every reflection inside darkened the stone again.
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
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cycles::{translate, Session};

const SIZE: usize = 64;

/// The middle of a Cycles picture of a ball of radius 1 in `material`, from
/// 3.5 in front, in a white world, with every limit lifted.
fn middle(material: Material) -> [f64; 3] {
    let scene = SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 128, 64))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)],
        vec![material],
        Arc::new(Environment::uniform(Spectrum::ONE)),
    );
    let lighting = IntegratorConfig { max_bounces: 64, max_transmission_bounces: 64, ..IntegratorConfig::product() };
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
    let translation = translate::build(&mut session, &scene, &view).expect("the scene translates");
    for warning in &translation.warnings {
        eprintln!("translation: {warning}");
    }
    translate::configure(&mut session, &translate::Quality::with_lighting(false, false, None, &lighting)).expect("the settings take");
    let rgba = session.render(SIZE, SIZE, 1024, |_, _| {}).expect("Cycles renders");
    let mut sum = [0.0f64; 3];
    let mut count = 0.0;
    for y in SIZE / 2 - 2..SIZE / 2 + 2 {
        for x in SIZE / 2 - 2..SIZE / 2 + 2 {
            for c in 0..3 {
                sum[c] += rgba[(y * SIZE + x) * 4 + c] as f64;
            }
            count += 1.0;
        }
    }
    sum.map(|s| s / count)
}

#[test]
fn a_dispersed_garnet_passes_what_an_undispersed_one_does() {
    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    // The Garnet preset: index 1.79, Abbe 56, its colour at the surface.
    let colour = Spectrum { r: 0.3575, g: 0.0854, b: 0.0777 };
    let garnet = Material { specular_ior: 1.79, transmission_color: colour, transmission_dispersion_abbe: 56.0, ..MaterialType::Gem.defaults() };
    let still = Material { transmission_dispersion_abbe: 0.0, ..garnet.clone() };
    let f0 = ((1.79f64 - 1.0) / 2.79).powi(2);
    let expected = [colour.r, colour.g, colour.b].map(|c| f0 + c as f64 * (1.0 - f0));
    let (dispersed, undispersed) = (middle(garnet), middle(still));
    println!("expected {expected:.4?}\nundispersed {undispersed:.4?}\ndispersed {dispersed:.4?}");
    for (c, channel) in ["red", "green", "blue"].iter().enumerate() {
        assert!(
            (undispersed[c] - expected[c]).abs() < 0.02 * expected[c] + 0.002,
            "the undispersed control passes {:.4} {channel} where a tinted ball passes {:.4}",
            undispersed[c],
            expected[c]
        );
        assert!(
            (dispersed[c] - expected[c]).abs() < 0.03 * expected[c] + 0.002,
            "the dispersed garnet passes {:.4} {channel} in Cycles where a tinted ball passes {:.4}",
            dispersed[c],
            expected[c]
        );
    }
}
