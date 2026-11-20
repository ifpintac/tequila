//! The material-graph nodes ported after stage 1 (`docs/07` R3 tier A, the
//! procedural nodes one family per pull request) against the processor:
//! Tequila's noise primitives (`noise.rs`) one by one, then each node at
//! random inputs over the ranges presets use, through the whole path a
//! program takes - `program::lower_with`, the interpreter, the write-back -
//! against `shade.rs`'s `shade`; the library's presets the card now runs, at
//! random shading points; and their bumps, against `surface_bump.rs`'s
//! `bumped_normal`.
//!
//! Each comparison says how many items the card had to the bit and how many
//! were outside the bound; at most one item in a thousand may be outside it
//! (a cell boundary or a comparison a last-bit difference puts the other
//! side), and each such item is printed.

use std::sync::Arc;
use tequila_core::ids::{InstanceId, MaterialId, MeshId};
use tequila_core::math::{vec2, vec3, Mat4, Transform, Vec3};
use tequila_core::rng::Pcg32;
use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{CompiledGraph, Instruction, Register, Requirements};
use tequila_materials::material_type::Param;
use tequila_materials::openpbr::Material;
use tequila_render_core::shade::{shade, Registers, TextureTable};
use tequila_render_gpu::harness::Harness;
use tequila_render_gpu::material::GpuMaterial;
use tequila_render_gpu::program::{self, CardProgram, ProgramTable, Refusal};
use tequila_render_gpu::textures::{CardPictures, Pictures};
use tequila_texture::noise::{self, Basis, Fractal, Metric};
use tequila_texture::procedural::{Axis, Procedural};
use tequila_texture::projection::{Mapping, PointWorld, Projection, RayKind, SurfacePoint};

// ---- The harness ----------------------------------------------------------

/// Words an item gives back: the material (40 floats), the bump's height
/// here, the Tangent drawn (world space), the shading normal, the heights a
/// step along.
const OUT: usize = 49;
/// Words an item takes: graphs.rs's, then the point's curvature, occlusion,
/// closeness and vertex colour.
const IN: usize = 137;

#[derive(Clone, Copy)]
struct Surface {
    normal: Vec3,
    geometric: Vec3,
    footprint: f32,
    derivatives: Option<(Vec3, Vec3)>,
    transform: Transform,
    tangent: Option<(Vec3, f32)>,
}

struct Case {
    program: CompiledGraph,
    point: SurfacePoint,
    surface: Surface,
    normal: Option<Vec3>,
    heights: Option<[f32; 2]>,
    label: String,
    /// Added to each word's absolute bound: how far the processor's own
    /// answer moves for an ulp of the point, where a case measures it.
    slack: Option<Box<[f32; OUT]>>,
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
    var b: array<f32, 48>;
    for (var k = 0u; k < 40u; k++) {{
        b[k] = inf(83u + k);
    }}
    pt.footprint = inf(123u);
    pt.uv_footprint = vec2<f32>(inf(124u), inf(125u));
    pt.has_uv_footprint = inp(126u);
    s.mesh_tangent = vec4<f32>(inf(127u), inf(128u), inf(129u), inf(130u));
    pt.curvature = inf(131u);
    pt.occlusion = inf(132u);
    pt.closeness = inf(133u);
    pt.vertex_color = {};
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
        v3(134),
    )
}

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

/// No pictures, but the header's tables (the correctly rounded powf reads
/// them), as a scene with none packs them.
fn no_pictures() -> CardPictures {
    let mut card = CardPictures::default();
    card.finish();
    card
}

fn card(harness: &Harness, cases: &[Case], lowered: &[CardProgram], pictures: &CardPictures) -> Vec<[f32; OUT]> {
    let table = ProgramTable::new(&lowered.iter().cloned().map(Some).collect::<Vec<_>>());
    let mut inputs: Vec<u32> = Vec::with_capacity(cases.len() * IN);
    for (case, h) in cases.iter().zip(&table.headers) {
        inputs.extend_from_slice(&[h.main_start, h.main_end, h.bump_start, h.bump_end, h.constants, h.writes_start, h.writes_count, h.bump_here, h.bump_height, h.tangent, h.flags]);
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
        let (uvf, has) = match p.uv_footprint {
            Some(f) => (f, 1u32),
            None => ([0.0, 0.0], 0),
        };
        f.extend_from_slice(&[p.footprint, uvf[0], uvf[1]]);
        inputs.extend(f.iter().map(|v| v.to_bits()));
        inputs.push(has);
        let t = s.tangent.map_or([0.0; 4], |(t, sign)| [t.x, t.y, t.z, sign]);
        inputs.extend(t.iter().map(|v| v.to_bits()));
        let c = p.vertex_color;
        let f = [p.curvature, p.occlusion, w.closeness, c.r, c.g, c.b];
        inputs.extend(f.iter().map(|v| v.to_bits()));
    }
    assert_eq!(inputs.len(), cases.len() * IN);
    let words = harness.run_wgsl_with(
        &["program"],
        &kernel_body(),
        &inputs,
        IN,
        OUT,
        &[(11, &table.words), (12, bytemuck::cast_slice(&table.constants)), (16, &pictures.texels), (17, &pictures.header)],
    );
    words.chunks(OUT).map(|c| std::array::from_fn(|k| f32::from_bits(c[k]))).collect()
}

#[derive(Clone, Copy)]
struct Bound {
    absolute: f32,
    relative: f32,
}

