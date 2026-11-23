//! Material graphs on the card (`docs/07` R3, stage 1) against the
//! processor: every node the card runs, at random inputs over their ranges
//! (coordinates up to 1e3), through the whole path a program takes -
//! `program::lower`, the interpreter, the write-back into the material -
//! against `shade.rs`'s `shade`; every preset of the standard library the
//! card runs, at random shading points; and the bump, against
//! `surface_bump.rs`'s `bumped_normal` on a part of its own.
//!
//! The bound: arithmetic to 1e-5 absolute and 1e-4 relative, noise and
//! Voronoi to 1e-4; at most one item in a thousand may be outside it - a
//! cell boundary or a discontinuous node (floor, a comparison, Voronoi's
//! nearest cell) that a last-bit difference puts the other side, or an
//! ill-conditioned spot (a tangent near a quarter turn) - and each such item
//! is printed.

use std::sync::Arc;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec2, vec3, Mat4, Transform, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{CompiledGraph, Instruction, NodeKind, Register, Requirements, SocketType, Value};
use tequila_materials::material_type::Param;
use tequila_materials::nodes::{InputSpec, MixData, ShaderNode};
use tequila_materials::openpbr::Material;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_render_gpu::harness::Harness;
use tequila_render_gpu::material::GpuMaterial;
use tequila_render_gpu::program::{self, CardProgram, ProgramTable};
use tequila_texture::cycles::noise::FractalKind;
use tequila_texture::cycles::ops::{BlendMode, ColorModel, MapRangeKind, MappingKind, MathOp, VectorMathOp};
use tequila_texture::cycles::textures::GradientKind;
use tequila_texture::cycles::voronoi::{VoronoiFeature, VoronoiMetric};
use tequila_texture::projection::{PointWorld, RayKind, SurfacePoint};

/// Words an item gives back: the material (40 floats), the bump's height
/// here, the Tangent drawn (world space), the shading normal.
const OUT: usize = 49;
/// Words an item takes: the program's header, the point, the surface, the
/// material it is written over.
const IN: usize = 123;

/// Everything the card needs of a point beyond the program.
#[derive(Clone, Copy)]
struct Surface {
    normal: Vec3,
    geometric: Vec3,
    footprint: f32,
    derivatives: Option<(Vec3, Vec3)>,
    transform: Transform,
}

/// One item: a program, where it runs, and what the processor makes of it.
struct Case {
    program: CompiledGraph,
    point: SurfacePoint,
    surface: Surface,
    /// What the processor's normal is with the bump applied, where the case
    /// compares it.
    normal: Option<Vec3>,
    /// The bump's heights a step along the tangent and the bitangent, where
    /// the case compares them.
    heights: Option<[f32; 2]>,
    label: String,
}

fn kernel_body() -> String {
    let v3 = |k: usize| format!("vec3<f32>(inf({k}u), inf({}u), inf({}u))", k + 1, k + 2);
    let row = |k: usize| format!("vec4<f32>(inf({k}u), inf({}u), inf({}u), inf({}u))", k + 1, k + 2, k + 3);
    format!(
        "
    let p = GpuProgram(inp(0u), inp(1u), inp(2u), inp(3u), inp(4u), inp(5u), inp(6u), inp(7u), inp(8u), inp(9u), inp(10u), 0u);
    var pt: ProgPoint;
    pt.object = {};
    pt.scale = inf(14u);
    pt.object_normal = {};
    pt.backfacing = inf(18u);
    pt.uv = vec2<f32>(inf(19u), inf(20u));
    pt.object_random = inf(21u);
    pt.object_number = inf(22u);
    pt.position = {};
    pt.ray = inp(26u);
    pt.normal = {};
    pt.true_normal = {};
    pt.incoming = {};
    pt.generated = {};
    pt.object_location = {};
    pt.object_color = {};
    var s: ProgSurface;
    s.normal = {};
    s.geometric = {};
    s.footprint = inf(51u);
    s.has_derivatives = inp(52u) != 0u;
    s.dpdu = {};
    s.dpdv = {};
    s.object_to_world = array<vec4<f32>, 3>({}, {}, {});
    s.world_to_object = array<vec4<f32>, 3>({}, {}, {});
    var b: array<f32, 44>;
    for (var k = 0u; k < 40u; k++) {{
        b[k] = inf(83u + k);
    }}
    let r = prog_shade(p, pt, prog_material_from(b, GpuMaterial()), s);
    let f = prog_material_floats(r.material);
    for (var k = 0u; k < 40u; k++) {{
        outf(k, f[k]);
    }}
    outf(40u, r.here);
    outf(41u, r.drawn.x); outf(42u, r.drawn.y); outf(43u, r.drawn.z);
    outf(44u, r.normal.x); outf(45u, r.normal.y); outf(46u, r.normal.z);
    outf(47u, r.heights.x); outf(48u, r.heights.y);",
        v3(11),
        v3(15),
        v3(23),
        v3(27),
        v3(30),
        v3(33),
        v3(36),
        v3(39),
        v3(42),
        v3(45),
        v3(48),
        v3(53),
        v3(56),
        row(59),
        row(63),
        row(67),
        row(71),
        row(75),
        row(79),
    )
}

