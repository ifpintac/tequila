// The transport (docs/07 R2.5, R3): the processor's path for a scene of
// opaque OpenPBR surfaces (openpbr.wgsl) under the environment and the
// analytic lamps (lamps.wgsl), standing on a shadow catcher, draw for draw -
// camera ray, next-event estimation of the sky by its facing tables with
// power-heuristic MIS and of one lamp chosen as the processor chooses it,
// the stack's bounce, Russian roulette, the bounce limit, the clamps; the
// lamps a bounce off a sharp enough reflection meets along its way, shared
// with the light sample there, and the stand-ins the camera sees them by;
// and the floor: its shadow, its reflection (a path of its own, run in the
// same loop, as WGSL has no recursion), a floor a bounce meets passed,
// glowing or shaded. integrator.rs is the specification; the line
// references in the comments are to it. One invocation is one pixel,
// drawing frame.samples samples in order.
//#include sampler
//#include environment
//#include openpbr
//#include lamps

@group(0) @binding(6) var<storage, read> scene_materials: array<GpuMaterial>;
// The shadow catchers (scene.rs's GpuCatcher), by an instance's `catcher`.
@group(0) @binding(7) var<storage, read> scene_catchers: array<GpuCatcher>;

struct Surface {
    position: vec3<f32>,
    magnitude: f32,
    geometric: vec3<f32>,
    shading: vec3<f32>,
};

// The hit as Embree's resolve builds it (embree.rs): the point from the
// triangle's corners in the mesh, through the part's place.
fn surface_of(hit: RayIntersection) -> Surface {
    let inst = scene_instances[hit.instance_custom_data];
    let mesh = scene_meshes[inst.mesh];
    let first = mesh.first_index + 3u * hit.primitive_index;
    let i0 = mesh.base_vertex + scene_indices[first];
    let i1 = mesh.base_vertex + scene_indices[first + 1u];
    let i2 = mesh.base_vertex + scene_indices[first + 2u];
    let c0 = scene_position(i0);
    let c1 = scene_position(i1);
    let c2 = scene_position(i2);
    let u = hit.barycentrics.x;
    let v = hit.barycentrics.y;
    let w = 1.0 - u - v;
    var s: Surface;
    s.position = rows_point(inst.object_to_world, c0 * w + c1 * u + c2 * v);
    let largest = max(max(max(abs(c0.x), abs(c0.y)), max(abs(c0.z), abs(c1.x))), max(max(abs(c1.y), abs(c1.z)), max(abs(c2.x), max(abs(c2.y), abs(c2.z)))));
    let m = inst.object_to_world;
    var magnitude = 0.0;
    for (var r = 0u; r < 3u; r++) {
        magnitude = max(magnitude, (abs(m[r].x) + abs(m[r].y) + abs(m[r].z)) * largest + abs(m[r].w));
    }
    s.magnitude = magnitude;
    let g = rows_normal(inst.world_to_object, cross(c1 - c0, c2 - c0));
    s.geometric = normalized(g);
    if (mesh.has_normals != 0u) {
        let n = rows_normal(inst.world_to_object, scene_normal(i0) * w + scene_normal(i1) * u + scene_normal(i2) * v);
        let len2 = dot(n, n);
        s.shading = select(s.geometric, n * (1.0 / sqrt(len2)), len2 > 0.0);
    } else {
        s.shading = s.geometric;
    }
    return s;
}

// offset_ray_origin_at.
fn offset_at(p: vec3<f32>, n: vec3<f32>, magnitude: f32) -> vec3<f32> {
    let m = max(max(max(abs(p.x), abs(p.y)), max(abs(p.z), magnitude)), 0.125);
    return p + n * (m * (1.0 / 262144.0));
}

// integrator.rs valid_for_reflection.
fn valid_for_reflection(geometric: vec3<f32>, wo: vec3<f32>, shading: vec3<f32>) -> vec3<f32> {
    let iz = dot(wo, geometric);
    if (iz <= 0.0) {
        return shading;
    }
    let mirrored = shading * (2.0 * dot(shading, wo)) - wo;
    let threshold = min(0.9 * iz, 0.01);
    if (dot(mirrored, geometric) >= threshold) {
        return shading;
    }
    let along = shading - geometric * dot(shading, geometric);
    let al2 = dot(along, along);
    let x = select(shading, along * (1.0 / sqrt(al2)), al2 > 0.0);
    let ix = dot(wo, x);
    let a = ix * ix + iz * iz;
    let b = 2.0 * (a + iz * threshold);
    let c = (threshold + iz) * (threshold + iz);
    let root = sqrt(max(b * b - 4.0 * a * c, 0.0));
    let nz2 = select(0.25 * (b - root) / a, 0.25 * (b + root) / a, ix < 0.0);
    let nx = sqrt(max(1.0 - nz2, 0.0));
    let nz = sqrt(max(nz2, 0.0));
    let r = x * nx + geometric * nz;
    let r2 = dot(r, r);
    return select(geometric, r * (1.0 / sqrt(r2)), r2 > 0.0);
}

// sample_uniform_disk_concentric, then the hemisphere above it.
fn sample_cosine_hemisphere(u: vec2<f32>) -> vec3<f32> {
    let o = vec2<f32>(2.0 * u.x - 1.0, 2.0 * u.y - 1.0);
    var d = vec2<f32>(0.0, 0.0);
    if (!(o.x == 0.0 && o.y == 0.0)) {
        var r: f32;
        var theta: f32;
        if (abs(o.x) > abs(o.y)) {
            r = o.x;
            theta = (PI / 4.0) * (o.y / o.x);
        } else {
            r = o.y;
            theta = (PI / 2.0) - (PI / 4.0) * (o.x / o.y);
        }
        d = vec2<f32>(r * cos(theta), r * sin(theta));
    }
    let z = sqrt(max(1.0 - d.x * d.x - d.y * d.y, 0.0));
    return vec3<f32>(d.x, d.y, z);
}