const NOISE: Bound = Bound { absolute: 1e-4, relative: 1e-4 };
/// The bump's heights a step along the surface. Looser than graphs.rs's
/// (2e-9): the card moves the point through its rows of the inverse
/// (prog_shade, stage 1) where the processor takes `Transform::inv_vector`,
/// an ulp apart now and then, and the patterns here are fine - a wave of
/// 9,600 bands a unit moves 2e-4 of a band for an ulp of a point 0.3 out,
/// which is 1e-8 of a height 1e-4 deep. The height at the point itself,
/// where both read the same point, is held to the noise bound.
const HEIGHTS: Bound = Bound { absolute: 1e-7, relative: 1e-4 };

fn close(card: f32, processor: f32, bound: Bound) -> bool {
    if card.is_nan() && processor.is_nan() {
        return true;
    }
    if card.is_infinite() || processor.is_infinite() {
        return card == processor;
    }
    (card - processor).abs() <= bound.absolute + bound.relative * processor.abs()
}

/// What a comparison found: items, items to the bit, items outside.
struct Found {
    items: usize,
    exact: usize,
    outside: usize,
}

/// Every case on both sides; each word within `bound` but for at most one
/// item in a thousand (checked by the caller).
fn compare(harness: &Harness, name: &str, cases: &[Case], bound: Bound, normal_bound: Option<Bound>) -> Found {
    let pictures = no_pictures();
    let lowered: Vec<CardProgram> = cases
        .iter()
        .map(|c| {
            program::lower_with(&c.program, Pictures::On(&pictures))
                .unwrap_or_else(|why| panic!("{name}: {} is refused: {why}", c.label))
                .expect("a program to run")
        })
        .collect();
    let on_card = card(harness, cases, &lowered, &pictures);
    let mut outside = Vec::new();
    let mut exact = 0;
    for (i, case) in cases.iter().enumerate() {
        let want = processor(case);
        let got = on_card[i];
        let mut worst: Option<(usize, f32, f32)> = None;
        let mut same = true;
        for k in 0..OUT {
            if k == 40 && case.normal.is_some() && case.heights.is_none() {
                continue;
            }
            let b = if k >= 47 {
                match case.heights {
                    Some(_) => HEIGHTS,
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
            if got[k].to_bits() != want[k].to_bits() && !(got[k] == 0.0 && want[k] == 0.0) {
                same = false;
            }
            let b = Bound { absolute: b.absolute + case.slack.as_ref().map_or(0.0, |s| s[k]), ..b };
            if !close(got[k], want[k], b) {
                let d = (got[k] - want[k]).abs();
                if worst.is_none_or(|(_, g, w)| d > (g - w).abs()) {
                    worst = Some((k, got[k], want[k]));
                }
            }
        }
        exact += same as usize;
        if !same && std::env::var("TEQUILA_NODES_DEBUG").is_ok() && exact + outside.len() < 1_000_000 {
            let k = (0..40).find(|&k| got[k].to_bits() != want[k].to_bits()).unwrap_or(0);
            eprintln!("  not to the bit: {} word {k}: card {:e} ({:08x}), processor {:e} ({:08x})", case.label, got[k], got[k].to_bits(), want[k], want[k].to_bits());
        }
        if let Some((k, g, w)) = worst {
            outside.push(format!("  {} word {k}: card {g}, processor {w}", case.label));
        }
    }
    eprintln!("{name}: {} items, {exact} to the bit, {} outside the bound", cases.len(), outside.len());
    for line in outside.iter().take(12) {
        eprintln!("{line}");
    }
    Found { items: cases.len(), exact, outside: outside.len() }
}

fn within_budget(name: &str, outside: usize, total: usize) {
    assert!(outside * 1000 <= total, "{name}: {outside} of {total} items outside the bound, more than one in a thousand");
}

fn compare_all(harness: &Harness, name: &str, cases: &[Case], bound: Bound) {
    let mut found = Found { items: 0, exact: 0, outside: 0 };
    for (i, batch) in cases.chunks(4096).enumerate() {
        let f = compare(harness, &format!("{name}, batch {i}"), batch, bound, None);
        found.items += f.items;
        found.exact += f.exact;
        found.outside += f.outside;
    }
    eprintln!("{name}: {} items, {} to the bit ({:.2}%), {} outside the bound", found.items, found.exact, 100.0 * found.exact as f64 / found.items.max(1) as f64, found.outside);
    within_budget(name, found.outside, found.items);
}

// ---- Random inputs --------------------------------------------------------

fn uniform(rng: &mut Pcg32, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * rng.next_f32()
}

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

fn unit(rng: &mut Pcg32) -> Vec3 {
    loop {
        let v = vec3(uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0));
        let l = v.length();
        if l > 0.1 && l <= 1.0 {
            return v * (1.0 / l);
        }
    }
}

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
        curvature: uniform(rng, -500.0, 500.0),
        occlusion: uniform(rng, -0.1, 1.1),
        vertex_color: Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() },
        world: PointWorld {
            position: vec3(uniform(rng, -reach, reach), uniform(rng, -reach, reach), uniform(rng, -reach, reach)),
            normal,
            true_normal: if rng.next_f32() < 0.5 { normal } else { unit(rng) },
            incoming: unit(rng),
            backfacing: rng.next_f32() < 0.2,
            generated: vec3(rng.next_f32(), rng.next_f32(), rng.next_f32()),
            object_location: vec3(wide(rng), wide(rng), wide(rng)),
            object_random: rng.next_f32(),
            closeness: uniform(rng, -0.1, 1.1),
            object_color: Spectrum { r: rng.next_f32(), g: rng.next_f32(), b: rng.next_f32() },
            object_number: (rng.next_f32() * 10.0).floor(),
            ray: rays[(rng.next_f32() * 3.0) as usize % 3],
        },
    }
}