/// GpuMaterial's floats before its emission, which no stage-1 program
/// drives (the kernel leaves it at nought).
fn floats(m: &GpuMaterial) -> [f32; 40] {
    let all: &[f32] = bytemuck::cast_slice(std::slice::from_ref(m));
    all[..40].try_into().expect("forty floats")
}

fn ray_code(ray: RayKind) -> u32 {
    match ray {
        RayKind::Camera => 0,
        RayKind::Shadow => 1,
        RayKind::Diffuse => 2,
        RayKind::Glossy => 3,
        RayKind::Transmission => 4,
    }
}

/// The parameter a program's bump height is in, as `BumpSource::of` picks.
fn bump_param(program: &CompiledGraph) -> Option<Param> {
    let driven = |p: Param| program.driven.iter().any(|(q, _)| *q == p);
    if driven(Param::Bump) {
        Some(Param::Bump)
    } else if driven(Param::Displacement) && program.constants.displacement_detail <= 0.0 {
        Some(Param::Displacement)
    } else {
        None
    }
}

/// What the processor makes of a case: the material, the height here, the
/// Tangent drawn, the normal.
fn processor(case: &Case) -> [f32; OUT] {
    let mut registers = Registers::new();
    let mut scratch = Material::default();
    let material = shade(&case.program, &TextureTable::new(), &case.point, &mut registers, &mut scratch).clone();
    let mut out = [0.0f32; OUT];
    out[..40].copy_from_slice(&floats(&GpuMaterial::from_material(&material)));
    let register = |p: Param| case.program.driven.iter().find(|(q, _)| *q == p).map(|(_, r)| *r);
    if let Some(r) = bump_param(&case.program).and_then(register) {
        out[40] = registers.value(r).as_float();
    }
    if let Some(r) = register(Param::Tangent) {
        let t = case.surface.transform.vector(registers.value(r).as_vector());
        out[41..44].copy_from_slice(&[t.x, t.y, t.z]);
    }
    let n = case.normal.unwrap_or(case.surface.normal);
    out[44..47].copy_from_slice(&[n.x, n.y, n.z]);
    if let Some(h) = case.heights {
        out[47..49].copy_from_slice(&h);
    }
    out
}

fn rows(m: &Mat4) -> Vec<f32> {
    (0..3).flat_map(|r| m.m[r]).collect()
}

/// Run every case on the card, its program lowered (`None` for a case the
/// card refuses, which runs as its constants).
fn card(harness: &Harness, cases: &[Case], lowered: &[CardProgram]) -> Vec<[f32; OUT]> {
    let table = ProgramTable::new(&lowered.iter().cloned().map(Some).collect::<Vec<_>>());
    let mut inputs: Vec<u32> = Vec::with_capacity(cases.len() * IN);
    for (case, header) in cases.iter().zip(&table.headers) {
        let h = header;
        inputs.extend_from_slice(&[
            h.main_start,
            h.main_end,
            h.bump_start,
            h.bump_end,
            h.constants,
            h.writes_start,
            h.writes_count,
            h.bump_here,
            h.bump_height,
            h.tangent,
            h.flags,
        ]);
        let p = &case.point;
        let w = &p.world;
        let s = &case.surface;
        let (dpdu, dpdv) = s.derivatives.unwrap_or((Vec3::ZERO, Vec3::ZERO));
        let mut f: Vec<f32> = vec![p.position.x, p.position.y, p.position.z, p.scale, p.normal.x, p.normal.y, p.normal.z];
        f.push(if w.backfacing { 1.0 } else { 0.0 });
        f.extend_from_slice(&[p.uv.x, p.uv.y, w.object_random, w.object_number, w.position.x, w.position.y, w.position.z]);
        inputs.extend(f.iter().map(|v| v.to_bits()));
        inputs.push(ray_code(w.ray));
        let mut f: Vec<f32> = Vec::new();
        for v in [w.normal, w.true_normal, w.incoming, w.generated, w.object_location] {
            f.extend_from_slice(&[v.x, v.y, v.z]);
        }
        f.extend_from_slice(&[w.object_color.r, w.object_color.g, w.object_color.b]);
        f.extend_from_slice(&[s.normal.x, s.normal.y, s.normal.z, s.geometric.x, s.geometric.y, s.geometric.z, s.footprint]);
        inputs.extend(f.iter().map(|v| v.to_bits()));
        inputs.push(s.derivatives.is_some() as u32);
        let mut f: Vec<f32> = vec![dpdu.x, dpdu.y, dpdu.z, dpdv.x, dpdv.y, dpdv.z];
        f.extend(rows(&s.transform.matrix));
        f.extend(rows(&s.transform.inverse));
        f.extend_from_slice(&floats(&GpuMaterial::from_material(&case.program.constants)));
        inputs.extend(f.iter().map(|v| v.to_bits()));
    }
    assert_eq!(inputs.len(), cases.len() * IN);
    let words = harness.run_wgsl_with(
        &["program"],
        &kernel_body(),
        &inputs,
        IN,
        OUT,
        &[(11, &table.words), (12, bytemuck::cast_slice(&table.constants))],
    );
    words.chunks(OUT).map(|c| std::array::from_fn(|k| f32::from_bits(c[k]))).collect()
}