fn mis_power(a: f32, b: f32) -> f32 {
    if (a > 3.4e38) {
        return 1.0;
    }
    let a2 = a * a;
    let denom = a2 + b * b;
    if (denom <= 0.0) {
        return 0.0;
    }
    return a2 / denom;
}

// clamp_light: a contribution's summed red, green and blue brought down to
// the ceiling; a ceiling at or below zero is none.
fn clamp_light(c: vec3<f32>, ceiling: f32) -> vec3<f32> {
    if (!(ceiling > 0.0)) {
        return c;
    }
    let sum = abs(c.x) + abs(c.y) + abs(c.z);
    if (sum > ceiling) {
        return c * (ceiling / sum);
    }
    return c;
}

// light_ceiling.
fn light_ceiling(specular_chain: bool, found_at: u32) -> f32 {
    if (found_at == 0u || specular_chain) {
        return frame.clamp_direct;
    }
    return frame.clamp_indirect;
}

fn trace(origin: vec3<f32>, direction: vec3<f32>) -> RayIntersection {
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, RayDesc(RAY_FLAG_FORCE_OPAQUE, MASK_SURFACE, 0.0, 3.0e38, origin, direction));
    rayQueryProceed(&rq);
    return rayQueryGetCommittedIntersection(&rq);
}

// Intersector::occluded through shadow_ray: both ends offset, stopped just
// short of the aim, passing what casts no shadow (a lamp's stand-in) as
// Embree's MASK_SHADOW does.
fn occluded(from_point: vec3<f32>, from_normal: vec3<f32>, to: vec3<f32>, to_normal: vec3<f32>) -> bool {
    let origin = offset_at(from_point, from_normal, 0.0);
    let aim = offset_at(to, to_normal, 0.0);
    let delta = aim - origin;
    let distance = sqrt(dot(delta, delta));
    let direction = select(from_normal, delta / distance, distance > 0.0);
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, RayDesc(RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT, MASK_SHADOW, 0.0, distance * (1.0 - 1e-4), origin, direction));
    rayQueryProceed(&rq);
    return rayQueryGetCommittedIntersection(&rq).kind != RAY_QUERY_INTERSECTION_NONE;
}

// A stack that is mirrors only - every lobe it holds a smooth reflection -
// which next-event estimation skips (BsdfFlags DELTA without GLOSSY or
// DIFFUSE).
fn pure_delta(b: OpenPbr) -> bool {
    if (b.count == 0u) {
        return false;
    }
    for (var slot = 0u; slot < OPENPBR_LOBES; slot++) {
        if (!openpbr_has(b, slot)) {
            continue;
        }
        switch slot {
            case 1u: { if (!ggx_is_smooth(b.coat.distribution)) { return false; } }
            case 2u: { if (!ggx_is_smooth(b.metal.distribution)) { return false; } }
            case 3u: { if (!ggx_is_smooth(b.specular.distribution)) { return false; } }
            default: { return false; }
        }
    }
    return true;
}

struct PathOut {
    radiance: vec3<f32>,
    background: vec3<f32>,
    coverage: f32,
};

fn is_finite(x: f32) -> bool {
    return (bitcast<u32>(x) & 0x7f800000u) != 0x7f800000u;
}

fn sanitized(c: vec3<f32>) -> vec3<f32> {
    var out = c;
    for (var i = 0u; i < 3u; i++) {
        let v = c[i];
        // Not finite, or not above zero: nothing.
        out[i] = select(0.0, v, v > 0.0 && v < 3.4e38);
    }
    return out;
}

// ic:215: a bound on the crossings of a path, rather than a budget spent
// from its bounces.
const MAX_CUTOUT_CROSSINGS: u32 = 64u;

// The surface a seeing bounce left, or a mirror's view (ic:2654's
// SeenFrom, for a path of opaque surfaces: no glass to see or pass, and a
// surface lit from one side): where it was, its geometric normal on the
// side the bounce left by, the way it left, its instance (by its index in
// the snapshot), and the density the lobes that see gave that way -
// infinite off a mirror-smooth lobe (`delta`), which no light sample stands
// in for - and the share of the value its glossy lobes gave.
struct SeenFrom {
    valid: bool,
    delta: bool,
    position: vec3<f32>,
    normal: vec3<f32>,
    direction: vec3<f32>,
    density: f32,
    glossy: vec3<f32>,
    instance: u32,
};

// What a path carries from one segment to the next: the ray, what it has
// gathered and what it carries, how the last bounce left (its density, or
// a mirror's, whose density is infinite and an escape after which is
// weighed 1), the specular chain, whether a shadow catcher met next is seen
// as the camera would see it (ic:859), the bounces and the floor crossings,
// the least density a bounce left by (ic:879, for Filter Glossy) and the
// surface a bounce that meets the lamps left (ic:907).
struct PathState {
    origin: vec3<f32>,
    direction: vec3<f32>,
    radiance: vec3<f32>,
    throughput: vec3<f32>,
    // Which way the surface the path last drew the sky from faced.
    sky_facing: vec3<f32>,
    // Read only once a bounce has set it: the camera ray is weighed 1.
    prev_pdf: f32,
    prev_delta: bool,
    specular_chain: bool,
    catcher_view: bool,
    bounce: u32,
    crossings: u32,
    max_bounces: u32,
    // Read only below 1 (filtered_roughness): any value from 1 up is the
    // processor's infinity.
    least_pdf: f32,
    seen: SeenFrom,
};

