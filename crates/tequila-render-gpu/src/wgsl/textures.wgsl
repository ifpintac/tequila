// Pictures (textures.rs): every level of every pyramid a material samples,
// as tequila-texture keeps it, read and filtered as the processor reads and
// filters it - level.rs's decode, mip.rs's bilinear and trilinear lookup
// and its level for a footprint, texture.rs's lookups through a mapping
// and at a place the graph worked out, projection.rs's projections - and
// shade.rs's colour_of. One instruction (program.rs's OP_TEXTURE) reaches
// all of it from one call: the projection is called from one place (in a
// loop for a normal map's moved points), the lookup from one (in a loop
// over the contributions), a level's bilinear lookup from one (over one or
// two levels) and a texel from one (over the four taps).
//
// Rust adds every product it rounds; the card fuses a product into the add
// after it unless the product goes through cy_exact first, so every
// product an add reads does. Divisions are tex_div's, rounded as IEEE
// division rounds; square roots cy_sqrt's. The arctangent is correctly
// rounded, as the processor's atan2f is all but a few times in a million,
// and so is powf (the tri-planar weights); acos (spherical v) is the
// card's, within an ulp or two of the processor's.
//#include nodes

@group(0) @binding(16) var<storage, read> texture_texels: array<u32>;
@group(0) @binding(17) var<storage, read> texture_header: array<u32>;

// textures.rs's layout of the header.
const TEX_TABLE_UNORM8: u32 = 256u;
const TEX_TABLE_LOG2: u32 = 512u;
const TEX_LOG2_K: u32 = 544u;
const TEX_TABLE_ATAN: u32 = 548u;
const TEX_HALF_PI: u32 = 582u;
const TEX_PI: u32 = 584u;
const TEX_TABLE_EXP2: u32 = 588u;
const TEX_LN2: u32 = 622u;
const TEX_PYRAMIDS: u32 = 624u;

// program.rs's settings word.
const TEX_MODE_COLOUR: u32 = 0u;
const TEX_MODE_ALPHA: u32 = 1u;
const TEX_MODE_PROJECT: u32 = 2u;
const TEX_MODE_NORMAL: u32 = 3u;
const TEX_FLIP_U: u32 = 0x800u;
const TEX_FLIP_V: u32 = 0x1000u;
const TEX_REAL_WORLD: u32 = 0x2000u;
const TEX_RIGID: u32 = 0x4000u;
const TEX_ROTATES: u32 = 0x8000u;
const TEX_AT: u32 = 0x10000u;
const TEX_MANAGED: u32 = 0x20000u;

const TEX_WRAP_REPEAT: u32 = 0u;
const TEX_WRAP_CLAMP: u32 = 1u;
const TEX_WRAP_MIRROR: u32 = 2u;

// a / b rounded as IEEE division rounds it: the card's reciprocal sharpened
// by a Newton step, the quotient corrected once by its exact remainder
// (Markstein); for the finite, normal operands a lookup divides by.
fn tex_div(a: f32, b: f32) -> f32 {
    var y = 1.0 / b;
    let e = fma(-b, y, 1.0);
    y = fma(e, y, y);
    let q = cy_exact(a * y);
    let r = fma(-q, b, a);
    return select(fma(r, y, q), a / b, a == 0.0 || !(abs(b) <= 3.4e38) || b == 0.0 || !(abs(a) <= 3.4e38));
}

// a + b exactly, as (sum, error) (Knuth's TwoSum), each step through
// cy_exact: the card's compiler otherwise sees that (a + b) - a is b, and
// reassociates the error's two halves until it is nought.
fn tex_two_sum(a: f32, b: f32) -> vec2<f32> {
    let s = cy_exact(a + b);
    let bb = cy_exact(s - a);
    return vec2<f32>(s, cy_exact(a - cy_exact(s - bb)) + cy_exact(b - bb));
}

