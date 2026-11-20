// The material programs (program.rs): one generic loop over every
// material's bytecode, the registers a fixed bank, each node family reached
// from one arm of one switch (docs/07 §7). shade.rs's `shade` is the
// specification: its point inputs, its Apply through nodes.wgsl, its
// write-back into the material.
//#include nodes
//#include openpbr
//#include textures

// Every material's instructions and write-back pairs, their constants, and
// where each material's are (program.rs's GpuProgram). A kernel that runs
// programs binds the first two; the third is the transport's.
@group(0) @binding(11) var<storage, read> program_words: array<u32>;
@group(0) @binding(12) var<storage, read> program_constants: array<vec4<f32>>;

struct GpuProgram {
    main_start: u32,
    main_end: u32,
    bump_start: u32,
    bump_end: u32,
    constants: u32,
    writes_start: u32,
    writes_count: u32,
    bump_here: u32,
    bump_height: u32,
    tangent: u32,
    flags: u32,
    _pad: u32,
};

const PROGRAM_RUNS: u32 = 1u;
const PROGRAM_BUMPS: u32 = 2u;
const PROGRAM_NONE: u32 = 0xFFFFFFFFu;

const OP_INPUT: u32 = 1u;
const OP_MATH: u32 = 2u;
const OP_VECTOR_MATH: u32 = 3u;
const OP_MAP_RANGE: u32 = 4u;
const OP_MIX: u32 = 5u;
const OP_NOISE: u32 = 6u;
const OP_VORONOI: u32 = 7u;
const OP_WHITE_NOISE: u32 = 8u;
const OP_MAPPING: u32 = 9u;
const OP_COMBINE_XYZ: u32 = 10u;
const OP_SEPARATE_COLOR: u32 = 11u;
const OP_GRADIENT: u32 = 12u;
const OP_LEGACY: u32 = 13u;
const OP_LAYER_WEIGHT: u32 = 14u;
const OP_FRESNEL: u32 = 15u;
const OP_CLAMP: u32 = 16u;
const OP_COMBINE_COLOR: u32 = 17u;
const OP_HSV: u32 = 18u;
const OP_BRIGHT_CONTRAST: u32 = 19u;
const OP_GAMMA: u32 = 20u;
const OP_INVERT_COLOR: u32 = 21u;
const OP_CHECKER: u32 = 22u;
const OP_GRADIENT_TEXTURE: u32 = 23u;
const OP_TEXTURE: u32 = 24u;

// RayKind, as the Light Path node reads it.
const RAY_CAMERA: u32 = 0u;
const RAY_SHADOW: u32 = 1u;
const RAY_DIFFUSE: u32 = 2u;
const RAY_GLOSSY: u32 = 3u;
const RAY_TRANSMISSION: u32 = 4u;

// What the shading point says to the nodes that read it (SurfacePoint and
// its PointWorld): the part's own space, scaled to scene units by `scale`,
// and the world.
struct ProgPoint {
    object: vec3<f32>,
    scale: f32,
    object_normal: vec3<f32>,
    backfacing: f32,
    uv: vec2<f32>,
    object_random: f32,
    object_number: f32,
    position: vec3<f32>,
    ray: u32,
    normal: vec3<f32>,
    true_normal: vec3<f32>,
    incoming: vec3<f32>,
    generated: vec3<f32>,
    object_location: vec3<f32>,
    object_color: vec3<f32>,
    // The footprint in the part's own space, and in UV where the surface
    // has UVs to measure it by (SurfacePoint::uv_footprint): what a picture
    // picks its level by.
    footprint: f32,
    uv_footprint: vec2<f32>,
    has_uv_footprint: u32,
};

// The registers, and the program's first constant.
var<private> prog_regs: array<vec4<f32>, 48>;
var<private> prog_constants: u32;
// The operands of the instruction being run.
var<private> prog_operands: u32;

// An operand (program.rs): a register or a constant, as it is, one lane in
// every lane, or a colour's luminance in every lane.
fn prog_value(operand: u32) -> vec4<f32> {
    let index = operand & 0xFFFu;
    var v: vec4<f32>;
    if ((operand & 0x1000u) != 0u) {
        v = program_constants[prog_constants + index];
    } else {
        v = prog_regs[index];
    }
    switch (operand >> 13u) & 7u {
        case 1u: { return vec4<f32>(0.2722287 * v.x + 0.6740818 * v.y + 0.0536895 * v.z); }
        case 2u: { return vec4<f32>(v.x); }
        case 3u: { return vec4<f32>(v.y); }
        case 4u: { return vec4<f32>(v.z); }
        case 5u: { return vec4<f32>(v.w); }
        default: { return v; }
    }
}

