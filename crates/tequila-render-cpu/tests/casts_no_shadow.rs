//! A part that casts no shadow lets light by on every road the light takes:
//! a shadow ray passes it, and so does a bounce whose light a light sample
//! shares, and the photons a lamp sends out for its caustics. Each of those
//! once treated it differently. The floor's bounces stopped at it, so a
//! glossy floor mirrored a glowing panel behind it at the light sample's
//! share alone; and photons were focused by it, so a glass that cast no
//! shadow cast a caustic on top of the light it let by, and the floor under
//! it was lit twice. A lamp's links are kept the same way: its photons land
//! only on the parts it lights.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::fresnel::metals;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::light::{AreaLight, Light, LightKind, LightLinks, LightSet};
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use std::sync::Arc;

const SIZE: usize = 32;

/// A quad `half` either way about `centre`, facing `facing`.
fn quad(centre: Vec3, facing: Vec3, half: f32) -> Mesh {
    let across = if facing.y.abs() < 0.9 { Vec3::Y.cross(facing).normalize() } else { Vec3::X.cross(facing).normalize() };
    let up = facing.cross(across).normalize();
    let (u, v) = (across * half, up * half);
    let corners = vec![centre - u - v, centre + u - v, centre + u + v, centre - u + v];
    // Wound to face `facing`.
    let indices = if u.cross(v).dot(facing) > 0.0 { vec![0, 1, 2, 0, 2, 3] } else { vec![0, 2, 1, 0, 3, 2] };
    Mesh::new(corners, indices).with_normals(vec![facing; 4])
}

fn luminance_box(framebuffer: &Framebuffer, from: usize, to: usize) -> f32 {
    let mut sum = 0.0;
    for y in from..to {
        for x in from..to {
            sum += framebuffer.pixel(x, y).mean().luminance();
        }
    }
    sum / ((to - from) * (to - from)) as f32
}

fn render(scene: SceneSnapshot, config: IntegratorConfig, camera: tequila_render_core::camera::Camera, samples: u32) -> Framebuffer {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext { camera, config, reject_fireflies: false, ..RenderContext::default() };
    for _ in 0..samples {
        render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 1);
    }
    framebuffer
}

/// A glossy floor mirrors a glowing panel past a black plate that casts no
/// shadow as it mirrors it in the open: the floor's light sample passes the
/// plate to the panel, and so does its bounce, whose share of the panel MIS
/// sets against that sample's.
#[test]
fn a_glossy_floor_mirrors_a_glowing_panel_past_a_part_that_casts_no_shadow() {
    let mirrored = vec3(0.0, 1.0, -2.0).normalize();
    let mirrors = |plate: Option<bool>| {
        let floor = Mesh::new(vec![vec3(-6.0, 0.0, -6.0), vec3(6.0, 0.0, -6.0), vec3(6.0, 0.0, 6.0), vec3(-6.0, 0.0, 6.0)], vec![0, 2, 1, 0, 3, 2])
            .with_normals(vec![Vec3::Y; 4]);
        let mut meshes = vec![Arc::new(floor), Arc::new(quad(mirrored * 2.5, -mirrored, 0.3))];
        let mut materials = vec![
            Material::metal(metals::SILVER, 0.1),
            Material { emission_color: Spectrum::ONE, emission_luminance: 10.0, ..MaterialType::Emissive.defaults() },
        ];
        let mut instances = vec![
            Instance::new(InstanceId(0), MeshId(0), MaterialId(0)),
            Instance::new(InstanceId(1), MeshId(1), MaterialId(1)),
        ];
        if let Some(casts_shadows) = plate {
            meshes.push(Arc::new(quad(mirrored * 1.2, -mirrored, 0.3)));
            materials.push(Material::diffuse(Spectrum::ZERO));
            let mut part = Instance::new(InstanceId(2), MeshId(2), MaterialId(2));
            part.casts_shadows = casts_shadows;
            instances.push(part);
        }
        let scene = SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(Spectrum::ZERO)));
        let camera = tequila_render_core::camera::Camera::look_at(vec3(0.0, 1.0, 2.0), Vec3::ZERO, Vec3::Y);
        luminance_box(&render(scene, IntegratorConfig::product(), camera, 128), SIZE * 3 / 8, SIZE * 5 / 8)
    };
    let (open, shadowed, passed) = (mirrors(None), mirrors(Some(true)), mirrors(Some(false)));
    println!("the panel in the floor in the open {open:.4}, behind a plate casting shadows {shadowed:.4}, behind one casting none {passed:.4}");
    assert!(shadowed < 0.2 * open, "the plate casting shadows hid little of the panel: {shadowed:.4} against {open:.4}");
    assert!((passed / open - 1.0).abs() < 0.03, "behind a plate casting no shadow the floor mirrors the panel at {passed:.4}, in the open {open:.4}");
}

