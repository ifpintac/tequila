// The transport (docs/07 R2.5, R3): the processor's path for a scene of
// OpenPBR surfaces (openpbr.wgsl), glass among them, under the environment
// and the analytic lamps (lamps.wgsl), standing on a shadow catcher, draw
// for draw - camera ray, next-event estimation of the sky by its facing
// tables (the whole sky through glass) with power-heuristic MIS and of one
// lamp chosen as the processor chooses it, shadow rays through clear glass,
// the stack's bounce, Russian roulette, the bounce limit, the clamps; the
// lamps a bounce off a sharp enough reflection meets along its way, shared
// with the light sample there, and the stand-ins the camera sees them by;
// glass: the media a path is in, the interfaces between them and the
// surfaces that are none, Beer-Lambert absorption, the glass a light sample
// passed and the glass a seeing bounce sees, and the split at the first
// plain glass the camera sees (its reflection a path of its own); and the
// floor: its shadow, its reflection (a path of its own), a floor a bounce
// meets passed, glowing or shaded. The paths the processor traces from
// within a path run in the same loop, as WGSL has no recursion.
// integrator.rs is the specification; the line references in the comments
// are to it. One invocation is one pixel, drawing frame.samples samples in
// order.
//#include sampler
//#include environment
//#include openpbr
//#include lamps
//#include program

@group(0) @binding(6) var<storage, read> scene_materials: array<GpuMaterial>;
// The shadow catchers (scene.rs's GpuCatcher), by an instance's `catcher`.
@group(0) @binding(7) var<storage, read> scene_catchers: array<GpuCatcher>;
// Where each material's program is (program.rs's GpuProgram), by material.
@group(0) @binding(13) var<storage, read> scene_programs: array<GpuProgram>;

struct Surface {
    position: vec3<f32>,
    magnitude: f32,
    geometric: vec3<f32>,
    shading: vec3<f32>,
    // Whether the ray met the back of the triangle (Hit::is_backface): set
    // by facing, which turns both normals to it.
    backfacing: bool,
};

// The hit's place as Embree's resolve builds it (embree.rs): the point from
// the triangle's corners in the mesh, through the part's place, its
// magnitude and its geometric normal - what stepping past it asks (the
// probe, a shadow ray's walk); its shading normal the geometric one.
fn hit_place(hit: RayIntersection) -> Surface {
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
    s.shading = s.geometric;
    return s;
}

// The hit as Embree's resolve builds it, its shading normal the mesh's
// own interpolated where it has them.
fn surface_of(hit: RayIntersection) -> Surface {
    var s = hit_place(hit);
    let inst = scene_instances[hit.instance_custom_data];
    let mesh = scene_meshes[inst.mesh];
    if (mesh.has_normals != 0u) {
        let first = mesh.first_index + 3u * hit.primitive_index;
        let i0 = mesh.base_vertex + scene_indices[first];
        let i1 = mesh.base_vertex + scene_indices[first + 1u];
        let i2 = mesh.base_vertex + scene_indices[first + 2u];
        let u = hit.barycentrics.x;
        let v = hit.barycentrics.y;
        let w = 1.0 - u - v;
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
            case 4u: { if (!ggx_is_smooth(b.transmission.distribution)) { return false; } }
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
    // Not finite, or not above zero: nothing. Lane by lane without a
    // run-time index into the vector, which this driver has written to the
    // wrong lane inside a loop (program.wgsl).
    return select(vec3<f32>(0.0), c, (c > vec3<f32>(0.0)) & (c < vec3<f32>(3.4e38)));
}

// ic:215: a bound on the crossings of a path, rather than a budget spent
// from its bounces.
const MAX_CUTOUT_CROSSINGS: u32 = 64u;

// The processor's f32::INFINITY for a density no light sample stands in
// for (off a mirror-smooth lobe, at a path's start, past a floor passed):
// the largest float, which mis_power weighs 1 and a comparison tells apart.
const PDF_INFINITE: f32 = 3.40282347e38;
// What no solid, no medium and no instance is.
const NO_SOLID: u32 = 0xFFFFFFFFu;
const NO_MEDIUM: u32 = 0xFFFFFFFFu;
const NO_INSTANCE: u32 = 0xFFFFFFFFu;

// The surface a seeing bounce left, or a mirror's view, or the camera's
// view through glass (ic:2654's SeenFrom): where it was and its hit's
// magnitude, its geometric normal on the side the bounce left by, whether
// it lets light through (its light sample chose lamps on both sides), the
// way it left, its instance (by its index in the snapshot; NO_INSTANCE for
// the camera's view and a split reflection's start), whether it is the
// camera's view, whether it sees glass that bends light as glass, the
// density the lobes that see gave that way (PDF_INFINITE off a
// mirror-smooth lobe, which no light sample stands in for), the share of
// the value its glossy lobes gave, which way its light sample drew the sky
// (zero: the whole sky), and whether the path has met glass that bends
// light as glass since.
struct SeenFrom {
    valid: bool,
    position: vec3<f32>,
    magnitude: f32,
    normal: vec3<f32>,
    two_sided: bool,
    direction: vec3<f32>,
    instance: u32,
    camera: bool,
    sees_glass: bool,
    density: f32,
    glossy: vec3<f32>,
    sky_facing: vec3<f32>,
    met: bool,
};

// A mirror's view, or the camera's through glass (ic:2467-2497): the lamps
// it meets are its whole.
fn seen_view(position: vec3<f32>, magnitude: f32, normal: vec3<f32>, direction: vec3<f32>, instance: u32, camera: bool) -> SeenFrom {
    return SeenFrom(true, position, magnitude, normal, false, direction, instance, camera, false, PDF_INFINITE, vec3<f32>(1.0), vec3<f32>(0.0), false);
}

// SeenFrom::meets: as the bounce or the mirror it is (met_by_bounces), or,
// for the camera's view through glass, a panel it sees at all.
fn seen_meets(seen: SeenFrom, light: GpuLight) -> bool {
    if (seen.camera) {
        return (light.flags & (LF_CAN_BE_MET | LF_CAMERA)) == (LF_CAN_BE_MET | LF_CAMERA) && light.kind == LIGHT_AREA;
    }
    if (seen.instance == NO_INSTANCE) {
        return (light.flags & (LF_CAN_BE_MET | LF_SPECULAR)) == (LF_CAN_BE_MET | LF_SPECULAR);
    }
    return lamp_met_by_bounces(light, seen.instance);
}

// A medium a path is inside (ic:338's Medium, without scattering, walks or
// Volume Maps): the solid it is, its priority, its index, and the material
// whose volume absorbs along the path (its GLASS_VOLUME).
struct Medium {
    solid: u32,
    priority: f32,
    ior: f32,
    material: u32,
};

// MediumStack (ic:563): at most four, in the order entered.
const MAX_MEDIA: u32 = 4u;

struct Media {
    media: array<Medium, 4>,
    len: u32,
};

fn media_position(m: Media, solid: u32) -> u32 {
    for (var i = 0u; i < m.len; i++) {
        if (m.media[i].solid == solid) {
            return i;
        }
    }
    return NO_MEDIUM;
}

// MediumStack::top: the medium present, the highest priority, ties to the
// later entry; leaving out the entry at `skip`.
fn media_top(m: Media, skip: u32) -> u32 {
    var best = NO_MEDIUM;
    for (var i = 0u; i < m.len; i++) {
        if (i != skip && (best == NO_MEDIUM || m.media[i].priority >= m.media[best].priority)) {
            best = i;
        }
    }
    return best;
}

fn media_solid(m: Media, at: u32) -> u32 {
    if (at == NO_MEDIUM) {
        return NO_SOLID;
    }
    return m.media[at].solid;
}

// MediumStack::beyond: the medium present on the far side of a surface of
// `solid`, whose priority is `priority`.
fn media_beyond(m: Media, solid: u32, priority: f32) -> u32 {
    let at = media_position(m, solid);
    if (at != NO_MEDIUM) {
        return media_solid(m, media_top(m, at));
    }
    let top = media_top(m, NO_MEDIUM);
    if (top != NO_MEDIUM && m.media[top].priority > priority) {
        return m.media[top].solid;
    }
    return solid;
}

// MediumStack::ior_of: air for none, a medium's own index, else the index
// of what is being entered.
fn media_ior_of(m: Media, solid: u32, entering: f32) -> f32 {
    if (solid == NO_SOLID) {
        return 1.0;
    }
    let at = media_position(m, solid);
    if (at != NO_MEDIUM) {
        return m.media[at].ior;
    }
    return entering;
}

// MediumStack::cross_with: crossing a surface of `solid` leaves it if the
// path is in it, enters it otherwise; a fifth medium is not recorded.
fn media_cross(m: ptr<function, Media>, solid: u32, priority: f32, ior: f32, material: u32) {
    let at = media_position(*m, solid);
    if (at != NO_MEDIUM) {
        for (var j = at; j + 1u < (*m).len; j++) {
            (*m).media[j] = (*m).media[j + 1u];
        }
        (*m).len -= 1u;
    } else if ((*m).len < MAX_MEDIA) {
        (*m).media[(*m).len] = Medium(solid, priority, ior, material);
        (*m).len += 1u;
    }
}

// The colour a volume lets through, raised to `exponent` channel by
// channel; a channel not finite lets nothing by.
fn survive(c: f32, exponent: f32) -> f32 {
    if (!is_finite(c)) {
        return 0.0;
    }
    return cy_powf(clamp(c, 0.0, 1.0), exponent);
}

fn beer_lambert(colour: vec3<f32>, exponent: f32) -> vec3<f32> {
    return vec3<f32>(survive(colour.x, exponent), survive(colour.y, exponent), survive(colour.z, exponent));
}

// Material::transmittance_over: what `distance` of the material's volume
// lets by, its colour reached at its depth, thickened by its density.
fn transmittance_over(m: GpuMaterial, distance: f32) -> vec3<f32> {
    let depth = m.transmission_tint.w;
    if (depth <= 0.0 || distance <= 0.0) {
        return vec3<f32>(1.0);
    }
    return beer_lambert(m.transmission_color.xyz, cy_div(distance * max(m.volume_density, 0.0), depth));
}

// enclosing_media (ic:240-286): the media a path starts inside, by counting
// along its first ray, through everything - opaque surfaces too - up to 32
// surfaces, the crossings of each medium's boundary: an odd count of one
// solid's is inside it. Nothing to count where nothing transmits.
fn enclosing_media(origin: vec3<f32>, direction: vec3<f32>) -> Media {
    var md: Media;
    md.len = 0u;
    if ((frame.glass_flags & SCENE_TRANSMITS) == 0u) {
        return md;
    }
    var solids: array<u32, 4>;
    var materials: array<u32, 4>;
    var counts: array<u32, 4>;
    var used = 0u;
    var from_point = origin;
    for (var crossing = 0u; crossing < 32u; crossing++) {
        let hit = trace(from_point, direction);
        if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
            break;
        }
        let inst = scene_instances[hit.instance_custom_data];
        if ((scene_materials[inst.material].glass & GLASS_BOUNDARY) != 0u) {
            var found = false;
            for (var j = 0u; j < used; j++) {
                if (!found && solids[j] == inst.solid) {
                    counts[j] += 1u;
                    found = true;
                }
            }
            if (!found && used < MAX_MEDIA) {
                solids[used] = inst.solid;
                materials[used] = inst.material;
                counts[used] = 1u;
                used += 1u;
            }
        }
        let raw = hit_place(hit);
        let g = select(raw.geometric, -raw.geometric, dot(direction, raw.geometric) > 0.0);
        from_point = offset_at(raw.position, -g, raw.magnitude);
    }
    for (var j = 0u; j < used; j++) {
        if ((counts[j] & 1u) == 1u) {
            let m = scene_materials[materials[j]];
            md.media[md.len] = Medium(solids[j], m.medium_priority, m.base_ior, materials[j]);
            md.len += 1u;
        }
    }
    return md;
}

