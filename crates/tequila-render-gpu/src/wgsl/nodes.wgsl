// Blender's shader nodes as Cycles computes them (tequila-texture's cycles
// module: hash.rs, noise.rs, voronoi.rs, ops.rs, textures.rs; and
// tequila-materials' nodes.rs and graph.rs for the nodes of Tequila's own),
// for the material programs on the card (program.wgsl). Each is written
// once: Noise and Voronoi over a vec4 point with the dimensions a run-time
// number, rather than Rust's four monomorphised copies.
//
// What the card's arithmetic would otherwise get wrong, done by hand: the
// hashes are integer-only and bit for bit, their integers turned into floats
// rounded to nearest; sine and cosine reduce their argument first (WGSL's
// are only specified on [-pi, pi]); Rust's `%` is a true fmod; a power of a
// negative number to a whole exponent is defined, as powf has it; and a
// float made an integer saturates, as Rust's `as` does.

const CY_TAU: f32 = 6.28318530717958647692;
const CY_PI: f32 = 3.14159265358979323846;
const CY_F32_MAX: f32 = 3.40282346638528859812e38;
const CY_F32_MIN: f32 = -3.40282346638528859812e38;

// ---- Integers and floats -------------------------------------------------

// `n as f32`, rounded to nearest, ties to even, whatever the card's
// conversion rounds to: the high 24 bits exactly, the low 8 exactly, and
// one rounding adding them.
fn cy_u32_to_f32(n: u32) -> f32 {
    return f32(n >> 8u) * 256.0 + f32(n & 0xFFu);
}

// Zero, where the compiler cannot see it is (program.wgsl sets it from a
// word it reads): `cy_exact` passes a product through it, so the product is
// rounded as Rust rounds it rather than fused into what follows.
var<private> cy_zero: u32;

fn cy_exact(x: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(x) | cy_zero);
}

fn cy_exact3(v: vec3<f32>) -> vec3<f32> {
    return bitcast<vec3<f32>>(bitcast<vec3<u32>>(v) | vec3<u32>(cy_zero));
}

fn cy_exact4(v: vec4<f32>) -> vec4<f32> {
    return bitcast<vec4<f32>>(bitcast<vec4<u32>>(v) | vec4<u32>(cy_zero));
}

// a / b rounded as Rust rounds it: the card's quotient, corrected by its
// remainder.
fn cy_div(a: f32, b: f32) -> f32 {
    let q = a / b;
    let r = fma(-q, b, a);
    return fma(r, 1.0 / b, q);
}

// cy_div, and where an operand is nought or not finite, or the quotient
// is not, the card's own, as IEEE has it there (a remainder of
// infinities is not one).
fn cy_quot(a: f32, b: f32) -> f32 {
    let q = a / b;
    let r = fma(-q, b, a);
    let special = a == 0.0 || b == 0.0 || !(abs(a) <= 3.4e38) || !(abs(b) <= 3.4e38) || !(abs(q) <= 3.4e38);
    return select(fma(r, 1.0 / b, q), q, special);
}

fn cy_quot4(a: vec4<f32>, b: f32) -> vec4<f32> {
    return vec4<f32>(cy_quot(a.x, b), cy_quot(a.y, b), cy_quot(a.z, b), cy_quot(a.w, b));
}

// A vector with lane `i` set: a write by a run-time index into a vector,
// done without one (the card's compiler has put such writes in the wrong
// lane inside a loop).
fn cy_set(v: vec4<f32>, i: u32, x: f32) -> vec4<f32> {
    return select(v, vec4<f32>(x), vec4<u32>(0u, 1u, 2u, 3u) == vec4<u32>(i));
}

fn cy_seti(v: vec4<i32>, i: u32, x: i32) -> vec4<i32> {
    return select(v, vec4<i32>(x), vec4<u32>(0u, 1u, 2u, 3u) == vec4<u32>(i));
}

// `x as i32`: saturating, NaN to 0.
fn cy_to_i32(x: f32) -> i32 {
    if (x != x) {
        return 0;
    }
    if (x >= 2147483648.0) {
        return 2147483647;
    }
    if (x <= -2147483648.0) {
        return i32(-2147483647) - 1;
    }
    return i32(x);
}

// Rust's `%` on floats: fmodf, exact (musl's).
fn cy_fmod(x: f32, y: f32) -> f32 {
    var ux = bitcast<u32>(x);
    var uy = bitcast<u32>(y);
    var ex = i32((ux >> 23u) & 0xFFu);
    var ey = i32((uy >> 23u) & 0xFFu);
    let sx = ux & 0x80000000u;
    if ((uy << 1u) == 0u || y != y || ex == 0xFF) {
        return bitcast<f32>(0x7FC00000u);
    }
    if ((ux << 1u) <= (uy << 1u)) {
        if ((ux << 1u) == (uy << 1u)) {
            return bitcast<f32>(sx);
        }
        return x;
    }
    if (ex == 0) {
        var i = ux << 9u;
        loop {
            if ((i >> 31u) != 0u) { break; }
            ex -= 1;
            i = i << 1u;
        }
        ux = ux << u32(1 - ex);
    } else {
        ux = (ux & 0x007FFFFFu) | 0x00800000u;
    }
    if (ey == 0) {
        var i = uy << 9u;
        loop {
            if ((i >> 31u) != 0u) { break; }
            ey -= 1;
            i = i << 1u;
        }
        uy = uy << u32(1 - ey);
    } else {
        uy = (uy & 0x007FFFFFu) | 0x00800000u;
    }
    loop {
        if (ex <= ey) { break; }
        let i = ux - uy;
        if ((i >> 31u) == 0u) {
            if (i == 0u) {
                return bitcast<f32>(sx);
            }
            ux = i;
        }
        ux = ux << 1u;
        ex -= 1;
    }
    let i = ux - uy;
    if ((i >> 31u) == 0u) {
        if (i == 0u) {
            return bitcast<f32>(sx);
        }
        ux = i;
    }
    loop {
        if ((ux >> 23u) != 0u) { break; }
        ux = ux << 1u;
        ex -= 1;
    }
    if (ex > 0) {
        ux = (ux - 0x00800000u) | (u32(ex) << 23u);
    } else {
        ux = ux >> u32(1 - ex);
    }
    return bitcast<f32>(ux | sx);
}

// Sine and cosine of any argument (Cephes' sinf/cosf): reduced by quarter
// turns in three parts, then a polynomial on [-pi/4, pi/4]. (sin, cos).
fn cy_sincos(xx: f32) -> vec2<f32> {
    let x0 = abs(xx);
    var j = cy_to_i32(x0 * 1.27323954473516);
    var y = f32(j);
    if ((j & 1) != 0) {
        j += 1;
        y += 1.0;
    }
    let k = (j >> 1) & 3;
    // In fused steps, which the compiler cannot fold into one constant.
    var x = fma(-y, 0.78515625, x0);
    x = fma(-y, 2.4187564849853515625e-4, x);
    x = fma(-y, 3.77489497744594108e-8, x);
    let z = x * x;
    let s = ((-1.9515295891e-4 * z + 8.3321608736e-3) * z - 1.6666654611e-1) * z * x + x;
    let c = ((2.443315711809948e-5 * z - 1.388731625493765e-3) * z + 4.166664568298827e-2) * z * z - 0.5 * z + 1.0;
    var sine: f32;
    var cosine: f32;
    switch k {
        case 0: { sine = s; cosine = c; }
        case 1: { sine = c; cosine = -s; }
        case 2: { sine = -s; cosine = -c; }
        default: { sine = -c; cosine = s; }
    }
    if (xx < 0.0) {
        sine = -sine;
    }
    if (x0 != x0 || x0 > 3.4e38) {
        return vec2<f32>(bitcast<f32>(0x7FC00000u));
    }
    return vec2<f32>(sine, cosine);
}

fn cy_sin(x: f32) -> f32 {
    return cy_sincos(x).x;
}

fn cy_cos(x: f32) -> f32 {
    return cy_sincos(x).y;
}

// Sine and cosine as the processor's nodes and patterns have them
// (tequila-texture's trig.rs: one implementation on both, as the platform's
// sinf cannot be matched): Cephes' sinf and cosf, the reduction by quarter
// turns and both polynomials in fused steps as written there. (sin, cos).
fn cy_trig(xx: f32) -> vec2<f32> {
    let x0 = abs(xx);
    var j = cy_to_i32(x0 * 1.27323954473516);
    var y = cy_u32_to_f32(u32(j));
    if ((j & 1) != 0) {
        j += 1;
        y += 1.0;
    }
    let k = (j >> 1) & 3;
    var x = fma(-y, 0.78515625, x0);
    x = fma(-y, 2.4187564849853515625e-4, x);
    x = fma(-y, 3.77489497744594108e-8, x);
    let z = cy_exact(x * x);
    let p = fma(fma(-1.9515295891e-4, z, 8.3321608736e-3), z, -1.6666654611e-1);
    let s = fma(cy_exact(p * z), x, x);
    let q = fma(fma(2.443315711809948e-5, z, -1.388731625493765e-3), z, 4.166664568298827e-2);
    let c = fma(cy_exact(q * z), z, fma(-0.5, z, 1.0));
    var sine: f32;
    var cosine: f32;
    switch k {
        case 0: { sine = s; cosine = c; }
        case 1: { sine = c; cosine = -s; }
        case 2: { sine = -s; cosine = -c; }
        default: { sine = -c; cosine = s; }
    }
    if (xx < 0.0) {
        sine = -sine;
    }
    if (x0 != x0 || x0 > 3.4e38) {
        return vec2<f32>(bitcast<f32>(0x7FC00000u));
    }
    return vec2<f32>(sine, cosine);
}

// A square root rounded as Rust rounds it: the card's, corrected by its
// remainder.
fn cy_sqrt(a: f32) -> f32 {
    let s = sqrt(a);
    let r = fma(-s, s, a);
    return select(s, s + r * (0.5 / s), s > 0.0 && s < 1e38);
}

