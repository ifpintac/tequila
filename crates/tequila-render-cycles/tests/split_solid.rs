//! A clear part modelled in pieces is one solid in Cycles too.
//!
//! The palma P8's lime shell comes in as two parts per half, split by
//! material slot: the outside with the rim where the halves meet, and the
//! inside. Neither is closed; together they are. Cycles keys the volumes a
//! ray is inside by object and shader, so a ray that went in through the
//! outside and out through the inside was still inside the outside's
//! volume: the air in the shell took its tint, and the board showed greener
//! and darker through it than Tequila's renderer shows it. The translation
//! makes such a solid one object with one shader.
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
use tequila_render_cycles::{translate, Session};

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

/// The tray of `tequila-render-cpu`'s `split_solid` test, the palma's shell
/// halves' shape: a box two across, closed towards the camera and open at
/// the back, walls a tenth thick, as its outside with the rim and its
/// inside, two open surfaces that close each other.
fn tray() -> (Vec<[Vec3; 4]>, Vec<[Vec3; 4]>) {
    let (outer, inner, front, inside_front, back) = (1.0f32, 0.9f32, 0.3f32, 0.2f32, -0.3f32);
    let p = |x: f32, y: f32, z: f32| vec3(x, y, z);
    let side = |r: f32| [(-r, -r), (r, -r), (r, r), (-r, r)];
    let (o, i) = (side(outer), side(inner));
    let mut outside = vec![[p(-outer, -outer, front), p(outer, -outer, front), p(outer, outer, front), p(-outer, outer, front)]];
    let mut inside = vec![[p(-inner, -inner, inside_front), p(-inner, inner, inside_front), p(inner, inner, inside_front), p(inner, -inner, inside_front)]];
    for k in 0..4 {
        let (a, b) = (o[k], o[(k + 1) % 4]);
        let (c, d) = (i[k], i[(k + 1) % 4]);
        outside.push([p(a.0, a.1, back), p(b.0, b.1, back), p(b.0, b.1, front), p(a.0, a.1, front)]);
        outside.push([p(a.0, a.1, back), p(c.0, c.1, back), p(d.0, d.1, back), p(b.0, b.1, back)]);
        inside.push([p(c.0, c.1, back), p(c.0, c.1, inside_front), p(d.0, d.1, inside_front), p(d.0, d.1, back)]);
    }
    (outside, inside)
}

/// The palma's shell material at this scale, absorbing only, so the picture
/// settles in few samples; the outside a little rougher than the inside, as
/// the palma's are, so the two parts are two materials.
fn lime(roughness: f32) -> Material {
    Material {
        transmission_weight: 1.0,
        transmission_color: Spectrum { r: 0.76, g: 0.96, b: 0.04 },
        transmission_depth: 0.5,
        transmission_scatter: Spectrum::ZERO,
        specular_ior: 1.585,
        specular_roughness: roughness,
        medium_priority: 1.0,
        ..MaterialType::Plastic.defaults()
    }
}

fn snapshot(parts: Vec<(Mesh, Material)>) -> SceneSnapshot {
    let mut meshes = Vec::new();
    let mut instances = Vec::new();
    let mut materials = Vec::new();
    for (i, (mesh, material)) in parts.into_iter().enumerate() {
        meshes.push(Arc::new(mesh));
        instances.push(Instance::new(InstanceId(i as u32), MeshId(i as u32), MaterialId(i as u32)).with_transform(Transform::IDENTITY));
        materials.push(material);
    }
    SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(Spectrum::ONE)))
}