// Pane::passes: whether a shadow ray taking in `glass` passes clear glass.
fn pane_passes(glass: u32, bends: bool) -> bool {
    return glass == GLASS_CLEAR || (glass == GLASS_UNBENT && !bends);
}

// Pane::crossing: the light straight through the surface, met at `cos` to
// its normal: what the coat and the interface do not reflect, tinted.
fn pane_crossing(m: GpuMaterial, cos_in: f32) -> vec3<f32> {
    let c = clamp(abs(cos_in), 0.0, 1.0);
    let surface = 1.0 - fresnel_dielectric(c, m.pane_ior);
    let coat = 1.0 - m.pane_coat * fresnel_dielectric(c, m.pane_coat_ior);
    let glass = m.transmission_tint.xyz * (m.pane_weight * surface * coat);
    return vec3<f32>(1.0 - m.pane_opacity) + glass * m.pane_opacity;
}

// Pane::inside: what `distance` of the volume behind the surface lets by.
fn pane_inside(m: GpuMaterial, distance: f32) -> vec3<f32> {
    if (m.pane_depth > 0.0 && distance > 0.0) {
        return beer_lambert(m.transmission_color.xyz, cy_div(distance, m.pane_depth));
    }
    return vec3<f32>(1.0);
}

// How much light gets from one point to another, and whether it came
// through glass that bends light on the way.
struct Visibility {
    through: vec3<f32>,
    bent: bool,
};

// shadow_transmittance_through (ic:3308-3447), every part a blocker (a
// lamp's shadow links are a lamp limit), both ends offset and stopped just
// short of the aim (intersect.rs's shadow_ray): where no glass is passed,
// one any-hit query among the parts that cast shadows, all or nothing
// (Intersector::occluded, Embree's MASK_SHADOW); else the closest surfaces
// along the shadow ray one after another, over every part, at most 64 -
// clear glass taking its tint at each face and its volume between the way
// in and the way out of one solid (one at a time, as the processor pairs
// them), a part that casts no shadow passed, anything else (the card has no
// cutouts) a shadow. One ray query either way, for the code's size.
fn shadow_through(glass_in: u32, from_point: vec3<f32>, from_normal: vec3<f32>, to: vec3<f32>, to_normal: vec3<f32>) -> Visibility {
    var glass = glass_in;
    if ((frame.glass_flags & SCENE_PANES) == 0u) {
        glass = GLASS_SOLID;
    }
    let solid = glass == GLASS_SOLID;
    let flags = select(RAY_FLAG_FORCE_OPAQUE, RAY_FLAG_FORCE_OPAQUE | RAY_FLAG_TERMINATE_ON_FIRST_HIT, solid);
    let mask = select(MASK_SURFACE, MASK_SHADOW, solid);
    let start = offset_at(from_point, from_normal, 0.0);
    let aim = offset_at(to, to_normal, 0.0);
    let delta = aim - start;
    let distance = sqrt(dot(delta, delta));
    let direction = select(from_normal, delta / distance, distance > 0.0);
    var origin = start;
    var remaining = distance * (1.0 - 1e-4);
    var through = vec3<f32>(1.0);
    var bent = false;
    var along = 0.0;
    var inside = NO_SOLID;
    var since = 0.0;
    for (var crossing = 0u; crossing < MAX_CUTOUT_CROSSINGS; crossing++) {
        var rq: ray_query;
        rayQueryInitialize(&rq, scene_tlas, RayDesc(flags, mask, 0.0, remaining, origin, direction));
        rayQueryProceed(&rq);
        let hit = rayQueryGetCommittedIntersection(&rq);
        if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
            return Visibility(through, bent);
        }
        if (solid) {
            return Visibility(vec3<f32>(0.0), false);
        }
        let inst = scene_instances[hit.instance_custom_data];
        let open = (inst.flags & INSTANCE_CASTS_SHADOWS) == 0u;
        let at = along + hit.t;
        let m = scene_materials[inst.material];
        let raw = hit_place(hit);
        let bends = (m.glass & GLASS_BENDS) != 0u;
        if ((m.glass & GLASS_PANE) != 0u && pane_passes(glass, bends) && !open) {
            through *= pane_crossing(m, dot(direction, raw.geometric));
            bent = bent || bends;
            if ((m.glass & GLASS_PANE_VOLUME) != 0u) {
                if (inside == NO_SOLID) {
                    inside = inst.solid;
                    since = at;
                } else if (inside == inst.solid) {
                    through *= pane_inside(m, at - since);
                    inside = NO_SOLID;
                }
            }
        } else if (!open) {
            return Visibility(vec3<f32>(0.0), false);
        }
        if (max(max(through.x, through.y), through.z) < 1e-3) {
            return Visibility(vec3<f32>(0.0), false);
        }
        remaining -= max(hit.t, 1e-6);
        if (remaining <= 0.0) {
            return Visibility(through, bent);
        }
        let g = select(raw.geometric, -raw.geometric, dot(direction, raw.geometric) > 0.0);
        origin = offset_at(raw.position, -g, raw.magnitude);
        along = at;
    }
    return Visibility(vec3<f32>(0.0), false);
}

