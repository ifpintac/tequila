//! Material graphs as GLSL, so the GPU viewport shows a procedural material
//! as the renderer does.
//!
//! The Solid and Material Preview views read a material's parameters from
//! uniforms and, where a parameter comes straight from an image, from that
//! image. Everything the standard library is made of - noise, Voronoi, ramps
//! and the maths between them - was a flat colour there: a marble showed as
//! its ground and nothing else, a wood as a beige ball. Here a material's
//! compiled program is written out as a GLSL function, node for node, over
//! `procedural.glsl`'s port of the same Cycles algorithms the renderer runs,
//! so the viewport's patterns are the render's.
//!
//! What it cannot write - image lookups (which the view already does its own
//! way), Tequila's own procedurals, the Gabor texture - leaves the
//! parameters that depend on it to the view's constants, as before.

use tequila_materials::graph::{CompiledGraph, Instruction, NodeKind, Register, Value};
use tequila_materials::material_type::Param;
use tequila_materials::nodes::{MixData, PointInput, ShaderNode};
use tequila_texture::cycles::noise::FractalKind;
use tequila_texture::cycles::ops::{BlendMode, ColorModel, MapRangeKind, MappingKind, MathOp, VectorMathOp};
use tequila_texture::cycles::textures::{GradientKind, WaveDirection, WaveKind, WaveProfile};
use tequila_texture::cycles::voronoi::{VoronoiFeature, VoronoiMetric};
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::OnceLock;

/// The GLSL library, its fractal sums written out once per dimension.
pub fn library() -> &'static str {
    static LIBRARY: OnceLock<String> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let mut fractals = String::new();
        for (ty, seeds, offset) in [("float", "1.0, 2.0", "fs_seed_offset1"), ("vec2", "2.0, 3.0", "fs_seed_offset2"), ("vec3", "3.0, 4.0", "fs_seed_offset3"), ("vec4", "4.0, 5.0", "fs_seed_offset4")] {
            let (g, b) = seeds.split_once(", ").expect("two seeds");
            fractals.push_str(&FRACTAL_TEMPLATE.replace("{T}", ty).replace("{G}", g).replace("{B}", b).replace("{OFFSET}", offset));
        }
        include_str!("procedural.glsl").replace("@FRACTALS@", &fractals)
    })
}

/// Cycles' five fractal sums and the Noise texture over them, for a point
/// of type `{T}`.
const FRACTAL_TEMPLATE: &str = r#"
// fBM, filtered: an octave finer than the pixel (`footprint`, in the
// point's units) is replaced by its mean, zero, fading out between half a
// pixel and one, its share of the normalization kept. What the view shows
// is the render's pattern averaged over the pixel, as the render's own
// samples average it - and the octaves nobody can see cost nothing.
float fs_fbm({T} p, float detail, float roughness, float lacunarity, bool normalize, float footprint) {
    float fscale = 1.0, amp = 1.0, maxamp = 0.0, sum = 0.0;
    for (int i = 0; i <= int(detail); i++) {
        float keep = clamp(2.0 - 2.0 * fscale * footprint, 0.0, 1.0);
        if (keep > 0.0) sum += fs_snoise(fscale * p) * amp * keep;
        maxamp += amp;
        amp *= roughness;
        fscale *= lacunarity;
    }
    float rmd = detail - floor(detail);
    if (rmd != 0.0) {
        float keep = clamp(2.0 - 2.0 * fscale * footprint, 0.0, 1.0);
        float t = keep > 0.0 ? fs_snoise(fscale * p) * keep : 0.0;
        float sum2 = sum + t * amp;
        return normalize ? mix(0.5 * sum / maxamp + 0.5, 0.5 * sum2 / (maxamp + amp) + 0.5, rmd) : mix(sum, sum2, rmd);
    }
    return normalize ? 0.5 * sum / maxamp + 0.5 : sum;
}
float fs_multi_fractal({T} p, float detail, float roughness, float lacunarity) {
    float value = 1.0, pwr = 1.0;
    for (int i = 0; i <= int(detail); i++) {
        value *= pwr * fs_snoise(p) + 1.0;
        pwr *= roughness;
        p *= lacunarity;
    }
    float rmd = detail - floor(detail);
    if (rmd != 0.0) value *= rmd * pwr * fs_snoise(p) + 1.0;
    return value;
}
float fs_hetero_terrain({T} p, float detail, float roughness, float lacunarity, float offset) {
    float pwr = roughness;
    float value = offset + fs_snoise(p);
    p *= lacunarity;
    for (int i = 1; i <= int(detail); i++) {
        float increment = (fs_snoise(p) + offset) * pwr * value;
        value += increment;
        pwr *= roughness;
        p *= lacunarity;
    }
    float rmd = detail - floor(detail);
    if (rmd != 0.0) value += rmd * ((fs_snoise(p) + offset) * pwr * value);
    return value;
}
float fs_hybrid_multi_fractal({T} p, float detail, float roughness, float lacunarity, float offset, float gain) {
    float pwr = 1.0, value = 0.0, weight = 1.0;
    int i = 0;
    while (weight > 0.001 && i <= int(detail)) {
        weight = min(weight, 1.0);
        float signal = (fs_snoise(p) + offset) * pwr;
        pwr *= roughness;
        value += weight * signal;
        weight *= gain * signal;
        p *= lacunarity;
        i++;
    }
    float rmd = detail - floor(detail);
    if (rmd != 0.0 && weight > 0.001) {
        weight = min(weight, 1.0);
        value += rmd * weight * ((fs_snoise(p) + offset) * pwr);
    }
    return value;
}
float fs_ridged_multi_fractal({T} p, float detail, float roughness, float lacunarity, float offset, float gain) {
    float pwr = roughness;
    float signal = offset - abs(fs_snoise(p));
    signal *= signal;
    float value = signal;
    for (int i = 1; i <= int(detail); i++) {
        p *= lacunarity;
        float weight = clamp(signal * gain, 0.0, 1.0);
        signal = offset - abs(fs_snoise(p));
        signal *= signal;
        signal *= weight;
        value += signal * pwr;
        pwr *= roughness;
    }
    return value;
}
// kind: 0 multifractal, 1 fBM, 2 hybrid, 3 ridged, 4 hetero terrain.
float fs_noise_select({T} p, int kind, float detail, float roughness, float lacunarity, float offset, float gain, bool normalize, float footprint) {
    float d = clamp(detail, 0.0, 15.0);
    float r = max(roughness, 0.0);
    if (kind == 0) return fs_multi_fractal(p, d, r, lacunarity);
    if (kind == 1) return fs_fbm(p, d, r, lacunarity, normalize, footprint);
    if (kind == 2) return fs_hybrid_multi_fractal(p, d, r, lacunarity, offset, gain);
    if (kind == 3) return fs_ridged_multi_fractal(p, d, r, lacunarity, offset, gain);
    return fs_hetero_terrain(p, d, r, lacunarity, offset);
}
float fs_noise_fac({T} p, int kind, float detail, float roughness, float lacunarity, float offset, float gain, float distortion, bool normalize) {
    if (distortion != 0.0) p = fs_distort(p, distortion);
    float footprint = fs_footprint(p);
    return fs_noise_select(p, kind, detail, roughness, lacunarity, offset, gain, normalize, footprint);
}
vec3 fs_noise_color({T} p, int kind, float detail, float roughness, float lacunarity, float offset, float gain, float distortion, bool normalize) {
    if (distortion != 0.0) p = fs_distort(p, distortion);
    float footprint = fs_footprint(p);
    float value = fs_noise_select(p, kind, detail, roughness, lacunarity, offset, gain, normalize, footprint);
    return vec3(value,
        fs_noise_select(p + {OFFSET}({G}), kind, detail, roughness, lacunarity, offset, gain, normalize, footprint),
        fs_noise_select(p + {OFFSET}({B}), kind, detail, roughness, lacunarity, offset, gain, normalize, footprint));
}
"#;