/// The mean colour over the middle of a Cycles picture from 3.5 in front.
fn render(scene: &SceneSnapshot) -> [f64; 3] {
    let lighting = tequila_render_core::settings::IntegratorConfig {
        max_bounces: 32,
        max_transmission_bounces: 32,
        ..tequila_render_core::settings::IntegratorConfig::product()
    };
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
    let translation = translate::build(&mut session, scene, &view).expect("the scene translates");
    for warning in &translation.warnings {
        eprintln!("translation: {warning}");
    }
    translate::configure(&mut session, &translate::Quality::with_lighting(false, false, None, &lighting)).expect("the settings take");
    let rgba = session.render(SIZE, SIZE, 64, |_, _| {}).expect("Cycles renders");
    let mut sum = [0.0f64; 3];
    let mut count = 0.0;
    for y in SIZE * 3 / 8..SIZE * 5 / 8 {
        for x in SIZE * 3 / 8..SIZE * 5 / 8 {
            let px = &rgba[(y * SIZE + x) * 4..];
            for c in 0..3 {
                sum[c] += px[c] as f64;
            }
            count += 1.0;
        }
    }
    sum.map(|s| s / count)
}

/// Seen through the tray's closed face and out through its open back, in a
/// white world: split into its outside and its inside, of two materials,
/// the tray passes what it passes whole.
#[test]
fn a_clear_part_split_by_material_is_as_clear_as_the_whole_in_cycles() {
    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    let (outside, inside) = tray();
    let whole: Vec<[Vec3; 4]> = outside.iter().chain(&inside).copied().collect();
    let split = snapshot(vec![(mesh(&outside), lime(0.035)), (mesh(&inside), lime(0.02))]);
    assert_eq!(split.solid(InstanceId(1)), InstanceId(0), "the tray's two parts are one solid");
    let one = render(&snapshot(vec![(mesh(&whole), lime(0.02))]));
    let two = render(&split);
    println!("whole {one:?}\nsplit {two:?}");
    assert!(one[1] > 0.5 && one[2] < one[1], "the whole tray is not a clear lime in Cycles: {one:?}");
    for (c, channel) in ["red", "green", "blue"].iter().enumerate() {
        assert!(
            (one[c] - two[c]).abs() < 0.05 * one[c].max(0.1),
            "split into its outside and its inside the tray passes {:.4} {channel} in Cycles, where whole it passes {:.4}",
            two[c],
            one[c]
        );
    }
}

/// The palma's shell is two such halves, front and back, their rims
/// meeting: a ray goes in through the front half's outside, out through its
/// inside, across the air between, in through the back half's inside and
/// out through its outside. Each half split in two, the box passes what it
/// passes with each half whole.
#[test]
fn two_split_halves_are_as_clear_as_two_whole_ones_in_cycles() {
    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    let (outside, inside) = tray();
    // The back half: the front one mirrored through its open back, at
    // -0.3, wound again so its faces still face out of the solid.
    let mirror = |quads: &[[Vec3; 4]]| -> Vec<[Vec3; 4]> {
        quads.iter().map(|q| { let m = q.map(|v| vec3(v.x, v.y, -0.6 - v.z)); [m[3], m[2], m[1], m[0]] }).collect()
    };
    let (back_outside, back_inside) = (mirror(&outside), mirror(&inside));
    let whole = |a: &[[Vec3; 4]], b: &[[Vec3; 4]]| -> Vec<[Vec3; 4]> { a.iter().chain(b).copied().collect() };
    let one = render(&snapshot(vec![(mesh(&whole(&outside, &inside)), lime(0.02)), (mesh(&whole(&back_outside, &back_inside)), lime(0.02))]));
    let split = snapshot(vec![
        (mesh(&outside), lime(0.035)),
        (mesh(&inside), lime(0.02)),
        (mesh(&back_outside), lime(0.035)),
        (mesh(&back_inside), lime(0.02)),
    ]);
    // The halves' rims meet edge to edge, so all four parts are one solid
    // to Tequila's renderer, as the palma's are.
    assert_eq!(split.solid(InstanceId(1)), InstanceId(0));
    let two = render(&split);
    println!("whole halves {one:?}
split halves {two:?}");
    assert!(one[1] > 0.4, "the whole box is not a clear lime in Cycles: {one:?}");
    for (c, channel) in ["red", "green", "blue"].iter().enumerate() {
        assert!(
            (one[c] - two[c]).abs() < 0.05 * one[c].max(0.1),
            "with its halves split the box passes {:.4} {channel} in Cycles, where with them whole it passes {:.4}",
            two[c],
            one[c]
        );
    }
}