/// How close is close: an absolute and a relative bound.
#[derive(Clone, Copy)]
struct Bound {
    absolute: f32,
    relative: f32,
}

const ARITHMETIC: Bound = Bound { absolute: 1e-5, relative: 1e-4 };
const NOISE: Bound = Bound { absolute: 1e-4, relative: 1e-4 };

fn close(card: f32, processor: f32, bound: Bound) -> bool {
    if card.is_nan() && processor.is_nan() {
        return true;
    }
    if card.is_infinite() || processor.is_infinite() {
        return card == processor || (card.abs() > 1e30 && processor.abs() > 1e30 && card.signum() == processor.signum());
    }
    (card - processor).abs() <= bound.absolute + bound.relative * processor.abs()
}

/// Every case on both sides, each word within `bound` but for at most one
/// item in a thousand. Returns how many were outside it.
fn compare(harness: &Harness, name: &str, cases: &[Case], bound: Bound, normal_bound: Option<Bound>) -> usize {
    let lowered: Vec<CardProgram> = cases
        .iter()
        .map(|c| program::lower(&c.program).unwrap_or_else(|why| panic!("{name}: {} is refused: {why}", c.label)).expect("a program to run"))
        .collect();
    let on_card = card(harness, cases, &lowered);
    let mut outside = Vec::new();
    for (i, case) in cases.iter().enumerate() {
        let want = processor(case);
        let got = on_card[i];
        if i < 4 && std::env::var("TEQUILA_GRAPH_DEBUG").is_ok() {
            eprintln!("  #{i} {}: here {} / {}, heights {:?} / {:?}, normal {:?} / {:?}", case.label, got[40], want[40], &got[47..49], &want[47..49], &got[44..47], &want[44..47]);
        }
        let mut worst: Option<(usize, f32, f32)> = None;
        for k in 0..OUT {
            let b = if k >= 47 {
                match case.heights {
                    Some(_) => Bound { absolute: 2e-9, relative: 1e-4 },
                    None => continue,
                }
            } else if k >= 44 {
                match normal_bound {
                    Some(b) => b,
                    None => continue,
                }
            } else {
                bound
            };
            if !close(got[k], want[k], b) {
                let d = (got[k] - want[k]).abs();
                if worst.is_none_or(|(_, g, w)| d > (g - w).abs()) {
                    worst = Some((k, got[k], want[k]));
                }
            }
        }
        if let Some((k, g, w)) = worst {
            outside.push(format!("  {} word {k}: card {g}, processor {w}", case.label));
        }
    }
    eprintln!("{name}: {} items, {} outside the bound", cases.len(), outside.len());
    for line in outside.iter().take(12) {
        eprintln!("{line}");
    }
    outside.len()
}

/// At most one item in a thousand outside the bound.
fn within_budget(name: &str, outside: usize, total: usize) {
    assert!(outside * 1000 <= total, "{name}: {outside} of {total} items outside the bound, more than one in a thousand");
}

/// [`compare`], held to [`within_budget`].
fn compare_all(harness: &Harness, name: &str, cases: &[Case], bound: Bound) {
    let outside = compare(harness, name, cases, bound, None);
    within_budget(name, outside, cases.len());
}

// ---- Random inputs -------------------------------------------------------

fn uniform(rng: &mut Pcg32, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * rng.next_f32()
}

/// A number with no range of its own: mostly small, now and then up to 1e3.
fn wide(rng: &mut Pcg32) -> f32 {
    let r = rng.next_f32();
    let span = if r < 0.4 {
        1.0
    } else if r < 0.7 {
        10.0
    } else if r < 0.9 {
        100.0
    } else {
        1000.0
    };
    uniform(rng, -span, span)
}

fn random_value(rng: &mut Pcg32, spec: &InputSpec) -> Value {
    match spec.ty {
        SocketType::Float => Value::Float(match spec.range {
            Some((lo, hi)) => {
                let r = rng.next_f32();
                if r < 0.03 {
                    lo
                } else if r < 0.06 {
                    hi
                } else {
                    uniform(rng, lo, hi)
                }
            }
            None => wide(rng),
        }),
        SocketType::Vector | SocketType::Normal => Value::Vector(vec3(wide(rng), wide(rng), wide(rng))),
        SocketType::Color => {
            let c = |rng: &mut Pcg32| if rng.next_f32() < 0.1 { uniform(rng, -0.5, 2.0) } else { rng.next_f32() };
            Value::Color(Spectrum { r: c(rng), g: c(rng), b: c(rng) })
        }
        _ => Value::Float(0.0),
    }
}

fn unit(rng: &mut Pcg32) -> Vec3 {
    loop {
        let v = vec3(uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0));
        let l = v.length();
        if l > 0.1 && l <= 1.0 {
            return v * (1.0 / l);
        }
    }
}