// atan2f, correctly rounded (the processor's is, all but a few times in a
// million): the angle of the smaller over the larger, t in [0, 1], as
// atan(c) + atan((t - c) / (1 + t c)) with c = k/16 nearest t and atan(c)
// from the header's table, each part in two floats; then a quarter turn
// less where the axes were swapped, a half turn less for x below nought,
// the sign of y. Zeros as atan2f has them.
fn tex_atan2(y: f32, x: f32) -> f32 {
    let negative_x = (bitcast<u32>(x) & 0x80000000u) != 0u;
    let negative_y = (bitcast<u32>(y) & 0x80000000u) != 0u;
    let ax = abs(x);
    let ay = abs(y);
    if (ay == 0.0) {
        let pi = bitcast<f32>(texture_header[TEX_PI]);
        return select(select(0.0, pi, negative_x), select(-0.0, -pi, negative_x), negative_y);
    }
    let swap = ay > ax;
    let num = select(ay, ax, swap);
    let den = select(ax, ay, swap);
    // t = num / den in two parts.
    let t = tex_div(num, den);
    let t_low = tex_div(fma(-t, den, num), den);
    let k = u32(t * 16.0 + 0.5);
    let c = f32(k) * 0.0625;
    // (t - c) in two parts; t - c itself is exact.
    let n = tex_two_sum(t - c, t_low);
    // 1 + t c in two parts.
    let p = cy_exact(t * c);
    let p_err = fma(t, c, -p) + t_low * c;
    let d = cy_exact(1.0 + p);
    let d_low = (p - cy_exact(d - 1.0)) + p_err;
    // r = n / d in two parts.
    let r = tex_div(n.x, d);
    let rem = fma(-r, d, n.x) + n.y - r * d_low;
    let r_low = tex_div(rem, d);
    let r2 = r * r;
    let corr = r * r2 * (-0.33333334 + r2 * (0.2 - r2 * 0.14285715));
    let th = bitcast<f32>(texture_header[TEX_TABLE_ATAN + 2u * k]);
    let tl = bitcast<f32>(texture_header[TEX_TABLE_ATAN + 2u * k + 1u]);
    var a = tex_two_sum(th, r);
    a.y = a.y + (tl + (r_low + corr));
    if (swap) {
        let h = tex_two_sum(bitcast<f32>(texture_header[TEX_HALF_PI]), -a.x);
        a = vec2<f32>(h.x, h.y + (bitcast<f32>(texture_header[TEX_HALF_PI + 1u]) - a.y));
    }
    if (negative_x) {
        let h = tex_two_sum(bitcast<f32>(texture_header[TEX_PI]), -a.x);
        a = vec2<f32>(h.x, h.y + (bitcast<f32>(texture_header[TEX_PI + 1u]) - a.y));
    }
    let angle = a.x + a.y;
    return select(angle, -angle, negative_y);
}

// Wrap::apply: Repeat t - floor(t); Clamp to [0, 1], a NaN kept; Mirror
// folding alternate tiles, its fract Rust's (x - trunc(x)).
fn tex_wrap(mode: u32, t: f32) -> f32 {
    if (mode == TEX_WRAP_CLAMP) {
        return select(select(t, 1.0, t > 1.0), 0.0, t < 0.0);
    }
    if (mode == TEX_WRAP_MIRROR) {
        let half = t * 0.5;
        var two = (half - trunc(half)) * 2.0;
        if (two < 0.0) {
            two = two + 2.0;
        }
        if (two > 1.0) {
            two = 2.0 - two;
        }
        return two;
    }
    return t - floor(t);
}

// Wrap::is_inside.
fn tex_inside(mode: u32, t: f32) -> bool {
    return mode != TEX_WRAP_CLAMP || (t >= 0.0 && t <= 1.0);
}

// Value `i` of a level (Level::colour): an sRGB byte through the curve's
// table, a byte over 255 by its table, sixteen bits over 65535, a half
// float, a float. `level` is its record: first word, width, height,
// encoding | stored << 8.
fn tex_value(level: vec4<u32>, i: u32) -> f32 {
    let encoding = level.w & 0xFFu;
    if (encoding <= 1u) {
        let b = (texture_texels[level.x + (i >> 2u)] >> ((i & 3u) * 8u)) & 0xFFu;
        return bitcast<f32>(texture_header[encoding * TEX_TABLE_UNORM8 + b]);
    }
    if (encoding == 4u) {
        return bitcast<f32>(texture_texels[level.x + i]);
    }
    let h = (texture_texels[level.x + (i >> 1u)] >> ((i & 1u) * 16u)) & 0xFFFFu;
    if (encoding == 2u) {
        return tex_unorm16(h);
    }
    return unpack2x16float(h).x;
}

