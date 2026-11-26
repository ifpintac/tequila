//! A solid's subsurface colour reaches Cycles. Blender 4's principled shader
//! has no colour of its own for what scatters below the surface, and uses
//! its base colour; Tequila's has one, so the translation gives Cycles that
//! as the base, as far as the part scatters. Given the base colour alone,
//! the Translucent preset's pink body rendered grey.
//!
//! Skipped where Cycles is not installed.

#[test]
fn a_translucent_part_scatters_its_own_colour() {
    use tequila_core::math::Vec3;
    use tequila_core::spectrum::Spectrum;
    use tequila_render_core::camera::Camera;
    use tequila_render_cycles::{translate, Session};

    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    // The demo's ball, made of the Translucent preset's colours: a grey base
    // and a pink below the surface, which is all it shows.
    let mut scene = tequila_scene::demo::demo_scene(tequila_scene::demo::DemoMaterial::Diffuse, 0.5);
    scene.materials[0] = tequila_render_core::material::Material {
        base_color: Spectrum::splat(0.18),
        subsurface_weight: 1.0,
        subsurface_color: Spectrum { r: 0.9, g: 0.75, b: 0.7 },
        subsurface_radius: Spectrum::splat(0.05),
        specular_weight: 0.0,
        ..tequila_render_core::material::Material::default()
    };
    let (width, height) = (96, 96);
    let lighting = tequila_render_core::settings::IntegratorConfig::product();
    let view = translate::View {
        camera: Camera::look_at(Vec3 { x: 0.0, y: 1.0, z: 4.0 }, Vec3 { x: 0.0, y: 1.0, z: 0.0 }, Vec3::Y),
        width,
        height,
        transparent: false,
        ground_illumination: false,
        light_materials: true,
        light_groups: false,
        shadow_glass: lighting.shadow_glass(),
        manifold_caustics: false,
    };
    let mut session = Session::new("CPU", 0, None).expect("a Cycles session");
    translate::build(&mut session, &scene, &view).expect("the scene translates");
    translate::configure(&mut session, &translate::Quality::with_lighting(false, false, None, &lighting)).expect("the settings take");
    let rgba = session.render(width, height, 32, |_, _| {}).expect("Cycles renders");

    // The middle of the ball.
    let (mut r, mut b) = (0.0f64, 0.0f64);
    for y in height / 2 - 6..height / 2 + 6 {
        for x in width / 2 - 6..width / 2 + 6 {
            let px = &rgba[(y * width + x) * 4..];
            r += px[0] as f64;
            b += px[2] as f64;
        }
    }
    assert!(r > 1.15 * b, "the ball is not pink: red {r:.3} against blue {b:.3} in its middle");
}
