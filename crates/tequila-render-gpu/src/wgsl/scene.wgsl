enable wgpu_ray_query;
// A scene on the card: scene.rs's records, and the buffers and acceleration
// structure GpuScene uploads, at @group(0) bindings 0-5.

struct GpuMesh {
    first_index: u32,
    base_vertex: u32,
    has_normals: u32,
    vertex_count: u32,
};

struct GpuInstance {
    // Row-major 3 x 4, as Transform::matrix and Transform::inverse.
    object_to_world: array<vec4<f32>, 3>,
    world_to_object: array<vec4<f32>, 3>,
    mesh: u32,
    material: u32,
    instance: u32,
    _pad: u32,
};

@group(0) @binding(0) var scene_tlas: acceleration_structure;
@group(0) @binding(1) var<storage, read> scene_positions: array<f32>;
@group(0) @binding(2) var<storage, read> scene_normals: array<f32>;
@group(0) @binding(3) var<storage, read> scene_indices: array<u32>;
@group(0) @binding(4) var<storage, read> scene_meshes: array<GpuMesh>;
@group(0) @binding(5) var<storage, read> scene_instances: array<GpuInstance>;

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
