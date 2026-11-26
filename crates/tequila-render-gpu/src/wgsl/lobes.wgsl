// The lobes: tequila_materials::lobes' OrenNayarDiffuse (EON),
// DielectricSpecular and Conductor, each with the thin film over it its
// `film` holds (film.wgsl; none on a coat or a flake), and
// DielectricTransmission, glass. Shading frame, z along the normal.
//#include energy
//#include film

const L_FRAC_1_PI: f32 = 0.318309886183790671538;
const FUJII_A: f32 = 0.5 - 2.0 / (3.0 * 3.14159265358979323846);
const FUJII_B: f32 = 2.0 / 3.0 - 28.0 / (15.0 * 3.14159265358979323846);
// The share of a rough lobe's samples drawn from its compensation lobe,
// of what its single scattering misses (dielectric.rs).
const COMPENSATION_SHARE: f32 = 0.25;

struct LobeSample {
    ok: bool,
    wi: vec3<f32>,
    f: vec3<f32>,
    pdf: f32,
    is_delta: bool,
    // Through the surface rather than off it (BsdfSample::is_transmission),
    // and the index ahead over the index behind for the crossing (its eta,
    // 1 off the surface).
    is_transmission: bool,
    eta: f32,
};

fn no_sample() -> LobeSample {
    var s: LobeSample;
    s.ok = false;
    return s;
}

// sample_cosine_hemisphere, concentric.
fn lobe_cosine_hemisphere(u: vec2<f32>) -> vec3<f32> {
    let o = vec2<f32>(2.0 * u.x - 1.0, 2.0 * u.y - 1.0);
    var d = vec2<f32>(0.0, 0.0);
    if (!(o.x == 0.0 && o.y == 0.0)) {
        var r: f32;
        var theta: f32;
        if (abs(o.x) > abs(o.y)) {
            r = o.x;
            theta = (MF_PI / 4.0) * (o.y / o.x);
        } else {
            r = o.y;
            theta = (MF_PI / 2.0) - (MF_PI / 4.0) * (o.x / o.y);
        }
        d = vec2<f32>(r * cos(theta), r * sin(theta));
    }
    return vec3<f32>(d.x, d.y, sqrt(max(1.0 - d.x * d.x - d.y * d.y, 0.0)));
}

fn cosine_pdf(cos_theta: f32) -> f32 {
    return max(cos_theta, 0.0) * L_FRAC_1_PI;
}

// --- OrenNayarDiffuse --------------------------------------------------

struct Diffuse {
    albedo: vec3<f32>,
    roughness: f32,
};

fn diffuse_new(albedo: vec3<f32>, roughness: f32) -> Diffuse {
    return Diffuse(clamp(albedo, vec3<f32>(0.0), vec3<f32>(1.0)), clamp(roughness, 0.0, 1.0));
}

fn diffuse_single_scatter_albedo(l: Diffuse, mu_in: f32) -> f32 {
    let r = l.roughness;
    let a = 1.0 / (1.0 + FUJII_A * r);
    if (r <= 0.0) {
        return a;
    }
    let b = r * a;
    let mu = clamp(mu_in, 1e-3, 1.0);
    let si = sqrt(max(1.0 - mu * mu, 0.0));
    let g = si * (acos(clamp(mu, -1.0, 1.0)) - si * mu) + (2.0 / 3.0) * ((si / mu) * (1.0 - si * si * si) - si);
    return clamp(a + (b * L_FRAC_1_PI) * g, 0.0, 1.0);
}

fn diffuse_eval(l: Diffuse, wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    let mu_o = wo.z;
    let mu_i = wi.z;
    if (mu_o <= 0.0 || mu_i <= 0.0) {
        return vec3<f32>(0.0);
    }
    let r = l.roughness;
    let a = 1.0 / (1.0 + FUJII_A * r);
    let s = dot(wi, wo) - mu_i * mu_o;
    let s_over_t = select(s, s / max(mu_i, mu_o), s > 0.0);
    let f_ss = l.albedo * (L_FRAC_1_PI * a * (1.0 + r * s_over_t));
    if (r <= 0.0) {
        return f_ss;
    }
    let e_o = diffuse_single_scatter_albedo(l, mu_o);
    let e_i = diffuse_single_scatter_albedo(l, mu_i);
    let e_avg = (1.0 / (1.0 + FUJII_A * r)) * (1.0 + FUJII_B * r);
    let denom = vec3<f32>(1.0) - l.albedo * (1.0 - e_avg);
    let rho_ms = (l.albedo * l.albedo) * e_avg / denom;
    let eps = 1e-7;
    let f_ms = rho_ms * (L_FRAC_1_PI * max(1.0 - e_o, eps) * max(1.0 - e_i, eps) / max(1.0 - e_avg, eps));
    return f_ss + f_ms;
}

