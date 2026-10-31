//! Glass in a glossy reflection is glass. A nearly mirror-smooth floor shows
//! a lamp through a prism where the prism bends its light, as a mirror
//! shows a glowing panel there - the panel, which a bounce can meet, and the
//! mirror, every bounce off which sees the glass, being the light along the
//! bent path and nothing else. Passed straight through by the floor's
//! bounces, as its light samples pass it, the prism showed the floor nothing
//! of the lamp: the light sample looks for it along the straight line, off
//! the floor's narrow lobe, and a bounce cannot meet a lamp. A lamp and a
//! glowing panel the same are weighed alike between the light samples and
//! the bounces, so the floor shows them alike at any roughness; and through
//! a flat slab, where the straight line and the bent path nearly agree, the
//! lamp, found both ways, is counted once.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::fresnel::metals;
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

/// The light glows at this radiance.
const RADIANCE: f32 = 10.0;

/// The camera, looking down at the floor's origin, which mirrors it along
/// [`frame`]'s first way up to the glass.
const CAMERA: Vec3 = Vec3 { x: 0.0, y: 1.0, z: 2.0 };

#[derive(Clone, Copy, Debug, PartialEq)]
enum Glass {
    /// A prism of 30 degrees across the mirrored way, its base up, which
    /// bends the light 16 degrees upwards.
    Prism,
    /// A slab 5 cm thick square to it, which lets it by straight.
    Slab,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lighting {
    /// An analytic area light, which no bounce meets but one that sees
    /// glass.
    Lamp,
    /// A glowing panel the same size, radiance and place.
    Panel,
}

/// The way the floor's origin mirrors the camera, and the two at right
/// angles to it, upwards and across.
fn frame() -> (Vec3, Vec3, Vec3) {
    (vec3(0.0, 1.0, -2.0).normalize(), vec3(0.0, 2.0, 1.0).normalize(), Vec3::X)
}

/// The way light along the mirrored way leaves the prism: Snell's law at
/// its two faces, each 15 degrees off square to that way.
fn bent() -> Vec3 {
    let (mirrored, up, _) = frame();
    let half = 15f32.to_radians();
    let inside = (half.sin() / 1.5).asin();
    let out = (1.5 * (2.0 * half - inside).sin()).asin();
    let deviation = out - half;
    mirrored * deviation.cos() + up * deviation.sin()
}

/// A closed block of `section`, points in the plane of `up` and `along`
/// about `centre`, drawn out `half` either way `across`.
fn block(centre: Vec3, along: Vec3, up: Vec3, across: Vec3, section: &[(f32, f32)], half: f32) -> Mesh {
    let mut positions = Vec::new();
    for side in [-half, half] {
        for &(v, w) in section {
            positions.push(centre + up * v + along * w + across * side);
        }
    }
    let n = section.len() as u32;
    let mut indices = Vec::new();
    // The two ends, fanned, and the sides between them.
    for i in 1..n - 1 {
        indices.extend_from_slice(&[0, i + 1, i]);
        indices.extend_from_slice(&[n, n + i, n + i + 1]);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        indices.extend_from_slice(&[i, j, n + j, i, n + j, n + i]);
    }
    Mesh::new(positions, indices)
}

/// The floor's luminance in the middle of the picture, where it mirrors the
/// glass and the light through it, averaged over two renders of `samples`
/// samples with different seeds, and how much a pixel there differs between
/// the two: its noise.
fn render(glass: Glass, lighting: Lighting, roughness: f32, samples: u32) -> (f32, f32) {
    let (mirrored, up, across) = frame();
    let floor = Mesh::new(vec![vec3(-6.0, 0.0, -6.0), vec3(6.0, 0.0, -6.0), vec3(6.0, 0.0, 6.0), vec3(-6.0, 0.0, 6.0)], vec![0, 2, 1, 0, 3, 2])
        .with_normals(vec![Vec3::Y; 4]);
    let centre = mirrored * 1.2;
    let (piece, towards) = match glass {
        // Tall enough that every straight line from the floor in view to
        // the lamp crosses it: seen past it, the lamp is found only by the
        // floor's light sample, which on so narrow a lobe is specks, as a
        // lamp in a mirror-like floor always is.
        Glass::Prism => {
            let t = 0.6 * 15f32.to_radians().tan();
            (block(centre, mirrored, up, across, &[(-0.25, 0.0), (0.35, -t), (0.35, t)], 0.4), bent())
        }
        Glass::Slab => (block(centre, mirrored, up, across, &[(-0.4, -0.025), (0.4, -0.025), (0.4, 0.025), (-0.4, 0.025)], 0.5), mirrored),
    };
    // The light 3 m past the glass, 60 cm square, facing back along the way
    // it is seen by. Past the prism the floor in the middle of the picture
    // sees it along the bent path; the straight lines to it leave the floor
    // 8 to 17 degrees above where the middle of the picture mirrors, so the
    // light the shadow rays find straight through falls outside the middle.
    let at = centre + towards * 3.0;
    let half_u = across * 0.3;
    let half_v = towards.cross(across).normalize() * 0.3;
    let half_v = if half_u.cross(half_v).dot(towards) > 0.0 { -half_v } else { half_v };
    let clear = Material { transmission_color: Spectrum::ONE, transmission_depth: 0.0, ..MaterialType::GlassSolid.defaults() };
    let mut meshes = vec![Arc::new(floor), Arc::new(piece)];
    let mut materials = vec![Material::metal(metals::SILVER, roughness), clear];
    if lighting == Lighting::Panel {
        let corners = vec![at - half_u - half_v, at + half_u - half_v, at + half_u + half_v, at - half_u + half_v];
        let facing = half_u.cross(half_v).normalize();
        meshes.push(Arc::new(Mesh::new(corners, vec![0, 1, 2, 0, 2, 3]).with_normals(vec![facing; 4])));
        materials.push(Material { emission_color: Spectrum::ONE, emission_luminance: RADIANCE, ..MaterialType::Emissive.defaults() });
    }
    let instances = (0..meshes.len() as u32).map(|i| Instance::new(InstanceId(i), MeshId(i), MaterialId(i))).collect();
    let mut scene = SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(Spectrum::ZERO)));
    if lighting == Lighting::Lamp {
        let lamp = AreaLight { position: at, half_u, half_v, radiance: Spectrum::splat(RADIANCE), disc: false, two_sided: false, spread: std::f32::consts::PI };
        scene = scene.with_lights(Arc::new(LightSet::new(vec![Light::new(LightKind::Area(lamp))])));
    }
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut boxes = Vec::new();
    for seed in 0..2 {
        let mut framebuffer = Framebuffer::new(SIZE, SIZE);
        let context = RenderContext {
            camera: tequila_render_core::camera::Camera::look_at(CAMERA, Vec3::ZERO, Vec3::Y),
            config: IntegratorConfig::product(),
            reject_fireflies: false,
            scene_seed: seed,
            ..RenderContext::default()
        };
        for _ in 0..samples {
            render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 1);
        }
        let mut values = Vec::new();
        for y in SIZE * 3 / 8..SIZE * 5 / 8 {
            for x in SIZE * 3 / 8..SIZE * 5 / 8 {
                values.push(framebuffer.pixel(x, y).mean().luminance());
            }
        }
        boxes.push(values);
    }
    let count = boxes[0].len() as f32;
    let mean = boxes.iter().flatten().sum::<f32>() / (2.0 * count);
    let noise = (boxes[0].iter().zip(&boxes[1]).map(|(a, b)| (a - b) * (a - b)).sum::<f32>() / (2.0 * count)).sqrt();
    (mean, noise)
}