// SeenFrom::weight: the weight of light a bounce meets past glass it saw,
// where `straight` is the density the light sample at the surface gave the
// same light along the way the bounce left, and `reach` how far along it
// that light is (PDF_INFINITE: past everything). Shared with the sample
// where its straight line gets there through clear glass; where something
// stops it, the bounce takes all of it.
fn seen_weight(seen: SeenFrom, straight: f32, reach: f32) -> f32 {
    if (!(straight > 0.0) || seen.density >= PDF_INFINITE) {
        return 1.0;
    }
    let weight = mis_power(seen.density, straight);
    if (weight >= 1.0) {
        return 1.0;
    }
    var far = frame.shadow_far;
    if (reach < PDF_INFINITE) {
        far = reach * (1.0 - 1e-3);
    }
    let target_point = seen.position + seen.direction * far;
    let from_point = offset_at(seen.position, seen.normal, seen.magnitude);
    let open = shadow_through(GLASS_CLEAR, from_point, seen.normal, target_point, -seen.direction);
    return select(weight, 1.0, all(open.through == vec3<f32>(0.0)));
}

// What a path carries from one segment to the next: the ray, what it has
// gathered and what it carries, which way the surface it last drew the sky
// from faced (zero: the whole sky), how the last bounce left (its density;
// PDF_INFINITE off a mirror, at the start and past a floor, after which an
// escape is weighed 1), the specular chain, whether a shadow catcher met
// next is seen as the camera would see it (ic:859), the bounces and the
// crossings, the least density a bounce left by (ic:879, for Filter
// Glossy), the surface a bounce that meets the lamps left (ic:907), what
// kind of ray the segment is (RAY_*, for the Light Path node); and through
// glass: the relative indices squared (Russian roulette's eta_scale), the
// transmissions, the clear glass the last light sample passed (GLASS_*,
// ic:898) and the density the lobes that pass glass gave (ic:903), the
// media the path is in; whether it is the reflection split off the first
// glass the camera sees (`reflected`) or a floor's reflection
// (`in_floor`), and whether it has yet to ask which media it starts in.
struct PathState {
    origin: vec3<f32>,
    direction: vec3<f32>,
    radiance: vec3<f32>,
    throughput: vec3<f32>,
    sky_facing: vec3<f32>,
    prev_pdf: f32,
    specular_chain: bool,
    catcher_view: bool,
    bounce: u32,
    crossings: u32,
    max_bounces: u32,
    // Read only below 1 (filtered_roughness): any value from 1 up is the
    // processor's infinity.
    least_pdf: f32,
    seen: SeenFrom,
    ray: u32,
    eta_scale: f32,
    transmission_bounces: u32,
    through_glass: u32,
    pass_pdf: f32,
    media: Media,
    reflected: bool,
    in_floor: bool,
    fresh: bool,
};

// A path from `origin` along `direction` (trace's set-up): the camera's, a
// split reflection's (`reflected`), which shows the lamps as a reflection
// from its start (ic:935-951), or a floor's reflection's (`in_floor`),
// which sees no floor as the camera would.
fn path_start(origin: vec3<f32>, direction: vec3<f32>, max_bounces: u32, reflected: bool, in_floor: bool) -> PathState {
    var st: PathState;
    st.origin = origin;
    st.direction = direction;
    st.radiance = vec3<f32>(0.0);
    st.throughput = vec3<f32>(1.0);
    st.sky_facing = vec3<f32>(0.0);
    st.prev_pdf = PDF_INFINITE;
    st.specular_chain = true;
    st.catcher_view = !in_floor;
    st.bounce = 0u;
    st.crossings = 0u;
    st.max_bounces = max_bounces;
    st.least_pdf = 3.0e38;
    st.seen.valid = false;
    if (reflected && (frame.lamp_flags & LAMPS_IN_MIRRORS) != 0u) {
        let magnitude = max(max(abs(origin.x), abs(origin.y)), abs(origin.z));
        st.seen = SeenFrom(true, origin, magnitude, direction, false, direction, NO_INSTANCE, false, false, PDF_INFINITE, vec3<f32>(1.0), vec3<f32>(0.0), false);
    }
    st.ray = select(RAY_CAMERA, RAY_GLOSSY, reflected || in_floor);
    st.eta_scale = 1.0;
    st.transmission_bounces = 0u;
    st.through_glass = GLASS_SOLID;
    st.pass_pdf = PDF_INFINITE;
    st.media.len = 0u;
    st.reflected = reflected;
    st.in_floor = in_floor;
    st.fresh = true;
    return st;
}

// What the path meets along the segment just traced: the lamps the surface
// a seeing bounce, a mirror's view or the camera's view through glass left
// looks for (ic:1017-1048), each as far as what the ray hit - a little past
// a lamp's own stand-in, which is where the lamp is - weighed against the
// light sample there as it chose lamps (weight_of_lamp, ic:2795); and where
// the segment escapes, the sky, weighed against having been sampled
// (ic:1259-1324). One pass of the loop each, the sky last, so the two share
// their weighing past glass a bounce saw (SeenFrom::weight, a shadow ray
// of its own); each clamped as light found past the bounce before. A sun's
// disc, infinitely far, only where the ray escapes. What the sky adds is
// handed back: the background where the camera ray escapes.
fn meet_and_escape(st: ptr<function, PathState>, hit: RayIntersection) -> vec3<f32> {
    let seen = (*st).seen;
    let open = hit.kind == RAY_QUERY_INTERSECTION_NONE;
    var reach = 0.0;
    if (!open) {
        let stand_in = (scene_instances[hit.instance_custom_data].flags & INSTANCE_STAND_IN) != 0u;
        reach = select(hit.t, hit.t * (1.0 + 1e-3) + 1e-5, stand_in);
    }
    let bounce = (*st).bounce;
    let found_at = select(bounce - 1u, 0u, bounce == 0u);
    let ceiling = light_ceiling((*st).specular_chain, found_at);
    let lamps = select(0u, frame.light_count, seen.valid);
    var sky = vec3<f32>(0.0);
    for (var i = 0u; i <= lamps; i++) {
        let is_sky = i == lamps;
        var emitted: vec3<f32>;
        var lobes = vec3<f32>(1.0);
        var weight = 1.0;
        var weigh = false;
        var straight = 0.0;
        var along = PDF_INFINITE;
        if (is_sky) {
            if (!open) {
                break;
            }
            emitted = env_radiance(bounce == 0u && !(*st).reflected, (*st).direction);
            if (all(emitted == vec3<f32>(0.0))) {
                break;
            }
            // Past glass a glossy bounce saw, against the sky's sample
            // there, straight on the way it left.
            if (!(*st).specular_chain) {
                let met_glass = seen.valid && seen.met;
                let pdf = env_pdf_sky(select((*st).direction, seen.direction, met_glass), select((*st).sky_facing, seen.sky_facing, met_glass));
                if (met_glass) {
                    weigh = true;
                    straight = pdf;
                } else {
                    weight = mis_power((*st).prev_pdf, pdf);
                }
            }
        } else {
            let light = scene_lights[i];
            if (!seen_meets(seen, light)) {
                continue;
            }
            let met = light_met_along(light, (*st).origin, (*st).direction);
            if (!met.ok || (!open && (met.infinite || met.t >= reach))) {
                continue;
            }
            emitted = met.radiance;
            lobes = select(seen.glossy, vec3<f32>(1.0), light_has(light, LF_DIFFUSE));
            if (seen.density < PDF_INFINITE) {
                let chosen = lamps_selection_pdf_at(frame.light_hierarchy != 0u, seen.position, seen.normal, seen.two_sided, i);
                straight = light_density_along(light, seen.position, seen.direction) * chosen;
                if (seen.met) {
                    let from_seen = light_met_along(light, seen.position, seen.direction);
                    weigh = true;
                    along = select(PDF_INFINITE, from_seen.t, from_seen.ok && !from_seen.infinite);
                } else {
                    weight = mis_power(seen.density, straight);
                }
            }
        }
        if (weigh) {
            weight = seen_weight(seen, straight, along);
        }
        let contribution = clamp_light((*st).throughput * lobes * emitted * weight, ceiling);
        (*st).radiance += contribution;
        if (is_sky) {
            sky = contribution;
        }
    }
    return sky;
}

