// An orthonormal frame about a unit vector: tequila_core::math::Onb, its
// from_normal [Duff et al. 2017], to_local and to_world, bit for bit.

struct Frame {
    t: vec3<f32>,
    b: vec3<f32>,
    n: vec3<f32>,
};

fn frame_of(n: vec3<f32>) -> Frame {
    // copysign(1, n.z), -0.0 counting as negative.
    let sign = select(1.0, -1.0, (bitcast<u32>(n.z) & 0x80000000u) != 0u);
    let a = -1.0 / (sign + n.z);
    let b = n.x * n.y * a;
    return Frame(
        vec3<f32>(1.0 + sign * n.x * n.x * a, sign * b, -sign * n.x),
        vec3<f32>(b, sign + n.y * n.y * a, -n.y),
        n,
    );
}

fn to_local(f: Frame, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(v, f.t), dot(v, f.b), dot(v, f.n));
}

fn to_world(f: Frame, v: vec3<f32>) -> vec3<f32> {
    return f.t * v.x + f.b * v.y + f.n * v.z;
}