fn path_start(origin: vec3<f32>, direction: vec3<f32>, max_bounces: u32, catcher_view: bool) -> PathState {
    var seen: SeenFrom;
    seen.valid = false;
    return PathState(origin, direction, vec3<f32>(0.0), vec3<f32>(1.0), vec3<f32>(0.0, 1.0, 0.0), 0.0, false, true, catcher_view, 0u, 0u, max_bounces, 3.0e38, seen);
}

// ic:1259-1324: the sky, weighed against having been sampled. What it adds
// is the background when the camera ray escapes.
fn path_escape(st: ptr<function, PathState>) -> vec3<f32> {
    let direction = (*st).direction;
    let weight = select(mis_power((*st).prev_pdf, env_pdf_facing(direction, (*st).sky_facing)), 1.0, (*st).specular_chain || (*st).prev_delta);
    let sky = env_radiance((*st).bounce == 0u, direction);
    let found_at = select((*st).bounce - 1u, 0u, (*st).bounce == 0u);
    let contribution = clamp_light((*st).throughput * sky * weight, light_ceiling((*st).specular_chain, found_at));
    (*st).radiance += contribution;
    return contribution;
}

// facing_forward: both normals to the side the ray came from; the shading
// normal kept on the geometric one's side, and turned so the surface's
// mirror reflection stays above it (ic:1600-1621).
fn facing(raw: Surface, direction: vec3<f32>) -> Surface {
    var surf = raw;
    if (dot(direction, surf.geometric) > 0.0) {
        surf.geometric = -surf.geometric;
        surf.shading = -surf.shading;
    }
    if (dot(surf.shading, surf.geometric) <= 0.0) {
        surf.shading = surf.geometric;
    }
    surf.shading = valid_for_reflection(surf.geometric, -direction, surf.shading);
    return surf;
}

// catcher_view_through: the chance a path reflected off `m` still sees a
// shadow catcher as the camera would - always off a mirror, fading out by a
// roughness of 0.15. The material's own roughness, not the floored one.
const CATCHER_SMOOTH: f32 = 0.0316;
const CATCHER_BLURRED: f32 = 0.15;

fn catcher_view_through(m: GpuMaterial) -> f32 {
    var roughness = m.specular_roughness;
    if (m.coat_weight > 0.0) {
        roughness = min(roughness, m.coat_roughness);
    }
    let t = clamp((roughness - CATCHER_SMOOTH) / (CATCHER_BLURRED - CATCHER_SMOOTH), 0.0, 1.0);
    return 1.0 - t * t * (3.0 - 2.0 * t);
}

// integrator.rs's seeing_share: up to this roughness a reflection's bounce
// sees - meets the lamps it is turned to - and from this one on it does not.
const SEEN_UP_TO: f32 = 0.5;
const PASSED_FROM: f32 = 0.8;

// filtered_roughness (ic:2643): the roughness a mirror-smooth surface is
// blurred to past a bounce that left by a density of `least_pdf` at most,
// as Cycles' Filter Glossy has it.
fn filtered_roughness(least_pdf: f32) -> f32 {
    if (!(least_pdf < 1.0)) {
        return 0.0;
    }
    return sqrt(0.5 * sqrt(1.0 - max(least_pdf, 0.0)));
}

// left_to_bounces (ic:4435): the share of a light sample's light the lobes
// that do not see make up, and what the ones that see give it.
fn left_to_bounces(pdf: f32, seen_in: f32) -> vec2<f32> {
    if (!(pdf > 0.0)) {
        return vec2<f32>(1.0, 0.0);
    }
    let seen = clamp(seen_in, 0.0, pdf);
    return vec2<f32>((pdf - seen) / pdf, seen);
}