fn diffuse_pdf(l: Diffuse, wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    if (wo.z <= 0.0 || wi.z <= 0.0) {
        return 0.0;
    }
    return cosine_pdf(wi.z);
}

// A lobe's draw: its sample's direction, and for a mirror-smooth one its
// whole sample; a direction its own sample would keep only where its own
// density there is above nought (and, for the transmission, its value not
// black), which the stack tells from the evaluation it makes of every lobe
// in that direction anyway (openpbr_sample_seen), so that the kernel holds
// each lobe's evaluation fewer times. The lobes' samples are their draws
// with those checks.
fn diffuse_draw(l: Diffuse, wo: vec3<f32>, u: vec2<f32>) -> LobeSample {
    if (wo.z <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, lobe_cosine_hemisphere(u), vec3<f32>(0.0), 0.0, false, false, 1.0);
}

fn diffuse_sample(l: Diffuse, wo: vec3<f32>, u: vec2<f32>) -> LobeSample {
    let s = diffuse_draw(l, wo, u);
    let pdf = diffuse_pdf(l, wo, s.wi);
    if (!s.ok || pdf <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, s.wi, diffuse_eval(l, wo, s.wi), pdf, false, false, 1.0);
}

// --- DielectricSpecular --------------------------------------------------

struct Specular {
    distribution: Ggx,
    ior: f32,
    tint: vec3<f32>,
    weight: f32,
    // The thin film over the interface (film.wgsl): none where its weight
    // is nought, as specular_new makes it.
    film: vec4<f32>,
};

fn specular_new(roughness: f32, ior: f32) -> Specular {
    return Specular(ggx_from_roughness(roughness, 0.0), max(ior, 1.0), vec3<f32>(1.0), 1.0, vec4<f32>(0.0));
}

// The microfacet a pair of directions reflects off, as the reflections'
// evaluations find it: |wo . wm|, or -1 where there is none.
fn lobe_half_cos(wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    let h = wo + wi;
    let len2 = dot(h, h);
    if (!(len2 > 0.0)) {
        return -1.0;
    }
    var wm = h * (1.0 / sqrt(len2));
    if (wm.z < 0.0) {
        wm = -wm;
    }
    return abs(dot(wo, wm));
}

// The film's reflectance at a microfacet seen at `cos_theta`, and in w 1;
// nought, w 0, where the lobe wears none. The stack works it out once in
// each of its lobe functions, for whichever lobe it reads (openpbr_film),
// and hands it to the lobe's *_given functions, so the film's arithmetic is
// built once a place rather than once a lobe.
fn specular_film(l: Specular, cos_theta: f32) -> vec4<f32> {
    if (FEATURE_FILM && film_on(l.film)) {
        return vec4<f32>(film_over(vec3<f32>(fresnel_dielectric(cos_theta, l.ior)), cos_theta, l.film, vec3<f32>(max(l.ior, 1.0)), vec3<f32>(0.0)), 1.0);
    }
    return vec4<f32>(0.0);
}

// The reflectance at a microfacet: the film's, where `filmed` holds it,
// else the bare interface's.
fn specular_reflectance_given(l: Specular, cos_theta: f32, filmed: vec4<f32>) -> vec3<f32> {
    if (filmed.w != 0.0) {
        return filmed.xyz;
    }
    return vec3<f32>(fresnel_dielectric(cos_theta, l.ior));
}

// The reflectance at a microfacet, bare or under the film.
fn specular_reflectance(l: Specular, cos_theta: f32) -> vec3<f32> {
    return specular_reflectance_given(l, cos_theta, specular_film(l, cos_theta));
}