/// A shading point with every field random, as a part anywhere might give.
fn random_point(rng: &mut Pcg32, reach: f32) -> SurfacePoint {
    let rays = [RayKind::Camera, RayKind::Diffuse, RayKind::Glossy];
    let normal = unit(rng);
    SurfacePoint {
        position: vec3(uniform(rng, -reach, reach), uniform(rng, -reach, reach), uniform(rng, -reach, reach)),
        normal: unit(rng),
        uv: vec2(uniform(rng, -0.5, 1.5), uniform(rng, -0.5, 1.5)),
        footprint: uniform(rng, 1e-5, 2e-3),
        scale: if rng.next_f32() < 0.5 { 1.0 } else { uniform(rng, 0.1, 10.0) },
        uv_footprint: None,
        curvature: 0.0,
        occlusion: 0.0,
        vertex_color: Spectrum::ONE,
        world: PointWorld {
            position: vec3(uniform(rng, -reach, reach), uniform(rng, -reach, reach), uniform(rng, -reach, reach)),
            normal,
            true_normal: if rng.next_f32() < 0.5 { normal } else { unit(rng) },
            incoming: unit(rng),
            backfacing: rng.next_f32() < 0.2,
            generated: vec3(rng.next_f32(), rng.next_f32(), rng.next_f32()),
            object_location: vec3(wide(rng), wide(rng), wide(rng)),
            object_random: rng.next_f32(),
            closeness: 0.0,
            object_color: Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() },
            object_number: (rng.next_f32() * 10.0).floor(),
            ray: rays[(rng.next_f32() * 3.0) as usize % 3],
        },
    }
}

fn flat_surface(point: &SurfacePoint) -> Surface {
    Surface { normal: point.world.normal, geometric: point.world.true_normal, footprint: point.footprint, derivatives: None, transform: Transform::IDENTITY }
}

const FLOAT_PARAMS: [Param; 4] = [Param::SpecularRoughness, Param::CoatRoughness, Param::FuzzRoughness, Param::BaseWeight];
const COLOUR_PARAMS: [Param; 4] = [Param::BaseColor, Param::SpecularColor, Param::CoatColor, Param::FuzzColor];

/// A program of one node, its inputs literals, each output read wired to a
/// parameter of its kind.
fn single_node(kind: &NodeKind, inputs: &[Value], outputs: &[u8]) -> CompiledGraph {
    let n = inputs.len();
    let mut instructions: Vec<Instruction> = inputs.iter().enumerate().map(|(i, v)| Instruction::Load { dst: Register(i as u16), value: *v }).collect();
    let mut driven = Vec::new();
    let (mut f, mut c) = (0, 0);
    for (k, output) in outputs.iter().enumerate() {
        let dst = Register((n + k) as u16);
        instructions.push(Instruction::Apply { dst, kind: Box::new(kind.clone()), args: (0..n).map(|i| Register(i as u16)).collect(), output: *output });
        let param = match kind.output_type_at(*output) {
            Some(SocketType::Float) | None => {
                f += 1;
                FLOAT_PARAMS[f - 1]
            }
            _ => {
                c += 1;
                COLOUR_PARAMS[c - 1]
            }
        };
        driven.push((param, dst));
    }
    driven.sort_by_key(|(p, _)| p.key());
    CompiledGraph {
        decals: Vec::new(),
        instructions,
        registers: n + outputs.len(),
        constants: Material::default(),
        driven,
        requirements: Requirements::default(),
        topology: 0,
        layer: None,
    }
}

/// `count` cases of `node`, its outputs `outputs`, at random inputs.
fn node_cases(rng: &mut Pcg32, node: ShaderNode, outputs: &[u8], count: usize, cases: &mut Vec<Case>) {
    node_cases_tweaked(rng, node, outputs, count, cases, &|_, _| {});
}

/// [`node_cases`], each input list tweaked after it is drawn.
fn node_cases_tweaked(rng: &mut Pcg32, node: ShaderNode, outputs: &[u8], count: usize, cases: &mut Vec<Case>, tweak: &dyn Fn(&mut Pcg32, &mut Vec<Value>)) {
    let kind = NodeKind::Node(node);
    let specs = node.inputs();
    for _ in 0..count {
        let mut inputs: Vec<Value> = specs.iter().map(|s| random_value(rng, s)).collect();
        // A normal and the way to the viewer are directions.
        if matches!(node, ShaderNode::LayerWeight | ShaderNode::Fresnel) {
            inputs[1] = Value::Vector(unit(rng));
            inputs[2] = Value::Vector(unit(rng));
        }
        tweak(rng, &mut inputs);
        let point = random_point(rng, 1.0);
        cases.push(Case {
            program: single_node(&kind, &inputs, outputs),
            surface: flat_surface(&point),
            point,
            normal: None,
            heights: None,
            label: format!("{node:?} {outputs:?} at {inputs:?}"),
        });
    }
}

fn harness() -> Option<Harness> {
    Harness::for_test()
}

// ---- One node at a time --------------------------------------------------

#[test]
fn math_is_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(1, 1);
    let mut cases = Vec::new();
    for &op in MathOp::ALL {
        for clamp in [false, true] {
            node_cases(&mut rng, ShaderNode::Math { op, clamp }, &[0], 500, &mut cases);
        }
    }
    compare_all(&h, "Math", &cases, ARITHMETIC);
}

