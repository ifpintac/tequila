// The opaque lobes: tequila_materials::lobes' OrenNayarDiffuse (EON),
// DielectricSpecular and Conductor, without thin films (thin_film.wgsl, to
// come). Shading frame, z along the normal.
//#include energy

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

fn diffuse_sample(l: Diffuse, wo: vec3<f32>, u: vec2<f32>) -> LobeSample {
    if (wo.z <= 0.0) {
        return no_sample();
    }
    let wi = lobe_cosine_hemisphere(u);
    let pdf = cosine_pdf(wi.z);
    if (pdf <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, wi, diffuse_eval(l, wo, wi), pdf, false);
}

// --- DielectricSpecular (no film) --------------------------------------

struct Specular {
    distribution: Ggx,
    ior: f32,
    tint: vec3<f32>,
    weight: f32,
};

fn specular_new(roughness: f32, ior: f32) -> Specular {
    return Specular(ggx_from_roughness(roughness, 0.0), max(ior, 1.0), vec3<f32>(1.0), 1.0);
}

fn specular_reflectance(l: Specular, cos_theta: f32) -> vec3<f32> {
    return vec3<f32>(fresnel_dielectric(cos_theta, l.ior));
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
    let single = specular_reflectance(l, abs(dot(wo, wm))) * (d * g2 / (4.0 * wo.z * wi.z));
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

fn specular_sample(l: Specular, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    if (wo.z <= 0.0 || l.weight <= 0.0) {
        return no_sample();
    }
    if (ggx_is_smooth(l.distribution)) {
        let wi = vec3<f32>(-wo.x, -wo.y, wo.z);
        let f = specular_reflectance(l, wo.z) * l.weight;
        return LobeSample(true, wi, l.tint * f * (1.0 / abs(wi.z)), 1.0, true);
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
    let pdf = specular_pdf(l, wo, wi);
    if (pdf <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, wi, specular_eval(l, wo, wi), pdf, false);
}

// --- Conductor (no film) -----------------------------------------------

struct Conductor {
    distribution: Ggx,
    n: vec3<f32>,
    k: vec3<f32>,
    tint: vec3<f32>,
};

fn conductor_reflectance(l: Conductor, cos_theta: f32) -> vec3<f32> {
    return fresnel_conductor(cos_theta, l.n, l.k);
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
    let f = conductor_reflectance(l, abs(dot(wo, wm)));
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

fn conductor_sample(l: Conductor, wo: vec3<f32>, u: vec2<f32>, uc: f32) -> LobeSample {
    if (wo.z <= 0.0) {
        return no_sample();
    }
    if (ggx_is_smooth(l.distribution)) {
        let wi = vec3<f32>(-wo.x, -wo.y, wo.z);
        let f = conductor_reflectance(l, wo.z);
        return LobeSample(true, wi, f * l.tint / abs(wi.z), 1.0, true);
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
    let pdf = conductor_pdf(l, wo, wi);
    if (pdf <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, wi, conductor_eval(l, wo, wi), pdf, false);
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
