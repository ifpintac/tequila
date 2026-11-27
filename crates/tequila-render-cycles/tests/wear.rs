//! The wear patterns (`tequila_texture::wear`) come out of Cycles where
//! Tequila puts them: not alike on average, but the same scratches, specks,
//! prints and scuffs on the same pixels.
//!
//! A flat tile seen from above, unlit, glowing as bright as the pattern is
//! (a Flat material, its colour the pattern through a black-to-white ramp),
//! rendered by Cycles and compared pixel by pixel with the pattern evaluated
//! here, as Tequila's renderer evaluates it, at the same points of the tile
//! and with the same footprint - so the translation is checked, node for
//! node, without any of the rest of either renderer in the way. Skipped
//! where Cycles is not installed.

use std::sync::Arc;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec2, vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeKind, Socket};
use tequila_materials::material_type::{MaterialType, Param};
use tequila_render_core::camera::Camera;
use tequila_render_core::environment::Environment;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_core::settings::IntegratorConfig;
use tequila_render_cycles::{translate, Session};
use tequila_texture::gradient::Gradient;
use tequila_texture::procedural::Procedural;
use tequila_texture::wear::{Dust, ScratchDirection, Scratches, Scuffs, Fingerprints, WearPoint};

const TILE: f32 = 0.2;
const SIZE: usize = 128;
const HEIGHT: f32 = 0.2;
const SEED: u32 = 5;

/// A flat tile in the XZ plane, facing up.
fn tile() -> Mesh {
    let h = TILE * 0.5;
    let positions = vec![vec3(-h, 0.0, -h), vec3(h, 0.0, -h), vec3(h, 0.0, h), vec3(-h, 0.0, h)];
    Mesh::new(positions, vec![0, 2, 1, 0, 3, 2])
        .with_normals(vec![Vec3::Y; 4])
        .with_uvs(vec![vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0)])
}

fn camera() -> Camera {
    Camera::look_at(vec3(0.0, HEIGHT, 0.0), Vec3::ZERO, vec3(0.0, 0.0, -1.0))
}

/// The tile glowing as bright as `procedural` is.
fn scene(procedural: Procedural) -> SceneSnapshot {
    let mut graph = MaterialGraph::from_material(MaterialType::Flat.defaults());
    let pattern = graph.add_node(Node::new(NodeKind::Procedural { procedural: Box::new(procedural), seed: SEED }));
    let ramp = graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::new(Gradient::two(Spectrum::ZERO, Spectrum::ONE)) }));
    graph.connect(pattern, ramp, Socket::Input(0));
    let output = graph.output();
    graph.connect(ramp, output, Socket::Param(Param::EmissionColor));
    let program = graph.compile().expect("compiles");
    SceneSnapshot::new(
        vec![Arc::new(tile())],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0))],
        vec![graph.material().clone()],
        Arc::new(Environment::uniform(Spectrum::ZERO)),
    )
    .with_programs(vec![Some(Arc::new(program))], Arc::new(tequila_render_core::shade::TextureTable::new()))
}

/// Cycles' picture of the tile, a brightness a pixel.
fn cycles(procedural: Procedural) -> Vec<f64> {
    let lighting = IntegratorConfig::product();
    let view = translate::View {
        camera: camera(),
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
    let translation = translate::build(&mut session, &scene(procedural), &view).expect("translates");
    // Nothing of the pattern left out.
    assert!(!translation.warnings.iter().any(|w| w.contains("not translated") || w.contains("procedural")), "{:?}", translation.warnings);
    translate::configure(&mut session, &translate::Quality::with_lighting(false, false, None, &lighting)).expect("configures");
    let rgba = session.render(SIZE, SIZE, 64, |_, _| {}).expect("renders");
    rgba.chunks_exact(4).map(|p| (p[0] + p[1] + p[2]) as f64 / 3.0).collect()
}

/// The pattern as Tequila's renderer reads it at each pixel: averaged over
/// the pixel, at the point of the tile each part of it sees, with the
/// footprint its ray has there.
fn tequila(procedural: Procedural) -> Vec<f64> {
    let camera = camera();
    let spread = camera.vertical_fov(1.0) / SIZE as f32;
    let steps = 4;
    let mut out = Vec::with_capacity(SIZE * SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut sum = 0.0f64;
            for s in 0..steps * steps {
                let film = vec2(
                    (x as f32 + ((s % steps) as f32 + 0.5) / steps as f32) / SIZE as f32,
                    (y as f32 + ((s / steps) as f32 + 0.5) / steps as f32) / SIZE as f32,
                );
                let ray = camera.pick_ray(film, 1.0);
                let direction = ray.direction.normalize();
                let t = -ray.origin.y / direction.y;
                let position = ray.origin + direction * t;
                let point = WearPoint { footprint: t * spread, ..WearPoint::flat(position) };
                sum += procedural.evaluate_at(&point, SEED) as f64;
            }
            out.push(sum / (steps * steps) as f64);
        }
    }
    out
}