// v / 65535 rounded as IEEE division rounds it, from the bits: v / 65535 =
// N / (2^32 - 1) with N = v * 65537, so its binary digits are N's 32 again
// and again. The 24 after the leading one, rounded up where the next is a
// 1 (the digits after that are never all nought, so there is no tie).
fn tex_unorm16(v: u32) -> f32 {
    if (v == 0u || v == 65535u) {
        return f32(v / 65535u);
    }
    let n = v * 65537u;
    let lz = countLeadingZeros(n);
    var m: u32;
    var up: u32;
    if (lz < 8u) {
        m = n >> (8u - lz);
        up = (n >> (7u - lz)) & 1u;
    } else {
        // The rest from the next copy of N; none needed at exactly 8.
        m = (n << (lz - 8u)) | select(n >> (40u - lz), 0u, lz == 8u);
        up = (n >> (39u - lz)) & 1u;
    }
    return ldexp(f32(m + up), -i32(lz + 24u));
}

// Level::texel: (x, y) as linear RGB, a grey level as three equal ones.
fn tex_texel(level: vec4<u32>, x: u32, y: u32) -> vec3<f32> {
    let stored = level.w >> 8u;
    let at = (y * level.y + x) * stored;
    let count = select(3u, 1u, stored < 3u);
    var c: array<f32, 3>;
    for (var k = 0u; k < count; k++) {
        c[k] = tex_value(level, at + k);
    }
    if (stored < 3u) {
        return vec3<f32>(c[0]);
    }
    return vec3<f32>(c[0], c[1], c[2]);
}

// MipPyramid::bilinear: texel centres at half-integers, the four taps
// clamped or wrapped (rem_euclid) by `wrap`, blended across then up.
fn tex_bilinear(level: vec4<u32>, u: f32, v: f32, wrap: u32) -> vec3<f32> {
    let w = i32(level.y);
    let h = i32(level.z);
    let x = cy_exact(tex_wrap(wrap, u) * f32(level.y)) - 0.5;
    let y = cy_exact(tex_wrap(wrap, v) * f32(level.z)) - 0.5;
    let x0 = floor(x);
    let y0 = floor(y);
    let fx = x - x0;
    let fy = y - y0;
    let ix = cy_to_i32(x0);
    let iy = cy_to_i32(y0);
    var c: array<vec3<f32>, 4>;
    for (var k = 0u; k < 4u; k++) {
        var tx = ix + i32(k & 1u);
        var ty = iy + i32(k >> 1u);
        if (wrap == TEX_WRAP_CLAMP) {
            tx = clamp(tx, 0, w - 1);
            ty = clamp(ty, 0, h - 1);
        } else {
            tx = ((tx % w) + w) % w;
            ty = ((ty % h) + h) % h;
        }
        c[k] = tex_texel(level, u32(tx), u32(ty));
    }
    let gx = 1.0 - fx;
    let top = cy_exact3(c[0] * gx) + cy_exact3(c[1] * fx);
    let bottom = cy_exact3(c[2] * gx) + cy_exact3(c[3] * fx);
    return cy_exact3(top * (1.0 - fy)) + cy_exact3(bottom * fy);
}

// log2 of x >= 1, correctly rounded (the processor's log2f is within an ulp
// of it, and agrees with it all but about once in a thousand): x = 2^e m,
// m = c (1 + r) with c = 1 + k/16 from the header's table, log2 m =
// log2 c + (2 / ln 2) atanh((m - c) / (m + c)), each part carried in two
// floats and added once at the end. Below 1 the level is nought whatever
// the logarithm, and -1 says so.
fn tex_log2(x: f32) -> f32 {
    if (!(x >= 1.0)) {
        return -1.0;
    }
    let parts = tex_log2_parts(x);
    return parts.x + parts.y;
}

