// The scene's lamps (lights.rs's GpuLights) at @group(0) bindings 8-10:
// their light and shadow links, an IES lamp's light sample and profile, a
// lamp's bound onto a floor point (shadow_at's), and choosing one for a
// light sample as tequila_render_core::light::LightSet does: by its alias table, or down its light hierarchy by each node's
// importance from the point, the one variate rescaled all the way down;
// and the chance a point chose a lamp, which a bounce meeting the lamp is
// weighed against. The table and the hierarchy are the processor's as it
// built them (LightSet::alias_table, ::hierarchy), not built again here: a
// tie its f32 sums break one way could break the other.
//#include features
//#include lights

@group(0) @binding(8) var<storage, read> scene_lights: array<GpuLight>;
@group(0) @binding(9) var<storage, read> scene_light_nodes: array<GpuLightNode>;
// Each lamp's linked instances, sorted, from its `links`: those of its light
// links, then those of its shadow links.
@group(0) @binding(10) var<storage, read> scene_light_links: array<u32>;

// LightLinks::lights: whether a lamp lights an instance (by its index in
// the snapshot).
fn lamp_lights(l: GpuLight, instance: u32) -> bool {
    let only = (l.flags & LF_LINKS_ONLY) != 0u;
    if (!only && (l.flags & LF_LINKS_ALL_BUT) == 0u) {
        return true;
    }
    return links_hold(l.links, l.link_count, instance) == only;
}

// LightLinks::lights of a lamp's shadow links: whether an instance casts its
// shadow.
fn lamp_casts(l: GpuLight, instance: u32) -> bool {
    let only = (l.flags & LF_SHADOW_LINKS_ONLY) != 0u;
    if (!only && (l.flags & LF_SHADOW_LINKS_ALL_BUT) == 0u) {
        return true;
    }
    return links_hold(l.links + l.link_count, l.shadow_link_count, instance) == only;
}