fn flat_surface(point: &SurfacePoint) -> Surface {
    Surface { normal: point.world.normal, geometric: point.world.true_normal, footprint: point.footprint, derivatives: None, transform: Transform::IDENTITY, tangent: None }
}

/// A program of the instructions `instructions`, register `value` driving
/// a number (the specular roughness) and, where given, `colour` a colour.
fn program_of(instructions: Vec<Instruction>, registers: usize, value: Option<Register>, colour: Option<Register>) -> CompiledGraph {
    let mut driven = Vec::new();
    if let Some(r) = value {
        driven.push((Param::SpecularRoughness, r));
    }
    if let Some(r) = colour {
        driven.push((Param::BaseColor, r));
    }
    driven.sort_by_key(|(p, _)| p.key());
    CompiledGraph {
        decals: Vec::new(),
        instructions,
        registers,
        constants: Material::default(),
        driven,
        requirements: Requirements::default(),
        topology: 0,
        layer: None,
    }
}

fn harness() -> Option<Harness> {
    Harness::for_test()
}

/// A mapping a procedural is read through: planar, through the UVs,
/// cylindrical or tri-planar, its controls at random.
fn random_mapping(rng: &mut Pcg32) -> Mapping {
    let projection = [Projection::Planar, Projection::Uv, Projection::Cylindrical, Projection::TriPlanar][(rng.next_u32() % 4) as usize];
    let mut m = Mapping { projection, axis: [Axis::X, Axis::Y, Axis::Z][(rng.next_u32() % 3) as usize], ..Mapping::default() };
    m.scale = vec3(uniform(rng, 0.3, 8.0), uniform(rng, 0.3, 8.0), 1.0);
    m.offset = vec2(uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0));
    if rng.next_f32() < 0.5 {
        m.rotation = uniform(rng, -0.5, 0.5);
        m.pivot = vec2(uniform(rng, -1.0, 1.0), uniform(rng, -1.0, 1.0));
    }
    m
}

// ---- Tequila's noise, one primitive at a time -----------------------------

/// `procedural.wgsl`'s primitives against `noise.rs`'s: gradient and value
/// noise, fBm, turbulence and ridged of either, Worley's cells by every
/// metric, at points up to a thousand out, to the bit.
#[test]
fn tequilas_noise_is_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(21, 1);
    // An item: the point, the seed, the fractal's settings word (or 0xFFFF_FFF0
    // + metric for Worley), its lacunarity and gain.
    let mut inputs: Vec<u32> = Vec::new();
    let mut want: Vec<[f32; 3]> = Vec::new();
    let mut labels = Vec::new();
    for i in 0..60_000 {
        let p = if i % 3 == 0 { vec3(wide(&mut rng), wide(&mut rng), wide(&mut rng)) } else { vec3(uniform(&mut rng, -5.0, 5.0), uniform(&mut rng, -5.0, 5.0), uniform(&mut rng, -5.0, 5.0)) * uniform(&mut rng, 1.0, 400.0) };
        let seed = if rng.next_f32() < 0.2 { rng.next_u32() % 16 } else { rng.next_u32() };
        let kind = rng.next_u32() % 4;
        let (settings, lacunarity, gain, w, label) = if kind == 3 {
            let metric = rng.next_u32() % 3;
            let m = [Metric::Euclidean, Metric::Manhattan, Metric::Chebyshev][metric as usize];
            let c = noise::worley(p, seed, m);
            (0xFFFF_FFF0 | metric, 0.0, 0.0, [c.f1, c.f2, c.id], format!("worley {m:?} at {p:?} seed {seed}"))
        } else {
            let basis = if rng.next_f32() < 0.5 { Basis::Perlin } else { Basis::Value };
            let octaves = if rng.next_f32() < 0.3 { 1 } else { 1 + rng.next_u32() % 8 };
            let fractal = Fractal { octaves, lacunarity: uniform(&mut rng, 1.5, 3.0), gain: uniform(&mut rng, 0.3, 0.7), basis };
            let v = match kind {
                0 => noise::fbm(p, seed, fractal),
                1 => noise::turbulence(p, seed, fractal),
                _ => noise::ridged(p, seed, fractal),
            };
            // One octave of fBm is the basis itself.
            if kind == 0 && octaves == 1 {
                let b = match basis {
                    Basis::Perlin => noise::perlin(p, seed),
                    Basis::Value => noise::value(p, seed),
                };
                assert_eq!(b.to_bits(), v.to_bits(), "one octave of fBm is its basis");
            }
            (program::settings::fractal(kind, basis, false, octaves), fractal.lacunarity, fractal.gain, [v, 0.0, 0.0], format!("fractal {kind} {fractal:?} at {p:?} seed {seed}"))
        };
        inputs.extend_from_slice(&[p.x.to_bits(), p.y.to_bits(), p.z.to_bits(), seed, settings, lacunarity.to_bits(), gain.to_bits(), 0]);
        want.push(w);
        labels.push(label);
    }
    let body = "
    cy_zero = inp(7u);
    let p = vec3<f32>(inf(0u), inf(1u), inf(2u));
    let seed = inp(3u);
    let settings = inp(4u);
    if ((settings & 0xFFFFFFF0u) == 0xFFFFFFF0u) {
        let c = tq_worley(p, seed, settings & 3u);
        outf(0u, c.x); outf(1u, c.y); outf(2u, c.z);
    } else {
        outf(0u, tq_fractal(settings, p, vec4<f32>(1.0, inf(5u), inf(6u), 0.0), seed));
        outf(1u, 0.0); outf(2u, 0.0);
    }";
    let got = h.run_wgsl(&["procedural"], body, &inputs, 8, 3);
    let (mut exact, mut outside) = (0, Vec::new());
    for (i, w) in want.iter().enumerate() {
        let g: [f32; 3] = std::array::from_fn(|k| f32::from_bits(got[3 * i + k]));
        if (0..3).all(|k| g[k].to_bits() == w[k].to_bits()) {
            exact += 1;
        } else if (0..3).any(|k| !close(g[k], w[k], Bound { absolute: 1e-6, relative: 1e-6 })) {
            outside.push(format!("  {}: card {g:?}, processor {w:?}", labels[i]));
        }
    }
    eprintln!("Tequila's noise: {} items, {exact} to the bit, {} outside 1e-6", want.len(), outside.len());
    for line in outside.iter().take(12) {
        eprintln!("{line}");
    }
    within_budget("Tequila's noise", outside.len(), want.len());
}

