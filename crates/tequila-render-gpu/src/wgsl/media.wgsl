// Media that scatter (docs/07 R3 tier B): the arithmetic of integrator.rs's
// random walk that traces nothing - Henyey-Greenstein's phase function and a
// turn drawn by it, a flight drawn in one channel and weighed over all
// three (walk_flight), and the walk under a surface guided back towards
// where it came in (WalkGuide, GuidedTurn) - for transport.wgsl's walks
// through a volume that scatters and under a surface.
//
// Its exponential, logarithm, power and arctangent are correctly rounded,
// as the processor's expf, logf, powf and atanf are all but a few times in
// ten thousand: each carried in two floats on the pictures' tables
// (textures.wgsl's tex_log2_parts, tex_powf, tex_atan2), where the card's
// own, and FreeBSD's ports, are an ulp off one time in ten - which a
// guided walk's turn, v - (v + 1) e^x for a diffusion length v in the
// hundreds, multiplies a hundredfold. The products a sum reads are rounded
// as Rust rounds them (cy_exact), and the quotients and roots correctly
// rounded (tex_div, cy_sqrt).
//#include textures
//#include onb
//#include sampler

const MD_PI: f32 = 3.14159265358979323846;
const MD_TAU: f32 = 6.28318530717958647692;

// 2^(yh + yl) for |yl| far under an ulp of yh (tex_powf's last steps):
// 2^(j/16) from the table times e^g for |g| under ln 2 / 32, the rest of a
// series in one float.
fn md_exp2_parts(yh: f32, yl: f32) -> f32 {
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

// expf, correctly rounded: 2^(x log2 e), the product in two floats.
fn md_exp(x: f32) -> f32 {
    if (!(x == x)) {
        return x;
    }
    if (x > 88.72284) {
        return bitcast<f32>(0x7F800000u);
    }
    if (x < -103.97209) {
        return 0.0;
    }
    let yh = cy_exact(x * 1.4426950216293335);
    let yl = fma(x, 1.4426950216293335, -yh) + x * 1.925963033500011e-8;
    return md_exp2_parts(yh, yl);
}

// logf, correctly rounded: log2 in two floats (tex_log2_parts) times ln 2
// in two; near 1, where that is good only to an absolute 2^-40, as
// 2 atanh(f / (2 + f)) for f = x - 1 (exact), its leading term in two
// floats.
fn md_ln(x_in: f32) -> f32 {
    var x = x_in;
    if (x == 1.0) {
        return 0.0;
    }
    if (x > 0.9375 && x < 1.0625) {
        let f = x - 1.0;
        let d = cy_exact(2.0 + f);
        let d_low = f - cy_exact(d - 2.0);
        let s = tex_div(f, d);
        var r = fma(-s, d, f);
        r = fma(-s, d_low, r);
        let s_low = tex_div(r, d);
        let s2 = cy_exact(s * s);
        let corr = cy_exact(s * s2) * (0.6666667 + s2 * (0.4 + s2 * 0.2857143));
        return 2.0 * s + (2.0 * s_low + corr);
    }
    if (!(x > 0.0)) {
        return select(bitcast<f32>(0x7FC00000u), bitcast<f32>(0xFF800000u), x == 0.0);
    }
    if (x > 3.4e38) {
        return x;
    }
    // A subnormal, scaled up by 2^25, the 25 taken off the logarithm.
    var shift = 0.0;
    if (x < 1.17549435e-38) {
        x = x * 33554432.0;
        shift = 25.0;
    }
    var l = tex_log2_parts(x);
    if (shift != 0.0) {
        let t = tex_two_sum(l.x, -shift);
        l = vec2<f32>(t.x, t.y + l.y);
    }
    let ln2h = bitcast<f32>(texture_header[TEX_LN2]);
    let ln2l = bitcast<f32>(texture_header[TEX_LN2 + 1u]);
    let p = cy_exact(l.x * ln2h);
    let p_low = fma(l.x, ln2h, -p) + (l.x * ln2l + l.y * ln2h);
    return p + p_low;
}

// powf for x at or over nought, as the guide and the walk's remap raise
// their albedos and arctangents: tex_powf's, correctly rounded; nought for
// x nought, one for y nought.
fn md_pow(x: f32, y: f32) -> f32 {
    if (x == 1.0 || y == 0.0) {
        return 1.0;
    }
    if (x == 0.0) {
        return select(bitcast<f32>(0x7F800000u), 0.0, y > 0.0);
    }
    return tex_powf(x, y);
}

// atanf, correctly rounded (tex_atan2 of y over 1).
fn md_atan(y: f32) -> f32 {
    return tex_atan2(y, 1.0);
}

// sinf and cosf of t (a few turns either way at most), correctly rounded
// all but now and then: t less a whole number of quarter turns, in two
// floats (pi / 2 in three parts), then each series with its leading terms
// in two floats - sin r = r - r^3/6 + r^5 T(r^2), cos r = 1 - r^2/2 +
// r^4 C(r^2) - against Cephes' (cy_sincos), an ulp or two off one time in
// ten. (sin, cos).
fn md_sincos(t: f32) -> vec2<f32> {
    let k = round(t * 0.63661975);
    let a = t - k * 1.5703125;
    let p2 = cy_exact(k * 0.0004838267923332751);
    let p2e = fma(k, 0.0004838267923332751, -p2);
    let s1 = tex_two_sum(a, -p2);
    let low = (s1.y - p2e) - k * 2.5633440682570896e-12;
    let rh = cy_exact(s1.x + low);
    let rl = low - cy_exact(rh - s1.x);
    let r2h = cy_exact(rh * rh);
    let r2l = fma(rh, rh, -r2h) + 2.0 * rh * rl;
    // r^3 / 6 in two floats.
    let p = cy_exact(rh * r2h);
    let pe = fma(rh, r2h, -p) + (rh * r2l + rl * r2h);
    let q = tex_div(p, 6.0);
    let qe = (fma(-q, 6.0, p) + pe) / 6.0;
    let t5 = p * r2h * (0.008333334 + r2h * (-0.0001984127 + r2h * (2.7557319e-6 + r2h * (-2.5052108e-8 + r2h * 1.6059044e-10))));
    let sd = tex_two_sum(rh, -q);
    let sine = sd.x + (((sd.y + rl) - qe) + t5);
    // 1 - r^2/2, its rounding kept, and the rest.
    let h = 0.5 * r2h;
    let c0 = cy_exact(1.0 - h);
    let e0 = cy_exact(1.0 - c0) - h;
    let r4 = r2h * r2h;
    let rest = r4 * (0.041666668 + r2h * (-0.0013888889 + r2h * (2.4801588e-5 + r2h * (-2.7557319e-7 + r2h * 2.0876757e-9))));
    let cosine = c0 + ((e0 - 0.5 * r2l) + rest);
    switch (i32(k) & 3) {
        case 0: { return vec2<f32>(sine, cosine); }
        case 1: { return vec2<f32>(cosine, -sine); }
        case 2: { return vec2<f32>(-sine, -cosine); }
        default: { return vec2<f32>(-cosine, sine); }
    }
}

// sample_cosine_hemisphere: the concentric disc (sample_uniform_disk_
// concentric) and the hemisphere above it, its quotients, roots, sines and
// cosines correctly rounded and its products rounded as Rust rounds them -
// the walk's doorway's draw in and out (DiffuseTransmission), where the
// diffuse lobe's (lobe_cosine_hemisphere) is the card's own trigonometry.
fn md_cosine_hemisphere(u: vec2<f32>) -> vec3<f32> {
    let o = vec2<f32>(2.0 * u.x - 1.0, 2.0 * u.y - 1.0);
    var d = vec2<f32>(0.0);
    if (!(o.x == 0.0 && o.y == 0.0)) {
        var r: f32;
        var theta: f32;
        if (abs(o.x) > abs(o.y)) {
            r = o.x;
            theta = cy_exact((MD_PI / 4.0) * tex_div(o.y, o.x));
        } else {
            r = o.y;
            theta = (MD_PI / 2.0) - cy_exact((MD_PI / 4.0) * tex_div(o.x, o.y));
        }
        let sc = md_sincos(theta);
        d = vec2<f32>(cy_exact(r * sc.y), cy_exact(r * sc.x));
    }
    let z = cy_sqrt(max(cy_exact(1.0 - cy_exact(d.x * d.x)) - cy_exact(d.y * d.y), 0.0));
    return vec3<f32>(d.x, d.y, z);
}

// Vec3::dot, summed in order, each product rounded.
fn md_dot(a: vec3<f32>, b: vec3<f32>) -> f32 {
    return (cy_exact(a.x * b.x) + cy_exact(a.y * b.y)) + cy_exact(a.z * b.z);
}

// henyey_greenstein: the phase function per steradian, `cos_theta` between
// the way light was travelling and the way it leaves.
fn md_phase(cos_theta: f32, g_in: f32) -> f32 {
    let g = clamp(g_in, -0.99, 0.99);
    let gg = cy_exact(g * g);
    let d = max((1.0 + gg) - cy_exact(cy_exact(2.0 * g) * cos_theta), 1e-6);
    return tex_div(1.0 - gg, cy_exact(cy_exact((4.0 * MD_PI) * d) * cy_sqrt(d)));
}

// sample_henyey_greenstein's cosine to the way the light was going, drawn
// by the phase function of anisotropy `g` from `u0`; the way on is
// md_turned's about it, `u1` of a turn round.
fn md_phase_cosine(g: f32, u0: f32) -> f32 {
    var cos_theta: f32;
    if (abs(g) < 1e-3) {
        cos_theta = 1.0 - 2.0 * u0;
    } else {
        let gg = cy_exact(g * g);
        let s = tex_div(1.0 - gg, (1.0 - g) + cy_exact(cy_exact(2.0 * g) * u0));
        cos_theta = tex_div((1.0 + gg) - cy_exact(s * s), 2.0 * g);
    }
    return clamp(cos_theta, -1.0, 1.0);
}

// sample_henyey_greenstein: a way on from `forward`, drawn by the phase
// function of anisotropy `g` from `u` (for the tests; the kernel turns
// once, md_turned at its collision).
fn md_sample_phase(forward: vec3<f32>, g: f32, u: vec2<f32>) -> vec3<f32> {
    return md_turned(forward, md_phase_cosine(g, u.x), u.y);
}

// The way at `cos_theta` to `axis`, turned `turn` of a turn round it, in
// the axis's own frame (Onb::from_normal).
fn md_turned(axis: vec3<f32>, cos_theta: f32, turn: f32) -> vec3<f32> {
    let sin_theta = cy_sqrt(max(1.0 - cy_exact(cos_theta * cos_theta), 0.0));
    let sc = md_sincos(MD_TAU * turn);
    let local = vec3<f32>(cy_exact(sin_theta * sc.y), cy_exact(sin_theta * sc.x), cos_theta);
    let f = frame_of(axis);
    return (cy_exact3(f.t * local.x) + cy_exact3(f.b * local.y)) + cy_exact3(f.n * local.z);
}

// A walk's next flight (walk_flight): each channel's chance, in proportion
// to the path's throughput there times the medium's albedo (by throughput
// alone where that is nothing), a channel picked by `pick`, and the optical
// depth flown in it from `along`, its extinction stretched by `stretch`.
struct WalkFlight {
    chance: vec3<f32>,
    depth: f32,
};

fn md_walk_flight(throughput: vec3<f32>, albedo: vec3<f32>, sigma: vec3<f32>, stretch: f32, pick: f32, along: f32) -> WalkFlight {
    let scattered = max(throughput * albedo, vec3<f32>(0.0));
    let carried = max(throughput, vec3<f32>(0.0));
    var weights = scattered;
    var total = (scattered.x + scattered.y) + scattered.z;
    if (!(total > 0.0)) {
        weights = carried;
        total = (carried.x + carried.y) + carried.z;
    }
    var chance = vec3<f32>(1.0 / 3.0);
    if (total > 0.0) {
        chance = vec3<f32>(tex_div(weights.x, total), tex_div(weights.y, total), tex_div(weights.z, total));
    }
    var s = sigma.z;
    var below = chance.x;
    if (pick < below) {
        s = sigma.x;
    } else {
        below = below + chance.y;
        if (pick < below) {
            s = sigma.y;
        }
    }
    let depth = tex_div(-md_ln(max(1.0 - along, 1e-12)), max(cy_exact(s * stretch), 1e-9));
    return WalkFlight(chance, depth);
}

// What survives `t` of a medium of extinction `sigma`, channel by channel:
// the three by one call of the exponential, for the kernel's size (each
// lane chosen by a select: a vector's lane written by a run-time index has
// gone to the wrong lane on this driver).
fn md_survive(sigma: vec3<f32>, t: f32) -> vec3<f32> {
    var kept = vec3<f32>(0.0);
    for (var c = 0u; c < 3u; c++) {
        let s = select(select(sigma.z, sigma.y, c == 1u), sigma.x, c == 0u);
        let e = md_exp(cy_exact(-s * t));
        kept = select(kept, vec3<f32>(e), vec3<bool>(c == 0u, c == 1u, c == 2u));
    }
    return kept;
}

// walk_coefficients' albedo for one channel's colour, the walk's fit at
// its anisotropy worked out on the processor (GpuMaterial's walk_fit: a, b,
// c, d, and in walk_fit_ef e and f): the single-scattering albedo a walk
// needs to give `colour` back (Chiang, Kutz and Burley's fit, as Cycles'
// subsurface_random_walk_remap has it). The three channels, and each one's
// three powers, by one call of the power and one of the arctangent, for
// the kernel's size.
fn md_walk_albedo(colour: vec3<f32>, fit: vec4<f32>, e: f32, f: f32) -> vec3<f32> {
    var out = vec3<f32>(0.0);
    var blend = 0.0;
    var near = 0.0;
    for (var k = 0u; k < 9u; k++) {
        let c = k / 3u;
        let term = k - 3u * c;
        let albedo = clamp(select(select(colour.z, colour.y, c == 1u), colour.x, c == 0u), 0.0, 1.0);
        // blend = albedo^0.25; then atan(b albedo)^c and atan(e albedo)^f.
        let turned = md_atan(cy_exact(select(fit.y, e, term == 2u) * albedo));
        let raised = md_pow(select(turned, albedo, term == 0u), select(select(f, fit.z, term == 1u), 0.25, term == 0u));
        if (term == 0u) {
            blend = raised;
        } else if (term == 1u) {
            near = cy_exact(cy_exact((1.0 - blend) * fit.x) * raised);
        } else {
            let far = cy_exact(cy_exact(blend * fit.w) * raised);
            out = select(out, vec3<f32>(clamp(near + far, 0.0, 0.999999)), vec3<bool>(c == 0u, c == 1u, c == 2u));
        }
    }
    return out;
}

// ---- The guided walk (WalkGuide) -----------------------------------------

// A walk under a surface guided back towards where it came in: the solid
// it walks in (NO_WALK for no guide), the entry and the surface's normal
// there out of the part, Dwivedi's diffusion length and its logarithm, the
// share of turns guided, how far into the part the surface opposite lies
// (below nought for none seen), whether the walk has turned yet, and the
// turn that set the flight now flown (GuidedTurn; `turned` false before the
// first).
const NO_WALK: u32 = 0xFFFFFFFFu;

struct WalkGuide {
    solid: u32,
    entry: vec3<f32>,
    normal: vec3<f32>,
    diffusion: f32,
    phase_log: f32,
    share: f32,
    opposite: f32,
    flown: bool,
    turned: bool,
    guided: bool,
    backward: bool,
    backward_fraction: f32,
    forward_factor: f32,
    backward_factor: f32,
    forward_stretch: f32,
    backward_stretch: f32,
};

fn md_no_guide() -> WalkGuide {
    var g: WalkGuide;
    g.solid = NO_WALK;
    return g;
}

// WalkGuide::new for a medium of `albedo` and anisotropy `g`: none where a
// guide would not help or cannot be worked out.
fn md_guide_new(solid: u32, entry: vec3<f32>, normal: vec3<f32>, albedo: vec3<f32>, g: f32) -> WalkGuide {
    var guide = md_no_guide();
    let alpha = max(max(albedo.x, albedo.y), albedo.z);
    if (!(alpha > 0.0 && alpha < 1.0)) {
        return guide;
    }
    let exponent = (2.44294 - cy_exact(0.0215813 * alpha)) + tex_div(0.578637, alpha);
    // alpha^exponent and |g|^(1/8) by one call of the power.
    var raised = 0.0;
    var g_root = 0.0;
    for (var k = 0u; k < 2u; k++) {
        let p = md_pow(select(alpha, abs(g), k == 1u), select(exponent, 0.125, k == 1u));
        raised = select(raised, p, k == 0u);
        g_root = select(g_root, p, k == 1u);
    }
    let diffusion = tex_div(1.0, cy_sqrt(1.0 - raised));
    let phase_log = md_ln(tex_div(diffusion + 1.0, diffusion - 1.0));
    let share = 1.0 - max(g_root, 0.5);
    if (!(diffusion > 1.0 && diffusion <= 3.4e38 && phase_log > 0.0 && phase_log <= 3.4e38 && share > 0.0)) {
        return guide;
    }
    guide.solid = solid;
    guide.entry = entry;
    guide.normal = normal;
    guide.diffusion = diffusion;
    guide.phase_log = phase_log;
    guide.share = share;
    guide.opposite = -1.0;
    guide.flown = false;
    guide.turned = false;
    return guide;
}

// WalkGuide::look: how far the first flight looks for the surface opposite,
// ten mean free paths in the channel that travels furthest.
fn md_guide_look(sigma: vec3<f32>) -> f32 {
    return tex_div(10.0, max(min(min(sigma.x, sigma.y), sigma.z), 1e-9));
}

// WalkGuide::stretch: what the next flight's extinction is multiplied by.
fn md_guide_stretch(guide: WalkGuide) -> f32 {
    if (guide.turned && guide.guided) {
        return select(guide.forward_stretch, guide.backward_stretch, guide.backward);
    }
    return 1.0;
}

// Dwivedi's density for a direction at `mu` to the entry's normal.
fn md_dwivedi(guide: WalkGuide, mu: f32) -> f32 {
    return tex_div(tex_div(1.0, cy_exact((guide.diffusion - mu) * guide.phase_log)), MD_TAU);
}

// WalkGuide::turn at `at`, in two halves about the one turn a walk's
// collision makes (md_turned, called once, for the kernel's size). The
// first draws whether the turn is guided by Dwivedi's distribution about
// the entry's normal (mirrored across to the surface opposite) or by the
// phase function, on the open stream, and the two variates of the turn:
// (u0, u1) and, guided, the cosine to the normal; the guide keeps which.
// Its numbers drawn first, in the processor's order (the coin for a guided
// turn, the coin across where there is a surface opposite, the turn's
// two), then its two exponentials by one call, in a loop that draws none.
fn md_guide_pick(guide: ptr<function, WalkGuide>, at: vec3<f32>, s: ptr<function, Sampler>) -> vec3<f32> {
    (*guide).flown = true;
    let opposite = (*guide).opposite;
    let has_opposite = opposite > 0.0;
    let v = (*guide).diffusion;
    let guided_coin = sampler_next_unstructured(s);
    var across_coin = 0.0;
    if (has_opposite) {
        across_coin = sampler_next_unstructured(s);
    }
    let u0 = sampler_next_unstructured(s);
    let u1 = sampler_next_unstructured(s);
    let x = clamp(md_dot(at - (*guide).entry, -(*guide).normal), 0.0, opposite);
    // e^((d - 2x) / v) for the share across, e^(-u0 ln((v + 1) / (v - 1)))
    // for the guided cosine.
    var e = vec2<f32>(0.0);
    for (var k = 0u; k < 2u; k++) {
        let across = k == 0u;
        let r = md_exp(select(cy_exact(-u0 * (*guide).phase_log), tex_div(opposite - 2.0 * x, v), across));
        e = select(vec2<f32>(e.x, r), vec2<f32>(r, e.y), across);
    }
    var backward_fraction = 0.0;
    var backward = false;
    if (has_opposite) {
        backward_fraction = tex_div(1.0, 1.0 + e.x);
        backward = across_coin < backward_fraction;
    }
    let guided = guided_coin < (*guide).share;
    var cos_theta = 0.0;
    if (guided) {
        cos_theta = clamp(v - cy_exact((v + 1.0) * e.y), -1.0, 1.0);
        if (backward) {
            cos_theta = -cos_theta;
        }
    }
    (*guide).guided = guided;
    (*guide).backward = backward;
    (*guide).backward_fraction = backward_fraction;
    return vec3<f32>(u0, u1, cos_theta);
}

// The second half: the turn to `next` kept for the flight after it, by its
// cosine to the entry's normal and the phase function's density for the
// turn (`density`, henyey_greenstein of the way the path was going and
// `next`, which the walk keeps too). Out and across by one loop.
fn md_guide_keep(guide: ptr<function, WalkGuide>, next: vec3<f32>, density: f32, guided_cos: f32) {
    let v = (*guide).diffusion;
    let cos_theta = select(md_dot(next, (*guide).normal), guided_cos, (*guide).guided);
    let phase = max(density, 1e-12);
    (*guide).turned = true;
    for (var k = 0u; k < 2u; k++) {
        let across = k == 1u;
        // Dwivedi's density at the cosine, out or mirrored across, against
        // the phase function's; the flight's extinction stretched by it
        // (-x / v is x / v negated exactly).
        let mu = select(cos_theta, -cos_theta, across);
        let factor = tex_div(md_dwivedi(*guide, mu), phase);
        let stretch = 1.0 - tex_div(mu, v);
        if (across) {
            (*guide).backward_factor = factor;
            (*guide).backward_stretch = stretch;
        } else {
            (*guide).forward_factor = factor;
            (*guide).forward_stretch = stretch;
        }
    }
}

// WalkGuide::turn whole (for the tests): the way on, the turn kept.
fn md_guide_turn(guide: ptr<function, WalkGuide>, at: vec3<f32>, forward: vec3<f32>, g: f32, s: ptr<function, Sampler>) -> vec3<f32> {
    let pick = md_guide_pick(guide, at, s);
    var next: vec3<f32>;
    if ((*guide).guided) {
        next = md_turned((*guide).normal, pick.z, pick.y);
    } else {
        next = md_turned(forward, md_phase_cosine(g, pick.x), pick.y);
    }
    md_guide_keep(guide, next, md_phase(md_dot(forward, next), g), pick.z);
    return next;
}

// A flight's own density in one channel of extinction `s`, from what
// survives it (`past`): of a collision at its end, or of flying past it.
fn md_flight(s: f32, past: f32, collided: bool) -> f32 {
    return select(past, cy_exact(s * past), collided);
}

// GuidedTurn::density: the density with which the flight after the turn
// ends at `t`, by every way it could have been drawn - the phase function
// and the extinction as it is (`kept`, what survives `t` of it), or guided
// out or across with the extinction stretched - in each channel, over the
// channels' chances (md_walk_end has the kernel's).
fn md_guided_density(guide: WalkGuide, chance: vec3<f32>, sigma: vec3<f32>, kept: vec3<f32>, t: f32, collided: bool) -> f32 {
    return md_walk_end(guide, true, chance, sigma, t, collided).w;
}

// Where a flight ends, at `t`: what survives it in each channel (xyz), and
// the density of ending there (w) - a collision (`collided`) or the surface
// reached - after a plain turn (the phase function's, the extinction as it
// is) or a guided one (`turned`, GuidedTurn::density). Every exponential
// by one call, in one loop: each channel's flight as it is, then
// stretched out and across.
fn md_walk_end(guide: WalkGuide, turned: bool, chance: vec3<f32>, sigma: vec3<f32>, t: f32, collided: bool) -> vec4<f32> {
    let across_too = turned && guide.backward_fraction > 0.0;
    var kept = vec3<f32>(0.0);
    var total = 0.0;
    var out_flight = 0.0;
    for (var k = 0u; k < 9u; k++) {
        let c = k / 3u;
        let kind = k - 3u * c;
        if ((kind == 1u && !turned) || (kind == 2u && !across_too)) {
            continue;
        }
        let s = select(select(sigma.z, sigma.y, c == 1u), sigma.x, c == 0u);
        let stretched = select(cy_exact(s * select(guide.forward_stretch, guide.backward_stretch, kind == 2u)), s, kind == 0u);
        let past = md_exp(cy_exact(-stretched * t));
        if (kind == 0u) {
            kept = select(kept, vec3<f32>(past), vec3<bool>(c == 0u, c == 1u, c == 2u));
            if (turned) {
                continue;
            }
        } else if (kind == 1u) {
            out_flight = md_flight(stretched, past, collided);
            if (across_too) {
                continue;
            }
        }
        let p = select(select(chance.z, chance.y, c == 1u), chance.x, c == 0u);
        let plain = select(select(kept.z, kept.y, c == 1u), kept.x, c == 0u);
        if (!turned) {
            // The plain density, a channel at a time.
            total += select(cy_exact(p * plain), cy_exact(cy_exact(p * s) * plain), collided);
            continue;
        }
        var guided = cy_exact(guide.forward_factor * out_flight);
        if (across_too) {
            guided = cy_exact(guided * (1.0 - guide.backward_fraction)) + cy_exact(cy_exact(guide.backward_fraction * guide.backward_factor) * md_flight(stretched, past, collided));
        }
        total += cy_exact(p * (cy_exact((1.0 - guide.share) * md_flight(s, plain, collided)) + cy_exact(guide.share * guided)));
    }
    return vec4<f32>(kept, total);
}