/// A solid glass ball over a white floor, a lamp above and to one side of
/// it so its caustic falls on open floor, and the floor seen from above.
fn ball_and_floor(ball: Instance, lamp: Light) -> SceneSnapshot {
    let sphere = tequila_scene::demo::uv_sphere(0.6, 64, 32);
    let floor = Mesh::new(
        vec![vec3(-4.0, -0.75, -4.0), vec3(4.0, -0.75, -4.0), vec3(4.0, -0.75, 4.0), vec3(-4.0, -0.75, 4.0)],
        vec![0, 2, 1, 0, 3, 2],
    )
    .with_normals(vec![Vec3::Y; 4]);
    let mut diffuse = MaterialType::Diffuse.defaults();
    diffuse.base_color = Spectrum::splat(0.8);
    SceneSnapshot::new(
        vec![Arc::new(sphere), Arc::new(floor)],
        vec![ball, Instance::new(InstanceId(1), MeshId(1), MaterialId(1)).with_transform(Transform::IDENTITY)],
        vec![MaterialType::GlassSolid.defaults(), diffuse],
        Arc::new(Environment::uniform(Spectrum::ZERO)),
    )
    .with_lights(Arc::new(LightSet::new(vec![lamp])))
}

fn caustic_lamp() -> Light {
    let towards = vec3(-1.0, -1.0, 0.0).normalize();
    let side = vec3(0.0, 0.0, 1.0);
    let across = side.cross(towards).normalize();
    Light::new(LightKind::Area(AreaLight {
        position: vec3(3.0, 3.0, 0.0),
        half_u: across * 0.1,
        half_v: side * 0.1,
        radiance: Spectrum::splat(400.0),
        disc: false,
        two_sided: false,
        spread: std::f32::consts::PI,
    }))
}

/// The floor's light, in 8 by 8 blocks, but for the ones the ball covers.
fn floor_blocks(framebuffer: &Framebuffer) -> Vec<f32> {
    let mut out = Vec::new();
    for by in (0..SIZE).step_by(8) {
        for bx in (0..SIZE).step_by(8) {
            // The ball is in the middle of the picture.
            if (8..24).contains(&bx) && (8..24).contains(&by) {
                continue;
            }
            let mut sum = 0.0;
            for y in by..by + 8 {
                for x in bx..bx + 8 {
                    sum += framebuffer.pixel(x, y).mean().luminance();
                }
            }
            out.push(sum / 64.0);
        }
    }
    out
}

/// A glass ball that casts no shadow - none at all, or none of this lamp's -
/// focuses none of its light: the light passes it on its way to the floor,
/// as a shadow ray does, and the floor round it is lit as in the open, with
/// caustics on as with them off. With manifold caustics it always was: the
/// walk passes what casts no shadow. The photons were focused by it, and
/// cast a caustic on top of the light it had let by.
#[test]
fn a_glass_that_casts_no_shadow_casts_no_caustic() {
    let camera = tequila_render_core::camera::Camera::look_at(vec3(0.0, 2.6, 2.4), vec3(0.0, -0.75, 0.0), Vec3::Y);
    for case in ["casts no shadow", "out of the lamp's shadow links"] {
        let mut ball = Instance::new(InstanceId(0), MeshId(0), MaterialId(0));
        let mut lamp = caustic_lamp();
        if case == "casts no shadow" {
            ball.casts_shadows = false;
        } else {
            lamp.shadow_links = LightLinks::AllBut(Arc::from(vec![0u32]));
        }
        let scene = ball_and_floor(ball, lamp);
        let off = floor_blocks(&render(scene.clone(), IntegratorConfig::product(), camera, 64));
        let on = floor_blocks(&render(scene, IntegratorConfig { caustics: true, ..IntegratorConfig::product() }, camera, 256));
        let worst = off.iter().zip(&on).map(|(a, b)| (b - a) / a.max(1e-3)).fold(0.0f32, f32::max);
        println!("{case}: caustics off {off:.3?}, on {on:.3?}: brighter by at most {worst:.3}");
        assert!(worst < 0.1, "{case}: with caustics the floor round the ball is brighter by {worst:.3}");
    }
}

/// A lamp linked to light the ball alone casts no caustic on the floor it
/// does not light, by photons as by manifold caustics.
#[test]
fn a_lamps_photons_land_only_on_what_it_lights() {
    let camera = tequila_render_core::camera::Camera::look_at(vec3(0.0, 2.6, 2.4), vec3(0.0, -0.75, 0.0), Vec3::Y);
    let mut lamp = caustic_lamp();
    lamp.links = LightLinks::Only(Arc::from(vec![0u32]));
    let scene = ball_and_floor(Instance::new(InstanceId(0), MeshId(0), MaterialId(0)), lamp);
    let on = floor_blocks(&render(scene, IntegratorConfig { caustics: true, ..IntegratorConfig::product() }, camera, 128));
    let brightest = on.iter().copied().fold(0.0f32, f32::max);
    println!("the floor the lamp does not light, with caustics: {on:.3?}");
    assert!(brightest < 0.02, "the lamp lights no floor, and its caustic does: {brightest:.3}");
}