// log2 of a positive normal x in two floats, their sum within 2^-40 of it
// (tex_log2's working).
fn tex_log2_parts(x: f32) -> vec2<f32> {
    let bits = bitcast<u32>(x);
    let e = f32(i32(bits >> 23u) - 127);
    let m = bitcast<f32>((bits & 0x007FFFFFu) | 0x3F800000u);
    let k = (bits >> 19u) & 15u;
    let c = 1.0 + f32(k) * 0.0625;
    // m - c exactly; m + c in two parts (Fast2Sum, m >= c).
    // Every step of the two-part sums through cy_exact: the card's compiler
    // otherwise sees that (a + b) - a is b and leaves the error out
    // (tex_two_sum).
    let f = m - c;
    let d = cy_exact(m + c);
    let d_low = c - cy_exact(d - m);
    let s = f / d;
    var r = fma(-s, d, f);
    r = fma(-s, d_low, r);
    let s_low = r / d;
    let s2 = s * s;
    let corr = s * s2 * (0.33333334 + s2 * 0.2);
    let lo = s_low + corr;
    let a = cy_exact(s + lo);
    let a_low = lo - cy_exact(a - s);
    let kh = bitcast<f32>(texture_header[TEX_LOG2_K]);
    let kl = bitcast<f32>(texture_header[TEX_LOG2_K + 1u]);
    let p = cy_exact(kh * a);
    let p_low = fma(kh, a, -p) + (kh * a_low + kl * a);
    let th = bitcast<f32>(texture_header[TEX_TABLE_LOG2 + 2u * k]);
    let tl = bitcast<f32>(texture_header[TEX_TABLE_LOG2 + 2u * k + 1u]);
    // e + th + p, exactly in two parts (TwoSum twice), then the rest.
    let s1 = cy_exact(e + th);
    let b1 = cy_exact(s1 - e);
    let l1 = cy_exact(e - cy_exact(s1 - b1)) + cy_exact(th - b1);
    let s3 = cy_exact(s1 + p);
    let b3 = cy_exact(s3 - s1);
    let l3 = cy_exact(s1 - cy_exact(s3 - b3)) + cy_exact(p - b3);
    return vec2<f32>(s3, ((l1 + l3) + tl) + p_low);
}

// x^k for x in [0, 1] and k >= 1, correctly rounded, as the processor's
// powf is all but about once in ten thousand: 2^(k log2 x), the logarithm
// and the product in two floats, 2^f as 2^(j/16) from the header's table
// times e^(g) for |g| under ln 2 / 32, the rest of a series in one float.
fn tex_powf(x: f32, k: f32) -> f32 {
    if (x == 1.0) {
        return 1.0;
    }
    if (!(x >= 1.17549435e-38)) {
        return 0.0;
    }
    let l = tex_log2_parts(x);
    let yh = cy_exact(k * l.x);
    let yl = fma(k, l.x, -yh) + k * l.y;
    if (yh < -160.0) {
        return 0.0;
    }
    let n = floor(yh + 0.5);
    let f0 = yh - n;
    let j = floor(f0 * 16.0 + 0.5);
    let ff = tex_two_sum(f0 - j * 0.0625, yl);
    let ln2h = bitcast<f32>(texture_header[TEX_LN2]);
    let ln2l = bitcast<f32>(texture_header[TEX_LN2 + 1u]);
    let gh = cy_exact(ff.x * ln2h);
    let gl = fma(ff.x, ln2h, -gh) + (ff.x * ln2l + ff.y * ln2h);
    let poly = gh * gh * (0.5 + gh * (0.16666667 + gh * (0.041666668 + gh * 0.008333334)));
    let rest = gl + poly;
    let at = TEX_TABLE_EXP2 + 2u * u32(j + 8.0);
    let th = bitcast<f32>(texture_header[at]);
    let tl = bitcast<f32>(texture_header[at + 1u]);
    let a = cy_exact(th * gh);
    let a_err = fma(th, gh, -a);
    let sum = tex_two_sum(th, a);
    let r = sum.x + (sum.y + (a_err + (th * rest + tl * (1.0 + gh))));
    return ldexp(r, i32(n));
}