// ---- Tequila's procedurals ------------------------------------------------

/// A fractal's settings at random.
fn random_fractal(rng: &mut Pcg32) -> Fractal {
    Fractal {
        octaves: rng.next_u32() % 7,
        lacunarity: uniform(rng, 1.5, 3.0),
        gain: uniform(rng, 0.3, 0.7),
        basis: if rng.next_f32() < 0.5 { Basis::Perlin } else { Basis::Value },
    }
}

/// How many kinds of pattern the card runs (all but Wood (Advanced)).
const PATTERNS: usize = 16;

/// A pattern of each kind the card runs, its settings at random over the
/// ranges the library and the menu's defaults use.
fn random_procedural(rng: &mut Pcg32, which: usize) -> Procedural {
    use tequila_texture::procedural::{CellOutput, HoleShape, WeavePattern};
    let axis = [Axis::X, Axis::Y, Axis::Z][(rng.next_u32() % 3) as usize];
    let pick = |rng: &mut Pcg32, n: u32| (rng.next_u32() % n) as usize;
    match which {
        3 => Procedural::Noise { fractal: random_fractal(rng), frequency: uniform(rng, 0.5, 50.0) },
        4 => Procedural::Cellular {
            metric: [Metric::Euclidean, Metric::Manhattan, Metric::Chebyshev][pick(rng, 3)],
            output: [CellOutput::Distance, CellOutput::Edges, CellOutput::Id][pick(rng, 3)],
            frequency: uniform(rng, 0.5, 50.0),
        },
        5 => Procedural::Marble { axis, frequency: uniform(rng, 0.5, 20.0), distortion: uniform(rng, 0.0, 5.0), fractal: random_fractal(rng) },
        6 => Procedural::Wood { axis, frequency: uniform(rng, 1.0, 30.0), distortion: uniform(rng, 0.0, 1.0), grain: uniform(rng, 0.0, 1.0), fractal: random_fractal(rng) },
        7 => Procedural::Brushed { frequency: uniform(rng, 1.0, 200.0), stretch: uniform(rng, 0.5, 100.0) },
        8 => Procedural::NoiseTexture { basis: if rng.next_f32() < 0.5 { Basis::Perlin } else { Basis::Value }, frequency: uniform(rng, 1.0, 50.0), contrast: uniform(rng, -0.5, 4.0), bias: uniform(rng, -0.5, 0.5) },
        9 => Procedural::Ridged { fractal: random_fractal(rng), frequency: uniform(rng, 0.5, 20.0) },
        10 => Procedural::Granite { frequency: uniform(rng, 1.0, 30.0), speckle: uniform(rng, -0.1, 1.1), fine: uniform(rng, -0.1, 1.1) },
        11 => Procedural::Spots { frequency: uniform(rng, 1.0, 30.0), coverage: uniform(rng, -0.1, 1.1), variance: uniform(rng, -0.1, 1.1), softness: uniform(rng, -0.1, 1.1) },
        12 => Procedural::Camouflage { frequency: uniform(rng, 0.5, 10.0), layers: rng.next_u32() % 7 },
        13 => Procedural::Contour { axis, frequency: uniform(rng, 1.0, 30.0), width: uniform(rng, -0.1, 1.1), softness: uniform(rng, -0.1, 1.1), distortion: if rng.next_f32() < 0.3 { 0.0 } else { uniform(rng, 0.0, 2.0) } },
        14 => Procedural::Weave { pattern: [WeavePattern::Plain, WeavePattern::Twill, WeavePattern::Satin][pick(rng, 3)], frequency: uniform(rng, 1.0, 100.0), width: uniform(rng, 0.2, 1.1), variance: uniform(rng, -0.1, 1.1) },
        15 => Procedural::Mesh { shape: [HoleShape::Round, HoleShape::Square, HoleShape::Hex][pick(rng, 3)], frequency: uniform(rng, 1.0, 50.0), size: uniform(rng, -0.1, 1.1), softness: uniform(rng, -0.1, 1.1) },
        0 => Procedural::Scratches {
            frequency: if rng.next_f32() < 0.5 { 110.0 } else { uniform(rng, 1.0, 200.0) },
            width: uniform(rng, 0.002, 0.2),
            density: uniform(rng, -0.1, 1.1),
            spread: uniform(rng, -0.1, 0.6),
        },
        1 => Procedural::BrushedRadial {
            axis,
            frequency: if rng.next_f32() < 0.5 { 2600.0 } else { uniform(rng, 1.0, 3000.0) },
            stretch: uniform(rng, 1.0, 200.0),
        },
        _ => Procedural::Leather {
            frequency: if rng.next_f32() < 0.5 { 430.0 } else { uniform(rng, 1.0, 500.0) },
            crack_width: if rng.next_f32() < 0.1 { 0.0 } else { uniform(rng, 0.0, 0.3) },
            wrinkle: uniform(rng, -0.1, 1.1),
        },
    }
}