// atan2f to a couple of units in the last place (Cephes' atanf, its
// argument brought under tan(pi/8) first): the card's own is looser, and
// an angle times a radius, wrapped by a pitch, shows it.
fn cy_atan(xx: f32) -> f32 {
    var x = abs(xx);
    var y = 0.0;
    if (x > 2.414213562373095) {
        y = 1.5707963267948966;
        x = -cy_div(1.0, x);
    } else if (x > 0.4142135623730950) {
        y = 0.7853981633974483;
        x = cy_div(x - 1.0, x + 1.0);
    }
    let z = x * x;
    y += (((8.05374449538e-2 * z - 1.38776856032e-1) * z + 1.99777106478e-1) * z - 3.33329491539e-1) * z * x + x;
    return select(y, -y, xx < 0.0);
}

fn cy_atan2(y: f32, x: f32) -> f32 {
    if (x == 0.0) {
        if (y == 0.0) {
            return 0.0;
        }
        return select(1.5707963267948966, -1.5707963267948966, y < 0.0);
    }
    if (y == 0.0) {
        return select(0.0, select(3.141592653589793, -3.141592653589793, (bitcast<u32>(y) & 0x80000000u) != 0u), x < 0.0);
    }
    var w = 0.0;
    if (x < 0.0) {
        w = select(3.141592653589793, -3.141592653589793, y < 0.0);
    }
    return w + cy_atan(cy_div(y, x));
}

// powf: a whole power of a negative number is defined, x^0 is 1 whatever x.
fn cy_powf(x: f32, y: f32) -> f32 {
    if (y == 0.0 || x == 1.0) {
        return 1.0;
    }
    if (x == 0.0) {
        return select(bitcast<f32>(0x7F800000u), 0.0, y > 0.0);
    }
    if (x < 0.0) {
        if (trunc(y) != y) {
            return bitcast<f32>(0x7FC00000u);
        }
        let magnitude = exp2(y * log2(-x));
        let odd = abs(y) < 16777216.0 && (cy_to_i32(y) & 1) != 0;
        return select(magnitude, -magnitude, odd);
    }
    return exp2(y * log2(x));
}

// ---- Cycles' hashes (hash.rs): lookup3 over integer words ----------------

fn cy_rot(x: u32, k: u32) -> u32 {
    return (x << k) | (x >> (32u - k));
}

// hash_uint .. hash_uint4 of the first `n` words of `k`.
fn cy_hash_words(k: vec4<u32>, n: u32) -> u32 {
    let init = 0xdeadbeefu + (n << 2u) + 13u;
    var a = init;
    var b = init;
    var c = init;
    if (n == 4u) {
        a += k.x;
        b += k.y;
        c += k.z;
        a -= c; a ^= cy_rot(c, 4u); c += b;
        b -= a; b ^= cy_rot(a, 6u); a += c;
        c -= b; c ^= cy_rot(b, 8u); b += a;
        a -= c; a ^= cy_rot(c, 16u); c += b;
        b -= a; b ^= cy_rot(a, 19u); a += c;
        c -= b; c ^= cy_rot(b, 4u); b += a;
        a += k.w;
    } else {
        a += k.x;
        if (n >= 2u) { b += k.y; }
        if (n >= 3u) { c += k.z; }
    }
    c ^= b; c -= cy_rot(b, 14u);
    a ^= c; a -= cy_rot(c, 11u);
    b ^= a; b -= cy_rot(a, 25u);
    c ^= b; c -= cy_rot(b, 16u);
    a ^= c; a -= cy_rot(c, 4u);
    b ^= a; b -= cy_rot(a, 14u);
    c ^= b; c -= cy_rot(b, 24u);
    return c;
}

// uint_to_float_incl: [0, 1], 1/u32::MAX as f32 being 2^-32.
fn cy_unit(h: u32) -> f32 {
    return cy_u32_to_f32(h) * 2.3283064365386963e-10;
}

// hash_floatN_to_float of the first `n` floats of `k`, by their bits.
fn cy_hashf(k: vec4<f32>, n: u32) -> f32 {
    return cy_unit(cy_hash_words(bitcast<vec4<u32>>(k), n));
}

// hash_floatN_to_float3, the White Noise Texture's colour and Voronoi's in
// one dimension: three hashes of the key with a component more (or, in four
// dimensions, the key turned), one hash computed in one place.
fn cy_hashf3(k: vec4<f32>, n: u32) -> vec3<f32> {
    var out = vec4<f32>(0.0);
    for (var c = 0u; c < 3u; c++) {
        var key = k;
        var words = n;
        if (c > 0u) {
            if (n == 4u) {
                key = select(vec4<f32>(k.w, k.z, k.y, k.x), vec4<f32>(k.z, k.x, k.w, k.y), c == 1u);
            } else {
                key = cy_set(key, n, f32(c));
                words = n + 1u;
            }
        }
        out = cy_set(out, c, cy_hashf(key, words));
    }
    return out.xyz;
}
// The PCG hashes Voronoi's cells are jittered by, in 2 to 4 dimensions,
// each component in [0, 2^31).
fn cy_pcg(v: vec4<i32>, n: u32) -> vec4<i32> {
    let m = 1664525;
    var x = v.x * m + 1013904223;
    var y = v.y * m + 1013904223;
    var z = v.z * m + 1013904223;
    var w = v.w * m + 1013904223;
    if (n == 2u) {
        x += y * m;
        y += x * m;
        x ^= x >> 16u;
        y ^= y >> 16u;
        x += y * m;
        y += x * m;
        return vec4<i32>(x & 0x7FFFFFFF, y & 0x7FFFFFFF, 0, 0);
    }
    if (n == 3u) {
        x += y * z;
        y += z * x;
        z += x * y;
        x ^= x >> 16u;
        y ^= y >> 16u;
        z ^= z >> 16u;
        x += y * z;
        y += z * x;
        z += x * y;
        return vec4<i32>(x & 0x7FFFFFFF, y & 0x7FFFFFFF, z & 0x7FFFFFFF, 0);
    }
    x += y * w;
    y += z * x;
    z += x * y;
    w += y * z;
    x ^= x >> 16u;
    y ^= y >> 16u;
    z ^= z >> 16u;
    w ^= w >> 16u;
    x += y * w;
    y += z * x;
    z += x * y;
    w += y * z;
    return vec4<i32>(x & 0x7FFFFFFF, y & 0x7FFFFFFF, z & 0x7FFFFFFF, w & 0x7FFFFFFF);
}

// hash_intN_to_floatN: times 1 / 0x7FFFFFFF as f32, which is 2^-31.
fn cy_pcg_unit(v: vec4<i32>, n: u32) -> vec4<f32> {
    let h = bitcast<vec4<u32>>(cy_pcg(v, n));
    let s = 4.656612873077393e-10;
    return vec4<f32>(cy_u32_to_f32(h.x) * s, cy_u32_to_f32(h.y) * s, cy_u32_to_f32(h.z) * s, cy_u32_to_f32(h.w) * s);
}

// ---- Perlin noise and its fractals (noise.rs) ----------------------------

// Every product an add reads through cy_exact, as Rust rounds it: the card
// would fuse it into the add.
fn cy_fade(t: f32) -> f32 {
    return cy_exact(cy_exact(t * t) * t) * (cy_exact(t * (cy_exact(t * 6.0) - 15.0)) + 10.0);
}

fn cy_negate_if(value: f32, condition: u32) -> f32 {
    return select(value, -value, condition != 0u);
}

fn cy_grad(hash: u32, f: vec4<f32>, dims: u32) -> f32 {
    switch dims {
        case 1u: {
            let h = hash & 15u;
            let g = 1.0 + f32(h & 7u);
            return cy_exact(cy_negate_if(g, h & 8u) * f.x);
        }
        case 2u: {
            let h = hash & 7u;
            let u = select(f.y, f.x, h < 4u);
            let v = 2.0 * select(f.x, f.y, h < 4u);
            return cy_negate_if(u, h & 1u) + cy_negate_if(v, h & 2u);
        }
        case 3u: {
            let h = hash & 15u;
            let u = select(f.y, f.x, h < 8u);
            let vt = select(f.z, f.x, h == 12u || h == 14u);
            let v = select(vt, f.y, h < 4u);
            return cy_negate_if(u, h & 1u) + cy_negate_if(v, h & 2u);
        }
        default: {
            let h = hash & 31u;
            let u = select(f.y, f.x, h < 24u);
            let v = select(f.z, f.y, h < 16u);
            let s = select(f.w, f.z, h < 8u);
            return cy_negate_if(u, h & 1u) + cy_negate_if(v, h & 2u) + cy_negate_if(s, h & 4u);
        }
    }
}

// perlin_1d .. perlin_4d: the gradients at the cell's corners, the first
// axis fastest, mixed axis by axis as Cycles mixes them - (1 - t) a + t b,
// but a + t (b - a) for the one axis of 1D and the fourth of 4D.
fn cy_perlin(p: vec4<f32>, dims: u32) -> f32 {
    let fl = floor(p);
    let f = p - fl;
    let cell = vec4<i32>(cy_to_i32(fl.x), cy_to_i32(fl.y), cy_to_i32(fl.z), cy_to_i32(fl.w));
    let corners = 1u << dims;
    var v: array<f32, 16>;
    for (var c = 0u; c < corners; c++) {
        let d = vec4<u32>(c & 1u, (c >> 1u) & 1u, (c >> 2u) & 1u, (c >> 3u) & 1u);
        let at = bitcast<vec4<u32>>(cell + vec4<i32>(d));
        v[c] = cy_grad(cy_hash_words(at, dims), f - vec4<f32>(d), dims);
    }
    let t = vec4<f32>(cy_fade(f.x), cy_fade(f.y), cy_fade(f.z), cy_fade(f.w));
    var count = corners;
    for (var axis = 0u; axis < dims; axis++) {
        count = count >> 1u;
        let s = t[axis];
        let lerp_form = (dims == 1u && axis == 0u) || axis == 3u;
        for (var k = 0u; k < count; k++) {
            let a = v[2u * k];
            let b = v[2u * k + 1u];
            v[k] = select(cy_exact((1.0 - s) * a) + cy_exact(s * b), a + cy_exact(s * (b - a)), lerp_form);
        }
    }
    return v[0];
}