// Operand `k` of the instruction being run.
fn prog_arg(k: u32) -> vec4<f32> {
    return prog_value(program_words[prog_operands + k]);
}

// shade.rs's point_input, by program.rs's point_input_code.
fn prog_point_input(code: u32, pt: ProgPoint) -> vec4<f32> {
    switch code {
        case 0u: { return vec4<f32>(pt.object * pt.scale, 0.0); }
        case 1u: { return vec4<f32>(pt.generated, 0.0); }
        case 2u: { return vec4<f32>(pt.uv.x, 1.0 - pt.uv.y, 0.0, 0.0); }
        case 3u: { return vec4<f32>(pt.object_normal, 0.0); }
        case 4u: {
            let n = pt.normal;
            let i = pt.incoming;
            return vec4<f32>(n * (2.0 * cy_dot3(n, i)) - i, 0.0);
        }
        case 5u: { return vec4<f32>(pt.position, 0.0); }
        case 6u: { return vec4<f32>(pt.normal, 0.0); }
        case 7u: { return vec4<f32>(pt.true_normal, 0.0); }
        case 8u: { return vec4<f32>(pt.incoming, 0.0); }
        case 9u: { return vec4<f32>(pt.backfacing); }
        case 10u: { return vec4<f32>(pt.object_location, 0.0); }
        case 11u: { return vec4<f32>(pt.object_random); }
        case 12u: { return vec4<f32>(pt.object_color, 0.0); }
        case 13u: { return vec4<f32>(pt.object_number); }
        default: { return vec4<f32>(select(0.0, 1.0, pt.ray == code - 14u)); }
    }
}

// tequila_texture's Gradient::evaluate over the instruction's stops, each
// an operand (colour, position), after the input.
fn prog_gradient(settings: u32, t_in: f32) -> vec3<f32> {
    let count = settings >> 4u;
    let interpolation = settings & 15u;
    let t = select(0.0, clamp(t_in, 0.0, 1.0), abs(t_in) <= CY_F32_MAX);
    if (count == 0u) {
        return vec3<f32>(0.5);
    }
    let first = prog_arg(1u);
    if (count == 1u) {
        return first.xyz;
    }
    if (t <= first.w) {
        return first.xyz;
    }
    let last = prog_arg(count);
    if (t >= last.w) {
        return last.xyz;
    }
    var upper = count - 1u;
    for (var k = 0u; k < count; k++) {
        if (prog_arg(1u + k).w >= t) {
            upper = k;
            break;
        }
    }
    upper = max(upper, 1u);
    let a = prog_arg(upper);
    let b = prog_arg(upper + 1u);
    let span = b.w - a.w;
    if (span <= 0.0) {
        return b.xyz;
    }
    var f = (t - a.w) / span;
    switch interpolation {
        case 2u: { return select(a.xyz, b.xyz, t >= b.w); }
        case 1u: { f = f * f * (3.0 - 2.0 * f); }
        default: {}
    }
    return a.xyz * (1.0 - f) + b.xyz * f;
}

// Tequila's own nodes (graph.rs's NodeKind::evaluate), on the operands.
fn prog_legacy(settings: u32, a: array<vec4<f32>, 10>) -> vec4<f32> {
    switch settings & 15u {
        case 0u: {
            let t = a[2].x;
            return vec4<f32>(a[0].xyz * (1.0 - t) + a[1].xyz * t, 0.0);
        }
        case 1u: { return vec4<f32>(a[0].xyz * a[1].xyz, 0.0); }
        case 2u: { return vec4<f32>(a[0].xyz + a[1].xyz, 0.0); }
        case 3u: { return vec4<f32>(vec3<f32>(1.0) - a[0].xyz, 0.0); }
        case 4u: {
            let c = a[0].xyz;
            let r = a[1];
            let d = r.y - r.x;
            // f32::signum: 1 for +0.
            let sign = select(1.0, -1.0, d < 0.0 || (d == 0.0 && (bitcast<u32>(d) & 0x80000000u) != 0u));
            let span = max(abs(d), 1e-8) * sign;
            return vec4<f32>(r.z + (c - vec3<f32>(r.x)) / span * (r.w - r.z), 0.0);
        }
        case 5u: {
            let r = a[1];
            return vec4<f32>(min(max(a[0].xyz, vec3<f32>(r.x)), vec3<f32>(r.y)), 0.0);
        }
        case 6u: { return vec4<f32>(a[0][(settings >> 4u) & 3u]); }
        default: { return vec4<f32>(a[0].x * a[1].x); }
    }
}

