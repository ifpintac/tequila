// The scene's lamps (lights.rs's GpuLights) at @group(0) bindings 8-10, and
// choosing one for a light sample as tequila_render_core::light::LightSet
// does: by its alias table, or down its light hierarchy by each node's
// importance from the point, the one variate rescaled all the way down;
// and the chance a point chose a lamp, which a bounce meeting the lamp is
// weighed against. The table and the hierarchy are the processor's as it
// built them (LightSet::alias_table, ::hierarchy), not built again here: a
// tie its f32 sums break one way could break the other.
//#include lights

@group(0) @binding(8) var<storage, read> scene_lights: array<GpuLight>;
@group(0) @binding(9) var<storage, read> scene_light_nodes: array<GpuLightNode>;
// Each lamp's linked instances, sorted, from its `links`.
@group(0) @binding(10) var<storage, read> scene_light_links: array<u32>;

// LightLinks::lights: whether a lamp lights an instance (by its index in
// the snapshot).
fn lamp_lights(l: GpuLight, instance: u32) -> bool {
    let only = (l.flags & LF_LINKS_ONLY) != 0u;
    if (!only && (l.flags & LF_LINKS_ALL_BUT) == 0u) {
        return true;
    }
    var lo = l.links;
    var hi = l.links + l.link_count;
    var found = false;
    while (lo < hi) {
        let mid = (lo + hi) / 2u;
        let at = scene_light_links[mid];
        if (at == instance) {
            found = true;
            break;
        }
        if (at < instance) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    return found == only;
}

// integrator.rs's met_by_bounces: a lamp a ray can meet, that shows in
// reflections and lights the instance the bounce left.
fn lamp_met_by_bounces(l: GpuLight, instance: u32) -> bool {
    return (l.flags & (LF_CAN_BE_MET | LF_SPECULAR)) == (LF_CAN_BE_MET | LF_SPECULAR) && lamp_lights(l, instance);
}

struct LampChoice {
    ok: bool,
    index: u32,
    probability: f32,
};

// LightSet::select: the alias table, `u` unclamped for the fraction.
fn lamps_select(count: u32, u: f32) -> LampChoice {
    var choice: LampChoice;
    choice.ok = false;
    if (count == 0u) {
        return choice;
    }
    let n = f32(count);
    let index = min(u32(clamp(u, 0.0, 1.0) * n), count - 1u);
    let entry = scene_lights[index];
    let fractional = u * n - f32(index);
    var chosen = entry.alias_alt;
    if (fractional < entry.alias_p) {
        chosen = index;
    }
    let p = scene_lights[chosen].probability;
    if (p <= 0.0) {
        return choice;
    }
    return LampChoice(true, chosen, p);
}

// LightSet::left_share: the left child's share of the importance, a coin
// flip where neither has any (or it is not finite).
fn lamps_left_share(node: GpuLightNode, point: vec3<f32>, normal: vec3<f32>, two_sided: bool) -> f32 {
    // The two children by one call site, for the code's size.
    var importance = vec2<f32>(0.0);
    for (var side = 0u; side < 2u; side++) {
        importance[side] = light_node_importance(scene_light_nodes[select(node.left, node.right, side == 1u)], point, normal, two_sided);
    }
    let left = importance.x;
    let right = importance.y;
    let total = left + right;
    if (total > 0.0 && lt_finite(total)) {
        return left / total;
    }
    return 0.5;
}

// LightSet::select_at: with `hierarchy`, down the hierarchy from the root;
// else the alias table.
fn lamps_select_at(count: u32, hierarchy: bool, point: vec3<f32>, normal: vec3<f32>, two_sided: bool, u_in: f32) -> LampChoice {
    if (!hierarchy) {
        return lamps_select(count, u_in);
    }
    var choice: LampChoice;
    choice.ok = false;
    var index = 0u;
    var probability = 1.0;
    var u = clamp(u_in, 0.0, 1.0);
    // Every descent ends at a leaf; the bound only keeps a broken table
    // from looping.
    for (var depth = 0u; depth < 64u; depth++) {
        let node = scene_light_nodes[index];
        if (node.left == LIGHT_LEAF) {
            if (probability <= 0.0) {
                return choice;
            }
            return LampChoice(true, node.light, probability);
        }
        let p_left = lamps_left_share(node, point, normal, two_sided);
        if (u < p_left) {
            u = select(0.0, u / p_left, p_left > 0.0);
            probability *= p_left;
            index = node.left;
        } else {
            let p_right = 1.0 - p_left;
            u = select(0.0, (u - p_left) / p_right, p_right > 0.0);
            probability *= p_right;
            index = node.right;
        }
        u = clamp(u, 0.0, 1.0);
        if (probability <= 0.0) {
            return choice;
        }
    }
    return choice;
}

// LightSet::selection_pdf_at: the chance lamp `id` is chosen from `point`,
// from its leaf up; by power alone under the alias table.
fn lamps_selection_pdf_at(hierarchy: bool, point: vec3<f32>, normal: vec3<f32>, two_sided: bool, id: u32) -> f32 {
    let lamp = scene_lights[id];
    if (!hierarchy) {
        return lamp.probability;
    }
    var probability = 1.0;
    var node = lamp.leaf;
    for (var depth = 0u; depth < 64u; depth++) {
        let parent_index = scene_light_nodes[node].parent;
        if (parent_index == LIGHT_LEAF) {
            break;
        }
        let parent = scene_light_nodes[parent_index];
        let left = lamps_left_share(parent, point, normal, two_sided);
        probability *= select(1.0 - left, left, parent.left == node);
        node = parent_index;
    }
    return probability;
}