// Next-event estimation at a vertex (ic:2043-2201): one sample of the sky
// (sample_environment_light, ic:4279) and one of the lamps
// (sample_analytic_lights, ic:4550), each with its own shadow ray, summed
// before the throughput, as the processor sums them. One pass of the loop
// is one light source, so the two share their evaluation of the stack,
// their shadow ray and their arithmetic. An environment turned down to
// nothing, and a scene with no lamp, draw nothing (ic:4299, 4565).
fn direct_light(s: ptr<function, Sampler>, bsdf: OpenPbr, f: Frame, wo: vec3<f32>, surf: Surface, instance: u32) -> vec3<f32> {
    var direct = vec3<f32>(0.0);
    for (var source = 0u; source < 2u; source++) {
        let sky = source == 0u;
        var draw: LightDraw;
        var diffuse = true;
        var specular = true;
        var shadow = true;
        var p_select = 1.0;
        var is_shared = false;
        if (sky) {
            if (frame.env_intensity <= 0.0) {
                continue;
            }
            let e = env_sample_facing(surf.geometric, sampler_light_sample(s));
            if (!e.ok || e.pdf <= 0.0 || all(e.radiance == vec3<f32>(0.0))) {
                continue;
            }
            draw = LightDraw(true, e.direction, e.radiance, 0.0, true, e.pdf, false);
        } else {
            if (frame.light_count == 0u) {
                continue;
            }
            // The lamp chosen from this point, facing the side the path is on
            // (an opaque surface is lit from one side), and its own variates.
            let choice = lamps_select_at(frame.light_count, frame.light_hierarchy != 0u, surf.position, surf.geometric, false, sampler_light_select(s));
            if (!choice.ok) {
                continue;
            }
            let light = scene_lights[choice.index];
            // Light linking: a lamp kept to other parts lights nothing here.
            if (!lamp_lights(light, instance)) {
                continue;
            }
            draw = light_sample(light, surf.position, sampler_light_sample(s));
            if (!draw.ok || all(draw.radiance == vec3<f32>(0.0)) || (!draw.is_delta && draw.pdf <= 0.0)) {
                continue;
            }
            diffuse = light_has(light, LF_DIFFUSE);
            specular = light_has(light, LF_SPECULAR);
            shadow = light_has(light, LF_SHADOW);
            p_select = choice.probability;
            // Shared with the bounces that meet it (ic:4686).
            is_shared = (frame.lamp_flags & LAMPS_SEEN) != 0u && frame.shadows != 0u && !draw.is_delta && lamp_met_by_bounces(light, instance);
        }
        // Not lit from behind: an opaque surface only reflects.
        if (dot(draw.direction, surf.geometric) <= 0.0) {
            continue;
        }
        let wi = to_local(f, draw.direction);
        if (wi.z <= 0.0) {
            continue;
        }
        let value = openpbr_eval_selective(bsdf, wo, wi, diffuse, specular);
        if (all(value == vec3<f32>(0.0))) {
            continue;
        }
        // The stack's density, and its seeing lobes', in one pass.
        let densities = openpbr_densities(bsdf, wo, wi, SEEN_UP_TO, PASSED_FROM);
        let bsdf_pdf = select(densities.x, 0.0, draw.is_delta);
        if (frame.shadows != 0u && shadow) {
            // Just short of a lamp; past everything for the sky or a sun.
            var reach = frame.shadow_far;
            if (!draw.infinite) {
                reach = draw.distance * (1.0 - 1e-3);
            }
            let from_point = offset_at(surf.position, surf.geometric, surf.magnitude);
            if (occluded(from_point, surf.geometric, surf.position + draw.direction * reach, -draw.direction)) {
                continue;
            }
        }
        var scale: vec3<f32>;
        if (sky) {
            scale = draw.radiance * ((abs(wi.z) * mis_power(draw.pdf, bsdf_pdf)) / draw.pdf);
        } else {
            // No MIS against the bounces that cannot meet a lamp; the share
            // of one they can meet that this sample keeps.
            var share = 1.0;
            if (is_shared) {
                let left = left_to_bounces(bsdf_pdf, densities.y);
                share = left.x + (1.0 - left.x) * mis_power(draw.pdf * p_select, left.y);
            }
            let density = select(draw.pdf * p_select, p_select, draw.is_delta);
            scale = draw.radiance * ((abs(wi.z) * share) / density);
        }
        direct += value * scale;
    }
    return direct;
}