// Run instructions [start, end) at `pt`, into the registers. An
// instruction's operands are read first, in one place, so every arm reads
// plain values and the node it runs is reached from that arm alone.
fn prog_run(start: u32, end: u32, pt: ProgPoint) {
    var pc = start;
    loop {
        if (pc >= end) {
            break;
        }
        let head = program_words[pc];
        let settings = program_words[pc + 1u];
        prog_operands = pc + 2u;
        // No instruction has 128 operands: zero, which the compiler cannot know.
        cy_zero = head & 0x80000000u;
        var a: array<vec4<f32>, 10>;
        let count = min(head >> 24u, 10u);
        for (var k = 0u; k < count; k++) {
            a[k] = prog_value(program_words[prog_operands + k]);
        }
        var out = vec4<f32>(0.0);
        var out2 = vec4<f32>(0.0);
        switch head & 0xFFu {
            case 1u: { out = prog_point_input(settings, pt); }
            case 2u: {
                var v = cy_math(settings & 0xFFu, a[0].x, a[1].x, a[2].x);
                if ((settings & 0x100u) != 0u) {
                    v = clamp(v, 0.0, 1.0);
                }
                out = vec4<f32>(v);
            }
            case 3u: { out = cy_vector_math(settings, a[0].xyz, a[1].xyz, a[2].xyz, a[3].x); }
            case 4u: { out = vec4<f32>(cy_map_range(settings, a[0].x, a[1].x, a[2].x, a[3].x, a[4].x, a[5].x)); }
            case 5u: { out = cy_mix_node(settings, a[0].x, a[1], a[2]); }
            case 6u: {
                out = cy_noise_node(settings, a[0].xyz, a[1].x, a[2].x, a[3].x, a[4].x, a[5].x, a[6].x, a[7].x, a[8].x);
            }
            case 7u: {
                let cells = cy_voronoi_node(settings, a[0].xyz, a[1].x, a[2].x, a[3].x, a[4].x, a[5].x, a[6].x, a[7].x, a[8].x);
                out = cells.a;
                out2 = cells.b;
            }
            case 8u: { out = cy_white_noise_node(settings, a[0].xyz, a[1].x); }
            case 9u: { out = vec4<f32>(cy_mapping_node(settings, a[0].xyz, a[1].xyz, a[2].xyz, a[3].xyz), 0.0); }
            case 10u: { out = vec4<f32>(a[0].x, a[1].x, a[2].x, 0.0); }
            case 11u: { out = vec4<f32>(cy_separate_color(settings, a[0].xyz), 0.0); }
            case 12u: { out = vec4<f32>(prog_gradient(settings, a[0].x), 0.0); }
            case 13u: { out = prog_legacy(settings, a); }
            case 14u: {
                let lw = cy_layer_weight(a[0].x, a[1].xyz, a[2].xyz, a[3].x > 0.5);
                out = vec4<f32>(lw, 0.0, 0.0);
            }
            case 15u: { out = vec4<f32>(cy_fresnel(a[0].x, a[1].xyz, a[2].xyz, a[3].x > 0.5)); }
            case 16u: { out = vec4<f32>(cy_clamp_node(settings != 0u, a[0].x, a[1].x, a[2].x)); }
            case 17u: { out = vec4<f32>(cy_combine_color(settings, vec3<f32>(a[0].x, a[1].x, a[2].x)), 0.0); }
            case 18u: { out = vec4<f32>(cy_hsv_node(a[4].xyz, a[0].x, a[1].x, a[2].x, a[3].x), 0.0); }
            case 19u: {
                let contrast = a[2].x;
                let m = 1.0 + contrast;
                let b = a[1].x - contrast * 0.5;
                out = vec4<f32>(max(m * a[0].xyz + vec3<f32>(b), vec3<f32>(0.0)), 0.0);
            }
            case 20u: { out = vec4<f32>(cy_gamma_node(a[0].xyz, a[1].x), 0.0); }
            case 21u: {
                let fac = a[0].x;
                let c = a[1].xyz;
                out = vec4<f32>(fac * (vec3<f32>(1.0) - c) + (1.0 - fac) * c, 0.0);
            }
            case 22u: {
                let fac = cy_checker(a[0].xyz * a[3].x);
                out = vec4<f32>(select(a[2].xyz, a[1].xyz, fac == 1.0), fac);
            }
            case 23u: {
                let fac = cy_gradient_texture(settings, a[0].xyz);
                out = vec4<f32>(vec3<f32>(fac), fac);
            }
            case 24u: {
                let tp = TexPoint(pt.object, pt.object_normal, pt.uv, pt.footprint, pt.scale, pt.uv_footprint, pt.has_uv_footprint != 0u);
                out = tex_instruction(settings, a[0], a[1], a[2], a[3], tp);
            }
            default: {}
        }
        prog_regs[(head >> 8u) & 0xFFu] = out;
        let second = (head >> 16u) & 0xFFu;
        if (second != 0xFFu) {
            prog_regs[second] = out2;
        }
        pc = pc + 2u + (head >> 24u);
    }
}