/// Through the prism, the floor shows the lamp as a mirror shows the panel:
/// along the bent path. The shadow ray's straight line, which used to be
/// the only way to the lamp, showed 0.94 of the mirror's 2.41 here, as
/// specks (noise 10, against 0.14 now).
#[test]
fn a_glossy_floor_shows_a_lamp_where_a_prism_bends_its_light() {
    let (bent, _) = render(Glass::Prism, Lighting::Panel, 0.0, 64);
    let (lamp, noise) = render(Glass::Prism, Lighting::Lamp, 0.04, 64);
    println!("through the prism: the lamp in a floor of roughness 0.04 {lamp:.4} (noise {noise:.4}), a panel in a mirror {bent:.4}");
    assert!(bent > 1.5, "the mirror shows little of the panel through the prism: {bent:.4}");
    assert!((lamp / bent - 1.0).abs() < 0.05, "the floor shows the lamp through the prism at {lamp:.4}, the light along the bent path being {bent:.4}");
    assert!(noise < 0.1 * lamp, "the lamp through the prism is speckled: noise {noise:.4} on {lamp:.4}");
}

/// A lamp and a glowing panel the same show alike through the prism and
/// the slab, at every roughness, and the lamp no noisier: what the floor's
/// bounces meet and what its light samples find are shared alike for both.
/// Through the slab at 0.04 the lamp used to be specks (noise 5.4, the
/// panel's 0.14), where the floor's mirror-like lobe met its light samples;
/// through the prism at 0.45 there were specks too (0.60, the panel's 0.04),
/// and half the light.
#[test]
fn a_lamp_and_a_glowing_panel_show_alike_through_glass() {
    let cases = [
        (Glass::Prism, 0.04),
        (Glass::Prism, 0.1),
        (Glass::Prism, 0.3),
        (Glass::Prism, 0.45),
        (Glass::Prism, 0.6),
        (Glass::Slab, 0.04),
        (Glass::Slab, 0.2),
        (Glass::Slab, 0.4),
        (Glass::Slab, 0.6),
    ];
    for (glass, roughness) in cases {
        let (panel, panel_noise) = render(glass, Lighting::Panel, roughness, 64);
        let (lamp, lamp_noise) = render(glass, Lighting::Lamp, roughness, 64);
        println!("{glass:?} at roughness {roughness}: the lamp {lamp:.4} (noise {lamp_noise:.4}), the panel {panel:.4} (noise {panel_noise:.4})");
        assert!(panel > 0.1, "{glass:?} at roughness {roughness}: the floor shows little of the panel: {panel:.4}");
        assert!((lamp / panel - 1.0).abs() < 0.05, "{glass:?} at roughness {roughness}: the floor shows the lamp at {lamp:.4}, the panel at {panel:.4}");
        assert!(lamp_noise < panel_noise * 1.25 + 0.01, "{glass:?} at roughness {roughness}: the lamp is noisy, {lamp_noise:.4} against the panel's {panel_noise:.4}");
    }
}
