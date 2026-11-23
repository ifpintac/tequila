//! The glowing triangles on the card against `tequila_render_core::emitters`
//! (`emitters.wgsl`): the left child's share at a node of the tree, choosing
//! a triangle down the tree and a point on it, with the density of that
//! choice, and the density a light sample from a point gives a point a
//! bounce hit, from its leaf up - on random scenes of glowing and dark parts
//! and random points, normals and variates. And the weight a bounce that
//! hits a glow takes against the light sample (`transport.wgsl`'s
//! mis_against_area), against `mis_power` of `area_to_solid_angle_pdf`.
//!
//! The tolerances are `tests/lights.rs`'s: what goes through the card's
//! trigonometry (asin, acos and cos in a node's importance) to 2e-3
//! relative, and a chance under 1e-5 counted as none ([`NONE`]); one item in
//! a thousand may be looser, or choose another triangle where its variate
//! lies on the boundary between two.

use std::sync::Arc;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec2, vec3, Mat4, Transform, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_materials::material_type::MaterialType;
use tequila_materials::openpbr::Material;
use tequila_render_core::emitters::Receiver;
use tequila_render_core::environment::Environment;
use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
use tequila_render_gpu::emitters::{EmitterRecords, EMITTER_NONE};
use tequila_render_gpu::harness::Harness;

fn unit_vector(rng: &mut Pcg32) -> Vec3 {
    let z = 1.0 - 2.0 * rng.next_f32();
    let r = (1.0 - z * z).max(0.0).sqrt();
    let phi = std::f32::consts::TAU * rng.next_f32();
    vec3(r * phi.cos(), r * phi.sin(), z)
}

fn point(rng: &mut Pcg32, extent: f32) -> Vec3 {
    vec3((rng.next_f32() * 2.0 - 1.0) * extent, (rng.next_f32() * 2.0 - 1.0) * extent, (rng.next_f32() * 2.0 - 1.0) * extent)
}

/// A bent strip of `n` by `n` quads, a panel or a curved light guide.
fn strip(rng: &mut Pcg32, n: u32) -> Mesh {
    let bend = rng.next_f32() * 2.0;
    let mut positions = Vec::new();
    for j in 0..=n {
        for i in 0..=n {
            let (u, v) = (i as f32 / n as f32 - 0.5, j as f32 / n as f32 - 0.5);
            positions.push(vec3(u, bend * u * u, v * (0.3 + rng.next_f32() * 0.01)));
        }
    }
    let mut indices = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let a = j * (n + 1) + i;
            indices.extend_from_slice(&[a, a + 1, a + n + 2, a, a + n + 2, a + n + 1]);
        }
    }
    Mesh::new(positions, indices)
}

/// Several parts, some glowing with their own colours and brightness, some
/// dark, some mirrored or squashed, placed about the origin.
fn scene(rng: &mut Pcg32) -> SceneSnapshot {
    let parts = 2 + (rng.next_f32() * 5.0) as usize;
    let mut meshes = Vec::new();
    let mut instances = Vec::new();
    let mut materials = Vec::new();
    for k in 0..parts {
        let n = 1 + (rng.next_f32() * 6.0) as u32;
        meshes.push(Arc::new(strip(rng, n)));
        let glowing = k == 0 || rng.next_f32() < 0.6;
        materials.push(if glowing {
            Material {
                emission_color: Spectrum { r: 0.2 + rng.next_f32(), g: 0.2 + rng.next_f32(), b: 0.2 + rng.next_f32() },
                emission_luminance: 0.5 + rng.next_f32() * 20.0,
                ..MaterialType::Emissive.defaults()
            }
        } else {
            MaterialType::Diffuse.defaults()
        });
        let axis = unit_vector(rng);
        let scale = vec3(0.2 + rng.next_f32(), 0.2 + rng.next_f32(), if rng.next_f32() < 0.2 { -1.0 } else { 0.2 + rng.next_f32() });
        let matrix = Mat4::translation(point(rng, 2.0)) * Mat4::rotation(axis, rng.next_f32() * 6.0) * Mat4::scale(scale);
        let transform = Transform::from_matrix(matrix).expect("an invertible place");
        instances.push(Instance::new(InstanceId(k as u32), MeshId(k as u32), MaterialId(k as u32)).with_transform(transform));
    }
    SceneSnapshot::new(meshes, instances, materials, Arc::new(Environment::uniform(Spectrum::ZERO)))
}