// The material as floats, GpuMaterial's members in order, for writing a
// parameter by its offset.
fn prog_material_floats(m: GpuMaterial) -> array<f32, 44> {
    return array<f32, 44>(
        m.base_color.x, m.base_color.y, m.base_color.z, m.base_color.w,
        m.metal_n.x, m.metal_n.y, m.metal_n.z, m.metal_n.w,
        m.metal_k.x, m.metal_k.y, m.metal_k.z, m.metal_k.w,
        m.specular_color.x, m.specular_color.y, m.specular_color.z, m.specular_color.w,
        m.coat_color.x, m.coat_color.y, m.coat_color.z, m.coat_color.w,
        m.fuzz_color.x, m.fuzz_color.y, m.fuzz_color.z, m.fuzz_color.w,
        m.base_weight, m.base_metalness, m.base_diffuse_roughness, m.specular_weight,
        m.specular_roughness, m.specular_roughness_anisotropy, m.specular_ior, m.coat_weight,
        m.coat_roughness, m.coat_ior, m.coat_darkening, m.fuzz_weight,
        m.fuzz_roughness, m.geometry_occlusion, m.specular_rotation, m._pad,
        m.emission.x, m.emission.y, m.emission.z, m.emission.w,
    );
}

// The floats back into a material; the glass after them, which no program
// writes, as `base` has it.
fn prog_material_from(f: array<f32, 44>, base: GpuMaterial) -> GpuMaterial {
    return GpuMaterial(
        vec4<f32>(f[0], f[1], f[2], f[3]),
        vec4<f32>(f[4], f[5], f[6], f[7]),
        vec4<f32>(f[8], f[9], f[10], f[11]),
        vec4<f32>(f[12], f[13], f[14], f[15]),
        vec4<f32>(f[16], f[17], f[18], f[19]),
        vec4<f32>(f[20], f[21], f[22], f[23]),
        f[24], f[25], f[26], f[27], f[28], f[29], f[30], f[31], f[32], f[33], f[34], f[35], f[36], f[37], f[38], f[39],
        vec4<f32>(f[40], f[41], f[42], f[43]),
        base.transmission_color, base.transmission_tint, base.volume_density, base.medium_priority, base.base_ior, base.glass,
        base.pane_weight, base.pane_opacity, base.pane_ior, base.pane_coat, base.pane_coat_ior, base.pane_depth, base._pad2,
    );
}

// write_param for every driven parameter of `p`: a colour as a colour, a
// number as a number (a colour's luminance), over `base`.
fn prog_write_back(p: GpuProgram, base: GpuMaterial) -> GpuMaterial {
    var f = prog_material_floats(base);
    for (var k = 0u; k < p.writes_count; k++) {
        let slot = program_words[p.writes_start + 2u * k];
        let v = prog_value(program_words[p.writes_start + 2u * k + 1u]);
        let at = slot & 0xFFu;
        if ((slot & 0x100u) != 0u) {
            f[at] = v.x;
            f[at + 1u] = v.y;
            f[at + 2u] = v.z;
        } else {
            f[at] = v.x;
        }
    }
    return prog_material_from(f, base);
}

// Vec3 / f32 as tequila-core has it: times the reciprocal.
fn prog_over(v: vec3<f32>, s: f32) -> vec3<f32> {
    return v * (1.0 / s);
}

// try_normalize, or `fallback`.
fn prog_normalized(v: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let l2 = cy_dot3(v, v);
    return select(fallback, prog_over(v, sqrt(l2)), l2 > 0.0 && l2 <= 3.4e38);
}

// The change in UV that moves a point by `offset` (surface_bump.rs
// uv_offset): least squares on the UV derivatives.
fn prog_uv_offset(dpdu: vec3<f32>, dpdv: vec3<f32>, offset: vec3<f32>) -> vec2<f32> {
    let a = cy_dot3(dpdu, dpdu);
    let b = cy_dot3(dpdu, dpdv);
    let c = cy_dot3(dpdv, dpdv);
    let det = a * c - b * b;
    if (abs(det) < 1e-20) {
        return vec2<f32>(0.0);
    }
    let pu = cy_dot3(dpdu, offset);
    let pv = cy_dot3(dpdv, offset);
    return vec2<f32>((c * pu - b * pv) / det, (a * pv - b * pu) / det);
}

