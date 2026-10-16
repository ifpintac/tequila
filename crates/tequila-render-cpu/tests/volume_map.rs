//! A medium whose density a Volume Map varies (`docs/02` §6).
//!
//! A cube of medium, its Density wired to a Volume Map, rendered straight on
//! against a uniformly lit world. What the map changes has to show in the
//! picture, and how the walk samples it has to be unbiased:
//!
//! - the fog sphere (`tequila-texture/tests/data/sphere.vdb`, written by
//!   OpenVDB) makes an absorbing cube dark in the middle and clear at its
//!   corners, where the ball does not reach;
//! - a grid of 0.5 everywhere renders as the same medium at Density 0.5;
//! - a medium that only scatters, however patchy, sends back all the light a
//!   white world sends in.

use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec3, Transform, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket, TextureId};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_materials::openpbr::Material;
use tequila_render_core::density::DensityField;
use tequila_render_core::environment::Environment;
use tequila_render_core::framebuffer::Framebuffer;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::scheduler::ProgressiveSchedule;
use tequila_render_core::shade::TextureTable;
use tequila_render_cpu::{render_pass, Device, EmbreeIntersector, RenderContext};
use tequila_texture::volume::VolumeGrid;
use std::sync::Arc;

const SIZE: usize = 48;

/// A closed cube from -1 to 1, flat-shaded, faces wound outward.
fn cube() -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    for axis in 0..3 {
        for sign in [-1.0f32, 1.0] {
            let n = [Vec3::X, Vec3::Y, Vec3::Z][axis] * sign;
            let (u, v) = match axis {
                0 => (Vec3::Y, Vec3::Z),
                1 => (Vec3::Z, Vec3::X),
                _ => (Vec3::X, Vec3::Y),
            };
            let (u, v) = if sign > 0.0 { (u, v) } else { (v, u) };
            let base = positions.len() as u32;
            for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                positions.push(n + u * a + v * b);
                normals.push(n);
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    Mesh::new(positions, indices).with_normals(normals)
}

fn sphere_grid() -> Arc<VolumeGrid> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tequila-texture/tests/data/sphere.vdb");
    Arc::new(tequila_texture::volume::read_vdb(&std::fs::read(path).expect("fixture")).expect("reads"))
}

fn constant_grid(value: f32) -> Arc<VolumeGrid> {
    Arc::new(VolumeGrid::new("density".into(), vec![value; 8 * 8 * 8], [8, 8, 8], Vec3::splat(-0.875), Vec3::splat(0.25)))
}

/// Render the cube of `medium`, its Density driven by `grid` when there is
/// one, in a white world.
fn render(medium: Material, grid: Option<Arc<VolumeGrid>>, samples: u32) -> Framebuffer {
    let mut graph = MaterialGraph::from_material(medium.clone());
    if grid.is_some() {
        let output = graph.output();
        let map = graph.add_node(Node::new(NodeKind::VolumeMap { volume: TextureId(1) }));
        graph.connect(map, output, Socket::Param(Param::VolumeDensity));
    }
    let program = Arc::new(graph.compile().expect("compiles"));
    let mut scene = SceneSnapshot::new(
        vec![Arc::new(cube())],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)],
        vec![medium],
        Arc::new(Environment::uniform(Spectrum::ONE)),
    )
    .with_programs(vec![Some(program)], Arc::new(TextureTable::new()));
    if let Some(grid) = grid {
        let field = DensityField::fitted(grid, (Vec3::splat(-1.0), Vec3::splat(1.0)), Transform::IDENTITY);
        scene = scene.with_densities(vec![field.map(Arc::new)]);
    }
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut framebuffer = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext {
        camera: camera(),
        reject_fireflies: false,
        ..RenderContext::default()
    };
    render_pass(&intersector, &mut framebuffer, &context, &ProgressiveSchedule::default(), samples);
    framebuffer
}

fn camera() -> tequila_render_core::camera::Camera {
    tequila_render_core::camera::Camera::look_at(vec3(0.0, 0.0, 6.0), Vec3::ZERO, Vec3::Y)
}

