enable wgpu_ray_query;
// A scene on the card: scene.rs's records, and the buffers and acceleration
// structure GpuScene uploads, at @group(0) bindings 0-5.

struct GpuMesh {
    first_index: u32,
    base_vertex: u32,
    has_normals: u32,
    vertex_count: u32,
    // Its first UV and first tangent, NO_DATA for a mesh without.
    uv_base: u32,
    tangent_base: u32,
    // Its first vertex in scene_attributes and which of the measures it
    // has (scene.rs's HAS_*), NO_DATA for none.
    attribute_base: u32,
    attribute_flags: u32,
    // Its bounds in its own space, for the Generated coordinate.
    bounds_min: vec4<f32>,
    bounds_max: vec4<f32>,
};

const NO_DATA: u32 = 0xFFFFFFFFu;
const HAS_CURVATURE: u32 = 1u;
const HAS_OCCLUSION: u32 = 2u;
const HAS_CLOSENESS: u32 = 4u;
const HAS_COLORS: u32 = 8u;

struct GpuInstance {
    // Row-major 3 x 4, as Transform::matrix and Transform::inverse.
    object_to_world: array<vec4<f32>, 3>,
    world_to_object: array<vec4<f32>, 3>,
    mesh: u32,
    material: u32,
    instance: u32,
    // Its shadow catcher's record (scene.rs's GpuCatcher), NO_CATCHER for an
    // ordinary surface.
    catcher: u32,
    // The lamp it stands in for, by its place among the scene's lamps;
    // NO_LIGHT for any other instance.
    light: u32,
    // INSTANCE_STAND_IN for a lamp's stand-in, INSTANCE_CASTS_SHADOWS for a
    // part that casts shadows.
    flags: u32,
    // The solid a medium is keyed by (scene.rs).
    solid: u32,
    // Where its triangles start among the glowing triangles' leaves
    // (emitters.wgsl), EMITTER_NONE for a part that is not one of those
    // lights.
    emitter: u32,
    // What Object Info reads, and the part's scale as the shading point
    // holds it (scene.rs's GpuInstance).
    object_color: vec4<f32>,
    inv_length_scale: f32,
    scale: f32,
    object_random: f32,
    object_number: f32,
};

const NO_CATCHER: u32 = 0xFFFFFFFFu;
const NO_LIGHT: u32 = 0xFFFFFFFFu;
const INSTANCE_STAND_IN: u32 = 1u;
const INSTANCE_CASTS_SHADOWS: u32 = 2u;
// The top-level structure's masks (scene.rs's MASK_*): every instance is a
// surface, and those that cast shadows are in the shadow rays' mask too.
const MASK_SURFACE: u32 = 1u;
const MASK_SHADOW: u32 = 2u;
// GpuCatcher's fade.
const FADE_NONE: u32 = 0u;
const FADE_BY_HEIGHT: u32 = 1u;
const FADE_BY_DISTANCE: u32 = 2u;

// A shadow catcher (scene.rs's GpuCatcher, ShadowCatcher's fields). A
// kernel that reads them binds them; this module only declares the record.
struct GpuCatcher {
    shadow_color: vec4<f32>,
    // The glow the product sees of the floor, and in w 1 when there is one.
    bounce: vec4<f32>,
    reflections: f32,
    reflection_roughness: f32,
    reflection_fade: f32,
    fade: u32,
    reflection_fresnel: u32,
    occlusion_only: u32,
    occlusion_radius: f32,
    shadows: u32,
    bounce_falloff: f32,
    _pad: array<u32, 3>,
};

@group(0) @binding(0) var scene_tlas: acceleration_structure;
@group(0) @binding(1) var<storage, read> scene_positions: array<f32>;
@group(0) @binding(2) var<storage, read> scene_normals: array<f32>;
@group(0) @binding(3) var<storage, read> scene_indices: array<u32>;
@group(0) @binding(4) var<storage, read> scene_meshes: array<GpuMesh>;
@group(0) @binding(5) var<storage, read> scene_instances: array<GpuInstance>;
// Every mesh's UVs (two floats a vertex) and tangents (four), from its
// uv_base and tangent_base.
@group(0) @binding(14) var<storage, read> scene_uvs: array<f32>;
@group(0) @binding(15) var<storage, read> scene_tangents: array<vec4<f32>>;
// Every mesh's per-vertex measures (Mesh's curvature, occlusion, closeness
// and colours), two vec4s a vertex from its attribute_base: (curvature,
// occlusion, closeness, 0), (r, g, b, a) in linear sRGB.
@group(0) @binding(20) var<storage, read> scene_attributes: array<vec4<f32>>;
// The scene's materials (material.rs's GpuMaterial, declared in
// openpbr.wgsl), by an instance's `material`. A kernel that reads them
// includes openpbr; this module only names the binding.

fn scene_position(i: u32) -> vec3<f32> {
    return vec3<f32>(scene_positions[3u * i], scene_positions[3u * i + 1u], scene_positions[3u * i + 2u]);
}

fn scene_normal(i: u32) -> vec3<f32> {
    return vec3<f32>(scene_normals[3u * i], scene_normals[3u * i + 1u], scene_normals[3u * i + 2u]);
}

// Row-major 3 x 4 times a point / a vector.
fn rows_point(m: array<vec4<f32>, 3>, p: vec3<f32>) -> vec3<f32> {
    let q = vec4<f32>(p, 1.0);
    return vec3<f32>(dot(m[0], q), dot(m[1], q), dot(m[2], q));
}

fn rows_vector(m: array<vec4<f32>, 3>, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(m[0].xyz, v), dot(m[1].xyz, v), dot(m[2].xyz, v));
}

// A normal through the inverse's transpose.
fn rows_normal(inverse: array<vec4<f32>, 3>, n: vec3<f32>) -> vec3<f32> {
    return inverse[0].xyz * n.x + inverse[1].xyz * n.y + inverse[2].xyz * n.z;
}