// One vertex of a path off an opaque OpenPBR surface: the bounce limit,
// Filter Glossy, next-event estimation of the sky and the lamps and the
// bounce, whether it sees, the catcher's view and Russian roulette
// (ic:1862-2562). False where the path ends here.
fn shade_vertex(st: ptr<function, PathState>, s: ptr<function, Sampler>, hit: RayIntersection, surf: Surface) -> bool {
    let bounce = (*st).bounce;
    if (bounce >= (*st).max_bounces) {
        return false;
    }
    let f = frame_of(surf.shading);
    let wo = to_local(f, -(*st).direction);
    if (wo.z <= 0.0) {
        return false;
    }
    // The stack as the integrator builds it (ic:2003-2032): every
    // reflection lobe at least as rough as the regularization's floor from
    // its bounce on, the lobe chances aimed at `wo`; and where lamps show in
    // mirrors, one with a mirror-smooth lobe blurred past a diffuse or rough
    // bounce as Cycles' Filter Glossy blurs it - built again, all its
    // reflections at least that rough (one call site, for the code's size).
    let inst = scene_instances[hit.instance_custom_data];
    let material = scene_materials[inst.material];
    let floor = select(0.0, frame.regularization_roughness, bounce >= frame.regularization_start);
    var blur = 0.0;
    if ((frame.lamp_flags & LAMPS_IN_MIRRORS) != 0u && frame.regularization_roughness > 0.0) {
        blur = filtered_roughness((*st).least_pdf);
    }
    var roughness_floor = floor;
    var bsdf: OpenPbr;
    loop {
        bsdf = openpbr_aim(openpbr_to_aim(material, roughness_floor), wo);
        if (roughness_floor == floor && blur > floor && openpbr_has_delta(bsdf)) {
            roughness_floor = blur;
            continue;
        }
        break;
    }
    sampler_begin_bounce(s, bounce);

    // Next-event estimation (ic:2043-2224), but for a stack of mirrors
    // only; clamped as one.
    let samples_lights = !pure_delta(bsdf);
    if (samples_lights) {
        (*st).sky_facing = surf.geometric;
        let direct = direct_light(s, bsdf, f, wo, surf, inst.instance);
        (*st).radiance += clamp_light((*st).throughput * direct, light_ceiling((*st).specular_chain, bounce));
    }

    // The bounce (ic:2316-2433).
    let chosen = openpbr_sample_seen(bsdf, wo, sampler_bsdf_direction(s), sampler_bsdf_lobe(s), SEEN_UP_TO, PASSED_FROM);
    let sample = chosen.sample;
    if (!sample.ok || !(sample.pdf > 0.0) || all(sample.f == vec3<f32>(0.0))) {
        return false;
    }
    let wi_world = to_world(f, sample.wi);
    if (dot(wi_world, surf.geometric) <= 0.0) {
        return false;
    }
    let throughput = (*st).throughput * (sample.f * (abs(sample.wi.z) / sample.pdf));
    if (!(is_finite(throughput.x) && is_finite(throughput.y) && is_finite(throughput.z)) || all(throughput == vec3<f32>(0.0))) {
        return false;
    }
    (*st).throughput = throughput;
    // Whether this bounce sees (ic:2413): off a reflection sharp enough to
    // show what it is turned to, it meets the lamps there - always off a
    // mirror-smooth one, else by a draw of the open stream against how
    // sharp it is.
    var sees = false;
    if (samples_lights && frame.shadows != 0u && (frame.lamp_flags & LAMPS_SEEN) != 0u && chosen.roughness >= 0.0) {
        if (sample.is_delta) {
            sees = true;
        } else {
            let share = openpbr_seeing_share(chosen.roughness, SEEN_UP_TO, PASSED_FROM);
            sees = share >= 1.0 || (share > 0.0 && sampler_next_unstructured(s) < share);
        }
    }
    if (!sample.is_delta) {
        (*st).least_pdf = min((*st).least_pdf, sample.pdf);
    }
    (*st).prev_pdf = sample.pdf;
    (*st).prev_delta = sample.is_delta;
    // What meets the lamps along the way on (ic:2442-2498): a bounce that
    // sees, or a mirror's view, which no light sample stands in for.
    if (sees || (sample.is_delta && (frame.lamp_flags & LAMPS_IN_MIRRORS) != 0u)) {
        var glossy = vec3<f32>(1.0);
        var density = 0.0;
        if (!sample.is_delta) {
            density = chosen.seen;
            if ((frame.lamp_flags & LAMPS_FOR_REFLECTIONS) != 0u) {
                for (var c = 0u; c < 3u; c++) {
                    glossy[c] = select(0.0, clamp(chosen.glossy[c] / sample.f[c], 0.0, 1.0), sample.f[c] > 0.0);
                }
            }
        }
        (*st).seen = SeenFrom(true, sample.is_delta, surf.position, surf.geometric, wi_world, density, glossy, inst.instance);
    } else {
        (*st).seen.valid = false;
    }
    (*st).specular_chain = (*st).specular_chain && sample.is_delta;
    // ic:2510-2511: through a mirror the catcher is still seen as the camera
    // sees it; off a reflection that is not one, by a draw of the open
    // stream against how near a mirror it is.
    if ((*st).catcher_view && !sample.is_delta) {
        (*st).catcher_view = sampler_next_unstructured(s) < catcher_view_through(material);
    }
    (*st).origin = offset_at(surf.position, surf.geometric, surf.magnitude);
    (*st).direction = wi_world;
    (*st).bounce = bounce + 1u;
    // Russian roulette, reading the vertex just left (ic:2556-2562).
    if ((*st).bounce >= frame.rr_start) {
        let q = clamp(max(max(throughput.x, throughput.y), throughput.z) * 1.0, 0.0, 0.95);
        if (sampler_russian_roulette(s) >= q) {
            return false;
        }
        (*st).throughput = throughput * (1.0 / q);
    }
    return true;
}

// The lamps the surface a seeing bounce left meets along the ray just
// traced (ic:1017-1048): no geometry, each looked for along the way, as far
// as what the ray hit - a little past a lamp's own stand-in, which is
// where the lamp is - weighed against the light sample there as it chose
// lamps (weight_of_lamp, ic:2795), each clamped as light found past the
// bounce before. A sun's disc, infinitely far, only where the ray escapes.
fn meet_lamps(st: ptr<function, PathState>, hit: RayIntersection) {
    let seen_from = (*st).seen;
    var reach = 0.0;
    let open = hit.kind == RAY_QUERY_INTERSECTION_NONE;
    if (!open) {
        let stand_in = (scene_instances[hit.instance_custom_data].flags & INSTANCE_STAND_IN) != 0u;
        reach = select(hit.t, hit.t * (1.0 + 1e-3) + 1e-5, stand_in);
    }
    let bounce = (*st).bounce;
    let found_at = select(bounce - 1u, 0u, bounce == 0u);
    for (var i = 0u; i < frame.light_count; i++) {
        let light = scene_lights[i];
        if (!lamp_met_by_bounces(light, seen_from.instance)) {
            continue;
        }
        let met = light_met_along(light, (*st).origin, (*st).direction);
        if (!met.ok || (!open && (met.infinite || met.t >= reach))) {
            continue;
        }
        var weight = 1.0;
        if (!seen_from.delta) {
            let chosen = lamps_selection_pdf_at(frame.light_hierarchy != 0u, seen_from.position, seen_from.normal, false, i);
            weight = mis_power(seen_from.density, light_density_along(light, seen_from.position, seen_from.direction) * chosen);
        }
        let lobes = select(seen_from.glossy, vec3<f32>(1.0), light_has(light, LF_DIFFUSE));
        (*st).radiance += clamp_light((*st).throughput * lobes * met.radiance * weight, light_ceiling((*st).specular_chain, found_at));
    }
}

// ---- The shadow catcher -------------------------------------------------

