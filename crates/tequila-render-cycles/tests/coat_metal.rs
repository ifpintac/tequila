//! A metal under a clear coat loses the coat's share of its light twice.
//!
//! Light crosses a clear coat down to the metal and back up, and part of
//! what the metal returns is reflected down again for another try. Seen
//! straight on in a white world, a coated metal whose uncoated self reads
//! `A` reads `F + (1 - F)^2 A / (1 - F A)`, `F` the coat's reflectance
//! straight on (0.04 at an index of 1.5): Tequila's renderer does. Cycles'
//! principled shader takes the coat's share once, and read about 3% over:
//! a clear-coated grey metal (carbon fibre's graphite) 0.206 where the coat
//! makes it 0.200. Now 0.201.

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

fn metal(grey: f32, metalness: f32, coat: f32) -> Material {
    Material {
        base_color: Spectrum::splat(grey),
        base_metalness: metalness,
        specular_roughness: 0.18,
        coat_weight: coat,
        coat_roughness: 0.015,
        coat_ior: 1.5,
        ..MaterialType::Generic.defaults()
    }
}

/// The middle of a ball of `material` in a white world, seen straight on.
fn middle(material: Material) -> f32 {
    let scene = SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(1.0, 128, 64))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)],
        vec![material],
        Arc::new(Environment::uniform(Spectrum::ONE)),
    );
    let mut lighting = IntegratorConfig { max_bounces: 64, max_transmission_bounces: 64, ..IntegratorConfig::product() };
    lighting.cycles.filter_glossy = Some(0.0);
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
    translate::build(&mut session, &scene, &view).expect("translates");
    translate::configure(&mut session, &translate::Quality::with_lighting(false, false, None, &lighting)).expect("configures");
    let rgba = session.render(SIZE, SIZE, 512, |_, _| {}).expect("renders");
    let mut sum = 0.0f64;
    for y in SIZE / 2 - 4..SIZE / 2 + 4 {
        for x in SIZE / 2 - 4..SIZE / 2 + 4 {
            let i = (y * SIZE + x) * 4;
            sum += (rgba[i] + rgba[i + 1] + rgba[i + 2]) as f64 / 3.0;
        }
    }
    (sum / 64.0) as f32
}

#[test]
fn a_clear_coat_takes_its_share_both_ways() {
    if tequila_render_cycles::available().is_err() {
        return;
    }
    let f = 0.04;
    for (grey, metalness) in [(0.184, 0.9), (0.184, 1.0), (0.6, 1.0)] {
        let bare = middle(metal(grey, metalness, 0.0));
        let coated = middle(metal(grey, metalness, 1.0));
        let expected = f + (1.0 - f) * (1.0 - f) * bare / (1.0 - f * bare);
        assert!(
            (coated / expected - 1.0).abs() < 0.01,
            "grey {grey} at metalness {metalness}: {bare:.4} bare, {coated:.4} coated, where two crossings of the coat make {expected:.4}"
        );
    }
}