// MipPyramid::sample: the level whose texels match the footprint `width`
// (level_for), bilinear within it, blended with the next where the level is
// between two.
fn tex_pyramid_sample(pyramid: u32, u: f32, v: f32, width: f32, wrap: u32) -> vec3<f32> {
    let record = TEX_PYRAMIDS + 4u * pyramid;
    let first = texture_header[record];
    let count = texture_header[record + 1u];
    let resolution = bitcast<f32>(texture_header[record + 2u]);
    let wide = width * resolution;
    let texels = select(1e-8, wide, wide > 1e-8);
    let level = min(max(tex_log2(texels), 0.0), f32(count - 1u));
    let lower = floor(level);
    let blend = level - lower;
    let low = u32(lower);
    let two = blend > 0.0 && low + 1u < count;
    var c: array<vec3<f32>, 2>;
    for (var k = 0u; k < select(1u, 2u, two); k++) {
        let at = first + 4u * min(low + k, count - 1u);
        let rec = vec4<u32>(texture_header[at], texture_header[at + 1u], texture_header[at + 2u], texture_header[at + 3u]);
        c[k] = tex_bilinear(rec, u, v, wrap);
    }
    if (!two) {
        return c[0];
    }
    return cy_exact3(c[0] * (1.0 - blend)) + cy_exact3(c[1] * blend);
}

// texture.rs's sample_axes: one wrap for both axes, or each axis wrapped
// first and the lookup clamped.
fn tex_lookup(pyramid: u32, u_in: f32, v_in: f32, width: f32, wrap_u: u32, wrap_v: u32) -> vec3<f32> {
    var u = u_in;
    var v = v_in;
    var wrap = wrap_u;
    if (wrap_u != wrap_v) {
        u = tex_wrap(wrap_u, u_in);
        v = tex_wrap(wrap_v, v_in);
        wrap = TEX_WRAP_CLAMP;
    }
    return tex_pyramid_sample(pyramid, u, v, width, wrap);
}

// `s`, or 1e-12 with its sign where it is smaller (Mapping::spin's safe).
fn tex_safe(s: f32) -> f32 {
    let tiny = select(1e-12, -1e-12, (bitcast<u32>(s) & 0x80000000u) != 0u);
    return select(s, tiny, abs(s) < 1e-12);
}

// Mapping::finish_anisotropic on projected (u, v) and the footprint along
// each: the scale as repeats (scale.offset's xy), the turn about the pivot
// (turn: cos, sin, pivot), the offset, the flips; (u, v, width()).
fn tex_finish(settings: u32, scale_offset: vec4<f32>, turn: vec4<f32>, u_in: f32, v_in: f32, fu: f32, fv: f32) -> vec3<f32> {
    let su = scale_offset.x;
    let sv = scale_offset.y;
    var u = cy_exact(u_in * su);
    var v = cy_exact(v_in * sv);
    if ((settings & TEX_ROTATES) != 0u) {
        let cs = turn.x;
        let sn = turn.y;
        let pu = u - turn.z;
        let pv = v - turn.w;
        if ((settings & TEX_RIGID) != 0u) {
            let du = tex_div(pu, tex_safe(su));
            let dv = tex_div(pv, tex_safe(sv));
            u = cy_exact((cy_exact(du * cs) - cy_exact(dv * sn)) * su) + turn.z;
            v = cy_exact((cy_exact(du * sn) + cy_exact(dv * cs)) * sv) + turn.w;
        } else {
            u = (cy_exact(pu * cs) - cy_exact(pv * sn)) + turn.z;
            v = (cy_exact(pu * sn) + cy_exact(pv * cs)) + turn.w;
        }
    }
    u = u + scale_offset.z;
    v = v + scale_offset.w;
    if ((settings & TEX_FLIP_U) != 0u) {
        u = -u;
    }
    if ((settings & TEX_FLIP_V) != 0u) {
        v = -v;
    }
    return vec3<f32>(u, v, max(fu * abs(su), fv * abs(sv)));
}

// The axis's two components across it and the one along it (Axis::across,
// Axis::of): (a, b, along).
fn tex_across(axis: u32, p: vec3<f32>) -> vec3<f32> {
    if (axis == 0u) {
        return vec3<f32>(p.y, p.z, p.x);
    }
    if (axis == 1u) {
        return vec3<f32>(p.x, p.z, p.y);
    }
    return p;
}

// What a projection makes of a point: up to three (u, v, width, weight).
struct TexProjected {
    c: array<vec4<f32>, 3>,
    count: u32,
};

// What a lookup through a mapping needs of the shading point
// (SurfacePoint): the part's own space, its normal there, the UVs, the
// footprint, the part's scale, and the footprint in UV where it is known.
struct TexPoint {
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    footprint: f32,
    scale: f32,
    uv_footprint: vec2<f32>,
    has_uv_footprint: bool,
};

