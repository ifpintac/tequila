// Tequila's own procedurals (tequila-texture's noise.rs and procedural.rs)
// for the material programs on the card (program.wgsl's OP_PROCEDURAL).
// A procedural is lowered (program.rs) to a few instructions of one
// opcode: stages, which do a pattern's arithmetic, and the two lookups the
// patterns are built on - a fractal of gradient or value noise (fBm,
// turbulence, ridged; one octave of fBm is the noise itself), and Worley's
// cells - so each noise is read in one place however many patterns read
// it, and the interpreter's loop runs a pattern's steps in order.
//
// Rust rounds every product an add reads; the card fuses a product into
// the add after it unless the product goes through cy_exact, so every one
// does. Division is tex_div's and square roots cy_sqrt's, both rounded as
// IEEE rounds them; sine and cosine are cy_sincos's, an ulp or so from the
// processor's.
//#include nodes
//#include textures

// procedural.rs's patterns, by program.rs's procedural_code.
const TQ_SCRATCHES: u32 = 0u;
const TQ_BRUSHED_RADIAL: u32 = 1u;
const TQ_LEATHER: u32 = 2u;

// OP_PROCEDURAL's settings: the step in bits 0-1.
const TQ_STAGE: u32 = 0u;
const TQ_FRACTAL: u32 = 1u;
const TQ_WORLEY: u32 = 2u;
// A stage's pattern in bits 2-6, its step in 7-9, the axis in 10-11, and
// whether its point is a mapping's (u, v) rather than the part's own.
const TQ_MAPPED: u32 = 0x1000u;

// ---- The hashes ----------------------------------------------------------

// noise.rs's hash: three rounds of an integer mixer.
fn tq_hash(c: vec3<i32>, seed: u32) -> u32 {
    var h = bitcast<u32>(c.x) * 0x8da6b343u;
    h ^= bitcast<u32>(c.y) * 0xd8163841u;
    h = h + bitcast<u32>(c.z) * 0xcb1ab31fu;
    h ^= seed * 0x9e3779b9u;
    h ^= h >> 15u;
    h = h * 0x2c1b3c6du;
    h ^= h >> 12u;
    h = h * 0x297a2d39u;
    h ^= h >> 15u;
    return h;
}

// Its top 24 bits over 2^24, exactly.
fn tq_hash01(c: vec3<i32>, seed: u32) -> f32 {
    return f32(tq_hash(c, seed) >> 8u) * (1.0 / 16777216.0);
}