/// Every pattern the card runs, in the part's space and through a mapping,
/// against `shade`.
#[test]
fn tequilas_procedurals_are_the_processors() {
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(22, 1);
    let mut cases = Vec::new();
    for which in 0..PATTERNS {
        for i in 0..2400 {
            let procedural = random_procedural(&mut rng, which);
            let seed = if rng.next_f32() < 0.2 { rng.next_u32() % 16 } else { rng.next_u32() };
            let mapping = (i % 3 == 2).then(|| Box::new(random_mapping(&mut rng)));
            let label = format!("{procedural:?} seed {seed} mapping {:?}", mapping.as_ref().map(|m| m.projection));
            let program = program_of(vec![Instruction::Evaluate { dst: Register(0), procedural: Box::new(procedural), seed, mapping }], 1, Some(Register(0)), None);
            let reach = if rng.next_f32() < 0.8 { 0.15 } else { 1.0 };
            let point = random_point(&mut rng, reach);
            let label = format!("{label} at {:?} scale {}", point.position, point.scale);
            cases.push(Case { program, surface: flat_surface(&point), point, normal: None, heights: None, slack: None, label });
        }
    }
    compare_all(&h, "Tequila's procedurals", &cases, NOISE);
}

/// The patterns the card has no port of are refused by name, and stage 1's
/// lowering refuses every one as it did.
#[test]
fn unported_patterns_are_refused() {
    let pictures = no_pictures();
    for procedural in Procedural::catalog() {
        let program = program_of(vec![Instruction::Evaluate { dst: Register(0), procedural: Box::new(procedural), seed: 3, mapping: None }], 1, Some(Register(0)), None);
        assert_eq!(program::lower(&program), Err(Refusal::Instruction("a Tequila procedural")));
        let ported = !matches!(procedural, Procedural::WoodAdvanced { .. });
        match program::lower_with(&program, Pictures::On(&pictures)) {
            Ok(Some(_)) => assert!(ported, "{} runs on the card", procedural.label()),
            Err(Refusal::Procedural(name)) => assert!(!ported && name == procedural.label(), "{} is refused", procedural.label()),
            other => panic!("{}: {other:?}", procedural.label()),
        }
    }
}

// ---- The library ----------------------------------------------------------

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

/// The presets the card runs now and stage 1's lowering did not.
fn newly_running() -> Vec<(String, CompiledGraph)> {
    let pictures = no_pictures();
    library()
        .into_iter()
        .filter(|(_, p)| program::lower(p).is_err() && matches!(program::lower_with(p, Pictures::On(&pictures)), Ok(Some(_))))
        .collect()
}

/// How many of the library's presets the transport's lowering runs on the
/// card (its pictures none loaded: an image texture takes its fallback),
/// and why it refuses the others. Prints the tally; asserts nothing.
#[test]
fn the_librarys_tally() {
    let pictures = no_pictures();
    let presets = library();
    let mut refused: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let (mut runs, mut constants) = (0, 0);
    for (name, program) in &presets {
        match program::lower_with(program, Pictures::On(&pictures)) {
            Ok(Some(_)) => runs += 1,
            Ok(None) => constants += 1,
            Err(why) => {
                let key = match why {
                    Refusal::Node(n) => format!("the {n} node"),
                    Refusal::Param(p) => format!("driving {}", p.label()),
                    other => other.to_string(),
                };
                refused.entry(key).or_default().push(name.clone());
            }
        }
    }
    eprintln!("the library: {} presets with a program; the card runs {runs}, {constants} are their constants, {} refused", presets.len(), presets.len() - runs - constants);
    for (why, names) in &refused {
        eprintln!("  refused, {why}: {} ({})", names.len(), names.join(", "));
    }
}

/// Every preset the card runs now that stage 1's lowering refused, at
/// random shading points: the parameter block, the bump's height and the
/// Tangent, against `shade`.
#[test]
fn the_newly_running_presets_are_the_processors() {
    let Some(h) = harness() else { return };
    let presets = newly_running();
    eprintln!("{} presets newly on the card: {}", presets.len(), presets.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(", "));
    let mut rng = Pcg32::new(23, 1);
    let mut cases = Vec::new();
    for (name, program) in &presets {
        for _ in 0..512 {
            let reach = if rng.next_f32() < 0.8 { 0.15 } else { 1.0 };
            let mut point = random_point(&mut rng, reach);
            point.scale = 1.0;
            cases.push(Case { label: name.clone(), program: program.clone(), surface: flat_surface(&point), point, normal: None, heights: None, slack: None });
        }
    }
    compare_all(&h, "the newly running presets", &cases, NOISE);
}

// ---- The bump -------------------------------------------------------------

