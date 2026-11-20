// R2.0 spike: a camera ray and AO_RAYS ambient-occlusion rays per pixel,
// through ray queries. Output: per pixel the unoccluded fraction, or -1 for
// a camera ray that met nothing.
enable wgpu_ray_query;

struct Params {
    origin: vec3<f32>,
    width: u32,
    right: vec3<f32>,   // camera +X scaled by the half width at distance 1
    height: u32,
    up: vec3<f32>,      // camera +Y scaled by the half height
    ao_rays: u32,
    forward: vec3<f32>,
    radius: f32,
    seed: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

struct MeshRange {
    first_index: u32,
    base_vertex: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var scene: acceleration_structure;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read> positions: array<f32>;
@group(0) @binding(3) var<storage, read> indices: array<u32>;
@group(0) @binding(4) var<storage, read> meshes: array<MeshRange>;
@group(0) @binding(5) var<storage, read_write> result: array<f32>;

fn hash(x: u32) -> u32 {
    var v = x;
    v ^= v >> 16u; v *= 0x7feb352du;
    v ^= v >> 15u; v *= 0x846ca68bu;
    v ^= v >> 16u;
    return v;
}

fn unit(x: u32) -> f32 {
    return f32(x >> 8u) * (1.0 / 16777216.0);
}

fn vertex(i: u32) -> vec3<f32> {
    return vec3<f32>(positions[3u * i], positions[3u * i + 1u], positions[3u * i + 2u]);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= params.width || id.y >= params.height) {
        return;
    }
    let pixel = id.y * params.width + id.x;
    let sx = 2.0 * (f32(id.x) + 0.5) / f32(params.width) - 1.0;
    let sy = 1.0 - 2.0 * (f32(id.y) + 0.5) / f32(params.height);
    let dir = normalize(params.forward + params.right * sx + params.up * sy);

    var rq: ray_query;
    rayQueryInitialize(&rq, scene, RayDesc(RAY_FLAG_FORCE_OPAQUE, 0xFFu, 0.0, 1e30, params.origin, dir));
    rayQueryProceed(&rq);
    let hit = rayQueryGetCommittedIntersection(&rq);
    if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
        result[pixel] = -1.0;
        return;
    }

    // The triangle's own normal, from its corners in the mesh, to the world.
    let range = meshes[hit.instance_custom_data];
    let first = range.first_index + 3u * hit.primitive_index;
    let a = vertex(range.base_vertex + indices[first]);
    let b = vertex(range.base_vertex + indices[first + 1u]);
    let c = vertex(range.base_vertex + indices[first + 2u]);
    var n = normalize((cross(b - a, c - a) * hit.world_to_object).xyz);
    if (dot(n, dir) > 0.0) {
        n = -n;
    }
    let p = params.origin + dir * hit.t + n * (1e-4 * params.radius);

    // A frame round the normal, and cosine-weighted directions in it.
    let s = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let t = normalize(cross(n, s));
    let bt = cross(n, t);
    var open = 0u;
    for (var k = 0u; k < params.ao_rays; k++) {
        let h = hash(pixel * 9781u + k * 6271u + params.seed * 26699u);
        let u1 = unit(h);
        let u2 = unit(hash(h ^ 0x9e3779b9u));
        let r = sqrt(u1);
        let phi = 6.2831853 * u2;
        let d = normalize(t * (r * cos(phi)) + bt * (r * sin(phi)) + n * sqrt(max(0.0, 1.0 - u1)));
        var shadow: ray_query;
        rayQueryInitialize(&shadow, scene, RayDesc(RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT, 0xFFu, 0.0, params.radius, p, d));
        rayQueryProceed(&shadow);
        if (rayQueryGetCommittedIntersection(&shadow).kind == RAY_QUERY_INTERSECTION_NONE) {
            open += 1u;
        }
    }
    result[pixel] = f32(open) / f32(params.ao_rays);
}