fn specular_compensation_probability(l: Specular, wo: vec3<f32>) -> f32 {
    let e_o = energy_white_directional(abs(wo.z), ggx_average_alpha(l.distribution));
    return clamp((1.0 - e_o) * COMPENSATION_SHARE, 0.0, 0.5);
}

fn specular_compensation_scale(l: Specular, e_avg: f32) -> f32 {
    let f_avg = fresnel_dielectric_average(l.ior);
    return multiple_scattering_scale(vec3<f32>(f_avg), e_avg).y;
}

fn specular_directional_albedo(l: Specular, wo: vec3<f32>) -> f32 {
    let mu = abs(wo.z);
    let alpha = ggx_average_alpha(l.distribution);
    let single = energy_dielectric_directional(mu, alpha, l.ior);
    let e_o = energy_white_directional(mu, alpha);
    let e_avg = energy_white_average(alpha);
    return clamp(l.weight * (single + specular_compensation_scale(l, e_avg) * (1.0 - e_o)), 0.0, 1.0);
}

fn specular_hemispherical_albedo(l: Specular) -> f32 {
    let alpha = ggx_average_alpha(l.distribution);
    let e_avg = energy_white_average(alpha);
    return clamp(l.weight * (energy_dielectric_hemispherical(alpha, l.ior) + specular_compensation_scale(l, e_avg) * (1.0 - e_avg)), 0.0, 1.0);
}

fn specular_eval(l: Specular, wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    return specular_eval_given(l, wo, wi, specular_film(l, lobe_half_cos(wo, wi)));
}

// specular_eval with the film's reflectance at the microfacet given
// (specular_film).
fn specular_eval_given(l: Specular, wo: vec3<f32>, wi: vec3<f32>, filmed: vec4<f32>) -> vec3<f32> {
    if (wo.z <= 0.0 || wi.z <= 0.0 || ggx_is_smooth(l.distribution) || l.weight <= 0.0) {
        return vec3<f32>(0.0);
    }
    let h = wo + wi;
    let len2 = dot(h, h);
    if (!(len2 > 0.0)) {
        return vec3<f32>(0.0);
    }
    var wm = h * (1.0 / sqrt(len2));
    if (wm.z < 0.0) {
        wm = -wm;
    }
    let d = ggx_d(l.distribution, wm);
    let g2 = ggx_g2(l.distribution, wo, wi);
    let single = specular_reflectance_given(l, abs(dot(wo, wm)), filmed) * (d * g2 / (4.0 * wo.z * wi.z));
    let alpha = ggx_average_alpha(l.distribution);
    let e_o = energy_white_directional(wo.z, alpha);
    let e_i = energy_white_directional(wi.z, alpha);
    let e_avg = energy_white_average(alpha);
    let multi = specular_compensation_scale(l, e_avg) * multiple_scattering_value(e_o, e_i, e_avg);
    return l.tint * ((single + vec3<f32>(multi)) * l.weight);
}

fn specular_pdf(l: Specular, wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    if (wo.z <= 0.0 || wi.z <= 0.0 || ggx_is_smooth(l.distribution) || l.weight <= 0.0) {
        return 0.0;
    }
    let p_comp = specular_compensation_probability(l, wo);
    return (1.0 - p_comp) * ggx_reflection_pdf(l.distribution, wo, wi) + p_comp * cosine_pdf(wi.z);
}

fn specular_draw(l: Specular, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    return specular_draw_given(l, wo, u, uc, specular_film(l, wo.z));
}

// specular_draw with the film's reflectance seen at wo.z given.
fn specular_draw_given(l: Specular, wo: vec3<f32>, u: vec2<f32>, uc: f32, filmed: vec4<f32>) -> LobeSample {
    if (wo.z <= 0.0 || l.weight <= 0.0) {
        return no_sample();
    }
    if (ggx_is_smooth(l.distribution)) {
        let wi = vec3<f32>(-wo.x, -wo.y, wo.z);
        let f = specular_reflectance_given(l, wo.z, filmed) * l.weight;
        return LobeSample(true, wi, l.tint * f * (1.0 / abs(wi.z)), 1.0, true, false, 1.0);
    }
    let p_comp = specular_compensation_probability(l, wo);
    var wi: vec3<f32>;
    if (uc < p_comp) {
        wi = lobe_cosine_hemisphere(u);
    } else {
        let wm = ggx_sample_visible_normal(l.distribution, wo, u);
        wi = -wo + wm * (2.0 * dot(wo, wm));
    }
    if (wi.z <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, wi, vec3<f32>(0.0), 0.0, false, false, 1.0);
}

