// Media that scatter (docs/07 R3 tier B): the arithmetic of integrator.rs's
// random walk that traces nothing - Henyey-Greenstein's phase function and a
// turn drawn by it, a flight drawn in one channel and weighed over all
// three (walk_flight), and the walk under a surface guided back towards
// where it came in (WalkGuide, GuidedTurn) - for transport.wgsl's walks
// through a volume that scatters and under a surface.
//
// Its exponential and logarithm are FreeBSD's expf and logf (musl's before
// its tables), each within an ulp of the true value as the processor's are,
// where the card's own are looser by a growing margin over the range a walk
// reaches; the products a sum reads are rounded as Rust rounds them
// (cy_exact), and the quotients and roots are correctly rounded (cy_div,
// cy_sqrt).
//#include nodes
//#include onb
//#include sampler

const MD_PI: f32 = 3.14159265358979323846;
const MD_TAU: f32 = 6.28318530717958647692;

// expf (FreeBSD's e_expf.c): x = k ln 2 + r, |r| at most half ln 2, and a
// rational fit of e^r.
fn md_exp(x_in: f32) -> f32 {
    var x = x_in;
    let bits = bitcast<u32>(x);
    let sign = bits >> 31u;
    let hx = bits & 0x7FFFFFFFu;
    if (hx > 0x7F800000u) {
        return x;
    }
    if (hx >= 0x42AEAC50u) {
        if (hx >= 0x42B17218u && sign == 0u) {
            return bitcast<f32>(0x7F800000u);
        }
        if (sign != 0u && hx >= 0x42CFF1B5u) {
            return 0.0;
        }
    }
    var k = 0;
    var hi = x;
    var lo = 0.0;
    if (hx > 0x3EB17218u) {
        if (hx > 0x3F851592u) {
            // invln2 x + half[sign], truncated towards nought (C's cast).
            let t = cy_exact(1.4426950216 * x) + select(0.5, -0.5, sign != 0u);
            k = i32(trunc(t));
        } else {
            k = 1 - i32(sign) - i32(sign);
        }
        let kf = f32(k);
        hi = x - cy_exact(kf * 6.9314575195e-1);
        lo = cy_exact(kf * 1.4286067653e-6);
        x = hi - lo;
    } else if (hx <= 0x39000000u) {
        return 1.0 + x;
    }
    let xx = cy_exact(x * x);
    let c = x - cy_exact(xx * (1.6666625440e-1 + cy_exact(xx * -2.7667332906e-3)));
    let y = 1.0 + ((cy_div(cy_exact(x * c), 2.0 - c) - lo) + hi);
    if (k == 0) {
        return y;
    }
    return md_scale(y, k);
}

// scalbnf: y times 2^k, through the subnormals as the processor's does.
fn md_scale(y: f32, k: i32) -> f32 {
    var v = y;
    var n = k;
    if (n > 127) {
        v = v * 1.7014118e38;
        n -= 127;
        if (n > 127) {
            v = v * 1.7014118e38;
            n -= 127;
            n = min(n, 127);
        }
    } else if (n < -126) {
        // 2^-126 times 2^24, so the last step rounds once into the
        // subnormals.
        v = v * (1.17549435e-38 * 16777216.0);
        n += 126 - 24;
        if (n < -126) {
            v = v * (1.17549435e-38 * 16777216.0);
            n += 126 - 24;
            n = max(n, -126);
        }
    }
    return v * bitcast<f32>(u32(0x7F + n) << 23u);
}

