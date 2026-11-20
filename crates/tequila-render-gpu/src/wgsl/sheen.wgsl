// The fuzz layer: tequila_materials::lobes::Sheen, Zeltner sheen as linearly
// transformed cosines, reading the processor's own tables (sheen::raw_tables:
// Cycles' fitted a, b and albedo, and the average albedo per roughness row,
// uploaded by tables.rs at @group(1) @binding(2)). Shading frame, z along
// the normal; unlike the other lobes it answers in the lower hemisphere too,
// mirrored, as the processor's does.
//#include lobes

@group(1) @binding(2) var<storage, read> sheen_tables: array<f32>;

const SHEEN_SIZE: u32 = 32u;
// Where each table starts (tables.rs checks): the three fitted tables end
// to end, [table][roughness][cos_o], then the 32 average albedos.
const SHEEN_LTC: u32 = 0u;
const SHEEN_AVERAGE: u32 = 3072u;

struct Sheen {
    color: vec3<f32>,
    roughness: f32,
    weight: f32,
};

fn sheen_new(color: vec3<f32>, roughness: f32, weight: f32) -> Sheen {
    return Sheen(color, clamp(roughness, 1e-3, 1.0), clamp(weight, 0.0, 1.0));
}

// Spectrum::max_component.
fn spectrum_max(c: vec3<f32>) -> f32 {
    return max(max(c.x, c.y), c.z);
}

// The fit for one view and roughness; `ok` is Cycles' test for a valid one.
struct SheenFit {
    a: f32,
    b: f32,
    albedo: f32,
    ok: bool,
};

struct SheenCell {
    i0: u32,
    i1: u32,
    t: f32,
};

fn sheen_cell(t: f32) -> SheenCell {
    let x = clamp(t, 0.0, 1.0) * f32(SHEEN_SIZE - 1u);
    let i = min(u32(x), SHEEN_SIZE - 1u);
    return SheenCell(i, min(i + 1u, SHEEN_SIZE - 1u), x - f32(i));
}

// sheen.rs's lookup: bilinear on the uniform grid, as Cycles'
// lookup_table_read_2D reads it.
fn sheen_lookup(table: u32, cos_o: f32, roughness: f32) -> f32 {
    let x = sheen_cell(cos_o);
    let y = sheen_cell(roughness);
    let base = SHEEN_LTC + table * SHEEN_SIZE * SHEEN_SIZE;
    let row0 = sheen_tables[base + y.i0 * SHEEN_SIZE + x.i0] * (1.0 - x.t) + sheen_tables[base + y.i0 * SHEEN_SIZE + x.i1] * x.t;
    let row1 = sheen_tables[base + y.i1 * SHEEN_SIZE + x.i0] * (1.0 - x.t) + sheen_tables[base + y.i1 * SHEEN_SIZE + x.i1] * x.t;
    return row0 * (1.0 - y.t) + row1 * y.t;
}

fn sheen_fit(l: Sheen, cos_o: f32) -> SheenFit {
    let a = sheen_lookup(0u, cos_o, l.roughness);
    let b = sheen_lookup(1u, cos_o, l.roughness);
    let albedo = sheen_lookup(2u, cos_o, l.roughness);
    return SheenFit(a, b, albedo, abs(a) >= 1e-5 && albedo >= 1e-5);
}

// The layer's directional albedo seen from `cos_o`, before colour and weight.
fn sheen_layer_albedo(l: Sheen, cos_o: f32) -> f32 {
    let fit = sheen_fit(l, abs(cos_o));
    return select(0.0, fit.albedo, fit.ok);
}

// The transformed cosine's density at `local`, in the fit's frame.
fn sheen_density(fit: SheenFit, local: vec3<f32>) -> f32 {
    let len_sqr = sqr(fit.a * local.x + fit.b * local.z) + sqr(fit.a * local.y) + sqr(local.z);
    if (local.z <= 0.0 || len_sqr <= 0.0) {
        return 0.0;
    }
    return L_FRAC_1_PI * local.z * sqr(fit.a / len_sqr);
}

// sheen.rs's average_albedo: the table's rows, linear in roughness.
fn sheen_average_albedo(roughness: f32) -> f32 {
    let y = clamp(roughness, 0.0, 1.0) * f32(SHEEN_SIZE - 1u);
    let i = min(u32(y), SHEEN_SIZE - 1u);
    let j = min(i + 1u, SHEEN_SIZE - 1u);
    let t = y - f32(i);
    return sheen_tables[SHEEN_AVERAGE + i] * (1.0 - t) + sheen_tables[SHEEN_AVERAGE + j] * t;
}

fn sheen_hemispherical_albedo(l: Sheen) -> f32 {
    return clamp(l.weight * spectrum_max(l.color) * sheen_average_albedo(l.roughness), 0.0, 1.0);
}

