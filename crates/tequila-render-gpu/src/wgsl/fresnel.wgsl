// Fresnel: tequila_materials::fresnel.
//#include microfacet

fn fresnel_dielectric(cos_in: f32, eta_in: f32) -> f32 {
    var cos_i = clamp(cos_in, -1.0, 1.0);
    var eta = eta_in;
    if (cos_i < 0.0) {
        eta = 1.0 / eta;
        cos_i = -cos_i;
    }
    let sin2_i = max(1.0 - cos_i * cos_i, 0.0);
    let sin2_t = sin2_i / (eta * eta);
    if (sin2_t >= 1.0) {
        return 1.0;
    }
    let cos_t = sqrt(max(1.0 - sin2_t, 0.0));
    let r_parallel = (eta * cos_i - cos_t) / (eta * cos_i + cos_t);
    let r_perpendicular = (cos_i - eta * cos_t) / (cos_i + eta * cos_t);
    return 0.5 * (sqr(r_parallel) + sqr(r_perpendicular));
}

fn fresnel_conductor_scalar(cos_in: f32, n: f32, k: f32) -> f32 {
    let cos_i = clamp(cos_in, 0.0, 1.0);
    let cos2 = cos_i * cos_i;
    let sin2 = 1.0 - cos2;
    let n2 = n * n;
    let k2 = k * k;
    let t0 = n2 - k2 - sin2;
    let a2_plus_b2 = sqrt(max(t0 * t0 + 4.0 * n2 * k2, 0.0));
    let t1 = a2_plus_b2 + cos2;
    let a = sqrt(max(0.5 * (a2_plus_b2 + t0), 0.0));
    let t2 = 2.0 * a * cos_i;
    let rs = (t1 - t2) / (t1 + t2);
    let t3 = cos2 * a2_plus_b2 + sqr(sin2);
    let t4 = t2 * sin2;
    let rp = rs * (t3 - t4) / (t3 + t4);
    return 0.5 * (rs + rp);
}

fn fresnel_conductor(cos_i: f32, n: vec3<f32>, k: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        fresnel_conductor_scalar(cos_i, n.x, k.x),
        fresnel_conductor_scalar(cos_i, n.y, k.y),
        fresnel_conductor_scalar(cos_i, n.z, k.z),
    );
}

fn fresnel_conductor_average(f0: vec3<f32>) -> vec3<f32> {
    return f0 + (vec3<f32>(1.0) - f0) * (1.0 / 21.0);
}

// The Kulla-Conty fit, and its reciprocal below an index of one.
fn fresnel_dielectric_average(eta: f32) -> f32 {
    if (eta >= 1.0) {
        return clamp((eta - 1.0) / (4.08567 + 1.00071 * eta), 0.0, 1.0);
    }
    let inv = 1.0 / max(eta, 1e-4);
    let f = clamp((inv - 1.0) / (4.08567 + 1.00071 * inv), 0.0, 1.0);
    return clamp(1.0 - (1.0 - f) * eta * eta, 0.0, 1.0);
}
