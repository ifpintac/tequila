// Metallic paint's flake field: tequila_render_core::flakes' apply, which
// says whether a shading point lies on a flake and how that flake leans.
// Worley cells a flake across in the part's object space, in scene units
// (the point's object position times its instance's scale over the flake's
// size), seeded as the processor seeds them; a cell holds a flake with
// chance flake_weight, and each flake takes a tilt from its own cell,
// uniform over the cap of directions within the spread. The stack turns it
// into a metal lobe under the coat in the flake's frame (openpbr.wgsl's
// LOBE_FLAKE). No draw of the sampler: the field is the surface's.
//#include procedural

// flakes.rs's MAX_TILT, sixty degrees (FRAC_PI_3 as an f32).
const FLAKE_MAX_TILT: f32 = 1.04719755119659774615;
// The field's seed.
const FLAKE_SEED: u32 = 0x0f1a4e5du;

// flakes.rs's unit: a number in [0, 1) from a cell's bits and a stream
// index, exactly (the top 24 bits over 2^24).
fn flake_unit(bits: u32, stream: u32) -> f32 {
    var h = bits ^ (stream * 0x9e3779b9u);
    h ^= h >> 16u;
    h = h * 0x7feb352du;
    h ^= h >> 15u;
    h = h * 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// noise.rs's worley, Euclidean, as procedural.wgsl's tq_worley finds it -
// the nearest feature point of the 27 cells round `p`, the same cells in the
// same order - and of it only the nearest one's id: the flake's. Its own
// copy, its count opaque to the compiler so it keeps one copy of the cell's
// arithmetic rather than 27.
fn flake_cell(p: vec3<f32>, seed: u32) -> f32 {
    let base = floor(p);
    var f1 = CY_F32_MAX;
    var id = 0.0;
    for (var k = 0u; k < (27u | cy_zero); k++) {
        let o = vec3<f32>(f32(k % 3u) - 1.0, f32((k / 3u) % 3u) - 1.0, f32(k / 9u) - 1.0);
        let cell = base + o;
        let c = vec3<i32>(cy_to_i32(cell.x), cy_to_i32(cell.y), cy_to_i32(cell.z));
        let point = cell + vec3<f32>(tq_hash01(c, seed), tq_hash01(c, seed ^ 0x5bf03635u), tq_hash01(c, seed ^ 0x1b873593u));
        let d = tq_distance(point - p, 0u);
        if (d < f1) {
            f1 = d;
            id = tq_hash01(c, seed ^ 0x68bc21ebu);
        }
    }
    return id;
}

// flakes::apply at a point `object` of a part of scale `scale`, for a flake
// of `size` covering `weight` of the surface and leaning up to `spread` of
// sixty degrees: the flake's normal in the shading frame and, in w, the
// point's coverage, 1 on a flake and 0 off one (the normal then the
// material's own, straight up).
fn flake_at(object: vec3<f32>, scale: f32, size_in: f32, weight: f32, spread: f32) -> vec4<f32> {
    let size = max(size_in, 1e-7);
    let cell = flake_cell(cy_exact3(object * cy_div(scale, size)), FLAKE_SEED);
    // Three numbers per cell, from the one the lookup gives.
    let seed = bitcast<u32>(cell);
    if (flake_unit(seed, 1u) >= weight) {
        return vec4<f32>(0.0, 0.0, 1.0, 0.0);
    }
    let a = flake_unit(seed, 2u);
    let b = flake_unit(seed, 3u);
    let cap = cy_cos(cy_exact(clamp(spread, 0.0, 1.0) * FLAKE_MAX_TILT));
    let cos_theta = 1.0 - cy_exact(a * (1.0 - cap));
    let sin_theta = cy_sqrt(max(1.0 - cy_exact(cos_theta * cos_theta), 0.0));
    let turn = cy_sincos(cy_exact(b * CY_TAU));
    return vec4<f32>(cy_exact(sin_theta * turn.y), cy_exact(sin_theta * turn.x), cos_theta, 1.0);
}