/// Mean luminance over a few pixels around where `world` lands.
fn around(image: &Framebuffer, world: Vec3) -> f32 {
    let film = camera().project(world, 1.0).expect("in view");
    let (cx, cy) = ((film.x * SIZE as f32) as i64, (film.y * SIZE as f32) as i64);
    let mut total = 0.0;
    let mut n = 0.0;
    for y in cy - 1..=cy + 1 {
        for x in cx - 1..=cx + 1 {
            if (0..SIZE as i64).contains(&x) && (0..SIZE as i64).contains(&y) {
                total += image.pixel(x as usize, y as usize).mean().luminance();
                n += 1.0;
            }
        }
    }
    total / n
}

fn absorbing() -> Material {
    Material {
        transmission_color: Spectrum::splat(0.05),
        transmission_depth: 0.3,
        transmission_scatter: Spectrum::ZERO,
        ..MaterialType::ScatteringMedium.defaults()
    }
}

#[test]
fn the_map_decides_where_the_medium_is() {
    let image = render(absorbing(), Some(sphere_grid()), 64);
    // Straight through the middle: two metres of the ball.
    let centre = around(&image, vec3(0.0, 0.0, 1.0));
    // Through a corner of the cube's face, which the ball does not reach.
    let corner = around(&image, vec3(0.85, 0.85, 1.0));
    assert!(centre < 0.1, "the middle of the ball lets {centre} through");
    assert!(corner > 0.85, "the corner, outside the ball, lets only {corner} through");
}

/// Thin enough that density 1 and 0.5 differ plainly: straight through, the
/// cube's two metres pass 0.09 of the light at one and 0.3 at the other.
fn thin() -> Material {
    Material { transmission_color: Spectrum::splat(0.3), transmission_depth: 1.0, ..absorbing() }
}

#[test]
fn a_constant_map_is_the_same_as_the_same_density() {
    let mapped = render(thin(), Some(constant_grid(0.5)), 256);
    let uniform = render(Material { volume_density: 0.5, ..thin() }, None, 256);
    for point in [vec3(0.0, 0.0, 1.0), vec3(0.5, -0.3, 1.0), vec3(-0.7, 0.6, 1.0)] {
        let (a, b) = (around(&mapped, point), around(&uniform, point));
        assert!((a - b).abs() < 0.03 + 0.05 * b, "at {point:?}: mapped {a} against uniform {b}");
    }
}

#[test]
fn a_patchy_medium_that_only_scatters_loses_nothing() {
    let scattering = Material {
        transmission_color: Spectrum::ONE,
        transmission_depth: 0.2,
        transmission_scatter: Spectrum::splat(3.0),
        ..MaterialType::ScatteringMedium.defaults()
    };
    // 1024 samples: at 256 a pixel strays 6% on noise alone (measured),
    // which is no test of a bias.
    let image = render(scattering, Some(sphere_grid()), 1024);
    for point in [vec3(0.0, 0.0, 1.0), vec3(0.4, 0.4, 1.0), vec3(0.85, -0.85, 1.0)] {
        let value = around(&image, point);
        assert!((value - 1.0).abs() < 0.06, "a white world comes back as {value} at {point:?}");
    }
}

/// Dark, but for one small bright light off to the side: the sun, as far as
/// a medium is concerned.
fn sun_only() -> Environment {
    let (width, height) = (512, 256);
    let towards = vec3(1.0, 0.4, 0.3).normalize();
    let mut data = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let d = tequila_render_core::pins::equirect_direction((x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32);
            let lit = if d.dot(towards) > 0.9995 { 2000.0 } else { 0.0 };
            data.extend_from_slice(&[lit, lit, lit]);
        }
    }
    let image = tequila_texture::image::Image {
        width,
        height,
        channels: 3,
        data,
        color_space: tequila_texture::image::ColorSpace::LinearSrgb,
    };
    Environment::new(Arc::new(image), 1.0)
}