// Signed noise in about [-1, 1] (snoise_1d .. snoise_4d): every lane
// repeated every 100000, half a cell over from a million out, as Cycles
// keeps far coordinates precise.
fn cy_snoise(p: vec4<f32>, dims: u32) -> f32 {
    var w = p;
    for (var i = 0u; i < 4u; i++) {
        let x = p[i];
        if (abs(x) < 100000.0) {
            w = cy_set(w, i, x + 0.0);
        } else {
            w = cy_set(w, i, cy_fmod(x, 100000.0) + select(0.0, 0.5, abs(x) >= 1000000.0));
        }
    }
    let v = cy_perlin(w, dims);
    switch dims {
        case 1u: { return cy_exact(0.25 * v); }
        case 2u: { return cy_exact(0.6616 * v); }
        case 3u: { return cy_exact(0.9820 * v); }
        default: { return cy_exact(0.8344 * v); }
    }
}

// random_floatN_offset: each component in [100, 200].
fn cy_noise_offset(seed: f32, dims: u32) -> vec4<f32> {
    var o = vec4<f32>(0.0);
    for (var i = 0u; i < dims; i++) {
        let one = dims == 1u;
        o = cy_set(o, i, 100.0 + cy_exact(cy_hashf(vec4<f32>(seed, select(f32(i), 0.0, one), 0.0, 0.0), select(2u, 1u, one)) * 100.0));
    }
    return o;
}

struct CyFractal {
    kind: u32,
    dims: u32,
    detail: f32,
    roughness: f32,
    lacunarity: f32,
    offset: f32,
    gain: f32,
    normalize: bool,
};

// The octaves of one fractal (NoiseSettings::select: fBM 0, multifractal 1,
// hybrid multifractal 2, ridged multifractal 3, hetero terrain 4), taken one
// noise at a time so the noise is read in one place: `q` is where the next
// octave reads it (for fBM, the point the scale multiplies), `remainder`
// that the next is the fractional octave, `done` that `value` is the answer.
struct CyOctaves {
    q: vec4<f32>,
    fscale: f32,
    amp: f32,
    maxamp: f32,
    sum: f32,
    value: f32,
    pwr: f32,
    weight: f32,
    signal: f32,
    i: i32,
    remainder: bool,
    done: bool,
};

fn cy_octaves_start(p: vec4<f32>, s: CyFractal) -> CyOctaves {
    return CyOctaves(p, 1.0, 1.0, 0.0, 0.0, select(1.0, 0.0, s.kind == 2u), select(1.0, s.roughness, s.kind >= 3u), 1.0, 0.0, 0, false, false);
}

// Where the next octave reads the noise.
fn cy_octaves_point(st: CyOctaves, s: CyFractal) -> vec4<f32> {
    return select(st.q, cy_exact4(st.fscale * st.q), s.kind == 0u);
}

// One octave's noise `t` taken in, as noise.rs's loops take it.
fn cy_octaves_step(st: ptr<function, CyOctaves>, t: f32, s: CyFractal) {
    let whole = cy_to_i32(s.detail);
    let rmd = s.detail - floor(s.detail);
    switch s.kind {
        case 0u: {
            if (!(*st).remainder) {
                (*st).sum += cy_exact(t * (*st).amp);
                (*st).maxamp += (*st).amp;
                (*st).amp *= s.roughness;
                (*st).fscale *= s.lacunarity;
                (*st).i += 1;
                if ((*st).i > whole) {
                    if (rmd != 0.0) {
                        (*st).remainder = true;
                    } else {
                        (*st).done = true;
                        (*st).value = select((*st).sum, cy_exact(cy_quot(0.5 * (*st).sum, (*st).maxamp)) + 0.5, s.normalize);
                    }
                }
            } else {
                let sum = (*st).sum;
                // The sums through cy_exact too: the card's compiler would
                // take (x + c) - (y + c) for x - y, and (s + d) - s for d.
                let sum2 = cy_exact(sum + cy_exact(t * (*st).amp));
                if (s.normalize) {
                    let a = cy_exact(cy_exact(cy_quot(0.5 * sum, (*st).maxamp)) + 0.5);
                    let b = cy_exact(cy_exact(cy_quot(0.5 * sum2, (*st).maxamp + (*st).amp)) + 0.5);
                    (*st).value = a + cy_exact(rmd * (b - a));
                } else {
                    (*st).value = sum + cy_exact(rmd * (sum2 - sum));
                }
                (*st).done = true;
            }
        }
        case 1u: {
            if (!(*st).remainder) {
                (*st).value *= cy_exact((*st).pwr * t) + 1.0;
                (*st).pwr *= s.roughness;
                (*st).q = cy_exact4(s.lacunarity * (*st).q);
                (*st).i += 1;
                if ((*st).i > whole) {
                    (*st).remainder = rmd != 0.0;
                    (*st).done = rmd == 0.0;
                }
            } else {
                (*st).value *= cy_exact(rmd * (*st).pwr * t) + 1.0;
                (*st).done = true;
            }
        }
        case 2u: {
            if (!(*st).remainder) {
                (*st).weight = min((*st).weight, 1.0);
                let signal = (t + s.offset) * (*st).pwr;
                (*st).pwr *= s.roughness;
                (*st).value += cy_exact((*st).weight * signal);
                (*st).weight *= s.gain * signal;
                (*st).q = cy_exact4(s.lacunarity * (*st).q);
                (*st).i += 1;
                if (!((*st).weight > 0.001 && (*st).i <= whole)) {
                    let more = rmd != 0.0 && (*st).weight > 0.001;
                    (*st).remainder = more;
                    (*st).done = !more;
                }
            } else {
                let weight = min((*st).weight, 1.0);
                let signal = (t + s.offset) * (*st).pwr;
                (*st).value += cy_exact(rmd * weight * signal);
                (*st).done = true;
            }
        }
        case 3u: {
            if ((*st).i == 0) {
                var signal = s.offset - abs(t);
                signal *= signal;
                (*st).signal = signal;
                (*st).value = signal;
            } else {
                let weight = clamp((*st).signal * s.gain, 0.0, 1.0);
                var signal = s.offset - abs(t);
                signal *= signal;
                signal *= weight;
                (*st).signal = signal;
                (*st).value += cy_exact(signal * (*st).pwr);
                (*st).pwr *= s.roughness;
            }
            (*st).i += 1;
            if ((*st).i > whole) {
                (*st).done = true;
            } else {
                (*st).q = cy_exact4(s.lacunarity * (*st).q);
            }
        }
        default: {
            if (!(*st).remainder) {
                if ((*st).i == 0) {
                    (*st).value = s.offset + t;
                } else {
                    let increment = (t + s.offset) * (*st).pwr * (*st).value;
                    (*st).value += cy_exact(increment);
                    (*st).pwr *= s.roughness;
                }
                (*st).q = cy_exact4(s.lacunarity * (*st).q);
                (*st).i += 1;
                if ((*st).i > whole) {
                    (*st).remainder = rmd != 0.0;
                    (*st).done = rmd == 0.0;
                }
            } else {
                let increment = (t + s.offset) * (*st).pwr * (*st).value;
                (*st).value += cy_exact(rmd * increment);
                (*st).done = true;
            }
        }
    }
}

// The Noise Texture node (nodes.rs): its point scaled and distorted, then
// the fractal at it - the Fac in every lane, or with the colour asked for
// (bit 9) the colour, whose red is the Fac, its green and blue the fractal
// again at the colour seeds' offsets. `settings`: dimensions in bits 0-3,
// the fractal in 4-7, normalize bit 8. Every noise it reads - the
// distortion's, each octave of each fractal - is read in one place, one
// loop round it.
fn cy_noise_node(settings: u32, vector: vec3<f32>, w_in: f32, scale: f32, detail: f32, roughness: f32, lacunarity: f32, offset: f32, gain: f32, distortion: f32) -> vec4<f32> {
    let dims = settings & 15u;
    let p3 = cy_exact3(vector * scale);
    let w = cy_exact(w_in * scale);
    var p: vec4<f32>;
    switch dims {
        case 1u: { p = vec4<f32>(w, 0.0, 0.0, 0.0); }
        case 2u: { p = vec4<f32>(p3.x, p3.y, 0.0, 0.0); }
        case 3u: { p = vec4<f32>(p3, 0.0); }
        default: { p = vec4<f32>(p3, w); }
    }
    let s = CyFractal((settings >> 4u) & 15u, dims, clamp(detail, 0.0, 15.0), max(roughness, 0.0), lacunarity, offset, gain, (settings & 0x100u) != 0u);
    let colour = (settings & 0x200u) != 0u;
    let stages = select(1u, 3u, colour);
    var distorting = distortion != 0.0;
    var k = 0u;
    var d = vec4<f32>(0.0);
    var stage = 0u;
    var restart = false;
    var st = cy_octaves_start(p, s);
    var out = vec4<f32>(0.0);
    loop {
        var q = vec4<f32>(0.0);
        if (distorting || restart) {
            // The distortion's offsets 0, 1, 2..., the colour seeds' dims and
            // dims + 1.
            let off = cy_noise_offset(select(f32(dims + stage - 1u), f32(k), distorting), dims);
            if (restart) {
                st = cy_octaves_start(p + off, s);
                restart = false;
            }
            q = p + off;
        }
        if (!distorting) {
            q = cy_octaves_point(st, s);
        }
        let t = cy_snoise(q, dims);
        if (distorting) {
            d = cy_set(d, k, cy_exact(t * distortion));
            k += 1u;
            if (k == dims) {
                p = p + d;
                distorting = false;
                st = cy_octaves_start(p, s);
            }
            continue;
        }
        cy_octaves_step(&st, t, s);
        if (st.done) {
            out = cy_set(out, stage, st.value);
            stage += 1u;
            if (stage == stages) {
                break;
            }
            restart = true;
        }
    }
    if (!colour) {
        return vec4<f32>(out.x);
    }
    return vec4<f32>(out.xyz, 0.0);
}