fn receiver(rng: &mut Pcg32) -> Receiver {
    Receiver { position: point(rng, 3.0), magnitude: 0.0, normal: unit_vector(rng), two_sided: rng.next_f32() < 0.3 }
}

/// Every word of the records the kernel binds: the nodes, the leaves, and
/// per instance its first leaf slot.
fn bound(records: &EmitterRecords) -> (Vec<u32>, Vec<u32>) {
    let nodes: &[u32] = bytemuck::cast_slice(&records.nodes);
    let leaves = if records.leaves.is_empty() { vec![EMITTER_NONE] } else { records.leaves.clone() };
    (nodes.to_vec(), leaves)
}

fn close(card: f32, processor: f32, relative: f32, absolute: f32) -> bool {
    (card - processor).abs() <= absolute + relative * processor.abs()
}

/// A share or a density per square metre this small is none: where the
/// processor's importance is nought because a point lies a hair past a
/// node's cone or horizon (an angle at a quarter turn), the card's acos,
/// a few units in the last place off, can leave it a hair inside, and its
/// importance a cosine of 1e-7 rather than nought.
const NONE: f32 = 1e-5;

const SCENES: usize = 40;
const PER_SCENE: usize = 500;

/// The left child's share at random nodes, choosing a triangle and a point
/// on it, and the density of a random triangle (of a glowing part or not),
/// each from random points.
#[test]
fn choosing_a_glowing_triangle_is_the_processors() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(17, 3);
    let (mut total, mut shares, mut chosen, mut densities) = (0usize, 0usize, 0usize, 0usize);
    let (mut share_off, mut choice_off, mut density_off, mut position_off) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for _ in 0..SCENES {
        let scene = scene(&mut rng);
        let emitters = scene.emitters();
        assert!(!emitters.is_empty(), "every scene has a glowing part");
        let records = EmitterRecords::of(&scene, &|n| n);
        let (nodes, leaves) = bound(&records);
        let internal: Vec<u32> = records.nodes.iter().enumerate().filter(|(_, n)| n.left != EMITTER_NONE).map(|(i, _)| i as u32).collect();
        let mut inputs: Vec<u32> = Vec::new();
        let mut cases = Vec::new();
        for _ in 0..PER_SCENE {
            let at = receiver(&mut rng);
            let u = rng.next_f32();
            let uv = vec2(rng.next_f32(), rng.next_f32());
            let node = if internal.is_empty() { 0 } else { internal[(rng.next_f32() * internal.len() as f32) as usize % internal.len()] };
            let instance = (rng.next_f32() * scene.instances.len() as f32) as usize % scene.instances.len();
            let triangles = scene.meshes[instance].indices.len() / 3;
            let primitive = (rng.next_f32() * triangles as f32) as u32 % triangles as u32;
            let first = records.first[instance];
            inputs.extend([at.position.x, at.position.y, at.position.z, at.normal.x, at.normal.y, at.normal.z].map(f32::to_bits));
            inputs.push(at.two_sided as u32);
            inputs.extend([u, uv.x, uv.y].map(f32::to_bits));
            inputs.extend([node, first, primitive]);
            cases.push((at, u, uv, node, instance, primitive));
        }
        let body = "
            let at = vec3<f32>(inf(0u), inf(1u), inf(2u));
            let normal = vec3<f32>(inf(3u), inf(4u), inf(5u));
            let two_sided = inp(6u) != 0u;
            outf(0u, emitters_left_share(scene_emitter_nodes[inp(10u)], at, normal, two_sided));
            let d = emitters_sample(at, normal, two_sided, inf(7u), vec2<f32>(inf(8u), inf(9u)));
            out(1u, select(0u, 1u, d.ok));
            out(2u, d.instance);
            out(3u, d.primitive);
            outf(4u, d.density);
            outf(5u, d.position.x); outf(6u, d.position.y); outf(7u, d.position.z);
            outf(8u, emitters_density(at, normal, two_sided, inp(11u), inp(12u)));
        ";
        let out = h.run_wgsl_with(&["emitters"], body, &inputs, 13, 9, &[(18, &nodes), (19, &leaves)]);
        for (k, (at, u, uv, node, instance, primitive)) in cases.iter().enumerate() {
            let o = &out[k * 9..(k + 1) * 9];
            let f = |i: usize| f32::from_bits(o[i]);
            total += 1;
            if let Some(share) = emitters.left_share_at(*node, at) {
                shares += 1;
                if !close(f(0), share, 2e-3, NONE) {
                    share_off.push(format!("node {node}: card {} processor {share}", f(0)));
                }
            }
            match emitters.sample(&scene, at, *u, *uv) {
                Some(s) => {
                    chosen += 1;
                    if o[1] == 0 || o[2] != s.instance.0 || o[3] != s.primitive {
                        choice_off.push(format!("card {}/{} processor {}/{}", o[2], o[3], s.instance.0, s.primitive));
                    } else {
                        if !close(f(4), s.density, 2e-3, NONE) {
                            density_off.push(format!("drawn: card {} processor {}", f(4), s.density));
                        }
                        let d = (vec3(f(5), f(6), f(7)) - s.position).length();
                        if d > 1e-5 * (1.0 + s.position.length()) {
                            position_off.push(format!("card {:?} processor {:?}", vec3(f(5), f(6), f(7)), s.position));
                        }
                    }
                }
                None => {
                    if o[1] != 0 {
                        choice_off.push("the card chose where the processor did not".into());
                    }
                }
            }
            let want = emitters.density(at, InstanceId(*instance as u32), *primitive);
            if want > 0.0 {
                densities += 1;
            }
            if !close(f(8), want, 2e-3, NONE) {
                density_off.push(format!("part {instance} triangle {primitive}: card {} processor {want}", f(8)));
            }
        }
    }
    println!(
        "{total} items: {shares} shares ({} off), {chosen} triangles chosen ({} another, {} positions off), {densities} of a glowing part's triangle ({} densities off)",
        share_off.len(),
        choice_off.len(),
        position_off.len(),
        density_off.len()
    );
    let allowed = total / 1000;
    for (name, off) in [("shares", &share_off), ("choices", &choice_off), ("densities", &density_off), ("positions", &position_off)] {
        assert!(off.len() <= allowed, "{name}: {} of {total} outside the bound, first {:?}", off.len(), off.iter().take(4).collect::<Vec<_>>());
    }
    assert!(shares > total / 2 && chosen > total / 2 && densities > total / 10, "the cases reach what they test");
}