// Mapping::project: the frame's units (in_frame_units), the projection's
// coordinates and footprints, and finish_anisotropic on each.
fn tex_project(settings: u32, scale_offset: vec4<f32>, turn: vec4<f32>, sharpness: f32, pt: TexPoint) -> TexProjected {
    let projection = (settings >> 2u) & 7u;
    var p = pt.position;
    var fp = pt.footprint;
    if ((settings & TEX_REAL_WORLD) != 0u && projection != 0u && pt.scale != 1.0) {
        p = p * pt.scale;
        fp = fp * pt.scale;
    }
    // Before finishing: (u, v, footprint across, footprint up) each.
    var raw: array<vec4<f32>, 3>;
    var weights = vec3<f32>(1.0, 0.0, 0.0);
    var count = 1u;
    // The angles the angular projections read, from one arctangent: the
    // cylinder's turn; the sphere's turn and its angle from the pole.
    let q = tex_across((settings >> 5u) & 3u, p);
    let sphere = cy_sqrt(cy_exact(cy_exact(p.x * p.x) + cy_exact(p.y * p.y)) + cy_exact(p.z * p.z));
    let d = p * tex_div(1.0, sphere);
    let pole = clamp(d.y, -1.0, 1.0);
    var towards: array<vec2<f32>, 2>;
    var angles = 0u;
    if (projection == 2u) {
        towards[0] = q.yx;
        angles = 1u;
    } else if (projection == 3u && !(sphere < 1e-6)) {
        towards[0] = d.zx;
        towards[1] = vec2<f32>(cy_sqrt(cy_exact((1.0 - pole) * (1.0 + pole))), pole);
        angles = 2u;
    }
    var angle = array<f32, 2>(0.0, 0.0);
    for (var k = 0u; k < angles; k++) {
        angle[k] = tex_atan2(towards[k].x, towards[k].y);
    }
    switch projection {
        case 0u: {
            let f = select(vec2<f32>(fp), pt.uv_footprint, pt.has_uv_footprint);
            raw[0] = vec4<f32>(pt.uv, f);
        }
        case 1u: {
            raw[0] = vec4<f32>(q.x, q.y, fp, fp);
        }
        case 2u: {
            let u = tex_div(angle[0], CY_TAU) + 0.5;
            let radius = max(cy_sqrt(cy_exact(q.x * q.x) + cy_exact(q.y * q.y)), 1e-6);
            let angular = tex_div(fp, radius * CY_TAU);
            raw[0] = vec4<f32>(u, q.z, angular, fp);
        }
        case 3u: {
            if (sphere < 1e-6) {
                raw[0] = vec4<f32>(0.0, 0.5, fp, fp);
            } else {
                let u = tex_div(angle[0], CY_TAU) + 0.5;
                let v = tex_div(angle[1], CY_PI);
                let angular = tex_div(fp, sphere * CY_TAU);
                raw[0] = vec4<f32>(u, v, angular, angular * 2.0);
            }
        }
        default: {
            let n = abs(pt.normal);
            let along = array<f32, 3>(n.x, n.y, n.z);
            var powered: array<f32, 3>;
            for (var k = 0u; k < 3u; k++) {
                powered[k] = tex_powf(along[k], sharpness);
            }
            var w = vec3<f32>(powered[0], powered[1], powered[2]);
            let total = (w.x + w.y) + w.z;
            if (total <= 1e-12) {
                w = vec3<f32>(1.0 / 3.0);
            } else {
                w = vec3<f32>(tex_div(w.x, total), tex_div(w.y, total), tex_div(w.z, total));
            }
            let planes = array<vec2<f32>, 3>(vec2<f32>(p.z, p.y), vec2<f32>(p.x, p.z), vec2<f32>(p.x, p.y));
            let all = array<f32, 3>(w.x, w.y, w.z);
            count = 0u;
            var kept = array<f32, 3>(0.0, 0.0, 0.0);
            for (var k = 0u; k < 3u; k++) {
                if (all[k] > 1e-4) {
                    raw[count] = vec4<f32>(planes[k], fp, fp);
                    kept[count] = all[k];
                    count += 1u;
                }
            }
            weights = vec3<f32>(kept[0], kept[1], kept[2]);
        }
    }
    var out: TexProjected;
    out.count = count;
    let weight = array<f32, 3>(weights.x, weights.y, weights.z);
    for (var k = 0u; k < count; k++) {
        let r = raw[k];
        out.c[k] = vec4<f32>(tex_finish(settings, scale_offset, turn, r.x, r.y, r.z, r.w), weight[k]);
    }
    return out;
}

