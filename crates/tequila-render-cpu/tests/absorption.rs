//! Beer-Lambert absorption through a solid transmissive material.
//!
//! `docs/02-MATERIAL-SYSTEM.md` §2.2: `transmission_depth` is "the distance at
//! which `transmission_color` is reached via Beer-Lambert absorption", and
//! §3.1 separates **Glass** from **Glass (Solid)** on exactly this — a pane is
//! thin-walled with no volume, a block has one.
//!
//! The property worth testing is not "tinted", which a surface tint would also
//! produce. It is that the tint **deepens with distance travelled**: a thick
//! piece of green glass is a different colour from a thin one, and that is the
//! whole reason the parameter exists.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::{spectrum, Spectrum};
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

/// A transmissive sphere of radius `radius`, seen against the studio dome.
///
/// The sphere's size changes rather than the material, so the two runs share
/// one `transmission_depth` and differ only in the distance light travels
/// through the volume — which is the thing being measured.
fn render(radius: f32, depth: f32) -> Framebuffer {
    let studio = tequila_scene::demo::studio_environment();

    let material = Material {
        transmission_weight: 1.0,
        transmission_color: spectrum(0.35, 0.9, 0.5),
        transmission_depth: depth,
        geometry_thin_walled: false,
        specular_roughness: 0.0,
        specular_ior: 1.5,
        ..MaterialType::GlassSolid.defaults()
    };

    let scene = SceneSnapshot::new(
        vec![Arc::new(tequila_scene::demo::uv_sphere(radius, 64, 32))],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))
            .with_transform(Transform::IDENTITY)],
        vec![material],
        Arc::new(Environment::new(studio.image().clone(), 1.0)),
    );

    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(64, 64);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(
            vec3(0.0, 0.0, 4.0 * radius.max(1.0)),
            Vec3::ZERO,
            Vec3::Y,
        ),
        ..RenderContext::default()
    };
    render_pass(
        &intersector,
        &mut framebuffer,
        &context,
        &ProgressiveSchedule::default(),
        256,
    );
    framebuffer
}

/// The middle of the sphere, where the path through the glass is longest.
fn centre(image: &Framebuffer) -> Spectrum {
    let mut total = Spectrum::ZERO;
    let mut n = 0.0;
    for y in 28..36 {
        for x in 28..36 {
            total = total + image.pixel(x, y).mean();
            n += 1.0;
        }
    }
    total * (1.0 / n)
}

/// A thicker piece of the same glass absorbs more.
///
/// Against the same material at the same depth, so nothing but the distance
/// through the volume differs. A surface tint would give the two spheres the
/// same colour, which is what this separates from absorption.
#[test]
fn a_thicker_block_of_the_same_glass_is_darker_and_more_saturated() {
    let thin = centre(&render(0.35, 0.5));
    let thick = centre(&render(1.4, 0.5));

    assert!(
        thin.luminance() > 1e-4,
        "the thin sphere is black ({thin:?}), so there is nothing to compare"
    );
    assert!(
        thick.luminance() < thin.luminance() * 0.9,
        "a sphere four times thicker transmitted {:.4} against the thin one's \
         {:.4} — absorption is not scaling with distance, which is what \
         separates a volume from a tint",
        thick.luminance(),
        thin.luminance()
    );

    // And it saturates: the absorbed channels fall faster than the passed one.
    // Green is the least absorbed of the three, so its share has to rise.
    let share = |c: Spectrum| c.g / (c.r + c.g + c.b).max(1e-6);
    assert!(
        share(thick) > share(thin),
        "the thick sphere is merely darker, not more saturated ({:.3} against \
         {:.3}); Beer-Lambert is per channel and the least absorbed one must \
         come to dominate",
        share(thick),
        share(thin)
    );
}

/// `transmission_depth = 0` is the thin case, and must not absorb at all.
///
/// §2.2 is explicit that zero means no volume. Reading it as "absorb over zero
/// distance" would divide by it and turn every pane black.
#[test]
fn a_depth_of_zero_is_a_pane_and_absorbs_nothing() {
    let material = Material {
        transmission_color: spectrum(0.35, 0.9, 0.5),
        transmission_depth: 0.0,
        ..Material::default()
    };
    for distance in [0.0, 0.1, 1.0, 1000.0] {
        assert_eq!(
            material.transmittance_over(distance),
            Spectrum::ONE,
            "a pane absorbed something over {distance}"
        );
    }
}

/// The transmittance is the colour at exactly one depth, which is the
/// definition §2.2 gives.
#[test]
fn one_depth_of_travel_reaches_the_transmission_colour() {
    let material = Material {
        transmission_color: spectrum(0.25, 0.5, 0.75),
        transmission_depth: 2.0,
        ..Material::default()
    };

    let at_one = material.transmittance_over(2.0);
    assert!((at_one.r - 0.25).abs() < 1e-5, "red reached {}", at_one.r);
    assert!((at_one.g - 0.5).abs() < 1e-5);
    assert!((at_one.b - 0.75).abs() < 1e-5);

    // Twice the distance squares it, which is what makes it Beer-Lambert
    // rather than a linear fade.
    let at_two = material.transmittance_over(4.0);
    assert!((at_two.r - 0.0625).abs() < 1e-5, "red reached {}", at_two.r);
    assert!((at_two.b - 0.5625).abs() < 1e-5);

    // And nothing absorbs over no distance.
    assert_eq!(material.transmittance_over(0.0), Spectrum::ONE);
}

/// A fully absorbing channel goes to zero rather than to NaN.
#[test]
fn a_black_channel_absorbs_completely_and_stays_finite() {
    let material = Material {
        transmission_color: spectrum(0.0, 1.0, 0.5),
        transmission_depth: 1.0,
        ..Material::default()
    };
    let t = material.transmittance_over(3.0);
    assert_eq!(t.r, 0.0, "a black channel must absorb completely");
    assert_eq!(t.g, 1.0, "a white channel must pass completely");
    assert!(t.b.is_finite() && t.b > 0.0 && t.b < 1.0);
}