// blocked_towards with neither glass nor cutouts: one shadow ray from the
// point to `reach` along `direction`, and 1 less the luminance of what gets
// through - 0 or 1.
fn blocked_towards(position: vec3<f32>, magnitude: f32, normal: vec3<f32>, direction: vec3<f32>, reach: f32) -> f32 {
    let aim = position + direction * reach;
    let through = select(1.0, 0.0, occluded(offset_at(position, normal, magnitude), normal, aim, -direction));
    return 1.0 - clamp(luminance(vec3<f32>(through)), 0.0, 1.0);
}

// occlusion_at: ambient occlusion within `radius`, one cosine-weighted
// direction drawn from the light-sample dimensions of whatever bounce the
// sampler is on.
fn occlusion_at(position: vec3<f32>, magnitude: f32, normal: vec3<f32>, radius: f32, s: ptr<function, Sampler>) -> f32 {
    let f = frame_of(normal);
    let direction = to_world(f, sample_cosine_hemisphere(sampler_light_sample(s)));
    // A finite distance: stopped a thousandth short of it.
    return blocked_towards(position, magnitude, normal, direction, max(radius, 1e-3) * (1.0 - 1e-3));
}

// ic:3065: how many directions shadow_at draws before it keeps the last.
const MAX_SHADOW_DRAWS: u32 = 32u;

// shadow_at (ic:3087-3177): the share of the light onto this point the scene
// blocks, one direction drawn in proportion to the light it would bring.
// The card has no lamps, so the sky is all of it: a floor facing straight
// up keeps its first draw; any other surface keeps a draw by its cosine,
// the draws after the first on the dimensions of the bounces after.
fn shadow_at(position: vec3<f32>, magnitude: f32, normal: vec3<f32>, bounce: u32, s: ptr<function, Sampler>) -> f32 {
    if (frame.shadows == 0u) {
        return 0.0;
    }
    let total = env_irradiance_bound(normal);
    if (!(total > 0.0)) {
        return 0.0;
    }
    for (var draw = 0u; draw < MAX_SHADOW_DRAWS; draw++) {
        sampler_begin_bounce(s, bounce + draw);
        // Drawn in the processor's order, the open stream's past the Sobol'
        // limit; with no lamp to choose, the pick chooses the sky.
        let pick = sampler_light_select(s);
        let u = sampler_light_sample(s);
        let chance = sampler_bsdf_lobe(s);
        _ = pick;
        let last = draw + 1u == MAX_SHADOW_DRAWS;
        let sample = env_sample_irradiance(normal, u);
        if (!sample.ok) {
            continue;
        }
        if (chance < sample.keep || (last && sample.keep > 0.0)) {
            return blocked_towards(position, magnitude, normal, sample.direction, frame.shadow_far);
        }
    }
    return 0.0;
}

// What a catcher a path meets other than as the camera would see it does
// (ic:1472-1519): ends the path glowing, lets it through, or is shaded as
// its material.
const MET_SHADE: u32 = 0u;
const MET_PASSED: u32 = 1u;
const MET_END: u32 = 2u;

fn floor_met(st: ptr<function, PathState>, s: ptr<function, Sampler>, c: GpuCatcher, raw: Surface) -> u32 {
    let direction = (*st).direction;
    // A floor with a bounce glows for everything but the camera, darker
    // toward the contact over its falloff.
    if (c.bounce.w != 0.0) {
        let towards = select(-raw.geometric, raw.geometric, dot(raw.geometric, -direction) > 0.0);
        var open = 1.0;
        if (c.bounce_falloff > 0.0) {
            open = 1.0 - occlusion_at(raw.position, raw.magnitude, towards, c.bounce_falloff, s);
        }
        (*st).radiance += (*st).throughput * c.bounce.xyz * open;
        return MET_END;
    }
    // Ground illumination shades the floor as its material.
    if (frame.ground_illumination != 0u) {
        return MET_SHADE;
    }
    // Else straight on through it, to what lies below the horizon; light
    // sampling did not see past it, so what is found beyond takes full
    // weight, as past a mirror.
    if ((*st).crossings >= MAX_CUTOUT_CROSSINGS) {
        return MET_END;
    }
    (*st).crossings += 1u;
    let g = select(raw.geometric, -raw.geometric, dot(direction, raw.geometric) > 0.0);
    (*st).origin = offset_at(raw.position, -g, raw.magnitude);
    (*st).prev_delta = true;
    (*st).seen.valid = false;
    return MET_PASSED;
}

// What the floor shows and how much of the backdrop it hides.
struct Caught {
    radiance: vec3<f32>,
    coverage: f32,
};

// catch_shadow up to the reflection (ic:2888-2925): what is behind the floor
// darkened towards the shadow's colour by the share of the light the scene
// blocks there - a cast shadow, or ambient occlusion - and that share.
fn catch_shadow(c: GpuCatcher, raw: Surface, normal: vec3<f32>, behind: vec3<f32>, bounce: u32, s: ptr<function, Sampler>) -> Caught {
    var coverage = 0.0;
    if (c.shadows == 0u) {
        coverage = 0.0;
    } else if (c.occlusion_only != 0u) {
        coverage = occlusion_at(raw.position, raw.magnitude, normal, c.occlusion_radius, s);
    } else {
        coverage = shadow_at(raw.position, raw.magnitude, normal, bounce, s);
        sampler_begin_bounce(s, bounce);
    }
    coverage = clamp(coverage, 0.0, 1.0);
    return Caught(lerp3(behind, c.shadow_color.xyz, coverage), coverage);
}

