// The GGX energy tables: tequila_materials::energy's lookups, reading its
// own tables (GgxEnergyTable::raw, uploaded by tables.rs at @group(1)
// @binding(1)), and the Kulla-Conty multiple-scattering terms.
//#include fresnel

@group(1) @binding(1) var<storage, read> energy_tables: array<f32>;

const E_MU_STEPS: u32 = 32u;
const E_ROUGHNESS_STEPS: u32 = 32u;
const E_IOR_STEPS: u32 = 16u;
const E_SIDES: u32 = 2u;
const E_MAX_IOR_PARAM: f32 = 0.5;
// Where each of ENERGY_TABLE_LAYOUT's tables starts (tables.rs checks).
const E_SCALE: u32 = 0u;
const E_BIAS: u32 = 1024u;
const E_AVERAGE_SCALE: u32 = 2048u;
const E_AVERAGE_BIAS: u32 = 2080u;
const E_WHITE_AVERAGE: u32 = 2112u;
const E_DIELECTRIC: u32 = 2144u;
const E_DIELECTRIC_AVERAGE: u32 = 18528u;
const E_INTERFACE: u32 = 19040u;

struct GridAt {
    i: u32,
    f: f32,
};

fn grid_at(x: f32, steps: u32) -> GridAt {
    let scaled = clamp(x, 0.0, 1.0) * f32(steps - 1u);
    let i = min(u32(scaled), steps - 2u);
    return GridAt(i, clamp(scaled - f32(i), 0.0, 1.0));
}

fn mu_param(mu: f32) -> f32 {
    return sqrt(clamp(mu, 0.0, 1.0));
}

fn ior_param(ior_in: f32) -> f32 {
    let ior = max(ior_in, 1.0);
    return sqrt(clamp((ior - 1.0) / (ior + 1.0) / E_MAX_IOR_PARAM, 0.0, 1.0));
}

fn roughness_at(alpha: f32) -> GridAt {
    return grid_at(sqrt(max(alpha, 0.0)), E_ROUGHNESS_STEPS);
}

// pair_at, one table at a time.
fn table2d(base: u32, r: GridAt, m: GridAt) -> f32 {
    let ri1 = min(r.i + 1u, E_ROUGHNESS_STEPS - 1u);
    let mi1 = min(m.i + 1u, E_MU_STEPS - 1u);
    let v00 = energy_tables[base + r.i * E_MU_STEPS + m.i];
    let v01 = energy_tables[base + r.i * E_MU_STEPS + mi1];
    let v10 = energy_tables[base + ri1 * E_MU_STEPS + m.i];
    let v11 = energy_tables[base + ri1 * E_MU_STEPS + mi1];
    let v0 = v00 + (v01 - v00) * m.f;
    let v1 = v10 + (v11 - v10) * m.f;
    return v0 + (v1 - v0) * r.f;
}

fn table1d(base: u32, r: GridAt) -> f32 {
    let ri1 = min(r.i + 1u, E_ROUGHNESS_STEPS - 1u);
    let a = energy_tables[base + r.i];
    return a + (energy_tables[base + ri1] - a) * r.f;
}

fn energy_white_directional(cos_theta: f32, alpha: f32) -> f32 {
    let r = roughness_at(alpha);
    let m = grid_at(mu_param(cos_theta), E_MU_STEPS);
    return clamp(table2d(E_SCALE, r, m) + table2d(E_BIAS, r, m), 0.0, 1.0);
}

fn energy_white_average(alpha: f32) -> f32 {
    return clamp(table1d(E_WHITE_AVERAGE, roughness_at(alpha)), 0.0, 1.0);
}

