// The scene's glowing triangles as lights (tequila_render_core::emitters),
// at @group(0) bindings 18 and 19: the tree as the processor built it
// (emitters.rs here writes the records), choosing a triangle from a point
// down the tree by each node's importance, the one variate rescaled all the
// way down; a point on the triangle; and the density a light sample from a
// point gives a point a bounce hit, from its leaf up. A node's centre and
// radius and a leaf's corners and normal come worked out by the processor.
// Where a part's triangles start among the leaves is its instance's
// `emitter` (scene.wgsl), which the caller reads.

struct GpuEmitterNode {
    // The bounds' centre; w the radius importance takes.
    centre: vec4<f32>,
    // The triangles' power-weighted normal; w the half-angle they spread
    // over (theta_o).
    axis: vec4<f32>,
    // A leaf's triangle's corners in world space; a.w its area.
    a: vec4<f32>,
    b: vec4<f32>,
    c: vec4<f32>,
    // A leaf's triangle's normal, the side it glows from.
    normal: vec4<f32>,
    // What a leaf's part gives off, in nits.
    radiance: vec4<f32>,
    power: f32,
    // The node above, EMITTER_NONE at the root.
    parent: u32,
    // The children, EMITTER_NONE for a leaf.
    left: u32,
    right: u32,
    // A leaf's part, by its place among the card's instances, and the
    // triangle in its mesh.
    instance: u32,
    primitive: u32,
    _pad: vec2<u32>,
};

const EMITTER_NONE: u32 = 0xFFFFFFFFu;
const EM_FRAC_PI_2: f32 = 1.57079632679489661923;

@group(0) @binding(18) var<storage, read> scene_emitter_nodes: array<GpuEmitterNode>;
// Per triangle of each glowing part, from its instance's `emitter`, the
// leaf that holds it, EMITTER_NONE for one with no area.
@group(0) @binding(19) var<storage, read> scene_emitter_leaves: array<u32>;

// emitters.rs importance: the node's power over the distance squared, cut
// down by how far the point sits outside the cone its triangles glow into
// and by how obliquely the point sees them; a surface that transmits sees
// both ways.
fn emitter_importance(n: GpuEmitterNode, position: vec3<f32>, normal: vec3<f32>, two_sided: bool) -> f32 {
    if (n.power <= 0.0) {
        return 0.0;
    }
    let radius = n.centre.w;
    let offset = n.centre.xyz - position;
    let distance = sqrt(dot(offset, offset));
    var direction = normal;
    if (distance > 1e-9) {
        direction = offset * (1.0 / distance);
    }
    let distance_squared = max(distance * distance, radius * radius);
    var theta_u = EM_FRAC_PI_2;
    if (distance > radius) {
        theta_u = asin(clamp(radius / distance, -1.0, 1.0));
    }
    let cos_theta = clamp(dot(-direction, n.axis.xyz), -1.0, 1.0);
    let theta_prime = max(acos(cos_theta) - n.axis.w - theta_u, 0.0);
    if (theta_prime >= EM_FRAC_PI_2) {
        return 0.0;
    }
    var cos_incident = clamp(dot(direction, normal), -1.0, 1.0);
    if (two_sided) {
        cos_incident = abs(cos_incident);
    }
    let theta_incident = max(acos(cos_incident) - theta_u, 0.0);
    if (theta_incident >= EM_FRAC_PI_2) {
        return 0.0;
    }
    return n.power * cos(theta_prime) * cos(theta_incident) / distance_squared;
}

// Emitters::left_share: the left child's share of the importance, a coin
// flip where neither has any (or it is not finite). The two children by
// one call site, into an array (a vector's lane written by a run-time index
// in a loop has gone to the wrong lane on this driver).
fn emitters_left_share(node: GpuEmitterNode, position: vec3<f32>, normal: vec3<f32>, two_sided: bool) -> f32 {
    var importance: array<f32, 2>;
    for (var side = 0u; side < 2u; side++) {
        importance[side] = emitter_importance(scene_emitter_nodes[select(node.left, node.right, side == 1u)], position, normal, two_sided);
    }
    let l = importance[0];
    let total = l + importance[1];
    if (total > 0.0 && (bitcast<u32>(total) & 0x7f800000u) != 0x7f800000u) {
        return l / total;
    }
    return 0.5;
}

// Emitters::density: the chance a light sample from the point chooses this
// point on triangle `primitive` of a part whose triangles start at leaf
// slot `first` (its instance's `emitter`), per unit area; 0 for a part that
// is not one of these lights.
fn emitters_density(position: vec3<f32>, normal: vec3<f32>, two_sided: bool, first: u32, primitive: u32) -> f32 {
    if (first == EMITTER_NONE) {
        return 0.0;
    }
    let leaf = scene_emitter_leaves[first + primitive];
    if (leaf == EMITTER_NONE) {
        return 0.0;
    }
    var probability = 1.0;
    var node = leaf;
    // The tree is split at medians: far shallower than the bound, which
    // only keeps a broken one from looping.
    for (var depth = 0u; depth < 64u; depth++) {
        let parent_index = scene_emitter_nodes[node].parent;
        if (parent_index == EMITTER_NONE) {
            break;
        }
        let parent = scene_emitter_nodes[parent_index];
        let left = emitters_left_share(parent, position, normal, two_sided);
        probability *= select(1.0 - left, left, parent.left == node);
        node = parent_index;
    }
    return probability / scene_emitter_nodes[leaf].a.w;
}

// A point chosen on a glowing triangle (EmitterSample): where, the side it
// glows from, what it gives off, the density of choosing it per unit area.
struct EmitterDraw {
    ok: bool,
    position: vec3<f32>,
    normal: vec3<f32>,
    radiance: vec3<f32>,
    density: f32,
    // The triangle: its part among the card's instances, its index in the
    // part's mesh.
    instance: u32,
    primitive: u32,
};

// Emitters::sample: `u` chooses the triangle down the tree, `uv` the point
// on it (sample_uniform_triangle's fold).
fn emitters_sample(position: vec3<f32>, normal: vec3<f32>, two_sided: bool, u_in: f32, uv: vec2<f32>) -> EmitterDraw {
    var draw: EmitterDraw;
    draw.ok = false;
    var node = 0u;
    var probability = 1.0;
    var u = clamp(u_in, 0.0, 1.0);
    for (var depth = 0u; depth < 64u; depth++) {
        let n = scene_emitter_nodes[node];
        if (n.left == EMITTER_NONE) {
            break;
        }
        let left = emitters_left_share(n, position, normal, two_sided);
        if (u < left) {
            u = select(0.0, u / left, left > 0.0);
            probability *= left;
            node = n.left;
        } else {
            u = select(0.0, (u - left) / (1.0 - left), left < 1.0);
            probability *= 1.0 - left;
            node = n.right;
        }
        u = clamp(u, 0.0, 1.0);
    }
    if (!(probability > 0.0)) {
        return draw;
    }
    let leaf = scene_emitter_nodes[node];
    var b0 = uv.x * 0.5;
    var b1 = uv.y * 0.5;
    let offset = b1 - b0;
    if (offset > 0.0) {
        b1 += offset;
    } else {
        b0 -= offset;
    }
    draw.ok = true;
    draw.position = leaf.a.xyz * b0 + leaf.b.xyz * b1 + leaf.c.xyz * (1.0 - b0 - b1);
    draw.normal = leaf.normal.xyz;
    draw.radiance = leaf.radiance.xyz;
    draw.density = probability / leaf.a.w;
    draw.instance = leaf.instance;
    draw.primitive = leaf.primitive;
    return draw;
}