// Vec3::dot, left to right as Rust adds it.
fn tex_dot(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return cy_exact(cy_exact(a.x * b.x) + cy_exact(a.y * b.y)) + cy_exact(a.z * b.z);
}

// Vec3::cross.
fn tex_cross(a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        cy_exact(a.y * b.z) - cy_exact(a.z * b.y),
        cy_exact(a.z * b.x) - cy_exact(a.x * b.z),
        cy_exact(a.x * b.y) - cy_exact(a.y * b.x),
    );
}

// Vec3::try_normalize: (the unit vector, 1), or (0, 0) for a zero or
// non-finite one.
fn tex_normalized(v: vec3<f32>) -> vec4<f32> {
    let l2 = tex_dot(v, v);
    if (l2 > 0.0 && l2 <= CY_F32_MAX) {
        return vec4<f32>(v * tex_div(1.0, cy_sqrt(l2)), 1.0);
    }
    return vec4<f32>(0.0);
}

// `v` square to `n`, made a unit vector (try_normalize(v - n n.v)).
fn tex_square_to(n: vec3<f32>, v: vec3<f32>) -> vec4<f32> {
    return tex_normalized(v - cy_exact3(n * tex_dot(n, v)));
}

// TangentFrame::decode_normal_map at strength 1: the texel a direction in
// the frame (tangent, bitangent, n), its blue kept above the surface.
fn tex_decode_normal(texel: vec3<f32>, n: vec3<f32>, tangent: vec3<f32>, bitangent: vec3<f32>) -> vec3<f32> {
    let x = texel.x * 2.0 - 1.0;
    let y = texel.y * 2.0 - 1.0;
    let zz = texel.z * 2.0 - 1.0;
    let z = select(1e-4, zz, zz > 1e-4);
    let world = cy_exact3(cy_exact3(tangent * x) + cy_exact3(bitangent * y)) + cy_exact3(n * z);
    let unit = tex_normalized(world);
    return select(n, unit.xyz, unit.w != 0.0);
}

// A normal map through the UVs (surface_bump.rs bumped_normal's NormalMap
// branch): the texel decoded in the frame the UVs run in - the mesh's own
// tangent where it has one (`mesh_tangent`: world space, its handedness in
// w, nought for none), else the UVs' u and up the image; `n` where there
// are no UVs or no frame.
fn tex_uv_normal(texel: vec3<f32>, n: vec3<f32>, has_derivatives: bool, dpdu: vec3<f32>, dpdv: vec3<f32>, mesh_tangent: vec4<f32>) -> vec3<f32> {
    if (!has_derivatives) {
        return n;
    }
    let tangent = tex_square_to(n, dpdu);
    let down = tex_square_to(n, dpdv);
    if (tangent.w == 0.0 || down.w == 0.0) {
        return n;
    }
    var t = tangent.xyz;
    var b = -down.xyz;
    if (mesh_tangent.w != 0.0) {
        let along = tex_square_to(n, mesh_tangent.xyz);
        if (along.w != 0.0) {
            t = along.xyz;
            b = tex_cross(n, t) * mesh_tangent.w;
        }
    }
    return tex_decode_normal(texel, n, t, b);
}

// srgb_to_acescg, left to right as Rust adds it.
fn tex_acescg(v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        cy_exact(cy_exact(0.6130974 * v.x) + cy_exact(0.3395234 * v.y)) + cy_exact(0.0473792 * v.z),
        cy_exact(cy_exact(0.0701936 * v.x) + cy_exact(0.9163535 * v.y)) + cy_exact(0.0134529 * v.z),
        cy_exact(cy_exact(0.0206158 * v.x) + cy_exact(0.1095696 * v.y)) + cy_exact(0.8698146 * v.z),
    );
}