/// mis_against_area: a bounce's density against a light sample's per unit
/// area, through the distance and the cosine at the glow, as the processor
/// weighs a glow a bounce hits.
#[test]
fn a_glow_hit_is_weighed_as_the_processor_weighs_it() {
    let Some(h) = Harness::for_test() else { return };
    let mut rng = Pcg32::new(23, 5);
    let mut inputs = Vec::new();
    let mut want = Vec::new();
    for k in 0..20_000 {
        let pdf = if k % 50 == 0 { 0.0 } else { (rng.next_f32() * 12.0 - 6.0).exp() };
        let density = (rng.next_f32() * 12.0 - 6.0).exp();
        let d2 = (rng.next_f32() * 8.0 - 4.0).exp();
        let cos = if k % 97 == 0 { 1e-10 } else { rng.next_f32() * 2.0 - 1.0 };
        inputs.extend([pdf, density, d2, cos].map(f32::to_bits));
        let light = tequila_core::sampling::area_to_solid_angle_pdf(density, d2, cos);
        want.push(tequila_core::sampling::mis_power(pdf, light));
    }
    // The transport's own function, lifted out of it.
    let source = tequila_render_gpu::shader::source("transport");
    let at = source.find("fn mis_against_area").expect("the transport has it");
    let end = at + source[at..].find("\n}\n").expect("its end") + 3;
    let mis = source.find("fn mis_power").expect("and mis_power");
    let mis_end = mis + source[mis..].find("\n}\n").expect("its end") + 3;
    let body = format!(
        "outf(0u, mis_against_area(inf(0u), inf(1u), inf(2u), inf(3u)));
    }}
    {}
    {}
    fn unused() {{",
        &source[mis..mis_end],
        &source[at..end]
    );
    let out = h.run_wgsl(&[], &body, &inputs, 4, 1);
    let mut off = Vec::new();
    for (k, w) in want.iter().enumerate() {
        let card = f32::from_bits(out[k]);
        if !close(card, *w, 1e-4, 1e-6) {
            off.push((k, card, *w));
        }
    }
    println!("{} weights, {} outside the bound", want.len(), off.len());
    assert!(off.len() <= want.len() / 1000, "{} outside: {:?}", off.len(), &off[..off.len().min(4)]);
}