/// A triangle of a part placed turned, scaled and moved, with UVs, wearing
/// `program` (graphs.rs's).
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

/// The bumped normal of every newly running preset that bumps, at random
/// points of a placed triangle, against `bumped_normal`: the height here to
/// the noise bound, a step along to [`HEIGHTS`], the normal (forward differences over a tenth of a
/// footprint, which cancel) to 1e-3.
#[test]
fn the_newly_running_bumps_are_the_processors() {
    use tequila_render_core::intersect::Hit;
    let Some(h) = harness() else { return };
    let pictures = no_pictures();
    let presets = newly_running();
    let mut rng = Pcg32::new(24, 1);
    let mut cases = Vec::new();
    let mut count = 0;
    for (name, program) in &presets {
        let Ok(Some(card)) = program::lower_with(program, Pictures::On(&pictures)) else { continue };
        if !card.has_bump() {
            continue;
        }
        count += 1;
        for k in 0..96 {
            let angle = uniform(&mut rng, -3.0, 3.0);
            let axis = unit(&mut rng);
            let scale = if k % 2 == 0 { 1.0 } else { uniform(&mut rng, 0.2, 3.0) };
            // Placed within a product's reach (graphs.rs places it up to ten
            // out): a wave of 8,400 bands a unit, distorted, read ten units
            // out is its rounding's on the processor too.
            let matrix = Mat4::translation(vec3(uniform(&mut rng, -0.15, 0.15), 0.1, -0.2)) * Mat4::rotation(axis, angle) * Mat4::scale(Vec3::splat(scale));
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
            let surface = Surface { normal: shading, geometric, footprint: point.footprint, derivatives: scene.uv_derivatives(&hit), transform, tangent: scene.tangent_frame(&hit) };
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
            // How far the processor's own answers move for an ulp of the
            // point: the parameters here, and the heights a step along - the
            // card moves its point through its own rows of the inverse
            // (prog_shade, stage 1), an ulp from Transform::inv_vector now
            // and then, and over a step of a few hundred ulps a fine
            // pattern's forward differences are only that good.
            let p = point.position;
            let nudge = vec3(p.x.abs().max(1e-3), p.y.abs().max(1e-3), p.z.abs().max(1e-3)) * f32::EPSILON;
            let nudged = |offset: Vec3, by: Vec3| {
                let moved = SurfacePoint { position: point.position + by, ..point };
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
                let moved = SurfacePoint { position: moved.position + transform.inv_vector(offset), uv: moved.uv + vec2(du, dv), ..moved };
                let mut registers = Registers::new();
                shade(&restricted, &TextureTable::new(), &moved, &mut registers, &mut Material::default());
                registers.value(register).as_float()
            };
            let moved_by = [(nudged(tangent * step, nudge) - heights[0]).abs(), (nudged(bitangent * step, nudge) - heights[1]).abs()];
            let mut slack = [0.0f32; OUT];
            let at_nudge = SurfacePoint { position: point.position + nudge, ..point };
            let mine = floats(&GpuMaterial::from_material(shade(program, &scene.textures, &point, &mut Registers::new(), &mut Material::default())));
            let theirs = floats(&GpuMaterial::from_material(shade(program, &scene.textures, &at_nudge, &mut Registers::new(), &mut Material::default())));
            for k in 0..40 {
                slack[k] = 2.0 * (mine[k] - theirs[k]).abs();
            }
            let normal_slack = 2.0 * (moved_by[0] + moved_by[1]) / step;
            for k in 44..47 {
                slack[k] = normal_slack;
            }
            slack[47] = 2.0 * moved_by[0];
            slack[48] = 2.0 * moved_by[1];
            cases.push(Case { label: name.clone(), program: program.clone(), point, surface, normal: Some(bumped), heights: Some(heights), slack: Some(Box::new(slack)) });
        }
    }
    eprintln!("bumped normals: {count} newly running presets that bump");
    let mut found = Found { items: 0, exact: 0, outside: 0 };
    for (i, batch) in cases.chunks(4096).enumerate() {
        let f = compare(&h, &format!("bumped normals, batch {i}"), batch, NOISE, Some(Bound { absolute: 1e-3, relative: 0.0 }));
        found.items += f.items;
        found.exact += f.exact;
        found.outside += f.outside;
    }
    eprintln!("bumped normals: {} points, {} to the bit, {} outside the bound", found.items, found.exact, found.outside);
    within_budget("bumped normals", found.outside, found.items);
}


// ---- Cycles' Wave texture -------------------------------------------------

/// A program of one node, its inputs literals, each output read wired to a
/// parameter of its kind (graphs.rs's).
fn single_node(node: tequila_materials::nodes::ShaderNode, inputs: &[tequila_materials::graph::Value], outputs: &[u8]) -> CompiledGraph {
    use tequila_materials::graph::{NodeKind, SocketType, Value};
    const FLOATS: [Param; 3] = [Param::SpecularRoughness, Param::CoatRoughness, Param::FuzzRoughness];
    const COLOURS: [Param; 3] = [Param::BaseColor, Param::SpecularColor, Param::CoatColor];
    let kind = NodeKind::Node(node);
    let n = inputs.len();
    let mut instructions: Vec<Instruction> = inputs.iter().enumerate().map(|(i, v): (usize, &Value)| Instruction::Load { dst: Register(i as u16), value: *v }).collect();
    let mut driven = Vec::new();
    let (mut f, mut c) = (0, 0);
    for (k, output) in outputs.iter().enumerate() {
        let dst = Register((n + k) as u16);
        instructions.push(Instruction::Apply { dst, kind: Box::new(kind.clone()), args: (0..n).map(|i| Register(i as u16)).collect(), output: *output });
        let param = match kind.output_type_at(*output) {
            Some(SocketType::Float) | None => {
                f += 1;
                FLOATS[f - 1]
            }
            _ => {
                c += 1;
                COLOURS[c - 1]
            }
        };
        driven.push((param, dst));
    }
    driven.sort_by_key(|(p, _)| p.key());
    CompiledGraph { decals: Vec::new(), instructions, registers: n + outputs.len(), constants: Material::default(), driven, requirements: Requirements::default(), topology: 0, layer: None }
}