// procedural.rs's own hash01, of two integers.
fn tq_hash2(x: i32, y: i32, seed: u32) -> f32 {
    var h = (bitcast<u32>(x) * 0x8da6b343u) ^ (bitcast<u32>(y) * 0xd8732f1bu);
    h ^= seed * 0x9e3779b9u;
    h ^= h >> 15u;
    h = h * 0x2c1b3c6du;
    h ^= h >> 12u;
    h = h * 0x297a2d39u;
    h ^= h >> 15u;
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// A seed, from the two sixteen-bit halves a constant holds as numbers.
fn tq_seed(halves: vec4<f32>) -> u32 {
    return u32(halves.x) | (u32(halves.y) << 16u);
}

// ---- noise.rs ------------------------------------------------------------

fn tq_quintic(t: f32) -> f32 {
    return t * t * t * (cy_exact(t * (cy_exact(t * 6.0) - 15.0)) + 10.0);
}

fn tq_lerp(a: f32, b: f32, t: f32) -> f32 {
    return a + cy_exact((b - a) * t);
}

// One of the twelve gradient directions.
fn tq_gradient(c: vec3<i32>, seed: u32) -> vec3<f32> {
    switch tq_hash(c, seed) % 12u {
        case 0u: { return vec3<f32>(1.0, 1.0, 0.0); }
        case 1u: { return vec3<f32>(-1.0, 1.0, 0.0); }
        case 2u: { return vec3<f32>(1.0, -1.0, 0.0); }
        case 3u: { return vec3<f32>(-1.0, -1.0, 0.0); }
        case 4u: { return vec3<f32>(1.0, 0.0, 1.0); }
        case 5u: { return vec3<f32>(-1.0, 0.0, 1.0); }
        case 6u: { return vec3<f32>(1.0, 0.0, -1.0); }
        case 7u: { return vec3<f32>(-1.0, 0.0, -1.0); }
        case 8u: { return vec3<f32>(0.0, 1.0, 1.0); }
        case 9u: { return vec3<f32>(0.0, -1.0, 1.0); }
        case 10u: { return vec3<f32>(0.0, 1.0, -1.0); }
        default: { return vec3<f32>(0.0, -1.0, -1.0); }
    }
}

// Gradient noise (perlin, `value` false) or value noise (value) at `p`:
// the eight corners, the first axis fastest, mixed along x, then y, then z.
// A gradient's components are 0 and 1 and -1, so its dot with the offset
// is exact however it is added.
fn tq_basis(p: vec3<f32>, seed: u32, value: bool) -> f32 {
    let fl = floor(p);
    let cell = vec3<i32>(cy_to_i32(fl.x), cy_to_i32(fl.y), cy_to_i32(fl.z));
    let f = p - fl;
    var v: array<f32, 8>;
    for (var k = 0u; k < 8u; k++) {
        let d = vec3<u32>(k & 1u, (k >> 1u) & 1u, (k >> 2u) & 1u);
        let at = cell + vec3<i32>(d);
        if (value) {
            v[k] = tq_hash01(at, seed) * 2.0 - 1.0;
        } else {
            let g = tq_gradient(at, seed);
            let o = f - vec3<f32>(d);
            v[k] = g.x * o.x + g.y * o.y + g.z * o.z;
        }
    }
    let t = vec3<f32>(tq_quintic(f.x), tq_quintic(f.y), tq_quintic(f.z));
    let x00 = tq_lerp(v[0], v[1], t.x);
    let x10 = tq_lerp(v[2], v[3], t.x);
    let x01 = tq_lerp(v[4], v[5], t.x);
    let x11 = tq_lerp(v[6], v[7], t.x);
    let r = tq_lerp(tq_lerp(x00, x10, t.y), tq_lerp(x01, x11, t.y), t.z);
    if (value) {
        return r;
    }
    return clamp(r * 1.3333, -1.0, 1.0);
}

// A fractal of the basis (noise.rs's fbm 0, turbulence 1, ridged 2), its
// point `p` times `k` first: `shape` the scale, lacunarity and gain; the
// settings word the kind (bits 2-3), the value basis (bit 4), the answer
// brought from [-1, 1] to [0, 1] (bit 5), the octaves (from bit 8). The
// basis read in one place, once an octave.
fn tq_fractal(settings: u32, point: vec3<f32>, shape: vec4<f32>, seed: u32) -> f32 {
    let kind = (settings >> 2u) & 3u;
    let value = (settings & 0x10u) != 0u;
    let octaves = max(settings >> 8u, 1u);
    // Every product through cy_exact: the card would fuse the point's into
    // the noise's `p - floor(p)`, the amplitude's into the total.
    var p = cy_exact3(point * shape.x);
    var sum = 0.0;
    var amplitude = 1.0;
    var total = 0.0;
    var weight = 1.0;
    for (var octave = 0u; octave < octaves; octave++) {
        let n = tq_basis(p, seed + octave * 0x9e37u, value);
        switch kind {
            case 1u: { sum = sum + cy_exact(abs(n) * amplitude); }
            case 2u: {
                let r = 1.0 - abs(n);
                let ridge = r * r * weight;
                weight = clamp(ridge * 2.0, 0.0, 1.0);
                sum = sum + cy_exact(ridge * amplitude);
            }
            default: { sum = sum + cy_exact(n * amplitude); }
        }
        total = total + amplitude;
        amplitude = cy_exact(amplitude * shape.z);
        p = cy_exact3(p * shape.y);
    }
    var r = 0.0;
    if (total > 0.0) {
        r = clamp(tex_div(sum, total), select(-1.0, 0.0, kind != 0u), 1.0);
    }
    if ((settings & 0x20u) != 0u) {
        r = r * 0.5 + 0.5;
    }
    return r;
}

fn tq_distance(d: vec3<f32>, metric: u32) -> f32 {
    switch metric {
        case 1u: { return abs(d.x) + abs(d.y) + abs(d.z); }
        case 2u: { return max(max(abs(d.x), abs(d.y)), abs(d.z)); }
        default: { return cy_sqrt(cy_exact(cy_exact(d.x * d.x) + cy_exact(d.y * d.y)) + cy_exact(d.z * d.z)); }
    }
}

// noise.rs's worley: the nearest and next feature points of the 27 cells
// round `p`, by `metric` (Euclidean 0, Manhattan 1, Chebyshev 2), and the
// nearest one's id. (f1, f2, id, 0).
fn tq_worley(p: vec3<f32>, seed: u32, metric: u32) -> vec4<f32> {
    let base = floor(p);
    var f1 = CY_F32_MAX;
    var f2 = CY_F32_MAX;
    var id = 0.0;
    for (var k = 0u; k < 27u; k++) {
        let o = vec3<f32>(f32(k % 3u) - 1.0, f32((k / 3u) % 3u) - 1.0, f32(k / 9u) - 1.0);
        let cell = base + o;
        let c = vec3<i32>(cy_to_i32(cell.x), cy_to_i32(cell.y), cy_to_i32(cell.z));
        let point = cell + vec3<f32>(tq_hash01(c, seed), tq_hash01(c, seed ^ 0x5bf03635u), tq_hash01(c, seed ^ 0x1b873593u));
        let d = tq_distance(point - p, metric);
        if (d < f1) {
            f2 = f1;
            f1 = d;
            id = tq_hash01(c, seed ^ 0x68bc21ebu);
        } else if (d < f2) {
            f2 = d;
        }
    }
    return vec4<f32>(f1, f2, id, 0.0);
}

// ---- procedural.rs -------------------------------------------------------

fn tq_smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (edge1 <= edge0) {
        return select(1.0, 0.0, x < edge0);
    }
    let t = clamp(tex_div(x - edge0, edge1 - edge0), 0.0, 1.0);
    // Through cy_exact: `1 - smoothstep` would take the product into a fused
    // multiply-add.
    return cy_exact(t * t * (3.0 - cy_exact(2.0 * t)));
}