// The White Noise Texture: its Value in every lane, or the colour (bit 4).
fn cy_white_noise_node(settings: u32, vector: vec3<f32>, w: f32) -> vec4<f32> {
    let dims = settings & 15u;
    var k: vec4<f32>;
    switch dims {
        case 1u: { k = vec4<f32>(w, 0.0, 0.0, 0.0); }
        case 2u: { k = vec4<f32>(vector.x, vector.y, 0.0, 0.0); }
        case 3u: { k = vec4<f32>(vector, 0.0); }
        default: { k = vec4<f32>(vector, w); }
    }
    if ((settings & 16u) == 0u) {
        return vec4<f32>(cy_hashf(k, dims));
    }
    return vec4<f32>(cy_hashf3(k, dims), 0.0);
}

// ---- Voronoi (voronoi.rs) ------------------------------------------------

struct CyVoronoi {
    dims: u32,
    feature: u32,
    metric: u32,
    scale: f32,
    detail: f32,
    roughness: f32,
    lacunarity: f32,
    smoothness: f32,
    exponent: f32,
    randomness: f32,
    max_distance: f32,
    normalize: bool,
    colour: bool,
};

// An octave's answer: distance (or the distance to the edge, or the radius),
// colour, position (W in w).
struct CyCells {
    distance: f32,
    color: vec3<f32>,
    position: vec4<f32>,
};

// The distance between two points of the cells by the metric, and the
// cheaper measure that orders points as it does (distance_bound): (bound,
// distance). In one dimension always the plain difference.
fn cy_cell_distance(a: vec4<f32>, b: vec4<f32>, p: CyVoronoi) -> vec2<f32> {
    if (p.dims == 1u) {
        let d = abs(a.x - b.x);
        return vec2<f32>(d, d);
    }
    var sum = 0.0;
    var most = CY_F32_MIN;
    for (var i = 0u; i < p.dims; i++) {
        let d = a[i] - b[i];
        switch p.metric {
            case 0u: { sum += cy_exact(d * d); }
            case 1u: { sum += abs(d); }
            case 2u: { most = max(most, abs(d)); }
            default: { sum += cy_powf(abs(d), p.exponent); }
        }
    }
    switch p.metric {
        case 0u: { return vec2<f32>(sum, cy_sqrt(sum)); }
        case 1u: { return vec2<f32>(sum); }
        case 2u: { return vec2<f32>(most); }
        default: { return vec2<f32>(sum, cy_powf(sum, 1.0 / p.exponent)); }
    }
}

fn cy_dot_n(a: vec4<f32>, b: vec4<f32>, dims: u32) -> f32 {
    var sum = 0.0;
    for (var i = 0u; i < dims; i++) {
        sum += cy_exact(a[i] * b[i]);
    }
    return sum;
}

// A cell's feature point, relative to the cell the point is in: in one
// dimension on float cells hashed by their bits, else jittered by PCG.
fn cy_cell_point(cellf: vec4<f32>, cell: vec4<i32>, offset: vec4<i32>, p: CyVoronoi) -> vec4<f32> {
    if (p.dims == 1u) {
        let o = f32(offset.x);
        // `cell + offset`, a negative zero made positive as adding a zero
        // makes it - but for the distance to the edge, which hashes the
        // cell itself.
        var key = cellf.x + o;
        if (offset.x == 0) {
            key = select(select(cellf.x, 0.0, cellf.x == 0.0), cellf.x, p.feature == 3u);
        }
        return vec4<f32>(o + cy_exact(cy_hashf(vec4<f32>(key, 0.0, 0.0, 0.0), 1u) * p.randomness), 0.0, 0.0, 0.0);
    }
    let j = cy_pcg_unit(cell + offset, p.dims);
    var out = vec4<f32>(0.0);
    for (var i = 0u; i < p.dims; i++) {
        out = cy_set(out, i, f32(offset[i]) + cy_exact(j[i] * p.randomness));
    }
    return out;
}

// A cell's colour: hash_float_to_float3 of its float cell in one dimension;
// PCG in three of a 2D cell (its third zero), of a 3D one, of a 4D one.
fn cy_cell_color(cellf: vec4<f32>, cell: vec4<i32>, offset: vec4<i32>, dims: u32) -> vec3<f32> {
    if (dims == 1u) {
        var key = cellf.x + f32(offset.x);
        if (offset.x == 0) {
            key = select(cellf.x, 0.0, cellf.x == 0.0);
        }
        return cy_hashf3(vec4<f32>(key, 0.0, 0.0, 0.0), 1u);
    }
    let c = cell + offset;
    return cy_pcg_unit(select(c, vec4<i32>(c.xy, 0, 0), dims == 2u), select(dims, 3u, dims == 2u)).xyz;
}

// The block of cells `reach` each way: its first offset, the next one (the
// first axis fastest, the last outermost, as Cycles' loops nest), how many.
fn cy_first_offset(reach: i32, dims: u32) -> vec4<i32> {
    var o = vec4<i32>(0);
    for (var i = 0u; i < dims; i++) {
        o = cy_seti(o, i, -reach);
    }
    return o;
}

fn cy_next_offset(o_in: vec4<i32>, reach: i32, dims: u32) -> vec4<i32> {
    var o = o_in;
    for (var i = 0u; i < dims; i++) {
        if (o[i] < reach) {
            return cy_seti(o, i, o[i] + 1);
        }
        o = cy_seti(o, i, -reach);
    }
    return o;
}

fn cy_offset_count(reach: i32, dims: u32) -> u32 {
    let width = u32(2 * reach + 1);
    var n = 1u;
    for (var i = 0u; i < dims; i++) {
        n *= width;
    }
    return n;
}

fn cy_mask(v: vec4<f32>, dims: u32) -> vec4<f32> {
    var out = vec4<f32>(0.0);
    for (var i = 0u; i < dims; i++) {
        out = cy_set(out, i, v[i]);
    }
    return out;
}

fn cy_smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if (x < edge0) {
        return 0.0;
    }
    if (x >= edge1) {
        return 1.0;
    }
    let t = cy_div(x - edge0, edge1 - edge0);
    return (3.0 - 2.0 * t) * (t * t);
}

// The walk of the cells in three dimensions by the Euclidean metric, for
// F1, F2 and smooth F1 (cy_cells, which hands it these): the same cells in
// the same order (the first axis fastest), the same arithmetic on the same
// values, but the lanes by constant index and no step of the general walk's
// bookkeeping - the Headphones' grains spent most of their render here
// (docs/07 R3, G1.1). Its answer is the general walk's to the bit
// (tests/graphs.rs), and the processor's: each square rounded, then added,
// as Rust has them.
fn cy_cells3(coord: vec4<f32>, p: CyVoronoi) -> CyCells {
    let cellf = floor(coord);
    let local = (coord - cellf).xyz;
    let cell = vec3<i32>(cy_to_i32(cellf.x), cy_to_i32(cellf.y), cy_to_i32(cellf.z));
    let mode = p.feature;
    let reach = select(1, 2, mode == 2u);
    var best = CY_F32_MAX;
    var second = CY_F32_MAX;
    var best_distance = 0.0;
    var best_offset = vec3<i32>(0);
    var second_offset = vec3<i32>(0);
    var best_point = vec3<f32>(0.0);
    var second_point = vec3<f32>(0.0);
    var smooth_distance = 0.0;
    var smooth_color = vec3<f32>(0.0);
    var smooth_position = vec3<f32>(0.0);
    var h = -1.0;
    var correction = 0.0;
    var out: CyCells;
    out.distance = 0.0;
    out.color = vec3<f32>(0.0);
    out.position = vec4<f32>(0.0);
    let colour_along = mode == 2u && p.colour;
    for (var oz = -reach; oz <= reach; oz++) {
        for (var oy = -reach; oy <= reach; oy++) {
            for (var ox = -reach; ox <= reach; ox++) {
                let o = vec3<i32>(ox, oy, oz);
                let j = cy_pcg_unit(vec4<i32>(cell + o, 0), 3u);
                let point = vec3<f32>(f32(ox), f32(oy), f32(oz)) + cy_exact3(j.xyz * p.randomness);
                let e = point - local;
                let sum = cy_exact(cy_exact(e.x * e.x) + cy_exact(e.y * e.y)) + cy_exact(e.z * e.z);
                let d = vec2<f32>(sum, cy_sqrt(sum));
                if (mode == 0u) {
                    if (d.x < best) {
                        best_offset = o;
                        best = d.x;
                        best_point = point;
                        best_distance = d.y;
                    }
                } else if (mode == 1u) {
                    if (d.y < best) {
                        second = best;
                        best = d.y;
                        second_offset = best_offset;
                        best_offset = o;
                        second_point = best_point;
                        best_point = point;
                    } else if (d.y < second) {
                        second = d.y;
                        second_offset = o;
                        second_point = point;
                    }
                } else {
                    if (h == -1.0) {
                        h = 1.0;
                    } else {
                        h = cy_smoothstep(0.0, 1.0, 0.5 + cy_quot(0.5 * (smooth_distance - d.y), p.smoothness));
                    }
                    correction = p.smoothness * h * (1.0 - h);
                    smooth_distance = smooth_distance + cy_exact(h * (d.y - smooth_distance)) - correction;
                    correction = cy_quot(correction, 1.0 + cy_exact(3.0 * p.smoothness));
                    smooth_position = smooth_position + cy_exact3(h * (point - smooth_position)) - vec3<f32>(correction);
                    if (colour_along) {
                        let color = cy_pcg_unit(vec4<i32>(cell + o, 0), 3u).xyz;
                        smooth_color = smooth_color + cy_exact3(h * (color - smooth_color)) - vec3<f32>(correction);
                    }
                }
            }
        }
    }
    var position = vec3<f32>(0.0);
    switch mode {
        case 0u: {
            out.distance = best_distance;
            position = best_point + cellf.xyz;
        }
        case 1u: {
            out.distance = second;
            position = second_point + cellf.xyz;
        }
        default: {
            out.distance = smooth_distance;
            out.color = smooth_color;
            position = cellf.xyz + smooth_position;
        }
    }
    // F1's and F2's colour, the cell's whose point was taken.
    if (mode <= 1u && p.colour) {
        out.color = cy_pcg_unit(vec4<i32>(cell + select(best_offset, second_offset, mode == 1u), 0), 3u).xyz;
    }
    out.position = vec4<f32>(position, 0.0);
    return out;
}

