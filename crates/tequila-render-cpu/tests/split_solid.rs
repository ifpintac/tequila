//! A clear part modelled in pieces is still one solid, and what lies behind
//! the camera's view decides where the camera is.
//!
//! The palma P8's lime shell comes in as two parts per half, split by
//! material slot: the outside with the rim where the halves meet, and the
//! inside. Neither is closed; together they are. The medium a path is in
//! was told apart by part, so a path that went in through the outside and
//! on through the inside was in both, and was still in the outside's
//! volume when it left through the far wall: the air in the shell and the
//! room beyond it absorbed, and the shell rendered an opaque dark green
//! where Cycles showed the board and the screen through it.
//!
//! And a frosted label lies flat on the shell's back, in the plane of the
//! shell's own surface. Counting crossings ahead of the camera to find
//! whether it starts inside the shell stepped past the label and over the
//! shell's surface with it, and the screen, in front of all that, showed
//! through a dark patch of the shell's volume.

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

const SIZE: usize = 48;

/// Quads, each wound so its normal faces the way given by the order of its
/// corners, as triangles.
fn mesh(quads: &[[Vec3; 4]]) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for quad in quads {
        let first = positions.len() as u32;
        positions.extend_from_slice(quad);
        indices.extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
    }
    Mesh::new(positions, indices)
}

/// A tray, the palma's shell halves' shape: a box two across whose face
/// towards the camera (+Z, at 0.3) is closed and whose back (-Z, at -0.3)
/// is open, with walls a tenth thick. As modelled, its outside and the rim
/// round the opening, then its inside: two open surfaces that close each
/// other along the inner edge of the rim. Every corner is worked out the
/// same way in both, so the edges they share meet exactly.
fn tray() -> (Vec<[Vec3; 4]>, Vec<[Vec3; 4]>) {
    let (outer, inner, front, inside_front, back) = (1.0f32, 0.9f32, 0.3f32, 0.2f32, -0.3f32);
    let p = |x: f32, y: f32, z: f32| vec3(x, y, z);
    // The four sides as (x, y) corner pairs going round anticlockwise seen
    // from the front.
    let side = |r: f32| [(-r, -r), (r, -r), (r, r), (-r, r)];
    let (o, i) = (side(outer), side(inner));
    let mut outside = vec![[p(-outer, -outer, front), p(outer, -outer, front), p(outer, outer, front), p(-outer, outer, front)]];
    let mut inside = vec![[p(-inner, -inner, inside_front), p(-inner, inner, inside_front), p(inner, inner, inside_front), p(inner, -inner, inside_front)]];
    for k in 0..4 {
        let (a, b) = (o[k], o[(k + 1) % 4]);
        let (c, d) = (i[k], i[(k + 1) % 4]);
        // Outer wall, facing out.
        outside.push([p(a.0, a.1, back), p(b.0, b.1, back), p(b.0, b.1, front), p(a.0, a.1, front)]);
        // The rim between the walls, facing back.
        outside.push([p(a.0, a.1, back), p(c.0, c.1, back), p(d.0, d.1, back), p(b.0, b.1, back)]);
        // Inner wall, facing into the tray.
        inside.push([p(c.0, c.1, back), p(c.0, c.1, inside_front), p(d.0, d.1, inside_front), p(d.0, d.1, back)]);
    }
    (outside, inside)
}

/// The palma's shell material at this scale: a lime plastic all
/// transmission, its volume half a unit deep and faintly scattering.
fn lime() -> Material {
    Material {
        transmission_weight: 1.0,
        transmission_color: Spectrum { r: 0.76, g: 0.96, b: 0.04 },
        transmission_depth: 0.5,
        transmission_scatter: Spectrum { r: 0.07, g: 0.09, b: 0.02 },
        specular_ior: 1.585,
        medium_priority: 1.0,
        ..MaterialType::Plastic.defaults()
    }
}

fn snapshot(parts: Vec<(Mesh, Material)>, environment: Spectrum) -> SceneSnapshot {
    let mut meshes = Vec::new();
    let mut instances = Vec::new();
    let mut materials = Vec::new();
    for (i, (mesh, material)) in parts.into_iter().enumerate() {
        meshes.push(Arc::new(mesh));
        instances.push(Instance::new(InstanceId(i as u32), MeshId(i as u32), MaterialId(i as u32)).with_transform(Transform::IDENTITY));
        materials.push(material);
    }
    SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(environment)))
}

/// The mean colour over the middle of a picture from 3.5 in front.
fn render(scene: SceneSnapshot) -> Spectrum {
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera: tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 3.5), Vec3::ZERO, Vec3::Y),
        config: IntegratorConfig { max_bounces: 32, max_transmission_bounces: 32, ..IntegratorConfig::product() },
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), 128);
    let mut sum = Spectrum::ZERO;
    let mut count = 0.0;
    for y in SIZE * 3 / 8..SIZE * 5 / 8 {
        for x in SIZE * 3 / 8..SIZE * 5 / 8 {
            sum += framebuffer.pixel(x, y).mean();
            count += 1.0;
        }
    }
    sum * (1.0 / count)
}