// Axis::across: the two components across the axis (x 0, y 1, z 2).
fn tq_across(axis: u32, p: vec3<f32>) -> vec2<f32> {
    switch axis {
        case 0u: { return p.yz; }
        case 1u: { return p.xz; }
        default: { return p.xy; }
    }
}

// Scratches (procedural.rs's scratches) at (a, b) of its plane: one
// candidate scratch a cell of the 3 x 3 round the point, the distance to
// the nearest over the width. `c` frequency, width, density and spread,
// already floored and clamped as the pattern floors and clamps them.
fn tq_scratches(a: f32, b: f32, c: vec4<f32>, seed: u32) -> f32 {
    let p = vec2<f32>(cy_exact(a * c.x), cy_exact(b * c.x));
    let base = floor(p);
    var nearest = CY_F32_MAX;
    for (var k = 0u; k < 9u; k++) {
        let cell = base + vec2<f32>(f32(k % 3u) - 1.0, f32(k / 3u) - 1.0);
        let cx = cy_to_i32(cell.x);
        let cy = cy_to_i32(cell.y);
        if (tq_hash2(cx, cy, seed ^ 0x51ed2701u) > c.z) {
            continue;
        }
        let centre = cell + vec2<f32>(tq_hash2(cx, cy, seed), tq_hash2(cx, cy, seed ^ 0x2f3911abu));
        let angle = cy_exact((tq_hash2(cx, cy, seed ^ 0x7c931055u) - 0.5) * c.w * CY_TAU);
        let sc = cy_sincos(angle);
        let half_length = 0.2 + cy_exact(tq_hash2(cx, cy, seed ^ 0x11a73c4du) * 0.3);
        let offset = p - centre;
        // The dot with the scratch's direction; its third component is
        // nought by nought, which only turns a -0 to +0.
        let along = clamp(cy_exact(cy_exact(offset.x * sc.y) + cy_exact(offset.y * sc.x)) + 0.0, -half_length, half_length);
        let closest = centre + vec2<f32>(cy_exact(sc.y * along), cy_exact(sc.x * along));
        let d = p - closest;
        nearest = min(nearest, cy_sqrt(cy_exact(cy_exact(d.x * d.x) + cy_exact(d.y * d.y)) + 0.0));
    }
    if (nearest == CY_F32_MAX) {
        return 1.0;
    }
    return clamp(tex_div(nearest, c.y), 0.0, 1.0);
}