/// Smoke in sunlight is lit by the sun at every scattering event, not only
/// when a scattered path happens to leave straight towards a disc a quarter
/// of a degree across. Without sampling the light from inside the medium the
/// sun's light arrives as rare bright specks — most pixels get none of it at
/// a working sample count, and the median pixel sits far below the mean. With
/// it, the light is spread as it should be.
#[test]
fn a_medium_is_lit_by_a_small_light_at_every_scatter() {
    let smoke = Material {
        transmission_color: Spectrum::splat(0.98),
        transmission_depth: 0.3,
        transmission_scatter: Spectrum::splat(2.0),
        ..MaterialType::ScatteringMedium.defaults()
    };
    let scene = SceneSnapshot::new(
        vec![Arc::new(cube())],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(Transform::IDENTITY)],
        vec![smoke],
        Arc::new(sun_only()),
    );
    let device = Device::shared().expect("an Embree device");
    let intersector = EmbreeIntersector::build(&device, scene).expect("builds");
    let mut image = Framebuffer::new(SIZE, SIZE);
    let context = RenderContext { camera: camera(), reject_fireflies: false, ..RenderContext::default() };
    render_pass(&intersector, &mut image, &context, &ProgressiveSchedule::default(), 32);
    let mut values: Vec<f32> = Vec::new();
    for y in SIZE / 3..2 * SIZE / 3 {
        for x in SIZE / 3..2 * SIZE / 3 {
            values.push(image.pixel(x, y).mean().luminance());
        }
    }
    values.sort_by(f32::total_cmp);
    let median = values[values.len() / 2];
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    assert!(mean > 0.0, "the smoke is not lit at all");
    assert!(
        median > 0.5 * mean,
        "the median pixel is {median} against a mean of {mean}: the sun reaches the smoke as specks"
    );
}

/// The Material tab's sphere shows a Volume Map's shape, fitted to the
/// sphere as the render fits it to a part. It used to show the medium
/// evenly at its described density, so a VDB loaded onto Density changed
/// nothing in the panel and it looked as if the load had failed.
#[test]
fn the_preview_shows_the_volume() {
    // Dense on one side of the grid, empty on the other.
    let n = 16;
    let data: Vec<f32> = (0..n * n * n).map(|i| if i % n < n / 2 { 1.0 } else { 0.0 }).collect();
    let grid = VolumeGrid::new("density".into(), data, [n, n, n], Vec3::splat(-0.46875), Vec3::splat(0.0625));
    let mut table = TextureTable::new();
    table.insert_volume(TextureId(1), Arc::new(grid));
    // Sized for the preview's five-centimetre ball rather than the metre cube
    // above: dark within two centimetres.
    let mut graph = MaterialGraph::from_material(Material { transmission_depth: 0.02, ..absorbing() });
    let output = graph.output();
    let map = graph.add_node(Node::new(NodeKind::VolumeMap { volume: TextureId(1) }));
    graph.connect(map, output, Socket::Param(Param::VolumeDensity));
    let device = Device::shared().expect("an Embree device");
    let thumbnail = tequila_render_cpu::render_thumbnail_textured(
        &device,
        &graph,
        Arc::new(table),
        Arc::new(tequila_scene::demo::preview_environment()),
        tequila_render_cpu::PreviewSettings { size: 48, samples: 64, ..Default::default() },
    )
    .expect("renders");
    let (w, h) = (thumbnail.width, thumbnail.height);
    let side = |from: usize, to: usize| -> f32 {
        let mut total = 0.0;
        for y in h * 2 / 5..h * 3 / 5 {
            for x in from..to {
                total += thumbnail.pixel(x, y).luminance();
            }
        }
        total
    };
    let (left, right) = (side(w * 3 / 10, w * 9 / 20), side(w * 11 / 20, w * 7 / 10));
    let lopsided = left.max(right) / left.min(right).max(1e-6);
    assert!(lopsided > 1.5, "one side of the sphere reads {left}, the other {right}: the volume does not show");
}
