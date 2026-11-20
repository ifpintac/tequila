//! A clear coat darkens the matte base under it by the light it turns back.
//!
//! What a matte base returns is spread over the hemisphere, and from inside
//! a coat of index 1.5 lets out only `(1 - E_F) / n²` of it (0.40), turning
//! the rest back for the base to absorb a share of again: Saunderson's
//! correction, and OpenPBR's coat darkening. Seen straight on in a white
//! world, a matte base of albedo `A` under the coat reads
//! `F + (1 - F) A (1 - K) / (1 - A K)`. Cycles' principled shader has no
//! part of it, and a grey 0.5 read 0.52 under a coat where it is 0.317.
//!
//! And under a coat the base's interface is against the coat, not the air
//! (OpenPBR's `n_b / n_c`): a plastic lacquered at its own index reflects no
//! second highlight from under the lacquer, where Cycles' principled took
//! the base's index against air whatever was over it.

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

fn coated(grey: f32, metalness: f32, specular: f32, coat: f32) -> Material {
    Material {
        base_color: Spectrum::splat(grey),
        base_metalness: metalness,
        specular_weight: specular,
        specular_ior: 1.5,
        specular_roughness: 0.3,
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
    let rgba = session.render(SIZE, SIZE, 1024, |_, _| {}).expect("renders");
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
fn a_clear_coat_darkens_a_matte_base_by_the_light_it_turns_back() {
    if tequila_render_cycles::available().is_err() {
        return;
    }
    let f = 0.04;
    let n = 1.5f32;
    let turned_back = 1.0 - (1.0 - tequila_materials::fresnel::dielectric_average(n)) / (n * n);
    for grey in [0.18f32, 0.5, 0.8] {
        let matte = middle(coated(grey, 0.0, 0.0, 1.0));
        let expected = f + (1.0 - f) * grey * (1.0 - turned_back) / (1.0 - grey * turned_back);
        assert!(
            (matte / expected - 1.0).abs() < 0.02,
            "grey {grey} under a clear coat reads {matte:.4}; the light the coat turns back makes it {expected:.4}"
        );
        // Lacquered at its own index, a plastic is its matte colour under
        // the lacquer: no surface between the two.
        let plastic = middle(coated(grey, 0.0, 1.0, 1.0));
        assert!(
            (plastic / matte - 1.0).abs() < 0.01,
            "grey {grey}: the lacquered plastic reads {plastic:.4}, the lacquered matte {matte:.4}"
        );
    }
    // A base partly metal and partly matte (a carbon weave's graphite) under
    // a coat: each share darkened its own way, weighed by metalness.
    let (metal, matte, both) = (middle(coated(0.184, 1.0, 1.0, 1.0)), middle(coated(0.184, 0.0, 1.0, 1.0)), middle(coated(0.184, 0.9, 1.0, 1.0)));
    let weighed = 0.9 * metal + 0.1 * matte;
    assert!(
        (both / weighed - 1.0).abs() < 0.015,
        "a 0.9 metal under a coat reads {both:.4}; its metal ({metal:.4}) and matte ({matte:.4}) shares weighed make {weighed:.4}"
    );
}