fn specular_sample(l: Specular, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    let s = specular_draw(l, wo, u, uc);
    if (!s.ok || s.is_delta) {
        return s;
    }
    let pdf = specular_pdf(l, wo, s.wi);
    if (pdf <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, s.wi, specular_eval(l, wo, s.wi), pdf, false, false, 1.0);
}

// --- Conductor -----------------------------------------------------------

struct Conductor {
    distribution: Ggx,
    n: vec3<f32>,
    k: vec3<f32>,
    tint: vec3<f32>,
    // The thin film over the metal (film.wgsl), none where its weight is
    // nought.
    film: vec4<f32>,
};

// The film's reflectance over the metal's complex index n - ik at a
// microfacet seen at `cos_theta`, and in w 1; nought, w 0, where it wears
// none (see specular_film).
fn conductor_film(l: Conductor, cos_theta: f32) -> vec4<f32> {
    if (FEATURE_FILM && film_on(l.film)) {
        return vec4<f32>(film_over(fresnel_conductor(cos_theta, l.n, l.k), cos_theta, l.film, l.n, -l.k), 1.0);
    }
    return vec4<f32>(0.0);
}

// The reflectance at a microfacet: the film's, where `filmed` holds it,
// else the bare metal's.
fn conductor_reflectance_given(l: Conductor, cos_theta: f32, filmed: vec4<f32>) -> vec3<f32> {
    if (filmed.w != 0.0) {
        return filmed.xyz;
    }
    return fresnel_conductor(cos_theta, l.n, l.k);
}

// The reflectance at a microfacet, bare or under the film, over the metal's
// complex index n - ik.
fn conductor_reflectance(l: Conductor, cos_theta: f32) -> vec3<f32> {
    return conductor_reflectance_given(l, cos_theta, conductor_film(l, cos_theta));
}

fn conductor_compensation_probability(l: Conductor, wo: vec3<f32>) -> f32 {
    let e_o = energy_white_directional(abs(wo.z), ggx_average_alpha(l.distribution));
    return clamp(1.0 - e_o, 0.0, 0.5);
}

fn conductor_compensation_scale(l: Conductor, e_avg: f32) -> vec3<f32> {
    let f_avg = fresnel_conductor_average(fresnel_conductor(1.0, l.n, l.k));
    return multiple_scattering_scale(f_avg, e_avg);
}

fn conductor_eval(l: Conductor, wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    return conductor_eval_given(l, wo, wi, conductor_film(l, lobe_half_cos(wo, wi)));
}

// conductor_eval with the film's reflectance at the microfacet given
// (conductor_film).
fn conductor_eval_given(l: Conductor, wo: vec3<f32>, wi: vec3<f32>, filmed: vec4<f32>) -> vec3<f32> {
    if (wo.z <= 0.0 || wi.z <= 0.0 || ggx_is_smooth(l.distribution)) {
        return vec3<f32>(0.0);
    }
    let h = wo + wi;
    let len2 = dot(h, h);
    if (!(len2 > 0.0)) {
        return vec3<f32>(0.0);
    }
    var wm = h * (1.0 / sqrt(len2));
    if (wm.z < 0.0) {
        wm = -wm;
    }
    let d = ggx_d(l.distribution, wm);
    let g2 = ggx_g2(l.distribution, wo, wi);
    let f = conductor_reflectance_given(l, abs(dot(wo, wm)), filmed);
    let single = f * (d * g2 / (4.0 * wo.z * wi.z));
    let alpha = ggx_average_alpha(l.distribution);
    let e_o = energy_white_directional(wo.z, alpha);
    let e_i = energy_white_directional(wi.z, alpha);
    let e_avg = energy_white_average(alpha);
    let multi = conductor_compensation_scale(l, e_avg) * multiple_scattering_value(e_o, e_i, e_avg);
    return (single + multi) * l.tint;
}