/// The Wave Texture by every kind, direction and profile, at random inputs
/// over their ranges: the coordinates and scale kept where the bands number
/// under ten thousand, and only where the processor's own answer moves by
/// less than 1e-5 when the coordinates move by an ulp. Elsewhere it is its
/// rounding's: a distorted wave of many rough octaves reads its noise at
/// coordinates of thousands, where the Noise Texture's port (stage 1) is
/// good to 1e-4 rather than to the bit, and the distortion multiplies that.
#[test]
fn wave_is_the_processors() {
    use tequila_materials::graph::Value;
    use tequila_materials::nodes::ShaderNode;
    use tequila_texture::cycles::textures::{WaveDirection, WaveKind, WaveProfile};
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(25, 1);
    let mut cases = Vec::new();
    for kind in [WaveKind::Bands, WaveKind::Rings] {
        for direction in [WaveDirection::X, WaveDirection::Y, WaveDirection::Z, WaveDirection::Diagonal] {
            for profile in [WaveProfile::Sine, WaveProfile::Saw, WaveProfile::Triangle] {
                for outputs in [&[0u8][..], &[1], &[0, 1]] {
                    for _ in 0..300 {
                        let node = ShaderNode::Wave { kind, direction, profile };
                        let scale = if rng.next_f32() < 0.3 { uniform(&mut rng, 0.0, 200.0) } else { uniform(&mut rng, 0.0, 30.0) };
                        let reach = 8.0 / scale.max(0.5);
                        let vector = vec3(uniform(&mut rng, -reach, reach), uniform(&mut rng, -reach, reach), uniform(&mut rng, -reach, reach));
                        let distortion = if rng.next_f32() < 0.3 { 0.0 } else { uniform(&mut rng, -10.0, 10.0) };
                        let inputs = vec![
                            Value::Vector(vector),
                            Value::Float(scale),
                            Value::Float(distortion),
                            Value::Float(uniform(&mut rng, 0.0, 15.0)),
                            Value::Float(uniform(&mut rng, 0.0, 10.0)),
                            Value::Float(uniform(&mut rng, 0.0, 1.0)),
                            Value::Float(uniform(&mut rng, -10.0, 10.0)),
                        ];
                        let wave = |v: Vec3| node.evaluate(&[Value::Vector(v), inputs[1], inputs[2], inputs[3], inputs[4], inputs[5], inputs[6]], 1).as_float();
                        let nudged = vector * (1.0 + f32::EPSILON);
                        if (wave(vector) - wave(nudged)).abs() > 1e-5 {
                            continue;
                        }
                        let point = random_point(&mut rng, 1.0);
                        cases.push(Case { label: format!("{node:?} {outputs:?} at {inputs:?}"), program: single_node(node, &inputs, outputs), surface: flat_surface(&point), point, normal: None, heights: None, slack: None });
                    }
                }
            }
        }
    }
    compare_all(&h, "Wave", &cases, NOISE);
}

/// Stage 1's lowering refuses the Wave Texture as it did; the transport's
/// refuses a distorted Wave whose detail or roughness is a constant the
/// Noise Texture's fBm would clamp (Wave's own does not).
#[test]
fn wave_refusals() {
    use tequila_materials::graph::Value;
    use tequila_materials::nodes::ShaderNode;
    use tequila_texture::cycles::textures::{WaveDirection, WaveKind, WaveProfile};
    let pictures = no_pictures();
    let node = ShaderNode::Wave { kind: WaveKind::Bands, direction: WaveDirection::X, profile: WaveProfile::Sine };
    let inputs = |distortion: f32, detail: f32, roughness: f32| {
        vec![Value::Vector(Vec3::ONE), Value::Float(5.0), Value::Float(distortion), Value::Float(detail), Value::Float(1.0), Value::Float(roughness), Value::Float(0.0)]
    };
    let lowered = |i: Vec<Value>| program::lower_with(&single_node(node, &i, &[1]), Pictures::On(&pictures)).map(|p| p.is_some());
    assert_eq!(program::lower(&single_node(node, &inputs(1.0, 2.0, 0.5), &[1])), Err(Refusal::Node("Wave Texture")));
    assert_eq!(lowered(inputs(1.0, 2.0, 0.5)), Ok(true));
    assert_eq!(lowered(inputs(0.0, 20.0, -1.0)), Ok(true), "undistorted, the fBm is not read");
    assert!(lowered(inputs(1.0, 20.0, 0.5)).is_err());
    assert!(lowered(inputs(1.0, -1.0, 0.5)).is_err());
    assert!(lowered(inputs(1.0, 2.0, -0.5)).is_err());
}

// ---- What the snapshot measured at the mesh's corners ---------------------