// TangentFrame::from_normal's tangent: square to the normal and to Z, or
// to X for a normal near Z.
fn prog_normal_tangent(normal: vec3<f32>) -> vec3<f32> {
    let n = prog_over(normal, sqrt(cy_dot3(normal, normal)));
    let up = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(n.z) < 0.9);
    let t = cross(up, n);
    return prog_over(t, sqrt(cy_dot3(t, t)));
}

// What the bump needs of the surface beyond the shading point: the normals,
// the footprint, the UV derivatives (world space) and the part's place.
struct ProgSurface {
    normal: vec3<f32>,
    geometric: vec3<f32>,
    footprint: f32,
    has_derivatives: bool,
    dpdu: vec3<f32>,
    dpdv: vec3<f32>,
    object_to_world: array<vec4<f32>, 3>,
    world_to_object: array<vec4<f32>, 3>,
};

// What a material's program makes of a point: its parameters, the shading
// normal its bump leaves, and the Tangent it draws in world space (zero
// where it draws none); and the height it found here, for the tests.
struct ProgShaded {
    material: GpuMaterial,
    normal: vec3<f32>,
    drawn: vec3<f32>,
    here: f32,
    heights: vec2<f32>,
};

// The material program `p` at `pt`, written over `base` (shade), then with
// a height bump the bump's program run twice more a step along the surface,
// forward differences against the height found here (surface_bump.rs
// bumped_normal: the step a tenth of the footprint, floored; only the
// position and the UVs move; a normal turned from the geometry is not
// taken). The three runs are one loop round one call of the interpreter,
// which a kernel then holds once.
fn prog_shade(p: GpuProgram, pt: ProgPoint, base: GpuMaterial, s: ProgSurface) -> ProgShaded {
    var shaded = ProgShaded(base, s.normal, vec3<f32>(0.0), 0.0, vec2<f32>(0.0));
    if ((p.flags & PROGRAM_RUNS) == 0u) {
        return shaded;
    }
    let n = s.normal;
    var tangent = prog_normal_tangent(n);
    if (s.has_derivatives) {
        let t = s.dpdu - n * cy_dot3(n, s.dpdu);
        let l2 = cy_dot3(t, t);
        if (l2 > 0.0 && l2 <= 3.4e38) {
            tangent = prog_over(t, sqrt(l2));
        }
    }
    let bitangent = cross(n, tangent);
    let step = max(s.footprint * pt.scale * 0.1, 1e-6);
    let runs = select(1u, 3u, (p.flags & PROGRAM_BUMPS) != 0u);
    prog_constants = p.constants;
    var heights = vec2<f32>(0.0);
    for (var run = 0u; run < runs; run++) {
        var at = pt;
        var start = p.main_start;
        var end = p.main_end;
        if (run > 0u) {
            var offset = tangent * step;
            if (run == 2u) {
                offset = bitangent * step;
            }
            at.object = pt.object + prog_rows_vector(s.world_to_object, offset);
            if (s.has_derivatives) {
                at.uv = pt.uv + prog_uv_offset(s.dpdu, s.dpdv, offset);
            }
            start = p.bump_start;
            end = p.bump_end;
        }
        prog_run(start, end, at);
        if (run == 0u) {
            shaded.material = prog_write_back(p, base);
            if (p.bump_here != PROGRAM_NONE) {
                shaded.here = prog_value(p.bump_here).x;
            }
            if (p.tangent != PROGRAM_NONE) {
                shaded.drawn = prog_rows_vector(s.object_to_world, prog_value(p.tangent).xyz);
            }
        } else {
            let h = prog_value(p.bump_height).x;
            if (run == 1u) {
                heights.x = h;
            } else {
                heights.y = h;
            }
        }
    }
    shaded.heights = heights;
    if (runs == 3u) {
        let along_t = (heights.x - shaded.here) / step;
        let along_b = (heights.y - shaded.here) / step;
        var bumped = prog_normalized(n - tangent * along_t - bitangent * along_b, n);
        if (cy_dot3(bumped, s.geometric) <= 1e-4) {
            bumped = n;
        }
        shaded.normal = bumped;
    }
    return shaded;
}

// Row-major 3 x 4 times a vector (scene.wgsl's rows_vector).
fn prog_rows_vector(m: array<vec4<f32>, 3>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(m[0].xyz, v), dot(m[1].xyz, v), dot(m[2].xyz, v));
}