fn conductor_pdf(l: Conductor, wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    if (wo.z <= 0.0 || wi.z <= 0.0 || ggx_is_smooth(l.distribution)) {
        return 0.0;
    }
    let p_comp = conductor_compensation_probability(l, wo);
    return (1.0 - p_comp) * ggx_reflection_pdf(l.distribution, wo, wi) + p_comp * cosine_pdf(wi.z);
}

fn conductor_draw(l: Conductor, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    return conductor_draw_given(l, wo, u, uc, conductor_film(l, wo.z));
}

// conductor_draw with the film's reflectance seen at wo.z given.
fn conductor_draw_given(l: Conductor, wo: vec3<f32>, u: vec2<f32>, uc: f32, filmed: vec4<f32>) -> LobeSample {
    if (wo.z <= 0.0) {
        return no_sample();
    }
    if (ggx_is_smooth(l.distribution)) {
        let wi = vec3<f32>(-wo.x, -wo.y, wo.z);
        let f = conductor_reflectance_given(l, wo.z, filmed);
        return LobeSample(true, wi, f * l.tint / abs(wi.z), 1.0, true, false, 1.0);
    }
    let p_comp = conductor_compensation_probability(l, wo);
    var wi: vec3<f32>;
    if (uc < p_comp) {
        wi = lobe_cosine_hemisphere(u);
    } else {
        let wm = ggx_sample_visible_normal(l.distribution, wo, u);
        wi = -wo + wm * (2.0 * dot(wo, wm));
        if (wi.z <= 0.0) {
            return no_sample();
        }
    }
    if (wi.z <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, wi, vec3<f32>(0.0), 0.0, false, false, 1.0);
}

fn conductor_sample(l: Conductor, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    let s = conductor_draw(l, wo, u, uc);
    if (!s.ok || s.is_delta) {
        return s;
    }
    let pdf = conductor_pdf(l, wo, s.wi);
    if (pdf <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, s.wi, conductor_eval(l, wo, s.wi), pdf, false, false, 1.0);
}