// A pattern's step (program.rs's lowering of each): `a` its operands - the
// point first (the part's own, in scene units, or a mapping's u, v), then
// the earlier steps' answers and the constants, as the lowering lays them.
fn tq_stage(settings: u32, a: array<vec4<f32>, 10>) -> vec4<f32> {
    let pattern = (settings >> 2u) & 31u;
    let step = (settings >> 7u) & 7u;
    let axis = (settings >> 10u) & 3u;
    let mapped = (settings & TQ_MAPPED) != 0u;
    // A solid pattern's point: on the plane z = 0 of a mapping's space.
    let p = select(a[0].xyz, vec3<f32>(a[0].xy, 0.0), mapped);
    switch pattern {
        case TQ_SCRATCHES: {
            // Flat: on the mapping's (u, v), or the part's x-z plane.
            let ab = select(a[0].xz, a[0].xy, mapped);
            return vec4<f32>(tq_scratches(ab.x, ab.y, a[1], tq_seed(a[2])));
        }
        case TQ_BRUSHED_RADIAL: {
            // Brushed (Radial): the point the noise is read at, on a circle
            // of its own round the axis. a[1]: frequency, stretch.
            let uv = tq_across(axis, p);
            let radius = cy_sqrt(cy_exact(uv.x * uv.x) + cy_exact(uv.y * uv.y));
            var cu = 0.0;
            var cv = 0.0;
            if (radius > 1e-6) {
                cu = tex_div(uv.x, radius);
                cv = tex_div(uv.y, radius);
            }
            let across = cy_exact(radius * a[1].x);
            let around = tex_div(across, a[1].y);
            return vec4<f32>(across, cy_exact(cu * around), cy_exact(cv * around), 0.0);
        }
        default: {
            if (step == 0u) {
                // Leather: the cells' point. a[1]: frequency.
                return vec4<f32>(p * a[1].x, 0.0);
            }
            // a[1] the cells, a[2] the fine wrinkle, a[3]: crack width,
            // wrinkle.
            let pebble = 1.0 - tq_smoothstep(0.0, 1.0, a[1].x);
            var crack = 1.0;
            if (a[3].x > 0.0) {
                crack = tq_smoothstep(0.0, a[3].x, a[1].y - a[1].x);
            }
            let w = a[3].y;
            return vec4<f32>(clamp((cy_exact(pebble * (1.0 - w)) + cy_exact(a[2].x * w)) * crack, 0.0, 1.0));
        }
    }
}

// One OP_PROCEDURAL instruction: a stage, a fractal (a[0] its point, a[1]
// its scale, lacunarity and gain, a[2] its seed) or Worley's cells (a[0]
// the point, a[1] the seed; the metric in bits 2-3).
fn tq_instruction(settings: u32, a: array<vec4<f32>, 10>) -> vec4<f32> {
    switch settings & 3u {
        case TQ_FRACTAL: { return vec4<f32>(tq_fractal(settings, a[0].xyz, a[1], tq_seed(a[2]))); }
        case TQ_WORLEY: { return tq_worley(a[0].xyz, tq_seed(a[1]), (settings >> 2u) & 3u); }
        default: { return tq_stage(settings, a); }
    }
}

// ---- Cycles' Wave texture (cycles/textures.rs's wave) ---------------------

// The Wave Texture, in two instructions round the fBm its distortion reads,
// which is the Noise Texture's 3D fBM at the detail scale (an OP_NOISE
// between them, program.rs). Bit 5 clear: its point - the coordinates `a`
// times the scale `b`, nudged off the lattice. Set: its value at that point
// `a`, `b` the fBm (read only with a distortion); the kind in bit 0
// (rings), the direction in bits 1-2 (x, y, z, diagonal or spherical), the
// profile in 3-4 (sine, saw, triangle).
fn cy_wave(settings: u32, a: vec3<f32>, b: f32, distortion: f32, phase: f32) -> vec4<f32> {
    if ((settings & 32u) == 0u) {
        return vec4<f32>((cy_exact3(a * b) + vec3<f32>(0.000001)) * 0.999999, 0.0);
    }
    let direction = (settings >> 1u) & 3u;
    var n: f32;
    if ((settings & 1u) != 0u) {
        var r = a;
        if (direction == 0u) {
            r.x = 0.0;
        } else if (direction == 1u) {
            r.y = 0.0;
        } else if (direction == 2u) {
            r.z = 0.0;
        }
        n = cy_sqrt(cy_exact(cy_exact(r.x * r.x) + cy_exact(r.y * r.y)) + cy_exact(r.z * r.z));
        n = cy_exact(n * 20.0);
    } else {
        switch direction {
            case 0u: { n = cy_exact(a.x * 20.0); }
            case 1u: { n = cy_exact(a.y * 20.0); }
            case 2u: { n = cy_exact(a.z * 20.0); }
            default: { n = cy_exact((a.x + a.y + a.z) * 10.0); }
        }
    }
    n = n + phase;
    if (distortion != 0.0) {
        n = n + cy_exact(distortion * (b * 2.0 - 1.0));
    }
    var v: f32;
    switch (settings >> 3u) & 3u {
        case 1u: {
            let m = tex_div(n, CY_TAU);
            v = m - floor(m);
        }
        case 2u: {
            let m = tex_div(n, CY_TAU);
            v = abs(m - floor(m + 0.5)) * 2.0;
        }
        default: { v = 0.5 + 0.5 * cy_sin(n - 1.57079632679489661923); }
    }
    return vec4<f32>(v);
}