// One octave of the cells round `coord` (already scaled): in three
// dimensions by the Euclidean metric, F1, F2 and smooth F1 by cy_cells3;
// the rest by the general walk.
fn cy_cells(coord: vec4<f32>, p: CyVoronoi) -> CyCells {
    if (p.dims == 3u && p.metric == 0u && p.feature <= 2u) {
        return cy_cells3(coord, p);
    }
    return cy_cells_general(coord, p);
}

// The general walk of one octave of the cells round `coord` (already
// scaled): F1 0, F2 1, smooth F1 2, the distance to the edge 3 (twice round the cells: the
// nearest point, then the edges to it), the n-sphere radius 4 (the nearest
// point, then its own nearest). One walk, the feature point found in one
// place, the colour - F1's and F2's after the walk, a step of its own - in
// another. In one dimension the position comes back in w, as Cycles' W.
fn cy_cells_general(coord: vec4<f32>, p: CyVoronoi) -> CyCells {
    let cellf = floor(coord);
    let local = cy_mask(coord - cellf, p.dims);
    let cell = vec4<i32>(cy_to_i32(cellf.x), cy_to_i32(cellf.y), cy_to_i32(cellf.z), cy_to_i32(cellf.w));
    let mode = p.feature;
    let reach = select(1, 2, mode == 2u);
    let count = cy_offset_count(reach, p.dims);
    let two_walks = mode == 4u || (mode == 3u && p.dims > 1u);
    let total = select(count, 2u * count, two_walks);
    let colour_after = (mode == 0u || mode == 1u) && (p.colour || p.dims == 1u);
    let colour_along = mode == 2u && (p.colour || p.dims == 1u);
    let steps = total + select(0u, 1u, colour_after);
    var o = cy_first_offset(reach, p.dims);
    var best = CY_F32_MAX;
    var second = CY_F32_MAX;
    var best_distance = 0.0;
    var best_offset = vec4<i32>(0);
    var second_offset = vec4<i32>(0);
    var best_point = vec4<f32>(0.0);
    var second_point = vec4<f32>(0.0);
    var smooth_distance = 0.0;
    var smooth_color = vec3<f32>(0.0);
    var smooth_position = vec4<f32>(0.0);
    var h = -1.0;
    var correction = 0.0;
    var line = vec3<f32>(0.0);
    var out: CyCells;
    out.distance = 0.0;
    out.color = vec3<f32>(0.0);
    out.position = vec4<f32>(0.0);
    for (var n = 0u; n < steps; n++) {
        let second_walk = two_walks && n >= count;
        if (two_walks && n == count) {
            o = cy_first_offset(reach, p.dims);
        }
        let colour_now = colour_after && n == total;
        var at = o;
        if (mode == 4u && second_walk) {
            at = o + best_offset;
        }
        if (colour_now) {
            at = select(best_offset, second_offset, mode == 1u);
        }
        let skip = colour_now || (mode == 4u && second_walk && all(o == vec4<i32>(0)));
        if (!skip) {
            let point = cy_cell_point(cellf, cell, at, p);
            let v = point - local;
            if (mode <= 2u) {
                let d = cy_cell_distance(point, local, p);
                if (mode == 0u) {
                    if (d.x < best) {
                        best_offset = o;
                        best = d.x;
                        best_point = point;
                        best_distance = d.y;
                    }
                } else if (mode == 1u) {
                    if (d.y < best) {
                        second = best;
                        best = d.y;
                        second_offset = best_offset;
                        best_offset = o;
                        second_point = best_point;
                        best_point = point;
                    } else if (d.y < second) {
                        second = d.y;
                        second_offset = o;
                        second_point = point;
                    }
                } else {
                    if (h == -1.0) {
                        h = 1.0;
                    } else {
                        h = cy_smoothstep(0.0, 1.0, 0.5 + cy_quot(0.5 * (smooth_distance - d.y), p.smoothness));
                    }
                    correction = p.smoothness * h * (1.0 - h);
                    smooth_distance = smooth_distance + cy_exact(h * (d.y - smooth_distance)) - correction;
                    correction = cy_quot(correction, 1.0 + cy_exact(3.0 * p.smoothness));
                    smooth_position = smooth_position + cy_exact4(h * (point - smooth_position)) - vec4<f32>(correction);
                }
            } else if (mode == 3u) {
                if (p.dims == 1u) {
                    if (o.x < 0) {
                        line.x = point.x;
                    } else if (o.x == 0) {
                        line.y = point.x;
                    } else {
                        line.z = point.x;
                    }
                } else if (!second_walk) {
                    let d = cy_dot_n(v, v, p.dims);
                    if (d < best) {
                        best = d;
                        best_point = v;
                    }
                } else {
                    let perpendicular = v - best_point;
                    let len2 = cy_dot_n(perpendicular, perpendicular, p.dims);
                    if (len2 > 0.0001) {
                        let len = cy_sqrt(len2);
                        let mid = (best_point + v) * 0.5;
                        let unit = vec4<f32>(cy_quot(perpendicular.x, len), cy_quot(perpendicular.y, len), cy_quot(perpendicular.z, len), cy_quot(perpendicular.w, len));
                        second = min(second, cy_dot_n(mid, unit, p.dims));
                    }
                }
            } else {
                var diff = v;
                if (second_walk) {
                    diff = best_point - point;
                }
                let d = select(cy_dot_n(diff, diff, p.dims), abs(diff.x), p.dims == 1u);
                if (!second_walk) {
                    if (d < best) {
                        best = d;
                        best_point = point;
                        best_offset = o;
                    }
                } else if (d < second) {
                    second = d;
                    second_point = point;
                }
            }
        }
        if (colour_now || (colour_along && n < total)) {
            let color = cy_cell_color(cellf, cell, at, p.dims);
            if (mode == 2u) {
                smooth_color = smooth_color + cy_exact3(h * (color - smooth_color)) - vec3<f32>(correction);
            } else {
                out.color = color;
            }
        }
        o = cy_next_offset(o, reach, p.dims);
    }
    var position = vec4<f32>(0.0);
    switch mode {
        case 0u: {
            out.distance = best_distance;
            position = best_point + cellf;
        }
        case 1u: {
            out.distance = second;
            position = second_point + cellf;
        }
        case 2u: {
            out.distance = smooth_distance;
            out.color = smooth_color;
            position = cellf + smooth_position;
        }
        case 3u: {
            if (p.dims == 1u) {
                out.distance = min(abs((line.y + line.x) * 0.5 - local.x), abs((line.y + line.z) * 0.5 - local.x));
            } else {
                out.distance = second;
            }
        }
        default: {
            let gap = second_point - best_point;
            out.distance = select(cy_sqrt(cy_dot_n(gap, gap, p.dims)), abs(gap.x), p.dims == 1u) * 0.5;
        }
    }
    if (p.dims == 1u) {
        out.position = vec4<f32>(0.0, 0.0, 0.0, position.x);
    } else {
        out.position = cy_mask(position, p.dims);
    }
    return out;
}

// The Voronoi Texture node. `settings`: dimensions bits 0-3, feature 4-7,
// metric 8-11, normalize bit 12, the colour wanted bit 13. Its colour and
// distance (or radius) in the first value, the position and W the second.
// The octaves (fractal, fractal_distance_to_edge) are one loop round one
// walk of the cells.
struct CyVoronoiOut {
    a: vec4<f32>,
    b: vec4<f32>,
};