fn correlation(a: &[f64], b: &[f64]) -> f64 {
    let (ma, mb) = (a.iter().sum::<f64>() / a.len() as f64, b.iter().sum::<f64>() / b.len() as f64);
    let (mut cov, mut va, mut vb) = (0.0, 0.0, 0.0);
    for (x, y) in a.iter().zip(b) {
        cov += (x - ma) * (y - mb);
        va += (x - ma) * (x - ma);
        vb += (y - mb) * (y - mb);
    }
    cov / (va.sqrt() * vb.sqrt()).max(1e-12)
}

#[test]
fn wear_patterns_land_where_tequila_puts_them() {
    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    let patterns = [
        ("scratches", Procedural::WearScratches(Scratches { frequency: 25.0, width: 0.012, density: 0.8, ..Scratches::default() })),
        (
            "parallel scratches",
            Procedural::WearScratches(Scratches { frequency: 25.0, width: 0.012, density: 0.8, direction: ScratchDirection::AlongX, spread: 0.2, ..Scratches::default() }),
        ),
        ("swirls", Procedural::WearScratches(Scratches { frequency: 60.0, width: 0.02, density: 1.0, direction: ScratchDirection::Swirl, layers: 1, ..Scratches::default() })),
        ("dust", Procedural::Dust(Dust { speck_frequency: 300.0, specks: 0.8, ..Dust::default() })),
        ("fingerprints", Procedural::Fingerprints(Fingerprints { frequency: 12.0, prints: 0.9, ..Fingerprints::default() })),
        ("scuffs", Procedural::Scuffs(Scuffs { frequency: 10.0, coverage: 0.9, ..Scuffs::default() })),
    ];
    let mut wrong = Vec::new();
    for (name, procedural) in patterns {
        let (a, b) = (tequila(procedural), cycles(procedural));
        let (ma, mb) = (a.iter().sum::<f64>() / a.len() as f64, b.iter().sum::<f64>() / b.len() as f64);
        let r = correlation(&a, &b);
        eprintln!("{name}: correlation {r:.4}, mean {ma:.4} here, {mb:.4} in Cycles");
        if r < 0.95 || (mb / ma.max(1e-6) - 1.0).abs() > 0.1 {
            wrong.push(format!("{name}: correlation {r:.3}, mean {ma:.4} here and {mb:.4} in Cycles"));
        }
    }
    assert!(wrong.is_empty(), "patterns that differ: {wrong:#?}");
}

/// Scratches are the largest of the patterns as nodes - eight cells a layer,
/// each a scratch worked out whole - and Cycles runs a shader in a stack of
/// 255 numbers: every direction, at every number of layers, has to fit it
/// and still draw what Tequila draws. A shader that did not fit drew nothing.
#[test]
fn every_kind_of_scratches_fits_cycles() {
    if let Err(why) = tequila_render_cycles::available() {
        eprintln!("skipped: {why}");
        return;
    }
    let mut wrong = Vec::new();
    for direction in [ScratchDirection::Random, ScratchDirection::AlongY, ScratchDirection::Swirl] {
        for layers in 1..=3 {
            let procedural = Procedural::WearScratches(Scratches { frequency: 25.0, width: 0.012, density: 1.0, direction, layers, ..Scratches::default() });
            let (a, b) = (tequila(procedural), cycles(procedural));
            let (ma, mb) = (a.iter().sum::<f64>() / a.len() as f64, b.iter().sum::<f64>() / b.len() as f64);
            let r = correlation(&a, &b);
            eprintln!("{direction:?}, {layers} layers: correlation {r:.4}, mean {ma:.5} here, {mb:.5} in Cycles");
            if r < 0.95 || (mb / ma.max(1e-6) - 1.0).abs() > 0.1 {
                wrong.push(format!("{direction:?}, {layers} layers: correlation {r:.3}, mean {ma:.5} here and {mb:.5} in Cycles"));
            }
        }
    }
    assert!(wrong.is_empty(), "scratches that differ: {wrong:#?}");
}