#[test]
fn vector_math_is_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(2, 1);
    let mut cases = Vec::new();
    for &op in VectorMathOp::ALL {
        node_cases(&mut rng, ShaderNode::VectorMath { op }, &[0], 1000, &mut cases);
    }
    compare_all(&h, "Vector Math", &cases, ARITHMETIC);
}

#[test]
fn map_range_mix_and_the_converters_are_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(3, 1);
    let mut cases = Vec::new();
    for kind in MapRangeKind::ALL {
        for clamp in [false, true] {
            node_cases(&mut rng, ShaderNode::MapRange { kind, clamp }, &[0], 1000, &mut cases);
        }
    }
    for data in [MixData::Float, MixData::Vector, MixData::Color] {
        for &blend in BlendMode::ALL {
            if data != MixData::Color && blend != BlendMode::Mix {
                continue;
            }
            for (clamp_factor, clamp_result) in [(true, false), (false, true), (false, false)] {
                node_cases(&mut rng, ShaderNode::Mix { data, blend, clamp_factor, clamp_result }, &[0], 300, &mut cases);
            }
        }
    }
    for kind in MappingKind::ALL {
        node_cases(&mut rng, ShaderNode::Mapping { kind }, &[0], 1000, &mut cases);
    }
    node_cases(&mut rng, ShaderNode::CombineXyz, &[0], 500, &mut cases);
    node_cases(&mut rng, ShaderNode::SeparateXyz, &[0, 1, 2], 500, &mut cases);
    node_cases(&mut rng, ShaderNode::SeparateXyz, &[1], 500, &mut cases);
    for model in ColorModel::ALL {
        node_cases(&mut rng, ShaderNode::SeparateColor { model }, &[0, 1, 2], 500, &mut cases);
        node_cases(&mut rng, ShaderNode::SeparateColor { model }, &[2], 300, &mut cases);
        node_cases(&mut rng, ShaderNode::CombineColor { model }, &[0], 500, &mut cases);
    }
    for range in [false, true] {
        node_cases(&mut rng, ShaderNode::Clamp { range }, &[0], 500, &mut cases);
    }
    for node in [ShaderNode::HueSaturationValue, ShaderNode::BrightContrast, ShaderNode::Gamma, ShaderNode::InvertColor, ShaderNode::Checker] {
        node_cases(&mut rng, node, &[0], 500, &mut cases);
    }
    node_cases(&mut rng, ShaderNode::Checker, &[1], 500, &mut cases);
    node_cases(&mut rng, ShaderNode::Checker, &[0, 1], 500, &mut cases);
    for kind in GradientKind::ALL {
        node_cases(&mut rng, ShaderNode::GradientTexture { kind }, &[0, 1], 300, &mut cases);
    }
    node_cases(&mut rng, ShaderNode::LayerWeight, &[0, 1], 1000, &mut cases);
    node_cases(&mut rng, ShaderNode::Fresnel, &[0], 1000, &mut cases);
    compare_all(&h, "Map Range, Mix and the converters", &cases, ARITHMETIC);
}

/// Tequila's own nodes, and the colour ramp.
#[test]
fn tequilas_own_nodes_and_the_colour_ramp_are_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(4, 1);
    let mut cases = Vec::new();
    let colour = |rng: &mut Pcg32| Value::Color(Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() });
    for _ in 0..600 {
        let t = Value::Float(uniform(&mut rng, -0.5, 1.5));
        let from = vec2(uniform(&mut rng, -2.0, 2.0), uniform(&mut rng, -2.0, 2.0));
        let to = vec2(uniform(&mut rng, -2.0, 2.0), uniform(&mut rng, -2.0, 2.0));
        let (low, high) = (uniform(&mut rng, -1.0, 0.5), uniform(&mut rng, 0.5, 2.0));
        let channel = [tequila_materials::graph::Channel::Red, tequila_materials::graph::Channel::Green, tequila_materials::graph::Channel::Blue][(rng.next_f32() * 3.0) as usize % 3];
        let stops: Vec<tequila_texture::gradient::Stop> = (0..1 + (rng.next_f32() * 8.0) as usize)
            .map(|_| tequila_texture::gradient::Stop { at: if rng.next_f32() < 0.1 { 0.5 } else { rng.next_f32() }, color: Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() } })
            .collect();
        let interpolation = tequila_texture::gradient::Interpolation::ALL[(rng.next_f32() * 3.0) as usize % 3];
        let gradient = tequila_texture::gradient::Gradient::new(stops).with_interpolation(interpolation);
        let nodes: Vec<(NodeKind, Vec<Value>)> = vec![
            (NodeKind::Mix, vec![colour(&mut rng), colour(&mut rng), t]),
            (NodeKind::Multiply, vec![colour(&mut rng), colour(&mut rng)]),
            (NodeKind::Add, vec![colour(&mut rng), colour(&mut rng)]),
            (NodeKind::Invert, vec![colour(&mut rng)]),
            (NodeKind::Remap { from, to }, vec![colour(&mut rng)]),
            (NodeKind::Clamp { low, high }, vec![colour(&mut rng)]),
            (NodeKind::Channel { channel }, vec![colour(&mut rng)]),
            (NodeKind::Bump { depth: uniform(&mut rng, 1e-4, 1e-2) }, vec![Value::Float(wide(&mut rng))]),
            (NodeKind::Bump { depth: 0.001 }, vec![colour(&mut rng)]),
            (NodeKind::Gradient { gradient: Box::new(gradient) }, vec![t]),
        ];
        for (kind, inputs) in nodes {
            let point = random_point(&mut rng, 1.0);
            cases.push(Case { label: format!("{kind:?} at {inputs:?}"), program: single_node(&kind, &inputs, &[0]), surface: flat_surface(&point), point, normal: None, heights: None });
        }
    }
    compare_all(&h, "Tequila's own nodes", &cases, ARITHMETIC);
}