fn energy_conductor_directional(cos_theta: f32, alpha: f32, f0: vec3<f32>) -> vec3<f32> {
    let r = roughness_at(alpha);
    let m = grid_at(mu_param(cos_theta), E_MU_STEPS);
    return clamp(f0 * table2d(E_SCALE, r, m) + vec3<f32>(table2d(E_BIAS, r, m)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn energy_conductor_average(alpha: f32, f0: vec3<f32>) -> vec3<f32> {
    let r = roughness_at(alpha);
    return clamp(f0 * table1d(E_AVERAGE_SCALE, r) + vec3<f32>(table1d(E_AVERAGE_BIAS, r)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn lerp1(a: f32, b: f32, t: f32) -> f32 {
    return a + (b - a) * t;
}

fn energy_dielectric_directional(cos_theta: f32, alpha: f32, ior: f32) -> f32 {
    let r = roughness_at(alpha);
    let ii = grid_at(ior_param(ior), E_IOR_STEPS);
    let m = grid_at(mu_param(cos_theta), E_MU_STEPS);
    let ri1 = min(r.i + 1u, E_ROUGHNESS_STEPS - 1u);
    let ii1 = min(ii.i + 1u, E_IOR_STEPS - 1u);
    let mi1 = min(m.i + 1u, E_MU_STEPS - 1u);
    let b = E_DIELECTRIC;
    let m00 = lerp1(energy_tables[b + (r.i * E_IOR_STEPS + ii.i) * E_MU_STEPS + m.i], energy_tables[b + (r.i * E_IOR_STEPS + ii.i) * E_MU_STEPS + mi1], m.f);
    let m01 = lerp1(energy_tables[b + (r.i * E_IOR_STEPS + ii1) * E_MU_STEPS + m.i], energy_tables[b + (r.i * E_IOR_STEPS + ii1) * E_MU_STEPS + mi1], m.f);
    let m10 = lerp1(energy_tables[b + (ri1 * E_IOR_STEPS + ii.i) * E_MU_STEPS + m.i], energy_tables[b + (ri1 * E_IOR_STEPS + ii.i) * E_MU_STEPS + mi1], m.f);
    let m11 = lerp1(energy_tables[b + (ri1 * E_IOR_STEPS + ii1) * E_MU_STEPS + m.i], energy_tables[b + (ri1 * E_IOR_STEPS + ii1) * E_MU_STEPS + mi1], m.f);
    return clamp(lerp1(lerp1(m00, m01, ii.f), lerp1(m10, m11, ii.f), r.f), 0.0, 1.0);
}

fn energy_dielectric_hemispherical(alpha: f32, ior: f32) -> f32 {
    let r = roughness_at(alpha);
    let ii = grid_at(ior_param(ior), E_IOR_STEPS);
    let ri1 = min(r.i + 1u, E_ROUGHNESS_STEPS - 1u);
    let ii1 = min(ii.i + 1u, E_IOR_STEPS - 1u);
    let b = E_DIELECTRIC_AVERAGE;
    return clamp(lerp1(
        lerp1(energy_tables[b + r.i * E_IOR_STEPS + ii.i], energy_tables[b + r.i * E_IOR_STEPS + ii1], ii.f),
        lerp1(energy_tables[b + ri1 * E_IOR_STEPS + ii.i], energy_tables[b + ri1 * E_IOR_STEPS + ii1], ii.f),
        r.f,
    ), 0.0, 1.0);
}

fn energy_interface_directional(cos_theta: f32, alpha: f32, ior: f32, from_inside: bool) -> f32 {
    let r = roughness_at(alpha);
    let ii = grid_at(ior_param(ior), E_IOR_STEPS);
    let m = grid_at(mu_param(cos_theta), E_MU_STEPS);
    let ri1 = min(r.i + 1u, E_ROUGHNESS_STEPS - 1u);
    let ii1 = min(ii.i + 1u, E_IOR_STEPS - 1u);
    let mi1 = min(m.i + 1u, E_MU_STEPS - 1u);
    let side = select(0u, 1u, from_inside);
    let b = E_INTERFACE;
    let r0 = (r.i * E_SIDES + side) * E_IOR_STEPS;
    let r1 = (ri1 * E_SIDES + side) * E_IOR_STEPS;
    let m00 = lerp1(energy_tables[b + (r0 + ii.i) * E_MU_STEPS + m.i], energy_tables[b + (r0 + ii.i) * E_MU_STEPS + mi1], m.f);
    let m01 = lerp1(energy_tables[b + (r0 + ii1) * E_MU_STEPS + m.i], energy_tables[b + (r0 + ii1) * E_MU_STEPS + mi1], m.f);
    let m10 = lerp1(energy_tables[b + (r1 + ii.i) * E_MU_STEPS + m.i], energy_tables[b + (r1 + ii.i) * E_MU_STEPS + mi1], m.f);
    let m11 = lerp1(energy_tables[b + (r1 + ii1) * E_MU_STEPS + m.i], energy_tables[b + (r1 + ii1) * E_MU_STEPS + mi1], m.f);
    return clamp(lerp1(lerp1(m00, m01, ii.f), lerp1(m10, m11, ii.f), r.f), 0.0, 1.0);
}

fn multiple_scattering_value(e_o: f32, e_i: f32, e_avg: f32) -> f32 {
    let missing = 1.0 - e_avg;
    if (missing <= 1e-6) {
        return 0.0;
    }
    return ((1.0 - e_o) * (1.0 - e_i)) / (MF_PI * missing);
}

fn multiple_scattering_scale(f_avg: vec3<f32>, e_avg: f32) -> vec3<f32> {
    let denom = vec3<f32>(1.0) - f_avg * (1.0 - e_avg);
    if (min(min(denom.x, denom.y), denom.z) <= 1e-6) {
        return vec3<f32>(0.0);
    }
    return (f_avg * f_avg * e_avg) / denom;
}