fn conductor_albedo(l: Conductor, wo: vec3<f32>) -> vec3<f32> {
    let mu = abs(wo.z);
    if (ggx_is_smooth(l.distribution)) {
        return clamp(conductor_reflectance(l, mu) * l.tint, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    let f0 = fresnel_conductor(1.0, l.n, l.k);
    let alpha = ggx_average_alpha(l.distribution);
    let single = energy_conductor_directional(mu, alpha, f0);
    let e_o = energy_white_directional(mu, alpha);
    let e_avg = energy_white_average(alpha);
    return clamp((single + conductor_compensation_scale(l, e_avg) * (1.0 - e_o)) * l.tint, vec3<f32>(0.0), vec3<f32>(1.0));
}

// --- DielectricTransmission --------------------------------------------
//
// Rough or smooth glass at one interface: reflect or refract by Fresnel on
// the microfacet, the reflect-or-refract choice made with `uc`, the stack's
// rescaled lobe variate; a thin wall leaves straight on (smooth) or off the
// mirrored half vector (rough). The side the ray arrives from is the
// caller's to say (`from_inside`): the shading frame has made wo.z positive
// whichever side it is on. The whole lobe is scaled by the reciprocal of
// what the interface passes on (Turquin), from the tables the reflection
// lobes read.

struct Transmission {
    distribution: Ggx,
    // The inside's index over the outside's, at least 1.
    ior: f32,
    tint: vec3<f32>,
    thin: bool,
    from_inside: bool,
};

// DielectricTransmission::new, with_tint, thin_walled, from_inside: with no
// index contrast every facet passes light straight on, so the surface is
// smooth whatever its roughness. Isotropic.
fn transmission_new(roughness: f32, ior: f32, tint: vec3<f32>, thin: bool, from_inside: bool) -> Transmission {
    let index_matched = ior <= 1.0 + 1e-3;
    return Transmission(ggx_from_roughness(select(roughness, 0.0, index_matched), 0.0), max(ior, 1.0), tint, thin, from_inside);
}

// relative_ior: the index ahead over the index behind, for the side the
// ray arrives from.
fn transmission_eta(l: Transmission) -> f32 {
    return select(l.ior, 1.0 / l.ior, l.from_inside);
}

fn transmission_energy_scale(l: Transmission, wo: vec3<f32>) -> f32 {
    if (ggx_is_smooth(l.distribution)) {
        return 1.0;
    }
    let mu = abs(wo.z);
    let alpha = ggx_average_alpha(l.distribution);
    var passed: f32;
    if (l.thin) {
        passed = energy_white_directional(mu, alpha);
    } else {
        passed = energy_interface_directional(mu, alpha, l.ior, l.from_inside);
    }
    if (passed <= 0.0) {
        return 1.0;
    }
    return min(1.0 / passed, 4.0);
}

struct Refracted {
    ok: bool,
    wi: vec3<f32>,
};

// DielectricTransmission::refract: `eta` already for the side `wi` arrives
// from, the normal flipped to that side; not ok past the critical angle.
fn transmission_refract(wi: vec3<f32>, normal_in: vec3<f32>, eta: f32) -> Refracted {
    var cos_i = dot(normal_in, wi);
    var normal = normal_in;
    if (cos_i < 0.0) {
        cos_i = -cos_i;
        normal = -normal_in;
    }
    let sin2_i = max(1.0 - cos_i * cos_i, 0.0);
    let sin2_t = sin2_i / (eta * eta);
    if (sin2_t >= 1.0) {
        return Refracted(false, vec3<f32>(0.0));
    }
    let cos_t = sqrt(1.0 - sin2_t);
    return Refracted(true, -wi * (1.0 / eta) + normal * (cos_i / eta - cos_t));
}

fn transmission_reflect(wo: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return -wo + n * (2.0 * dot(wo, n));
}

// The half vector eval and pdf share: the generalized one through a solid's
// surface, the plain one off it or through a thin wall (whose way through
// is the reflection mirrored); oriented to the macrosurface for the test
// that a facet faces both directions, then to the side wo is on.
struct TransmissionHalf {
    ok: bool,
    reflecting: bool,
    h: vec3<f32>,
    wi_e: vec3<f32>,
    eta: f32,
};

fn transmission_half(l: Transmission, wo: vec3<f32>, wi: vec3<f32>) -> TransmissionHalf {
    var r: TransmissionHalf;
    r.ok = false;
    let cos_o = wo.z;
    let cos_i = wi.z;
    if (cos_o == 0.0 || cos_i == 0.0) {
        return r;
    }
    let eta = transmission_eta(l);
    let reflecting = cos_o * cos_i > 0.0;
    var wi_e = wi;
    if (!reflecting && l.thin) {
        wi_e = vec3<f32>(wi.x, wi.y, -wi.z);
    }
    var h: vec3<f32>;
    if (reflecting || l.thin) {
        h = wo + wi_e;
    } else {
        h = wo + wi * eta;
    }
    let len2 = h.x * h.x + h.y * h.y + h.z * h.z;
    if (!(len2 > 0.0 && len2 <= 3.40282347e38)) {
        return r;
    }
    h = h * (1.0 / sqrt(len2));
    let upward = select(h, -h, h.z < 0.0);
    if (dot(upward, wo) * cos_o <= 0.0 || dot(upward, wi_e) * wi_e.z <= 0.0) {
        return r;
    }
    return TransmissionHalf(true, reflecting, select(upward, -upward, dot(upward, wo) < 0.0), wi_e, eta);
}

fn transmission_eval(l: Transmission, wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    if (ggx_is_smooth(l.distribution)) {
        return vec3<f32>(0.0);
    }
    let hv = transmission_half(l, wo, wi);
    if (!hv.ok) {
        return vec3<f32>(0.0);
    }
    let h = hv.h;
    let d = ggx_d(l.distribution, h);
    let f = fresnel_dielectric(abs(dot(wo, h)), hv.eta);
    let compensation = transmission_energy_scale(l, wo);
    let cos_o = wo.z;
    let cos_i = wi.z;
    if (hv.reflecting) {
        let g = ggx_g2(l.distribution, wo, wi);
        return vec3<f32>(compensation * f * d * g / (4.0 * abs(cos_o * cos_i)));
    }
    if (l.thin) {
        let g = ggx_g2(l.distribution, wo, hv.wi_e);
        return l.tint * (compensation * (1.0 - f) * d * g / (4.0 * abs(cos_o * cos_i)));
    }
    let g = ggx_g2(l.distribution, wo, wi);
    let sum = dot(wo, h) + hv.eta * dot(wi, h);
    let denominator = sum * sum;
    if (denominator <= 1e-9) {
        return vec3<f32>(0.0);
    }
    let value = compensation * (1.0 - f) * d * g * abs(dot(wi, h) * dot(wo, h)) / (denominator * abs(cos_o * cos_i));
    return l.tint * value;
}

fn transmission_pdf(l: Transmission, wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    if (ggx_is_smooth(l.distribution)) {
        return 0.0;
    }
    let hv = transmission_half(l, wo, wi);
    if (!hv.ok) {
        return 0.0;
    }
    let h = hv.h;
    let f = fresnel_dielectric(abs(dot(wo, h)), hv.eta);
    let visible = ggx_d_visible(l.distribution, wo, h);
    let cos_oh = abs(dot(wo, h));
    if (hv.reflecting) {
        return visible * f / (4.0 * cos_oh);
    }
    if (l.thin) {
        return visible * (1.0 - f) / (4.0 * cos_oh);
    }
    let sum = dot(wo, h) + hv.eta * dot(wi, h);
    let denominator = sum * sum;
    if (denominator <= 1e-9) {
        return 0.0;
    }
    return visible * (1.0 - f) * abs(hv.eta * hv.eta * dot(wi, h)) / denominator;
}

fn transmission_draw(l: Transmission, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    let eta = transmission_eta(l);
    if (ggx_is_smooth(l.distribution)) {
        let f = fresnel_dielectric(abs(wo.z), eta);
        if (uc < f) {
            let wi = vec3<f32>(-wo.x, -wo.y, wo.z);
            return LobeSample(true, wi, vec3<f32>(1.0 / abs(wi.z)), 1.0, true, false, 1.0);
        }
        var wi = -wo;
        var scale = 1.0;
        var crossing = 1.0;
        if (!l.thin) {
            let r = transmission_refract(wo, vec3<f32>(0.0, 0.0, 1.0), eta);
            if (!r.ok) {
                return no_sample();
            }
            wi = r.wi;
            scale = 1.0 / (eta * eta);
            crossing = eta;
        }
        return LobeSample(true, wi, l.tint * (scale / abs(wi.z)), 1.0, true, true, crossing);
    }
    let h = ggx_sample_visible_normal(l.distribution, wo, u);
    let f = fresnel_dielectric(abs(dot(wo, h)), eta);
    var wi: vec3<f32>;
    var through = false;
    var crossing = 1.0;
    if (uc < f) {
        wi = transmission_reflect(wo, h);
    } else if (l.thin) {
        let reflected = transmission_reflect(wo, h);
        wi = vec3<f32>(reflected.x, reflected.y, -reflected.z);
        through = true;
    } else {
        let r = transmission_refract(wo, h, eta);
        if (r.ok) {
            wi = r.wi;
            through = true;
            crossing = eta;
        } else {
            wi = transmission_reflect(wo, h);
        }
    }
    // On the side of the macrosurface its branch implies, or shadowed.
    let same_side = wi.z * wo.z > 0.0;
    if (same_side == through) {
        return no_sample();
    }
    return LobeSample(true, wi, vec3<f32>(0.0), 0.0, false, through, crossing);
}

fn transmission_sample(l: Transmission, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    let s = transmission_draw(l, wo, u, uc);
    if (!s.ok || s.is_delta) {
        return s;
    }
    let pdf = transmission_pdf(l, wo, s.wi);
    if (pdf <= 0.0) {
        return no_sample();
    }
    let value = transmission_eval(l, wo, s.wi);
    if (all(value == vec3<f32>(0.0))) {
        return no_sample();
    }
    return LobeSample(true, s.wi, value, pdf, false, s.is_transmission, s.eta);
}

fn transmission_albedo(l: Transmission) -> vec3<f32> {
    return clamp(l.tint, vec3<f32>(0.0), vec3<f32>(1.0));
}