#[test]
fn noise_is_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(5, 1);
    let mut cases = Vec::new();
    for dims in 1..=4u8 {
        for fractal in FractalKind::ALL {
            for normalize in [true, false] {
                for outputs in [&[0u8][..], &[1], &[0, 1]] {
                    let node = ShaderNode::Noise { dims, fractal, normalize };
                    // Undistorted, over every input's whole range.
                    node_cases_tweaked(&mut rng, node, outputs, 80, &mut cases, &|_, inputs| inputs[8] = Value::Float(0.0));
                    // Distorted: the distortion moves the point by a noise,
                    // whose last-bit difference the octaves then multiply -
                    // by the lacunarity to the detail. Where that comes to
                    // more than ten, the processor's own answer is
                    // already its rounding's: such a point is no test of the
                    // port.
                    node_cases_tweaked(&mut rng, node, outputs, 40, &mut cases, &|rng, inputs| {
                        let distortion = inputs[8].as_float();
                        loop {
                            let (detail, lacunarity) = (inputs[3].as_float().clamp(0.0, 15.0), inputs[5].as_float());
                            if distortion * lacunarity.max(1.0).powf(detail.floor() + 1.0) <= 10.0 {
                                break;
                            }
                            inputs[3] = Value::Float(uniform(rng, 0.0, 15.0));
                            inputs[5] = Value::Float(uniform(rng, 0.0, 8.0));
                        }
                    });
                }
            }
        }
    }
    compare_all(&h, "Noise", &cases, NOISE);
}

#[test]
fn white_noise_is_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(6, 1);
    let mut cases = Vec::new();
    for dims in 1..=4u8 {
        for outputs in [&[0u8][..], &[1], &[0, 1]] {
            node_cases(&mut rng, ShaderNode::WhiteNoise { dims }, outputs, 1000, &mut cases);
        }
    }
    compare_all(&h, "White Noise", &cases, NOISE);
}

#[test]
fn voronoi_is_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(7, 1);
    let mut cases = Vec::new();
    for dims in 1..=4u8 {
        for feature in VoronoiFeature::ALL {
            for metric in VoronoiMetric::ALL {
                let normalize = rng.next_f32() < 0.5;
                let node = ShaderNode::Voronoi { dims, feature, metric, normalize };
                let all: &[&[u8]] = match feature {
                    VoronoiFeature::DistanceToEdge | VoronoiFeature::NSphereRadius => &[&[0]],
                    _ => &[&[0], &[1], &[2, 3], &[0, 1, 2, 3]],
                };
                for outputs in all {
                    // A Minkowski exponent of 0 is no distance (its answer is
                    // infinity or NaN by how it is summed), and past 8 the
                    // card's powf - exp2 of a log2 good to 2^-22 - orders
                    // nearly equal cells otherwise now and then.
                    node_cases_tweaked(&mut rng, node, outputs, 60, &mut cases, &|rng, inputs| {
                        if metric == VoronoiMetric::Minkowski {
                            inputs[7] = Value::Float(uniform(rng, 0.25, 8.0));
                        }
                    });
                }
            }
        }
    }
    compare_all(&h, "Voronoi", &cases, NOISE);
}

/// Every point input, through Texture Coordinate, Geometry, Object Info and
/// Light Path.
#[test]
fn the_point_inputs_are_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(8, 1);
    let mut cases = Vec::new();
    for node in [ShaderNode::TextureCoordinate, ShaderNode::Geometry, ShaderNode::ObjectInfo, ShaderNode::LightPath] {
        let count = node.outputs().len() as u8;
        for output in 0..count {
            for _ in 0..200 {
                let point = random_point(&mut rng, 5.0);
                // The input, through a node that only passes it on.
                let pass = NodeKind::Node(ShaderNode::VectorMath { op: VectorMathOp::Add });
                let input = node.point_output(output).expect("a point input");
                let instructions = vec![
                    Instruction::Input { dst: Register(0), input },
                    Instruction::Load { dst: Register(1), value: Value::Vector(Vec3::ZERO) },
                    Instruction::Apply { dst: Register(2), kind: Box::new(pass.clone()), args: vec![Register(0), Register(1), Register(1), Register(1)], output: 0 },
                    Instruction::Apply { dst: Register(3), kind: Box::new(NodeKind::Node(ShaderNode::Math { op: MathOp::Add, clamp: false })), args: vec![Register(0), Register(1), Register(1)], output: 0 },
                ];
                let program = CompiledGraph {
                    decals: Vec::new(),
                    instructions,
                    registers: 4,
                    constants: Material::default(),
                    driven: vec![(Param::BaseColor, Register(2)), (Param::SpecularRoughness, Register(3)), (Param::CoatColor, Register(0))],
                    requirements: Requirements::default(),
                    topology: 0,
                    layer: None,
                };
                cases.push(Case { label: format!("{input:?}"), program, surface: flat_surface(&point), point, normal: None, heights: None });
            }
        }
    }
    compare_all(&h, "The point inputs", &cases, ARITHMETIC);
}