fn cy_voronoi_node(settings: u32, vector: vec3<f32>, w_in: f32, scale: f32, detail_in: f32, roughness_in: f32, lacunarity: f32, smoothness_in: f32, exponent: f32, randomness_in: f32) -> CyVoronoiOut {
    var p: CyVoronoi;
    p.dims = settings & 15u;
    p.feature = (settings >> 4u) & 15u;
    p.metric = (settings >> 8u) & 15u;
    p.normalize = (settings & 0x1000u) != 0u;
    p.colour = (settings & 0x2000u) != 0u;
    p.scale = scale;
    p.detail = clamp(detail_in, 0.0, 15.0);
    p.roughness = clamp(roughness_in, 0.0, 1.0);
    p.lacunarity = lacunarity;
    p.smoothness = clamp(smoothness_in / 2.0, 0.0, 0.5);
    p.exponent = exponent;
    p.randomness = clamp(randomness_in, 0.0, 1.0);
    let c3 = vector * scale;
    let w = w_in * scale;
    var coord: vec4<f32>;
    switch p.dims {
        case 1u: { coord = vec4<f32>(w, 0.0, 0.0, 0.0); }
        case 2u: { coord = vec4<f32>(c3.x, c3.y, 0.0, 0.0); }
        case 3u: { coord = vec4<f32>(c3, 0.0); }
        default: { coord = vec4<f32>(c3, w); }
    }
    let reach = 0.5 + 0.5 * p.randomness;
    let edge = p.feature == 3u;
    let radius = p.feature == 4u;
    let doubled = select(1.0, 2.0, p.feature == 1u);
    // Smooth F1 with no smoothness is F1.
    if (p.feature == 2u && p.smoothness == 0.0) {
        p.feature = 0u;
    }
    if (edge || p.dims == 1u) {
        p.max_distance = reach * select(doubled, 1.0, edge);
    } else {
        var r = vec4<f32>(0.0);
        for (var k = 0u; k < p.dims; k++) {
            r = cy_set(r, k, reach);
        }
        p.max_distance = cy_cell_distance(vec4<f32>(0.0), r, p).y * doubled;
    }
    let zero_input = p.detail == 0.0 || p.roughness == 0.0;
    let top = ceil(p.detail);
    let remainder = p.detail - floor(p.detail);
    var amplitude = 1.0;
    var max_amplitude = select(0.0, p.max_distance, edge);
    var octave_scale = 1.0;
    var distance = select(0.0, 8.0, edge);
    var color = vec3<f32>(0.0);
    var position = vec4<f32>(0.0);
    var i = 0;
    loop {
        if (!(f32(i) <= top)) {
            break;
        }
        let octave = cy_cells(cy_exact4(coord * octave_scale), p);
        if (radius) {
            distance = octave.distance;
            break;
        }
        if (zero_input) {
            max_amplitude = select(1.0, max_amplitude, edge);
            distance = octave.distance;
            color = octave.color;
            position = octave.position;
            break;
        }
        if (f32(i) <= p.detail) {
            if (edge) {
                max_amplitude = max_amplitude + cy_exact(amplitude * (cy_quot(p.max_distance, octave_scale) - max_amplitude));
                distance = distance + cy_exact(amplitude * (min(distance, cy_quot(octave.distance, octave_scale)) - distance));
            } else {
                max_amplitude += amplitude;
                distance += cy_exact(octave.distance * amplitude);
                color += cy_exact3(octave.color * amplitude);
                position = position + cy_exact4(amplitude * (cy_quot4(octave.position, octave_scale) - position));
            }
            octave_scale *= p.lacunarity;
            amplitude *= p.roughness;
        } else if (remainder != 0.0) {
            if (edge) {
                let lerp_amplitude = cy_exact(max_amplitude + cy_exact(amplitude * (cy_quot(p.max_distance, octave_scale) - max_amplitude)));
                max_amplitude = max_amplitude + cy_exact(remainder * (lerp_amplitude - max_amplitude));
                let lerp_distance = cy_exact(distance + cy_exact(amplitude * (min(distance, cy_quot(octave.distance, octave_scale)) - distance)));
                distance = distance + cy_exact(remainder * (min(distance, lerp_distance) - distance));
            } else {
                max_amplitude = max_amplitude + cy_exact(remainder * (cy_exact(max_amplitude + amplitude) - max_amplitude));
                distance = distance + cy_exact(remainder * (cy_exact(distance + cy_exact(octave.distance * amplitude)) - distance));
                color = color + cy_exact3(remainder * (cy_exact3(color + cy_exact3(octave.color * amplitude)) - color));
                let toward = cy_exact4(position + cy_exact4(amplitude * (cy_quot4(octave.position, octave_scale) - position)));
                position = position + cy_exact4(remainder * (toward - position));
            }
        }
        i++;
    }
    var out: CyVoronoiOut;
    out.a = vec4<f32>(0.0);
    out.b = vec4<f32>(0.0);
    if (radius) {
        out.a.w = distance;
        return out;
    }
    if (edge) {
        if (p.normalize) {
            distance = cy_quot(distance, max_amplitude);
        }
        out.a.w = distance;
        return out;
    }
    if (p.normalize) {
        distance = cy_quot(distance, max_amplitude * p.max_distance);
        color = vec3<f32>(cy_quot(color.x, max_amplitude), cy_quot(color.y, max_amplitude), cy_quot(color.z, max_amplitude));
    }
    if (p.scale != 0.0) {
        position = cy_quot4(position, p.scale);
    } else {
        position = vec4<f32>(0.0);
    }
    out.a = vec4<f32>(color, distance);
    out.b = position;
    return out;
}

// ---- Converters (ops.rs) -------------------------------------------------

fn cy_safe_divide(a: f32, b: f32) -> f32 {
    return select(0.0, cy_div(a, b), b != 0.0);
}

fn cy_compatible_powf(x: f32, y: f32) -> f32 {
    if (y == 0.0) {
        return 1.0;
    }
    if (x == 0.0) {
        return 0.0;
    }
    return cy_powf(x, y);
}

fn cy_safe_powf(a: f32, b: f32) -> f32 {
    if (a < 0.0 && b != f32(cy_to_i32(b))) {
        return 0.0;
    }
    return cy_compatible_powf(a, b);
}

fn cy_signf(f: f32) -> f32 {
    if (f == 0.0) {
        return 0.0;
    }
    return select(1.0, -1.0, f < 0.0);
}

fn cy_floored_fmod(a: f32, b: f32) -> f32 {
    return select(0.0, a - cy_exact(floor(cy_div(a, b)) * b), b != 0.0);
}

fn cy_smoothmin(a: f32, b: f32, k: f32) -> f32 {
    if (k != 0.0) {
        let h = max(k - abs(a - b), 0.0) / k;
        return min(a, b) - h * h * h * k * (1.0 / 6.0);
    }
    return min(a, b);
}

fn cy_wrapf(value: f32, maximum: f32, minimum: f32) -> f32 {
    let range = maximum - minimum;
    return select(minimum, value - cy_exact(range * floor(cy_div(value - minimum, range))), range != 0.0);
}

fn cy_pingpong(a: f32, b: f32) -> f32 {
    if (b != 0.0) {
        let x = cy_div(a - b, b * 2.0);
        return abs(cy_exact((x - floor(x)) * b) * 2.0 - b);
    }
    return 0.0;
}

// MathOp::apply, by MathOp::ALL's order.
fn cy_math(op: u32, a: f32, b: f32, c: f32) -> f32 {
    // Sine, cosine and tangent from one reduction, in one place.
    var sc = vec2<f32>(0.0, 1.0);
    if (op >= 29u && op <= 31u) {
        sc = cy_trig(a);
    }
    switch op {
        case 0u: { return a + b; }
        case 1u: { return a - b; }
        case 2u: { return a * b; }
        case 3u: { return cy_safe_divide(a, b); }
        case 4u: { return cy_exact(a * b) + c; }
        case 5u: { return cy_safe_powf(a, b); }
        case 6u: {
            if (a <= 0.0 || b <= 0.0) { return 0.0; }
            return cy_safe_divide(log(a), log(b));
        }
        case 7u: { return cy_sqrt(max(a, 0.0)); }
        case 8u: { return select(0.0, cy_div(1.0, cy_sqrt(a)), a > 0.0); }
        case 9u: { return abs(a); }
        case 10u: { return exp(a); }
        case 11u: { return min(a, b); }
        case 12u: { return max(a, b); }
        case 13u: { return select(0.0, 1.0, a < b); }
        case 14u: { return select(0.0, 1.0, a > b); }
        case 15u: { return cy_signf(a); }
        case 16u: { return select(0.0, 1.0, a == b || abs(a - b) <= max(c, 1.1920929e-7)); }
        case 17u: { return cy_smoothmin(a, b, c); }
        case 18u: { return -cy_smoothmin(-a, -b, c); }
        case 19u: { return floor(a + 0.5); }
        case 20u: { return floor(a); }
        case 21u: { return ceil(a); }
        case 22u: { return select(ceil(a), floor(a), a >= 0.0); }
        case 23u: { return a - floor(a); }
        case 24u: { return select(0.0, cy_fmod(a, b), b != 0.0); }
        case 25u: { return cy_floored_fmod(a, b); }
        case 26u: { return cy_wrapf(a, b, c); }
        case 27u: { return floor(select(0.0, cy_div(a, b), b != 0.0)) * b; }
        case 28u: { return cy_pingpong(a, b); }
        case 29u: { return sc.x; }
        case 30u: { return sc.y; }
        case 31u: { return cy_quot(sc.x, sc.y); }
        case 32u: { return asin(clamp(a, -1.0, 1.0)); }
        case 33u: { return acos(clamp(a, -1.0, 1.0)); }
        case 34u: { return cy_atan(a); }
        case 35u: { return select(cy_atan2(a, b), 0.0, a == 0.0 && b == 0.0); }
        case 36u: { return sinh(a); }
        case 37u: { return cosh(a); }
        case 38u: { return tanh(a); }
        case 39u: { return a * (CY_PI / 180.0); }
        default: { return a * (180.0 / CY_PI); }
    }
}

fn cy_safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let t = cy_sqrt(cy_dot3x(v, v));
    return select(v, v * cy_quot(1.0, t), t != 0.0);
}

fn cy_dot3(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return a.x * b.x + a.y * b.y + a.z * b.z;
}

// The dot product with each product rounded as Rust rounds it.
fn cy_dot3x(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return cy_exact(a.x * b.x) + cy_exact(a.y * b.y) + cy_exact(a.z * b.z);
}

// One component of the Vector Math operations that work component by
// component and call something heavy: divide 3, power 16, sign 17, modulo
// 23, wrap 24, snap 25, sine 26, cosine 27, tangent 28.
fn cy_vector_component(op: u32, a: f32, b: f32, c: f32) -> f32 {
    var sc = vec2<f32>(0.0, 1.0);
    if (op >= 26u) {
        sc = cy_trig(a);
    }
    switch op {
        case 3u: { return cy_safe_divide(a, b); }
        case 16u: { return cy_safe_powf(a, b); }
        case 17u: { return cy_signf(a); }
        case 23u: { return select(cy_fmod(a, b), 0.0, b == 0.0); }
        case 24u: { return cy_floored_fmod(a - c, b - c) + c; }
        case 25u: { return floor(select(0.0, cy_div(a, b), b != 0.0)) * b; }
        case 26u: { return sc.x; }
        case 27u: { return sc.y; }
        default: { return cy_quot(sc.x, sc.y); }
    }
}