// floor_reflection up to its path (ic:2982-3026): the mirror ray off a GGX
// facet of the floor's roughness, or off its normal; not ok where it does
// not leave the floor. By distance, how near the product is (`near`).
struct MirrorRay {
    ok: bool,
    origin: vec3<f32>,
    direction: vec3<f32>,
    // How squarely it leaves the floor: the reflection's cosine to it.
    rise: f32,
    near: f32,
};

fn mirror_ray(c: GpuCatcher, raw: Surface, normal: vec3<f32>, view: vec3<f32>, s: ptr<function, Sampler>) -> MirrorRay {
    var none: MirrorRay;
    none.ok = false;
    let roughness = clamp(c.reflection_roughness, 0.0, 1.0);
    var facet = normal;
    if (roughness > 1e-3) {
        let f = frame_of(normal);
        let wo = to_local(f, -view);
        facet = to_world(f, ggx_sample_visible_normal(ggx_from_roughness(roughness, 0.0), wo, sampler_bsdf_direction(s)));
    }
    let r = view - facet * (2.0 * dot(view, facet));
    let len2 = dot(r, r);
    if (!(len2 > 0.0 && is_finite(len2))) {
        return none;
    }
    let reflected = r / sqrt(len2);
    let rise = dot(reflected, normal);
    if (rise <= 0.0) {
        return none;
    }
    var near = 1.0;
    if (c.fade == FADE_BY_DISTANCE) {
        near = occlusion_at(raw.position, raw.magnitude, normal, c.reflection_fade, s);
        if (near <= 0.0) {
            return none;
        }
    }
    return MirrorRay(true, offset_at(raw.position, normal, raw.magnitude), reflected, rise, near);
}

// A floor the camera sees, held while its reflection's path is traced: the
// path that met it - what it has gathered and carries, and its bounce - and
// the floor - what it stands in front of, its shadow, the catcher and the
// way it is seen - with the mirror ray's rise and nearness.
struct FloorSeen {
    radiance: vec3<f32>,
    throughput: vec3<f32>,
    bounce: u32,
    behind: vec3<f32>,
    caught: Caught,
    catcher: GpuCatcher,
    normal: vec3<f32>,
    view: vec3<f32>,
    rise: f32,
    near: f32,
};

// floor_reflection after its path (ic:3034-3054) and catch_shadow after the
// reflection (ic:2933-2952): nothing where the path escaped at its first
// segment; else its share - the strength, by nearness, fading by the height
// of what it met first, weakened by Fresnel - taken from what the floor
// shows, and added to how much of the backdrop it hides.
fn reflected_on(floor: FloorSeen, seen: vec3<f32>, escaped: bool, depth: f32) -> Caught {
    let c = floor.catcher;
    if (escaped) {
        return floor.caught;
    }
    var share = clamp(c.reflections, 0.0, 1.0) * floor.near;
    if (c.fade == FADE_BY_HEIGHT) {
        let height = depth * floor.rise;
        let t = clamp(height / max(c.reflection_fade, 1e-6), 0.0, 1.0);
        share *= 1.0 - t * t * (3.0 - 2.0 * t);
        if (share <= 0.0) {
            return floor.caught;
        }
    }
    if (c.reflection_fresnel != 0u) {
        let cosine = clamp(abs(dot(-floor.view, floor.normal)), 0.0, 1.0);
        let m = 1.0 - cosine;
        share *= 0.04 + 0.96 * (m * m * m * m * m);
    }
    let caught = floor.caught;
    return Caught(lerp3(caught.radiance, seen, share), caught.coverage + share * (1.0 - caught.coverage));
}