// What passes through the layer to whatever is beneath, seen from `cos_o`.
fn sheen_transmittance(l: Sheen, cos_o: f32) -> f32 {
    return clamp(1.0 - l.weight * spectrum_max(l.color) * sheen_layer_albedo(l, cos_o), 0.0, 1.0);
}

fn sheen_upper(w: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(w.x, w.y, abs(w.z));
}

// The azimuth of `wo` as a cosine and sine; any, seen face on.
fn sheen_azimuth(wo: vec3<f32>) -> vec2<f32> {
    let len = sqrt(wo.x * wo.x + wo.y * wo.y);
    if (len < 1e-7) {
        return vec2<f32>(1.0, 0.0);
    }
    return vec2<f32>(wo.x / len, wo.y / len);
}

fn sheen_to_fit_frame(wo: vec3<f32>, w: vec3<f32>) -> vec3<f32> {
    let cs = sheen_azimuth(wo);
    return vec3<f32>(cs.x * w.x + cs.y * w.y, -cs.y * w.x + cs.x * w.y, w.z);
}

fn sheen_from_fit_frame(wo: vec3<f32>, w: vec3<f32>) -> vec3<f32> {
    let cs = sheen_azimuth(wo);
    return vec3<f32>(cs.x * w.x - cs.y * w.y, cs.y * w.x + cs.x * w.y, w.z);
}

fn sheen_eval(l: Sheen, wo_in: vec3<f32>, wi_in: vec3<f32>) -> vec3<f32> {
    if (l.weight <= 0.0 || !(wo_in.z * wi_in.z > 0.0)) {
        return vec3<f32>(0.0);
    }
    let wo = sheen_upper(wo_in);
    let wi = sheen_upper(wi_in);
    if (wo.z <= 1e-6 || wi.z <= 1e-6) {
        return vec3<f32>(0.0);
    }
    let fit = sheen_fit(l, wo.z);
    if (!fit.ok) {
        return vec3<f32>(0.0);
    }
    // The table's value is the lobe times the cosine; this is the lobe alone.
    let lobe = fit.albedo * sheen_density(fit, sheen_to_fit_frame(wo, wi)) / wi.z;
    return l.color * (l.weight * lobe);
}

fn sheen_pdf(l: Sheen, wo_in: vec3<f32>, wi_in: vec3<f32>) -> f32 {
    if (l.weight <= 0.0 || !(wo_in.z * wi_in.z > 0.0)) {
        return 0.0;
    }
    let wo = sheen_upper(wo_in);
    let wi = sheen_upper(wi_in);
    let fit = sheen_fit(l, wo.z);
    if (fit.ok && wo.z > 1e-6) {
        return sheen_density(fit, sheen_to_fit_frame(wo, wi));
    }
    return 0.0;
}

// Vec3::try_normalize's test: a length that is positive and finite.
fn sheen_finite(x: f32) -> bool {
    return abs(x) <= 3.40282347e38;
}

fn sheen_draw(l: Sheen, wo: vec3<f32>, u: vec2<f32>) -> LobeSample {
    if (l.weight <= 0.0) {
        return no_sample();
    }
    let up = sheen_upper(wo);
    if (up.z <= 1e-6) {
        return no_sample();
    }
    let fit = sheen_fit(l, up.z);
    if (!fit.ok) {
        return no_sample();
    }
    // A cosine sample through the transform, scaled by `a` so that a tight
    // fit cannot divide by a small number.
    let d = lobe_cosine_hemisphere(u);
    let v = vec3<f32>(d.x - d.z * fit.b, d.y, d.z * fit.a);
    let len2 = v.x * v.x + v.y * v.y + v.z * v.z;
    if (!(len2 > 0.0 && sheen_finite(len2))) {
        return no_sample();
    }
    let local = v * (1.0 / sqrt(len2));
    if (local.z <= 1e-6) {
        return no_sample();
    }
    var wi = sheen_from_fit_frame(up, local);
    if (wo.z < 0.0) {
        wi.z = -wi.z;
    }
    return LobeSample(true, wi, vec3<f32>(0.0), 0.0, false, false, 1.0);
}

// The draw (lobes.wgsl's diffuse_draw), kept where its own density is.
fn sheen_sample(l: Sheen, wo: vec3<f32>, u: vec2<f32>) -> LobeSample {
    let s = sheen_draw(l, wo, u);
    if (!s.ok) {
        return s;
    }
    let pdf = sheen_pdf(l, wo, s.wi);
    if (pdf <= 0.0) {
        return no_sample();
    }
    return LobeSample(true, s.wi, sheen_eval(l, wo, s.wi), pdf, false, false, 1.0);
}

fn sheen_albedo(l: Sheen, wo: vec3<f32>) -> vec3<f32> {
    return l.color * (l.weight * sheen_layer_albedo(l, abs(wo.z)));
}