// ---- The library ---------------------------------------------------------

/// The standard library's presets, compiled as the snapshot compiles them
/// (a graph of one node has no program).
fn library() -> Vec<(String, CompiledGraph)> {
    let directory = tequila_format::library::standard_directory();
    let (entries, problems) = tequila_format::library::read_directory(&directory);
    assert!(!entries.is_empty(), "no presets in {}: {problems:?}", directory.display());
    let mut out = Vec::new();
    for entry in entries {
        if entry.graph.len() <= 1 && entry.graph.decals.is_empty() {
            continue;
        }
        match entry.graph.compile() {
            Ok(program) => out.push((entry.name.clone(), program)),
            Err(why) => eprintln!("  {} does not compile: {why}", entry.name),
        }
    }
    out
}

/// Every preset the card runs, at random shading points: the parameter block,
/// the bump's height and the Tangent, against `shade`. Says how many presets
/// the card runs and why it refuses the others.
#[test]
fn the_librarys_presets_are_the_processors() {
    let Some(h) = harness() else { return };
    let presets = library();
    let mut rng = Pcg32::new(9, 1);
    let mut cases = Vec::new();
    let mut refused: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let (mut constants, mut runs, mut bumps) = (0, 0, 0);
    for (name, program) in &presets {
        match program::lower(program) {
            Err(why) => {
                let key = match why {
                    program::Refusal::Node(n) => format!("the {n} node"),
                    program::Refusal::Param(p) => format!("driving {}", p.label()),
                    other => other.to_string(),
                };
                refused.entry(key).or_default().push(name.clone());
                continue;
            }
            Ok(None) => {
                constants += 1;
                continue;
            }
            Ok(Some(card)) => {
                runs += 1;
                bumps += card.has_bump() as usize;
            }
        }
        for _ in 0..256 {
            // A product's part: within 15 cm of its origin mostly, now and
            // then a metre, at its own scale. (A knurl 0.7 mm in pitch wrapped
            // round a radius of metres is the last bit of an angle times it,
            // the processor's as much as the card's.)
            let reach = if rng.next_f32() < 0.8 { 0.15 } else { 1.0 };
            let mut point = random_point(&mut rng, reach);
            point.scale = 1.0;
            cases.push(Case { label: name.clone(), program: program.clone(), surface: flat_surface(&point), point, normal: None, heights: None });
        }
    }
    let total = presets.len();
    eprintln!("the library: {total} presets with a program; the card runs {runs} ({bumps} bumping), {constants} are their constants, {} refused", total - runs - constants);
    for (why, names) in &refused {
        eprintln!("  refused, {why}: {} ({})", names.len(), names.iter().take(4).cloned().collect::<Vec<_>>().join(", "));
    }
    // In batches, so no dispatch runs too long for the display.
    let mut outside = 0;
    for (i, batch) in cases.chunks(4096).enumerate() {
        outside += compare(&h, &format!("the library, batch {i}"), batch, Bound { absolute: 1e-4, relative: 1e-4 }, None);
    }
    eprintln!("the library: {} points, {outside} outside the bound", cases.len());
    within_budget("the library", outside, cases.len());
}

// ---- The bump ------------------------------------------------------------

/// A triangle of a part placed turned, scaled and moved, with UVs, wearing
/// `program`.
fn bump_scene(program: &CompiledGraph, transform: Transform, uvs: bool) -> tequila_render_core::scene::SceneSnapshot {
    use tequila_render_core::scene::{Instance, Mesh, SceneSnapshot};
    let mut mesh = Mesh::new(vec![vec3(0.0, 0.0, 0.0), vec3(0.3, 0.02, 0.0), vec3(0.05, 0.25, 0.04)], vec![0, 1, 2])
        .with_normals(vec![vec3(0.1, 0.0, 1.0).normalize(), vec3(0.0, 0.1, 1.0).normalize(), vec3(-0.1, -0.1, 1.0).normalize()]);
    if uvs {
        mesh = mesh.with_uvs(vec![vec2(0.1, 0.9), vec2(0.8, 0.85), vec2(0.2, 0.1)]);
    }
    SceneSnapshot::new(
        vec![Arc::new(mesh)],
        vec![Instance::new(InstanceId(0), MeshId(0), MaterialId(0)).with_transform(transform)],
        vec![Material::default()],
        Arc::new(tequila_render_core::environment::Environment::uniform(Spectrum::ONE)),
    )
    .with_programs(vec![Some(Arc::new(program.clone()))], Arc::new(TextureTable::new()))
}

