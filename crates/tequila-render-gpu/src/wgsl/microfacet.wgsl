// GGX: tequila_materials::microfacet, bit for bit where WGSL's arithmetic
// allows. In the shading frame, z along the normal.

const SMOOTH_ALPHA_THRESHOLD: f32 = 1e-3;
const MF_PI: f32 = 3.14159265358979323846;
const MF_TAU: f32 = 6.28318530717958647692;

struct Ggx {
    alpha_x: f32,
    alpha_y: f32,
};

fn sqr(x: f32) -> f32 {
    return x * x;
}

fn ggx_isotropic(alpha: f32) -> Ggx {
    return Ggx(alpha, alpha);
}

// Ggx::from_roughness.
fn ggx_from_roughness(roughness: f32, anisotropy: f32) -> Ggx {
    let alpha = sqr(clamp(roughness, 0.0, 1.0));
    let aniso = clamp(anisotropy, 0.0, 0.99);
    if (aniso <= 0.0) {
        return ggx_isotropic(alpha);
    }
    let ratio = sqrt(1.0 - aniso * aniso);
    return Ggx(min(alpha / sqrt(ratio), 1.0), max(alpha * sqrt(ratio), 0.0));
}

fn ggx_is_smooth(g: Ggx) -> bool {
    return g.alpha_x < SMOOTH_ALPHA_THRESHOLD && g.alpha_y < SMOOTH_ALPHA_THRESHOLD;
}

fn ggx_regularized(g: Ggx, min_alpha: f32) -> Ggx {
    return Ggx(max(g.alpha_x, min_alpha), max(g.alpha_y, min_alpha));
}

fn ggx_roughness(g: Ggx) -> f32 {
    return sqrt(max(max(g.alpha_x, g.alpha_y), 0.0));
}

fn ggx_average_alpha(g: Ggx) -> f32 {
    return sqrt(g.alpha_x * g.alpha_y);
}

fn ggx_d(g: Ggx, wm: vec3<f32>) -> f32 {
    let cos2 = wm.z * wm.z;
    if (cos2 <= 0.0) {
        return 0.0;
    }
    let denom = sqr(wm.x / g.alpha_x) + sqr(wm.y / g.alpha_y) + sqr(wm.z);
    if (denom <= 0.0) {
        return 0.0;
    }
    return 1.0 / (MF_PI * g.alpha_x * g.alpha_y * sqr(denom));
}

fn ggx_lambda(g: Ggx, w: vec3<f32>) -> f32 {
    let cos2 = w.z * w.z;
    if (cos2 <= 0.0) {
        return 0.0;
    }
    let alpha2_tan2 = (sqr(g.alpha_x * w.x) + sqr(g.alpha_y * w.y)) / cos2;
    if (alpha2_tan2 <= 0.0) {
        return 0.0;
    }
    return 0.5 * (sqrt(1.0 + alpha2_tan2) - 1.0);
}

fn ggx_g1(g: Ggx, w: vec3<f32>) -> f32 {
    return 1.0 / (1.0 + ggx_lambda(g, w));
}

fn ggx_g2(g: Ggx, wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    return 1.0 / (1.0 + ggx_lambda(g, wo) + ggx_lambda(g, wi));
}

fn ggx_d_visible(g: Ggx, wo: vec3<f32>, wm: vec3<f32>) -> f32 {
    let cos_o = abs(wo.z);
    if (cos_o <= 0.0) {
        return 0.0;
    }
    let cos_d = dot(wo, wm);
    if (cos_d <= 0.0) {
        return 0.0;
    }
    return ggx_g1(g, wo) * cos_d * ggx_d(g, wm) / cos_o;
}

fn mf_normalize(v: vec3<f32>) -> vec3<f32> {
    return v * (1.0 / sqrt(dot(v, v)));
}

// Ggx::sample_visible_normal [Heitz 2018].
fn ggx_sample_visible_normal(g: Ggx, wo: vec3<f32>, u: vec2<f32>) -> vec3<f32> {
    let flip = wo.z < 0.0;
    let wo_up = select(wo, -wo, flip);
    let vh = mf_normalize(vec3<f32>(g.alpha_x * wo_up.x, g.alpha_y * wo_up.y, wo_up.z));
    let len2 = vh.x * vh.x + vh.y * vh.y;
    let t1 = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-vh.y, vh.x, 0.0) / sqrt(len2), len2 > 0.0);
    let t2 = cross(vh, t1);
    let r = sqrt(max(u.x, 0.0));
    let phi = MF_TAU * u.y;
    let p1 = r * cos(phi);
    var p2 = r * sin(phi);
    let s = 0.5 * (1.0 + vh.z);
    p2 = (1.0 - s) * sqrt(max(1.0 - p1 * p1, 0.0)) + s * p2;
    let nh = t1 * p1 + t2 * p2 + vh * sqrt(max(1.0 - p1 * p1 - p2 * p2, 0.0));
    let wm = mf_normalize(vec3<f32>(g.alpha_x * nh.x, g.alpha_y * nh.y, max(nh.z, 1e-6)));
    return select(wm, -wm, flip);
}

fn ggx_reflection_pdf(g: Ggx, wo: vec3<f32>, wi: vec3<f32>) -> f32 {
    let h = wo + wi;
    let len2 = dot(h, h);
    if (!(len2 > 0.0)) {
        return 0.0;
    }
    var wm = h * (1.0 / sqrt(len2));
    if (dot(wm, wo) < 0.0) {
        wm = -wm;
    }
    let d = dot(wo, wm);
    if (d <= 0.0) {
        return 0.0;
    }
    return ggx_d_visible(g, wo, wm) / (4.0 * d);
}