// facing_forward: both normals to the side the ray came from; the shading
// normal kept on the geometric one's side, and turned so the surface's
// mirror reflection stays above it (ic:1600-1621).
fn facing(raw: Surface, direction: vec3<f32>) -> Surface {
    var surf = raw;
    if (dot(direction, surf.geometric) > 0.0) {
        surf.geometric = -surf.geometric;
        surf.shading = -surf.shading;
        surf.backfacing = true;
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
// that do not see make up, their density, and what the ones that see give.
fn left_to_bounces(pdf: f32, seen_in: f32) -> vec3<f32> {
    if (!(pdf > 0.0)) {
        return vec3<f32>(1.0, pdf, 0.0);
    }
    let seen = clamp(seen_in, 0.0, pdf);
    return vec3<f32>((pdf - seen) / pdf, pdf - seen, seen);
}

// Next-event estimation at a vertex (ic:2043-2201): one sample of the sky
// (sample_environment_light, ic:4279) and one of the lamps
// (sample_analytic_lights, ic:4550), each with its own shadow ray, summed
// before the throughput, as the processor sums them. One pass of the loop
// is one light source, so the two share their evaluation of the stack,
// their shadow ray and their arithmetic. An environment turned down to
// nothing, and a scene with no lamp, draw nothing (ic:4299, 4565). A
// surface that transmits is lit from both sides: it draws the whole sky,
// chooses lamps on both sides, and its shadow rays leave from the side the
// light is on; the light through clear glass is tinted, and through glass
// that bends light the sky's is shared with the bounces that see it.
fn direct_light(s: ptr<function, Sampler>, bsdf: OpenPbr, f: Frame, wo: vec3<f32>, surf: Surface, instance: u32, transmissive: bool) -> vec3<f32> {
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
            let e = env_sample_sky(select(surf.geometric, vec3<f32>(0.0), transmissive), sampler_light_sample(s));
            if (!e.ok || e.pdf <= 0.0 || all(e.radiance == vec3<f32>(0.0))) {
                continue;
            }
            draw = LightDraw(true, e.direction, e.radiance, 0.0, true, e.pdf, false);
        } else {
            if (frame.light_count == 0u) {
                continue;
            }
            // The lamp chosen from this point, facing the side the path is on
            // (both sides for a surface that transmits), and its own variates.
            let choice = lamps_select_at(frame.light_count, frame.light_hierarchy != 0u, surf.position, surf.geometric, transmissive, sampler_light_select(s));
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
        // Lit from behind only through a surface that transmits.
        let below = dot(draw.direction, surf.geometric) <= 0.0;
        if (below && !transmissive) {
            continue;
        }
        let wi = to_local(f, draw.direction);
        if (wi.z == 0.0 || (wi.z < 0.0 && !transmissive)) {
            continue;
        }
        let value = openpbr_eval_selective(bsdf, wo, wi, diffuse, specular);
        if (all(value == vec3<f32>(0.0))) {
            continue;
        }
        // The stack's density, and its seeing lobes', in one pass.
        let densities = openpbr_densities(bsdf, wo, wi, SEEN_UP_TO, PASSED_FROM);
        let bsdf_pdf = select(densities.x, 0.0, draw.is_delta);
        var visibility = vec3<f32>(1.0);
        var bent = false;
        if (frame.shadows != 0u && shadow) {
            // Just short of a lamp; past everything for the sky or a sun;
            // out of the side the light is on.
            var reach = frame.shadow_far;
            if (!draw.infinite) {
                reach = draw.distance * (1.0 - 1e-3);
            }
            let leaving = select(surf.geometric, -surf.geometric, below);
            let v = shadow_through(frame.shadow_glass, offset_at(surf.position, leaving, surf.magnitude), leaving, surf.position + draw.direction * reach, -draw.direction);
            if (all(v.through == vec3<f32>(0.0))) {
                continue;
            }
            visibility = v.through;
            bent = v.bent;
        }
        var scale: vec3<f32>;
        if (sky) {
            var weight = mis_power(draw.pdf, bsdf_pdf);
            // Through glass that bends light, shared with the bounces that
            // pass it and those that see it.
            if (bent && (frame.lamp_flags & LAMPS_SEEN) != 0u) {
                let left = left_to_bounces(bsdf_pdf, densities.y);
                weight = left.x * mis_power(draw.pdf, left.y) + (1.0 - left.x) * mis_power(draw.pdf, left.z);
            }
            scale = draw.radiance * ((abs(wi.z) * weight) / draw.pdf);
        } else {
            // No MIS against the bounces that cannot meet a lamp; the share
            // of one they can meet that this sample keeps.
            var share = 1.0;
            if (is_shared) {
                let left = left_to_bounces(bsdf_pdf, densities.y);
                share = left.x + (1.0 - left.x) * mis_power(draw.pdf * p_select, left.z);
            }
            let density = select(draw.pdf * p_select, p_select, draw.is_delta);
            scale = draw.radiance * ((abs(wi.z) * share) / density);
        }
        direct += value * (scale * visibility);
    }
    return direct;
}

// ---- The material at a point -------------------------------------------

// Vec3 / f32 as tequila-core has it: times the reciprocal.
fn vec_over(v: vec3<f32>, s: f32) -> vec3<f32> {
    return v * (1.0 / s);
}

// A hit triangle's corners, as indices into its mesh.
fn corners_of(mesh: GpuMesh, primitive: u32) -> vec3<u32> {
    let first = mesh.first_index + 3u * primitive;
    return vec3<u32>(scene_indices[first], scene_indices[first + 1u], scene_indices[first + 2u]);
}

fn mesh_uv(mesh: GpuMesh, i: u32) -> vec2<f32> {
    let k = 2u * (mesh.uv_base + i);
    return vec2<f32>(scene_uvs[k], scene_uvs[k + 1u]);
}

// How the surface moves per unit of u and of v, in world space
// (SceneSnapshot::uv_derivatives): per triangle, from its corners and UVs.
struct Derivatives {
    ok: bool,
    dpdu: vec3<f32>,
    dpdv: vec3<f32>,
};

fn uv_derivatives(inst: GpuInstance, mesh: GpuMesh, c: vec3<u32>) -> Derivatives {
    var d: Derivatives;
    d.ok = false;
    if (mesh.uv_base == NO_DATA) {
        return d;
    }
    let p0 = scene_position(mesh.base_vertex + c.x);
    let e1 = scene_position(mesh.base_vertex + c.y) - p0;
    let e2 = scene_position(mesh.base_vertex + c.z) - p0;
    let t0 = mesh_uv(mesh, c.x);
    let d1 = mesh_uv(mesh, c.y) - t0;
    let d2 = mesh_uv(mesh, c.z) - t0;
    let det = d1.x * d2.y - d1.y * d2.x;
    if (abs(det) < 1e-20 || !(abs(det) <= 3.4e38)) {
        return d;
    }
    d.dpdu = rows_vector(inst.object_to_world, vec_over(e1 * d2.y - e2 * d1.y, det));
    d.dpdv = rows_vector(inst.object_to_world, vec_over(e2 * d1.x - e1 * d2.x, det));
    d.ok = true;
    return d;
}

// The mesh's own tangent at a point of the part's space, in world space
// (SceneSnapshot::tangent_frame, its barycentrics worked out again from
// the point); w 0 where the mesh has none.
fn mesh_tangent(inst: GpuInstance, mesh: GpuMesh, c: vec3<u32>, object: vec3<f32>) -> vec4<f32> {
    if (mesh.tangent_base == NO_DATA) {
        return vec4<f32>(0.0);
    }
    let p0 = scene_position(mesh.base_vertex + c.x);
    let e1 = scene_position(mesh.base_vertex + c.y) - p0;
    let e2 = scene_position(mesh.base_vertex + c.z) - p0;
    let d = object - p0;
    let d11 = cy_dot3(e1, e1);
    let d12 = cy_dot3(e1, e2);
    let d22 = cy_dot3(e2, e2);
    let d31 = cy_dot3(d, e1);
    let d32 = cy_dot3(d, e2);
    let det = d11 * d22 - d12 * d12;
    if (abs(det) < 1e-30) {
        return vec4<f32>(0.0);
    }
    let v = (d22 * d31 - d12 * d32) / det;
    let w = (d11 * d32 - d12 * d31) / det;
    let u = 1.0 - v - w;
    let t0 = scene_tangents[mesh.tangent_base + c.x];
    let t1 = scene_tangents[mesh.tangent_base + c.y];
    let t2 = scene_tangents[mesh.tangent_base + c.z];
    let t = vec3<f32>(u * t0.x + v * t1.x + w * t2.x, u * t0.y + v * t1.y + w * t2.y, u * t0.z + v * t1.z + w * t2.z);
    return vec4<f32>(rows_vector(inst.object_to_world, t), 1.0);
}

// Onb::from_normal_tangent: the tangent made square to the normal, or the
// normal's own frame where it is along it.
fn frame_along(n: vec3<f32>, tangent: vec3<f32>) -> Frame {
    let t = tangent - n * cy_dot3(n, tangent);
    let l2 = cy_dot3(t, t);
    if (l2 > 0.0 && l2 <= 3.4e38) {
        let along = vec_over(t, sqrt(l2));
        return Frame(along, cross(n, along), n);
    }
    return frame_of(n);
}

// integrator.rs anisotropic_frame: along the graph's Tangent where it draws
// one, else the mesh's tangent, else the UVs' u, else the normal's frame;
// turned by the rotation, in half turns.
fn anisotropic_frame(hit: RayIntersection, surf: Surface, n: vec3<f32>, drawn: vec3<f32>, rotation: f32) -> Frame {
    let inst = scene_instances[hit.instance_custom_data];
    let mesh = scene_meshes[inst.mesh];
    let c = corners_of(mesh, hit.primitive_index);
    var along = drawn;
    var have = cy_dot3(drawn, drawn) > 1e-12;
    if (!have) {
        let t = mesh_tangent(inst, mesh, c, rows_point(inst.world_to_object, surf.position));
        if (t.w != 0.0) {
            along = t.xyz;
            have = true;
        }
    }
    if (!have) {
        let d = uv_derivatives(inst, mesh, c);
        if (d.ok) {
            along = d.dpdu;
            have = true;
        }
    }
    var f = frame_of(n);
    if (have) {
        f = frame_along(n, along);
    }
    if (rotation != 0.0) {
        let sc = cy_sincos(rotation * CY_PI);
        f = frame_along(n, f.t * sc.y + f.b * sc.x);
    }
    return f;
}

// Where Generated puts a point of the part's space: normalized to the
// mesh's bounds, 0 to 1, a flat axis at 0.5 (SceneSnapshot::generated_at).
fn generated_size(h: f32) -> f32 {
    return select(max(h, 1e-5), 1.0, h == 0.0);
}

fn generated_at(mesh: GpuMesh, object: vec3<f32>) -> vec3<f32> {
    let lo = mesh.bounds_min.xyz;
    let hi = mesh.bounds_max.xyz;
    let center = (lo + hi) * 0.5;
    let h = (hi - lo) * 0.5;
    let half = vec3<f32>(generated_size(h.x), generated_size(h.y), generated_size(h.z));
    return (object - center) / half * 0.5 + vec3<f32>(0.5);
}

// The material at `hit`, seen along `direction` by a ray of kind `ray`
// (ic:1626-1710): the shading point its program reads
// (SceneSnapshot::shading_point_along), and prog_shade's program, write-back
// and bump at it.
fn material_at(hit: RayIntersection, surf: Surface, direction: vec3<f32>, ray: u32) -> ProgShaded {
    let inst = scene_instances[hit.instance_custom_data];
    let p = scene_programs[inst.material];
    let base = scene_materials[inst.material];
    var s: ProgSurface;
    s.normal = surf.shading;
    s.geometric = surf.geometric;
    var pt: ProgPoint;
    if ((p.flags & PROGRAM_RUNS) == 0u) {
        return prog_shade(p, pt, base, s);
    }
    let mesh = scene_meshes[inst.mesh];
    let c = corners_of(mesh, hit.primitive_index);
    let u = hit.barycentrics.x;
    let v = hit.barycentrics.y;
    let w = 1.0 - u - v;
    pt.object = rows_point(inst.world_to_object, surf.position);
    pt.scale = inst.scale;
    let on = rows_normal(inst.object_to_world, surf.shading);
    pt.object_normal = vec_over(on, sqrt(cy_dot3(on, on)));
    pt.backfacing = select(0.0, 1.0, surf.backfacing);
    // Embree's resolve: the corners' UVs, or the barycentrics without.
    if (mesh.uv_base == NO_DATA) {
        pt.uv = vec2<f32>(u, v);
    } else {
        pt.uv = mesh_uv(mesh, c.x) * w + mesh_uv(mesh, c.y) * u + mesh_uv(mesh, c.z) * v;
    }
    pt.object_random = inst.object_random;
    pt.object_number = inst.object_number;
    pt.position = surf.position;
    pt.ray = ray;
    pt.normal = surf.shading;
    pt.true_normal = surf.geometric;
    pt.incoming = -vec_over(direction, sqrt(cy_dot3(direction, direction)));
    pt.generated = generated_at(mesh, pt.object);
    pt.object_location = vec3<f32>(inst.object_to_world[0].w, inst.object_to_world[1].w, inst.object_to_world[2].w);
    pt.object_color = inst.object_color.xyz;
    // No ray differentials: the footprint widens with the distance alone.
    s.footprint = (hit.t * frame.pixel_spread) * inst.inv_length_scale;
    let derivatives = uv_derivatives(inst, mesh, c);
    s.has_derivatives = derivatives.ok;
    s.dpdu = derivatives.dpdu;
    s.dpdv = derivatives.dpdv;
    s.object_to_world = inst.object_to_world;
    s.world_to_object = inst.world_to_object;
    return prog_shade(p, pt, base, s);
}

// What a vertex does with the path: ends it, sends it on, or sends it on
// once the reflection split off the first plain glass the camera sees has
// been traced (SplitOff).
const SHADE_END: u32 = 0u;
const SHADE_ON: u32 = 1u;
const SHADE_SPLIT: u32 = 2u;

// The reflection the camera's first plain glass splits off (ic:2267-2318):
// its ray, the share of the light it carries, what the camera's path
// carried to the glass, and whether the camera's path ends at the glass
// once it has been traced.
struct SplitOff {
    origin: vec3<f32>,
    direction: vec3<f32>,
    share: f32,
    carried: vec3<f32>,
    camera_ends: bool,
};

// Russian roulette, reading the vertex just left (ic:2556-2562), the
// relative indices crossed squared taken in: false where the path ends.
fn russian_roulette(st: ptr<function, PathState>, s: ptr<function, Sampler>) -> bool {
    let throughput = (*st).throughput;
    let q = clamp(max(max(throughput.x, throughput.y), throughput.z) * (*st).eta_scale, 0.0, 0.95);
    if (sampler_russian_roulette(s) >= q) {
        return false;
    }
    (*st).throughput = throughput * (1.0 / q);
    return true;
}

// Where a path that ends at the first glass ends: at once, or once the
// reflection split off there is traced.
fn shade_ends(split: ptr<function, SplitOff>, splitting: bool) -> u32 {
    if (splitting) {
        (*split).camera_ends = true;
        return SHADE_SPLIT;
    }
    return SHADE_END;
}

// One vertex of a path off an OpenPBR surface, glass among them: the bounce
// limit, which side of the surface the path is on and the index between
// the media either side, the glass a light sample passed or a seeing
// bounce sees, Filter Glossy, next-event estimation of the sky and the
// lamps, the split at the first plain glass the camera sees or the bounce,
// the media crossed, whether it sees, the catcher's view and Russian
// roulette (ic:1862-2562).
fn shade_vertex(st: ptr<function, PathState>, s: ptr<function, Sampler>, hit: RayIntersection, surf: Surface, split: ptr<function, SplitOff>) -> u32 {
    let bounce = (*st).bounce;
    if (bounce >= (*st).max_bounces) {
        return SHADE_END;
    }
    // The material at this point: its program run and its bump applied
    // (ic:1626-1710), then the frame - along the anisotropy's tangent where
    // the material is anisotropic (ic:1866-1885).
    let shaded = material_at(hit, surf, (*st).direction, (*st).ray);
    let material = shaded.material;
    var f: Frame;
    if (material.specular_roughness_anisotropy > 0.0) {
        f = anisotropic_frame(hit, surf, shaded.normal, shaded.drawn, material.specular_rotation);
    } else {
        f = frame_of(shaded.normal);
    }
    let wo = to_local(f, -(*st).direction);
    if (wo.z <= 0.0) {
        return SHADE_END;
    }
    let inst = scene_instances[hit.instance_custom_data];
    // Which side of the surface the path is on, read from the media it has
    // crossed into rather than from the winding, and the index between the
    // medium here and the one beyond: going from a denser medium into a
    // lighter one is the interface seen from inside (ic:1947-1970). The
    // stack's interface and transmission read that index.
    let boundary = (material.glass & GLASS_BOUNDARY) != 0u;
    var from_inside = false;
    var relative = material.specular_ior;
    if (boundary) {
        let media = (*st).media;
        let here = media_solid(media, media_top(media, NO_MEDIUM));
        let there = media_beyond(media, inst.solid, material.medium_priority);
        let n_here = media_ior_of(media, here, 1.0);
        let n_there = media_ior_of(media, there, material.specular_ior);
        if (n_there >= n_here) {
            relative = cy_div(n_there, n_here);
        } else {
            from_inside = true;
            relative = cy_div(n_here, n_there);
        }
    }
    var lobes = material;
    if (abs(relative - material.specular_ior) > 1e-6) {
        lobes.specular_ior = relative;
    }
    // Clear glass the last light sample was taken through, met on the way
    // on by a bounce that passes it - and what that bounce left by, weighed
    // as the lobes that pass glass that bends light are; glass met as glass
    // by a seeing bounce (ic:1974-1991).
    let pane = (material.glass & GLASS_PANE) != 0u;
    let bends = (material.glass & GLASS_BENDS) != 0u;
    let glass = (*st).through_glass;
    let passing = pane && (pane_passes(glass, bends) || (glass == GLASS_UNBENT && (inst.flags & INSTANCE_CASTS_SHADOWS) == 0u));
    if (passing && glass == GLASS_CLEAR && bends) {
        (*st).prev_pdf = min((*st).prev_pdf, (*st).pass_pdf);
    }
    let meets_glass = (frame.lamp_flags & LAMPS_MEETS_GLASS) != 0u;
    let seeing = meets_glass && (*st).seen.valid && (*st).seen.sees_glass && pane && !passing;
    // The stack as the integrator builds it (ic:2003-2032): every
    // reflection lobe, and the transmission, at least as rough as the
    // regularization's floor from its bounce on - but glass passed or seen,
    // which stays as it is - the lobe chances aimed at `wo`; and where lamps
    // show in mirrors, one with a mirror-smooth lobe blurred past a diffuse
    // or rough bounce as Cycles' Filter Glossy blurs it - built again, all
    // its lobes at least that rough (one call site, for the code's size).
    let kept = passing || seeing;
    let floor = select(select(0.0, frame.regularization_roughness, bounce >= frame.regularization_start), 0.0, kept);
    var blur = 0.0;
    if ((frame.lamp_flags & LAMPS_IN_MIRRORS) != 0u && frame.regularization_roughness > 0.0 && !kept) {
        blur = filtered_roughness((*st).least_pdf);
    }
    var roughness_floor = floor;
    var bsdf: OpenPbr;
    loop {
        bsdf = openpbr_aim(openpbr_build(lobes, roughness_floor, true, from_inside), wo);
        if (roughness_floor == floor && blur > floor && openpbr_has_delta(bsdf)) {
            roughness_floor = blur;
            continue;
        }
        break;
    }
    sampler_begin_bounce(s, bounce);

    // Next-event estimation (ic:2043-2224), but for a stack of mirrors
    // only and at glass being passed, whose light is the last sample's;
    // clamped as one. A surface that transmits draws the whole sky.
    let transmissive = openpbr_transmits(bsdf);
    let samples_lights = !pure_delta(bsdf) && !passing;
    if (samples_lights) {
        (*st).sky_facing = select(surf.geometric, vec3<f32>(0.0), transmissive);
        let direct = direct_light(s, bsdf, f, wo, surf, inst.instance, transmissive);
        (*st).radiance += clamp_light((*st).throughput * direct, light_ceiling((*st).specular_chain, bounce));
    }

    // The split (ic:2267-2318): at the first plain glass the camera ray
    // meets, the reflection is a path of its own, traced before the camera
    // goes on, on the same sampler; the camera's goes on through by what
    // Fresnel leaves (Pane::split), drawing no bsdf dimensions. Past the
    // critical angle the stack samples as anywhere else.
    var sample = no_sample();
    var chosen: ReflectionSample;
    chosen.roughness = -1.0;
    var splits = false;
    var share = 0.0;
    if (bounce == 0u && !(*st).reflected && !(*st).in_floor && (material.glass & (GLASS_PANE | GLASS_PLAIN)) == (GLASS_PANE | GLASS_PLAIN)) {
        let eta = select(relative, cy_div(1.0, relative), from_inside);
        let reflected = fresnel_dielectric(wo.z, eta);
        if (reflected < 1.0) {
            var wi = -wo;
            var scale = 1.0;
            var crossing = 1.0;
            var ok = true;
            if ((material.glass & GLASS_THIN) == 0u) {
                // Snell, about the normal the ray arrives against.
                let sin2_t = cy_div(max(1.0 - wo.z * wo.z, 0.0), eta * eta);
                ok = sin2_t < 1.0;
                let cos_t = cy_sqrt(max(1.0 - sin2_t, 0.0));
                wi = -wo * cy_div(1.0, eta) + vec3<f32>(0.0, 0.0, 1.0) * (cy_div(wo.z, eta) - cos_t);
                scale = cy_div(1.0, eta * eta);
                crossing = eta;
            }
            let c = abs(wi.z);
            if (ok && c > 0.0) {
                sample = LobeSample(true, wi, material.transmission_tint.xyz * cy_div((1.0 - reflected) * scale, c), 1.0, true, true, crossing);
                splits = true;
                share = reflected;
            }
        }
    }
    var splitting = false;
    if (splits) {
        let mirrored = to_world(f, vec3<f32>(-wo.x, -wo.y, wo.z));
        if (share > 0.0 && dot(mirrored, surf.geometric) > 0.0) {
            *split = SplitOff(offset_at(surf.position, surf.geometric, surf.magnitude), mirrored, share, (*st).throughput, false);
            splitting = true;
        }
    } else {
        // The bounce (ic:2316-2433).
        chosen = openpbr_sample_seen(bsdf, wo, sampler_bsdf_direction(s), sampler_bsdf_lobe(s), SEEN_UP_TO, PASSED_FROM);
        sample = chosen.sample;
    }
    if (!sample.ok || !(sample.pdf > 0.0) || all(sample.f == vec3<f32>(0.0))) {
        return shade_ends(split, splitting);
    }
    let wi_world = to_world(f, sample.wi);
    // A reflection does not leave below the geometric surface (§3.1).
    if (!sample.is_transmission && dot(wi_world, surf.geometric) <= 0.0) {
        return shade_ends(split, splitting);
    }
    let throughput = (*st).throughput * (sample.f * (abs(sample.wi.z) / sample.pdf));
    // Through the surface: counted against the transmissions, the relative
    // index squared kept for Russian roulette, and a medium's boundary
    // entered or left, recorded with the surface's own index (ic:2373-2399).
    if (sample.is_transmission) {
        (*st).transmission_bounces += 1u;
        if ((*st).transmission_bounces > frame.max_transmission_bounces) {
            return shade_ends(split, splitting);
        }
        (*st).eta_scale *= sample.eta * sample.eta;
        if (boundary) {
            // Through a local: naga's SPIR-V writer cannot pass a pointer
            // to a member.
            var media = (*st).media;
            media_cross(&media, inst.solid, material.medium_priority, material.specular_ior, inst.material);
            (*st).media = media;
        }
    }
    if (!(is_finite(throughput.x) && is_finite(throughput.y) && is_finite(throughput.z)) || all(throughput == vec3<f32>(0.0))) {
        return shade_ends(split, splitting);
    }
    (*st).throughput = throughput;
    // Straight on through glass passed, the last light sample's weighting
    // carries over (ic:2404). Whether this bounce sees (ic:2413): off a
    // reflection sharp enough to show what it is turned to, it meets the
    // lamps there and glass that bends light as glass - always off a
    // mirror-smooth one, else by a draw of the open stream against how sharp
    // it is. The transmission lobe's reflections are glass's, and do not.
    let onward = passing && sample.is_transmission && sample.is_delta;
    let lit_through = samples_lights && frame.shadows != 0u && frame.shadow_glass != GLASS_SOLID;
    let seen_on = (frame.lamp_flags & LAMPS_SEEN) != 0u;
    var sees = false;
    if (samples_lights && frame.shadows != 0u && !onward && seen_on && chosen.roughness >= 0.0) {
        if (sample.is_delta) {
            sees = true;
        } else {
            let seeing_share = openpbr_seeing_share(chosen.roughness, SEEN_UP_TO, PASSED_FROM);
            sees = seeing_share >= 1.0 || (seeing_share > 0.0 && sampler_next_unstructured(s) < seeing_share);
        }
    }
    if (!onward) {
        if (!sample.is_delta) {
            (*st).least_pdf = min((*st).least_pdf, sample.pdf);
        }
        (*st).prev_pdf = select(sample.pdf, PDF_INFINITE, sample.is_delta);
        (*st).pass_pdf = (*st).prev_pdf;
        if (seen_on && meets_glass && !sample.is_delta && !sees) {
            (*st).pass_pdf = max(sample.pdf - chosen.seen, 0.0);
        }
    }
    // What meets the lamps along the way on (ic:2442-2498): a bounce that
    // sees; the same view on through mirror-smooth glass passed or seen,
    // which has now met glass that bends light if it saw such glass; a
    // mirror's view, which no light sample stands in for; the camera's view
    // through its first glass.
    let lamps_in_mirrors = (frame.lamp_flags & LAMPS_IN_MIRRORS) != 0u;
    if (sees) {
        var glossy = vec3<f32>(1.0);
        var density = PDF_INFINITE;
        if (!sample.is_delta) {
            density = chosen.seen;
            if ((frame.lamp_flags & LAMPS_FOR_REFLECTIONS) != 0u) {
                // Whole vectors, not a lane by a run-time index in a loop:
                // this driver has written such a lane into the wrong one.
                glossy = select(vec3<f32>(0.0), clamp(chosen.glossy / sample.f, vec3<f32>(0.0), vec3<f32>(1.0)), sample.f > vec3<f32>(0.0));
            }
        }
        (*st).seen = SeenFrom(true, surf.position, surf.magnitude, surf.geometric, transmissive, wi_world, inst.instance, false, true, density, glossy, (*st).sky_facing, false);
    } else if (sample.is_delta && !samples_lights && (seeing || passing)) {
        (*st).seen.met = (*st).seen.met || (seeing && bends);
    } else if (!(sample.is_delta && lamps_in_mirrors)) {
        (*st).seen.valid = false;
    } else if (!sample.is_transmission) {
        (*st).seen = seen_view(surf.position, surf.magnitude, surf.geometric, wi_world, inst.instance, false);
    } else if ((*st).seen.valid && !(*st).seen.sees_glass) {
        // On through smooth glass, the mirror's view or the camera's.
    } else if (!(*st).seen.valid && (*st).specular_chain) {
        (*st).seen = seen_view(surf.position, surf.magnitude, surf.geometric, wi_world, NO_INSTANCE, true);
    } else {
        (*st).seen.valid = false;
    }
    // The glass the path passes on, as this vertex's light sample did
    // (ic:2499-2507).
    if (!onward) {
        if (!lit_through) {
            (*st).through_glass = GLASS_SOLID;
        } else if (sees) {
            (*st).through_glass = GLASS_UNBENT;
        } else {
            (*st).through_glass = frame.shadow_glass;
        }
    }
    (*st).specular_chain = (*st).specular_chain && sample.is_delta;
    // What the next vertex is seen by: a transmission ray through the
    // surface, a diffuse ray off the diffuse lobe, a glossy one off any
    // other (ic:2323).
    if (sample.is_transmission) {
        (*st).ray = RAY_TRANSMISSION;
    } else {
        (*st).ray = select(RAY_GLOSSY, RAY_DIFFUSE, chosen.slot == LOBE_DIFFUSE);
    }
    // ic:2510-2511: through a mirror the catcher is still seen as the camera
    // sees it; off a reflection that is not one, by a draw of the open
    // stream against how near a mirror it is; through rough glass, not.
    if ((*st).catcher_view && !sample.is_delta) {
        (*st).catcher_view = !sample.is_transmission && sampler_next_unstructured(s) < catcher_view_through(material);
    }
    (*st).origin = offset_at(surf.position, select(surf.geometric, -surf.geometric, sample.is_transmission), surf.magnitude);
    (*st).direction = wi_world;
    (*st).bounce = bounce + 1u;
    // The reflection first; the roulette once it is traced, as the
    // processor draws it after the recursion.
    if (splitting) {
        return SHADE_SPLIT;
    }
    if ((*st).bounce >= frame.rr_start && !russian_roulette(st, s)) {
        return SHADE_END;
    }
    return SHADE_ON;
}

// ---- The shadow catcher -------------------------------------------------

// blocked_towards with neither glass nor cutouts, as ambient occlusion asks
// it: one shadow ray from the point to `reach` along `direction`, and 1
// less the luminance of what gets through - 0 or 1.
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
// the draws after the first on the dimensions of the bounces after. The
// shadow ray passes the clear glass the configuration's shadow rays pass,
// as blocked_towards with the configuration's glass does (ic:3141): 1 less
// the luminance of what gets through.
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
            let open = shadow_through(frame.shadow_glass, offset_at(position, normal, magnitude), normal, position + sample.direction * frame.shadow_far, -sample.direction);
            return 1.0 - clamp(luminance(open.through), 0.0, 1.0);
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
    (*st).prev_pdf = PDF_INFINITE;
    (*st).through_glass = GLASS_SOLID;
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
    // Whether the path that met it is the camera's, whose own hit the floor
    // gives the backdrop it leaves showing and the coverage.
    camera: bool,
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

// One camera sample. WGSL has no recursion, so the paths the processor
// traces from within a path run in this same loop, each when the path that
// starts it has done what comes before it: the floor's reflection
// (floor_reflection's trace_path, ic:3027-3033) once the path that met the
// floor has ended there - the floor held (FloorSeen) and the loop going on
// with the mirror ray's path, at most three bounces, which sees no floor as
// the camera does and reflects no further - and the reflection split off
// the first plain glass the camera sees (ic:2304-2311) once the camera's
// vertex there is done - the camera's path parked, the loop going on with
// the reflection's, its own bounces from 0, a bounce fewer, showing the
// lamps as a reflection, then the camera's taken up again with what it
// brought, its Russian roulette drawn only then. Every path on the same
// sampler, each first asking which media it starts in.
fn path(x: u32, y: u32, sample_index: u32) -> PathOut {
    var s = sampler_new(x, y, sample_index, frame.scene_seed);
    let j = sampler_pixel_jitter(&s);
    let film = vec2<f32>((f32(x) + j.x) / f32(frame.width), (f32(y) + j.y) / f32(frame.height));
    let sx = 2.0 * film.x - 1.0;
    let sy = 1.0 - 2.0 * film.y;
    let camera_dir = normalized(vec3<f32>(sx * frame.half_width, sy * frame.half_height, -1.0));
    let origin = rows_point(frame.camera_to_world, vec3<f32>(0.0));
    let direction = normalized(rows_vector(frame.camera_to_world, camera_dir));

    var st = path_start(origin, direction, frame.max_bounces, false, false);
    var background = vec3<f32>(0.0);
    var coverage = 0.0;
    // The floor a path ended on, and whether there is one.
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
    // The camera's path parked at its first glass while the reflection
    // split off there is traced (`reflecting`); whether it ends there or
    // draws its roulette once the reflection is done; the split's share and
    // what the camera carried there; and the light the reflection brought.
    var parked: PathState;
    var reflecting = false;
    var parked_ends = false;
    var parked_roulette = false;
    var split_share = 0.0;
    var carried = vec3<f32>(0.0);
    var split_light = vec3<f32>(0.0);
    var out: PathOut;
    loop {
        if (st.fresh) {
            st.media = enclosing_media(st.origin, st.direction);
            st.fresh = false;
        }
        // One segment of the path: `ended` false where the path goes on.
        var ended = true;
        loop {
            var hit: RayIntersection;
            if (have_next) {
                hit = next;
                have_next = false;
            } else {
                hit = trace(st.origin, st.direction);
            }
            // The lamps met on the way, and the sky where the ray escapes.
            let sky = meet_and_escape(&st, hit);
            if (hit.kind == RAY_QUERY_INTERSECTION_NONE) {
                if (st.bounce == 0u) {
                    if (mirror) {
                        escaped = true;
                    } else if (!st.reflected) {
                        background = sky;
                    }
                }
                break;
            }
            if (st.bounce == 0u) {
                if (mirror) {
                    depth = hit.t;
                } else if (!st.reflected) {
                    coverage = 1.0;
                }
            }
            let raw = surface_of(hit);
            let inst = scene_instances[hit.instance_custom_data];
            if (inst.catcher != NO_CATCHER) {
                let c = scene_catchers[inst.catcher];
                if (st.catcher_view) {
                    // Seen as the camera would see it (ic:1396-1471): what it
                    // stands in front of, by the rule an escaping path follows,
                    // with its shadow and its reflection on it; and nothing
                    // further along this path. Never in the reflection's path,
                    // whose catcher_view is false.
                    let view = st.direction;
                    let normal = select(-raw.geometric, raw.geometric, dot(raw.geometric, -view) > 0.0);
                    let behind = env_radiance(st.bounce == 0u && !st.reflected, view);
                    let caught = catch_shadow(c, raw, normal, behind, st.bounce, &s);
                    floor = FloorSeen(st.radiance, st.throughput, st.bounce, behind, caught, c, normal, view, 0.0, 1.0, !st.reflected);
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
                                st = path_start(ray.origin, ray.direction, min(st.max_bounces, 3u), st.reflected, true);
                                mirror = true;
                                next = first;
                                have_next = true;
                                ended = false;
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
                    ended = false;
                    break;
                }
            }
            // Beer-Lambert across the segment just travelled, in the medium
            // present (ic:1527-1556): only between two surfaces.
            let top = media_top(st.media, NO_MEDIUM);
            if (top != NO_MEDIUM) {
                let medium = scene_materials[st.media.media[top].material];
                if ((medium.glass & GLASS_VOLUME) != 0u) {
                    let t = st.throughput * transmittance_over(medium, hit.t);
                    st.throughput = t;
                    if (!(is_finite(t.x) && is_finite(t.y) && is_finite(t.z)) || all(t == vec3<f32>(0.0))) {
                        break;
                    }
                }
            }
            // A lamp's stand-in shows only where the camera sees it straight on
            // (stand_in_shows, ic:1330); past the first surface the lamp itself
            // is met (meet_and_escape), and any other path passes the stand-in,
            // as shadow rays do (ic:1568).
            let stand_in = (inst.flags & INSTANCE_STAND_IN) != 0u;
            if (stand_in && !(st.specular_chain && ((frame.lamp_flags & LAMPS_IN_MIRRORS) == 0u || (st.bounce == 0u && !st.reflected)))) {
                if (st.crossings < MAX_CUTOUT_CROSSINGS) {
                    st.crossings += 1u;
                    let g = select(raw.geometric, -raw.geometric, dot(st.direction, raw.geometric) > 0.0);
                    st.origin = offset_at(raw.position, -g, raw.magnitude);
                    ended = false;
                }
                break;
            }
            // A medium's surface inside a medium that outranks it - the glass
            // wall inside the liquid it holds - is no interface: the path
            // goes straight on, and only the media it is in change
            // (ic:1581-1599).
            let base = scene_materials[inst.material];
            if ((base.glass & GLASS_BOUNDARY) != 0u
                && st.crossings < MAX_CUTOUT_CROSSINGS
                && media_beyond(st.media, inst.solid, base.medium_priority) == media_solid(st.media, top)) {
                st.crossings += 1u;
                var media = st.media;
                media_cross(&media, inst.solid, base.medium_priority, base.base_ior, inst.material);
                st.media = media;
                let g = select(raw.geometric, -raw.geometric, dot(st.direction, raw.geometric) > 0.0);
                st.origin = offset_at(raw.position, -g, raw.magnitude);
                ended = false;
                break;
            }
            // A glowing surface met from the front (ic:1813-1860): a stand-in,
            // its face as its lamp lights, falling off to its edges.
            let glow = base.emission;
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
            var off: SplitOff;
            let shaded = shade_vertex(&st, &s, hit, facing(raw, st.direction), &off);
            if (shaded == SHADE_ON) {
                ended = false;
            } else if (shaded == SHADE_SPLIT) {
                parked = st;
                parked_ends = off.camera_ends;
                parked_roulette = st.bounce >= frame.rr_start;
                split_share = off.share;
                carried = off.carried;
                reflecting = true;
                st = path_start(off.origin, off.direction, max(frame.max_bounces, 1u) - 1u, true, false);
                ended = false;
            }
            break;
        }
        if (!ended) {
            continue;
        }
        // A path has ended. One that ended on a floor has the floor's light
        // added to what it had, unclamped, and at the camera's own hit the
        // backdrop it leaves showing and the coverage.
        var result = st.radiance;
        if (on_floor) {
            var caught = floor.caught;
            if (mirror) {
                caught = reflected_on(floor, sanitized(st.radiance), escaped, depth);
            }
            result = floor.radiance + floor.throughput * caught.radiance;
            if (floor.bounce == 0u && floor.camera) {
                background = floor.throughput * floor.behind * (1.0 - caught.coverage);
                coverage = caught.coverage;
            }
            on_floor = false;
            mirror = false;
            escaped = false;
        }
        // The split's reflection: what it saw, times its share, joins the
        // camera's light; the camera's path goes on, or ends there.
        if (reflecting) {
            split_light += carried * sanitized(result) * split_share;
            st = parked;
            reflecting = false;
            if (parked_ends || (parked_roulette && !russian_roulette(&st, &s))) {
                out = PathOut(sanitized(st.radiance + split_light), background, coverage);
                break;
            }
            continue;
        }
        out = PathOut(sanitized(result + split_light), background, coverage);
        break;
    }
    return out;
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