// logf (FreeBSD's e_logf.c): x = 2^k m with m in [sqrt(2)/2, sqrt(2)],
// ln m from s = (m - 1)/(m + 1) and a fit in s^2.
fn md_ln(x_in: f32) -> f32 {
    var x = x_in;
    var ix = bitcast<u32>(x);
    var k = 0;
    if (ix < 0x00800000u || (ix >> 31u) != 0u) {
        if ((ix << 1u) == 0u) {
            return bitcast<f32>(0xFF800000u);
        }
        if ((ix >> 31u) != 0u) {
            return bitcast<f32>(0x7FC00000u);
        }
        k -= 25;
        x = x * 33554432.0;
        ix = bitcast<u32>(x);
    } else if (ix >= 0x7F800000u) {
        return x;
    } else if (ix == 0x3F800000u) {
        return 0.0;
    }
    ix += 0x3F800000u - 0x3F3504F3u;
    k += i32(ix >> 23u) - 0x7F;
    ix = (ix & 0x007FFFFFu) + 0x3F3504F3u;
    x = bitcast<f32>(ix);
    let f = x - 1.0;
    let s = cy_div(f, 2.0 + f);
    let z = cy_exact(s * s);
    let w = cy_exact(z * z);
    let t1 = cy_exact(w * (0.40000972152 + cy_exact(w * 0.24279078841)));
    let t2 = cy_exact(z * (0.66666662693 + cy_exact(w * 0.28498786688)));
    let r = t2 + t1;
    let hfsq = cy_exact(cy_exact(0.5 * f) * f);
    let dk = f32(k);
    return (((cy_exact(s * (hfsq + r)) + cy_exact(dk * 9.0580006145e-6)) - hfsq) + f) + cy_exact(dk * 6.9313812256e-1);
}

// powf for x over nought, as the guide and the walk's remap raise their
// albedos: e^(y ln x); nought for x nought.
fn md_pow(x: f32, y: f32) -> f32 {
    if (x == 1.0 || y == 0.0) {
        return 1.0;
    }
    if (x == 0.0) {
        return select(bitcast<f32>(0x7F800000u), 0.0, y > 0.0);
    }
    return md_exp(cy_exact(y * md_ln(x)));
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
    return cy_div(1.0 - gg, cy_exact(cy_exact((4.0 * MD_PI) * d) * cy_sqrt(d)));
}

// sample_henyey_greenstein: a way on from `forward`, drawn by the phase
// function of anisotropy `g` from `u`.
fn md_sample_phase(forward: vec3<f32>, g: f32, u: vec2<f32>) -> vec3<f32> {
    var cos_theta: f32;
    if (abs(g) < 1e-3) {
        cos_theta = 1.0 - 2.0 * u.x;
    } else {
        let gg = cy_exact(g * g);
        let s = cy_div(1.0 - gg, (1.0 - g) + cy_exact(cy_exact(2.0 * g) * u.x));
        cos_theta = cy_div((1.0 + gg) - cy_exact(s * s), 2.0 * g);
    }
    cos_theta = clamp(cos_theta, -1.0, 1.0);
    return md_turned(forward, cos_theta, u.y);
}

// The way at `cos_theta` to `axis`, turned `turn` of a turn round it, in
// the axis's own frame (Onb::from_normal).
fn md_turned(axis: vec3<f32>, cos_theta: f32, turn: f32) -> vec3<f32> {
    let sin_theta = cy_sqrt(max(1.0 - cy_exact(cos_theta * cos_theta), 0.0));
    let sc = cy_sincos(MD_TAU * turn);
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
        chance = vec3<f32>(cy_div(weights.x, total), cy_div(weights.y, total), cy_div(weights.z, total));
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
    let depth = cy_div(-md_ln(max(1.0 - along, 1e-12)), max(cy_exact(s * stretch), 1e-9));
    return WalkFlight(chance, depth);
}

// What survives `t` of a medium of extinction `sigma`, channel by channel.
fn md_survive(sigma: vec3<f32>, t: f32) -> vec3<f32> {
    return vec3<f32>(md_exp(cy_exact(-sigma.x * t)), md_exp(cy_exact(-sigma.y * t)), md_exp(cy_exact(-sigma.z * t)));
}

// The density of a plain flight - the phase function's turn, the
// extinction as it is - over the channels' chances: of ending in a
// collision at the end of `kept` (`collided`), or of flying past it.
fn md_plain_density(chance: vec3<f32>, sigma: vec3<f32>, kept: vec3<f32>, collided: bool) -> f32 {
    if (collided) {
        return (cy_exact(cy_exact(chance.x * sigma.x) * kept.x) + cy_exact(cy_exact(chance.y * sigma.y) * kept.y)) + cy_exact(cy_exact(chance.z * sigma.z) * kept.z);
    }
    return (cy_exact(chance.x * kept.x) + cy_exact(chance.y * kept.y)) + cy_exact(chance.z * kept.z);
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
    let exponent = (2.44294 - cy_exact(0.0215813 * alpha)) + cy_div(0.578637, alpha);
    let diffusion = cy_div(1.0, cy_sqrt(1.0 - md_pow(alpha, exponent)));
    let phase_log = md_ln(cy_div(diffusion + 1.0, diffusion - 1.0));
    let share = 1.0 - max(md_pow(abs(g), 0.125), 0.5);
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
    return cy_div(10.0, max(min(min(sigma.x, sigma.y), sigma.z), 1e-9));
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
    return cy_div(cy_div(1.0, cy_exact((guide.diffusion - mu) * guide.phase_log)), MD_TAU);
}