/// What the shading point knows, as the generated function reads it; the
/// surface shader fills these in before calling it.
pub const INPUTS: &str = "vec3 fs_in_object; vec3 fs_in_generated; vec2 fs_in_uv; vec3 fs_in_object_normal; vec3 fs_in_position; vec3 fs_in_normal; vec3 fs_in_true_normal; vec3 fs_in_incoming; float fs_in_backfacing; vec3 fs_in_object_location; float fs_in_object_random;
";

/// A material's program as GLSL, and which of the view's parameters it
/// computes.
#[derive(Clone, Debug, PartialEq)]
pub struct Generated {
    /// `void fs_material(out vec3 base, out float metal, out float rough,
    /// out float height)`, reading the `fs_in_*` globals.
    pub code: String,
    pub base: bool,
    pub metal: bool,
    pub rough: bool,
    /// A height in scene units for the bump.
    pub height: bool,
}

impl Generated {
    /// What a shader variant puts in place of the surface shader's marker:
    /// its switches, the inputs, the library and the function.
    pub fn source(&self) -> String {
        format!("{}{}{}
{}", self.defines(), INPUTS, library(), self.code)
    }

    /// The definitions a shader variant puts in front of its main.
    pub fn defines(&self) -> String {
        let mut out = String::from("#define FS_MATERIAL 1\n");
        for (on, name) in [(self.base, "FS_BASE"), (self.metal, "FS_METAL"), (self.rough, "FS_ROUGH"), (self.height, "FS_HEIGHT")] {
            if on {
                let _ = writeln!(out, "#define {name} 1");
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    Float,
    Vector,
    Color,
}

impl Ty {
    fn of(value: &Value) -> Ty {
        match value {
            Value::Float(_) | Value::Bool(_) => Ty::Float,
            Value::Vector(_) => Ty::Vector,
            Value::Color(_) => Ty::Color,
        }
    }
}

/// Texture calls already made, so a node read through two outputs - a
/// Voronoi's distance and its colour, a noise's value and its colour - is
/// evaluated once. The graph wires each output as an instruction of its
/// own, and a Voronoi is most of a pixel's cost.
#[derive(Default)]
struct Memo {
    /// Declarations to emit before the instruction that needed them.
    lines: String,
    vars: HashMap<String, String>,
    next: usize,
}

impl Memo {
    /// The variable holding `call`, declaring it as `ty` the first time.
    fn var(&mut self, ty: &str, call: String) -> String {
        if let Some(name) = self.vars.get(&call) {
            return name.clone();
        }
        let name = format!("m{}", self.next);
        self.next += 1;
        let _ = writeln!(self.lines, "    {ty} {name} = {call};");
        self.vars.insert(call, name.clone());
        name
    }

    fn has(&self, call: &str) -> Option<String> {
        self.vars.get(call).cloned()
    }
}

/// A typed GLSL expression of a register or a literal.
#[derive(Clone, Debug)]
struct Arg {
    expr: String,
    ty: Ty,
}

impl Arg {
    /// As `Value::as_float` reads it.
    fn f(&self) -> String {
        match self.ty {
            Ty::Float | Ty::Vector => format!("{}.x", self.expr),
            Ty::Color => format!("fs_luminance({}.xyz)", self.expr),
        }
    }
    /// As `Value::as_vector` and `Value::as_color` read it.
    fn v(&self) -> String {
        match self.ty {
            Ty::Float => format!("vec3({}.x)", self.expr),
            _ => format!("{}.xyz", self.expr),
        }
    }
}

fn lit(x: f32) -> String {
    let x = if x.is_finite() { x } else if x > 0.0 { 3.0e38 } else if x < 0.0 { -3.0e38 } else { 0.0 };
    let s = format!("{x:?}");
    if s.contains('.') || s.contains('e') { s } else { format!("{s}.0") }
}

fn vec3_lit(x: f32, y: f32, z: f32) -> String {
    format!("vec3({}, {}, {})", lit(x), lit(y), lit(z))
}

fn value_lit(value: &Value) -> (String, Ty) {
    match value {
        Value::Float(f) => (format!("vec4({}, 0.0, 0.0, 0.0)", lit(*f)), Ty::Float),
        Value::Bool(b) => (format!("vec4({}, 0.0, 0.0, 0.0)", if *b { "1.0" } else { "0.0" }), Ty::Float),
        Value::Vector(v) => (format!("vec4({}, 0.0)", vec3_lit(v.x, v.y, v.z)), Ty::Vector),
        Value::Color(c) => (format!("vec4({}, 0.0)", vec3_lit(c.r, c.g, c.b)), Ty::Color),
    }
}

fn float4(e: String) -> String {
    format!("vec4({e}, 0.0, 0.0, 0.0)")
}

fn vec4_of(e: String) -> String {
    format!("vec4({e}, 0.0)")
}

/// The program as GLSL, or `None` when none of the view's parameters can
/// be written.
pub fn generate(program: &CompiledGraph) -> Option<Generated> {
    let wanted = [Param::BaseColor, Param::BaseMetalness, Param::SpecularRoughness, Param::Bump];
    let program = program.restricted_to(&wanted);
    if program.driven.is_empty() {
        return None;
    }
    let mut body = String::new();
    // Each register's type and whether it could be written, as of now.
    let mut known: HashMap<u16, (Ty, bool)> = HashMap::new();
    let mut declared = std::collections::BTreeSet::new();
    let mut memo = Memo::default();
    // Registers holding an image's lookup, which the view draws its own way.
    let mut sampled = std::collections::HashSet::new();
    for instruction in &program.instructions {
        let dst = instruction.destination();
        // A register written again holds something new: calls that read
        // it would be stale, so nothing remembered before is trusted.
        if !declared.insert(dst.0) {
            memo.vars.clear();
        }
        let arg = |r: &Register, known: &HashMap<u16, (Ty, bool)>| -> Option<Arg> {
            match known.get(&r.0) {
                Some((ty, true)) => Some(Arg { expr: format!("r{}", r.0), ty: *ty }),
                _ => None,
            }
        };
        let written: Option<(String, Ty)> = match instruction {
            Instruction::Load { value, .. } => Some(value_lit(value)),
            Instruction::Input { input, .. } => point_input(*input),
            Instruction::Curvature { mode, radius, contrast, .. } => Some(value_lit(&Value::Float(mode.mask(0.0, *radius, *contrast)))),
            Instruction::Occlusion { contrast, .. } => Some(value_lit(&Value::Float(tequila_materials::graph::occlusion_shape(0.0, *contrast)))),
            Instruction::VertexColor { .. } => Some(value_lit(&Value::Color(tequila_core::spectrum::Spectrum::ONE))),
            Instruction::VolumeMap { .. } => Some(value_lit(&Value::Float(1.0))),
            Instruction::Apply { kind, args, output, .. } => {
                if let NodeKind::Channel { .. } = **kind {
                    if args.first().is_some_and(|r| sampled.contains(&r.0)) {
                        sampled.insert(dst.0);
                    }
                }
                let inputs: Option<Vec<Arg>> = args.iter().map(|r| arg(r, &known)).collect();
                // What cannot be written stands in as a neutral value - the
                // Gabor texture's mean is zero - so the rest of the pattern
                // still shows rather than the whole parameter going flat.
                inputs.and_then(|inputs| apply(kind, &inputs, *output, &mut memo)).or_else(|| match **kind {
                    NodeKind::Node(ShaderNode::Gabor { .. }) => Some(value_lit(&Value::Float(0.0))),
                    _ => Some(value_lit(&kind.evaluate_output(&vec![Value::Float(0.0); args.len()], *output))),
                })
            }
            // Images the view samples its own way: where a parameter reads
            // one directly, the view's texture path draws it; anywhere else
            // the image stands in as its fallback.
            Instruction::Sample { fallback, .. } | Instruction::SampleAlpha { fallback, .. } => {
                sampled.insert(dst.0);
                Some(value_lit(fallback))
            }
            Instruction::Project { .. } => Some(value_lit(&Value::Vector(tequila_core::math::Vec3::ZERO))),
            // Tequila's own procedurals, not written for the graphics card:
            // their middle value.
            Instruction::Evaluate { .. } => Some(value_lit(&Value::Float(0.5))),
        };
        body.push_str(&memo.lines);
        memo.lines.clear();
        match written {
            Some((expr, ty)) => {
                let _ = writeln!(body, "    r{} = {};", dst.0, expr);
                known.insert(dst.0, (ty, true));
            }
            None => {
                known.insert(dst.0, (Ty::Float, false));
            }
        }
    }

    let mut out = Generated { code: String::new(), base: false, metal: false, rough: false, height: false };
    let mut outputs = String::new();
    for (param, register) in &program.driven {
        let Some((ty, true)) = known.get(&register.0).copied() else { continue };
        if sampled.contains(&register.0) {
            continue;
        }
        let a = Arg { expr: format!("r{}", register.0), ty };
        match param {
            Param::BaseColor => {
                out.base = true;
                let _ = writeln!(outputs, "    base = {};", a.v());
            }
            Param::BaseMetalness => {
                out.metal = true;
                let _ = writeln!(outputs, "    metal = {};", a.f());
            }
            Param::SpecularRoughness => {
                out.rough = true;
                let _ = writeln!(outputs, "    rough = {};", a.f());
            }
            Param::Bump => {
                out.height = true;
                let _ = writeln!(outputs, "    height = {};", a.f());
            }
            _ => {}
        }
    }
    if !(out.base || out.metal || out.rough || out.height) {
        return None;
    }
    let mut code = String::from("void fs_material(out vec3 base, out float metal, out float rough, out float height) {\n");
    code.push_str("    base = vec3(0.0); metal = 0.0; rough = 0.0; height = 0.0;\n");
    for r in &declared {
        let _ = writeln!(code, "    vec4 r{r} = vec4(0.0);");
    }
    code.push_str(&body);
    code.push_str(&outputs);
    code.push_str("}\n");
    out.code = code;
    Some(out)
}

/// What the shading point knows, from the globals the shader fills in.
fn point_input(input: PointInput) -> Option<(String, Ty)> {
    Some(match input {
        PointInput::Object => (vec4_of("fs_in_object".into()), Ty::Vector),
        PointInput::Generated => (vec4_of("fs_in_generated".into()), Ty::Vector),
        PointInput::Uv => (vec4_of("vec3(fs_in_uv.x, 1.0 - fs_in_uv.y, 0.0)".into()), Ty::Vector),
        PointInput::ObjectNormal => (vec4_of("fs_in_object_normal".into()), Ty::Vector),
        PointInput::Reflection => (vec4_of("fs_in_normal * (2.0 * dot(fs_in_normal, fs_in_incoming)) - fs_in_incoming".into()), Ty::Vector),
        PointInput::Position => (vec4_of("fs_in_position".into()), Ty::Vector),
        PointInput::Normal => (vec4_of("fs_in_normal".into()), Ty::Vector),
        PointInput::TrueNormal => (vec4_of("fs_in_true_normal".into()), Ty::Vector),
        PointInput::Incoming => (vec4_of("fs_in_incoming".into()), Ty::Vector),
        PointInput::Backfacing => (float4("fs_in_backfacing".into()), Ty::Float),
        PointInput::ObjectLocation => (vec4_of("fs_in_object_location".into()), Ty::Vector),
        PointInput::ObjectRandom => (float4("fs_in_object_random".into()), Ty::Float),
    })
}

/// The type a node's output carries, found by asking the node itself on
/// zeros - the renderer's own evaluation is the definition.
fn output_type(kind: &NodeKind, count: usize, output: u8) -> Ty {
    let zeros = vec![Value::Float(0.0); count.max(10)];
    Ty::of(&kind.evaluate_output(&zeros[..count], output))
}

fn apply(kind: &NodeKind, a: &[Arg], output: u8, memo: &mut Memo) -> Option<(String, Ty)> {
    let zero = Arg { expr: "vec4(0.0)".into(), ty: Ty::Float };
    let get = |i: usize| a.get(i).cloned().unwrap_or_else(|| zero.clone());
    let f = |i: usize| get(i).f();
    let v = |i: usize| get(i).v();
    let ty = output_type(kind, a.len(), output);
    let expr = match kind {
        NodeKind::Constant(value) => return Some(value_lit(value)),
        NodeKind::Mix => vec4_of(format!("mix({}, {}, {})", v(0), v(1), f(2))),
        NodeKind::Multiply => vec4_of(format!("({} * {})", v(0), v(1))),
        NodeKind::Add => vec4_of(format!("({} + {})", v(0), v(1))),
        NodeKind::Invert => vec4_of(format!("(vec3(1.0) - {})", v(0))),
        NodeKind::Remap { from, to } => {
            let span = (from.y - from.x).abs().max(1e-8) * (from.y - from.x).signum();
            vec4_of(format!("(vec3({}) + ({} - vec3({})) / {} * {})", lit(to.x), v(0), lit(from.x), lit(span), lit(to.y - to.x)))
        }
        NodeKind::Clamp { low, high } => vec4_of(format!("clamp({}, {}, {})", v(0), lit(*low), lit(*high))),
        NodeKind::Bump { depth } => float4(format!("({} * {})", f(0), lit(*depth))),
        NodeKind::Channel { channel } => {
            let c = match channel {
                tequila_materials::graph::Channel::Red => "x",
                tequila_materials::graph::Channel::Green => "y",
                tequila_materials::graph::Channel::Blue => "z",
            };
            float4(format!("{}.{c}", v(0)))
        }
        NodeKind::Gradient { gradient } => vec4_of(ramp(gradient, &f(0))),
        NodeKind::Curvature { mode, radius, contrast } => return Some(value_lit(&Value::Float(mode.mask(0.0, *radius, *contrast)))),
        NodeKind::Occlusion { contrast, .. } => return Some(value_lit(&Value::Float(tequila_materials::graph::occlusion_shape(0.0, *contrast)))),
        NodeKind::VertexColor => return Some(value_lit(&Value::Color(tequila_core::spectrum::Spectrum::ONE))),
        NodeKind::VolumeMap { .. } => return Some(value_lit(&Value::Float(1.0))),
        NodeKind::Node(node) => return shader_node(node, a, output, memo).map(|e| (e, ty)),
        _ => return None,
    };
    Some((expr, ty))
}

/// A colour ramp, inline: held flat past its ends, as the renderer's is.
fn ramp(gradient: &tequila_texture::gradient::Gradient, t: &str) -> String {
    use tequila_texture::gradient::Interpolation;
    let stops = gradient.stops();
    let colour = |i: usize| vec3_lit(stops[i].color.r, stops[i].color.g, stops[i].color.b);
    match stops.len() {
        0 => return "vec3(0.5)".into(),
        1 => return colour(0),
        _ => {}
    }
    let last = stops.len() - 1;
    // A nested conditional on t, clamped as `Gradient::evaluate` clamps it.
    let t = format!("clamp({t}, 0.0, 1.0)");
    let mut expr = colour(last);
    for upper in (1..=last).rev() {
        let (a, b) = (&stops[upper - 1], &stops[upper]);
        let span = b.at - a.at;
        let segment = if span <= 0.0 {
            colour(upper)
        } else {
            let f = format!("(({t} - {}) / {})", lit(a.at), lit(span));
            match gradient.interpolation() {
                Interpolation::Linear => format!("mix({}, {}, {f})", colour(upper - 1), colour(upper)),
                Interpolation::Ease => format!("mix({}, {}, {f} * {f} * (3.0 - 2.0 * {f}))", colour(upper - 1), colour(upper)),
                Interpolation::Constant => format!("({t} >= {} ? {} : {})", lit(b.at), colour(upper), colour(upper - 1)),
            }
        };
        expr = format!("({t} <= {} ? {segment} : {expr})", lit(b.at));
    }
    format!("({t} <= {} ? {} : ({t} >= {} ? {} : {expr}))", lit(stops[0].at), colour(0), lit(stops[last].at), colour(last))
}

fn shader_node(node: &ShaderNode, a: &[Arg], output: u8, memo: &mut Memo) -> Option<String> {
    let zero = Arg { expr: "vec4(0.0)".into(), ty: Ty::Float };
    let get = |i: usize| a.get(i).cloned().unwrap_or_else(|| zero.clone());
    let f = |i: usize| get(i).f();
    let v = |i: usize| get(i).v();
    let b = |x: bool| if x { "true" } else { "false" };
    Some(match *node {
        ShaderNode::TextureCoordinate | ShaderNode::Geometry | ShaderNode::ObjectInfo => float4("0.0".into()),
        ShaderNode::LayerWeight => {
            let cos = format!("dot({}, {})", v(2), v(1));
            if output == 0 {
                float4(format!("fs_fresnel_dielectric_cos({cos}, ({} > 0.5) ? max(1.0 - {}, 1e-5) : 1.0 / max(1.0 - {}, 1e-5))", f(3), f(0), f(0)))
            } else {
                float4(format!(
                    "(1.0 - (({b} != 0.5) ? pow(abs({cos}), ({bc} < 0.5 ? 2.0 * {bc} : 0.5 / (1.0 - {bc}))) : abs({cos})))",
                    b = f(0),
                    bc = format!("clamp({}, 0.0, 1.0 - 1e-5)", f(0)),
                ))
            }
        }
        ShaderNode::Fresnel => {
            let eta = format!("max({}, 1e-5)", f(0));
            float4(format!("fs_fresnel_dielectric_cos(dot({}, {}), ({} > 0.5) ? 1.0 / {eta} : {eta})", v(2), v(1), f(3)))
        }
        ShaderNode::Noise { dims, fractal, normalize } => {
            let kind = match fractal {
                FractalKind::Multifractal => 0,
                FractalKind::Fbm => 1,
                FractalKind::HybridMultifractal => 2,
                FractalKind::RidgedMultifractal => 3,
                FractalKind::HeteroTerrain => 4,
            };
            let s = f(2);
            let point = match dims {
                1 => format!("({} * {s})", f(1)),
                2 => format!("({}.xy * {s})", v(0)),
                3 => format!("({} * {s})", v(0)),
                _ => format!("vec4({} * {s}, {} * {s})", v(0), f(1)),
            };
            let args = format!("{point}, {kind}, {}, {}, {}, {}, {}, {}, {}", f(3), f(4), f(5), f(6), f(7), f(8), b(normalize));
            // The colour's first channel is the value: a noise read both
            // ways is evaluated as colour once.
            let colour_call = format!("fs_noise_color({args})");
            if output == 0 {
                match memo.has(&colour_call) {
                    Some(colour) => float4(format!("{colour}.x")),
                    None => float4(memo.var("float", format!("fs_noise_fac({args})"))),
                }
            } else {
                vec4_of(memo.var("vec3", colour_call))
            }
        }
        ShaderNode::Voronoi { dims, feature, metric, normalize } => {
            if dims != 3 {
                return None;
            }
            let s = f(2);
            let coord = format!("({} * {s})", v(0));
            match feature {
                VoronoiFeature::DistanceToEdge => float4(memo.var("float", format!("fs_voronoi3_edge({coord}, {}, {}, {}, {}, {})", f(3), f(4), f(5), f(8), b(normalize)))),
                VoronoiFeature::NSphereRadius => float4(format!("fs_vor_radius({coord}, clamp({}, 0.0, 1.0))", f(8))),
                _ => {
                    let feature = match feature {
                        VoronoiFeature::F2 => 1,
                        VoronoiFeature::SmoothF1 => 2,
                        _ => 0,
                    };
                    let metric = match metric {
                        VoronoiMetric::Euclidean => 0,
                        VoronoiMetric::Manhattan => 1,
                        VoronoiMetric::Chebychev => 2,
                        VoronoiMetric::Minkowski => 3,
                    };
                    let call = memo.var("FsVoronoi", format!("fs_voronoi3({coord}, {s}, {}, {}, {}, {}, {}, {}, {feature}, {metric}, {})", f(3), f(4), f(5), f(6), f(7), f(8), b(normalize)));
                    match output {
                        0 => float4(format!("{call}.distance")),
                        1 => vec4_of(format!("{call}.color")),
                        2 => vec4_of(format!("{call}.position")),
                        _ => float4("0.0".into()),
                    }
                }
            }
        }
        ShaderNode::Wave { kind, direction, profile } => {
            let kind = match kind {
                WaveKind::Bands => 0,
                WaveKind::Rings => 1,
            };
            let direction = match direction {
                WaveDirection::X => 0,
                WaveDirection::Y => 1,
                WaveDirection::Z => 2,
                WaveDirection::Diagonal => 3,
            };
            let profile = match profile {
                WaveProfile::Sine => 0,
                WaveProfile::Saw => 1,
                WaveProfile::Triangle => 2,
            };
            let call = format!("fs_wave({} * {}, {kind}, {direction}, {profile}, {}, {}, {}, {}, {})", v(0), f(1), f(2), f(3), f(4), f(5), f(6));
            if output == 0 { vec4_of(format!("vec3({call})")) } else { float4(call) }
        }
        ShaderNode::Magic { depth } => {
            let call = format!("fs_magic({}, {}, {depth}, {})", v(0), f(1), f(2));
            if output == 0 { vec4_of(call) } else { float4(format!("dot({call}, vec3(1.0 / 3.0))")) }
        }
        ShaderNode::Brick { offset, offset_frequency, squash, squash_frequency } => {
            let tm = format!(
                "fs_brick({} * {}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
                v(0),
                f(4),
                f(5),
                f(6),
                f(7),
                f(8),
                f(9),
                lit(offset),
                offset_frequency,
                lit(squash),
                squash_frequency
            );
            if output == 0 {
                vec4_of(format!(
                    "mix((({tm}).y != 1.0 ? mix({c1}, {c2}, ({tm}).x) : {c1}), {mortar}, ({tm}).y)",
                    c1 = v(1),
                    c2 = v(2),
                    mortar = v(3)
                ))
            } else {
                float4(format!("({tm}).y"))
            }
        }
        ShaderNode::Checker => {
            let fac = format!("fs_checker({} * {})", v(0), f(3));
            if output == 0 { vec4_of(format!("({fac} == 1.0 ? {} : {})", v(1), v(2))) } else { float4(fac) }
        }
        ShaderNode::GradientTexture { kind } => {
            let kind = match kind {
                GradientKind::Linear => 0,
                GradientKind::Quadratic => 1,
                GradientKind::Easing => 2,
                GradientKind::Diagonal => 3,
                GradientKind::Spherical => 4,
                GradientKind::QuadraticSphere => 5,
                GradientKind::Radial => 6,
            };
            let fac = format!("fs_gradient_texture({}, {kind})", v(0));
            if output == 0 { vec4_of(format!("vec3({fac})")) } else { float4(fac) }
        }
        ShaderNode::WhiteNoise { dims } => {
            let (value, colour) = match dims {
                1 => (format!("fs_hashf1({})", f(1)), format!("fs_hashf1_3({})", f(1))),
                2 => (format!("fs_hashf2({}.xy)", v(0)), format!("fs_hashf2_3({}.xy)", v(0))),
                3 => (format!("fs_hashf3({})", v(0)), format!("fs_hashf3_3({})", v(0))),
                _ => (format!("fs_hashf4(vec4({}, {}))", v(0), f(1)), format!("fs_hashf4_3(vec4({}, {}))", v(0), f(1))),
            };
            if output == 0 { float4(value) } else { vec4_of(colour) }
        }
        ShaderNode::Gabor { .. } => return None,
        ShaderNode::Mix { data, blend, clamp_factor, clamp_result } => {
            let t = if clamp_factor { format!("clamp({}, 0.0, 1.0)", f(0)) } else { f(0) };
            match data {
                MixData::Float => float4(format!("mix({}, {}, {t})", f(1), f(2))),
                MixData::Vector => vec4_of(format!("mix({}, {}, {t})", v(1), v(2))),
                MixData::Color => {
                    let mode = BlendMode::ALL.iter().position(|m| *m == blend).unwrap_or(0);
                    let call = format!("fs_blend({mode}, {t}, {}, {})", v(1), v(2));
                    vec4_of(if clamp_result { format!("clamp({call}, 0.0, 1.0)") } else { call })
                }
            }
        }
        ShaderNode::HueSaturationValue => vec4_of(format!("fs_hue_saturation_value({}, {}, {}, {}, {})", v(4), f(0), f(1), f(2), f(3))),
        ShaderNode::BrightContrast => vec4_of(format!("max((1.0 + {c}) * {} + ({} - {c} * 0.5), vec3(0.0))", v(0), f(1), c = f(2))),
        ShaderNode::Gamma => vec4_of(format!("fs_gamma({}, {})", v(0), f(1))),
        ShaderNode::InvertColor => vec4_of(format!("mix({c}, vec3(1.0) - {c}, {})", f(0), c = v(1))),
        ShaderNode::Math { op, clamp } => {
            let e = math(op, &f(0), &f(1), &f(2));
            float4(if clamp { format!("clamp({e}, 0.0, 1.0)") } else { e })
        }
        ShaderNode::VectorMath { op } => {
            let (e, value) = vector_math(op, &v(0), &v(1), &v(2), &f(3));
            if value { float4(e) } else { vec4_of(e) }
        }
        ShaderNode::MapRange { kind, clamp } => {
            let kind = match kind {
                MapRangeKind::Linear => 0,
                MapRangeKind::Stepped => 1,
                MapRangeKind::Smoothstep => 2,
                MapRangeKind::Smootherstep => 3,
            };
            float4(format!("fs_map_range({kind}, {}, {}, {}, {}, {}, {}, {})", f(0), f(1), f(2), f(3), f(4), f(5), b(clamp)))
        }
        ShaderNode::Clamp { range } => float4(format!("fs_clamp_node({}, {}, {}, {})", f(0), f(1), f(2), b(range))),
        ShaderNode::SeparateXyz => float4(format!("({})[{}]", v(0), output.min(2))),
        ShaderNode::CombineXyz => vec4_of(format!("vec3({}, {}, {})", f(0), f(1), f(2))),
        ShaderNode::SeparateColor { model } => {
            let c = match model {
                ColorModel::Rgb => v(0),
                ColorModel::Hsv => format!("fs_rgb_to_hsv({})", v(0)),
                ColorModel::Hsl => format!("fs_rgb_to_hsl({})", v(0)),
            };
            float4(format!("({c})[{}]", output.min(2)))
        }
        ShaderNode::CombineColor { model } => {
            let c = format!("vec3({}, {}, {})", f(0), f(1), f(2));
            vec4_of(match model {
                ColorModel::Rgb => c,
                ColorModel::Hsv => format!("fs_hsv_to_rgb({c})"),
                ColorModel::Hsl => format!("fs_hsl_to_rgb({c})"),
            })
        }
        ShaderNode::Mapping { kind } => {
            let kind = match kind {
                MappingKind::Point => 0,
                MappingKind::Texture => 1,
                MappingKind::Vector => 2,
                MappingKind::Normal => 3,
            };
            vec4_of(format!("fs_mapping({kind}, {}, {}, {}, {})", v(0), v(1), v(2), v(3)))
        }
    })
}

fn math(op: MathOp, a: &str, b: &str, c: &str) -> String {
    match op {
        MathOp::Add => format!("({a} + {b})"),
        MathOp::Subtract => format!("({a} - {b})"),
        MathOp::Multiply => format!("({a} * {b})"),
        MathOp::Divide => format!("fs_safe_divide({a}, {b})"),
        MathOp::MultiplyAdd => format!("({a} * {b} + {c})"),
        MathOp::Power => format!("fs_safe_pow({a}, {b})"),
        MathOp::Logarithm => format!("fs_safe_log({a}, {b})"),
        MathOp::Sqrt => format!("sqrt(max({a}, 0.0))"),
        MathOp::InvSqrt => format!("({a} > 0.0 ? inversesqrt({a}) : 0.0)"),
        MathOp::Absolute => format!("abs({a})"),
        MathOp::Exponent => format!("exp({a})"),
        MathOp::Minimum => format!("min({a}, {b})"),
        MathOp::Maximum => format!("max({a}, {b})"),
        MathOp::LessThan => format!("(({a}) < ({b}) ? 1.0 : 0.0)"),
        MathOp::GreaterThan => format!("(({a}) > ({b}) ? 1.0 : 0.0)"),
        MathOp::Sign => format!("fs_sign({a})"),
        MathOp::Compare => format!("((({a}) == ({b}) || abs(({a}) - ({b})) <= max({c}, 1.1920929e-7)) ? 1.0 : 0.0)"),
        MathOp::SmoothMin => format!("fs_smoothmin({a}, {b}, {c})"),
        MathOp::SmoothMax => format!("(-fs_smoothmin(-({a}), -({b}), {c}))"),
        // Steps and waves filtered over the pixel (`fs_ffloor` in
        // procedural.glsl), so a pattern finer than the pixels is its mean.
        MathOp::Round => format!("fs_ffloor({a} + 0.5)"),
        MathOp::Floor => format!("fs_ffloor({a})"),
        MathOp::Ceil => format!("(-fs_ffloor(-({a})))"),
        MathOp::Trunc => format!("fs_ftrunc({a})"),
        MathOp::Fraction => format!("fs_ffract({a})"),
        MathOp::Modulo => format!("(({b}) != 0.0 ? fs_ftrunc_mod({a}, {b}) : 0.0)"),
        MathOp::FlooredModulo => format!("fs_ffloored_mod({a}, {b})"),
        MathOp::Wrap => format!("fs_fwrapf({a}, {b}, {c})"),
        MathOp::Snap => format!("(fs_ffloor(fs_safe_divide({a}, {b})) * ({b}))"),
        MathOp::PingPong => format!("fs_pingpong({a}, {b})"),
        MathOp::Sine => format!("fs_fsin({a})"),
        MathOp::Cosine => format!("fs_fcos({a})"),
        MathOp::Tangent => format!("tan({a})"),
        MathOp::Arcsine => format!("asin(clamp({a}, -1.0, 1.0))"),
        MathOp::Arccosine => format!("acos(clamp({a}, -1.0, 1.0))"),
        MathOp::Arctangent => format!("atan({a})"),
        MathOp::Arctan2 => format!("((({a}) == 0.0 && ({b}) == 0.0) ? 0.0 : atan({a}, {b}))"),
        MathOp::Sinh => format!("sinh({a})"),
        MathOp::Cosh => format!("cosh({a})"),
        MathOp::Tanh => format!("tanh({a})"),
        MathOp::Radians => format!("({a} * 0.017453292)"),
        MathOp::Degrees => format!("({a} * 57.29578)"),
    }
}

/// A vector maths node, and whether it gives a number rather than a vector.
fn vector_math(op: VectorMathOp, a: &str, b: &str, c: &str, param: &str) -> (String, bool) {
    let per = |f: &str| format!("vec3({f}({a}.x, {b}.x), {f}({a}.y, {b}.y), {f}({a}.z, {b}.z))");
    (
        match op {
            VectorMathOp::Add => format!("({a} + {b})"),
            VectorMathOp::Subtract => format!("({a} - {b})"),
            VectorMathOp::Multiply => format!("({a} * {b})"),
            VectorMathOp::Divide => format!("fs_safe_divide3({a}, {b})"),
            VectorMathOp::MultiplyAdd => format!("({a} * {b} + {c})"),
            VectorMathOp::CrossProduct => format!("cross({a}, {b})"),
            VectorMathOp::Project => format!("(dot({b}, {b}) != 0.0 ? {b} * (dot({a}, {b}) / dot({b}, {b})) : vec3(0.0))"),
            VectorMathOp::Reflect => format!("({a} - (length({b}) != 0.0 ? normalize({b}) : {b}) * 2.0 * dot({a}, (length({b}) != 0.0 ? normalize({b}) : {b})))"),
            VectorMathOp::Refract => format!("refract({a}, (length({b}) != 0.0 ? normalize({b}) : {b}), {param})"),
            VectorMathOp::Faceforward => format!("(dot({c}, {b}) < 0.0 ? {a} : -{a})"),
            VectorMathOp::DotProduct => return (format!("dot({a}, {b})"), true),
            VectorMathOp::Distance => return (format!("length({a} - {b})"), true),
            VectorMathOp::Length => return (format!("length({a})"), true),
            VectorMathOp::Scale => format!("({a} * {param})"),
            VectorMathOp::Normalize => format!("(length({a}) != 0.0 ? normalize({a}) : {a})"),
            VectorMathOp::Snap => format!("vec3(floor(fs_safe_divide({a}.x, {b}.x)) * {b}.x, floor(fs_safe_divide({a}.y, {b}.y)) * {b}.y, floor(fs_safe_divide({a}.z, {b}.z)) * {b}.z)"),
            VectorMathOp::Floor => format!("fs_ffloor3({a})"),
            VectorMathOp::Ceil => format!("(-fs_ffloor3(-({a})))"),
            VectorMathOp::Modulo => format!("vec3(({b}.x != 0.0 ? fs_trunc_mod({a}.x, {b}.x) : 0.0), ({b}.y != 0.0 ? fs_trunc_mod({a}.y, {b}.y) : 0.0), ({b}.z != 0.0 ? fs_trunc_mod({a}.z, {b}.z) : 0.0))"),
            VectorMathOp::Wrap => format!("vec3(fs_floored_mod({a}.x - {c}.x, {b}.x - {c}.x) + {c}.x, fs_floored_mod({a}.y - {c}.y, {b}.y - {c}.y) + {c}.y, fs_floored_mod({a}.z - {c}.z, {b}.z - {c}.z) + {c}.z)"),
            VectorMathOp::Fraction => format!("(({a}) - fs_ffloor3({a}))"),
            VectorMathOp::Absolute => format!("abs({a})"),
            VectorMathOp::Power => per("fs_safe_pow"),
            VectorMathOp::Sign => format!("vec3(fs_sign({a}.x), fs_sign({a}.y), fs_sign({a}.z))"),
            VectorMathOp::Minimum => format!("min({a}, {b})"),
            VectorMathOp::Maximum => format!("max({a}, {b})"),
            VectorMathOp::Sine => format!("vec3(fs_fsin({a}.x), fs_fsin({a}.y), fs_fsin({a}.z))"),
            VectorMathOp::Cosine => format!("vec3(fs_fcos({a}.x), fs_fcos({a}.y), fs_fcos({a}.z))"),
            VectorMathOp::Tangent => format!("tan({a})"),
        },
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every standard library preset's pattern reaches the viewport: a
    /// preset whose base colour the renderer computes from a graph has it
    /// written here too, unless it reads an image or one of Tequila's own
    /// procedurals (which the view handles otherwise).
    #[test]
    fn the_library_is_written_for_the_graphics_card() {
        let folder = crate::library_tab::standard_library_path();
        let (entries, _) = tequila_format::library::read_directory(&folder);
        let mut written = 0;
        let mut missing = Vec::new();
        for entry in &entries {
            let program = entry.graph.compile().expect("compiles");
            let base = program.driven.iter().find(|(p, _)| *p == Param::BaseColor).map(|(_, r)| *r);
            // A base colour read straight from an image is the view's own
            // texture path's to draw.
            let from_image = base.is_some_and(|r| {
                program.instructions.iter().rev().find(|i| i.destination() == r).is_some_and(|i| matches!(i, Instruction::Sample { .. }))
            });
            match generate(&program) {
                Some(g) if g.base || base.is_none() || from_image => written += 1,
                None if base.is_none() || from_image => {}
                _ => missing.push(entry.name.clone()),
            }
        }
        assert!(missing.is_empty(), "base colours the viewport cannot draw: {missing:?}");
        assert!(written > 100, "only {written} presets were written");
    }

    /// The fractal sums come out once per dimension, with nothing left of
    /// the template.
    #[test]
    fn the_library_has_every_dimension() {
        let l = library();
        assert!(!l.contains("@FRACTALS@") && !l.contains("{T}"));
        for t in ["float p", "vec2 p", "vec3 p", "vec4 p"] {
            assert!(l.contains(&format!("float fs_fbm({t},")), "no fBM over {t}");
            assert!(l.contains(&format!("float fs_footprint({t})")), "no footprint over {t}");
        }
    }

    /// A ramp reads as the renderer's: held flat past its ends, linear
    /// between.
    #[test]
    fn a_ramp_is_written_with_its_ends_held() {
        use tequila_core::spectrum::Spectrum;
        use tequila_texture::gradient::{Gradient, Stop};
        let g = Gradient::new(vec![Stop { at: 0.25, color: Spectrum::splat(0.0) }, Stop { at: 0.75, color: Spectrum::splat(1.0) }]);
        let code = ramp(&g, "x");
        assert!(code.contains("<= 0.25"), "{code}");
        assert!(code.contains(">= 0.75"), "{code}");
        assert!(code.contains("mix("), "{code}");
    }
}
