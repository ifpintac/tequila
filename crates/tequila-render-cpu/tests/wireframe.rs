//! A wireframe render tells the edges the camera sees from those behind the
//! model, by asking the scene, and draws them where the camera puts them.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_render_core::camera::{Camera, Projection};
use tequila_render_core::environment::Environment;
use tequila_render_core::material::Material;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_cpu::wireframe::draw;
use tequila_render_cpu::{Device, EmbreeIntersector};
use tequila_scene::render::WireEdges;
use std::sync::Arc;

/// A unit cube split at its creases, as a CAD part's mesh is.
fn cube() -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let faces: [[Vec3; 4]; 6] = [
        [vec3(0.0, 0.0, 1.0), vec3(1.0, 0.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(0.0, 1.0, 1.0)],
        [vec3(1.0, 0.0, 0.0), vec3(0.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0)],
        [vec3(0.0, 0.0, 0.0), vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 1.0), vec3(0.0, 1.0, 0.0)],
        [vec3(1.0, 0.0, 1.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 1.0, 0.0), vec3(1.0, 1.0, 1.0)],
        [vec3(0.0, 1.0, 1.0), vec3(1.0, 1.0, 1.0), vec3(1.0, 1.0, 0.0), vec3(0.0, 1.0, 0.0)],
        [vec3(0.0, 0.0, 0.0), vec3(1.0, 0.0, 0.0), vec3(1.0, 0.0, 1.0), vec3(0.0, 0.0, 1.0)],
    ];
    for quad in faces {
        let base = positions.len() as u32;
        positions.extend_from_slice(&quad);
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(positions, indices)
}

fn scene() -> SceneSnapshot {
    SceneSnapshot::new(
        vec![Arc::new(cube())],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![Material::diffuse(Spectrum::splat(0.5))],
        Arc::new(Environment::uniform(Spectrum::ONE)),
    )
}

/// The pixel a world point lands on.
fn pixel(camera: &Camera, width: usize, height: usize, at: Vec3) -> usize {
    let film = camera.project(at, width as f32 / height as f32).expect("in front of the camera");
    let (x, y) = ((film.x * width as f32) as usize, (film.y * height as f32) as usize);
    y * width + x
}

/// From a corner, off any line through two of the cube's edges at once:
/// from (3, 2.5, 4) one hidden edge's middle is exactly behind a visible one.
fn check(camera: Camera) {
    let (width, height) = (320, 240);
    let scene = scene();
    let lines = tequila_scene::wireframe::lines(&scene, &camera, WireEdges::Model);
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let coverage = draw(&intersector, &camera, width, height, &lines, 2.0, true).expect("draws");

    // From the (+, +, +) side, the three edges meeting at the origin are
    // behind the cube; the one between the top and the right-hand face is
    // in front.
    for hidden in [vec3(0.5, 0.0, 0.0), vec3(0.0, 0.5, 0.0), vec3(0.0, 0.0, 0.5)] {
        let at = pixel(&camera, width, height, hidden);
        assert!(coverage.behind[at] > 0.9, "the edge through {hidden:?} is drawn behind: {}", coverage.behind[at]);
        // Not drawn in front, which would cover the pixel fully; a line in
        // front passing a pixel away may brush it.
        assert!(coverage.front[at] < 0.5, "and not in front: {}", coverage.front[at]);
    }
    let seen = pixel(&camera, width, height, vec3(1.0, 1.0, 0.5));
    assert!(coverage.front[seen] > 0.9, "the edge in front is drawn in front: {}", coverage.front[seen]);
    // The middle of a face is not an edge at all.
    let face = pixel(&camera, width, height, vec3(0.5, 1.0, 0.5));
    assert!(coverage.front[face] < 0.01 && coverage.behind[face] < 0.01);
    // Without the hidden lines asked for, they are not drawn.
    let front_only = draw(&intersector, &camera, width, height, &lines, 2.0, false).expect("draws");
    assert!(front_only.behind.iter().all(|c| *c == 0.0));
    assert_eq!(front_only.front, coverage.front);
}

#[test]
fn hidden_edges_are_told_from_the_ones_in_front_in_perspective() {
    check(Camera::look_at(vec3(3.1, 2.3, 4.2), Vec3::splat(0.5), Vec3::Y));
}

#[test]
fn and_through_an_orthographic_camera() {
    let mut camera = Camera::look_at(vec3(3.1, 2.3, 4.2), Vec3::splat(0.5), Vec3::Y);
    camera.projection = Projection::Orthographic { scale: 2.5 };
    check(camera);
}

#[test]
fn a_shifted_lens_moves_the_drawing_with_it() {
    let mut camera = Camera::look_at(vec3(3.1, 2.3, 4.2), Vec3::splat(0.5), Vec3::Y);
    camera.projection = Projection::Shift { focal_length_mm: 50.0, shift_x: 0.1, shift_y: 0.05 };
    check(camera);
}

#[test]
fn a_360_camera_says_it_has_no_wireframe() {
    let mut camera = Camera::look_at(vec3(3.1, 2.3, 4.2), Vec3::splat(0.5), Vec3::Y);
    camera.projection = Projection::Equirectangular;
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene()).expect("builds");
    assert!(draw(&intersector, &camera, 64, 64, &[], 1.0, true).is_err());
}

