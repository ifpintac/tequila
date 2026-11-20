// The first transport (docs/07 R2.5): the processor's path for a scene of
// grey diffuse surfaces under the environment, draw for draw - camera ray,
// next-event estimation of the sky by its facing tables with power-heuristic
// MIS, a cosine-sampled Lambert bounce, Russian roulette, the bounce limit,
// the clamps. integrator.rs is the specification; the line references in
// the comments are to it. One invocation is one pixel, drawing
// frame.samples samples in order.
//#include sampler
//#include environment

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

// Onb::from_normal [Duff et al. 2017].
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
    rayQueryInitialize(&rq, scene_tlas, RayDesc(RAY_FLAG_FORCE_OPAQUE, 0xFFu, 0.0, 3.0e38, origin, direction));
    rayQueryProceed(&rq);
    return rayQueryGetCommittedIntersection(&rq);
}

// Intersector::occluded through shadow_ray: both ends offset, stopped just
// short of the aim.
fn occluded(from_point: vec3<f32>, from_normal: vec3<f32>, to: vec3<f32>, to_normal: vec3<f32>) -> bool {
    let origin = offset_at(from_point, from_normal, 0.0);
    let aim = offset_at(to, to_normal, 0.0);
    let delta = aim - origin;
    let distance = sqrt(dot(delta, delta));
    let direction = select(from_normal, delta / distance, distance > 0.0);
    var rq: ray_query;
    rayQueryInitialize(&rq, scene_tlas, RayDesc(RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT, 0xFFu, 0.0, distance * (1.0 - 1e-4), origin, direction));
    rayQueryProceed(&rq);
    return rayQueryGetCommittedIntersection(&rq).kind != RAY_QUERY_INTERSECTION_NONE;
}

const ALBEDO: f32 = 0.5;

struct PathOut {
    radiance: vec3<f32>,
    background: vec3<f32>,
};

fn sanitized(c: vec3<f32>) -> vec3<f32> {
    var out = c;
    for (var i = 0u; i < 3u; i++) {
        let v = c[i];
        // Not finite, or not above zero: nothing.
        out[i] = select(0.0, v, v > 0.0 && v < 3.4e38);
    }
    return out;
}

fn path(x: u32, y: u32, sample_index: u32) -> PathOut {
    var s = sampler_new(x, y, sample_index, frame.scene_seed);
    let j = sampler_pixel_jitter(&s);
    let film = vec2<f32>((f32(x) + j.x) / f32(frame.width), (f32(y) + j.y) / f32(frame.height));
    let sx = 2.0 * film.x - 1.0;
    let sy = 1.0 - 2.0 * film.y;
    let camera_dir = normalized(vec3<f32>(sx * frame.half_width, sy * frame.half_height, -1.0));
    var origin = rows_point(frame.camera_to_world, vec3<f32>(0.0));
    var direction = normalized(rows_vector(frame.camera_to_world, camera_dir));

    var radiance = vec3<f32>(0.0);
    var background = vec3<f32>(0.0);
    var throughput = vec3<f32>(1.0);
    // Read only once a bounce has set it: the camera ray is weighed 1.
    var prev_pdf = 0.0;
    var specular_chain = true;
    var sky_facing = vec3<f32>(0.0, 1.0, 0.0);
    var bounce = 0u;
    loop {
        let hit = trace(origin, direction);
        if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
            // ic:1259-1324: the sky, weighed against having been sampled.
            let weight = select(mis_power(prev_pdf, env_pdf_facing(direction, sky_facing)), 1.0, specular_chain);
            let sky = env_radiance(bounce == 0u, direction);
            if (bounce == 0u) {
                background = sky;
            }
            let found_at = select(bounce - 1u, 0u, bounce == 0u);
            radiance += clamp_light(throughput * sky * weight, light_ceiling(specular_chain, found_at));
            break;
        }
        var surf = surface_of(hit);
        // facing_forward: both normals to the side the ray came from.
        if (dot(direction, surf.geometric) > 0.0) {
            surf.geometric = -surf.geometric;
            surf.shading = -surf.shading;
        }
        if (dot(surf.shading, surf.geometric) <= 0.0) {
            surf.shading = surf.geometric;
        }
        surf.shading = valid_for_reflection(surf.geometric, -direction, surf.shading);
        if (bounce >= frame.max_bounces) {
            break;
        }
        let f = frame_of(surf.shading);
        let wo = to_local(f, -direction);
        if (wo.z <= 0.0) {
            break;
        }
        sampler_begin_bounce(&s, bounce);
        let lambert = vec3<f32>(ALBEDO) * (FRAC_1_PI * 1.0 * (1.0 + 0.0 * 0.0));

        // Next-event estimation of the sky (ic:4279-4412).
        sky_facing = surf.geometric;
        let light = env_sample_facing(surf.geometric, sampler_light_sample(&s));
        if (light.ok && any(light.radiance > vec3<f32>(0.0)) && dot(light.direction, surf.geometric) > 0.0) {
            let wi = to_local(f, light.direction);
            if (wi.z > 0.0) {
                let bsdf_pdf = max(wi.z, 0.0) * FRAC_1_PI;
                let w = mis_power(light.pdf, bsdf_pdf);
                let scale = light.radiance * ((abs(wi.z) * w) / light.pdf);
                let from_point = offset_at(surf.position, surf.geometric, surf.magnitude);
                let aim = surf.position + light.direction * frame.shadow_far;
                if (!occluded(from_point, surf.geometric, aim, -light.direction)) {
                    radiance += clamp_light(throughput * (lambert * scale), light_ceiling(specular_chain, bounce));
                }
            }
        }

        // The bounce (ic:2318-2433).
        let wi = sample_cosine_hemisphere(sampler_bsdf_direction(&s));
        let pdf = max(wi.z, 0.0) * FRAC_1_PI;
        if (!(wi.z > 0.0) || !(pdf > 0.0)) {
            break;
        }
        let wi_world = to_world(f, wi);
        if (dot(wi_world, surf.geometric) <= 0.0) {
            break;
        }
        throughput = throughput * (lambert * (abs(wi.z) / pdf));
        prev_pdf = pdf;
        specular_chain = false;
        origin = offset_at(surf.position, surf.geometric, surf.magnitude);
        direction = wi_world;
        bounce += 1u;
        // Russian roulette, reading the vertex just left (ic:2556-2562).
        if (bounce >= frame.rr_start) {
            let q = clamp(max(max(throughput.x, throughput.y), throughput.z) * 1.0, 0.0, 0.95);
            if (sampler_russian_roulette(&s) >= q) {
                break;
            }
            throughput = throughput * (1.0 / q);
        }
    }
    return PathOut(sanitized(radiance), background);
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
        let at = (i * pixels + pixel) * 2u;
        frame_samples[at] = vec4<f32>(out.radiance, 0.0);
        frame_samples[at + 1u] = vec4<f32>(out.background, 0.0);
    }
}