/// The bumped normal of every bumping preset the card runs, at random points
/// of a placed triangle, against `bumped_normal`: the heights to the
/// arithmetic bound, the normal (forward differences over a tenth of a
/// footprint, which cancel) to 1e-3.
#[test]
fn bumped_normals_are_the_processors() {
    use tequila_render_core::intersect::Hit;
    let Some(h) = harness() else { return };
    let presets = library();
    let mut rng = Pcg32::new(10, 1);
    let mut cases = Vec::new();
    let mut count = 0;
    for (name, program) in &presets {
        let Ok(Some(card)) = program::lower(program) else { continue };
        if !card.has_bump() {
            continue;
        }
        count += 1;
        for k in 0..48 {
            let angle = uniform(&mut rng, -3.0, 3.0);
            let axis = unit(&mut rng);
            let scale = if k % 2 == 0 { 1.0 } else { uniform(&mut rng, 0.2, 3.0) };
            let matrix = Mat4::translation(vec3(wide(&mut rng) * 0.01, 0.1, -0.2)) * Mat4::rotation(axis, angle) * Mat4::scale(Vec3::splat(scale));
            let transform = Transform::from_matrix(matrix).expect("invertible");
            let scene = bump_scene(program, transform, k % 4 != 3);
            let mesh = scene.mesh(MeshId(0));
            let (u, v) = (rng.next_f32(), rng.next_f32());
            let (u, v) = if u + v > 1.0 { (1.0 - u, 1.0 - v) } else { (u, v) };
            let w = 1.0 - u - v;
            let p = &mesh.positions;
            let position = transform.point(p[0] * w + p[1] * u + p[2] * v);
            let geometric = transform.normal(mesh.geometric_normal_scaled(0)).normalize();
            let n = &mesh.normals;
            let shading = transform.normal(n[0] * w + n[1] * u + n[2] * v).normalize();
            let uv = if mesh.uvs.is_empty() { vec2(u, v) } else { mesh.uvs[0] * w + mesh.uvs[1] * u + mesh.uvs[2] * v };
            let distance = uniform(&mut rng, 0.05, 3.0);
            let hit = Hit { t: distance, position, magnitude: 0.0, geometric_normal: geometric, shading_normal: shading, uv, instance: InstanceId(0), material: MaterialId(0), primitive: 0 };
            let direction = (-geometric + unit(&mut rng) * 0.5).normalize();
            let direction = if direction.dot(geometric) >= 0.0 { -geometric } else { direction };
            let point = scene.shading_point_along(&hit, shading, tequila_render_core::settings::DEFAULT_PIXEL_SPREAD, direction);
            let mut main = Registers::new();
            let mut scratch = Material::default();
            shade(program, &scene.textures, &point, &mut main, &mut scratch);
            let source = scene.bump(MaterialId(0)).expect("the preset bumps");
            let bumped = tequila_render_core::surface_bump::bumped_normal(source, &scene, &hit, &point, shading, geometric, &main, &mut Registers::new(), &mut Material::default());
            let surface = Surface { normal: shading, geometric, footprint: point.footprint, derivatives: scene.uv_derivatives(&hit), transform };
            // The heights bumped_normal differences, worked out as it works
            // them out.
            let n = shading;
            let derivatives = scene.uv_derivatives(&hit);
            let tangent = derivatives
                .and_then(|(dpdu, _)| (dpdu - n * n.dot(dpdu)).try_normalize())
                .unwrap_or_else(|| tequila_texture::TangentFrame::from_normal(n).tangent);
            let bitangent = n.cross(tangent);
            let step = (point.footprint * point.scale * 0.1).max(1e-6);
            let restricted = program.restricted_to(&[bump_param(program).expect("a bump")]);
            let register = restricted.driven[0].1;
            let height = |offset: Vec3| {
                let (du, dv) = match derivatives {
                    Some((dpdu, dpdv)) => {
                        let (a, b, c) = (dpdu.dot(dpdu), dpdu.dot(dpdv), dpdv.dot(dpdv));
                        let det = a * c - b * b;
                        if det.abs() < 1e-20 {
                            (0.0, 0.0)
                        } else {
                            let (pu, pv) = (dpdu.dot(offset), dpdv.dot(offset));
                            ((c * pu - b * pv) / det, (a * pv - b * pu) / det)
                        }
                    }
                    None => (0.0, 0.0),
                };
                let moved = SurfacePoint { position: point.position + transform.inv_vector(offset), uv: point.uv + vec2(du, dv), ..point };
                let mut registers = Registers::new();
                shade(&restricted, &TextureTable::new(), &moved, &mut registers, &mut Material::default());
                registers.value(register).as_float()
            };
            let heights = [height(tangent * step), height(bitangent * step)];
            cases.push(Case { label: name.clone(), program: program.clone(), point, surface, normal: Some(bumped), heights: Some(heights) });
        }
    }
    eprintln!("bumped normals: {count} bumping presets");
    let mut outside = 0;
    for (i, batch) in cases.chunks(4096).enumerate() {
        outside += compare(&h, &format!("bumped normals, batch {i}"), batch, Bound { absolute: 1e-4, relative: 1e-4 }, Some(Bound { absolute: 1e-3, relative: 0.0 }));
    }
    eprintln!("bumped normals: {} points, {outside} outside the bound", cases.len());
    within_budget("bumped normals", outside, cases.len());
}