// WalkGuide::turn at `at`, the walk heading `forward`: guided by Dwivedi's
// distribution about the entry's normal (mirrored across to the surface
// opposite), or by the phase function of anisotropy `g`, on the open
// stream; the way on, the turn kept for the flight after it.
fn md_guide_turn(guide: ptr<function, WalkGuide>, at: vec3<f32>, forward: vec3<f32>, g: f32, s: ptr<function, Sampler>) -> vec3<f32> {
    (*guide).flown = true;
    let guided = sampler_next_unstructured(s) < (*guide).share;
    var backward_fraction = 0.0;
    let opposite = (*guide).opposite;
    let has_opposite = opposite > 0.0;
    if (has_opposite) {
        let x = clamp(md_dot(at - (*guide).entry, -(*guide).normal), 0.0, opposite);
        backward_fraction = cy_div(1.0, 1.0 + md_exp(cy_div(opposite - 2.0 * x, (*guide).diffusion)));
    }
    var backward = false;
    if (has_opposite) {
        backward = sampler_next_unstructured(s) < backward_fraction;
    }
    let u0 = sampler_next_unstructured(s);
    let u1 = sampler_next_unstructured(s);
    let v = (*guide).diffusion;
    var next: vec3<f32>;
    var cos_theta: f32;
    if (guided) {
        cos_theta = clamp(v - cy_exact((v + 1.0) * md_exp(cy_exact(-u0 * (*guide).phase_log))), -1.0, 1.0);
        if (backward) {
            cos_theta = -cos_theta;
        }
        next = md_turned((*guide).normal, cos_theta, u1);
    } else {
        next = md_sample_phase(forward, g, vec2<f32>(u0, u1));
        cos_theta = md_dot(next, (*guide).normal);
    }
    let phase = max(md_phase(md_dot(forward, next), g), 1e-12);
    (*guide).turned = true;
    (*guide).guided = guided;
    (*guide).backward = backward;
    (*guide).backward_fraction = backward_fraction;
    (*guide).forward_factor = cy_div(md_dwivedi(*guide, cos_theta), phase);
    (*guide).backward_factor = cy_div(md_dwivedi(*guide, -cos_theta), phase);
    (*guide).forward_stretch = 1.0 - cy_div(cos_theta, v);
    (*guide).backward_stretch = 1.0 + cy_div(cos_theta, v);
    return next;
}

// A flight's own density at `t` in one channel of extinction `s`: of a
// collision there, or of flying past it.
fn md_flight(s: f32, t: f32, collided: bool) -> f32 {
    let past = md_exp(cy_exact(-s * t));
    return select(past, cy_exact(s * past), collided);
}

// GuidedTurn::density: the density with which the flight after the turn
// ends at `t`, by every way it could have been drawn - the phase function
// and the extinction as it is, or guided out or across with the extinction
// stretched - in each channel, over the channels' chances.
fn md_guided_density(guide: WalkGuide, chance: vec3<f32>, sigma: vec3<f32>, t: f32, collided: bool) -> f32 {
    var total = 0.0;
    for (var c = 0u; c < 3u; c++) {
        let s = select(select(sigma.z, sigma.y, c == 1u), sigma.x, c == 0u);
        let p = select(select(chance.z, chance.y, c == 1u), chance.x, c == 0u);
        var guided = cy_exact(guide.forward_factor * md_flight(cy_exact(s * guide.forward_stretch), t, collided));
        if (guide.backward_fraction > 0.0) {
            guided = cy_exact(guided * (1.0 - guide.backward_fraction))
                + cy_exact(cy_exact(guide.backward_fraction * guide.backward_factor) * md_flight(cy_exact(s * guide.backward_stretch), t, collided));
        }
        total += cy_exact(p * (cy_exact((1.0 - guide.share) * md_flight(s, t, collided)) + cy_exact(guide.share * guided)));
    }
    return total;
}