#[test]
fn the_pieces_of_a_split_part_are_one_solid() {
    let (outside, inside) = tray();
    let split = snapshot(vec![(mesh(&outside), lime()), (mesh(&inside), lime())], Spectrum::ONE);
    assert_eq!(split.solid(InstanceId(1)), InstanceId(0), "the inside of the tray is a piece of the same solid as its outside");
    assert_eq!(split.solid(InstanceId(0)), InstanceId(0));

    // Closed parts are solids of their own, touching or not.
    let whole: Vec<[Vec3; 4]> = outside.iter().chain(&inside).copied().collect();
    let shifted: Vec<[Vec3; 4]> = whole.iter().map(|q| q.map(|v| v + vec3(2.0, 0.0, 0.0))).collect();
    let two = snapshot(vec![(mesh(&whole), lime()), (mesh(&shifted), lime())], Spectrum::ONE);
    assert_eq!(two.solid(InstanceId(1)), InstanceId(1), "two closed trays side by side are two solids");

    // Pieces that bound no medium are left as they are: a thin-walled shell
    // has no inside to be in.
    let thin = Material { geometry_thin_walled: true, ..lime() };
    let panes = snapshot(vec![(mesh(&outside), thin.clone()), (mesh(&inside), thin)], Spectrum::ONE);
    assert_eq!(panes.solid(InstanceId(1)), InstanceId(1));
}

/// Seen through the tray's closed face and out through its open back, in a
/// white world: the split tray as the whole one, which only its tenth-thick
/// face tints.
#[test]
fn a_clear_part_split_by_material_is_as_clear_as_the_whole() {
    let (outside, inside) = tray();
    let whole: Vec<[Vec3; 4]> = outside.iter().chain(&inside).copied().collect();
    let one = render(snapshot(vec![(mesh(&whole), lime())], Spectrum::ONE));
    let split = render(snapshot(vec![(mesh(&outside), lime()), (mesh(&inside), lime())], Spectrum::ONE));
    println!("whole {one:?}\nsplit {split:?}");
    // The control: through a tenth of the lime, most of the white comes
    // back, and the blue least.
    assert!(one.g > 0.7 && one.b < one.g, "the whole tray is not a clear lime: {one:?}");
    for (a, b, channel) in [(one.r, split.r, "red"), (one.g, split.g, "green"), (one.b, split.b, "blue")] {
        assert!(
            (a - b).abs() < 0.04 * a.max(0.1),
            "the tray split into its outside and its inside passes {b:.4} {channel}, where whole it passes {a:.4}"
        );
    }
}

/// A screen with a block of dark glass behind it, and a label on the
/// block's back face, a hair's breadth inside it, as close as stepping past
/// the label steps: the screen glows the same with the label as without.
#[test]
fn a_label_on_the_glass_behind_a_screen_leaves_the_screen_alone() {
    let square = |z: f32, half: f32| mesh(&[[vec3(-half, -half, z), vec3(half, -half, z), vec3(half, half, z), vec3(-half, half, z)]]);
    let block = {
        let (h, front, back) = (1.0f32, -0.2f32, -1.0f32);
        let p = |x: f32, y: f32, z: f32| vec3(x * h, y * h, z);
        mesh(&[
            [p(-1.0, -1.0, front), p(1.0, -1.0, front), p(1.0, 1.0, front), p(-1.0, 1.0, front)],
            [p(-1.0, -1.0, back), p(-1.0, 1.0, back), p(1.0, 1.0, back), p(1.0, -1.0, back)],
            [p(-1.0, -1.0, back), p(1.0, -1.0, back), p(1.0, -1.0, front), p(-1.0, -1.0, front)],
            [p(1.0, -1.0, back), p(1.0, 1.0, back), p(1.0, 1.0, front), p(1.0, -1.0, front)],
            [p(1.0, 1.0, back), p(-1.0, 1.0, back), p(-1.0, 1.0, front), p(1.0, 1.0, front)],
            [p(-1.0, 1.0, back), p(-1.0, -1.0, back), p(-1.0, -1.0, front), p(-1.0, 1.0, front)],
        ])
    };
    let screen = Material {
        base_weight: 0.0,
        specular_weight: 0.0,
        emission_luminance: 1.0,
        emission_color: Spectrum::ONE,
        ..MaterialType::Plastic.defaults()
    };
    let dark_glass = Material { transmission_color: Spectrum::splat(0.5), transmission_depth: 0.1, transmission_scatter: Spectrum::ZERO, ..lime() };
    let label = Material { transmission_weight: 0.6, geometry_thin_walled: true, ..MaterialType::Plastic.defaults() };
    // The block's back face is at -1, and a step past the label at -1 is a
    // few millionths: the label one float in front of the face.
    let on_the_back = -1.0f32 + f32::EPSILON;
    let shot = |with_label: bool| {
        let mut parts = vec![(square(1.0, 0.5), screen.clone()), (block.clone(), dark_glass.clone())];
        if with_label {
            parts.push((square(on_the_back, 0.6), label.clone()));
        }
        render(snapshot(parts, Spectrum::ZERO))
    };
    let (bare, labelled) = (shot(false), shot(true));
    println!("no label {bare:?}\nlabel {labelled:?}");
    assert!(bare.g > 0.1, "the screen does not glow: {bare:?}");
    assert!(
        (labelled.g - bare.g).abs() < 0.02 * bare.g,
        "a label on the back of the glass behind the screen took the screen from {:.4} to {:.4}",
        bare.g,
        labelled.g
    );
}