// Whether the `count` sorted instances from `start` in the links buffer hold
// `instance`.
fn links_hold(start: u32, count: u32, instance: u32) -> bool {
    var lo = start;
    var hi = start + count;
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
    return found;
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
    // An array, not a vector: a vector's lane written by a run-time index in
    // a loop has gone to the wrong lane on this driver.
    var importance: array<f32, 2>;
    for (var side = 0u; side < 2u; side++) {
        importance[side] = light_node_importance(scene_light_nodes[select(node.left, node.right, side == 1u)], point, normal, two_sided);
    }
    let left = importance[0];
    let right = importance[1];
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

// Spectrum::luminance (ACEScg), as frame.wgsl has it.
fn lt_luminance(c: vec3<f32>) -> f32 {
    return 0.2722287 * c.x + 0.6740818 * c.y + 0.0536895 * c.z;
}

// light.rs's cone_cosine: the largest cosine against `normal` of any
// direction within the cone about `axis` whose half-angle has sine `sine`.
fn light_cone_cosine(normal: vec3<f32>, axis: vec3<f32>, sine_in: f32) -> f32 {
    if (sine_in >= 1.0) {
        return 1.0;
    }
    let sine = max(sine_in, 0.0);
    let cosine = sqrt(1.0 - sine * sine);
    let to_axis = clamp(dot(normal, axis), -1.0, 1.0);
    if (to_axis >= cosine) {
        return 1.0;
    }
    let away = sqrt(1.0 - to_axis * to_axis);
    return max(to_axis * cosine + away * sine, 0.0);
}

// Light::irradiance_bound: the most one light sample of the lamp brings onto
// a surface at `at` facing `normal` - the luminance of radiance times
// cosine over density - for shadow_at's rejection: exact for a lamp that is
// a point (its one sample), the sphere's cone for one with a radius, a sun's
// disc's, and an area lamp's from its sphere (its reach and its brightest
// radiance come worked out, in cos_a and cos_b); nothing for a lamp that is
// off or lights nothing.
fn light_irradiance_bound(l: GpuLight, at: vec3<f32>, normal: vec3<f32>) -> f32 {
    if (!light_has(l, LF_ENABLED) || (l.flags & (LF_DIFFUSE | LF_SPECULAR)) == 0u) {
        return 0.0;
    }
    if (l.kind == LIGHT_AREA) {
        let to_light = l.position.xyz - at;
        let facing = dot(to_light, l.normal.xyz);
        if (!light_has(l, LF_TWO_SIDED) && facing > 0.0) {
            return 0.0;
        }
        let distance = sqrt(dot(to_light, to_light));
        if (distance <= 0.0) {
            return 0.0;
        }
        let reach = l.cos_a;
        let cosine = light_cone_cosine(normal, to_light / distance, reach / distance);
        if (light_has(l, LF_DISC)) {
            let height = abs(facing);
            let nearest = max(distance - reach, height);
            if (nearest <= 0.0) {
                return 0.0;
            }
            return l.cos_b * cosine * l.normal.w * height / (nearest * nearest * nearest);
        }
        let sine = min(reach / distance, 1.0);
        let cone = LT_TAU * (1.0 - sqrt(1.0 - sine * sine));
        return l.cos_b * cosine * cone;
    }
    if (l.kind == LIGHT_DISTANT) {
        return max(lt_luminance(l.radiance.xyz), 0.0) * light_cone_cosine(normal, -l.normal.xyz, l.cos_b);
    }
    let offset = l.position.xyz - at;
    if (l.kind == LIGHT_POINT && l.position.w > 0.0) {
        let distance = sqrt(dot(offset, offset));
        if (distance <= 0.0) {
            return 0.0;
        }
        let nearest = max(distance - l.position.w, distance * 1e-3);
        return max(lt_luminance(l.radiance.xyz), 0.0) * light_cone_cosine(normal, offset / distance, l.position.w / distance) / (nearest * nearest);
    }
    // A point, a spot, an IES lamp: its one sample.
    let distance_squared = dot(offset, offset);
    if (distance_squared < 1e-12) {
        return 0.0;
    }
    let direction = offset * (1.0 / sqrt(distance_squared));
    var radiance = l.radiance.xyz * (1.0 / distance_squared);
    if (l.kind == LIGHT_SPOT) {
        let falloff = spot_falloff(l, dot(-direction, l.normal.xyz));
        if (falloff <= 0.0) {
            return 0.0;
        }
        radiance = l.radiance.xyz * falloff * (1.0 / distance_squared);
    } else if (l.kind == LIGHT_IES) {
        if (!FEATURE_IES) {
            return 0.0;
        }
        let candela = ies_candela(l, -direction);
        if (candela <= 0.0) {
            return 0.0;
        }
        radiance = l.radiance.xyz * (candela / distance_squared);
    }
    return max(lt_luminance(radiance), 0.0) * max(dot(direction, normal), 0.0);
}

// ---- An IES lamp's profile (ies.rs) -----------------------------------------
// In the links buffer at its lamp's `links` past its link lists: the
// vertical angles' count, the horizontal angles', the horizontal span the
// fold reflects about (IesProfile::candela_at's), then the vertical angles,
// the horizontal angles and the candela values (horizontal-major), as f32
// bits. The lamp's `half_u` is its `right`, `half_v` its `up`
// (direction × right), `normal` its aim.

fn ies_value(at: u32) -> f32 {
    return bitcast<f32>(scene_light_links[at]);
}

struct IesBracket {
    lower: u32,
    upper: u32,
    fraction: f32,
};

// ies.rs's bracket: where `value` falls in the `count` ascending values from
// `start`, clamped at both ends.
fn ies_bracket(start: u32, count: u32, value: f32) -> IesBracket {
    if (count < 2u || value <= ies_value(start)) {
        return IesBracket(0u, 0u, 0.0);
    }
    let last = count - 1u;
    if (value >= ies_value(start + last)) {
        return IesBracket(last, last, 0.0);
    }
    // partition_point(|entry| entry <= value).
    var lo = 0u;
    var hi = count;
    while (lo < hi) {
        let mid = (lo + hi) / 2u;
        if (ies_value(start + mid) <= value) {
            lo = mid + 1u;
        } else {
            hi = mid;
        }
    }
    let lower = lo - 1u;
    let low = ies_value(start + lower);
    let span = max(ies_value(start + lo) - low, 1e-9);
    return IesBracket(lower, lo, (value - low) / span);
}

// f32::rem_euclid by a positive `period`, for the angles the fold takes:
// from -0 to two periods, where Rust's % is one subtraction or two, exact.
fn ies_rem_euclid(x: f32, period: f32) -> f32 {
    var r = x;
    if (r >= period) {
        r = r - period;
    }
    if (r >= period) {
        r = r - period;
    }
    if (r < 0.0) {
        r = r + period;
    }
    return r;
}

// ies.rs's fold: a horizontal angle into the measured span, a partial sweep
// reflected about it.
fn ies_fold(angle: f32, span: f32) -> f32 {
    var a = ies_rem_euclid(angle, 360.0);
    if (span <= 0.0) {
        return 0.0;
    }
    if (span < 359.0 && a > span) {
        let period = select(360.0, 180.0, span <= 90.5);
        a = ies_rem_euclid(a, period);
        if (a > span) {
            a = abs(period - a);
        }
        if (a > span) {
            a = span;
        }
    }
    return a;
}

// IesLight::candela: the candela towards world direction `world` leaving the
// lamp - into the luminaire's frame (right, up, -aim), then
// IesProfile::candela_towards and candela_at, bilinear in the angles.
fn ies_candela(l: GpuLight, world: vec3<f32>) -> f32 {
    let local = vec3<f32>(dot(world, l.half_u.xyz), dot(world, l.half_v.xyz), -dot(world, l.normal.xyz));
    var d = vec3<f32>(0.0, 0.0, -1.0);
    let len2 = dot(local, local);
    if (len2 > 0.0 && lt_finite(len2)) {
        d = local * (1.0 / sqrt(len2));
    }
    let vertical = acos(clamp(-d.z, -1.0, 1.0)) * 57.2957795130823208767981548141051703;
    var horizontal = atan2(d.y, d.x) * 57.2957795130823208767981548141051703;
    if (horizontal < 0.0) {
        horizontal = horizontal + 360.0;
    }
    let at = l.links + l.link_count + l.shadow_link_count;
    let nv = scene_light_links[at];
    let nh = scene_light_links[at + 1u];
    let span = ies_value(at + 2u);
    let v_start = at + 3u;
    let h_start = v_start + nv;
    let c_start = h_start + nh;
    let v = ies_bracket(v_start, nv, vertical);
    let h = ies_bracket(h_start, nh, ies_fold(horizontal, span));
    let a = ies_value(c_start + h.lower * nv + v.lower) * (1.0 - v.fraction) + ies_value(c_start + h.lower * nv + v.upper) * v.fraction;
    let b = ies_value(c_start + h.upper * nv + v.lower) * (1.0 - v.fraction) + ies_value(c_start + h.upper * nv + v.upper) * v.fraction;
    return a * (1.0 - h.fraction) + b * h.fraction;
}

// Light::sample for any lamp: an IES lamp's here, as it reads its profile
// (IesLight::sample: a point, its intensity the profile's towards the way
// the light leaves it), the others' lights.wgsl's.
fn lamp_sample(l: GpuLight, at: vec3<f32>, u: vec2<f32>) -> LightDraw {
    if (FEATURE_IES && l.kind == LIGHT_IES) {
        if (!light_has(l, LF_ENABLED)) {
            return no_light_draw();
        }
        let offset = l.position.xyz - at;
        let distance_squared = dot(offset, offset);
        if (distance_squared < 1e-12) {
            return no_light_draw();
        }
        let distance = sqrt(distance_squared);
        let direction = offset * (1.0 / distance);
        let candela = ies_candela(l, -direction);
        if (candela <= 0.0) {
            return no_light_draw();
        }
        return LightDraw(true, direction, l.radiance.xyz * (candela / distance_squared), distance, false, 1.0, true);
    }
    return light_sample(l, at, u);
}