// VectorMathOp::apply, by VectorMathOp::ALL's order: the vector in xyz, or
// a value in every lane for the three that give one.
fn cy_vector_math(op: u32, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, param: f32) -> vec4<f32> {
    switch op {
        case 3u, 16u, 17u, 23u, 24u, 25u, 26u, 27u, 28u: {
            var out = vec4<f32>(0.0);
            for (var i = 0u; i < 3u; i++) {
                out = cy_set(out, i, cy_vector_component(op, a[i], b[i], c[i]));
            }
            return out;
        }
        case 0u: { return vec4<f32>(a + b, 0.0); }
        case 1u: { return vec4<f32>(a - b, 0.0); }
        case 2u: { return vec4<f32>(a * b, 0.0); }
        case 4u: { return vec4<f32>(cy_exact3(a * b) + c, 0.0); }
        case 5u: { return vec4<f32>(cy_exact(a.y * b.z) - cy_exact(a.z * b.y), cy_exact(a.z * b.x) - cy_exact(a.x * b.z), cy_exact(a.x * b.y) - cy_exact(a.y * b.x), 0.0); }
        case 6u: {
            let l2 = cy_dot3x(b, b);
            return vec4<f32>(select(vec3<f32>(0.0), b * cy_quot(cy_dot3x(a, b), l2), l2 != 0.0), 0.0);
        }
        case 7u: {
            let n = cy_safe_normalize(b);
            return vec4<f32>(a - cy_exact3(n * (2.0 * cy_dot3x(a, n))), 0.0);
        }
        case 8u: {
            let n = cy_safe_normalize(b);
            let eta = param;
            let d = cy_dot3x(n, a);
            let k = 1.0 - cy_exact(eta * eta * (1.0 - cy_exact(d * d)));
            return vec4<f32>(select(cy_exact3(a * eta) - cy_exact3(n * (cy_exact(eta * d) + cy_sqrt(k))), vec3<f32>(0.0), k < 0.0), 0.0);
        }
        case 9u: { return vec4<f32>(select(-a, a, cy_dot3x(c, b) < 0.0), 0.0); }
        case 10u: { return vec4<f32>(cy_dot3x(a, b)); }
        case 11u: {
            let d = a - b;
            return vec4<f32>(cy_sqrt(cy_dot3x(d, d)));
        }
        case 12u: { return vec4<f32>(cy_sqrt(cy_dot3x(a, a))); }
        case 13u: { return vec4<f32>(a * param, 0.0); }
        case 14u: { return vec4<f32>(cy_safe_normalize(a), 0.0); }
        case 15u: { return vec4<f32>(abs(a), 0.0); }
        case 18u: { return vec4<f32>(min(a, b), 0.0); }
        case 19u: { return vec4<f32>(max(a, b), 0.0); }
        case 20u: { return vec4<f32>(floor(a), 0.0); }
        case 21u: { return vec4<f32>(ceil(a), 0.0); }
        default: { return vec4<f32>(a - floor(a), 0.0); }
    }
}

fn cy_smootherstep(edge0: f32, edge1: f32, x_in: f32) -> f32 {
    let x = clamp(cy_safe_divide(x_in - edge0, edge1 - edge0), 0.0, 1.0);
    return x * x * x * cy_exact(x * cy_exact(cy_exact(x * 6.0) - 15.0) + 10.0);
}

// map_range: linear 0, stepped 1, smoothstep 2, smootherstep 3; clamp bit 4.
fn cy_map_range(settings: u32, value: f32, from_min: f32, from_max: f32, to_min: f32, to_max: f32, steps: f32) -> f32 {
    if (from_max == from_min) {
        return 0.0;
    }
    let kind = settings & 15u;
    var factor: f32;
    switch kind {
        case 0u: { factor = cy_div(value - from_min, from_max - from_min); }
        case 1u: {
            let f = cy_div(value - from_min, from_max - from_min);
            factor = select(0.0, floor(f * (steps + 1.0)) / steps, steps > 0.0);
        }
        case 2u: {
            factor = select(cy_smoothstep(from_min, from_max, value), 1.0 - cy_smoothstep(from_max, from_min, value), from_min > from_max);
        }
        default: {
            factor = select(cy_smootherstep(from_min, from_max, value), 1.0 - cy_smootherstep(from_max, from_min, value), from_min > from_max);
        }
    }
    let result = to_min + cy_exact(factor * (to_max - to_min));
    if ((settings & 16u) != 0u && kind <= 1u) {
        return clamp(result, min(to_min, to_max), max(to_min, to_max));
    }
    return result;
}

fn cy_rgb_to_hsv(rgb: vec3<f32>) -> vec3<f32> {
    let cmax = max(rgb.x, max(rgb.y, rgb.z));
    let cmin = min(rgb.x, min(rgb.y, rgb.z));
    let cdelta = cmax - cmin;
    let v = cmax;
    let s = select(0.0, cdelta / cmax, cmax != 0.0);
    var h = 0.0;
    if (s != 0.0) {
        let c = (vec3<f32>(cmax) - rgb) / cdelta;
        if (rgb.x == cmax) {
            h = c.z - c.y;
        } else if (rgb.y == cmax) {
            h = 2.0 + c.x - c.z;
        } else {
            h = 4.0 + c.y - c.x;
        }
        h /= 6.0;
        if (h < 0.0) {
            h += 1.0;
        }
    }
    return vec3<f32>(h, s, v);
}

fn cy_hsv_to_rgb(hsv: vec3<f32>) -> vec3<f32> {
    var h = hsv.x;
    let s = hsv.y;
    let v = hsv.z;
    if (s == 0.0) {
        return vec3<f32>(v);
    }
    if (h == 1.0) {
        h = 0.0;
    }
    h *= 6.0;
    let i = floor(h);
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    if (i == 0.0) { return vec3<f32>(v, t, p); }
    if (i == 1.0) { return vec3<f32>(q, v, p); }
    if (i == 2.0) { return vec3<f32>(p, v, t); }
    if (i == 3.0) { return vec3<f32>(p, q, v); }
    if (i == 4.0) { return vec3<f32>(t, p, v); }
    return vec3<f32>(v, p, q);
}

fn cy_rgb_to_hsl(rgb: vec3<f32>) -> vec3<f32> {
    let cmax = max(rgb.x, max(rgb.y, rgb.z));
    let cmin = min(rgb.x, min(rgb.y, rgb.z));
    let l = min(1.0, (cmax + cmin) / 2.0);
    var h = 0.0;
    var s = 0.0;
    if (cmax != cmin) {
        let cdelta = cmax - cmin;
        s = select(cdelta / (cmax + cmin), cdelta / (2.0 - cmax - cmin), l > 0.5);
        if (cmax == rgb.x) {
            h = (rgb.y - rgb.z) / cdelta + select(0.0, 6.0, rgb.y < rgb.z);
        } else if (cmax == rgb.y) {
            h = (rgb.z - rgb.x) / cdelta + 2.0;
        } else {
            h = (rgb.x - rgb.y) / cdelta + 4.0;
        }
    }
    h /= 6.0;
    return vec3<f32>(h, s, l);
}

fn cy_hsl_to_rgb(hsl: vec3<f32>) -> vec3<f32> {
    let h = hsl.x;
    let s = hsl.y;
    let l = hsl.z;
    let nr = clamp(abs(h * 6.0 - 3.0) - 1.0, 0.0, 1.0);
    let ng = clamp(2.0 - abs(h * 6.0 - 2.0), 0.0, 1.0);
    let nb = clamp(2.0 - abs(h * 6.0 - 4.0), 0.0, 1.0);
    let chroma = (1.0 - abs(2.0 * l - 1.0)) * s;
    return vec3<f32>((nr - 0.5) * chroma + l, (ng - 0.5) * chroma + l, (nb - 0.5) * chroma + l);
}

// ColorModel::separate and combine: RGB 0, HSV 1, HSL 2.
fn cy_separate_color(model: u32, c: vec3<f32>) -> vec3<f32> {
    switch model {
        case 1u: { return cy_rgb_to_hsv(c); }
        case 2u: { return cy_rgb_to_hsl(c); }
        default: { return c; }
    }
}

fn cy_combine_color(model: u32, c: vec3<f32>) -> vec3<f32> {
    switch model {
        case 1u: { return cy_hsv_to_rgb(c); }
        case 2u: { return cy_hsl_to_rgb(c); }
        default: { return c; }
    }
}

fn cy_interp(a: vec3<f32>, b: vec3<f32>, t: f32) -> vec3<f32> {
    return a + cy_exact3((b - a) * t);
}