// One OP_TEXTURE instruction at `pt` (shade.rs): `place` the graph's
// coordinates (with TEX_AT), then the mapping's constants (program.rs's
// mapping_constants). A colour (Sample: the lookup through the mapping
// after sample_pyramid's skip, weigh and renormalize, or sample_at's at the
// graph's place; colour_of), a number (SampleAlpha: the alpha's levels,
// clamped), the coordinates (Project: the first contribution's u, v and
// width), or a normal map laid down by the projection
// (Texture::sample_normal_with: each contribution decoded in the frame the
// projection's coordinates run in, read off the projection a step along
// each axis; the normal in the part's space and 1, or nought where none
// lands).
fn tex_instruction(settings: u32, place: vec4<f32>, scale_offset: vec4<f32>, turn: vec4<f32>, extra: vec4<f32>, pt: TexPoint) -> vec4<f32> {
    let mode = settings & 3u;
    var coords: array<vec4<f32>, 3>;
    var count = 1u;
    var wrap_u = (settings >> 7u) & 3u;
    var wrap_v = (settings >> 9u) & 3u;
    // A normal map's: the projection a step along X, Y and Z.
    var moved: array<array<vec4<f32>, 3>, 3>;
    var step = 1.0;
    if ((settings & TEX_AT) != 0u) {
        // sample_at: x across, y up from the bottom left, repeating, at the
        // sharpest level.
        coords[0] = vec4<f32>(place.x, 1.0 - place.y, 0.0, 1.0);
        wrap_u = TEX_WRAP_REPEAT;
        wrap_v = TEX_WRAP_REPEAT;
    } else {
        let passes = select(1u, 4u, mode == TEX_MODE_NORMAL);
        let p = pt.position;
        let reach = (cy_sqrt(tex_dot(p, p)) + 1.0) * 1e-4;
        step = select(reach, pt.footprint, pt.footprint > reach);
        for (var k = 0u; k < passes; k++) {
            var q = pt;
            if (k > 0u) {
                q.position = p + select(vec3<f32>(0.0), vec3<f32>(step), vec3<u32>(0u, 1u, 2u) == vec3<u32>(k - 1u));
            }
            let projected = tex_project(settings, scale_offset, turn, extra.x, q);
            if (k == 0u) {
                coords = projected.c;
                count = projected.count;
            } else {
                moved[k - 1u] = projected.c;
            }
        }
    }
    if (mode == TEX_MODE_PROJECT) {
        return vec4<f32>(coords[0].xyz, 0.0);
    }
    let pyramid = settings >> 18u;
    let n = pt.normal;
    var total = vec3<f32>(0.0);
    var weight = 0.0;
    var any = false;
    for (var k = 0u; k < count; k++) {
        let c = coords[k];
        if (!tex_inside(wrap_u, c.x) || !tex_inside(wrap_v, c.y)) {
            continue;
        }
        var tangent = vec4<f32>(0.0);
        var down = vec4<f32>(0.0);
        if (mode == TEX_MODE_NORMAL) {
            let s0 = moved[0][k];
            let s1 = moved[1][k];
            let s2 = moved[2][k];
            let across = vec3<f32>(tex_div(s0.x - c.x, step), tex_div(s1.x - c.x, step), tex_div(s2.x - c.x, step));
            let up = vec3<f32>(tex_div(s0.y - c.y, step), tex_div(s1.y - c.y, step), tex_div(s2.y - c.y, step));
            tangent = tex_square_to(n, across);
            down = tex_square_to(n, up);
            if (tangent.w == 0.0 || down.w == 0.0) {
                continue;
            }
        }
        let value = tex_lookup(pyramid, c.x, c.y, c.z, wrap_u, wrap_v);
        if (mode == TEX_MODE_NORMAL) {
            // Green is up the image, and v runs down it.
            total = total + cy_exact3(tex_decode_normal(value, n, tangent.xyz, -down.xyz) * c.w);
            any = true;
        } else {
            total = total + cy_exact3(value * c.w);
            weight = weight + c.w;
        }
    }
    if (mode == TEX_MODE_NORMAL) {
        return select(vec4<f32>(0.0), tex_normalized(total), any);
    }
    var result = vec3<f32>(0.0);
    if (weight > 1e-6) {
        result = total * tex_div(1.0, weight);
    }
    if (mode == TEX_MODE_ALPHA) {
        let a = result.x;
        return vec4<f32>(select(select(a, 1.0, a > 1.0), 0.0, a < 0.0));
    }
    if ((settings & TEX_MANAGED) != 0u) {
        result = tex_acescg(result);
    }
    return vec4<f32>(result, 0.0);
}