// One camera sample. WGSL has no recursion, so the path the floor's
// reflection traces (floor_reflection's trace_path, ic:3027-3033) runs in
// this same loop once the camera's path has met the floor and ended there:
// the floor is held (FloorSeen) and the loop goes on with the mirror ray's
// path - at most three bounces, in the floor's reflection, so a floor it
// meets is never seen as the camera sees it and never reflects again - on
// the same sampler, which it takes from bounce 0 again, with its own
// bounces, specular chain and clamps. When that path ends, the floor is
// finished with what it saw.
fn path(x: u32, y: u32, sample_index: u32) -> PathOut {
    var s = sampler_new(x, y, sample_index, frame.scene_seed);
    let j = sampler_pixel_jitter(&s);
    let film = vec2<f32>((f32(x) + j.x) / f32(frame.width), (f32(y) + j.y) / f32(frame.height));
    let sx = 2.0 * film.x - 1.0;
    let sy = 1.0 - 2.0 * film.y;
    let camera_dir = normalized(vec3<f32>(sx * frame.half_width, sy * frame.half_height, -1.0));
    let origin = rows_point(frame.camera_to_world, vec3<f32>(0.0));
    let direction = normalized(rows_vector(frame.camera_to_world, camera_dir));

    var st = path_start(origin, direction, frame.max_bounces, true);
    var background = vec3<f32>(0.0);
    var coverage = 0.0;
    // The floor the camera's path ended on, and whether there is one.
    var floor: FloorSeen;
    var on_floor = false;
    // Whether `st` is the floor's reflection's path; whether that escaped
    // at its first segment, and how far that went.
    var mirror = false;
    var escaped = false;
    var depth = 0.0;
    // The mirror ray's first hit, found before its path is set up.
    var next: RayIntersection;
    var have_next = false;
    loop {
        var hit: RayIntersection;
        if (have_next) {
            hit = next;
            have_next = false;
        } else {
            hit = trace(st.origin, st.direction);
        }
        // The lamps a seeing bounce, or a mirror, meets on the way.
        if (st.seen.valid) {
            meet_lamps(&st, hit);
        }
        if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
            let sky = path_escape(&st);
            if (st.bounce == 0u) {
                if (mirror) {
                    escaped = true;
                } else {
                    background = sky;
                }
            }
            break;
        }
        if (st.bounce == 0u) {
            if (mirror) {
                depth = hit.t;
            } else {
                coverage = 1.0;
            }
        }
        let raw = surface_of(hit);
        let catcher = scene_instances[hit.instance_custom_data].catcher;
        if (catcher != NO_CATCHER) {
            let c = scene_catchers[catcher];
            if (st.catcher_view) {
                // Seen as the camera would see it (ic:1396-1471): what it
                // stands in front of, by the rule an escaping path follows,
                // with its shadow and its reflection on it; and nothing
                // further along this path. Never in the reflection's path,
                // whose catcher_view is false.
                let view = st.direction;
                let normal = select(-raw.geometric, raw.geometric, dot(raw.geometric, -view) > 0.0);
                let behind = env_radiance(st.bounce == 0u, view);
                let caught = catch_shadow(c, raw, normal, behind, st.bounce, &s);
                floor = FloorSeen(st.radiance, st.throughput, st.bounce, behind, caught, c, normal, view, 0.0, 1.0);
                on_floor = true;
                if (c.reflections > 0.0) {
                    let ray = mirror_ray(c, raw, normal, view, &s);
                    if (ray.ok) {
                        // A short reflection is gone above its fade, and
                        // one that never fades meets nothing over most of
                        // the floor: one intersection says so first.
                        let first = trace(ray.origin, ray.direction);
                        let met = first.kind != RAY_QUERY_INTERSECTION_NONE;
                        let reaches = c.fade == FADE_BY_DISTANCE || (met && !(c.fade == FADE_BY_HEIGHT && first.t * ray.rise >= c.reflection_fade));
                        if (reaches) {
                            floor.rise = ray.rise;
                            floor.near = ray.near;
                            st = path_start(ray.origin, ray.direction, min(frame.max_bounces, 3u), false);
                            mirror = true;
                            next = first;
                            have_next = true;
                            continue;
                        }
                    }
                }
                break;
            }
            let met = floor_met(&st, &s, c, raw);
            if (met == MET_END) {
                break;
            }
            if (met == MET_PASSED) {
                continue;
            }
        }
        let inst = scene_instances[hit.instance_custom_data];
        // A lamp's stand-in shows only where the camera sees it straight on
        // (stand_in_shows, ic:1330); past the first surface the lamp itself
        // is met (meet_lamps), and any other path passes the stand-in, as
        // shadow rays do (ic:1568).
        let stand_in = (inst.flags & INSTANCE_STAND_IN) != 0u;
        if (stand_in && !(st.specular_chain && ((frame.lamp_flags & LAMPS_IN_MIRRORS) == 0u || st.bounce == 0u))) {
            if (st.crossings >= MAX_CUTOUT_CROSSINGS) {
                break;
            }
            st.crossings += 1u;
            let g = select(raw.geometric, -raw.geometric, dot(st.direction, raw.geometric) > 0.0);
            st.origin = offset_at(raw.position, -g, raw.magnitude);
            continue;
        }
        // A glowing surface met from the front (ic:1813-1860): a stand-in,
        // its face as its lamp lights, falling off to its edges.
        let glow = scene_materials[inst.material].emission;
        if (glow.w != 0.0 && dot(st.direction, raw.geometric) <= 0.0) {
            var face = 1.0;
            if (inst.light != NO_LIGHT) {
                let lamp = scene_lights[inst.light];
                if (lamp.kind == LIGHT_AREA) {
                    face = light_face_at(lamp, raw.position);
                }
            }
            let found_at = select(st.bounce - 1u, 0u, st.bounce == 0u);
            st.radiance += clamp_light(st.throughput * glow.xyz * (1.0 * face), light_ceiling(st.specular_chain, found_at));
        }
        if (!shade_vertex(&st, &s, hit, facing(raw, st.direction))) {
            break;
        }
    }
    if (!on_floor) {
        return PathOut(sanitized(st.radiance), background, coverage);
    }
    // The floor: its light added to the camera path's, unclamped, and at the
    // camera's own hit the backdrop it leaves showing and the coverage.
    var caught = floor.caught;
    if (mirror) {
        caught = reflected_on(floor, sanitized(st.radiance), escaped, depth);
    }
    let radiance = floor.radiance + floor.throughput * caught.radiance;
    if (floor.bounce == 0u) {
        background = floor.throughput * floor.behind * (1.0 - caught.coverage);
        coverage = caught.coverage;
    }
    return PathOut(sanitized(radiance), background, coverage);
}

@compute @workgroup_size(8, 8)
fn transport_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (id.x >= frame.width || id.y >= frame.height) {
        return;
    }
    let pixel = id.y * frame.width + id.x;
    let pixels = frame.width * frame.height;
    for (var i = 0u; i < frame.samples; i++) {
        let out = path(id.x, id.y, frame.sample_base + i);
        if ((frame.mode & 1u) != 0u) {
            accumulate(pixel, out.radiance, out.background, out.coverage);
        } else {
            let at = (i * pixels + pixel) * 2u;
            frame_samples[at] = vec4<f32>(out.radiance, out.coverage);
            frame_samples[at + 1u] = vec4<f32>(out.background, 0.0);
        }
    }
}