// BlendMode::blend of c2 over c1 by t, by BlendMode::ALL's order; the four
// modes through HSV (hue 15, saturation 16, colour 17, value 18) convert
// each way in one place.
fn cy_blend(mode: u32, t: f32, c1: vec3<f32>, c2: vec3<f32>) -> vec3<f32> {
    let tm = 1.0 - t;
    let one = vec3<f32>(1.0);
    if (mode >= 15u) {
        let hsv1 = cy_rgb_to_hsv(c1);
        let hsv2 = cy_rgb_to_hsv(c2);
        var hsv = hsv1;
        var back = true;
        switch mode {
            case 15u: {
                if (hsv2.y == 0.0) { return c1; }
                hsv.x = hsv2.x;
            }
            case 16u: {
                if (hsv1.y == 0.0) { return c1; }
                hsv.y = tm * hsv1.y + t * hsv2.y;
                back = false;
            }
            case 17u: {
                if (hsv2.y == 0.0) { return c1; }
                hsv.x = hsv2.x;
                hsv.y = hsv2.y;
            }
            default: {
                hsv.z = tm * hsv1.z + t * hsv2.z;
                back = false;
            }
        }
        let rgb = cy_hsv_to_rgb(hsv);
        return select(rgb, cy_interp(c1, rgb, t), back);
    }
    switch mode {
        case 0u: { return cy_interp(c1, c2, t); }
        case 1u: { return cy_interp(c1, min(c1, c2), t); }
        case 2u: { return cy_interp(c1, c1 * c2, t); }
        case 3u: {
            var out = vec4<f32>(0.0);
            for (var i = 0u; i < 3u; i++) {
                let tmp = tm + t * c2[i];
                out = cy_set(out, i, select(clamp(1.0 - (1.0 - c1[i]) / tmp, 0.0, 1.0), 0.0, tmp <= 0.0));
            }
            return out.xyz;
        }
        case 4u: { return cy_interp(c1, max(c1, c2), t); }
        case 5u: { return one - (vec3<f32>(tm) + (one - c2) * t) * (one - c1); }
        case 6u: {
            var out = vec4<f32>(0.0);
            for (var i = 0u; i < 3u; i++) {
                let o = c1[i];
                let tmp = 1.0 - t * c2[i];
                if (o == 0.0) {
                    out = cy_set(out, i, o);
                } else if (tmp <= 0.0) {
                    out = cy_set(out, i, 1.0);
                } else {
                    out = cy_set(out, i, min(o / tmp, 1.0));
                }
            }
            return out.xyz;
        }
        case 7u: { return cy_interp(c1, c1 + c2, t); }
        case 8u: {
            var out = vec4<f32>(0.0);
            for (var i = 0u; i < 3u; i++) {
                let o = c1[i];
                out = cy_set(out, i, select(1.0 - (tm + 2.0 * t * (1.0 - c2[i])) * (1.0 - o), o * (tm + 2.0 * t * c2[i]), o < 0.5));
            }
            return out.xyz;
        }
        case 9u: {
            let scr = one - (one - c2) * (one - c1);
            return c1 * tm + ((one - c1) * c2 * c1 + c1 * scr) * t;
        }
        case 10u: { return c1 + (c2 * 2.0 + vec3<f32>(-1.0)) * t; }
        case 11u: { return cy_interp(c1, abs(c1 - c2), t); }
        case 12u: {
            let excluded = c1 + c2 - 2.0 * c1 * c2;
            return max(cy_interp(c1, excluded, t), vec3<f32>(0.0));
        }
        case 13u: { return cy_interp(c1, c1 - c2, t); }
        default: {
            var out = vec4<f32>(0.0);
            for (var i = 0u; i < 3u; i++) {
                out = cy_set(out, i, select(c1[i], tm * c1[i] + t * c1[i] / c2[i], c2[i] != 0.0));
            }
            return out.xyz;
        }
    }
}

// The Mix node: data float 0, vector 1, colour 2 in bits 0-1, the blend in
// 2-7, clamp the factor bit 8, the result bit 9.
fn cy_mix_node(settings: u32, fac: f32, a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    let t = select(fac, clamp(fac, 0.0, 1.0), (settings & 0x100u) != 0u);
    switch settings & 3u {
        case 0u: { return vec4<f32>(a.x + cy_exact(t * (b.x - a.x))); }
        case 1u: { return vec4<f32>(a.xyz + cy_exact3(t * (b.xyz - a.xyz)), 0.0); }
        default: {
            var out = cy_blend((settings >> 2u) & 63u, t, a.xyz, b.xyz);
            if ((settings & 0x200u) != 0u) {
                out = clamp(out, vec3<f32>(0.0), vec3<f32>(1.0));
            }
            return vec4<f32>(out, 0.0);
        }
    }
}

// The rotation Cycles builds from Euler angles, applied (or transposed).
fn cy_mapping_node(kind: u32, vector: vec3<f32>, location: vec3<f32>, rotation: vec3<f32>, scale: vec3<f32>) -> vec3<f32> {
    var sines = vec4<f32>(0.0);
    var cosines = vec4<f32>(1.0);
    for (var i = 0u; i < 3u; i++) {
        let sc = cy_trig(rotation[i]);
        sines = cy_set(sines, i, sc.x);
        cosines = cy_set(cosines, i, sc.y);
    }
    let sx = sines.x;
    let sy = sines.y;
    let sz = sines.z;
    let cx = cosines.x;
    let cy = cosines.y;
    let cz = cosines.z;
    let r0 = vec3<f32>(cy * cz, cy_exact(sy * sx * cz) - cy_exact(cx * sz), cy_exact(sy * cx * cz) + cy_exact(sx * sz));
    let r1 = vec3<f32>(cy * sz, cy_exact(sy * sx * sz) + cy_exact(cx * cz), cy_exact(sy * cx * sz) - cy_exact(sx * cz));
    let r2 = vec3<f32>(-sy, cy * sx, cy * cx);
    switch kind {
        case 0u: {
            let v = vector * scale;
            return vec3<f32>(cy_dot3x(r0, v), cy_dot3x(r1, v), cy_dot3x(r2, v)) + location;
        }
        case 1u: {
            let v = vector - location;
            let t = vec3<f32>(cy_dot3x(vec3<f32>(r0.x, r1.x, r2.x), v), cy_dot3x(vec3<f32>(r0.y, r1.y, r2.y), v), cy_dot3x(vec3<f32>(r0.z, r1.z, r2.z), v));
            return vec3<f32>(cy_safe_divide(t.x, scale.x), cy_safe_divide(t.y, scale.y), cy_safe_divide(t.z, scale.z));
        }
        case 2u: {
            let v = vector * scale;
            return vec3<f32>(cy_dot3x(r0, v), cy_dot3x(r1, v), cy_dot3x(r2, v));
        }
        default: {
            let s = vec3<f32>(cy_safe_divide(vector.x, scale.x), cy_safe_divide(vector.y, scale.y), cy_safe_divide(vector.z, scale.z));
            let v = vec3<f32>(cy_dot3x(r0, s), cy_dot3x(r1, s), cy_dot3x(r2, s));
            let t = cy_sqrt(cy_dot3x(v, v));
            return select(v, v * cy_quot(1.0, t), t != 0.0);
        }
    }
}

// The Clamp node; with Range, either way round.
fn cy_clamp_node(range: bool, value: f32, lo: f32, hi: f32) -> f32 {
    if (range && lo > hi) {
        return clamp(value, hi, lo);
    }
    return min(max(value, lo), hi);
}

fn cy_hsv_node(color: vec3<f32>, hue: f32, saturation: f32, value: f32, fac: f32) -> vec3<f32> {
    var hsv = cy_rgb_to_hsv(color);
    let h = hsv.x + hue + 0.5;
    hsv.x = h - floor(h);
    hsv.y = clamp(hsv.y * saturation, 0.0, 1.0);
    hsv.z *= value;
    let out = cy_hsv_to_rgb(hsv);
    return max(fac * out + (1.0 - fac) * color, vec3<f32>(0.0));
}

fn cy_gamma_node(color: vec3<f32>, gamma: f32) -> vec3<f32> {
    if (gamma == 0.0) {
        return vec3<f32>(1.0);
    }
    var out = vec4<f32>(color, 0.0);
    for (var i = 0u; i < 3u; i++) {
        if (color[i] > 0.0) {
            out = cy_set(out, i, cy_powf(color[i], gamma));
        }
    }
    return out.xyz;
}

fn cy_checker(p_in: vec3<f32>) -> f32 {
    let p = (p_in + vec3<f32>(0.000001)) * 0.999999;
    let xi = abs(cy_to_i32(floor(p.x)));
    let yi = abs(cy_to_i32(floor(p.y)));
    let zi = abs(cy_to_i32(floor(p.z)));
    return select(0.0, 1.0, ((xi % 2) == (yi % 2)) == ((zi % 2) != 0));
}

fn cy_gradient_texture(kind: u32, p: vec3<f32>) -> f32 {
    var f: f32;
    switch kind {
        case 0u: { f = p.x; }
        case 1u: {
            let r = max(p.x, 0.0);
            f = r * r;
        }
        case 2u: {
            let r = clamp(p.x, 0.0, 1.0);
            let t = r * r;
            f = 3.0 * t - 2.0 * t * r;
        }
        case 3u: { f = (p.x + p.y) * 0.5; }
        case 6u: { f = cy_atan2(p.y, p.x) / CY_TAU + 0.5; }
        default: {
            let r = max(0.999999 - sqrt(p.x * p.x + p.y * p.y + p.z * p.z), 0.0);
            f = select(r, r * r, kind == 5u);
        }
    }
    return clamp(f, 0.0, 1.0);
}

// fresnel_dielectric_cos.
fn cy_fresnel_dielectric_cos(cosi: f32, eta: f32) -> f32 {
    let c = abs(cosi);
    var g = eta * eta - 1.0 + c * c;
    if (g > 0.0) {
        g = sqrt(g);
        let a = (g - c) / (g + c);
        let b = (c * (g + c) - 1.0) / (c * (g - c) + 1.0);
        return 0.5 * a * a * (1.0 + b * b);
    }
    return 1.0;
}

// Layer Weight's Fresnel and Facing.
fn cy_layer_weight(blend: f32, n: vec3<f32>, incoming: vec3<f32>, backfacing: bool) -> vec2<f32> {
    let cosine = incoming.x * n.x + incoming.y * n.y + incoming.z * n.z;
    var eta = max(1.0 - blend, 1e-5);
    eta = select(1.0 / eta, eta, backfacing);
    let fresnel = cy_fresnel_dielectric_cos(cosine, eta);
    var facing = abs(cosine);
    if (blend != 0.5) {
        var b = clamp(blend, 0.0, 1.0 - 1e-5);
        b = select(0.5 / (1.0 - b), 2.0 * b, b < 0.5);
        facing = cy_powf(facing, b);
    }
    return vec2<f32>(fresnel, 1.0 - facing);
}

fn cy_fresnel(ior: f32, n: vec3<f32>, incoming: vec3<f32>, backfacing: bool) -> f32 {
    var eta = max(ior, 1e-5);
    eta = select(eta, 1.0 / eta, backfacing);
    return cy_fresnel_dielectric_cos(incoming.x * n.x + incoming.y * n.y + incoming.z * n.z, eta);
}