/// The curvature by every mode, the baked occlusion, the distance to other
/// parts and the vertex colour, at random points (their measures over and
/// past their ranges) and settings, against `shade`.
#[test]
fn the_measures_are_the_processors() {
    use tequila_materials::graph::CurvatureMode;
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(26, 1);
    let mut cases = Vec::new();
    for i in 0..12_000 {
        let (instruction, colour) = match i % 6 {
            0..=2 => {
                let mode = CurvatureMode::ALL[i % 6];
                let radius = if rng.next_f32() < 0.05 { -0.001 } else { uniform(&mut rng, 0.0, 0.01) };
                let contrast = if rng.next_f32() < 0.3 { 1.0 } else { uniform(&mut rng, -0.5, 3.0) };
                (Instruction::Curvature { dst: Register(0), mode, radius, contrast }, false)
            }
            3 => (Instruction::Occlusion { dst: Register(0), contrast: if rng.next_f32() < 0.3 { 1.0 } else { uniform(&mut rng, 0.0, 4.0) } }, false),
            4 => (Instruction::Distance { dst: Register(0) }, false),
            _ => (Instruction::VertexColor { dst: Register(0) }, true),
        };
        let label = format!("{instruction:?}");
        let program = if colour { program_of(vec![instruction], 1, None, Some(Register(0))) } else { program_of(vec![instruction], 1, Some(Register(0)), None) };
        let mut point = random_point(&mut rng, 1.0);
        // Exact values at the ends now and then.
        match rng.next_u32() % 8 {
            0 => point.occlusion = 0.0,
            1 => point.occlusion = 1.0,
            2 => point.curvature = 0.0,
            3 => point.world.closeness = 1.0,
            _ => {}
        }
        let label = format!("{label} at curvature {} occlusion {} closeness {}", point.curvature, point.occlusion, point.world.closeness);
        cases.push(Case { program, surface: flat_surface(&point), point, normal: None, heights: None, slack: None, label });
    }
    compare_all(&h, "The measures", &cases, Bound { absolute: 1e-6, relative: 1e-6 });
}

/// Stage 1's lowering refuses the measures as it did; the transport's runs
/// them.
#[test]
fn the_measures_are_refused_only_by_stage_1() {
    use tequila_materials::graph::CurvatureMode;
    let pictures = no_pictures();
    for (instruction, why) in [
        (Instruction::Curvature { dst: Register(0), mode: CurvatureMode::Convex, radius: 0.002, contrast: 1.0 }, "the curvature"),
        (Instruction::Occlusion { dst: Register(0), contrast: 1.0 }, "the baked occlusion"),
        (Instruction::Distance { dst: Register(0) }, "the distance to other parts"),
        (Instruction::VertexColor { dst: Register(0) }, "vertex colours"),
    ] {
        let program = program_of(vec![instruction], 1, Some(Register(0)), None);
        assert_eq!(program::lower(&program), Err(Refusal::Instruction(why)));
        assert!(matches!(program::lower_with(&program, Pictures::On(&pictures)), Ok(Some(_))), "{why} runs on the card");
        assert!(program::reads_attributes(&program));
    }
}

// ---- Cycles' Gabor texture ------------------------------------------------

/// The Gabor Texture in a plane and in space, at random inputs over their
/// ranges, every output.
#[test]
fn gabor_is_the_processors() {
    use tequila_materials::graph::Value;
    use tequila_materials::nodes::ShaderNode;
    use tequila_texture::cycles::textures::GaborKind;
    let Some(h) = harness() else { return };
    let mut rng = Pcg32::new(27, 1);
    let mut cases = Vec::new();
    for kind in [GaborKind::Plane, GaborKind::Space] {
        for outputs in [&[0u8][..], &[1], &[2], &[0, 1, 2]] {
            for _ in 0..1500 {
                let node = ShaderNode::Gabor { kind };
                let scale = if rng.next_f32() < 0.3 { uniform(&mut rng, 0.0, 100.0) } else { uniform(&mut rng, 0.0, 20.0) };
                let reach = 20.0 / scale.max(0.5);
                let vector = vec3(uniform(&mut rng, -reach, reach), uniform(&mut rng, -reach, reach), uniform(&mut rng, -reach, reach));
                let anisotropy = match rng.next_u32() % 5 {
                    0 => 0.0,
                    1 => 1.0,
                    _ => uniform(&mut rng, -0.1, 1.1),
                };
                let inputs = vec![
                    Value::Vector(vector),
                    Value::Float(scale),
                    Value::Float(uniform(&mut rng, 0.0, 20.0)),
                    Value::Float(anisotropy),
                    Value::Float(uniform(&mut rng, -3.2, 3.2)),
                    Value::Vector(unit(&mut rng) * uniform(&mut rng, 0.2, 3.0)),
                ];
                let point = random_point(&mut rng, 1.0);
                cases.push(Case { label: format!("{node:?} {outputs:?} at {inputs:?}"), program: single_node(node, &inputs, outputs), surface: flat_surface(&point), point, normal: None, heights: None, slack: None });
            }
        }
    }
    compare_all(&h, "Gabor", &cases, NOISE);
    // Stage 1's lowering refuses it as it did.
    let any = single_node(ShaderNode::Gabor { kind: GaborKind::Space }, &[Value::Vector(Vec3::ONE), Value::Float(5.0), Value::Float(2.0), Value::Float(1.0), Value::Float(0.5), Value::Vector(Vec3::X)], &[0]);
    assert_eq!(program::lower(&any), Err(Refusal::Node("Gabor Texture")));
}
